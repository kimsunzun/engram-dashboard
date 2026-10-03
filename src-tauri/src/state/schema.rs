//! `state.json` 의 영속 모양 — 메모리 타입(`layout::ViewManager` 의 창 항목)과 따로 둔다(TRD S21-storage §6-1 ·
//! §6-2).
//!
//! 탭 하나([`TabEntry`])는 `layout::View` 와, 분할 트리([`PersistedNode`])는 `layout::LayoutNode` 와 **같은 JSON**
//! 이다 — 그 둘을 그대로 쓰지 않는 이유는 슬롯 내용 칸 하나다([`PersistedContent`]). 한쪽에 칸을 더하면 다른
//! 쪽도 같이 고친다(시험이 둘을 맞댄다).
//!
//! 읽기의 관용(창 · 탭만 건너뛰기)은 [`super::codec`] 이 진다 — 그래서 파일 · 창 · 탭 묶음(`StateFile` ·
//! `WindowEntry` · `WindowKind` · `TabStrip`)은 직렬화만 derive 하고, 읽기는 그쪽이 조각마다 한다.
//!
//! ★칸 더하기 규칙★ — 빠져도 읽히는 선택 칸을 더하는 것은 `version` 을 올리지 않는다. 옛 빌드가 다시 저장하면
//! 그 칸은 사라진다(내림은 지원하지 않는다 — TRD §12 R3). 잃으면 안 되는 변경이면 `version` 을 올린다.

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map, Value};
use uuid::Uuid;

use crate::layout::{SlotContent, SplitDir};
use crate::ui_settings::UiTheme;

/// 이 셸이 읽고 쓰는 형식 번호 — 이보다 큰 파일은 통째로 못 쓴다([`super::codec::Unusable::NewerVersion`]).
pub const STATE_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StateFile {
    pub version: u32,
    /// 이 스냅숏을 뜬 시각 — 유닉스 시각 ms.
    pub saved_at_ms: u64,
    /// 실행 표식 — `false` = 이 파일을 쓴 실행이 정상 종료를 적지 못했다(TRD §6-1 · §6-6).
    pub clean_exit: bool,
    /// 답한 크래시 사본의 신원([`super::codec::crash_copy_hash`]). `None` 이면 칸 자체를 쓰지 않는다 — 실을
    /// 때는 답한 뒤부터 그 사본을 지울 때까지뿐이다(TRD §6-4).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolved_crash_copy: Option<String>,
    pub windows: Vec<WindowEntry>,
}

/// 창 하나. `id` = 영속 신원 — `main` · `agent-tree` 고정, 팝아웃은 창이 처음 생길 때 뽑은 UUID 다. runtime
/// label 이 아니다(TRD §6-3).
///
/// 같은 `id` 의 중복 · main 부재 같은 모양 검사는 여기서 하지 않는다 — 복원 쪽(`from_persisted`)이 다시 세운다.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct WindowEntry {
    pub id: String,
    #[serde(flatten)]
    pub kind: WindowKind,
    /// `None` = 전역 테마를 따른다(파일에는 `null`).
    #[serde(serialize_with = "theme_wire")]
    pub theme: Option<UiTheme>,
    /// `None` = 이 창의 보통 자리를 본 적이 없다(파일에는 `null`) — 복원은 기본 자리로 연다.
    pub bounds: Option<Bounds>,
    pub maximized: bool,
}

/// 파일에서는 `"kind"` 문자열과 탭 칸(`active_tab` · `tabs`)이 창 항목에 평평하게 놓인다. 트리 창은 탭 칸이
/// 없다(레이아웃 모델 밖 — TRD §6-3).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WindowKind {
    Main(TabStrip),
    Tree,
    Popout(TabStrip),
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TabStrip {
    /// `tabs` 에 없을 수 있다 — 그 탭을 못 읽어 건너뛰었을 때. 고치는 것은 복원 쪽이다(TRD §6-2).
    pub active_tab: Uuid,
    pub tabs: Vec<TabEntry>,
}

/// 마지막 보통(최소화도 최대화도 아닌) 위치 · 크기 — `x` · `y` = 물리 픽셀 바깥 위치, `w` · `h` = 그 창의 배율로
/// 나눈 논리 안쪽 크기(TRD §6-3).
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
    pub name: String,
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

/// 슬롯 내용 — 이 빌드가 아는 것은 [`SlotContent`], 못 읽는 것(새 빌드가 쓴 종류 · 칸 모양이 바뀐 아는 종류)은
/// **원문 객체**다. 원문은 다시 쓸 때 손대지 않고 나간다 — 이 빌드가 저장해도 새 빌드가 쓴 슬롯이 지워지지
/// 않는다.
///
/// - ★지키는 것은 JSON 값이지 바이트가 아니다★ — 다시 쓰면 키가 정렬되고, 64비트를 넘는 정수와 왕복이 안 되는
///   실수는 바뀔 수 있다(serde_json `Map` 이 정렬 지도이고 `arbitrary_precision` 을 켜지 않았다).
/// - ★아는 종류에 붙은 모르는 칸은 `Known` 으로 읽히고 다시 쓸 때 사라진다★ — `SlotContent` 의 serde 가 모르는
///   칸을 무시한다.
/// - 객체가 아닌 값은 어느 쪽도 아니다 — 읽기 실패이고, 그 슬롯을 품은 탭을 못 읽는다.
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
        match SlotContent::deserialize(&raw) {
            Ok(known) if raw.is_object() => Ok(PersistedContent::Known(known)),
            _ => match raw {
                Value::Object(raw) => Ok(PersistedContent::Unknown(raw)),
                _ => Err(D::Error::custom("슬롯 내용이 JSON 객체가 아니다")),
            },
        }
    }
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
    use crate::layout::{LayoutNode, SlotContent, SplitDir, View};

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

    // ★`View` · `LayoutNode` · `SlotContent` 에 칸이나 변형이 늘면 이 둘이 컴파일을 깨뜨린다★ — 늘린 것에 이름을
    // 붙이고 그 이름을 아래 시험의 기대 목록과 맞대기 재료에 싣는다. 선택 칸은 `Some` 인 재료로도 싣는다 —
    // `skip_serializing_if` 로 숨는 칸은 `None` 만으로는 맞대지 못한다.
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
                    SlotContent::PresetPalette => "preset_palette",
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
    fn a_tab_is_the_same_json_as_a_view() {
        let views = [
            View {
                id: Uuid::new_v4(),
                name: "View 1".to_string(),
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
                        split(
                            SplitDir::LeftRight,
                            slot(SlotContent::PresetPalette),
                            slot(SlotContent::Usage {
                                show_claude: false,
                                show_codex: true,
                            }),
                        ),
                    ),
                ),
                focused_slot_id: Some(Uuid::new_v4()),
            },
            View {
                id: Uuid::new_v4(),
                name: "탭 둘".to_string(),
                layout: slot(SlotContent::Empty),
                focused_slot_id: None,
            },
        ];

        let mut seen = BTreeSet::new();
        for view in &views {
            view_shapes(view, &mut seen);
            let wire = serde_json::to_value(view).unwrap();
            let tab = TabEntry::deserialize(wire.clone()).unwrap();
            assert_eq!(serde_json::to_value(&tab).unwrap(), wire);
        }
        assert_eq!(
            seen,
            BTreeSet::from([
                "focused_slot_id=some",
                "focused_slot_id=none",
                "empty",
                "agent",
                "agent_list",
                "preset_palette",
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
