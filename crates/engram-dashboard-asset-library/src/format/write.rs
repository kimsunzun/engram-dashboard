//! The whole-file preset writer (ADR-0358): parse the file into a [`Node`] tree, replace the five list nodes, serialize
//! the tree with serde-saphyr, restore the line ending and BOM, and check that the returned text reads back to the same
//! tree.
//!
//! What a rewrite keeps: every key other than the five lists, known or not, with its value and position; inside a list,
//! each item's own node (unknown keys of an item map included) and every entry the reader could not model. What it
//! loses, by decision: comments, blank lines, quoting and flow style, key spacing, anchors and merge keys (expanded —
//! values equal), explicit tags (a `!!binary` value stays its base64 text), the sign of a NaN; indentation becomes two
//! spaces.
//!
//! Item identity never depends on position or tag: an item with an origin starts from that original entry, an item
//! without one from the first unused original modeled entry of the same spelling (TRD S25 step-7 §3-4 write).
//!
//! Known read-time losses — the reader changes these values before the tree exists, so neither the number guard nor the
//! round-trip check sees them, and a rewrite writes the changed value:
//! - a plain `012` (a leading-zero decimal) is read as the float `12.0` and written `12.0`;
//! - an integer spelling outside YAML 1.2's decimal form is read as its value and written in decimal: `1_000` → `1000`,
//!   `0b101` → `5`, `-0x10` → `-16`;
//! - a plain hex, octal or binary integer literal beyond 64 bits (`0xFFFFFFFFFFFFFFFFFF`), and a decimal one with `_`
//!   beyond 64 bits, is read as a string and written quoted;
//! - a merge key (`<<`) is expanded with the merged entries after the mapping's own keys, so a rewrite reorders them.
//!
//! Refused instead of lost: an integral float of magnitude ≥ 2^63, a number beyond the `f64` range read as an infinity
//! (both possibly an integer literal the reader already changed — premise 39), a non-zero number below the `f64` range
//! read as zero (`1e-400`), and content after an explicit `...`.

// ADR-0358
// ADR-0329

use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::fmt;

use engram_dashboard_agent::launch::Reason;
use serde::de::{self, Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};
use serde_json::json;
use serde_saphyr::Spanned;

use super::read::{
    not_a_map, parse_preset, ListName, PresetDoc, KIND_UNKNOWN, MULTI_DOCUMENT, WRONG_KIND,
};
use super::tree::{join_path, key_segment, Node, NodeVisitor};
use super::{options, source_of, ParseError};
use crate::backends::Backends;
use crate::path::{ItemOrigin, PresetItem};

/// The new content of the five lists.
///
/// Caller contract: the lists and `drop_unmodeled` come from [`parse_preset`] on the same text passed to
/// [`rewrite_lists`], list by list; an item the caller keeps carries the `origin` it was read with. An item without an
/// origin is matched to an original entry only by an unchanged spelling — so a kept item whose path changes must carry
/// its origin, or it is written as a new item and loses its map's other keys.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PresetLists {
    pub presets: Vec<PresetItem>,
    pub instructions: Vec<PresetItem>,
    pub skills: Vec<PresetItem>,
    pub settings: Vec<PresetItem>,
    pub folders: Vec<FolderItem>,
    /// Unmodeled entries ([`super::read::Unmodeled`]) the caller removes on purpose, as `(list, index)`; every other
    /// unmodeled entry is kept.
    pub drop_unmodeled: Vec<(ListName, u32)>,
}

/// One folder entry to write: `path` as it is to be stored. The caller passes an untouched entry's original spelling
/// (`FolderRef::raw`) so it is not rewritten, and the normalized form for a new or changed one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FolderItem {
    pub path: String,
    pub origin: Option<ItemOrigin>,
}

/// Why nothing was written.
#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum WriteError {
    /// The file does not read as a preset (the reader's refusal).
    #[error("{0}")]
    Parse(ParseError),
    /// The file is not a map with `kind: preset` — the reader's `kind-unknown` / `wrong-kind` refusal, with its line.
    #[error("not a preset file: {0}")]
    NotAPreset(ParseError),
    /// More than one YAML document, or content after an explicit `...` — the refusal with its line.
    #[error("{0}")]
    MultipleDocuments(ParseError),
    /// A number at `path` the writer cannot keep exactly: a float with an integral value of magnitude ≥ 2^63, an
    /// infinity not spelled `.inf`, or a zero whose spelling holds a non-zero digit (`1e-400`) — possibly a literal the
    /// reader already rounded, overflowed or underflowed (premise 39), so writing it would change the file's value.
    #[error("the number at {path} is outside the range the writer keeps exactly")]
    NumberOutOfRange { path: String },
    /// The returned text does not read back to the intended tree. `path` = the first node that differs; empty when the
    /// text does not parse back at all (`reread` = that parse error; its line is a line of the text that was not
    /// written, not of the file) or the serializer refused the tree (`reread` = `None`).
    #[error("the rewritten file does not read back the same (at {path})")]
    RoundTrip {
        path: String,
        reread: Option<ParseError>,
    },
}

impl WriteError {
    pub fn code(&self) -> &'static str {
        match self {
            WriteError::Parse(error)
            | WriteError::NotAPreset(error)
            | WriteError::MultipleDocuments(error) => error.code,
            WriteError::NumberOutOfRange { .. } => "preset-write-number-range",
            WriteError::RoundTrip { .. } => "preset-write-mismatch",
        }
    }

    /// The refusal as a reason: `args.path` = the node path for the two writer refusals that name one; a read-back
    /// parse failure adds its `message` and `written_line` (a line of the unwritten output — `Reason::line` stays a
    /// line of the file, so it is `None` there).
    pub fn to_reason(&self) -> Reason {
        let code = self.code().to_owned();
        match self {
            WriteError::Parse(error)
            | WriteError::NotAPreset(error)
            | WriteError::MultipleDocuments(error) => error.to_reason(),
            WriteError::NumberOutOfRange { path } => Reason {
                code,
                args: json!({ "path": path }),
                line: None,
            },
            WriteError::RoundTrip { path, reread } => Reason {
                code,
                args: match reread {
                    Some(error) => json!({
                        "path": path,
                        "message": error.message,
                        "written_line": error.line,
                    }),
                    None => json!({ "path": path }),
                },
                line: None,
            },
        }
    }
}

/// `text` with its five lists replaced by `lists` (see the caller contract on [`PresetLists`]), or why it cannot be
/// written. `backends` decides, as for [`parse_preset`], which original entries are modeled (an entry with a tag word
/// no backend knows is not). Pure.
///
/// An absent list key whose new list is empty stays absent; absent keys with entries are added, in [`ListName::ALL`]
/// order, right after the last list key the file has (at the end when it has none). A list key holding `null` whose
/// new list is empty keeps its `null`; any other emptied list is written as `[]`.
pub fn rewrite_lists(
    text: &str,
    lists: &PresetLists,
    backends: &dyn Backends,
) -> Result<String, WriteError> {
    let (bom, body) = match text.strip_prefix('\u{feff}') {
        Some(rest) => (true, rest),
        None => (false, text),
    };
    // The first line break decides (`\r\n` → CRLF; `\n` or a bare `\r` → LF).
    let crlf = body
        .find(['\r', '\n'])
        .is_some_and(|at| body[at..].starts_with("\r\n"));

    let mut tree = parse_tree(body)?;
    let doc = parse_preset(body, backends).map_err(|error| match error.code {
        KIND_UNKNOWN | WRONG_KIND => WriteError::NotAPreset(error),
        MULTI_DOCUMENT => WriteError::MultipleDocuments(error),
        _ => WriteError::Parse(error),
    })?;
    if let Some(path) = out_of_range(&tree, &mut Vec::new()) {
        return Err(WriteError::NumberOutOfRange { path });
    }
    if any_float(&tree, &|value| value.is_infinite() || value == 0.0) {
        if let Some(path) = changed_by_reading(body)? {
            return Err(WriteError::NumberOutOfRange { path });
        }
    }
    let Node::Map(entries) = &mut tree else {
        return Err(WriteError::NotAPreset(not_a_map()));
    };
    replace_lists(entries, &doc, lists);

    let written = serialize(&tree)?;
    let mut out = String::with_capacity(written.len() + 8);
    if bom {
        out.push('\u{feff}');
    }
    if crlf {
        out.push_str(&written.replace('\n', "\r\n"));
    } else {
        out.push_str(&written);
    }
    let out = mangle_for_test(out);

    let returned = out.strip_prefix('\u{feff}').unwrap_or(&out);
    let reread = parse_tree(returned).map_err(|error| WriteError::RoundTrip {
        path: String::new(),
        reread: Some(match error {
            WriteError::Parse(error) | WriteError::MultipleDocuments(error) => error,
            other => ParseError::new(other.code(), other.to_string(), None),
        }),
    })?;
    if let Some(path) = tree.first_difference(&reread) {
        return Err(WriteError::RoundTrip { path, reread: None });
    }
    Ok(out)
}

fn parse_tree(text: &str) -> Result<Node, WriteError> {
    serde_saphyr::from_str_with_options::<Node>(text, options()).map_err(|error| {
        let error = ParseError::from_yaml(&error, MULTI_DOCUMENT);
        if error.code == MULTI_DOCUMENT {
            WriteError::MultipleDocuments(error)
        } else {
            WriteError::Parse(error)
        }
    })
}

fn serialize(tree: &Node) -> Result<String, WriteError> {
    serde_saphyr::to_string_with_options(tree, serde_saphyr::ser_options! { indent_step: 2 })
        // The serializer refused the tree: nothing is written, the same outcome as a mismatch.
        .map_err(|_| WriteError::RoundTrip {
            path: String::new(),
            reread: None,
        })
}

#[cfg(test)]
thread_local! {
    /// Test hook: rewrites the text to be returned before the round-trip check reads it.
    pub(super) static MANGLE: std::cell::Cell<Option<fn(String) -> String>> = const { std::cell::Cell::new(None) };
}

#[cfg(test)]
fn mangle_for_test(out: String) -> String {
    match MANGLE.with(std::cell::Cell::get) {
        Some(mangle) => mangle(out),
        None => out,
    }
}

#[cfg(not(test))]
fn mangle_for_test(out: String) -> String {
    out
}

/// 2^63 as `f64` (exact).
const TWO_POW_63: f64 = 9_223_372_036_854_775_808.0;

/// The path of the first float, key or value, with an integral value of magnitude ≥ 2^63 (premise 39 — the reader
/// turns an integer literal beyond 64 bits into such a float before the tree exists, and a 21-digit literal cannot be
/// told from a written `1e20`, so both are refused).
fn out_of_range(node: &Node, path: &mut Vec<String>) -> Option<String> {
    match node {
        Node::Float(value)
            if value.is_finite() && value.fract() == 0.0 && value.abs() >= TWO_POW_63 =>
        {
            Some(join_path(path))
        }
        Node::Seq(items) => items.iter().enumerate().find_map(|(index, item)| {
            path.push(index.to_string());
            let found = out_of_range(item, path);
            path.pop();
            found
        }),
        Node::Map(entries) => entries.iter().find_map(|(key, value)| {
            path.push(key_segment(key));
            let found = out_of_range(key, path).or_else(|| out_of_range(value, path));
            path.pop();
            found
        }),
        _ => None,
    }
}

/// Whether any float of `node`, key or value, satisfies `test`.
fn any_float(node: &Node, test: &dyn Fn(f64) -> bool) -> bool {
    match node {
        Node::Float(value) => test(*value),
        Node::Seq(items) => items.iter().any(|item| any_float(item, test)),
        Node::Map(entries) => entries
            .iter()
            .any(|(key, value)| any_float(key, test) || any_float(value, test)),
        _ => false,
    }
}

/// The path of the first infinite or zero float whose raw source text (the whole span — an alias judged by the node it
/// names) is not one of the plain spellings below — whitelist, see [`super::source_of`]:
/// - infinity: `[-+]?.inf`, with `inf` / `Inf` / `INF` — anything else (`1e999`, a 400-digit integer) is a number
///   beyond the range read as an infinity (`parse_scalars.rs` `try_parse_float_incl_overflow`);
/// - zero: `[-+]?` digits with at most one `.`, at least one digit, all of them `0`, then an optional `[eE][-+]?\d+`
///   (`0.0`, `-0.0`, `0e5`, `.0`, `0.000e-400`) — anything else (`1e-400`) may be a number below the range read as zero.
///
/// Every other spelling is refused — quoted, tagged-quoted, and a block scalar whose span keeps its trailing line break
/// (`|- # e`, `|2-`). That refuses a real zero written unusually (e.g. `!!float |2-` with `0`) — a safe false
/// positive. A block scalar at the very end of a file with no final line break spans exactly its content, so a plain
/// `.inf` or `0` there is kept — its value really is that.
///
/// A positioned parse of its own, run only when the tree holds an infinity or a zero.
fn changed_by_reading(text: &str) -> Result<Option<String>, WriteError> {
    let scan: Spanned<Scan> = serde_saphyr::from_str_with_options(text, options())
        .map_err(|error| WriteError::Parse(ParseError::from_yaml(&error, MULTI_DOCUMENT)))?;
    Ok(scan_changed(&scan, text, &mut Vec::new()))
}

fn scan_changed(scan: &Spanned<Scan>, text: &str, path: &mut Vec<String>) -> Option<String> {
    match &scan.value {
        Scan::Float(value) if value.is_infinite() || *value == 0.0 => {
            let kept = source_of(text, &scan.defined).is_some_and(|source| {
                if value.is_infinite() {
                    is_plain_infinity(source)
                } else {
                    is_plain_zero(source)
                }
            });
            (!kept).then(|| join_path(path))
        }
        Scan::Seq(items) => items.iter().enumerate().find_map(|(index, item)| {
            path.push(index.to_string());
            let found = scan_changed(item, text, path);
            path.pop();
            found
        }),
        Scan::Map(entries) => entries.iter().find_map(|(key, value)| {
            path.push(match &key.value {
                Scan::Scalar(node) => key_segment(node),
                Scan::Float(value) => key_segment(&Node::Float(*value)),
                Scan::Seq(_) | Scan::Map(_) => "?".to_owned(),
            });
            let found = scan_changed(key, text, path).or_else(|| scan_changed(value, text, path));
            path.pop();
            found
        }),
        _ => None,
    }
}

/// `[-+]?\.(inf|Inf|INF)` — the whole text.
fn is_plain_infinity(source: &str) -> bool {
    let unsigned = source.strip_prefix(['+', '-']).unwrap_or(source);
    matches!(unsigned, ".inf" | ".Inf" | ".INF")
}

/// `[-+]?(\d*\.?\d*)([eE][-+]?\d+)?` — the whole text — with at least one mantissa digit and every mantissa digit `0`.
fn is_plain_zero(source: &str) -> bool {
    let unsigned = source.strip_prefix(['+', '-']).unwrap_or(source);
    let (mantissa, exponent) = match unsigned.find(['e', 'E']) {
        Some(at) => (&unsigned[..at], Some(&unsigned[at + 1..])),
        None => (unsigned, None),
    };
    let exponent_ok = exponent.is_none_or(|exponent| {
        let digits = exponent.strip_prefix(['+', '-']).unwrap_or(exponent);
        !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit())
    });
    let dots = mantissa.bytes().filter(|&b| b == b'.').count();
    let digits = mantissa.bytes().filter(u8::is_ascii_digit).count();
    exponent_ok && dots <= 1 && digits >= 1 && mantissa.bytes().all(|b| b == b'0' || b == b'.')
}

/// The document with every node's position — only what [`changed_by_reading`] needs.
enum Scan {
    Float(f64),
    Scalar(Node),
    Seq(Vec<Spanned<Scan>>),
    Map(Vec<(Spanned<Scan>, Spanned<Scan>)>),
}

impl<'de> Deserialize<'de> for Scan {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(ScanVisitor)
    }
}

struct ScanVisitor;

impl<'de> Visitor<'de> for ScanVisitor {
    type Value = Scan;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("any YAML value")
    }

    fn visit_f64<E: de::Error>(self, value: f64) -> Result<Scan, E> {
        Ok(Scan::Float(value))
    }

    fn visit_bool<E: de::Error>(self, value: bool) -> Result<Scan, E> {
        NodeVisitor.visit_bool(value).map(Scan::Scalar)
    }

    fn visit_i64<E: de::Error>(self, value: i64) -> Result<Scan, E> {
        NodeVisitor.visit_i64(value).map(Scan::Scalar)
    }

    fn visit_u64<E: de::Error>(self, value: u64) -> Result<Scan, E> {
        NodeVisitor.visit_u64(value).map(Scan::Scalar)
    }

    fn visit_i128<E: de::Error>(self, value: i128) -> Result<Scan, E> {
        NodeVisitor.visit_i128(value).map(Scan::Scalar)
    }

    fn visit_u128<E: de::Error>(self, value: u128) -> Result<Scan, E> {
        NodeVisitor.visit_u128(value).map(Scan::Scalar)
    }

    fn visit_str<E: de::Error>(self, value: &str) -> Result<Scan, E> {
        NodeVisitor.visit_str(value).map(Scan::Scalar)
    }

    fn visit_string<E: de::Error>(self, value: String) -> Result<Scan, E> {
        NodeVisitor.visit_string(value).map(Scan::Scalar)
    }

    fn visit_unit<E: de::Error>(self) -> Result<Scan, E> {
        Ok(Scan::Scalar(Node::Null))
    }

    fn visit_none<E: de::Error>(self) -> Result<Scan, E> {
        Ok(Scan::Scalar(Node::Null))
    }

    fn visit_some<D: Deserializer<'de>>(self, deserializer: D) -> Result<Scan, D::Error> {
        Scan::deserialize(deserializer)
    }

    fn visit_newtype_struct<D: Deserializer<'de>>(self, deserializer: D) -> Result<Scan, D::Error> {
        Scan::deserialize(deserializer)
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Scan, A::Error> {
        let mut items = Vec::new();
        while let Some(item) = seq.next_element::<Spanned<Scan>>()? {
            items.push(item);
        }
        Ok(Scan::Seq(items))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Scan, A::Error> {
        let mut entries = Vec::new();
        while let Some(entry) = map.next_entry::<Spanned<Scan>, Spanned<Scan>>()? {
            entries.push(entry);
        }
        Ok(Scan::Map(entries))
    }
}

/// Replaces the five list nodes of the top-level map `entries` (see [`rewrite_lists`] for absent and `null` keys).
fn replace_lists(entries: &mut Vec<(Node, Node)>, doc: &PresetDoc, lists: &PresetLists) {
    let dropped: HashSet<(ListName, u32)> = lists.drop_unmodeled.iter().copied().collect();
    let kept = |list: ListName| -> Vec<u32> {
        doc.unmodeled
            .iter()
            .filter(|entry| entry.list == list && !dropped.contains(&(list, entry.index)))
            .map(|entry| entry.index)
            .collect()
    };
    let position = |entries: &[(Node, Node)], list: ListName| {
        entries
            .iter()
            .position(|(key, _)| key.as_str() == Some(list.key()))
    };
    let original_of = |entries: &[(Node, Node)], list: ListName| -> Vec<Node> {
        match position(entries, list).map(|at| &entries[at].1) {
            Some(Node::Seq(items)) => items.clone(),
            _ => Vec::new(),
        }
    };

    let item_lists = [
        (ListName::Presets, &doc.presets, &lists.presets),
        (
            ListName::Instructions,
            &doc.instructions,
            &lists.instructions,
        ),
        (ListName::Skills, &doc.skills, &lists.skills),
        (ListName::Settings, &doc.settings, &lists.settings),
    ];
    let mut new_lists: Vec<(ListName, Vec<Node>)> = item_lists
        .into_iter()
        .map(|(list, read, wanted)| {
            let modeled = modeled_spellings(
                read.iter()
                    .map(|item| (item.origin, item.target.spelling())),
            );
            let wanted = wanted
                .iter()
                .map(|item| Wanted {
                    origin: item.origin,
                    spelling: item.target.spelling(),
                    render: Box::new(move |base| item_node(base, item, list.takes_tags())),
                })
                .collect();
            (
                list,
                build_list(&original_of(entries, list), &modeled, &kept(list), wanted),
            )
        })
        .collect();
    let folder_modeled = modeled_spellings(
        doc.folders
            .iter()
            .map(|folder| (folder.origin, folder.raw.clone())),
    );
    let folder_wanted = lists
        .folders
        .iter()
        .map(|folder| Wanted {
            origin: folder.origin,
            spelling: folder.path.clone(),
            render: Box::new(move |_| Node::Str(folder.path.clone())),
        })
        .collect();
    new_lists.push((
        ListName::Folders,
        build_list(
            &original_of(entries, ListName::Folders),
            &folder_modeled,
            &kept(ListName::Folders),
            folder_wanted,
        ),
    ));

    let mut absent = Vec::new();
    for (list, new_list) in new_lists {
        match position(entries, list) {
            Some(at) if new_list.is_empty() && matches!(entries[at].1, Node::Null) => {}
            Some(at) => entries[at].1 = Node::Seq(new_list),
            None if new_list.is_empty() => {}
            None => absent.push((Node::Str(list.key().to_owned()), Node::Seq(new_list))),
        }
    }
    let after_last_list = ListName::ALL
        .into_iter()
        .filter_map(|list| position(entries, list))
        .max()
        .map_or(entries.len(), |at| at + 1);
    entries.splice(after_last_list..after_last_list, absent);
}

/// The original modeled entries of one list as `(index, spelling)`, index order.
fn modeled_spellings(
    read: impl Iterator<Item = (Option<ItemOrigin>, String)>,
) -> Vec<(u32, String)> {
    let mut modeled: Vec<(u32, String)> = read
        .filter_map(|(origin, spelling)| origin.map(|origin| (origin.index, spelling)))
        .collect();
    modeled.sort_by_key(|(index, _)| *index);
    modeled
}

/// One entry of a new list: its origin, its spelling (the identity rule for an entry without an origin) and how it is
/// written from its original node (`None` = a new entry).
struct Wanted<'a> {
    origin: Option<ItemOrigin>,
    spelling: String,
    render: Box<dyn Fn(Option<&Node>) -> Node + 'a>,
}

/// The new entries of one list: each wanted entry written from the original node it is identified with, and every
/// kept unmodeled entry copied after the nearest original entry before it that is still in the list (at the head when
/// none is), in original order. Linear apart from the ordered set of taken entries (log n per lookup).
fn build_list(
    original: &[Node],
    modeled: &[(u32, String)],
    kept_unmodeled: &[u32],
    wanted: Vec<Wanted<'_>>,
) -> Vec<Node> {
    let modeled_set: HashSet<u32> = modeled.iter().map(|(index, _)| *index).collect();
    let mut by_spelling: HashMap<&str, VecDeque<u32>> = HashMap::new();
    for (index, spelling) in modeled {
        by_spelling
            .entry(spelling.as_str())
            .or_default()
            .push_back(*index);
    }
    let mut taken = BTreeSet::new();
    let mut assigned: Vec<Option<u32>> = vec![None; wanted.len()];
    for (slot, entry) in assigned.iter_mut().zip(&wanted) {
        if let Some(ItemOrigin { index }) = entry.origin {
            if modeled_set.contains(&index) && taken.insert(index) {
                *slot = Some(index);
            }
        }
    }
    for (slot, entry) in assigned.iter_mut().zip(&wanted) {
        if entry.origin.is_some() {
            continue;
        }
        if let Some(queue) = by_spelling.get_mut(entry.spelling.as_str()) {
            while let Some(index) = queue.pop_front() {
                if taken.insert(index) {
                    *slot = Some(index);
                    break;
                }
            }
        }
    }

    let node_at = |index: u32| original.get(index as usize);
    let mut anchored: HashMap<Option<u32>, Vec<u32>> = HashMap::new();
    for &index in kept_unmodeled {
        let anchor = taken.range(..index).next_back().copied();
        anchored.entry(anchor).or_default().push(index);
    }
    let copies = |anchor: Option<u32>| {
        anchored
            .get(&anchor)
            .into_iter()
            .flatten()
            .filter_map(|&index| node_at(index).cloned())
    };
    let mut out: Vec<Node> = copies(None).collect();
    for (entry, slot) in wanted.iter().zip(&assigned) {
        out.push((entry.render)(slot.and_then(node_at)));
        if slot.is_some() {
            out.extend(copies(*slot));
        }
    }
    out
}

/// An item written from its original node: a map keeps every key in order with `path` and `backend` set from the item
/// (`backend` added after `path`, or removed — a map left with only `path` after losing its tag becomes a string); a
/// string node or no node gives the item's spelling, or `{path, backend}` with a tag.
///
/// In a list that takes no tags (`tags_modeled = false` — `presets` · `settings`) the item's `backend` is not written
/// and the original node's `backend` key is left as it is: the reader models such a tag as `None` (a line problem),
/// and a rewrite must not drop it (ADR-0358 — only formatting may be lost).
fn item_node(base: Option<&Node>, item: &PresetItem, tags_modeled: bool) -> Node {
    let spelling = Node::Str(item.target.spelling());
    let find = |entries: &[(Node, Node)], key: &str| {
        entries.iter().position(|(k, _)| k.as_str() == Some(key))
    };
    // A modeled map entry always has a `path`; anything else is written as a new item.
    let Some((mut entries, path_at)) = (match base {
        Some(Node::Map(original)) => find(original, "path").map(|at| (original.clone(), at)),
        _ => None,
    }) else {
        return match item.backend.as_ref().filter(|_| tags_modeled) {
            None => spelling,
            Some(word) => Node::Map(vec![
                (Node::Str("path".to_owned()), spelling),
                (Node::Str("backend".to_owned()), Node::Str(word.clone())),
            ]),
        };
    };
    entries[path_at].1 = spelling.clone();
    if !tags_modeled {
        return Node::Map(entries);
    }
    match (&item.backend, find(&entries, "backend")) {
        (Some(word), Some(at)) => entries[at].1 = Node::Str(word.clone()),
        (Some(word), None) => entries.insert(
            path_at + 1,
            (Node::Str("backend".to_owned()), Node::Str(word.clone())),
        ),
        (None, Some(at)) => {
            entries.remove(at);
            if entries.len() == 1 {
                return spelling;
            }
        }
        (None, None) => {}
    }
    Node::Map(entries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backends::fake::WordBackends;
    use crate::path::AssetRef;

    // TRD S25 step-7 §8-1 P3 format write — every expected output is pinned byte for byte.

    fn two() -> WordBackends {
        WordBackends::new(&["claude", "codex"])
    }

    /// The Windows spelling where it is an absolute path, else the Unix one (no OS `cfg` in this crate).
    fn absolute(windows: &'static str, unix: &'static str) -> &'static str {
        if std::path::Path::new(windows).is_absolute() {
            windows
        } else {
            unix
        }
    }

    fn folder_a() -> &'static str {
        absolute(r"D:\Rules", "/srv/rules")
    }

    fn folder_b() -> &'static str {
        absolute(r"E:\Other", "/srv/other")
    }

    fn doc(text: &str) -> PresetDoc {
        parse_preset(text.strip_prefix('\u{feff}').unwrap_or(text), &two()).unwrap()
    }

    /// The lists of `text` as read — what an edit starts from.
    fn as_read(text: &str) -> PresetLists {
        let doc = doc(text);
        PresetLists {
            presets: doc.presets,
            instructions: doc.instructions,
            skills: doc.skills,
            settings: doc.settings,
            folders: doc
                .folders
                .iter()
                .map(|folder| FolderItem {
                    path: folder.raw.clone(),
                    origin: folder.origin,
                })
                .collect(),
            drop_unmodeled: Vec::new(),
        }
    }

    fn write(text: &str, lists: &PresetLists) -> String {
        rewrite_lists(text, lists, &two()).unwrap()
    }

    fn new_item(path: &str, backend: Option<&str>) -> PresetItem {
        PresetItem {
            target: AssetRef::parse(path).unwrap(),
            backend: backend.map(str::to_owned),
            line: 0,
            origin: None,
        }
    }

    fn base() -> String {
        format!(
            "# team preset\nkind: preset\nversion: 1\ndescription: team rules\nx-owner: me  # who\n\
             instructions:\n- rules/a.md\n- path: rules/b.md\n  note: keep me\nskills:\n- skills/s\n\
             folders:\n- '{}'\ntail: [1, 2]\n",
            folder_a()
        )
    }

    /// `base()` as written, with the instructions · skills · folders blocks given.
    fn golden(instructions: &str, skills: &str, folders: &str) -> String {
        format!(
            "kind: preset\nversion: 1\ndescription: team rules\nx-owner: me\n{instructions}{skills}{folders}tail:\n- 1\n- 2\n"
        )
    }

    fn untouched_skills() -> &'static str {
        "skills:\n- skills/s\n"
    }

    fn untouched_folders() -> String {
        format!("folders:\n- {}\n", folder_a())
    }

    #[test]
    fn an_unchanged_edit_keeps_unknown_keys_in_place_and_drops_comments_and_flow_style() {
        assert_eq!(
            write(&base(), &as_read(&base())),
            golden(
                "instructions:\n- rules/a.md\n- path: rules/b.md\n  note: keep me\n",
                untouched_skills(),
                &untouched_folders()
            )
        );
    }

    // ── an unknown key inside an item map ──────────────────────────────────────

    #[test]
    fn an_item_s_unknown_key_survives_an_add_elsewhere() {
        let mut lists = as_read(&base());
        lists.skills.push(new_item("skills/t", None));
        assert_eq!(
            write(&base(), &lists),
            golden(
                "instructions:\n- rules/a.md\n- path: rules/b.md\n  note: keep me\n",
                "skills:\n- skills/s\n- skills/t\n",
                &untouched_folders()
            )
        );
    }

    #[test]
    fn an_item_s_unknown_key_survives_a_reorder() {
        let text = "kind: preset\nversion: 1\ninstructions:\n- path: rules/b.md\n  note: keep me\n- rules/a.md\n- rules/c.md\n";
        let mut lists = as_read(text);
        lists.instructions.rotate_left(1);
        assert_eq!(
            write(text, &lists),
            "kind: preset\nversion: 1\ninstructions:\n- rules/a.md\n- rules/c.md\n- path: rules/b.md\n  note: keep me\n"
        );
    }

    #[test]
    fn an_item_s_unknown_key_survives_a_tag_on_and_off() {
        let mut lists = as_read(&base());
        lists.instructions[0].backend = Some("codex".to_owned());
        lists.instructions[1].backend = Some("claude".to_owned());
        let tagged = write(&base(), &lists);
        assert_eq!(
            tagged,
            golden(
                "instructions:\n- path: rules/a.md\n  backend: codex\n- path: rules/b.md\n  backend: claude\n  note: keep me\n",
                untouched_skills(),
                &untouched_folders()
            )
        );

        let mut lists = as_read(&tagged);
        lists.instructions[0].backend = None;
        lists.instructions[1].backend = None;
        assert_eq!(
            write(&tagged, &lists),
            golden(
                "instructions:\n- rules/a.md\n- path: rules/b.md\n  note: keep me\n",
                untouched_skills(),
                &untouched_folders()
            )
        );
    }

    #[test]
    fn an_item_s_unknown_key_survives_a_save_style_list_without_origins() {
        let mut lists = as_read(&base());
        lists.instructions = vec![
            new_item("rules/b.md", Some("codex")),
            new_item("rules/a.md", None),
        ];
        assert_eq!(
            write(&base(), &lists),
            golden(
                "instructions:\n- path: rules/b.md\n  backend: codex\n  note: keep me\n- rules/a.md\n",
                untouched_skills(),
                &untouched_folders()
            )
        );
    }

    #[test]
    fn without_origins_same_spellings_are_matched_first_unused_first() {
        let text = "kind: preset\nversion: 1\nskills:\n- {path: s, k: 1}\n- {path: s, k: 2}\n";
        let lists = PresetLists {
            skills: vec![
                new_item("s", None),
                new_item("s", None),
                new_item("s", None),
            ],
            ..PresetLists::default()
        };
        assert_eq!(
            write(text, &lists),
            "kind: preset\nversion: 1\nskills:\n- path: s\n  k: 1\n- path: s\n  k: 2\n- s\n"
        );
    }

    #[test]
    fn an_origin_preserving_path_change_writes_the_new_path_and_keeps_the_item_s_keys() {
        let mut lists = as_read(&base());
        lists.instructions[0].target = AssetRef::parse("rules/new-a.md").unwrap();
        lists.instructions[1].target = AssetRef::parse("rules/new.md").unwrap();
        assert_eq!(
            write(&base(), &lists),
            golden(
                "instructions:\n- rules/new-a.md\n- path: rules/new.md\n  note: keep me\n",
                untouched_skills(),
                &untouched_folders()
            )
        );
    }

    #[test]
    fn a_folder_entry_s_path_change_is_written() {
        let mut lists = as_read(&base());
        lists.folders[0].path = folder_b().to_owned();
        assert_eq!(
            write(&base(), &lists),
            golden(
                "instructions:\n- rules/a.md\n- path: rules/b.md\n  note: keep me\n",
                untouched_skills(),
                &format!("folders:\n- {}\n", folder_b())
            )
        );
    }

    #[test]
    fn a_map_losing_its_tag_stays_a_map_only_with_other_keys() {
        let text = "kind: preset\nversion: 1\nskills:\n- path: c\n  backend: codex\n  note: m\n- {path: d, backend: codex}\n- {path: e}\n";
        let mut lists = as_read(text);
        for item in &mut lists.skills {
            item.backend = None;
        }
        assert_eq!(
            write(text, &lists),
            "kind: preset\nversion: 1\nskills:\n- path: c\n  note: m\n- d\n- path: e\n"
        );
    }

    #[test]
    fn a_tag_in_presets_survives_an_edit_and_a_path_change() {
        // The reader models a tagged `presets` entry with no backend (a tag is not allowed there); the writer keeps the
        // original node, so the tag survives.
        let text = "kind: preset\nversion: 1\npresets:\n- {path: p.yaml, backend: codex, k: 1}\n- q.yaml\n";
        let mut lists = as_read(text);
        assert_eq!(lists.presets[0].backend, None);
        lists.presets.reverse();
        assert_eq!(
            write(text, &lists),
            "kind: preset\nversion: 1\npresets:\n- q.yaml\n- path: p.yaml\n  backend: codex\n  k: 1\n"
        );
        lists.presets[1].target = AssetRef::parse("moved/p.yaml").unwrap();
        assert_eq!(
            write(text, &lists),
            "kind: preset\nversion: 1\npresets:\n- q.yaml\n- path: moved/p.yaml\n  backend: codex\n  k: 1\n"
        );
    }

    #[test]
    fn a_tag_in_settings_survives_an_edit_and_a_path_change() {
        let text = "kind: preset\nversion: 1\nsettings:\n- path: s.yaml\n  backend: claude\n";
        let mut lists = as_read(text);
        assert_eq!(lists.settings[0].backend, None);
        lists.settings.push(new_item("t.yaml", None));
        assert_eq!(
            write(text, &lists),
            "kind: preset\nversion: 1\nsettings:\n- path: s.yaml\n  backend: claude\n- t.yaml\n"
        );
        lists.settings[0].target = AssetRef::parse("moved/s.yaml").unwrap();
        assert_eq!(
            write(text, &lists),
            "kind: preset\nversion: 1\nsettings:\n- path: moved/s.yaml\n  backend: claude\n- t.yaml\n"
        );
    }

    // ── unmodeled entries ──────────────────────────────────────────────────────

    const UNMODELED: &str = "kind: preset\nversion: 1\nskills:\n- s1\n- 42\n- s2\n- {path: s3, backend: fakebe, x: 1}\n- s4\n";

    #[test]
    fn unmodeled_entries_keep_their_node_and_anchor_after_a_reorder() {
        let mut lists = as_read(UNMODELED);
        assert_eq!(lists.skills.len(), 3);
        lists.skills.rotate_right(1);
        assert_eq!(
            write(UNMODELED, &lists),
            "kind: preset\nversion: 1\nskills:\n- s4\n- s1\n- 42\n- s2\n- path: s3\n  backend: fakebe\n  x: 1\n"
        );
    }

    #[test]
    fn an_unmodeled_entry_whose_anchor_is_removed_moves_to_the_previous_kept_entry_or_the_head() {
        let mut lists = as_read(UNMODELED);
        lists.skills.remove(0);
        assert_eq!(
            write(UNMODELED, &lists),
            "kind: preset\nversion: 1\nskills:\n- 42\n- s2\n- path: s3\n  backend: fakebe\n  x: 1\n- s4\n"
        );
        let mut lists = as_read(UNMODELED);
        lists.skills.remove(1);
        assert_eq!(
            write(UNMODELED, &lists),
            "kind: preset\nversion: 1\nskills:\n- s1\n- 42\n- path: s3\n  backend: fakebe\n  x: 1\n- s4\n"
        );
    }

    #[test]
    fn unmodeled_entries_go_only_when_listed_to_drop() {
        let mut lists = as_read(UNMODELED);
        lists.drop_unmodeled = vec![(ListName::Skills, 1)];
        assert_eq!(
            write(UNMODELED, &lists),
            "kind: preset\nversion: 1\nskills:\n- s1\n- s2\n- path: s3\n  backend: fakebe\n  x: 1\n- s4\n"
        );
        lists.drop_unmodeled.push((ListName::Skills, 3));
        // A drop naming another list leaves this one's entries alone.
        lists.drop_unmodeled.push((ListName::Instructions, 1));
        assert_eq!(
            write(UNMODELED, &lists),
            "kind: preset\nversion: 1\nskills:\n- s1\n- s2\n- s4\n"
        );
    }

    #[test]
    fn a_list_of_only_unmodeled_entries_keeps_them_and_folders_keep_theirs() {
        let text = format!(
            "kind: preset\nversion: 1\nskills:\n- 1\nfolders:\n- '{}'\n- [x]\n",
            folder_a()
        );
        let mut lists = as_read(&text);
        lists.folders.clear();
        assert_eq!(
            write(&text, &lists),
            "kind: preset\nversion: 1\nskills:\n- 1\nfolders:\n- - x\n"
        );
    }

    #[test]
    fn a_word_the_registry_learns_later_is_an_item_the_edit_may_remove() {
        let three = WordBackends::new(&["claude", "codex", "fakebe"]);
        let mut lists = {
            let doc = parse_preset(UNMODELED, &three).unwrap();
            PresetLists {
                skills: doc.skills,
                ..PresetLists::default()
            }
        };
        assert_eq!(lists.skills.len(), 4);
        lists.skills.retain(|item| item.backend.is_none());
        assert_eq!(
            rewrite_lists(UNMODELED, &lists, &three).unwrap(),
            "kind: preset\nversion: 1\nskills:\n- s1\n- 42\n- s2\n- s4\n"
        );
    }

    // ── each operation on each of the five lists ───────────────────────────────

    fn entry(list: ListName, n: u32) -> String {
        match list {
            ListName::Folders => format!("{}{n}", folder_a()),
            _ => format!("{}/{n}", list.key()),
        }
    }

    /// A preset holding `lists` in this order (`None` = `[]`), between two unknown keys.
    fn shaped(lists: &[(ListName, Option<&[u32]>)]) -> String {
        let mut text = String::from("kind: preset\nversion: 1\nk: v\n");
        for (list, entries) in lists {
            match entries {
                None => text.push_str(&format!("{}: []\n", list.key())),
                Some(entries) => {
                    text.push_str(&format!("{}:\n", list.key()));
                    for n in *entries {
                        text.push_str(&format!("- {}\n", entry(*list, *n)));
                    }
                }
            }
        }
        text.push_str("z: 9\n");
        text
    }

    fn all_with(
        target: ListName,
        entries: Option<&'static [u32]>,
    ) -> Vec<(ListName, Option<&'static [u32]>)> {
        ListName::ALL
            .into_iter()
            .map(|list| {
                (
                    list,
                    if list == target {
                        entries
                    } else {
                        Some(&[1u32, 2][..])
                    },
                )
            })
            .collect()
    }

    fn edit(
        lists: &mut PresetLists,
        list: ListName,
        change: impl Fn(&mut Vec<PresetItem>),
        folders: impl Fn(&mut Vec<FolderItem>),
    ) {
        match list {
            ListName::Presets => change(&mut lists.presets),
            ListName::Instructions => change(&mut lists.instructions),
            ListName::Skills => change(&mut lists.skills),
            ListName::Settings => change(&mut lists.settings),
            ListName::Folders => folders(&mut lists.folders),
        }
    }

    fn added(list: ListName, n: u32) -> (PresetItem, FolderItem) {
        let path = entry(list, n);
        let item = match list {
            ListName::Folders => new_item("placeholder", None),
            _ => new_item(&path, None),
        };
        (item, FolderItem { path, origin: None })
    }

    #[test]
    fn add_remove_reorder_and_empty_on_every_list() {
        for list in ListName::ALL {
            let text = shaped(&all_with(list, Some(&[1, 2])));
            let (item, folder) = added(list, 3);

            let mut lists = as_read(&text);
            edit(
                &mut lists,
                list,
                |items| items.push(item.clone()),
                |folders| folders.push(folder.clone()),
            );
            assert_eq!(
                write(&text, &lists),
                shaped(&all_with(list, Some(&[1, 2, 3]))),
                "add {list:?}"
            );

            let mut lists = as_read(&text);
            edit(
                &mut lists,
                list,
                |items| {
                    items.remove(0);
                },
                |folders| {
                    folders.remove(0);
                },
            );
            assert_eq!(
                write(&text, &lists),
                shaped(&all_with(list, Some(&[2]))),
                "remove {list:?}"
            );

            let mut lists = as_read(&text);
            edit(
                &mut lists,
                list,
                |items| items.reverse(),
                |folders| folders.reverse(),
            );
            assert_eq!(
                write(&text, &lists),
                shaped(&all_with(list, Some(&[2, 1]))),
                "reorder {list:?}"
            );

            let mut lists = as_read(&text);
            edit(&mut lists, list, Vec::clear, Vec::clear);
            assert_eq!(
                write(&text, &lists),
                shaped(&all_with(list, None)),
                "empty {list:?}"
            );
        }
    }

    #[test]
    fn the_first_item_into_an_empty_flow_list_on_every_list() {
        for list in ListName::ALL {
            let text = shaped(&all_with(list, None));
            let (item, folder) = added(list, 1);
            let mut lists = as_read(&text);
            edit(
                &mut lists,
                list,
                |items| items.push(item.clone()),
                |folders| folders.push(folder.clone()),
            );
            assert_eq!(
                write(&text, &lists),
                shaped(&all_with(list, Some(&[1]))),
                "{list:?}"
            );
        }
    }

    #[test]
    fn an_absent_list_key_is_added_after_the_last_list_key_on_every_list() {
        for list in ListName::ALL {
            let others: Vec<(ListName, Option<&[u32]>)> = ListName::ALL
                .into_iter()
                .filter(|&other| other != list)
                .map(|other| (other, Some(&[1u32, 2][..])))
                .collect();
            let text = shaped(&others);
            // Nothing to add: the key stays absent.
            assert_eq!(write(&text, &as_read(&text)), text, "{list:?} unchanged");

            let (item, folder) = added(list, 1);
            let mut lists = as_read(&text);
            edit(
                &mut lists,
                list,
                |items| items.push(item.clone()),
                |folders| folders.push(folder.clone()),
            );
            let mut expected = others.clone();
            expected.push((list, Some(&[1u32][..])));
            assert_eq!(write(&text, &lists), shaped(&expected), "{list:?}");
        }
    }

    #[test]
    fn absent_list_keys_go_in_list_order_and_at_the_end_when_no_list_key_exists() {
        let text = "kind: preset\nversion: 1\nk: v\n";
        let lists = PresetLists {
            skills: vec![new_item("s", None)],
            presets: vec![new_item("p.yaml", None)],
            ..PresetLists::default()
        };
        assert_eq!(
            write(text, &lists),
            "kind: preset\nversion: 1\nk: v\npresets:\n- p.yaml\nskills:\n- s\n"
        );
    }

    #[test]
    fn a_null_list_left_empty_stays_null() {
        let text = "kind: preset\nversion: 1\nskills:\nk: v\n";
        assert_eq!(
            write(text, &as_read(text)),
            "kind: preset\nversion: 1\nskills: null\nk: v\n"
        );
        let lists = PresetLists {
            skills: vec![new_item("s", None)],
            ..PresetLists::default()
        };
        assert_eq!(
            write(text, &lists),
            "kind: preset\nversion: 1\nskills:\n- s\nk: v\n"
        );
    }

    #[test]
    fn tag_on_and_off_on_both_tag_lists() {
        for list in [ListName::Instructions, ListName::Skills] {
            let text = shaped(&[(list, Some(&[1, 2]))]);
            let mut lists = as_read(&text);
            edit(
                &mut lists,
                list,
                |items| items[1].backend = Some("codex".to_owned()),
                |_| {},
            );
            let tagged = write(&text, &lists);
            assert_eq!(
                tagged,
                format!(
                    "kind: preset\nversion: 1\nk: v\n{key}:\n- {key}/1\n- path: {key}/2\n  backend: codex\nz: 9\n",
                    key = list.key()
                ),
                "{list:?}"
            );
            let mut lists = as_read(&tagged);
            edit(&mut lists, list, |items| items[1].backend = None, |_| {});
            assert_eq!(write(&tagged, &lists), text, "{list:?}");
        }
    }

    // ── line ending and BOM ────────────────────────────────────────────────────

    #[test]
    fn crlf_in_gives_crlf_out_and_lf_gives_lf() {
        let text = "kind: preset\r\nversion: 1\r\nskills:\r\n- a\r\n";
        let mut lists = as_read(text);
        lists.skills.push(new_item("b", None));
        assert_eq!(
            write(text, &lists),
            "kind: preset\r\nversion: 1\r\nskills:\r\n- a\r\n- b\r\n"
        );
        let text = "kind: preset\nversion: 1\nskills:\n- a\n";
        assert_eq!(
            write(text, &lists),
            "kind: preset\nversion: 1\nskills:\n- a\n- b\n"
        );
    }

    #[test]
    fn a_bom_is_kept_and_none_is_added() {
        let text = "\u{feff}kind: preset\nversion: 1\nskills:\n- a\n";
        assert_eq!(write(text, &as_read(text)), text);
        let text = "kind: preset\nversion: 1\nskills:\n- a\n";
        assert_eq!(write(text, &as_read(text)), text);
    }

    // ── values that must read back the same ────────────────────────────────────

    #[test]
    fn ambiguous_strings_are_quoted_and_read_back_as_the_same_strings() {
        let text = "kind: preset\nversion: 1\nwords: ['yes', 'on', 'null', '1.0', '012', '~', '-', ':x', 'y', '']\n\
                    skills: ['yes', 'on', 'null', '1.0', '012', '~', '-']\n";
        let written = write(text, &as_read(text));
        assert_eq!(
            written,
            "kind: preset\nversion: 1\nwords:\n- \"yes\"\n- \"on\"\n- \"null\"\n- \"1.0\"\n- \"012\"\n- \"~\"\n- '-'\n\
             - \":x\"\n- \"y\"\n- \"\"\nskills:\n- \"yes\"\n- \"on\"\n- \"null\"\n- \"1.0\"\n- \"012\"\n- \"~\"\n- '-'\n"
        );
        let reread = doc(&written);
        let skills: Vec<String> = reread
            .skills
            .iter()
            .map(|item| item.target.spelling())
            .collect();
        assert_eq!(skills, ["yes", "on", "null", "1.0", "012", "~", "-"]);
        let words = parse_tree(&written).unwrap();
        let expected = parse_tree(text).unwrap();
        assert!(words
            .get("words")
            .unwrap()
            .same(expected.get("words").unwrap()));
    }

    #[test]
    fn a_non_string_key_survives_and_an_alias_is_written_expanded() {
        let text = "kind: preset\nversion: 1\n7: seven\n[a, b]: pair\nbase: &b {x: 1}\ncopy: *b\n";
        assert_eq!(
            write(text, &as_read(text)),
            "kind: preset\nversion: 1\n7: seven\n? - a\n  - b\n: pair\nbase:\n  x: 1\ncopy:\n  x: 1\n"
        );
    }

    #[test]
    fn an_integer_beyond_64_bits_is_refused_though_the_round_trip_check_alone_would_pass() {
        let text = "kind: preset\nversion: 1\nextra:\n  big: 99999999999999999999\nskills: [a]\n";
        assert_eq!(
            rewrite_lists(text, &as_read(text), &two()),
            Err(WriteError::NumberOutOfRange {
                path: "/extra/big".to_owned()
            })
        );
        // Step 6 alone would let this file through: the digits were lost when it was read.
        let tree = parse_tree(text).unwrap();
        let reread = parse_tree(&serialize(&tree).unwrap()).unwrap();
        assert!(tree.same(&reread));

        let error = WriteError::NumberOutOfRange {
            path: "/extra/big".to_owned(),
        };
        assert_eq!(error.code(), "preset-write-number-range");
        assert_eq!(
            error.to_reason().args,
            serde_json::json!({ "path": "/extra/big" })
        );

        for (text, path) in [
            ("kind: preset\nversion: 1\nn: -99999999999999999999\n", "/n"),
            ("kind: preset\nversion: 1\nn: 1e20\n", "/n"),
            // Any float that large is integral, so a written float is refused too (the tree cannot tell them apart).
            ("kind: preset\nversion: 1\nn: 1.5e300\n", "/n"),
            ("kind: preset\nversion: 1\nn: [1, 9.3e18]\n", "/n/1"),
            (
                "kind: preset\nversion: 1\n1e19: x\n",
                "/10000000000000000000",
            ),
        ] {
            assert_eq!(
                rewrite_lists(text, &as_read(text), &two()),
                Err(WriteError::NumberOutOfRange {
                    path: path.to_owned()
                }),
                "{text:?}"
            );
        }
    }

    #[test]
    fn integers_at_the_64_bit_edges_and_other_floats_survive_exactly() {
        let text = "kind: preset\nversion: 1\na: 9223372036854775807\nb: 18446744073709551615\nc: -9223372036854775808\n\
                    d: -0.0\ne: 0.1\nf: 9.2e18\ng: 1.5e-300\n";
        assert_eq!(
            write(text, &as_read(text)),
            "kind: preset\nversion: 1\na: 9223372036854775807\nb: 18446744073709551615\nc: -9223372036854775808\n\
             d: -0.0\ne: 0.1\nf: 9.2e+18\ng: 1.5e-300\n"
        );
    }

    #[test]
    fn a_nan_in_an_unknown_key_does_not_stop_the_edit() {
        let text = "kind: preset\nversion: 1\nextra: [.nan, -.nan, .inf, -.inf]\nskills: [a]\n";
        let mut lists = as_read(text);
        lists.skills.push(new_item("b", None));
        assert_eq!(
            write(text, &lists),
            "kind: preset\nversion: 1\nextra:\n- .nan\n- .nan\n- .inf\n- -.inf\nskills:\n- a\n- b\n"
        );
    }

    // ── refusals ───────────────────────────────────────────────────────────────

    #[test]
    fn a_multi_document_file_is_refused() {
        let text = "kind: preset\nversion: 1\n---\nkind: preset\nversion: 1\n";
        let error = rewrite_lists(text, &PresetLists::default(), &two()).unwrap_err();
        assert!(
            matches!(error, WriteError::MultipleDocuments(_)),
            "{error:?}"
        );
        assert_eq!(error.code(), "preset-multi-document");
        assert_eq!(error.to_reason().line, Some(4));
    }

    #[test]
    fn a_file_that_is_not_a_preset_is_refused() {
        let cases: &[(&str, &str, Option<u32>)] = &[
            ("kind: settings\nversion: 1\n", "wrong-kind", Some(1)),
            ("version: 1\nkind: other\n", "kind-unknown", Some(2)),
            ("version: 1\n", "kind-unknown", None),
            ("- a\n", "kind-unknown", None),
            ("", "kind-unknown", None),
        ];
        for (text, code, line) in cases {
            let error = rewrite_lists(text, &PresetLists::default(), &two()).unwrap_err();
            assert!(
                matches!(error, WriteError::NotAPreset(_)),
                "{text:?}: {error:?}"
            );
            assert_eq!(
                (error.code(), error.to_reason().line),
                (*code, *line),
                "{text:?}"
            );
        }
        let error = rewrite_lists(
            "kind: preset\nversion: [\n",
            &PresetLists::default(),
            &two(),
        )
        .unwrap_err();
        assert_eq!(error.code(), "yaml-syntax");
        let error = rewrite_lists("kind: preset\n", &PresetLists::default(), &two()).unwrap_err();
        assert_eq!(error.code(), "field-invalid");
    }

    #[test]
    fn output_that_reads_back_differently_is_refused() {
        let text = "kind: preset\nversion: 1\nskills: [a]\n";
        MANGLE.with(|hook| hook.set(Some(|written| written.replace("- a", "- b"))));
        let result = rewrite_lists(text, &as_read(text), &two());
        MANGLE.with(|hook| hook.set(None));
        let error = result.unwrap_err();
        assert_eq!(
            error,
            WriteError::RoundTrip {
                path: "/skills/0".to_owned(),
                reread: None,
            }
        );
        assert_eq!(error.code(), "preset-write-mismatch");
        // Without the hook the same edit is written.
        assert!(rewrite_lists(text, &as_read(text), &two()).is_ok());
    }

    #[test]
    fn comments_and_blank_lines_are_gone() {
        let text = "# head\nkind: preset   # kind\n\nversion: 1\n# before skills\nskills:\n  - a   # first\n\n";
        assert_eq!(
            write(text, &as_read(text)),
            "kind: preset\nversion: 1\nskills:\n- a\n"
        );
    }

    // ── review round 1 ─────────────────────────────────────────────────────────

    #[test]
    fn a_number_beyond_the_f64_range_is_refused_though_the_round_trip_check_alone_would_pass() {
        let digits = "9".repeat(400);
        let cases = [
            ("n: 1e999\n".to_owned(), "/n"),
            ("n: -1e999\n".to_owned(), "/n"),
            (format!("n: {digits}\n"), "/n"),
            (format!("n: -{digits}\n"), "/n"),
            ("n: [1, -9e400]\n".to_owned(), "/n/1"),
            ("n: !!float 1e999\n".to_owned(), "/n"),
            ("1e999: x\n".to_owned(), "/inf"),
            ("a: &x 1e999\nb: *x\n".to_owned(), "/a"),
        ];
        for (body, path) in cases {
            let text = format!("kind: preset\nversion: 1\n{body}");
            assert_eq!(
                rewrite_lists(&text, &as_read(&text), &two()),
                Err(WriteError::NumberOutOfRange {
                    path: path.to_owned()
                }),
                "{body:?}"
            );
        }
        // The round-trip check alone would let it through: an infinity writes and reads back as `.inf`.
        let tree = parse_tree("n: 1e999\n").unwrap();
        let reread = parse_tree(&serialize(&tree).unwrap()).unwrap();
        assert!(tree.same(&reread));
    }

    #[test]
    fn an_explicit_infinity_in_any_spelling_is_kept() {
        let text =
            "kind: preset\nversion: 1\nv: [.inf, -.inf, +.inf, .Inf, -.INF, !!float .inf]\nm: &i .inf\nk: *i\n";
        assert_eq!(
            write(text, &as_read(text)),
            "kind: preset\nversion: 1\nv:\n- .inf\n- -.inf\n- .inf\n- .inf\n- -.inf\n- .inf\nm: .inf\nk: .inf\n"
        );
    }

    #[test]
    fn content_after_an_explicit_document_end_is_refused_not_deleted() {
        for tail in [
            "\n@todo\n",
            "\n`x`\n",
            "\n%TODO\n",
            "\n'til\n",
            "\r@todo\n",
            "\n# c\r@todo\n",
        ] {
            let text = format!("kind: preset\nversion: 1\nskills: [a]\n...{tail}");
            let error = rewrite_lists(&text, &PresetLists::default(), &two()).unwrap_err();
            assert!(
                matches!(error, WriteError::MultipleDocuments(_)),
                "{tail:?}: {error:?}"
            );
        }
        let text = "kind: preset\nversion: 1\nskills: [a]\n...\n# trailing note\n\n";
        assert_eq!(
            write(text, &as_read(text)),
            "kind: preset\nversion: 1\nskills:\n- a\n"
        );
    }

    #[test]
    fn a_binary_value_keeps_its_base64_text() {
        // `//79` is the base64 of 0xFF 0xFE 0xFD — not UTF-8 once decoded.
        let text =
            "kind: preset\nversion: 1\nblob: !!binary aGVsbG8=\nraw: !!binary //79\nskills: [a]\n";
        let mut lists = as_read(text);
        lists.skills.push(new_item("b", None));
        assert_eq!(
            write(text, &lists),
            "kind: preset\nversion: 1\nblob: aGVsbG8=\nraw: //79\nskills:\n- a\n- b\n"
        );
    }

    #[test]
    fn a_large_list_is_rewritten() {
        const N: usize = 50_000;
        let mut text = String::from("kind: preset\nversion: 1\nskills:\n");
        for n in 0..N {
            if n % 1000 == 999 {
                text.push_str(&format!("- {n}\n"));
            } else {
                text.push_str(&format!("- s/{n}\n"));
            }
        }
        // Reordered with origins, and the same list without origins (a save-style list).
        let mut with_origins = as_read(&text);
        with_origins.skills.reverse();
        let without_origins = PresetLists {
            skills: with_origins
                .skills
                .iter()
                .map(|item| new_item(&item.target.spelling(), None))
                .collect(),
            ..PresetLists::default()
        };
        for lists in [&with_origins, &without_origins] {
            let written = write(&text, lists);
            let reread = doc(&written);
            assert_eq!(reread.skills.len(), N - N / 1000);
            assert_eq!(reread.skills[0].target.spelling(), format!("s/{}", N - 2));
            // Each number entry stays right after the entry that preceded it in the file.
            assert_eq!(reread.unmodeled.len(), N / 1000);
            assert!(written
                .starts_with("kind: preset\nversion: 1\nskills:\n- s/49998\n- 49999\n- s/49997\n"));
            assert!(
                written.contains("- s/1000\n- s/998\n- 999\n- s/997\n"),
                "anchor after s/998"
            );
        }
    }

    #[test]
    fn the_round_trip_check_reads_the_returned_crlf_text() {
        let long = "aaaa bbbb cccc dddd eeee ffff gggg hhhh iiii jjjj kkkk llll mmmm nnnn oooo pppp qqqq rrrr";
        let text = format!(
            "kind: preset\r\nversion: 1\r\nnote: \"line1\\nline2\\n\"\r\nlong: '{long}'\r\nskills: [a]\r\n"
        );
        assert_eq!(
            write(&text, &as_read(&text)),
            "kind: preset\r\nversion: 1\r\nnote: |\r\n  line1\r\n  line2\r\nlong: >-\r\n  aaaa bbbb cccc dddd eeee \
             ffff gggg hhhh iiii jjjj kkkk llll mmmm nnnn oooo pppp\r\n  qqqq rrrr\r\nskills:\r\n- a\r\n"
        );
        // A change made after the line endings are restored is caught.
        MANGLE.with(|hook| hook.set(Some(|out| out.replace("- a\r\n", "- b\r\n"))));
        let result = rewrite_lists(&text, &as_read(&text), &two());
        MANGLE.with(|hook| hook.set(None));
        assert_eq!(
            result,
            Err(WriteError::RoundTrip {
                path: "/skills/0".to_owned(),
                reread: None
            })
        );
    }

    #[test]
    fn a_bom_and_crlf_together_are_kept() {
        let text = "\u{feff}kind: preset\r\nversion: 1\r\nskills:\r\n- a\r\n";
        let mut lists = as_read(text);
        lists.skills.push(new_item("b", None));
        assert_eq!(
            write(text, &lists),
            "\u{feff}kind: preset\r\nversion: 1\r\nskills:\r\n- a\r\n- b\r\n"
        );
    }

    #[test]
    fn keys_that_collapse_to_one_value_are_refused_with_the_read_back_error() {
        for (keys, line) in [
            ("1.0: a\n1.00: b\n", 4),
            ("true: a\nTrue: b\n", 4),
            ("null: a\n~: b\n", 4),
        ] {
            let text = format!("kind: preset\nversion: 1\n{keys}");
            let error = rewrite_lists(&text, &as_read(&text), &two()).unwrap_err();
            let WriteError::RoundTrip {
                path,
                reread: Some(reread),
            } = &error
            else {
                panic!("{keys:?}: {error:?}")
            };
            assert_eq!((path.as_str(), reread.line), ("", Some(line)), "{keys:?}");
            assert!(reread.message.contains("duplicate"), "{}", reread.message);
            let reason = error.to_reason();
            // `Reason::line` is a line of the file; the read-back line is one of the unwritten output.
            assert_eq!(
                (reason.code.as_str(), reason.line),
                ("preset-write-mismatch", None)
            );
            assert_eq!(reason.args["message"], serde_json::json!(reread.message));
            assert_eq!(reason.args["written_line"], serde_json::json!(line));
        }
    }

    #[test]
    fn a_nul_after_three_dots_is_refused_by_the_writer() {
        for text in [
            "kind: preset\nversion: 1\nskills: [a]\n...\0@todo\n",
            "kind: preset\nversion: 1\nskills: [a]\n...\0\n@todo keep this\n",
        ] {
            assert!(
                rewrite_lists(text, &as_read_lenient(text), &two()).is_err(),
                "{text:?}"
            );
        }
    }

    /// The lists of `text` when it reads, else empty lists (the refusal under test may come from the reader).
    fn as_read_lenient(text: &str) -> PresetLists {
        match parse_preset(text, &two()) {
            Ok(_) => as_read(text),
            Err(_) => PresetLists::default(),
        }
    }

    #[test]
    fn an_infinity_or_zero_not_spelled_plainly_is_refused() {
        for (body, path) in [
            ("v: !!float |- # e\n  1e-400\n", "/v"),
            // A real zero in a block scalar is refused too — a documented safe false positive.
            ("v: !!float |2-\n  0\n", "/v"),
            ("v: !!float '1e-400'\n", "/v"),
            ("v: !!float '0.0'\n", "/v"),
            ("v: !!float \".inf\"\n", "/v"),
            ("v: !!float |-\n  .inf\n", "/v"),
            ("v: .iNf\n", "/v"),
        ] {
            let text = format!("kind: preset\nversion: 1\n{body}");
            assert_eq!(
                rewrite_lists(&text, &as_read(&text), &two()),
                Err(WriteError::NumberOutOfRange {
                    path: path.to_owned()
                }),
                "{body:?}"
            );
        }
    }

    #[test]
    fn a_number_below_the_f64_range_read_as_zero_is_refused_and_real_zeros_are_kept() {
        for (body, path) in [
            ("v: 1e-400\n", "/v"),
            ("v: -5e-999\n", "/v"),
            ("v: [0.0, 0.5e-400]\n", "/v/1"),
            ("v: !!float 1e-400\n", "/v"),
            ("a: &x 1e-400\nb: *x\n", "/a"),
        ] {
            let text = format!("kind: preset\nversion: 1\n{body}");
            assert_eq!(
                rewrite_lists(&text, &as_read(&text), &two()),
                Err(WriteError::NumberOutOfRange {
                    path: path.to_owned()
                }),
                "{body:?}"
            );
        }
        let text = "kind: preset\nversion: 1\nv: [0.0, -0.0, 0e5, .0, 0.000e-400, +0.0]\n";
        assert_eq!(
            write(text, &as_read(text)),
            "kind: preset\nversion: 1\nv:\n- 0.0\n- -0.0\n- 0.0\n- 0.0\n- 0.0\n- 0.0\n"
        );
    }

    #[test]
    fn a_dots_key_that_is_not_a_document_end_reads_and_round_trips() {
        let text = "kind: preset\nversion: 1\n...#owner: team\n...x: 1\nskills: [a]\n";
        let doc = doc(text);
        assert_eq!(doc.skills.len(), 1);
        let mut lists = as_read(text);
        lists.skills.push(new_item("b", None));
        assert_eq!(
            write(text, &lists),
            "kind: preset\nversion: 1\n...#owner: team\n...x: 1\nskills:\n- a\n- b\n"
        );
    }

    #[test]
    fn bare_cr_line_breaks_are_read_and_written_as_lf() {
        let text = "kind: preset\rversion: 1\rskills:\r- a\r";
        let mut lists = as_read(text);
        assert_eq!(lists.skills.len(), 1);
        lists.skills.push(new_item("b", None));
        assert_eq!(
            write(text, &lists),
            "kind: preset\nversion: 1\nskills:\n- a\n- b\n"
        );
    }

    #[test]
    fn known_read_time_losses_are_pinned() {
        let text = "kind: preset\nversion: 1\nlead: 012\nunder: 1_000\nbin: 0b101\nneghex: -0x10\n\
                    bighex: 0xFFFFFFFFFFFFFFFFFF\nbigunder: 1_000_000_000_000_000_000_000\nbase: &b {p: 1}\n\
                    merged:\n  <<: *b\n  q: 2\n";
        assert_eq!(
            write(text, &as_read(text)),
            "kind: preset\nversion: 1\nlead: 12.0\nunder: 1000\nbin: 5\nneghex: -16\n\
             bighex: \"0xFFFFFFFFFFFFFFFFFF\"\nbigunder: \"1_000_000_000_000_000_000_000\"\nbase:\n  p: 1\n\
             merged:\n  q: 2\n  p: 1\n"
        );
    }
}
