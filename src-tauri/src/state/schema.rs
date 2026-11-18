//! The persisted shape of `state.json` — kept separate from the memory types (the window entries of `layout::ViewManager`) (TRD S21-storage §6-1 · §6-2).
//! One tab ([`TabEntry`]) is the same JSON as `layout::View` apart from the name field, and the split tree ([`PersistedNode`]) as `layout::LayoutNode` — those two are not used as is because of the slot content field ([`PersistedContent`]) and the name field. The name field is text (`name` — the visible name) + a bool (`named`) in the file and `layout::TabName` in memory, unpacked and moved by [`super::convert`] (TRD S26-window-chrome §3-3 — written as text so old builds also show that name). A field added on one side must be fixed on the other too (the tests hold everything outside the name field against each other).
//! Read leniency (skipping only windows · tabs) is [`super::codec`]'s: the file · window · tab groups (`StateFile` · `WindowEntry` · `WindowKind` · `TabStrip`) derive only serialization, and that side reads them piece by piece.
//! Field addition rule: adding an optional field that still reads when missing does not bump `version`; if an old build saves again the field disappears (downgrade unsupported — TRD §12 R3). If the change must not be lost, bump `version` — only together with a reader for the previous version: the codec accepts only `version == STATE_VERSION` files (the shape check of `codec::STATE_SPEC`), so bumping without a reader makes the new build fold previous-version files as corrupt and lose the whole saved screen (TRD §6-1).
//! Rename rule: when changing the serde names the persisted shape follows (fields · variants — including `SplitDir` · `SlotContent` reused as is), keep the old name on the reading side as `#[serde(alias = "old name")]` (otherwise tabs with the old name are skipped · become unknown content · `#[serde(default)]` fields silently return to default). `alias` does not cover the tag key itself (`type` of `tag = "type"`) — do not change it, or change it together with a reader reading both keys (TRD §6-1).

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map, Value};
use uuid::Uuid;

use crate::layout::{SlotContent, SplitDir};
use crate::theme::UiTheme;

pub const STATE_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StateFile {
    pub version: u32,
    pub saved_at_ms: u64,
    /// Run marker: `false` = the run that wrote the file did not record a normal exit (TRD §6-1 · §6-6).
    pub clean_exit: bool,
    /// The identity of the answered crash copy ([`super::codec::crash_copy_hash`]). `None` = the field is not written — carried only from the answer until that copy is removed (TRD §6-4).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolved_crash_copy: Option<String>,
    pub windows: Vec<WindowEntry>,
}

/// One window. `id` = the persisted identity — fixed `main`, and for a popout the UUID drawn when the window was first created; not the runtime label (TRD §6-3). Shape checks (duplicate `id`s · a missing main) are not done here — the restore side (`from_persisted`) re-establishes them.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct WindowEntry {
    pub id: String,
    /// Window number (TRD S26-window-chrome §3-3). A write is always a value (main is 1 too). A read takes only integers in `1..=MAX_SAVED_NUMBER` as `Some` and folds the rest to `None` — the window is kept (`codec`). Restore does not look at main's value (D3).
    pub number: Option<u32>,
    #[serde(flatten)]
    pub kind: WindowKind,
    #[serde(serialize_with = "theme_wire")]
    pub theme: Option<UiTheme>,
    /// `None` = this window's normal position was never seen (`null` in the file); restore opens it at the default position.
    pub bounds: Option<Bounds>,
    pub maximized: bool,
}

/// In the file the `"kind"` string and the tab fields (`active_tab` · `tabs`) lie flat in the window entry. A `kind` this version does not know (including `"tree"` of the tree window ADR-0225 removed) makes the read skip only that window (`codec`).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WindowKind {
    Main(TabStrip),
    Popout(TabStrip),
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TabStrip {
    /// TRD §6-2
    pub active_tab: Uuid,
    pub tabs: Vec<TabEntry>,
}

/// The last normal (neither minimized nor maximized) position · size — `x` · `y` = physical pixel outer position, `w` · `h` = the logical inner size divided by that window's scale (TRD §6-3).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Bounds {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TabEntry {
    pub id: Uuid,
    /// Write = the visible name at that moment (the set name, else the auto name). A read uses this text as the tab name only when `named == Some(true)` (TRD S26-window-chrome §3-3) — an auto tab's text is just the last run's value at that moment, and with `Some(false)` it only seeds the agent name ledger (§4-3 · `convert::agent_name_seed`).
    pub name: String,
    /// Whether the name was set. A write is always a value. A read folds an absent field · `null` · a non-bool to `None` — the tab is not skipped over it. `None` · `Some(false)` = auto name (D16).
    #[serde(default, deserialize_with = "lenient_bool")]
    pub named: Option<bool>,
    pub focused_slot_id: Option<Uuid>,
    pub layout: PersistedNode,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PersistedNode {
    Slot {
        id: Uuid,
        content: PersistedContent,
    },
    Split {
        id: Uuid,
        dir: SplitDir,
        ratio: f64,
        a: Box<PersistedNode>,
        b: Box<PersistedNode>,
    },
}

/// Slot content — what this build knows is a [`SlotContent`]; what it cannot read (a kind a newer build wrote · a known kind whose field shape changed) is the raw object, written back untouched so even this build's save does not erase a slot a newer build wrote.
/// - What is kept is the JSON value, not the bytes: rewriting sorts the keys, and integers over 64 bits and non-round-tripping floats may change (serde_json `Map` is a sorted map and `arbitrary_precision` is not enabled).
/// - Unknown fields attached to a known kind are read as `Known` and vanish when written again (the serde of `SlotContent` ignores unknown fields).
/// - A non-object value is neither — a read failure, and the tab holding that slot cannot be read.
/// - A retired kind ([`RETIRED_KINDS`]) is read as `Known(Empty)` without holding the raw object — it is content this version dropped, not content a newer version wrote, so it is written back as an empty slot.
// ADR-0060
#[derive(Debug, Clone, PartialEq)]
pub enum PersistedContent {
    Known(SlotContent),
    Unknown(Map<String, Value>),
}

impl Serialize for PersistedContent {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            PersistedContent::Known(content) => content.serialize(serializer),
            PersistedContent::Unknown(raw) => raw.serialize(serializer),
        }
    }
}

impl<'de> Deserialize<'de> for PersistedContent {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = Value::deserialize(deserializer)?;
        if is_retired(&raw) {
            return Ok(PersistedContent::Known(SlotContent::Empty));
        }
        match SlotContent::deserialize(&raw) {
            Ok(known) if raw.is_object() => Ok(PersistedContent::Known(known)),
            _ => match raw {
                Value::Object(raw) => Ok(PersistedContent::Unknown(raw)),
                _ => Err(D::Error::custom("slot content is not a JSON object")),
            },
        }
    }
}

/// The `type` strings of retired slot kinds.
// ADR-0314: old data is dropped without asking — held as unknown content, the raw object would survive every save and
//   that slot would stay as "unknown content" with a restore warning.
const RETIRED_KINDS: [&str; 1] = ["preset_palette"];

fn is_retired(raw: &Value) -> bool {
    raw.get("type")
        .and_then(Value::as_str)
        .is_some_and(|kind| RETIRED_KINDS.contains(&kind))
}

fn lenient_bool<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<bool>, D::Error> {
    Ok(Value::deserialize(deserializer)?.as_bool())
}

fn theme_wire<S: Serializer>(theme: &Option<UiTheme>, serializer: S) -> Result<S::Ok, S::Error> {
    theme.map(UiTheme::as_wire).serialize(serializer)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use serde::Deserialize;
    use serde_json::json;
    use uuid::Uuid;

    use super::{PersistedContent, PersistedNode, TabEntry};
    use crate::layout::{LayoutNode, SlotContent, SplitDir, TabName, View};

    fn slot(content: SlotContent) -> LayoutNode {
        LayoutNode::Slot {
            id: Uuid::new_v4(),
            content,
        }
    }

    fn split(dir: SplitDir, a: LayoutNode, b: LayoutNode) -> LayoutNode {
        LayoutNode::Split {
            id: Uuid::new_v4(),
            dir,
            ratio: 0.3,
            a: Box::new(a),
            b: Box::new(b),
        }
    }

    // If fields or variants grow in `View` · `LayoutNode` · `SlotContent` these two break the compile — name what was added and put it in the test's expected list below and the comparison material. Optional fields are also put in with `Some` material: a field hidden by `skip_serializing_if` cannot be compared with `None` alone.
    fn view_shapes(view: &View, seen: &mut BTreeSet<&'static str>) {
        let View {
            id: _,
            name: _,
            layout,
            focused_slot_id,
        } = view;
        seen.insert(match focused_slot_id {
            Some(_) => "focused_slot_id=some",
            None => "focused_slot_id=none",
        });
        node_shapes(layout, seen);
    }

    fn node_shapes(node: &LayoutNode, seen: &mut BTreeSet<&'static str>) {
        match node {
            LayoutNode::Slot { id: _, content } => {
                seen.insert(match content {
                    SlotContent::Empty => "empty",
                    SlotContent::Agent { agent_id: _ } => "agent",
                    SlotContent::AgentList => "agent_list",
                    SlotContent::Usage {
                        show_claude: _,
                        show_codex: _,
                    } => "usage",
                });
            }
            LayoutNode::Split {
                id: _,
                dir,
                ratio: _,
                a,
                b,
            } => {
                seen.insert(match dir {
                    SplitDir::LeftRight => "left_right",
                    SplitDir::TopBottom => "top_bottom",
                });
                node_shapes(a, seen);
                node_shapes(b, seen);
            }
        }
    }

    #[test]
    fn a_tab_is_the_same_json_as_a_view_but_for_the_name() {
        let views = [
            View {
                id: Uuid::new_v4(),
                name: TabName::Auto,
                layout: split(
                    SplitDir::LeftRight,
                    split(
                        SplitDir::TopBottom,
                        slot(SlotContent::Empty),
                        slot(SlotContent::Agent {
                            agent_id: "a-1".to_string(),
                        }),
                    ),
                    split(
                        SplitDir::TopBottom,
                        slot(SlotContent::AgentList),
                        slot(SlotContent::Usage {
                            show_claude: false,
                            show_codex: true,
                        }),
                    ),
                ),
                focused_slot_id: Some(Uuid::new_v4()),
            },
            View {
                id: Uuid::new_v4(),
                name: TabName::Named("Tab two".to_string()),
                layout: slot(SlotContent::Empty),
                focused_slot_id: None,
            },
        ];

        let mut seen = BTreeSet::new();
        for view in &views {
            view_shapes(view, &mut seen);
            let mut wire = serde_json::to_value(view).unwrap();
            assert!(wire.as_object_mut().unwrap().remove("name").is_some());
            // The name field is moved by `convert` (TRD S26-window-chrome §3-3) — plug in the file-side form (text + bool), read it, and check that everything outside the name field comes back as is.
            let mut file = wire.clone();
            file["name"] = json!("Visible name");
            file["named"] = json!(true);
            let tab = TabEntry::deserialize(file).unwrap();
            let mut back = serde_json::to_value(&tab).unwrap();
            let fields = back.as_object_mut().unwrap();
            assert_eq!(fields.remove("name"), Some(json!("Visible name")));
            assert_eq!(fields.remove("named"), Some(json!(true)));
            assert_eq!(back, wire);
        }
        assert_eq!(
            seen,
            BTreeSet::from([
                "focused_slot_id=some",
                "focused_slot_id=none",
                "empty",
                "agent",
                "agent_list",
                "usage",
                "left_right",
                "top_bottom",
            ])
        );
    }

    #[test]
    fn known_content_reads_as_slot_content() {
        let content: PersistedContent =
            serde_json::from_value(json!({ "type": "agent", "agent_id": "a-1" })).unwrap();
        assert_eq!(
            content,
            PersistedContent::Known(SlotContent::Agent {
                agent_id: "a-1".to_string()
            })
        );
    }

    #[test]
    fn unknown_content_keeps_the_raw_object() {
        let raw = json!({
            "type": "file_tree",
            "root": "C:\\work",
            "opts": { "depth": 3, "list": [1, 2.5, null, true, "x"], "big": 18446744073709551615u64 },
        });
        let content: PersistedContent = serde_json::from_value(raw.clone()).unwrap();
        assert!(matches!(content, PersistedContent::Unknown(_)));
        assert_eq!(serde_json::to_value(&content).unwrap(), raw);
    }

    #[test]
    fn a_known_kind_with_an_unreadable_shape_is_kept_raw() {
        for raw in [
            json!({ "type": "agent" }),
            json!({ "type": "usage", "show_claude": "yes" }),
            json!({ "no_type": 1 }),
        ] {
            let content: PersistedContent = serde_json::from_value(raw.clone()).unwrap();
            assert!(matches!(content, PersistedContent::Unknown(_)), "{raw}");
            assert_eq!(serde_json::to_value(&content).unwrap(), raw);
        }
    }

    #[test]
    fn a_non_object_content_is_unreadable() {
        for raw in [json!("empty"), json!(["empty"]), json!(null), json!(3)] {
            assert!(
                serde_json::from_value::<PersistedContent>(raw.clone()).is_err(),
                "{raw}"
            );
        }
    }

    #[test]
    fn an_unknown_node_type_is_unreadable() {
        let raw = json!({ "type": "grid", "id": Uuid::new_v4(), "cells": [] });
        assert!(serde_json::from_value::<PersistedNode>(raw).is_err());
    }
}
