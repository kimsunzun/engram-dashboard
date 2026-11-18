//! The text of `state.json` ↔ [`StateFile`] — read leniency and write caps (size · nesting — TRD S21-storage §6-2 · §6-4 I4). It does not open files — the caller reads · writes with base `file`; whether it can be read whole is base `file`'s one decision, and this module's part is [`STATE_SPEC`] (version key · version · cap · head shape check) and window parsing.
//! Reads fold into three layers:
//! - Unusable as a whole ([`Unusable`]) — corrupt (`not JSON` · `not a JSON object` · `` `version` is not an integer of 1 or more `` · the head (`saved_at_ms` · `clean_exit` · `resolved_crash_copy` · the `windows` array) unreadable · `exceeded the {cap} byte cap` · not UTF-8) or a `version` newer than this shell. Without `version` it is version 1.
//! - Skip only that window — unknown `kind` · an unreadable window field (tab fields included).
//! - Skip only that tab — an unreadable tab (unknown node kind · slot content that is not an object, etc.).
//! A window's `theme` unknown or `number` not an integer in `1..=MAX_SAVED_NUMBER` keeps the window and folds only that field to `None` (global theme · no number). `bounds` · `number` absent or `null` are `None` (a window whose position was never seen · a file that wrote no numbers) — not a warning; restore does not look at main's `number` (D3), so a missing one warns not at all. A tab's `named` absent or not a bool keeps the tab and reads `None` (auto name) — not a warning either (TRD S26-window-chrome §3-3; decided by the serde of `schema::TabEntry`). What was skipped and folded is returned as [`DecodeWarning`] — the caller emits the logs.

use std::fmt;

use engram_dashboard_base::file::{self, Parsed};
use serde::Deserialize;
use serde_json::{Map, Value};
use uuid::Uuid;

use super::schema::{
    Bounds, StateFile, TabEntry, TabStrip, WindowEntry, WindowKind, STATE_VERSION,
};
use crate::layout::numbering;
use crate::theme::UiTheme;

/// The read cap and also the write cap — text over it makes the next boot fold the whole file as unusable (I4). The caller reads with `engram_dashboard_base::file::read_file_capped` using this value.
pub const STATE_READ_CAP: u64 = 4 * 1024 * 1024;

/// The read rules of `state.json` · `state.crash.json` — the shape check is head parsing (each window is lenient, so not looked at). The boot decision and the writer's pre-save re-decision use the same definition.
// ADR-0291 R1~R7
pub const STATE_SPEC: file::Spec = file::Spec {
    version_key: "version",
    current: STATE_VERSION as u64,
    cap: STATE_READ_CAP,
    shape: head_shape,
};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Unusable {
    /// Corrupt — the base `file` decision's reason text as is (`exceeded the {cap} byte cap` · not UTF-8 · `not JSON` · `not a JSON object` · `` `version` is not an integer of 1 or more `` · the head cannot be read).
    #[error("{0}")]
    Corrupt(String),
    #[error("newer format than this shell (version {found} — this shell goes up to {STATE_VERSION})")]
    NewerVersion { found: u64 },
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EncodeError {
    #[error("the text to write ({len} bytes) exceeds the cap of {STATE_READ_CAP} bytes")]
    TooLarge { len: usize },
    /// It does not read back with the same parser as [`decode`] — reached through serde_json's nesting cap (128 levels), e.g. a split tree splitting the same slot again and again. The byte cap check cannot catch this.
    #[error("the text to write does not read back: {reason}")]
    Unparsable { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeWarning {
    WindowSkipped {
        index: usize,
        id: Option<String>,
        reason: String,
    },
    TabSkipped {
        window_id: String,
        index: usize,
        reason: String,
    },
    ThemeDropped { window_id: String, raw: String },
    /// A popout's window number field has a value that is not an integer in `1..=MAX_SAVED_NUMBER` — only that field folds to `None` and the window is kept (TRD S26-window-chrome §3-3). Absent · `null` · main's field do not raise this warning. `raw` = the JSON raw text of the received value, with the same cap as `ThemeDropped`.
    // ADR-0300
    NumberDropped { window_id: String, raw: String },
}

impl fmt::Display for DecodeWarning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DecodeWarning::WindowSkipped { index, id, reason } => match id {
                Some(id) => write!(f, "skipping window #{index} ({id}): {reason}"),
                None => write!(f, "skipping window #{index}: {reason}"),
            },
            DecodeWarning::TabSkipped {
                window_id,
                index,
                reason,
            } => write!(f, "skipping tab #{index} of window {window_id}: {reason}"),
            DecodeWarning::ThemeDropped { window_id, raw } => {
                write!(
                    f,
                    "unknown theme {raw} of window {window_id} — following the global theme"
                )
            }
            DecodeWarning::NumberDropped { window_id, raw } => {
                write!(f, "number {raw} of window {window_id} is unusable — discarding it")
            }
        }
    }
}

pub fn decode(text: &str) -> Result<(StateFile, Vec<DecodeWarning>), Unusable> {
    decode_parsed(file::parse(text, &STATE_SPEC))
}

pub fn decode_parsed(parsed: Parsed) -> Result<(StateFile, Vec<DecodeWarning>), Unusable> {
    let doc = match parsed {
        Parsed::Usable { doc, .. } => doc,
        Parsed::Unusable(reason) => return Err(Unusable::Corrupt(reason)),
        Parsed::Newer { found } => return Err(Unusable::NewerVersion { found }),
    };
    // A head that already passed the shape check ([`head_shape`]) — read once more to get the values.
    let head = Head::deserialize(Value::Object(doc))
        .map_err(|e| Unusable::Corrupt(not_a_state_file(&e)))?;

    let mut warnings = Vec::new();
    let windows = head
        .windows
        .into_iter()
        .enumerate()
        .filter_map(|(index, raw)| decode_window(index, raw, &mut warnings))
        .collect();
    let state = StateFile {
        version: STATE_VERSION,
        saved_at_ms: head.saved_at_ms,
        clean_exit: head.clean_exit,
        resolved_crash_copy: head.resolved_crash_copy,
        windows,
    };
    Ok((state, warnings))
}

pub fn encode(state: &StateFile) -> Result<String, EncodeError> {
    // A file people · agents open to look at, so lines are broken.
    let mut text = serde_json::to_string_pretty(state)
        .expect("only string keys and an infallible serializer, so JSON serialization cannot fail");
    text.push('\n');
    if file::check_cap(text.len(), &STATE_SPEC).is_err() {
        return Err(EncodeError::TooLarge { len: text.len() });
    }
    // Read back with the same decision as [`decode`] (`file::parse`) — do not switch to another parser: measuring with a path that does not measure nesting depth (`IgnoredAny` etc.) would let text passing the read-back be unreadable by `decode`.
    if let Parsed::Unusable(reason) = file::parse(&text, &STATE_SPEC) {
        return Err(EncodeError::Unparsable { reason });
    }
    Ok(text)
}

/// The shape check of [`STATE_SPEC`] — whether the head can be read. Each window is not looked at (the leniency of skipping only that window).
fn head_shape(version: u64, doc: &Map<String, Value>) -> Result<(), String> {
    // There is no reader for earlier versions — reading an earlier version's file as this version would misread the meaning of fields (`schema` header 「field addition rules」).
    if version != u64::from(STATE_VERSION) {
        return Err(format!(
            "no reader for version {version} (this shell is {STATE_VERSION})"
        ));
    }
    Head::deserialize(doc)
        .map(drop)
        .map_err(|e| not_a_state_file(&e))
}

fn not_a_state_file(error: &serde_json::Error) -> String {
    format!("not a state file shape: {error}")
}

/// The identity of a crash copy — the value for [`StateFile::resolved_crash_copy`], measured over the raw bytes read as is; boot and the writer must both use this function to see the same copy as the same.
pub fn crash_copy_hash(text: &str) -> String {
    fnv1a_hex(text.as_bytes())
}

/// 64-bit FNV-1a — the same bytes give the same value across builds · runs (std `DefaultHasher` does not promise that). For telling whether content is the same, not security — a deliberately crafted collision is not prevented.
fn fnv1a_64(bytes: &[u8]) -> u64 {
    const OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    bytes.iter().fold(OFFSET_BASIS, |hash, &byte| {
        (hash ^ u64::from(byte)).wrapping_mul(PRIME)
    })
}

fn fnv1a_hex(bytes: &[u8]) -> String {
    format!("{:016x}", fnv1a_64(bytes))
}

#[derive(Deserialize)]
struct Head {
    saved_at_ms: u64,
    clean_exit: bool,
    resolved_crash_copy: Option<String>,
    windows: Vec<Value>,
}

/// The `kind` spelling of [`WindowKind`] — the same `rename_all`, so the same spelling (the round-trip test holds them against each other).
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum KindTag {
    Main,
    Popout,
}

#[derive(Deserialize)]
struct WindowHead {
    id: String,
    #[serde(default)]
    number: Value,
    #[serde(default)]
    theme: Value,
    // Absent · `null` = `None`. Not an object or a non-number field (non-finite floats go out as `null`) is a read failure of that window — this version's writes do not produce such values (`WindowBounds` holds only finite values).
    #[serde(default)]
    bounds: Option<Bounds>,
    maximized: bool,
}

#[derive(Deserialize)]
struct StripHead {
    active_tab: Uuid,
    tabs: Vec<Value>,
}

fn decode_window(
    index: usize,
    raw: Value,
    warnings: &mut Vec<DecodeWarning>,
) -> Option<WindowEntry> {
    let id = raw.get("id").and_then(Value::as_str).map(str::to_string);
    let mut skip = |reason: String| {
        warnings.push(DecodeWarning::WindowSkipped {
            index,
            id: id.clone(),
            reason,
        });
    };
    let kind = match raw.get("kind") {
        None => {
            skip("`kind` is missing".to_string());
            return None;
        }
        Some(kind) => match KindTag::deserialize(kind) {
            Ok(kind) => kind,
            Err(_) => {
                skip(format!("unknown window kind {kind}"));
                return None;
            }
        },
    };
    let head = match WindowHead::deserialize(&raw) {
        Ok(head) => head,
        Err(e) => {
            skip(e.to_string());
            return None;
        }
    };
    let wrap: fn(TabStrip) -> WindowKind = match kind {
        KindTag::Main => WindowKind::Main,
        KindTag::Popout => WindowKind::Popout,
    };
    let kind = match decode_strip(&head.id, raw, warnings) {
        Ok(strip) => wrap(strip),
        Err(reason) => {
            warnings.push(DecodeWarning::WindowSkipped {
                index,
                id: Some(head.id),
                reason,
            });
            return None;
        }
    };
    let theme = match &head.theme {
        Value::Null => None,
        raw => match raw.as_str().and_then(UiTheme::from_wire) {
            Some(theme) => Some(theme),
            None => {
                warnings.push(DecodeWarning::ThemeDropped {
                    window_id: head.id.clone(),
                    raw: raw_for_warning(raw),
                });
                None
            }
        },
    };
    let number = numbering::saved_number(&head.number);
    if number.is_none() && !head.number.is_null() && matches!(kind, WindowKind::Popout(_)) {
        warnings.push(DecodeWarning::NumberDropped {
            window_id: head.id.clone(),
            raw: raw_for_warning(&head.number),
        });
    }
    Some(WindowEntry {
        id: head.id,
        number,
        kind,
        theme,
        bounds: head.bounds,
        maximized: head.maximized,
    })
}

// The cap (characters) of the raw text put in a warning — so that even a big value in one field does not make one log line that big.
const WARNING_RAW_MAX_CHARS: usize = 64;

fn raw_for_warning(value: &Value) -> String {
    let raw = value.to_string();
    match raw.char_indices().nth(WARNING_RAW_MAX_CHARS) {
        Some((cut, _)) => format!("{}…", &raw[..cut]),
        None => raw,
    }
}

/// `Err` = the tab fields themselves (`active_tab` · the `tabs` array) cannot be read — the whole window is skipped.
fn decode_strip(
    window_id: &str,
    raw: Value,
    warnings: &mut Vec<DecodeWarning>,
) -> Result<TabStrip, String> {
    let head = StripHead::deserialize(raw).map_err(|e| e.to_string())?;
    let tabs = head
        .tabs
        .into_iter()
        .enumerate()
        .filter_map(|(index, raw)| match TabEntry::deserialize(raw) {
            Ok(tab) => Some(tab),
            Err(e) => {
                warnings.push(DecodeWarning::TabSkipped {
                    window_id: window_id.to_string(),
                    index,
                    reason: e.to_string(),
                });
                None
            }
        })
        .collect();
    Ok(TabStrip {
        active_tab: head.active_tab,
        tabs,
    })
}

#[cfg(test)]
mod tests {
    use serde_json::{json, Value};
    use uuid::Uuid;

    use super::{
        crash_copy_hash, decode, encode, fnv1a_64, fnv1a_hex, DecodeWarning, EncodeError, Unusable,
        STATE_READ_CAP, WARNING_RAW_MAX_CHARS,
    };
    use crate::layout::{SlotContent, SplitDir};
    use crate::state::schema::{
        Bounds, PersistedContent, PersistedNode, StateFile, TabEntry, TabStrip, WindowEntry,
        WindowKind, STATE_VERSION,
    };
    use crate::theme::UiTheme;

    fn bounds() -> Bounds {
        Bounds {
            x: -1920.0,
            y: 60.5,
            w: 1280.0,
            h: 800.0,
        }
    }

    fn known(content: SlotContent) -> PersistedNode {
        PersistedNode::Slot {
            id: Uuid::new_v4(),
            content: PersistedContent::Known(content),
        }
    }

    fn tab(name: &str, layout: PersistedNode) -> TabEntry {
        TabEntry {
            id: Uuid::new_v4(),
            name: name.to_string(),
            named: Some(true),
            focused_slot_id: None,
            layout,
        }
    }

    fn full_state() -> StateFile {
        let focused = Uuid::new_v4();
        let first = TabEntry {
            id: Uuid::new_v4(),
            name: "Empty slot".to_string(),
            named: Some(false),
            focused_slot_id: Some(focused),
            layout: PersistedNode::Split {
                id: Uuid::new_v4(),
                dir: SplitDir::LeftRight,
                ratio: 0.25,
                a: Box::new(PersistedNode::Slot {
                    id: focused,
                    content: PersistedContent::Known(SlotContent::Agent {
                        agent_id: "a-1".to_string(),
                    }),
                }),
                b: Box::new(PersistedNode::Split {
                    id: Uuid::new_v4(),
                    dir: SplitDir::TopBottom,
                    ratio: 0.5,
                    a: Box::new(known(SlotContent::Usage {
                        show_claude: true,
                        show_codex: false,
                    })),
                    b: Box::new(known(SlotContent::Empty)),
                }),
            },
        };
        let second = tab("Tab two", known(SlotContent::AgentList));
        let popout = tab("View 3", known(SlotContent::PresetPalette));
        StateFile {
            version: STATE_VERSION,
            saved_at_ms: 1_759_400_000_000,
            clean_exit: false,
            resolved_crash_copy: Some(crash_copy_hash("{\"version\":1}")),
            windows: vec![
                WindowEntry {
                    id: "main".to_string(),
                    number: Some(1),
                    kind: WindowKind::Main(TabStrip {
                        active_tab: second.id,
                        tabs: vec![first, second],
                    }),
                    theme: Some(UiTheme::Light),
                    bounds: Some(bounds()),
                    maximized: true,
                },
                WindowEntry {
                    id: Uuid::new_v4().to_string(),
                    number: Some(2),
                    kind: WindowKind::Popout(TabStrip {
                        active_tab: popout.id,
                        tabs: vec![popout],
                    }),
                    theme: None,
                    bounds: Some(bounds()),
                    maximized: false,
                },
            ],
        }
    }

    fn main_strip(state: &mut StateFile) -> &mut TabStrip {
        match &mut state.windows[0].kind {
            WindowKind::Main(strip) => strip,
            other => panic!("must be the main window: {other:?}"),
        }
    }

    fn json_of(state: &StateFile) -> Value {
        serde_json::from_str(&encode(state).unwrap()).unwrap()
    }

    fn decode_value(doc: &Value) -> (StateFile, Vec<DecodeWarning>) {
        decode(&doc.to_string()).unwrap()
    }

    fn window(id: &str, kind: &str) -> Value {
        json!({
            "id": id, "kind": kind, "number": 2, "theme": null,
            "bounds": { "x": 0, "y": 0, "w": 800, "h": 600 }, "maximized": false,
        })
    }

    fn tabbed(id: &str, kind: &str, tabs: Vec<Value>) -> Value {
        let mut window = window(id, kind);
        window["active_tab"] = json!(Uuid::new_v4());
        window["tabs"] = Value::Array(tabs);
        window
    }

    /// A readable window — a popout holding one empty tab (the codec does not check whether the popout id is a UUID).
    fn readable(id: &str) -> Value {
        tabbed(id, "popout", vec![tab_json(json!({ "type": "empty" }))])
    }

    fn tab_json(content: Value) -> Value {
        json!({
            "id": Uuid::new_v4(), "name": "t", "named": false, "focused_slot_id": null,
            "layout": { "type": "slot", "id": Uuid::new_v4(), "content": content },
        })
    }

    fn file(windows: Vec<Value>) -> Value {
        json!({ "version": 1, "saved_at_ms": 5, "clean_exit": true, "windows": windows })
    }

    fn skipped_windows(warnings: &[DecodeWarning]) -> Vec<usize> {
        warnings
            .iter()
            .map(|w| match w {
                DecodeWarning::WindowSkipped { index, .. } => *index,
                other => panic!("must be a window skip: {other}"),
            })
            .collect()
    }

    // ── round trip ──

    #[test]
    fn a_full_file_round_trips() {
        let state = full_state();
        let text = encode(&state).unwrap();
        let (back, warnings) = decode(&text).unwrap();
        assert_eq!(warnings, vec![]);
        assert_eq!(back, state);
        assert_eq!(encode(&back).unwrap(), text);
    }

    #[test]
    fn the_file_has_the_trd_shape() {
        let doc = json_of(&full_state());
        let mut top: Vec<&str> = doc
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        top.sort_unstable();
        assert_eq!(
            top,
            [
                "clean_exit",
                "resolved_crash_copy",
                "saved_at_ms",
                "version",
                "windows"
            ]
        );
        assert_eq!(doc["version"], 1);
        let main = &doc["windows"][0];
        assert_eq!(main["kind"], "main");
        assert_eq!(main["number"], 1);
        assert_eq!(main["theme"], "light");
        assert_eq!(
            main["bounds"],
            json!({ "x": -1920.0, "y": 60.5, "w": 1280.0, "h": 800.0 })
        );
        assert_eq!(main["maximized"], true);
        assert!(main["active_tab"].is_string());
        assert_eq!(main["tabs"].as_array().unwrap().len(), 2);
        assert_eq!(main["tabs"][0]["layout"]["type"], "split");
        assert_eq!(main["tabs"][0]["layout"]["dir"], "left_right");
        assert_eq!(main["tabs"][0]["named"], false);
        assert_eq!(main["tabs"][1]["named"], true);

        let popout = &doc["windows"][1];
        assert_eq!(popout["kind"], "popout");
        assert_eq!(popout["number"], 2);
        assert_eq!(popout["theme"], Value::Null);
    }

    #[test]
    fn resolved_crash_copy_is_written_only_when_present() {
        let mut state = full_state();
        let hash = state.resolved_crash_copy.clone().unwrap();
        assert_eq!(json_of(&state)["resolved_crash_copy"], json!(hash));

        state.resolved_crash_copy = None;
        let doc = json_of(&state);
        assert!(!doc.as_object().unwrap().contains_key("resolved_crash_copy"));
        assert_eq!(decode_value(&doc).0.resolved_crash_copy, None);
    }

    #[test]
    fn crash_copy_hash_is_the_fnv1a_of_the_raw_text() {
        let text = "\u{feff}{\"version\":1}\r\n";
        assert_eq!(crash_copy_hash(text), fnv1a_hex(text.as_bytes()));
        assert_ne!(crash_copy_hash(text), crash_copy_hash(&text[3..]));
    }

    #[test]
    fn fnv1a_matches_the_published_vectors() {
        assert_eq!(fnv1a_64(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a_64(b"a"), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(fnv1a_64(b"foobar"), 0x8594_4171_f739_67e8);
        assert_eq!(fnv1a_hex(b""), "cbf29ce484222325");
        assert_eq!(fnv1a_hex(b"a"), "af63dc4c8601ec8c");
        assert_eq!(fnv1a_hex(b"foobar"), "85944171f73967e8");
    }

    // ── unknown slot content (ADR-0060) ──

    #[test]
    fn unknown_slot_content_survives_decode_and_encode() {
        let foreign = json!({
            "type": "file_tree", "root": "C:\\work",
            "opts": { "depth": 3, "list": [1, 2.5, null, true, "x"], "nested": { "k": [] } },
        });
        let malformed_known = json!({ "type": "agent", "agent_ref": 7 });
        let doc = file(vec![tabbed(
            "main",
            "main",
            vec![tab_json(foreign.clone()), tab_json(malformed_known.clone())],
        )]);
        let (mut state, warnings) = decode_value(&doc);
        assert_eq!(warnings, vec![]);
        for tab in &main_strip(&mut state).tabs {
            let PersistedNode::Slot { content, .. } = &tab.layout else {
                panic!("must be a slot")
            };
            assert!(matches!(content, PersistedContent::Unknown(_)));
        }

        let back = json_of(&state);
        assert_eq!(back["windows"][0]["tabs"][0]["layout"]["content"], foreign);
        assert_eq!(
            back["windows"][0]["tabs"][1]["layout"]["content"],
            malformed_known
        );
        assert_eq!(back["windows"][0]["tabs"], doc["windows"][0]["tabs"]);
    }

    // ── window · tab skips ──

    #[test]
    fn an_unknown_window_kind_skips_only_that_window() {
        let popout = Uuid::new_v4().to_string();
        let doc = file(vec![
            tabbed("main", "main", vec![tab_json(json!({ "type": "empty" }))]),
            window("dock-1", "dock"),
            tabbed(
                &popout,
                "popout",
                vec![tab_json(json!({ "type": "agent_list" }))],
            ),
        ]);
        let (state, warnings) = decode_value(&doc);
        let ids: Vec<&str> = state.windows.iter().map(|w| w.id.as_str()).collect();
        assert_eq!(ids, ["main", popout.as_str()]);
        assert!(matches!(
            warnings.as_slice(),
            [DecodeWarning::WindowSkipped { index: 1, id: Some(id), .. }] if id == "dock-1"
        ));
    }

    /// ADR-0225: an old entry of the removed tree-only window (`kind: "tree"`) is skipped as an unknown window kind and the rest read as is — the saved screen is not lost over one old entry.
    #[test]
    fn a_retired_tree_window_entry_is_skipped_and_the_rest_is_kept() {
        let mut tree = window("agent-tree", "tree");
        tree["theme"] = json!("e-ink");
        let doc = file(vec![
            tabbed("main", "main", vec![tab_json(json!({ "type": "empty" }))]),
            tree,
        ]);
        let (state, warnings) = decode_value(&doc);
        let ids: Vec<&str> = state.windows.iter().map(|w| w.id.as_str()).collect();
        assert_eq!(ids, ["main"]);
        assert!(
            matches!(
                warnings.as_slice(),
                [DecodeWarning::WindowSkipped { index: 1, id: Some(id), reason }]
                    if id == "agent-tree" && reason.contains("unknown window kind")
            ),
            "{warnings:?}"
        );
    }

    #[test]
    fn an_unreadable_window_is_skipped() {
        let mut no_kind = readable("w-0");
        no_kind.as_object_mut().unwrap().remove("kind");
        let mut no_maximized = readable("w-1");
        no_maximized.as_object_mut().unwrap().remove("maximized");
        let mut bad_active = tabbed("w-2", "popout", vec![tab_json(json!({ "type": "empty" }))]);
        bad_active["active_tab"] = json!("not-a-uuid");
        let no_tabs = window("w-3", "main");
        let doc = file(vec![
            no_kind,
            no_maximized,
            bad_active,
            no_tabs,
            json!("not a window"),
            readable("w-5"),
        ]);
        let (state, warnings) = decode_value(&doc);
        let ids: Vec<&str> = state.windows.iter().map(|w| w.id.as_str()).collect();
        assert_eq!(ids, ["w-5"]);
        assert_eq!(skipped_windows(&warnings), [0, 1, 2, 3, 4]);
    }

    #[test]
    fn a_bad_tab_skips_only_that_tab() {
        let good = tab_json(json!({ "type": "agent", "agent_id": "a-1" }));
        let mut bad_id = tab_json(json!({ "type": "empty" }));
        bad_id["id"] = json!("not-a-uuid");
        let mut unknown_node = tab_json(json!({ "type": "empty" }));
        unknown_node["layout"] = json!({ "type": "grid", "id": Uuid::new_v4(), "cells": [] });
        let not_object_content = tab_json(json!("empty"));
        let main = tabbed(
            "main",
            "main",
            vec![bad_id, good.clone(), unknown_node, not_object_content],
        );
        let (mut state, warnings) = decode_value(&file(vec![main.clone()]));

        let strip = main_strip(&mut state);
        assert_eq!(strip.tabs.len(), 1);
        assert_eq!(serde_json::to_value(&strip.tabs[0]).unwrap(), good);
        assert_eq!(strip.active_tab.to_string(), main["active_tab"]);
        let skipped: Vec<usize> = warnings
            .iter()
            .map(|w| match w {
                DecodeWarning::TabSkipped {
                    window_id, index, ..
                } if window_id == "main" => *index,
                other => panic!("must be a tab skip of main: {other}"),
            })
            .collect();
        assert_eq!(skipped, [0, 2, 3]);
    }

    // TRD S26-window-chrome §3-3 — even if `named` is not a bool the tab is kept and read as `None` (auto name). No warning.
    #[test]
    fn a_tab_named_flag_that_is_not_a_bool_reads_as_none_and_keeps_the_tab() {
        let flags = [
            None,
            Some(json!(null)),
            Some(json!(true)),
            Some(json!(false)),
            Some(json!("true")),
            Some(json!(1)),
            Some(json!({ "named": true })),
            Some(json!([true])),
        ];
        let tabs = flags
            .iter()
            .map(|flag| {
                let mut tab = tab_json(json!({ "type": "empty" }));
                match flag {
                    None => {
                        tab.as_object_mut().unwrap().remove("named");
                    }
                    Some(raw) => tab["named"] = raw.clone(),
                }
                tab
            })
            .collect();
        let (mut state, warnings) = decode_value(&file(vec![tabbed("main", "main", tabs)]));

        assert_eq!(warnings, vec![]);
        let named: Vec<Option<bool>> = main_strip(&mut state)
            .tabs
            .iter()
            .map(|t| t.named)
            .collect();
        assert_eq!(
            named,
            [None, None, Some(true), Some(false), None, None, None, None]
        );
    }

    #[test]
    fn an_unknown_theme_keeps_the_window_and_drops_the_theme() {
        let mut odd = readable("w-0");
        odd["theme"] = json!("sepia");
        let mut missing = readable("w-1");
        missing.as_object_mut().unwrap().remove("theme");
        let mut set = readable("w-2");
        set["theme"] = json!("dark");
        let (state, warnings) = decode_value(&file(vec![odd, missing, set]));
        let themes: Vec<Option<UiTheme>> = state.windows.iter().map(|w| w.theme).collect();
        assert_eq!(themes, [None, None, Some(UiTheme::Dark)]);
        assert_eq!(
            warnings,
            vec![DecodeWarning::ThemeDropped {
                window_id: "w-0".to_string(),
                raw: "\"sepia\"".to_string(),
            }]
        );
    }

    #[test]
    fn a_number_that_is_not_a_saved_number_keeps_the_window_and_drops_the_number() {
        // The first two (no field · `null`) = a file that did not write numbers — folded without a warning.
        let bad = [
            None,
            Some(Value::Null),
            Some(json!(0)),
            Some(json!(-1)),
            Some(json!(2.5)),
            Some(json!(2.0)),
            Some(json!("3")),
            Some(json!(true)),
            Some(json!(1000)),
            Some(json!(u64::MAX)),
        ];
        let windows: Vec<Value> = bad
            .iter()
            .enumerate()
            .map(|(i, raw)| {
                let mut w = readable(&format!("w-{i}"));
                match raw {
                    None => {
                        w.as_object_mut().unwrap().remove("number");
                    }
                    Some(raw) => w["number"] = raw.clone(),
                }
                w
            })
            .collect();
        let (state, warnings) = decode_value(&file(windows));
        assert_eq!(state.windows.len(), bad.len(), "no window is skipped");
        assert!(state.windows.iter().all(|w| w.number.is_none()));
        assert!(
            state
                .windows
                .iter()
                .all(|w| matches!(&w.kind, WindowKind::Popout(strip) if strip.tabs.len() == 1)),
            "tabs unchanged too"
        );
        let dropped: Vec<(String, String)> = warnings
            .iter()
            .map(|w| match w {
                DecodeWarning::NumberDropped { window_id, raw } => (window_id.clone(), raw.clone()),
                other => panic!("must be a number discard: {other}"),
            })
            .collect();
        let want: Vec<(String, String)> = bad
            .iter()
            .enumerate()
            .skip(2)
            .map(|(i, raw)| (format!("w-{i}"), raw.as_ref().unwrap().to_string()))
            .collect();
        assert_eq!(dropped, want);

        let mut edge = readable("w-max");
        edge["number"] = json!(999);
        let mut main = readable("main");
        main["kind"] = json!("main");
        main["number"] = json!(1);
        let (state, warnings) = decode_value(&file(vec![main, edge]));
        assert_eq!(warnings, vec![]);
        let numbers: Vec<Option<u32>> = state.windows.iter().map(|w| w.number).collect();
        assert_eq!(numbers, [Some(1), Some(999)]);
    }

    #[test]
    fn mains_number_is_never_warned_about() {
        for raw in [json!(0), json!(7), json!("1"), Value::Null] {
            let mut main = readable("main");
            main["kind"] = json!("main");
            main["number"] = raw.clone();
            let (state, warnings) = decode_value(&file(vec![main]));
            assert_eq!(warnings, vec![], "{raw}");
            assert_eq!(state.windows.len(), 1);
        }
    }

    #[test]
    fn a_long_raw_value_is_cut_in_the_warning() {
        let mut long_number = readable("w-0");
        long_number["number"] = json!("x".repeat(500));
        let mut long_theme = readable("w-1");
        long_theme["theme"] = json!("y".repeat(500));
        let mut edge = readable("w-2");
        let edge_raw = json!("z".repeat(WARNING_RAW_MAX_CHARS - 2));
        edge["number"] = edge_raw.clone();
        let (_, warnings) = decode_value(&file(vec![long_number, long_theme, edge]));
        let raws: Vec<&str> = warnings
            .iter()
            .map(|w| match w {
                DecodeWarning::NumberDropped { raw, .. }
                | DecodeWarning::ThemeDropped { raw, .. } => raw.as_str(),
                other => panic!("must be a number · theme discard: {other}"),
            })
            .collect();
        let cut = |c: char| format!("\"{}…", c.to_string().repeat(WARNING_RAW_MAX_CHARS - 1));
        assert_eq!(
            raws,
            [
                cut('x').as_str(),
                cut('y').as_str(),
                edge_raw.to_string().as_str(),
            ],
            "put in as is up to the cap"
        );
    }

    #[test]
    fn missing_or_null_bounds_read_as_no_place() {
        let mut missing = readable("w-0");
        missing.as_object_mut().unwrap().remove("bounds");
        let mut null = readable("w-1");
        null["bounds"] = Value::Null;
        // Non-finite floats become `null` in serialization — a place where one field is `null` is an unreadable window.
        let mut holed = readable("w-2");
        holed["bounds"] = json!({ "x": null, "y": 0, "w": 800, "h": 600 });
        let kept = readable("w-3");
        let (state, warnings) = decode_value(&file(vec![missing, null, holed, kept]));
        let places: Vec<(&str, Option<Bounds>)> = state
            .windows
            .iter()
            .map(|w| (w.id.as_str(), w.bounds))
            .collect();
        assert_eq!(
            places,
            [
                ("w-0", None),
                ("w-1", None),
                (
                    "w-3",
                    Some(Bounds {
                        x: 0.0,
                        y: 0.0,
                        w: 800.0,
                        h: 600.0
                    })
                )
            ]
        );
        assert_eq!(skipped_windows(&warnings), [2]);
    }

    #[test]
    fn a_window_without_a_place_writes_null_and_round_trips() {
        let mut state = full_state();
        state.windows[0].bounds = None;
        let doc = json_of(&state);
        assert_eq!(doc["windows"][0]["bounds"], Value::Null);
        let (back, warnings) = decode_value(&doc);
        assert_eq!(warnings, vec![]);
        assert_eq!(back, state);
    }

    // ── files unusable as a whole ──

    #[test]
    fn a_newer_version_is_unusable() {
        let mut doc = file(vec![]);
        doc["version"] = json!(2);
        assert_eq!(
            decode(&doc.to_string()),
            Err(Unusable::NewerVersion { found: 2 })
        );
        // Even with a different head shape it is a 「newer format」.
        assert_eq!(
            decode(r#"{"version":7,"everything":"else"}"#),
            Err(Unusable::NewerVersion { found: 7 })
        );
        doc["version"] = json!(2.0);
        assert_eq!(
            decode(&doc.to_string()),
            Err(Unusable::NewerVersion { found: 2 })
        );
    }

    #[test]
    fn an_integer_valued_float_version_reads_as_that_integer() {
        let mut doc = file(vec![readable("w-0")]);
        doc["version"] = json!(1.0);
        assert!(doc.to_string().contains("\"version\":1.0"));
        let (state, warnings) = decode_value(&doc);
        assert_eq!((state.version, state.windows.len()), (STATE_VERSION, 1));
        assert_eq!(warnings, vec![]);
    }

    /// ADR-0291 R1: a file without `version` is version 1.
    #[test]
    fn a_missing_version_reads_as_the_first_version() {
        let mut doc = file(vec![readable("w-0")]);
        doc.as_object_mut().unwrap().remove("version");
        let (state, warnings) = decode_value(&doc);
        assert_eq!((state.version, state.windows.len()), (STATE_VERSION, 1));
        assert_eq!(warnings, vec![]);
    }

    fn corrupt_reason(text: &str) -> String {
        match decode(text) {
            Err(Unusable::Corrupt(reason)) => reason,
            other => panic!("must be corrupt: {other:?}"),
        }
    }

    #[test]
    fn not_json_is_unusable() {
        assert!(corrupt_reason("{BROKEN").contains("not JSON"));
        assert!(corrupt_reason("").contains("not JSON"));
    }

    #[test]
    fn json_without_a_state_head_is_unusable() {
        let without = |key: &str| {
            let mut doc = file(vec![]);
            doc.as_object_mut().unwrap().remove(key);
            doc
        };
        let with = |key: &str, value: Value| {
            let mut doc = file(vec![]);
            doc[key] = value;
            doc
        };
        for doc in [
            json!([1, 2]),
            without("saved_at_ms"),
            without("clean_exit"),
            without("windows"),
            with("version", json!(0)),
            with("version", json!(1.5)),
            with("version", json!(-1)),
            with("version", json!(1e20)),
            with("version", json!(18_446_744_073_709_551_616.0_f64)),
            with("version", json!("1")),
            with("windows", json!({})),
            with("resolved_crash_copy", json!(3)),
        ] {
            assert!(
                matches!(decode(&doc.to_string()), Err(Unusable::Corrupt(_))),
                "{doc}"
            );
        }
        assert!(
            corrupt_reason(&without("windows").to_string()).contains("not a state file shape"),
            "the reason for an unreadable head is the shape check's text"
        );
    }

    #[test]
    fn a_bom_is_tolerated() {
        let text = encode(&full_state()).unwrap();
        assert_eq!(
            decode(&format!("\u{feff}{text}")).unwrap(),
            decode(&text).unwrap()
        );
    }

    // ── write cap ──

    #[test]
    fn encode_writes_up_to_the_cap_and_refuses_past_it() {
        let mut state = full_state();
        main_strip(&mut state).tabs[0].name.clear();
        let base = encode(&state).unwrap().len();
        let fill = STATE_READ_CAP as usize - base;

        main_strip(&mut state).tabs[0].name = "x".repeat(fill);
        assert_eq!(encode(&state).unwrap().len() as u64, STATE_READ_CAP);

        main_strip(&mut state).tabs[0].name.push('x');
        assert_eq!(
            encode(&state),
            Err(EncodeError::TooLarge {
                len: STATE_READ_CAP as usize + 1
            })
        );
    }

    /// `depth` levels of splits (always splitting the `a` side again) — a file with one window · one tab.
    fn nested_splits(depth: usize) -> StateFile {
        let mut layout = known(SlotContent::Empty);
        for _ in 0..depth {
            layout = PersistedNode::Split {
                id: Uuid::new_v4(),
                dir: SplitDir::LeftRight,
                ratio: 0.5,
                a: Box::new(layout),
                b: Box::new(known(SlotContent::Empty)),
            };
        }
        let only = tab("Deep tab", layout);
        StateFile {
            version: STATE_VERSION,
            saved_at_ms: 1,
            clean_exit: true,
            resolved_crash_copy: None,
            windows: vec![WindowEntry {
                id: "main".to_string(),
                number: Some(1),
                kind: WindowKind::Main(TabStrip {
                    active_tab: only.id,
                    tabs: vec![only],
                }),
                theme: None,
                bounds: Some(bounds()),
                maximized: false,
            }],
        }
    }

    #[test]
    fn encode_refuses_nesting_that_decode_cannot_read() {
        let deepest_readable = nested_splits(120);
        let text = encode(&deepest_readable).unwrap();
        assert_eq!(decode(&text).unwrap(), (deepest_readable, vec![]));

        let too_deep = nested_splits(121);
        let written = serde_json::to_string_pretty(&too_deep).unwrap();
        assert!(corrupt_reason(&written).contains("not JSON"));
        assert!(matches!(
            encode(&too_deep),
            Err(EncodeError::Unparsable { .. })
        ));
    }
}
