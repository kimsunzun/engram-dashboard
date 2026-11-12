//! Reading preset and settings files. A file is read whole or refused with a [`ParseError`]; inside a preset, an entry
//! of a list that cannot be used is a line problem and the rest of the file still reads (TRD S25 step-7 §3-4 read).
//!
//! Item lines come from serde-saphyr's `Spanned` on each list entry. The entries are read as plain trees and then
//! classified here, not through an untagged enum — `untagged` buffers the value and loses every position (B5).
// ADR-0329: backend tags sit on preset items; a tag word is accepted only when the backend registry knows it.
// ADR-0341: `folders` holds plain absolute folder paths.

use std::fmt;
use std::marker::PhantomData;

use engram_dashboard_agent::launch::Reason;
use serde::de::{self, Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};
use serde_json::json;
use serde_saphyr::Spanned;

use super::tree::{Node, NodeVisitor};
use super::{content_after_document_end, options, source_of, yaml_lines, ParseError};

use crate::backends::Backends;
use crate::path::{AssetRef, FolderRef, ItemOrigin, PresetItem};

/// The kind a file declares, read leniently ([`sniff_kind`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Sniff {
    Preset,
    Settings,
    /// A `kind` value that names no kind — the file is ours but broken.
    Unknown(String),
    /// No top-level `kind` line — not a library file.
    Absent,
}

/// The kind a file declares: the first `kind:` line at column 0, read as text, so it answers even for broken YAML
/// (M14). A quoted value is unquoted and a trailing ` # comment` dropped. A file whose only `kind` sits in a flow
/// mapping (`{kind: preset}`) answers `Absent`.
pub fn sniff_kind(text: &str) -> Sniff {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    for line in yaml_lines(text) {
        let Some(rest) = line.strip_prefix("kind:") else {
            continue;
        };
        if !rest.is_empty() && !rest.starts_with([' ', '\t']) {
            continue;
        }
        let value = unquote(strip_comment(rest).trim());
        return match value.as_str() {
            "preset" => Sniff::Preset,
            "settings" => Sniff::Settings,
            _ => Sniff::Unknown(value),
        };
    }
    Sniff::Absent
}

/// `value` up to the first `#` that starts it or follows whitespace.
fn strip_comment(value: &str) -> &str {
    let mut previous_is_space = true;
    for (index, c) in value.char_indices() {
        if c == '#' && previous_is_space {
            return &value[..index];
        }
        previous_is_space = c.is_whitespace();
    }
    value
}

fn unquote(value: &str) -> String {
    if value.len() >= 2 {
        if let Some(inner) = value.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')) {
            return inner.replace("''", "'");
        }
        if let Some(inner) = value.strip_prefix('"').and_then(|v| v.strip_suffix('"')) {
            return inner.to_owned();
        }
    }
    value.to_owned()
}

/// A preset file (§4 data shapes).
#[derive(Clone, Debug, PartialEq)]
pub struct PresetDoc {
    pub version: u32,
    pub description: Option<String>,
    /// `backend` always `None`: a tag here is a line problem (`backend-tag-not-allowed`); the entry stays modeled and a
    /// rewrite keeps its tag.
    pub presets: Vec<PresetItem>,
    pub instructions: Vec<PresetItem>,
    pub skills: Vec<PresetItem>,
    /// `backend` always `None`, as in `presets`.
    pub settings: Vec<PresetItem>,
    /// Every string entry, also one whose path is not absolute (that one is a line problem too).
    pub folders: Vec<FolderRef>,
    /// List entries the reader could not model — each is also a line problem. The writer keeps them.
    pub unmodeled: Vec<Unmodeled>,
    /// Per-entry breakage that does not stop the read, in file order of the lists (`presets` · `instructions` ·
    /// `skills` · `settings` · `folders`), entry order inside each.
    pub line_problems: Vec<Reason>,
}

/// A list entry the reader could not model.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Unmodeled {
    pub list: ListName,
    /// Position in that list, counting every entry.
    pub index: u32,
    pub line: u32,
}

/// The five lists of a preset.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ListName {
    Presets,
    Instructions,
    Skills,
    Settings,
    Folders,
}

impl ListName {
    /// File order of the lists when the writer adds a missing key.
    pub const ALL: [ListName; 5] = [
        ListName::Presets,
        ListName::Instructions,
        ListName::Skills,
        ListName::Settings,
        ListName::Folders,
    ];

    /// The key of this list in the file.
    pub fn key(self) -> &'static str {
        match self {
            ListName::Presets => "presets",
            ListName::Instructions => "instructions",
            ListName::Skills => "skills",
            ListName::Settings => "settings",
            ListName::Folders => "folders",
        }
    }

    pub(crate) fn takes_tags(self) -> bool {
        matches!(self, ListName::Instructions | ListName::Skills)
    }
}

/// A settings file: `raw` = every top-level key other than `kind` · `version` · `description`, in file order. Which
/// field belongs to a backend is decided later by `Backends::for_word(name)`; a name no backend knows is kept.
#[derive(Clone, Debug, PartialEq)]
pub struct SettingsDoc {
    pub version: u32,
    /// The scalar's text; a value YAML reads as a non-finite float or a tagged null is shown as its raw source text
    /// (`.inf` → ".inf", `!!null x` → "x"); a plain null spelling or an absent key is `None`.
    pub description: Option<String>,
    pub raw: Vec<RawField>,
}

/// One raw settings field. A non-string key is spelled as the file spells it; a non-finite float inside `value` is
/// `null` (JSON has no NaN or infinity).
#[derive(Clone, Debug, PartialEq)]
pub struct RawField {
    pub name: String,
    pub value: serde_json::Value,
    pub line: u32,
}

pub(crate) const KIND_UNKNOWN: &str = "kind-unknown";
/// An item of another kind: the writer's whole-file check answers `args {message}`; a preset entry pointing at an
/// item of another kind (resolve) answers `args {path, list}`.
pub(crate) const WRONG_KIND: &str = "wrong-kind";
const FIELD_INVALID: &str = "field-invalid";
pub(crate) const MULTI_DOCUMENT: &str = "preset-multi-document";
const ITEM_SHAPE_INVALID: &str = "item-shape-invalid";
/// An item spelling that is not a library reference: the reader answers `args {list, index, path (raw), why}`; an
/// unparsed attached path (resolve) answers `args {path (controls escaped), why}`.
pub(crate) const ITEM_PATH_INVALID: &str = "item-path-invalid";
const BACKEND_TAG_UNKNOWN: &str = "backend-tag-unknown";
const BACKEND_TAG_NOT_ALLOWED: &str = "backend-tag-not-allowed";
const FOLDER_PATH_RELATIVE: &str = "folder-path-relative";

/// Reads a preset file. `Err` = the file cannot be read as a preset: YAML refused (`yaml-syntax`), more than one
/// document or content after an explicit `...` (`preset-multi-document`), not a map or a `kind` other than `preset`
/// (`kind-unknown` · `wrong-kind`), or `version` · `description` · a list key of the wrong type (`field-invalid`). A
/// missing list key is an empty list.
pub fn parse_preset(text: &str, backends: &dyn Backends) -> Result<PresetDoc, ParseError> {
    let top: TopMap<Node, TopValue> = serde_saphyr::from_str_with_options(text, options())
        .map_err(|error| ParseError::from_yaml(&error, MULTI_DOCUMENT))?;
    refuse_document_tail(text, MULTI_DOCUMENT)?;
    let entries = top.0.ok_or_else(not_a_map)?;
    let get = |key: &str| {
        entries
            .iter()
            .find(|(k, _)| k.value.as_str() == Some(key))
            .map(|(_, value)| value)
    };
    let scalar = |key: &str| {
        get(key).map(|value| match &value.value {
            TopValue::Other(node) => Spanned::new(node.clone(), value.referenced, value.defined),
            TopValue::Seq(_) => {
                Spanned::new(Node::Seq(Vec::new()), value.referenced, value.defined)
            }
        })
    };
    check_kind(scalar("kind").as_ref(), "preset")?;
    let version = version_of(scalar("version").as_ref())?;
    let description = description_of(scalar("description").as_ref(), text)?;

    let entries_of = |list: ListName| -> Result<&[Spanned<Node>], ParseError> {
        match get(list.key()) {
            None => Ok(&[]),
            Some(value) => match &value.value {
                TopValue::Seq(items) => Ok(items),
                TopValue::Other(Node::Null) => Ok(&[]),
                TopValue::Other(_) => Err(ParseError::new(
                    FIELD_INVALID,
                    format!("`{}` must be a list", list.key()),
                    Some(line(value)),
                )),
            },
        }
    };

    let mut unmodeled = Vec::new();
    let mut line_problems = Vec::new();
    let mut item_lists: [Vec<PresetItem>; 4] = Default::default();
    let item_names = [
        ListName::Presets,
        ListName::Instructions,
        ListName::Skills,
        ListName::Settings,
    ];
    for (items, list) in item_lists.iter_mut().zip(item_names) {
        for (index, entry) in (0u32..).zip(entries_of(list)?) {
            let (item, problem) = read_item(list, index, entry, backends);
            line_problems.extend(problem);
            match item {
                Some(item) => items.push(item),
                None => unmodeled.push(Unmodeled {
                    list,
                    index,
                    line: line(entry),
                }),
            }
        }
    }
    let mut folders = Vec::new();
    for (index, entry) in (0u32..).zip(entries_of(ListName::Folders)?) {
        let (folder, problem) = read_folder(index, entry);
        line_problems.extend(problem);
        match folder {
            Some(folder) => folders.push(folder),
            None => unmodeled.push(Unmodeled {
                list: ListName::Folders,
                index,
                line: line(entry),
            }),
        }
    }
    let [presets, instructions, skills, settings] = item_lists;
    Ok(PresetDoc {
        version,
        description,
        presets,
        instructions,
        skills,
        settings,
        folders,
        unmodeled,
        line_problems,
    })
}

/// Reads a settings file. `Err` as for [`parse_preset`], with `kind: settings` required and every serde-saphyr refusal
/// — more than one document and content after an explicit `...` included — reported as `yaml-syntax`.
pub fn parse_settings(text: &str) -> Result<SettingsDoc, ParseError> {
    let top: TopMap<String, serde_json::Value> =
        serde_saphyr::from_str_with_options(text, options())
            .map_err(|error| ParseError::from_yaml(&error, super::YAML_SYNTAX))?;
    refuse_document_tail(text, super::YAML_SYNTAX)?;
    let entries = top.0.ok_or_else(not_a_map)?;

    let mut kind = None;
    let mut version = None;
    let mut description = None;
    let mut raw = Vec::new();
    for (key, value) in entries {
        // Only `description` gets the raw-text display; `kind` and `version` keep the scalar's real type, so a
        // `kind: !!null settings` is a null kind, not a settings file.
        let scalar = |raw_text_display: bool| {
            let source = if raw_text_display {
                source_of(text, &value.defined)
            } else {
                None
            };
            Spanned::new(
                json_scalar_node(&value.value, source),
                value.referenced,
                value.defined,
            )
        };
        match key.value.as_str() {
            "kind" => kind = Some(scalar(false)),
            "version" => version = Some(scalar(false)),
            "description" => description = Some(scalar(true)),
            _ => raw.push(RawField {
                line: line(&key),
                name: key.value,
                value: value.value,
            }),
        }
    }
    check_kind(kind.as_ref(), "settings")?;
    Ok(SettingsDoc {
        version: version_of(version.as_ref())?,
        description: description_of(description.as_ref(), text)?,
        raw,
    })
}

/// The scalar a JSON value stands for, enough for the `kind` · `version` · `description` checks.
///
/// JSON has no NaN or infinity, so a non-finite float (`.nan`, `.inf`, `1e999`, also tagged, quoted or block-spelled)
/// arrives as `null`. A `null` whose raw source text is not exactly a plain null spelling (`null` · `Null` · `NULL` ·
/// `~` · empty — whitelist, see [`super::source_of`]) comes back as that text, trimmed — so `description: .inf` shows
/// `.inf` instead of no description (a settings file is never rewritten, so the raw text is a lossless display). This
/// also turns `!!null x` into `x`. No source (an empty value, or a field other than `description`, which passes none)
/// is a null.
fn json_scalar_node(value: &serde_json::Value, source: Option<&str>) -> Node {
    let plain_null = |source: &str| matches!(source, "null" | "Null" | "NULL" | "~" | "");
    match value {
        serde_json::Value::Null => match source {
            Some(source) if !plain_null(source) => Node::Str(source.trim().to_owned()),
            _ => Node::Null,
        },
        serde_json::Value::String(text) => Node::Str(text.clone()),
        serde_json::Value::Number(number) => match (number.as_u64(), number.as_i64()) {
            (Some(n), _) => Node::Int(n.into()),
            (None, Some(n)) => Node::Int(n.into()),
            _ => Node::Float(number.as_f64().unwrap_or(f64::NAN)),
        },
        serde_json::Value::Bool(value) => Node::Bool(*value),
        // Only its kind matters to the checks: not a string, not a number.
        serde_json::Value::Array(_) | serde_json::Value::Object(_) => Node::Seq(Vec::new()),
    }
}

pub(crate) fn not_a_map() -> ParseError {
    ParseError::new(KIND_UNKNOWN, "the document is not a map", None)
}

fn line<T>(spanned: &Spanned<T>) -> u32 {
    u32::try_from(spanned.referenced.line()).unwrap_or(u32::MAX)
}

fn check_kind(kind: Option<&Spanned<Node>>, want: &str) -> Result<(), ParseError> {
    let Some(kind) = kind else {
        return Err(ParseError::new(KIND_UNKNOWN, "no `kind`", None));
    };
    match kind.value.as_str() {
        Some(value) if value == want => Ok(()),
        Some(other @ ("preset" | "settings")) => Err(ParseError::new(
            WRONG_KIND,
            format!("`kind: {other}`, expected `kind: {want}`"),
            Some(line(kind)),
        )),
        _ => Err(ParseError::new(
            KIND_UNKNOWN,
            format!("`kind` is not `{want}`"),
            Some(line(kind)),
        )),
    }
}

fn version_of(version: Option<&Spanned<Node>>) -> Result<u32, ParseError> {
    let Some(version) = version else {
        return Err(ParseError::new(FIELD_INVALID, "no `version`", None));
    };
    match &version.value {
        Node::Int(n) => u32::try_from(*n).ok(),
        _ => None,
    }
    .ok_or_else(|| {
        ParseError::new(
            FIELD_INVALID,
            "`version` must be a whole number from 0 to 4294967295",
            Some(line(version)),
        )
    })
}

/// `description`: text, or any other scalar taken as its text as the file spells it (`1.0`, `true` — as a string field
/// reads it elsewhere, e.g. a front-matter `name`); `null` = none; a list or a map is refused.
fn description_of(
    description: Option<&Spanned<Node>>,
    text: &str,
) -> Result<Option<String>, ParseError> {
    let Some(spanned) = description else {
        return Ok(None);
    };
    let spelled = |fallback: String| {
        source_of(text, &spanned.defined)
            .map(|source| source.trim().to_owned())
            .unwrap_or(fallback)
    };
    match &spanned.value {
        Node::Null => Ok(None),
        Node::Str(text) => Ok(Some(text.clone())),
        Node::Bool(value) => Ok(Some(spelled(value.to_string()))),
        Node::Int(value) => Ok(Some(spelled(value.to_string()))),
        Node::Float(value) => Ok(Some(spelled(value.to_string()))),
        Node::Seq(_) | Node::Map(_) => Err(ParseError::new(
            FIELD_INVALID,
            "`description` must be text",
            Some(line(spanned)),
        )),
    }
}

/// Refuses text after an explicit document end (`content_after_document_end`) with this reader's multi-document code.
fn refuse_document_tail(text: &str, code: &'static str) -> Result<(), ParseError> {
    match content_after_document_end(text) {
        Some(line) => Err(ParseError::new(
            code,
            "content after the document end marker `...`",
            Some(line),
        )),
        None => Ok(()),
    }
}

/// A line problem of entry `index` of `list`: `args` = the list and index, plus `extra`.
fn entry_problem(
    list: ListName,
    index: u32,
    line: u32,
    code: &str,
    extra: serde_json::Value,
) -> Reason {
    let mut args = serde_json::Map::new();
    args.insert("list".to_owned(), json!(list.key()));
    args.insert("index".to_owned(), json!(index));
    if let serde_json::Value::Object(extra) = extra {
        args.extend(extra);
    }
    Reason {
        code: code.to_owned(),
        args: serde_json::Value::Object(args),
        line: Some(line),
    }
}

/// One entry of `presets` · `instructions` · `skills` · `settings`: the item, or `None` = unmodeled. A string or a map
/// with a string `path` (and, when present, a string `backend`) is an item; other keys of a map are ignored here and
/// kept by the writer.
fn read_item(
    list: ListName,
    index: u32,
    entry: &Spanned<Node>,
    backends: &dyn Backends,
) -> (Option<PresetItem>, Option<Reason>) {
    let line = line(entry);
    let problem = |code: &str, extra| Some(entry_problem(list, index, line, code, extra));
    let shape = match &entry.value {
        Node::Str(path) => Some((path.as_str(), None)),
        map @ Node::Map(_) => match (map.get("path"), map.get("backend")) {
            (Some(Node::Str(path)), None) => Some((path.as_str(), None)),
            (Some(Node::Str(path)), Some(Node::Str(word))) => Some((path.as_str(), Some(word))),
            _ => None,
        },
        _ => None,
    };
    let Some((path, tag)) = shape else {
        return (None, problem(ITEM_SHAPE_INVALID, json!({})));
    };
    let target = match AssetRef::parse(path) {
        Ok(target) => target,
        Err(error) => {
            let extra = json!({ "path": path, "why": error.to_string() });
            return (None, problem(ITEM_PATH_INVALID, extra));
        }
    };
    let (backend, problem) = match tag {
        None => (None, None),
        Some(word) if !list.takes_tags() => (
            None,
            problem(BACKEND_TAG_NOT_ALLOWED, json!({ "backend": word })),
        ),
        Some(word) if backends.for_word(word).is_some() => (Some(word.clone()), None),
        Some(word) => {
            return (
                None,
                problem(BACKEND_TAG_UNKNOWN, json!({ "backend": word })),
            )
        }
    };
    let item = PresetItem {
        target,
        backend,
        line,
        origin: Some(ItemOrigin { index }),
    };
    (Some(item), problem)
}

/// One entry of `folders`: a string is a folder entry (a relative one is also a line problem); anything else is
/// unmodeled (`None`).
fn read_folder(index: u32, entry: &Spanned<Node>) -> (Option<FolderRef>, Option<Reason>) {
    let line = line(entry);
    let list = ListName::Folders;
    match &entry.value {
        Node::Str(raw) => {
            let folder = FolderRef::new(raw.clone(), line, Some(ItemOrigin { index }));
            let problem = folder.normalized.is_err().then(|| {
                entry_problem(
                    list,
                    index,
                    line,
                    FOLDER_PATH_RELATIVE,
                    json!({ "path": raw }),
                )
            });
            (Some(folder), problem)
        }
        _ => (
            None,
            Some(entry_problem(
                list,
                index,
                line,
                ITEM_SHAPE_INVALID,
                json!({}),
            )),
        ),
    }
}

/// The top level of a file: its entries with the key's position and the value as `V` (spanned), in file order.
/// `None` = the document is not a map.
struct TopMap<K, V>(Option<Vec<(Spanned<K>, Spanned<V>)>>);

impl<'de, K: Deserialize<'de>, V: Deserialize<'de>> Deserialize<'de> for TopMap<K, V> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(TopMapVisitor(PhantomData))
    }
}

struct TopMapVisitor<K, V>(PhantomData<(K, V)>);

impl<'de, K: Deserialize<'de>, V: Deserialize<'de>> Visitor<'de> for TopMapVisitor<K, V> {
    type Value = TopMap<K, V>;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("a map")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        let mut entries = Vec::new();
        while let Some(entry) = map.next_entry::<Spanned<K>, Spanned<V>>()? {
            entries.push(entry);
        }
        Ok(TopMap(Some(entries)))
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
        while seq.next_element::<de::IgnoredAny>()?.is_some() {}
        Ok(TopMap(None))
    }

    fn visit_bool<E: de::Error>(self, _: bool) -> Result<Self::Value, E> {
        Ok(TopMap(None))
    }

    fn visit_i64<E: de::Error>(self, _: i64) -> Result<Self::Value, E> {
        Ok(TopMap(None))
    }

    fn visit_u64<E: de::Error>(self, _: u64) -> Result<Self::Value, E> {
        Ok(TopMap(None))
    }

    fn visit_f64<E: de::Error>(self, _: f64) -> Result<Self::Value, E> {
        Ok(TopMap(None))
    }

    fn visit_str<E: de::Error>(self, _: &str) -> Result<Self::Value, E> {
        Ok(TopMap(None))
    }

    fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
        Ok(TopMap(None))
    }
}

/// A top-level value of a preset: a list keeps each entry's position; anything else is a plain tree.
enum TopValue {
    Seq(Vec<Spanned<Node>>),
    Other(Node),
}

impl<'de> Deserialize<'de> for TopValue {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(TopValueVisitor)
    }
}

struct TopValueVisitor;

impl<'de> Visitor<'de> for TopValueVisitor {
    type Value = TopValue;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("any YAML value")
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<TopValue, A::Error> {
        let mut items = Vec::new();
        while let Some(item) = seq.next_element::<Spanned<Node>>()? {
            items.push(item);
        }
        Ok(TopValue::Seq(items))
    }

    fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<TopValue, A::Error> {
        NodeVisitor.visit_map(map).map(TopValue::Other)
    }

    fn visit_bool<E: de::Error>(self, value: bool) -> Result<TopValue, E> {
        NodeVisitor.visit_bool(value).map(TopValue::Other)
    }

    fn visit_i64<E: de::Error>(self, value: i64) -> Result<TopValue, E> {
        NodeVisitor.visit_i64(value).map(TopValue::Other)
    }

    fn visit_u64<E: de::Error>(self, value: u64) -> Result<TopValue, E> {
        NodeVisitor.visit_u64(value).map(TopValue::Other)
    }

    fn visit_i128<E: de::Error>(self, value: i128) -> Result<TopValue, E> {
        NodeVisitor.visit_i128(value).map(TopValue::Other)
    }

    fn visit_u128<E: de::Error>(self, value: u128) -> Result<TopValue, E> {
        NodeVisitor.visit_u128(value).map(TopValue::Other)
    }

    fn visit_f64<E: de::Error>(self, value: f64) -> Result<TopValue, E> {
        NodeVisitor.visit_f64(value).map(TopValue::Other)
    }

    fn visit_str<E: de::Error>(self, value: &str) -> Result<TopValue, E> {
        NodeVisitor.visit_str(value).map(TopValue::Other)
    }

    fn visit_string<E: de::Error>(self, value: String) -> Result<TopValue, E> {
        NodeVisitor.visit_string(value).map(TopValue::Other)
    }

    fn visit_unit<E: de::Error>(self) -> Result<TopValue, E> {
        NodeVisitor.visit_unit().map(TopValue::Other)
    }

    fn visit_none<E: de::Error>(self) -> Result<TopValue, E> {
        NodeVisitor.visit_none().map(TopValue::Other)
    }

    fn visit_some<D: Deserializer<'de>>(self, deserializer: D) -> Result<TopValue, D::Error> {
        TopValue::deserialize(deserializer)
    }

    fn visit_newtype_struct<D: Deserializer<'de>>(
        self,
        deserializer: D,
    ) -> Result<TopValue, D::Error> {
        TopValue::deserialize(deserializer)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::backends::fake::WordBackends;
    use crate::backends::AgentBackends;
    use crate::path::{BuiltinId, LibPath};

    /// An absolute folder path: the Windows spelling where it is absolute, else a Unix one (no OS `cfg` in this crate).
    fn abs_folder() -> &'static str {
        let windows = r"D:\Rules";
        if std::path::Path::new(windows).is_absolute() {
            windows
        } else {
            "/srv/rules"
        }
    }

    fn two() -> WordBackends {
        WordBackends::new(&["claude", "codex"])
    }

    fn item(text: &str, backend: Option<&str>, line: u32, index: u32) -> PresetItem {
        PresetItem {
            target: AssetRef::parse(text).unwrap(),
            backend: backend.map(str::to_owned),
            line,
            origin: Some(ItemOrigin { index }),
        }
    }

    fn codes(doc: &PresetDoc) -> Vec<(&str, Option<u32>)> {
        doc.line_problems
            .iter()
            .map(|reason| (reason.code.as_str(), reason.line))
            .collect()
    }

    fn preset(body: &str) -> String {
        format!("kind: preset\nversion: 1\n{body}")
    }

    // ── kind sniff ─────────────────────────────────────────────────────────────

    #[test]
    fn sniff_reads_the_kind_line_even_in_broken_yaml() {
        let cases: &[(&str, Sniff)] = &[
            ("kind: preset\nversion: 1\n", Sniff::Preset),
            ("kind: settings\n", Sniff::Settings),
            ("version: 1\nkind: 'preset'   # a comment\n", Sniff::Preset),
            ("kind: \"settings\"\n", Sniff::Settings),
            ("\u{feff}kind: preset\n", Sniff::Preset),
            ("kind: preset\r\nversion: [\r\n", Sniff::Preset),
            ("kind: preset\n  : : broken [\n\tx", Sniff::Preset),
            ("kind: presets\n", Sniff::Unknown("presets".to_owned())),
            ("kind:\n", Sniff::Unknown(String::new())),
            ("kind: x#y\n", Sniff::Unknown("x#y".to_owned())),
            ("  kind: preset\n", Sniff::Absent),
            ("kinds: preset\n", Sniff::Absent),
            ("kind:preset\n", Sniff::Absent),
            ("{kind: preset}\n", Sniff::Absent),
            ("version: 1\n", Sniff::Absent),
            ("", Sniff::Absent),
        ];
        for (text, expected) in cases {
            assert_eq!(&sniff_kind(text), expected, "{text:?}");
        }
    }

    // ── preset: whole-file outcomes ────────────────────────────────────────────

    #[test]
    fn a_whole_preset_reads_every_list_with_lines_and_origins() {
        let folder = abs_folder();
        let text = format!(
            "kind: preset\nversion: 3\ndescription: team rules\npresets:\n- base.yaml\n- engram:/default\n\
             instructions:\n- rules/a.md\n- path: rules/b.md\n  backend: codex\n  note: kept\nskills:\n- {{path: \
             skills/x, backend: Claude}}\nsettings:\n- settings/s.yaml\nfolders:\n- '{folder}'\nother: 1\n"
        );
        let doc = parse_preset(&text, &two()).unwrap();
        assert_eq!(doc.version, 3);
        assert_eq!(doc.description.as_deref(), Some("team rules"));
        assert_eq!(
            doc.presets,
            vec![
                item("base.yaml", None, 5, 0),
                item("engram:/default", None, 6, 1)
            ]
        );
        assert_eq!(
            doc.presets[1].target,
            AssetRef::BuiltIn(BuiltinId::DefaultPreset)
        );
        assert_eq!(
            doc.instructions,
            vec![
                item("rules/a.md", None, 8, 0),
                item("rules/b.md", Some("codex"), 9, 1)
            ]
        );
        // The tag word is kept as spelled; the registry matched it case-insensitively.
        assert_eq!(doc.skills, vec![item("skills/x", Some("Claude"), 13, 0)]);
        assert_eq!(doc.settings, vec![item("settings/s.yaml", None, 15, 0)]);
        assert_eq!(doc.folders.len(), 1);
        assert_eq!(doc.folders[0].raw, folder);
        assert!(doc.folders[0].normalized.is_ok());
        assert_eq!(
            (doc.folders[0].line, doc.folders[0].origin),
            (17, Some(ItemOrigin { index: 0 }))
        );
        assert!(doc.unmodeled.is_empty() && doc.line_problems.is_empty());
    }

    #[test]
    fn missing_or_null_lists_are_empty_and_description_is_optional() {
        let doc = parse_preset("kind: preset\nversion: 0\nskills:\nfolders: ~\n", &two()).unwrap();
        assert_eq!(doc.version, 0);
        assert_eq!(doc.description, None);
        assert!(doc.presets.is_empty() && doc.skills.is_empty() && doc.folders.is_empty());
        let doc = parse_preset("kind: preset\nversion: 1\ndescription: null\n", &two()).unwrap();
        assert_eq!(doc.description, None);
    }

    #[test]
    fn whole_file_refusals_carry_their_code_and_line() {
        let cases: &[(&str, &str, Option<u32>)] = &[
            (
                "kind: preset\nversion: 1\ninstructions: [a.md\n",
                "yaml-syntax",
                Some(3),
            ),
            (
                "kind: preset\nversion: 1\nversion: 2\n",
                "yaml-syntax",
                Some(3),
            ),
            (
                "kind: preset\nversion: 1\n---\nkind: preset\nversion: 1\n",
                "preset-multi-document",
                Some(4),
            ),
            ("kind: settings\nversion: 1\n", "wrong-kind", Some(1)),
            ("kind: other\nversion: 1\n", "kind-unknown", Some(1)),
            ("version: 1\n", "kind-unknown", None),
            ("- kind\n- preset\n", "kind-unknown", None),
            ("plain text\n", "kind-unknown", None),
            ("", "kind-unknown", None),
            ("kind: preset\n", "field-invalid", None),
            ("kind: preset\nversion: one\n", "field-invalid", Some(2)),
            ("kind: preset\nversion: -1\n", "field-invalid", Some(2)),
            (
                "kind: preset\nversion: 4294967296\n",
                "field-invalid",
                Some(2),
            ),
            ("kind: preset\nversion: 1.0\n", "field-invalid", Some(2)),
            (
                "kind: preset\nversion: 1\ndescription: [a]\n",
                "field-invalid",
                Some(3),
            ),
            (
                "kind: preset\nversion: 1\n\nskills: rules/a.md\n",
                "field-invalid",
                Some(4),
            ),
            (
                "kind: preset\nversion: 1\nfolders: {a: 1}\n",
                "field-invalid",
                Some(3),
            ),
        ];
        for (text, code, line) in cases {
            let error = parse_preset(text, &two()).expect_err(text);
            assert_eq!(
                (error.code, error.line),
                (*code, *line),
                "{text:?}: {error}"
            );
        }
    }

    #[test]
    fn a_parse_error_becomes_a_reason_with_its_message() {
        let error = parse_preset("kind: preset\nversion: 1\nversion: 2\n", &two()).unwrap_err();
        let reason = error.to_reason();
        assert_eq!(
            (reason.code.as_str(), reason.line),
            ("yaml-syntax", Some(3))
        );
        assert_eq!(reason.args, json!({ "message": error.message }));
        assert!(error.message.contains("duplicate"), "{}", error.message);
    }

    // ── preset: entries ────────────────────────────────────────────────────────

    #[test]
    fn both_item_shapes_read_and_a_map_may_hold_other_keys() {
        let doc = parse_preset(
            &preset(
                "instructions:\n- a.md\n- path: b.md\n- {path: c.md, backend: codex, x: [1]}\n",
            ),
            &two(),
        )
        .unwrap();
        assert_eq!(
            doc.instructions,
            vec![
                item("a.md", None, 4, 0),
                item("b.md", None, 5, 1),
                item("c.md", Some("codex"), 6, 2)
            ]
        );
        assert!(doc.line_problems.is_empty());
    }

    #[test]
    fn a_tag_in_presets_or_settings_is_a_line_problem_and_the_entry_stays_modeled() {
        let doc = parse_preset(
            &preset(
                "presets:\n- {path: p.yaml, backend: codex}\nsettings:\n- path: s.yaml\n  backend: claude\n",
            ),
            &two(),
        )
        .unwrap();
        assert_eq!(doc.presets, vec![item("p.yaml", None, 4, 0)]);
        assert_eq!(doc.settings, vec![item("s.yaml", None, 6, 0)]);
        assert_eq!(
            codes(&doc),
            vec![
                ("backend-tag-not-allowed", Some(4)),
                ("backend-tag-not-allowed", Some(6))
            ]
        );
        assert_eq!(
            doc.line_problems[0].args,
            json!({ "list": "presets", "index": 0, "backend": "codex" })
        );
        assert!(doc.unmodeled.is_empty());
    }

    #[test]
    fn an_unknown_tag_word_is_a_line_problem_and_an_unmodeled_entry() {
        let text = preset("skills:\n- a\n- {path: b, backend: fakebe}\n- c\n");
        let doc = parse_preset(&text, &two()).unwrap();
        assert_eq!(
            doc.skills,
            vec![item("a", None, 4, 0), item("c", None, 6, 2)]
        );
        assert_eq!(
            doc.unmodeled,
            vec![Unmodeled {
                list: ListName::Skills,
                index: 1,
                line: 5
            }]
        );
        assert_eq!(codes(&doc), vec![("backend-tag-unknown", Some(5))]);
        assert_eq!(
            doc.line_problems[0].args,
            json!({ "list": "skills", "index": 1, "backend": "fakebe" })
        );

        // A backend the registry learns later is accepted with no change here (TRD §1-1).
        let three = WordBackends::new(&["claude", "codex", "fakebe"]);
        let doc = parse_preset(&text, &three).unwrap();
        assert_eq!(doc.skills[1], item("b", Some("fakebe"), 5, 1));
        assert!(doc.unmodeled.is_empty() && doc.line_problems.is_empty());
    }

    #[test]
    fn the_production_registry_knows_today_s_words_only() {
        let doc = parse_preset(
            &preset(
                "skills:\n- {path: a, backend: CODEX}\n- {path: b, backend: claude}\n- {path: c, backend: fakebe}\n",
            ),
            &AgentBackends,
        )
        .unwrap();
        assert_eq!(doc.skills.len(), 2);
        assert_eq!(codes(&doc), vec![("backend-tag-unknown", Some(6))]);
    }

    #[test]
    fn entries_the_reader_cannot_model_are_line_problems_and_unmodeled() {
        let text = preset(
            "instructions:\n- 7\n- true\n- ~\n- [a.md]\n- {backend: codex}\n- {path: 3}\n- {path: a.md, backend: 1}\n\
             - {path: a.md, backend: null}\n- /abs.md\n- a\\b.md\n- engram:/nope\n- ok.md\n",
        );
        let doc = parse_preset(&text, &two()).unwrap();
        assert_eq!(doc.instructions, vec![item("ok.md", None, 15, 11)]);
        let unmodeled: Vec<(u32, u32)> = doc.unmodeled.iter().map(|u| (u.index, u.line)).collect();
        assert_eq!(unmodeled, (0..11).map(|i| (i, i + 4)).collect::<Vec<_>>());
        assert!(doc
            .unmodeled
            .iter()
            .all(|u| u.list == ListName::Instructions));
        let expected: Vec<(&str, Option<u32>)> = (4..12)
            .map(|line| ("item-shape-invalid", Some(line)))
            .chain((12..15).map(|line| ("item-path-invalid", Some(line))))
            .collect();
        assert_eq!(codes(&doc), expected);
        assert_eq!(
            doc.line_problems[8].args,
            json!({ "list": "instructions", "index": 8, "path": "/abs.md", "why": "path starts with '/'" })
        );
    }

    #[test]
    fn yaml_1_1_scalars_read_as_text_under_strict_booleans() {
        let doc = parse_preset(
            &preset(
                "description: yes\ninstructions:\n- yes\n- on\n- No\n- y\n- 'true'\n- true\n- TRUE\n- 1.0\n- 012\n",
            ),
            &two(),
        )
        .unwrap();
        assert_eq!(doc.description.as_deref(), Some("yes"));
        let spellings: Vec<String> = doc
            .instructions
            .iter()
            .map(|i| i.target.spelling())
            .collect();
        assert_eq!(spellings, ["yes", "on", "No", "y", "true"]);
        // `true` / `TRUE` are booleans and `1.0` / `012` numbers — not text, so not a path.
        let unmodeled: Vec<u32> = doc.unmodeled.iter().map(|u| u.index).collect();
        assert_eq!(unmodeled, [5, 6, 7, 8]);
    }

    #[test]
    fn a_nan_in_a_key_engram_does_not_model_reads() {
        let doc = parse_preset(
            &preset("extra: .nan\nmore: [-.inf, .NaN]\ninstructions:\n- a.md\n"),
            &two(),
        )
        .unwrap();
        assert_eq!(doc.instructions, vec![item("a.md", None, 6, 0)]);
    }

    #[test]
    fn folders_take_strings_and_flag_relative_ones() {
        let folder = abs_folder();
        let text = preset(&format!(
            "folders:\n- '{folder}'\n- rules/team\n- team\n- 5\n- {{path: x}}\n"
        ));
        let doc = parse_preset(&text, &two()).unwrap();
        let raws: Vec<&str> = doc.folders.iter().map(|f| f.raw.as_str()).collect();
        assert_eq!(raws, [folder, "rules/team", "team"]);
        assert!(doc.folders[0].normalized.is_ok());
        assert!(doc.folders[1].normalized.is_err() && doc.folders[2].normalized.is_err());
        assert_eq!(
            codes(&doc),
            vec![
                ("folder-path-relative", Some(5)),
                ("folder-path-relative", Some(6)),
                ("item-shape-invalid", Some(7)),
                ("item-shape-invalid", Some(8)),
            ]
        );
        assert_eq!(
            doc.line_problems[0].args,
            json!({ "list": "folders", "index": 1, "path": "rules/team" })
        );
        let unmodeled: Vec<(ListName, u32)> =
            doc.unmodeled.iter().map(|u| (u.list, u.index)).collect();
        assert_eq!(unmodeled, [(ListName::Folders, 3), (ListName::Folders, 4)]);
    }

    #[test]
    fn mixed_zero_and_two_space_list_indentation_reads_alike() {
        let doc = parse_preset(
            &preset("instructions:\n- a.md\n- b.md\nskills:\n  - s1\n  - s2\n"),
            &two(),
        )
        .unwrap();
        assert_eq!(
            doc.instructions,
            vec![item("a.md", None, 4, 0), item("b.md", None, 5, 1)]
        );
        assert_eq!(
            doc.skills,
            vec![item("s1", None, 7, 0), item("s2", None, 8, 1)]
        );
    }

    #[test]
    fn explicit_tags_keep_the_tagged_value() {
        let doc = parse_preset(
            &preset("description: !!str 12\ninstructions:\n- !!str 7\n- !local a.md\n"),
            &two(),
        )
        .unwrap();
        assert_eq!(doc.description.as_deref(), Some("12"));
        assert_eq!(
            doc.instructions,
            vec![item("7", None, 5, 0), item("a.md", None, 6, 1)]
        );
        assert_eq!(
            doc.instructions[0].target,
            AssetRef::Library(LibPath::parse("7").unwrap())
        );
    }

    #[test]
    fn a_bom_and_crlf_read_like_plain_lf() {
        let doc = parse_preset(
            "\u{feff}kind: preset\r\nversion: 1\r\ninstructions:\r\n- a.md\r\n",
            &two(),
        )
        .unwrap();
        assert_eq!(doc.instructions, vec![item("a.md", None, 4, 0)]);
    }

    #[test]
    fn aliases_expand_into_list_entries() {
        let doc = parse_preset(
            &preset("base: &b rules/a.md\ninstructions:\n- *b\n- rules/b.md\n"),
            &two(),
        )
        .unwrap();
        let spellings: Vec<String> = doc
            .instructions
            .iter()
            .map(|i| i.target.spelling())
            .collect();
        assert_eq!(spellings, ["rules/a.md", "rules/b.md"]);
    }

    #[test]
    fn a_scalar_description_is_taken_as_its_text_as_spelled() {
        let cases = [
            ("description: 1.0\n", "1.0"),
            ("description: true\n", "true"),
            ("description: 0x1F\n", "0x1F"),
            ("description: 12   # a comment\n", "12"),
            ("d: &x 7\ndescription: *x\n", "7"),
            ("description: !!binary aGVsbG8=\n", "aGVsbG8="),
        ];
        for (body, expected) in cases {
            let doc = parse_preset(&preset(body), &two()).unwrap();
            assert_eq!(doc.description.as_deref(), Some(expected), "{body:?}");
        }
        let doc = parse_settings("kind: settings\nversion: 1\ndescription: 2.50\n").unwrap();
        assert_eq!(doc.description.as_deref(), Some("2.50"));
    }

    #[test]
    fn content_after_an_explicit_document_end_is_refused() {
        for tail in [
            "@todo",
            "`x`",
            "%TODO",
            "'til the end",
            "instructions: [x]",
            "--- ",
            "  a: 1",
        ] {
            let text = format!("kind: preset\nversion: 1\n...\n\n# note\n{tail}\n");
            let error = parse_preset(&text, &two()).expect_err(&text);
            assert_eq!(
                (error.code, error.line),
                ("preset-multi-document", Some(6)),
                "{tail:?}: {error}"
            );
        }
        // A bare `\r` is a line break to YAML, and content on the `...` line itself counts.
        for (text, line) in [
            ("kind: preset\nversion: 1\n...\r@todo\n", 4),
            ("kind: preset\nversion: 1\r...\r\n@todo\n", 4),
            ("kind: preset\nversion: 1\n...\n# c\r@todo\n", 5),
        ] {
            let error = parse_preset(text, &two()).expect_err(text);
            assert_eq!(
                (error.code, error.line),
                ("preset-multi-document", Some(line)),
                "{text:?}"
            );
            let error = parse_settings(&text.replace("preset", "settings")).expect_err(text);
            assert_eq!(
                (error.code, error.line),
                ("yaml-syntax", Some(line)),
                "{text:?}"
            );
        }
        for text in [
            "kind: preset\nversion: 1\n...\n",
            "kind: preset\nversion: 1\n... # end\n# a comment\n\n   \n\t# indented comment\n",
            "kind: preset\nversion: 1\n...\r\n# c\r\n",
            "kind: preset\nversion: 1\n...\r# c\r",
            "kind: preset\nversion: 1\ndescription: \"...\"\n",
            "kind: preset\nversion: 1\nx: |\n  ...\n  @todo\n",
        ] {
            assert!(parse_preset(text, &two()).is_ok(), "{text:?}");
        }
        // Content on the `...` line itself: serde-saphyr refuses it on its own (and the helper would too).
        for text in [
            "kind: preset\nversion: 1\n... @todo\n",
            "kind: preset\nversion: 1\n...\t'til\n",
        ] {
            assert_eq!(
                parse_preset(text, &two()).map(|_| ()).map_err(|e| e.line),
                Err(Some(3)),
                "{text:?}"
            );
            assert_eq!(content_after_document_end(text), Some(3), "{text:?}");
        }
    }

    #[test]
    fn a_nul_after_three_dots_ends_the_document_and_counts_as_content() {
        for text in [
            "kind: preset\nversion: 1\n...\0@todo\n",
            "kind: preset\nversion: 1\n...\0\n@todo keep this\n",
        ] {
            assert_eq!(content_after_document_end(text), Some(3), "{text:?}");
            assert!(parse_preset(text, &two()).is_err(), "{text:?}");
            assert!(
                parse_settings(&text.replace("preset", "settings")).is_err(),
                "{text:?}"
            );
        }
    }

    #[test]
    fn three_dots_followed_by_other_text_is_a_plain_key_not_a_document_end() {
        let doc = parse_preset(
            "kind: preset\nversion: 1\n...#owner: team\n...x: 1\nskills: [a]\n",
            &two(),
        )
        .unwrap();
        assert_eq!(doc.skills.len(), 1);
        let doc = parse_settings("kind: settings\nversion: 1\n...#owner: team\n").unwrap();
        assert_eq!(doc.raw[0].name, "...#owner");
        assert_eq!(doc.raw[0].value, json!("team"));
    }

    #[test]
    fn a_settings_description_keeps_a_non_finite_spelling_and_null_stays_none() {
        // (value, settings description, preset description)
        for (value, settings, preset) in [
            (".inf", Some(".inf"), Some(".inf")),
            ("-.Inf", Some("-.Inf"), Some("-.Inf")),
            (".nan", Some(".nan"), Some(".nan")),
            ("1e999", Some("1e999"), Some("1e999")),
            ("!!float '.inf'", Some("'.inf'"), Some("'.inf'")),
            // A block scalar's raw span is its content (the `|-` header is not part of it), trimmed.
            ("!!float |-\n  .inf", Some(".inf"), Some(".inf")),
            ("null", None, None),
            ("Null", None, None),
            ("~", None, None),
            ("", None, None),
            // The settings rule shows any `null` whose raw text is not a plain null spelling as that text.
            ("!!null x", Some("x"), None),
        ] {
            let text = format!("kind: settings\nversion: 1\ndescription: {value}\n");
            let doc = parse_settings(&text).unwrap();
            assert_eq!(doc.description.as_deref(), settings, "{value:?}");
            let text = format!("kind: preset\nversion: 1\ndescription: {value}\n");
            let doc = parse_preset(&text, &two()).unwrap();
            assert_eq!(doc.description.as_deref(), preset, "preset {value:?}");
        }
        let doc = parse_settings("kind: settings\nversion: 1\n").unwrap();
        assert_eq!(doc.description, None);
    }

    #[test]
    fn sniff_breaks_lines_as_yaml_does() {
        assert_eq!(sniff_kind("version: 1\rkind: preset\r"), Sniff::Preset);
        assert_eq!(sniff_kind("kind:\tsettings\n"), Sniff::Settings);
    }

    // ── settings ───────────────────────────────────────────────────────────────

    #[test]
    fn settings_keep_every_other_key_in_file_order_with_its_line() {
        let doc = parse_settings(
            "kind: settings\nversion: 2\ndescription: d\ncodex:\n- model=o3\nclaude:\n  model: opus\n  n: .nan\nfakebe: 1\n7: x\n",
        )
        .unwrap();
        assert_eq!(doc.version, 2);
        assert_eq!(doc.description.as_deref(), Some("d"));
        let raw: Vec<(&str, &serde_json::Value, u32)> = doc
            .raw
            .iter()
            .map(|f| (f.name.as_str(), &f.value, f.line))
            .collect();
        assert_eq!(
            raw,
            vec![
                ("codex", &json!(["model=o3"]), 4),
                ("claude", &json!({"model": "opus", "n": null}), 6),
                ("fakebe", &json!(1), 9),
                ("7", &json!("x"), 10),
            ]
        );
    }

    #[test]
    fn settings_refusals() {
        let cases: &[(&str, &str, Option<u32>)] = &[
            ("kind: preset\nversion: 1\n", "wrong-kind", Some(1)),
            ("version: 1\n", "kind-unknown", None),
            (
                "kind: !!null settings\nversion: 1\n",
                "kind-unknown",
                Some(1),
            ),
            (
                "kind: settings\nversion: !!null 1\n",
                "field-invalid",
                Some(2),
            ),
            ("kind: settings\n", "field-invalid", None),
            ("kind: settings\nversion: x\n", "field-invalid", Some(2)),
            (
                "kind: settings\nversion: 1\ndescription: {a: 1}\n",
                "field-invalid",
                Some(3),
            ),
            (
                "kind: settings\nversion: 1\n...\n@todo\n",
                "yaml-syntax",
                Some(4),
            ),
            (
                "kind: settings\nversion: 1\nclaude: {a: 1, a: 2}\n",
                "yaml-syntax",
                Some(3),
            ),
            (
                "kind: settings\nversion: 1\n---\nx: 1\n",
                "yaml-syntax",
                Some(4),
            ),
            (
                "kind: settings\nversion: 1\n[a]: x\n",
                "yaml-syntax",
                Some(3),
            ),
        ];
        for (text, code, line) in cases {
            let error = parse_settings(text).expect_err(text);
            assert_eq!(
                (error.code, error.line),
                (*code, *line),
                "{text:?}: {error}"
            );
        }
    }
}
