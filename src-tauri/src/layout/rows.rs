//! Layout rows (TRD S28 §4-5) — flattens the model's layout into one row per area, carried by the answers of `layout.get` and of commands that change the layout (rule §1-4). The command bus macro cannot carry recursive types, so rows are flat instead of a tree, and left-right · top-bottom · share travel as normalized coordinates. It only reads the model (`ViewManager`) and knows no locks — the apply service takes the lock (query = `apply::layout_rows`; for a command that changed the layout `apply::dock_target` puts the rows in the outcome inside the very lock that changes it). Row order = window number order (`window.list`) → tab row order → tree preorder; coordinates = `geometry::compute`, the single source of geometry inside the shell (ADR-0227).
// ADR-0307

use uuid::Uuid;

use super::geometry;
use super::manager::{ViewId, ViewManager, WindowLabel};
use super::tab_name::effective_tab_name;
use super::target::{tabs_of, windows_in_order};
use super::tree;
use super::types::SlotContent;

/// What an area holds. `Unknown` = content this version does not know (an area holding raw text read from a saved file — in-memory content is `Empty`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotKind {
    Empty,
    Agent,
    AgentList,
    Usage,
    Unknown,
}

/// One area's row. The id fields (`window_id` · `tab` · `slot` · `agent_id`) are always filled — the caller chooses whether to put them in the answer. `tab_index` = position in that window's tab row (from 0) — tab names can repeat, so it is the key grouping one tab's rows. `agent_id` = the agent id written in the area as is · `agent` = its display name (shell roster — `None` if absent); both only when `content = Agent`. `x` · `y` · `w` · `h` = normalized coordinates inside the tab canvas (0..1), rounded to two decimals.
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

/// Row filter — empty areas are not filtered out. `window` = only rows of that window (label) · `agent` = only rows of areas holding that agent (UUID value comparison — spelling-insensitive).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RowFilter {
    pub window: Option<WindowLabel>,
    pub agent: Option<Uuid>,
}

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

/// Rows of tabs `views` — the "changed tabs" part of a command answer; order is the module head's, not `views` order. A tab not in the model has no rows.
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

    // An area holding unknown content is `Unknown`, not in-memory content (`Empty`) — the raw text of a side table only stands for restore (in-crate door).
    #[test]
    fn a_slot_holding_foreign_content_is_unknown_and_its_neighbour_keeps_its_kind() {
        let mut mgr = ViewManager::new();
        let view = mgr.list_tabs(MAIN_WINDOW_LABEL).unwrap().active;
        let first = tree::first_slot_id(&mgr.views[&view].layout);
        let second = mgr.split_slot(view, first, SplitDir::LeftRight).unwrap();
        let mut raw = Map::new();
        raw.insert("type".into(), Value::String("from-the-future".into()));
        mgr.set_unknown_content(view, first, raw).unwrap();
        mgr.set_slot_content(view, second, SlotContent::AgentList)
            .unwrap();

        let rows = layout_rows(&mgr, &RowFilter::default());

        assert_eq!(
            rows.iter().map(|r| (r.slot, r.content)).collect::<Vec<_>>(),
            [(first, SlotKind::Unknown), (second, SlotKind::AgentList)]
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
