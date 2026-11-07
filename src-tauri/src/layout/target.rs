//! Resolving the target (TRD S28 §5-2) — resolves a target that an LLM command picked by id · agent · spatial word into (window, tab, area) of the current model.
//!
//! ★Call it inside the same lock that the apply service changes under★ — if you resolve, release the lock, and take it again to apply, another command ·
//! the mouse may have moved that agent in between and the wrong tab · area gets changed. So it only reads the model (`ViewManager`) and knows nothing of locks.
//!
//! Resolving an agent **name** → id is not its job — a single resolver of the daemon registry resolves it (ADR-0132 · rule §1-2). The
//! agent that arrives here is already an id. The shell's name ledger (`agent_names`) is used only to show the candidate · placement lines.
//!
//! Order (candidates · lines) is the same as `window.list`: window number order → tab row order → tree pre-order.
// ADR-0307

use uuid::Uuid;

use super::bands::{self, SizeCss};
use super::dock::{DockAnchor, DockSide};
use super::geometry::{self, PxRect};
use super::manager::{CanvasPx, ViewId, ViewManager, WindowLabel, WindowTabs, MAIN_WINDOW_LABEL};
use super::spatial::{resolve_spatial, SpatialToken};
use super::tree;
use super::types::{SlotContent, View};

/// Upper bound of the candidate · name lists (rule §1-3 — only as many as can be fixed in one next call).
pub const MAX_CANDIDATES: usize = 5;

/// The share used when the size of the area to compute the attach share from is unknown (the webview has not reported the canvas yet) (TRD S28 §4-2 · §11-2).
pub const UNMEASURED_SHARE: f64 = 0.5;

/// The target a command picked. The id in the agent branch is one the daemon resolved, and it is compared with the area's agent id by UUID value (spelling does not matter).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// Tab given by id.
    Tab(ViewId),
    /// The tab holding that agent — even if it appears in several areas of one tab it is that one tab. If several tabs, it is ambiguous.
    TabOf(Uuid),
    /// Area given by id — if `view` is given, it must be an area of that tab.
    Slot { view: Option<ViewId>, slot: Uuid },
    /// The area holding that agent — if several areas (even within the same tab), it is ambiguous.
    SlotOf(Uuid),
    /// The area a word picks in the visible tab of window `window` (label — the caller has already resolved the window argument · `None` = main)
    /// (same rule as `slot.resolveSpatial` · ADR-0068).
    At {
        token: SpatialToken,
        window: Option<WindowLabel>,
    },
}

/// The resolved target. `slot` = that area if the target picks an area (`Slot` · `SlotOf` · `At`), `None` if the target picks a tab.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Located {
    pub window: WindowLabel,
    pub view: ViewId,
    pub slot: Option<Uuid>,
}

/// One area where an agent is shown — a candidate for ambiguity · one line of [`agent_places`]. `tab_index` = its position in that window's tab row (from 0).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Place {
    pub window: WindowLabel,
    pub window_id: String,
    pub number: u32,
    pub view: ViewId,
    pub tab_index: usize,
    pub slot: Uuid,
}

/// By what words the target was picked — used to tell bus error codes apart (TRD S28 §5-2 · §5-3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetBy {
    Id,
    Agent,
    Spatial,
}

/// The target could not be resolved — the model is unchanged. Message = Korean reason + kind word in parentheses (same shape as `DockError`). Candidates are
/// not put into the message — the caller takes them out with [`Self::candidates`] and appends them.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LocateError {
    #[error("탭 없음: {0} (TabNotFound)")]
    TabNotFound(ViewId),
    #[error("칸 없음: {0} (SlotNotFound)")]
    SlotNotFound(Uuid),
    #[error("칸 {slot} 은 탭 {tab} 소속이 아님 (SlotNotInTab)")]
    SlotNotInTab { slot: Uuid, tab: ViewId },
    /// That agent is in no area. `shown` = display names of the agents currently shown in areas (the id if not in the ledger) —
    /// up to [`MAX_CANDIDATES`].
    #[error("에이전트 {agent} 가 어느 칸에도 없음 (AgentNotShown)")]
    AgentNotShown { agent: Uuid, shown: Vec<String> },
    /// That agent is shown in several areas (several tabs if the target picks a tab). `places` = all those areas.
    #[error("에이전트 {agent} 가 여러 곳에 떠 있음 (AgentAmbiguous)")]
    AgentAmbiguous { agent: Uuid, places: Vec<Place> },
    #[error("창 없음: {0} (WindowNotFound)")]
    WindowNotFound(WindowLabel),
    /// The word did not pick an area in that window's visible tab (no neighbor in that direction, etc.).
    #[error("'{}' 이 창 {window} 에서 칸을 못 집음 (NoSlotAt)", .token.as_str())]
    NoSlotAt {
        token: SpatialToken,
        window: WindowLabel,
    },
}

impl LocateError {
    pub fn by(&self) -> TargetBy {
        match self {
            LocateError::TabNotFound(_)
            | LocateError::SlotNotFound(_)
            | LocateError::SlotNotInTab { .. } => TargetBy::Id,
            LocateError::AgentNotShown { .. } | LocateError::AgentAmbiguous { .. } => {
                TargetBy::Agent
            }
            LocateError::WindowNotFound(_) | LocateError::NoSlotAt { .. } => TargetBy::Spatial,
        }
    }

    /// A hint to fix it in one next call (English — attached as is to the end of the bus message `— candidates: …` · TRD S28 §5-3). Unknown name
    /// = names of the agents that are shown · ambiguous = the `slot_id` of those areas · tab held (`in_tab=` — so it is not confused with the tab id argument `tab`) · window.
    /// Up to [`MAX_CANDIDATES`]; other errors give an empty list.
    /// The head of an area id is `slot_id=` — for another argument name use [`Self::candidates_as`].
    pub fn candidates(&self) -> Vec<String> {
        self.candidates_as("slot_id")
    }

    /// Same as [`Self::candidates`] but sets the head of the area id of an ambiguity candidate to `key=` — if you use the argument name that will receive that value (`slot_id` ·
    /// `beside_slot`), the next call copies it over as is.
    pub fn candidates_as(&self, key: &str) -> Vec<String> {
        match self {
            LocateError::AgentNotShown { shown, .. } => {
                shown.iter().take(MAX_CANDIDATES).cloned().collect()
            }
            LocateError::AgentAmbiguous { places, .. } => places
                .iter()
                .take(MAX_CANDIDATES)
                .map(|p| {
                    format!(
                        "{key}={} (in_tab={} · window={} · window_id={})",
                        p.slot, p.view, p.window, p.window_id
                    )
                })
                .collect(),
            _ => Vec::new(),
        }
    }
}

/// Resolve `target` in the current model — only tabs in the window's tab list are looked at (a temporary tab not yet in a window is not a target).
// ADR-0307
pub fn locate(mgr: &ViewManager, target: &Target) -> Result<Located, LocateError> {
    match target {
        Target::Tab(view) => {
            let window = tab_window(mgr, *view).ok_or(LocateError::TabNotFound(*view))?;
            Ok(Located {
                window,
                view: *view,
                slot: None,
            })
        }
        Target::TabOf(agent) => {
            let places = agent_places(mgr, *agent);
            let first = one_or_ambiguous(mgr, *agent, &places, |a, b| a.view == b.view)?;
            Ok(Located {
                window: first.window.clone(),
                view: first.view,
                slot: None,
            })
        }
        Target::Slot {
            view: Some(view),
            slot,
        } => {
            let window = tab_window(mgr, *view).ok_or(LocateError::TabNotFound(*view))?;
            if tree::contains_slot(&mgr.views[view].layout, *slot) {
                return Ok(Located {
                    window,
                    view: *view,
                    slot: Some(*slot),
                });
            }
            Err(match slot_home(mgr, *slot) {
                Some(_) => LocateError::SlotNotInTab {
                    slot: *slot,
                    tab: *view,
                },
                None => LocateError::SlotNotFound(*slot),
            })
        }
        Target::Slot { view: None, slot } => {
            let (window, view) = slot_home(mgr, *slot).ok_or(LocateError::SlotNotFound(*slot))?;
            Ok(Located {
                window,
                view,
                slot: Some(*slot),
            })
        }
        Target::SlotOf(agent) => {
            let places = agent_places(mgr, *agent);
            let first = one_or_ambiguous(mgr, *agent, &places, |a, b| a.slot == b.slot)?;
            Ok(Located {
                window: first.window.clone(),
                view: first.view,
                slot: Some(first.slot),
            })
        }
        Target::At { token, window } => {
            let label = window.as_deref().unwrap_or(MAIN_WINDOW_LABEL);
            let shown = mgr
                .windows
                .get(label)
                .and_then(|w| mgr.views.get(&w.active))
                .ok_or_else(|| LocateError::WindowNotFound(label.to_string()))?;
            let slot =
                resolve_spatial(&shown.layout, shown.focused_slot_id, *token).ok_or_else(|| {
                    LocateError::NoSlotAt {
                        token: *token,
                        window: label.to_string(),
                    }
                })?;
            Ok(Located {
                window: label.to_string(),
                view: shown.id,
                slot: Some(slot),
            })
        }
    }
}

// If places gather into one, its first place; if none, `AgentNotShown`; if they split, `AgentAmbiguous`. `same` = two places counted as the same target.
fn one_or_ambiguous<'p>(
    mgr: &ViewManager,
    agent: Uuid,
    places: &'p [Place],
    same: impl Fn(&Place, &Place) -> bool,
) -> Result<&'p Place, LocateError> {
    let first = places.first().ok_or_else(|| LocateError::AgentNotShown {
        agent,
        shown: shown_agents(mgr, MAX_CANDIDATES),
    })?;
    if places.iter().all(|p| same(p, first)) {
        Ok(first)
    } else {
        Err(LocateError::AgentAmbiguous {
            agent,
            places: places.to_vec(),
        })
    }
}

/// All areas where agent `agent` is shown — window number order → tab row order → tree pre-order.
pub fn agent_places(mgr: &ViewManager, agent: Uuid) -> Vec<Place> {
    let mut out = Vec::new();
    for (label, w) in windows_in_order(mgr) {
        for (tab_index, view) in tabs_of(mgr, w) {
            for slot in tree::slot_ids(&view.layout) {
                let content = tree::find_slot(&view.layout, slot);
                if content.is_some_and(|c| holds(c, agent)) {
                    out.push(Place {
                        window: label.clone(),
                        window_id: w.window_id.clone(),
                        number: w.number,
                        view: view.id,
                        tab_index,
                        slot,
                    });
                }
            }
        }
    }
    out
}

/// Display names of the agents currently shown in areas (the id if not in the ledger) — up to `max` without duplicates · order per the module header.
pub fn shown_agents(mgr: &ViewManager, max: usize) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for (_, w) in windows_in_order(mgr) {
        for (_, view) in tabs_of(mgr, w) {
            for slot in tree::slot_ids(&view.layout) {
                let Some(id) = tree::find_slot(&view.layout, slot).and_then(SlotContent::agent_id)
                else {
                    continue;
                };
                let name = mgr.agent_name(id).unwrap_or(id).to_string();
                if !out.contains(&name) {
                    out.push(name);
                    if out.len() >= max {
                        return out;
                    }
                }
            }
        }
    }
    out
}

/// Default share for seam docking — the same formula as the mouse, [`bands::attach_share`] (original size = canvas of the source tab's window · area = target area
/// px frame, or the whole target window canvas for `Outer`). If either canvas is unknown or there is no target area, [`UNMEASURED_SHARE`].
///
/// ★Differs from the mouse★: the mouse's original size is the drag size measured by the web (the restored size if the source window is maximized), and the model has
/// no such input, so the canvas stands in (TRD S28 §4-2 · §11-6).
pub fn seam_share(
    mgr: &ViewManager,
    source: ViewId,
    target: ViewId,
    anchor: DockAnchor,
    side: DockSide,
) -> f64 {
    let (Some(original), Some(canvas)) = (canvas_of(mgr, source), canvas_of(mgr, target)) else {
        return UNMEASURED_SHARE;
    };
    let frame = match anchor {
        DockAnchor::Outer => PxRect {
            x0: 0,
            y0: 0,
            x1: canvas.w,
            y1: canvas.h,
        },
        DockAnchor::Slot(slot) => {
            let rect = mgr.views.get(&target).and_then(|v| {
                geometry::compute(&v.layout)
                    .slots
                    .into_iter()
                    .find(|r| r.slot_id == slot)
            });
            match rect {
                Some(rect) => geometry::frame_px(canvas.w, canvas.h, &rect),
                None => return UNMEASURED_SHARE,
            }
        }
    };
    let original = SizeCss {
        w: f64::from(original.w),
        h: f64::from(original.h),
    };
    bands::attach_share(original, side, frame)
}

fn canvas_of(mgr: &ViewManager, view: ViewId) -> Option<CanvasPx> {
    mgr.view_owner
        .get(&view)
        .and_then(|label| mgr.windows.get(label))
        .and_then(|w| w.canvas)
}

/// Windows of the model — in window number order (same as `window.list`).
pub(crate) fn windows_in_order(mgr: &ViewManager) -> Vec<(&WindowLabel, &WindowTabs)> {
    let mut windows: Vec<_> = mgr.windows.iter().collect();
    windows.sort_by_key(|(_, w)| w.number);
    windows
}

/// Tabs of window `w` — (position in the tab row, tab). Entries with no View are skipped (there are none if invariant 1 holds).
pub(crate) fn tabs_of<'m>(
    mgr: &'m ViewManager,
    w: &'m WindowTabs,
) -> impl Iterator<Item = (usize, &'m View)> + 'm {
    w.tabs
        .iter()
        .enumerate()
        .filter_map(|(i, id)| mgr.views.get(id).map(|v| (i, v)))
}

fn holds(content: &SlotContent, agent: Uuid) -> bool {
    content
        .agent_id()
        .and_then(|id| Uuid::parse_str(id).ok())
        .is_some_and(|id| id == agent)
}

// The window that has `view` as a tab — the View · owner pair · tab list must all hold (same rule as the docking decision of `ViewManager`).
fn tab_window(mgr: &ViewManager, view: ViewId) -> Option<WindowLabel> {
    if !mgr.views.contains_key(&view) {
        return None;
    }
    let label = mgr.view_owner.get(&view)?;
    mgr.windows
        .get(label)?
        .tabs
        .contains(&view)
        .then(|| label.clone())
}

// The tab holding area `slot` (only those in a window's tab list) — (window, tab).
fn slot_home(mgr: &ViewManager, slot: Uuid) -> Option<(WindowLabel, ViewId)> {
    windows_in_order(mgr).into_iter().find_map(|(label, w)| {
        tabs_of(mgr, w)
            .find(|(_, v)| tree::contains_slot(&v.layout, slot))
            .map(|(_, v)| (label.clone(), v.id))
    })
}
