//! 배치 줄(TRD S28 §4-5) — 모델의 배치를 칸마다 한 줄로 편다. `layout.get` 과 배치를 바꾼 명령의 답이 싣는다(규칙
//! §1-4). 명령 버스 매크로가 재귀 타입을 못 실어 트리 대신 평평한 줄이고, 좌우 · 위아래 · 몫은 정규 좌표가 나른다.
//!
//! 모델(`ViewManager`)을 읽기만 하고 락을 모른다 — 락은 적용 서비스가 잡는다(조회 = `apply::layout_rows` · 바꾼 명령의
//! 답 = 바꾸는 그 락 안에서 `apply::dock_target` 이 결말에 싣는다). 줄 순서 =
//! 창 번호 순(`window.list`) → 탭 줄 순 → 트리 전위 순. 좌표 = 셸 안 기하의 단일 출처 `geometry::compute`(ADR-0227).
// ADR-0307

use uuid::Uuid;

use super::geometry;
use super::manager::{ViewId, ViewManager, WindowLabel};
use super::tab_name::effective_tab_name;
use super::target::{tabs_of, windows_in_order};
use super::tree;
use super::types::SlotContent;

/// 칸에 든 것의 종류. `Unknown` = 이 판이 모르는 내용(저장 파일에서 읽은 원문을 쥔 칸 — 메모리 내용은 `Empty`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotKind {
    Empty,
    Agent,
    AgentList,
    PresetPalette,
    Usage,
    Unknown,
}

/// 칸 하나의 줄. id 칸(`window_id` · `tab` · `slot` · `agent_id`)은 늘 채운다 — 답에 실을지는 부르는 쪽이 고른다.
///
/// - `tab_index` = 그 창 탭 줄 자리(0 부터) — 탭 이름은 겹칠 수 있어 같은 탭의 줄을 묶는 열쇠는 이것이다.
/// - `agent_id` = 칸에 적힌 에이전트 id 그대로 · `agent` = 그 표시 이름(셸 장부 — 없으면 `None`). 둘 다
///   `content = Agent` 일 때만.
/// - `x` · `y` · `w` · `h` = 탭 캔버스 안 정규 좌표(0~1) — 소수 둘째 자리로 반올림.
#[derive(Debug, Clone, PartialEq)]
pub struct LayoutRow {
    pub window: WindowLabel,
    pub window_id: String,
    pub window_number: u32,
    pub tab: ViewId,
    pub tab_index: u32,
    pub tab_name: String,
    pub tab_active: bool,
    pub slot: Uuid,
    pub content: SlotKind,
    pub agent_id: Option<String>,
    pub agent: Option<String>,
    pub focused: bool,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// 줄 거르기 — 빈 칸은 거르지 않는다. `window` = 그 창(label)의 줄만 · `agent` = 그 에이전트가 든 칸의 줄만(UUID 값
/// 비교 — 철자 무관).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RowFilter {
    pub window: Option<WindowLabel>,
    pub agent: Option<Uuid>,
}

/// 모델 전체의 줄 중 `filter` 에 드는 것.
pub fn layout_rows(mgr: &ViewManager, filter: &RowFilter) -> Vec<LayoutRow> {
    let mut rows = all_rows(mgr, |label, _| {
        filter.window.as_ref().is_none_or(|want| want == label)
    });
    if let Some(agent) = filter.agent {
        rows.retain(|row| {
            row.agent_id
                .as_deref()
                .and_then(|id| Uuid::parse_str(id).ok())
                == Some(agent)
        });
    }
    rows
}

/// 탭 `views` 의 줄 — 명령 답의 「바뀐 탭」 몫. 순서는 `views` 순이 아니라 모듈 머리의 순서다. 모델에 없는 탭은 줄이
/// 없다.
pub fn rows_of_views(mgr: &ViewManager, views: &[ViewId]) -> Vec<LayoutRow> {
    all_rows(mgr, |_, view| views.contains(&view))
}

fn all_rows(mgr: &ViewManager, keep_tab: impl Fn(&WindowLabel, ViewId) -> bool) -> Vec<LayoutRow> {
    let mut rows = Vec::new();
    for (label, w) in windows_in_order(mgr) {
        for (tab_index, view) in tabs_of(mgr, w) {
            if !keep_tab(label, view.id) {
                continue;
            }
            let tab_name = effective_tab_name(mgr, view);
            for rect in geometry::compute(&view.layout).slots {
                let slot = rect.slot_id;
                let content = tree::find_slot(&view.layout, slot).unwrap_or(&SlotContent::Empty);
                let agent_id = content.agent_id().map(str::to_string);
                rows.push(LayoutRow {
                    window: label.clone(),
                    window_id: w.window_id.clone(),
                    window_number: w.number,
                    tab: view.id,
                    tab_index: u32::try_from(tab_index).unwrap_or(u32::MAX),
                    tab_name: tab_name.clone(),
                    tab_active: w.active == view.id,
                    slot,
                    content: if mgr.unknown_content(view.id, slot).is_some() {
                        SlotKind::Unknown
                    } else {
                        kind(content)
                    },
                    agent: agent_id
                        .as_deref()
                        .and_then(|id| mgr.agent_name(id))
                        .map(str::to_string),
                    agent_id,
                    focused: view.focused_slot_id == Some(slot),
                    x: round2(rect.x0),
                    y: round2(rect.y0),
                    w: round2(rect.x1 - rect.x0),
                    h: round2(rect.y1 - rect.y0),
                });
            }
        }
    }
    rows
}

fn kind(content: &SlotContent) -> SlotKind {
    match content {
        SlotContent::Empty => SlotKind::Empty,
        SlotContent::Agent { .. } => SlotKind::Agent,
        SlotContent::AgentList => SlotKind::AgentList,
        SlotContent::PresetPalette => SlotKind::PresetPalette,
        SlotContent::Usage { .. } => SlotKind::Usage,
    }
}

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

#[cfg(test)]
mod tests {
    use serde_json::{Map, Value};

    use super::*;
    use crate::layout::types::SplitDir;
    use crate::layout::MAIN_WINDOW_LABEL;

    // 모르는 내용을 쥔 칸은 메모리 내용(`Empty`)이 아니라 `Unknown` 이다 — 곁표 원문은 복원만 세운다(crate 안 문).
    #[test]
    fn a_slot_holding_foreign_content_is_unknown_and_its_neighbour_keeps_its_kind() {
        let mut mgr = ViewManager::new();
        let view = mgr.list_tabs(MAIN_WINDOW_LABEL).unwrap().active;
        let first = tree::first_slot_id(&mgr.views[&view].layout);
        let second = mgr.split_slot(view, first, SplitDir::LeftRight).unwrap();
        let mut raw = Map::new();
        raw.insert("type".into(), Value::String("from-the-future".into()));
        mgr.set_unknown_content(view, first, raw).unwrap();
        mgr.set_slot_content(view, second, SlotContent::PresetPalette)
            .unwrap();

        let rows = layout_rows(&mgr, &RowFilter::default());

        assert_eq!(
            rows.iter().map(|r| (r.slot, r.content)).collect::<Vec<_>>(),
            [
                (first, SlotKind::Unknown),
                (second, SlotKind::PresetPalette)
            ]
        );
        assert_eq!(rows[0].agent_id, None);
    }

    #[test]
    fn coordinates_round_to_two_decimals() {
        assert_eq!(round2(1.0 / 3.0), 0.33);
        assert_eq!(round2(2.0 / 3.0), 0.67);
        assert_eq!(round2(0.5), 0.5);
    }
}
