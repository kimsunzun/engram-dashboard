//! ViewManager — the layout authority state (ADR-0035 partial amendment · ADR-0057 tab ownership model). LayoutState
//! owns it as `Arc<Mutex<ViewManager>>`.
//!
//! ★0 Tauri dependencies★: this type knows nothing of locks · emit. Taking/releasing locks · emit are done by the command layer
//! (`commands/layout.rs`). So the mutation methods only **return** the change result
//! (affected view_id · updated snapshot · tab list) and do not emit directly here →
//! unit-testable alone (headless).
//!
//! invalid view_id/slot_id/window → no-op + Err(LayoutError) (no panic · no partial change, TRD hard contract).
//!
//! ## ★Tab ownership model (ADR-0057, TRD B-tabs §2)★
//! A window owns a **tab list** (= several `View`s in the code) and switches among them. There is no global active view (the old
//! `active_view_id`) · window binding (the old `window_bindings`). Instead:
//! - `views`      — the global View pool (id lookup).
//! - `view_owner` — View → owning window (★unique ownership enforced★, a cached reverse index).
//! - `windows`    — window → tab list (`tabs: Vec<ViewId>`) + that window's active tab (`active`).
//!
//! ### Invariants (★load-bearing — pinned with `// ADR-0057` anchors★)
//! 1. **Bidirectional consistency:** `view_owner[v] == L` ⟺ `windows[L].tabs.contains(v)`. Always updated as a pair.
//! 2. **Unique ownership:** every `v ∈ views` has exactly 1 entry in `view_owner` (one View in two windows is forbidden).
//!    Exception: the tmp_view made by `prepare_detached_view` has no owner until the phase C insert/rollback.
//! 3. **Active membership:** `windows[L].active ∈ windows[L].tabs` always.
//! 4. **Main has at least 1 tab + non-closable:** `windows["main"].tabs.len() >= 1` is invariant. `close_window("main")`
//!    is forbidden (refused with `MainNotClosable`) — when the last tab leaves (close · docking into another window), an empty tab
//!    is forced up within the same operation.
//! 5. **Multiple agent references allowed:** the same `agent_id` can be assigned to slots of two different Views (two windows watch the same
//!    agent, progress independent · ADR-0046). A different matter from "one View in two windows" (forbidden by invariant 2).
//!
//! ## ★Side table of unknown slot content★
//! Slot content is written only through `write_slot_content`, and views are removed only through `remove_view`; those two and `close_slot` reap the `unknown_content` side table entries. Calling `tree::set_in_tree` directly or removing from `views` directly revives old raw text in a slot the user changed when saving (§12 R6). The free/occupied decision is `slot_is_free` alone.
//!
//! ## ★Window numbers★
//! Window entries go in only through `insert_window_entry` and out only through `retire_window_entry`. The number is computed with `numbering` before inserting (new popout = `insert_new_popout` · boot restore = `from_restored` · copy accept = `adopt_restored`) and freed in three places: `retire_window_entry(Os::Gone)` · `release_closing` · emptying `closing` in `adopt_restored`. If the OS window remains after a popout is taken out (`Os::Alive`), its number is held in `closing` until that window's `Destroyed`, so a new window in between never shows the same number (D13). Calling `insert` · `remove` on `windows` directly makes numbers overlap or leak. (TRD S21-storage §6-2; TRD S26-window-chrome §2-4 ① · §4-1)

use std::collections::{HashMap, HashSet};

use serde_json::{Map, Value};
use uuid::Uuid;

use super::dock::{self, DockAnchor, DockSide, DockTreeError};
use super::geometry::{self, has_area, PxRect, RectF64};
use super::numbering;
use super::tab_name::effective_tab_name;
use super::tree;
use super::types::{
    LayoutNode, SlotContent, SplitDir, SplitRatioOutcome, TabName, UiMetrics, View, ViewMeta,
    ViewSnapshot,
};
use crate::theme::UiTheme;
use crate::ui_text::Lang;

pub const MAIN_WINDOW_LABEL: &str = "main";

// The global View identifier (a global UUID, so tabs can move between windows and be saved/restored — ADR-0057).
pub type ViewId = Uuid;
pub type WindowLabel = String;

// An invalid id is a no-op + this error (no partial change).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LayoutError {
    #[error("view not found: {0}")]
    ViewNotFound(Uuid),
    #[error("slot not found: {0}")]
    SlotNotFound(Uuid),
    #[error("window not found: {0}")]
    WindowNotFound(String),
    #[error("window closed: {0}")]
    WindowClosed(String),
    #[error("the main window cannot be closed")]
    MainNotClosable,
    #[error("ui metrics refused: {0}")]
    InvalidMetrics(String),
    // ADR-0227
    #[error("split not found: {0}")]
    SplitNotFound(Uuid),
    // Why the value is carried as a string: holding an `f64` would break this enum's `Eq`.
    #[error("ratio refused: {0} — must be a finite number")]
    InvalidRatio(String),
    #[error("the area is too small to split further — the new boundary cannot be represented inside that area (floating-point limit). Widen the ratio of the split holding this area or split another area")]
    SplitTooDeep,
    #[error("the tab name is empty — empty text · whitespace-only names are not accepted")]
    EmptyName,
}

/// The result of `ViewManager::set_split_ratio` — `ratio` is the value the shell holds now (just written if `Applied`, otherwise the untouched original).
// ADR-0227
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SplitRatioResult {
    pub ratio: f64,
    pub outcome: SplitRatioOutcome,
}

// Defense line for reported metrics — not policy values; the actual border width · minimum area size are decided by the screen. (TRD §2d)
const INSET_MAX_PX: f64 = 64.0;
const MIN_PANE_PX_MAX: u32 = 1000;

/// The size of a window's tab content area (integer CSS px); all tabs of that window are drawn overlapping in it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CanvasPx {
    pub w: u32,
    pub h: u32,
}

/// The px rectangle of one area — `frame` = the area frame (integer CSS px from the canvas origin) · `content` = the frame reduced by its inner padding (width · height ≥ 0).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SlotPx {
    pub frame: PxRect,
    pub content: RectF64,
}

/// The window's last normal (neither minimized nor maximized) position. `x`·`y` = physical outer position (`outer_position` as is), `w`·`h` = logical inner size (`inner_size` divided by the window's scale); restore places it with the same pair (why the position is not logical: the `state::placement` head; TRD S21-storage §6-3).
///
/// The four values are always finite and `w` · `h` > 0 — only [`Self::new`] creates it. Non-finite floats go out to JSON as `null`, and a window of size 0 is not visible even when restored.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WindowBounds {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

impl WindowBounds {
    /// `None` if any is not finite (NaN · ±∞) or `w` · `h` is 0 or less.
    pub fn new(x: f64, y: f64, w: f64, h: f64) -> Option<Self> {
        let finite = [x, y, w, h].iter().all(|v| v.is_finite());
        (finite && w > 0.0 && h > 0.0).then_some(WindowBounds { x, y, w, h })
    }

    pub fn x(self) -> f64 {
        self.x
    }

    pub fn y(self) -> f64 {
        self.y
    }

    pub fn w(self) -> f64 {
        self.w
    }

    pub fn h(self) -> f64 {
        self.h
    }
}

/// One set read with the window getters (`is_minimized` · `is_maximized` · position). ★All read before taking the `ViewManager` lock★ — no OS calls while holding the layout lock (`apply.rs` head).
///
/// `bounds` = the position read then — `None` if it could not be read (getter failure · [`WindowBounds::new`] refusal). (TRD S21-storage §6-3)
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WindowPlacement {
    pub minimized: bool,
    pub maximized: bool,
    pub bounds: Option<WindowBounds>,
}

/// One window's screen attributes; they live and die with the window entry. Kept outside the window keyed by label, they would silently apply after a restart to a different window with the same label (ADR-0167).
///
/// Held by `WindowTabs::attrs`; only `ViewManager` setters change them, bumping `ViewManager::attrs_rev` (not the layout `version`, so dragging a window does not make the layout number jump).
// ADR-0265
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct WindowAttrs {
    /// `None` = follows the global theme (setting `theme.default`).
    pub theme: Option<UiTheme>,
    /// `None` = this window's normal position has not been seen yet.
    pub bounds: Option<WindowBounds>,
    /// Restore places it with `bounds` and then maximizes (user decision F8).
    pub maximized: bool,
}

impl WindowAttrs {
    /// Records one set of getters — `true` if changed. `memo` = this window's memory of the immediately previous read ([`PlacementMemo`]).
    ///
    /// - While minimized nothing changes: unminimizing returns to the state before it (including whether maximized), which is what the next restore sets up.
    /// - If maximized, only that marker is set and `bounds` stays at the normal position before maximizing (F8). ★But if the position the immediately previous read recorded as normal equals the position read now, that read is reverted★: when the user maximizes, tao emits `Moved` before the maximize marker is set (tao 0.35 `platform_impl/windows/event_loop.rs`: `Moved` at `WM_WINDOWPOSCHANGED`, then `WM_SIZE` sets the marker), so the maximized rectangle was recorded once as normal. tauri-plugin-window-state does the same via `prev_x` · `prev_y`. ★Do not drop the "same position" condition★: tao's `maximize` sets the marker first so there is no wrongly recorded read, and reverting without the condition would discard the real normal read (e.g. a size the boot restore applied).
    /// - If normal, the marker is lowered and `bounds` is recorded (if `None`, the previous value stays).
    pub(crate) fn observe(&mut self, seen: WindowPlacement, memo: &mut PlacementMemo) -> bool {
        let before = *self;
        let WindowPlacement {
            minimized,
            maximized,
            bounds,
        } = seen;
        let last = memo.last.take();
        if !minimized {
            self.maximized = maximized;
            if maximized {
                if let (Some((written, prior)), Some(now)) = (last, bounds) {
                    if written == now {
                        self.bounds = prior;
                    }
                }
            } else if let Some(bounds) = bounds {
                memo.last = Some((bounds, self.bounds));
                self.bounds = Some(bounds);
            }
        }
        *self != before
    }

    /// `true` if changed.
    pub(crate) fn set_theme(&mut self, theme: Option<UiTheme>) -> bool {
        let changed = self.theme != theme;
        self.theme = theme;
        changed
    }
}

/// One window's memory of the immediately previous read: (recorded position, the position before it) if that read recorded a normal position. Only [`WindowAttrs::observe`] uses it and the next read always clears it. Not persisted.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub(crate) struct PlacementMemo {
    last: Option<(WindowBounds, Option<WindowBounds>)>,
}

#[derive(Debug, Clone)]
pub struct WindowTabs {
    /// Tab order (left → right).
    pub tabs: Vec<ViewId>,
    pub active: ViewId,
    // Both are None until that window's webview reports, and vanish with the window entry. Per window, not per tab.
    // ADR-0227
    pub canvas: Option<CanvasPx>,
    // ADR-0227
    pub metrics: Option<UiMetrics>,
    /// Persisted identity — `main` for main, for a popout the UUID (lowercase with hyphens) drawn when its entry was first created. Save · restore use it; the bus · invoke window argument also accepts it (`super::window_arg`). The runtime label (map key) is an identity only within one boot (popout labels restart from 1 with a new run marker).
    // ADR-0167 (TRD S21-storage §6-3 · S26-window-chrome §4-5)
    pub window_id: String,
    /// Window number — main = 1, popouts ≥ 2, not overlapping among live and closing windows. Does not change while the entry is in the model. (TRD S26-window-chrome §4-1)
    pub number: u32,
    pub attrs: WindowAttrs,
    pub(crate) placement_memo: PlacementMemo,
}

impl WindowTabs {
    // Every new window entry is created here — measured values must stay empty until that window's webview reports.
    fn new(
        window_id: String,
        number: u32,
        attrs: WindowAttrs,
        tabs: Vec<ViewId>,
        active: ViewId,
    ) -> Self {
        WindowTabs {
            tabs,
            active,
            canvas: None,
            metrics: None,
            window_id,
            number,
            attrs,
            placement_memo: PlacementMemo::default(),
        }
    }

    fn new_main(attrs: WindowAttrs, tabs: Vec<ViewId>, active: ViewId) -> Self {
        Self::new(
            MAIN_WINDOW_LABEL.to_string(),
            numbering::MAIN_WINDOW_NUMBER,
            attrs,
            tabs,
            active,
        )
    }

    fn new_popout(number: u32, view: ViewId) -> Self {
        Self::new(
            Uuid::new_v4().to_string(),
            number,
            WindowAttrs::default(),
            vec![view],
            view,
        )
    }
}

// Whether the OS window still exists when the entry is taken out: decides between holding the number in `closing` and freeing it right away. (TRD S26-window-chrome §4-1)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Os {
    Alive,
    Gone,
}

// One popout that left the model whose OS window has not emitted `Destroyed` yet. (TRD S26-window-chrome §3-1 · §4-1)
#[derive(Debug, Clone)]
struct Closing {
    // The 0-tab self-close finds this table by persisted id ([`ViewManager::closing_label_for`]). (TRD §5-5)
    window_id: String,
    number: u32,
}

/// One window revived from persistence (memory shape); `state::convert` has already parsed and repaired the persisted shape.
#[derive(Debug, Clone)]
pub(crate) struct RestoredWindow {
    pub attrs: WindowAttrs,
    pub tabs: Vec<View>,
    /// If `tabs` is present, it must be one of them.
    pub active: Option<ViewId>,
    /// The saved number (after lenient reading — `None` = absent or unusable value). Only popouts' are looked at — main is always 1 (D3).
    pub number: Option<u32>,
}

// Per-window tab query result (list_tabs return / source of the window:tabs-updated payload). ADR-0057.
#[derive(Debug, Clone, PartialEq)]
pub struct WindowTabsSnapshot {
    pub label: WindowLabel,
    pub window_id: String,
    pub number: u32,
    pub tabs: Vec<ViewMeta>,
    pub active: ViewId,
    pub version: u64,
}

// The layout authority state (tab ownership model — ADR-0057). Accessed concurrently from the invoke thread pool; LayoutState wraps it in a Mutex.
pub struct ViewManager {
    pub views: HashMap<ViewId, View>,
    pub view_owner: HashMap<ViewId, WindowLabel>,
    pub windows: HashMap<WindowLabel, WindowTabs>,
    // +1 on every change (for the get_view race — popup pull vs listen). Starts at 0.
    pub version: u64,
    // +1 whenever window attributes (`WindowTabs::attrs`) change; `version` is not bumped. The writer only compares both for equality, so ★never revert or reset★: a value equal to the last seen one would hide the changes in between. (TRD S21-storage §6-4)
    attrs_rev: u64,
    // Raw text of slot content this version does not know (view → slot → JSON object). That slot's memory content is `Empty` and it counts as occupied while the entry exists. Not sent to the frontend (snapshots carry only slot ids, `foreign_slots`). Keyed by view first so it follows a tab moving to another window and is reaped at once when the view is removed.
    unknown_content: HashMap<ViewId, HashMap<Uuid, Map<String, Value>>>,
    // Tombstones of closed window labels. Popout labels are never reused within one boot, so it only grows. Not saved.
    closed_labels: HashSet<WindowLabel>,
    // Closing popouts (label → that window), not saved. If that window's `Destroyed` never comes, the number stays taken until the run ends (accepted). (TRD S26-window-chrome §4-1 W7)
    closing: HashMap<WindowLabel, Closing>,
    // Agent id → display name: a derived cache of the names from the daemon's `AgentListUpdated`, not saved. Tab auto names read it (an agent without an entry is "Agent"). Production writes only through `update_agent_names` (daemon list — overwrites) and `seed_agent_names` (boot · copy accept — empty entries only); nothing removes entries. (TRD S26-window-chrome §4-3)
    agent_names: HashMap<String, String>,
    // The language automatic tab names (and so OS window titles) are made in. Not saved; it is the effective `ui.language` setting, set before the boot model is swapped in and changed only through `set_language`; `adopt_restored` keeps it.
    // ADR-0373
    lang: Lang,
}

impl Default for ViewManager {
    fn default() -> Self {
        Self::new()
    }
}

impl ViewManager {
    // ★The boot default is a single empty slot like any new view★: no tree is laid down at boot; the user places it via the empty slot menu (`set_slot_content`). // ADR-0222
    pub fn new() -> Self {
        let mut mgr = Self::blank();
        let v0 = mgr.make_view();
        mgr.view_owner.insert(v0, MAIN_WINDOW_LABEL.to_string());
        mgr.insert_window_entry(
            MAIN_WINDOW_LABEL.to_string(),
            WindowTabs::new_main(WindowAttrs::default(), vec![v0], v0),
        );
        mgr
    }

    /// Builds a new model from revived windows: `version` · `attrs_rev` are 0 and the side table is empty (the caller sets unknown content with `set_unknown_content`). Each View's focus is fixed to the tree's first slot (as `fixup_focus`).
    ///
    /// If main has no tabs, one empty tab is set up (invariant 4). Any other input that cannot satisfy invariants 1–4 is `Err` and creates nothing: a popout without tabs · an active tab outside the tabs or absent · duplicate View ids · a popout label that is `main` or duplicated. Popout entry = (label, persisted id, window).
    ///
    /// Numbers: main = 1, popouts = the saved numbers fixed in this order with [`numbering::resolve_saved`]; fixed windows are told apart by comparing `RestoredWindow::number` with the resulting number. (TRD S26-window-chrome §3-4)
    pub(crate) fn from_restored(
        main: RestoredWindow,
        popouts: Vec<(WindowLabel, String, RestoredWindow)>,
    ) -> Result<Self, String> {
        let mut mgr = Self::blank();
        let RestoredWindow {
            attrs,
            tabs,
            active,
            number: _,
        } = main;
        let main_window = if tabs.is_empty() {
            let v0 = mgr.make_view();
            mgr.view_owner.insert(v0, MAIN_WINDOW_LABEL.to_string());
            WindowTabs::new_main(attrs, vec![v0], v0)
        } else {
            let (tabs, active) = mgr.adopt_tabs(MAIN_WINDOW_LABEL, tabs, active)?;
            WindowTabs::new_main(attrs, tabs, active)
        };
        mgr.insert_window_entry(MAIN_WINDOW_LABEL.to_string(), main_window);

        let saved: Vec<Option<u32>> = popouts.iter().map(|(_, _, w)| w.number).collect();
        let numbers = numbering::resolve_saved(&saved, [numbering::MAIN_WINDOW_NUMBER]);
        for ((label, window_id, restored), number) in popouts.into_iter().zip(numbers) {
            if mgr.windows.contains_key(&label) {
                return Err(format!("popout label {label} is duplicated"));
            }
            let RestoredWindow {
                attrs,
                tabs,
                active,
                number: _,
            } = restored;
            if tabs.is_empty() {
                return Err(format!("popout {window_id} has no tabs"));
            }
            let (tabs, active) = mgr.adopt_tabs(&label, tabs, active)?;
            mgr.insert_window_entry(
                label,
                WindowTabs::new(window_id, number, attrs, tabs, active),
            );
        }
        Ok(mgr)
    }

    // A model with no windows at all — invariant 4 does not hold yet; the creating side sets up main.
    fn blank() -> Self {
        Self {
            views: HashMap::new(),
            view_owner: HashMap::new(),
            windows: HashMap::new(),
            version: 0,
            attrs_rev: 0,
            unknown_content: HashMap::new(),
            closed_labels: HashSet::new(),
            closing: HashMap::new(),
            agent_names: HashMap::new(),
            lang: Lang::default(),
        }
    }

    // ── Inserting and removing window entries ──────────── (TRD S26-window-chrome §2-4 ① · §4-1)

    fn held_numbers(&self) -> Vec<u32> {
        self.windows
            .values()
            .map(|w| w.number)
            .chain(self.closing.values().map(|c| c.number))
            .collect()
    }

    // The only place that inserts a window entry. `entry` already holds its number (new popout = [`Self::insert_new_popout`]).
    // ADR-0300 — window numbers
    fn insert_window_entry(&mut self, label: WindowLabel, entry: WindowTabs) {
        // Production popout labels are never reused (`PopupCounter`); inserting with the label of a closing window would give that label two numbers · persisted ids.
        debug_assert!(
            !self.closing.contains_key(&label),
            "inserting a window entry with the label of closing window {label}"
        );
        let replaced = self.windows.insert(label, entry);
        debug_assert!(replaced.is_none(), "overwrote a window entry");
    }

    // A new popout entry — number = the smallest held by neither live nor closing windows (D2 · D13).
    fn insert_new_popout(&mut self, label: &str, view: ViewId) {
        let number = numbering::smallest_free(self.held_numbers());
        self.insert_window_entry(label.to_string(), WindowTabs::new_popout(number, view));
    }

    // The only place that removes a window entry. With `Os::Alive` the number is held in `closing` until that OS window's `Destroyed` ([`Self::release_closing`]); with `Os::Gone` it is freed right away. View · `view_owner` · version are the caller's job. main is never removed (invariant 4).
    // ADR-0300 — window numbers
    fn retire_window_entry(&mut self, label: &str, os: Os) -> Option<WindowTabs> {
        debug_assert_ne!(label, MAIN_WINDOW_LABEL, "removing the main window entry");
        let entry = self.windows.remove(label)?;
        if os == Os::Alive {
            self.closing.insert(
                label.to_string(),
                Closing {
                    window_id: entry.window_id.clone(),
                    number: entry.number,
                },
            );
        }
        Some(entry)
    }

    /// Frees the number of closing window `label`, called by that OS window's `Destroyed`. A no-op `false` if it is not a closing window (a prewarm window · already freed · freed by a copy accept · in the model); it looks up by label only, so it never touches a live window's number. `version` stays.
    /// `version` stays (the window entries that carry numbers into saves · notices do not change).
    // ADR-0300 — window numbers
    pub fn release_closing(&mut self, label: &str) -> bool {
        self.closing.remove(label).is_some()
    }

    /// The label of the closing window whose persisted id is `window_id` — same spelling first, otherwise the same UUID value (as `super::window_arg::resolve`). `None` if none. Used by the 0-tab self-close. (TRD S26-window-chrome §5-5)
    pub fn closing_label_for(&self, window_id: &str) -> Option<WindowLabel> {
        let by_spelling = self
            .closing
            .iter()
            .find(|(_, closing)| closing.window_id == window_id);
        if let Some((label, _)) = by_spelling {
            return Some(label.clone());
        }
        let id = Uuid::parse_str(window_id).ok()?;
        self.closing
            .iter()
            .find(|(_, closing)| Uuid::parse_str(&closing.window_id).ok() == Some(id))
            .map(|(label, _)| label.clone())
    }

    // Takes in the tabs of window `label`: the ownership pair (invariants 1·2) and the active check (invariant 3). Returns (tab order, active tab).
    fn adopt_tabs(
        &mut self,
        label: &str,
        tabs: Vec<View>,
        active: Option<ViewId>,
    ) -> Result<(Vec<ViewId>, ViewId), String> {
        let ids: Vec<ViewId> = tabs.iter().map(|v| v.id).collect();
        let active = match active {
            Some(active) if ids.contains(&active) => active,
            _ => {
                return Err(format!(
                    "active tab {active:?} of window {label} is outside the tab list"
                ))
            }
        };
        for mut view in tabs {
            if self.views.contains_key(&view.id) {
                return Err(format!("View {} is duplicated", view.id));
            }
            Self::fixup_focus(&mut view);
            self.view_owner.insert(view.id, label.to_string());
            self.views.insert(view.id, view);
        }
        Ok((ids, active))
    }

    // ── Queries ───────────────────────────────────────────────────────────────

    pub fn list_tabs(&self, label: &str) -> Result<WindowTabsSnapshot, LayoutError> {
        let wt = self
            .windows
            .get(label)
            .ok_or_else(|| LayoutError::WindowNotFound(label.to_string()))?;
        // Unique ownership, so tabs is that window's tab list — no filter needed.
        let tabs: Vec<ViewMeta> = wt
            .tabs
            .iter()
            .filter_map(|vid| {
                self.views.get(vid).map(|v| ViewMeta {
                    id: v.id,
                    name: effective_tab_name(self, v),
                    named: v.name.is_named(),
                })
            })
            .collect();
        Ok(WindowTabsSnapshot {
            label: label.to_string(),
            window_id: wt.window_id.clone(),
            number: wt.number,
            tabs,
            active: wt.active,
            version: self.version,
        })
    }

    pub fn list_windows(&self) -> Vec<WindowLabel> {
        self.windows.keys().cloned().collect()
    }

    pub fn snapshot(&self, view_id: Uuid) -> Result<ViewSnapshot, LayoutError> {
        let v = self
            .views
            .get(&view_id)
            .ok_or(LayoutError::ViewNotFound(view_id))?;
        let geo = geometry::compute(&v.layout);
        Ok(ViewSnapshot {
            view_id: v.id,
            layout: v.layout.clone(),
            focused_slot_id: v.focused_slot_id,
            foreign_slots: self.foreign_slots(v),
            slot_spatial: super::spatial::compute_spatial(&v.layout),
            slot_rects: geo.slots,
            split_rects: geo.splits,
            ratio_min: tree::RATIO_MIN,
            ratio_max: tree::RATIO_MAX,
            version: self.version,
        })
    }

    pub fn slot_agent(&self, view_id: Uuid, slot_id: Uuid) -> Result<Option<String>, LayoutError> {
        let v = self
            .views
            .get(&view_id)
            .ok_or(LayoutError::ViewNotFound(view_id))?;
        // ADR-0060: tells a missing slot (None) from an empty slot (Some(Empty)→None) — only missing is SlotNotFound.
        tree::find_slot(&v.layout, slot_id)
            .map(|content| content.agent_id().map(str::to_string))
            .ok_or(LayoutError::SlotNotFound(slot_id))
    }

    /// Whether automatic placement may put content into this slot: empty (`SlotContent::Empty`) and no unknown content. A missing view·slot is `Err`. The empty/occupied decision of ADR-0059 comes only from this; looking only at `SlotContent::is_empty` would overwrite a slot holding unknown content. (TRD S21-storage §6-2)
    pub fn slot_is_free(&self, view_id: ViewId, slot_id: Uuid) -> Result<bool, LayoutError> {
        let v = self
            .views
            .get(&view_id)
            .ok_or(LayoutError::ViewNotFound(view_id))?;
        self.free_in(v, slot_id)
            .ok_or(LayoutError::SlotNotFound(slot_id))
    }

    fn free_in(&self, view: &View, slot_id: Uuid) -> Option<bool> {
        let content = tree::find_slot(&view.layout, slot_id)?;
        Some(content.is_empty() && self.unknown_content(view.id, slot_id).is_none())
    }

    /// The raw text of the unknown content held by slot `slot_id` — `None` if none (or a missing view·slot). Saving writes the slot with this raw text instead of the memory content (`Empty`).
    pub(crate) fn unknown_content(
        &self,
        view_id: ViewId,
        slot_id: Uuid,
    ) -> Option<&Map<String, Value>> {
        self.unknown_content.get(&view_id)?.get(&slot_id)
    }

    /// The display name of agent `agent_id` — `None` if not in the name ledger.
    pub(super) fn agent_name(&self, agent_id: &str) -> Option<&str> {
        self.agent_names.get(agent_id).map(String::as_str)
    }

    /// Overwrites `names` (agent id, display name) into the name ledger without removing: slots of agents dropped from the list (dead) keep the last name. Returns the labels of windows where an auto-named tab's visible name actually changed (sorted, no duplicates); if any, `version` goes up, otherwise it stays — the web drops tab notices with the same version, and the writer rewrites tab names in the file only when the version changes. (TRD S26-window-chrome §4-3)
    pub(crate) fn update_agent_names(&mut self, names: &[(String, String)]) -> Vec<WindowLabel> {
        self.rename_auto_tabs(|mgr| {
            for (agent_id, name) in names {
                mgr.agent_names.insert(agent_id.clone(), name.clone());
            }
        })
    }

    /// The language automatic tab names are made in.
    pub fn language(&self) -> Lang {
        self.lang
    }

    /// Makes automatic tab names in `lang` from now on. Returns the labels of windows where an automatic tab's visible name actually changed (sorted · unique) and bumps `version` if any — the same contract as [`Self::update_agent_names`] (ADR-0301 invariant 2: without the bump the web and the OS title drop the notification as not newer). Only named tabs, or the same language, change nothing.
    // ADR-0373
    pub(crate) fn set_language(&mut self, lang: Lang) -> Vec<WindowLabel> {
        self.rename_auto_tabs(|mgr| mgr.lang = lang)
    }

    // Runs `change` and reports which windows' automatic tab names it changed — see `update_agent_names` for the contract.
    fn rename_auto_tabs(&mut self, change: impl FnOnce(&mut Self)) -> Vec<WindowLabel> {
        let before: Vec<(WindowLabel, ViewId, String)> = self
            .windows
            .iter()
            .flat_map(|(label, w)| w.tabs.iter().map(move |view| (label, view)))
            .filter_map(|(label, view)| {
                let v = self.views.get(view)?;
                (!v.name.is_named()).then(|| (label.clone(), v.id, effective_tab_name(self, v)))
            })
            .collect();
        change(self);
        let mut changed: Vec<WindowLabel> = before
            .into_iter()
            .filter(|(_, view, old)| {
                self.views
                    .get(view)
                    .is_some_and(|v| effective_tab_name(self, v) != *old)
            })
            .map(|(label, _, _)| label)
            .collect();
        changed.sort();
        changed.dedup();
        if !changed.is_empty() {
            self.bump_version();
        }
        changed
    }

    /// Fills only the empty entries of the name ledger with `names`; existing entries stay (daemon-list values are newer than the saved file's). `version` is not touched. The seed job of boot · copy accept; which names to seed is the caller's choice (`state::convert`). (TRD S26-window-chrome §4-3)
    pub(crate) fn seed_agent_names(&mut self, names: impl IntoIterator<Item = (String, String)>) {
        for (agent_id, name) in names {
            self.agent_names.entry(agent_id).or_insert(name);
        }
    }

    #[cfg(test)]
    pub(crate) fn put_agent_name(&mut self, agent_id: &str, name: &str) {
        self.agent_names
            .insert(agent_id.to_owned(), name.to_owned());
    }

    // Tree preorder — filtered following the tree so the table iteration order (HashMap) does not leak into the snapshot.
    fn foreign_slots(&self, view: &View) -> Vec<Uuid> {
        match self.unknown_content.get(&view.id) {
            None => Vec::new(),
            Some(slots) => tree::slot_ids(&view.layout)
                .into_iter()
                .filter(|s| slots.contains_key(s))
                .collect(),
        }
    }

    /// Slot resolution of `spawn_into` (D-7): spawn · lock · emit are the command layer's; this only makes the policy decision (assignment is `assign_agent`).
    /// - `slot=None`: traverses the tree in preorder (left first) and picks **the first empty slot** (`slot_is_free`); if none, `NoEmptySlot` — no automatic split · overwrite.
    /// - `slot=Some(s)`: `s` if empty · `SlotOccupied` if occupied (including unknown content) · `SlotNotFound` if not in the tree.
    // ADR-0059 (TRD §6 G9)
    pub fn resolve_spawn_slot(
        &self,
        view_id: ViewId,
        slot: Option<Uuid>,
    ) -> Result<Uuid, SpawnSlotError> {
        let v = self
            .views
            .get(&view_id)
            .ok_or(SpawnSlotError::ViewNotFound(view_id))?;
        match slot {
            Some(target) => match self.free_in(v, target) {
                Some(true) => Ok(target),
                Some(false) => Err(SpawnSlotError::SlotOccupied(target)),
                None => Err(SpawnSlotError::SlotNotFound(target)),
            },
            None => tree::slot_ids(&v.layout)
                .into_iter()
                .find(|&s| self.free_in(v, s) == Some(true))
                .ok_or(SpawnSlotError::NoEmptySlot),
        }
    }

    // None if absent (an orphan View — does not occur on normal paths).
    pub fn owner_of(&self, view_id: ViewId) -> Option<&WindowLabel> {
        self.view_owner.get(&view_id)
    }

    fn view_mut(&mut self, view_id: Uuid) -> Result<&mut View, LayoutError> {
        self.views
            .get_mut(&view_id)
            .ok_or(LayoutError::ViewNotFound(view_id))
    }

    // focus fallback — if the slot focused_slot_id pointed at vanishes, go to the tree's first slot (always ≥1 slot).
    fn fixup_focus(view: &mut View) {
        let valid = view
            .focused_slot_id
            .map(|fid| tree::contains_slot(&view.layout, fid))
            .unwrap_or(false);
        if !valid {
            view.focused_slot_id = Some(tree::first_slot_id(&view.layout));
        }
    }

    fn bump_version(&mut self) {
        self.version += 1;
    }

    // ── Internal helpers ───────────────────────────────────────────────────────────

    // The only door for writing slot content — removes the unknown content entry (whatever is written, even `Empty`). The caller bumps version.
    fn write_slot_content(
        &mut self,
        view_id: ViewId,
        slot_id: Uuid,
        content: SlotContent,
    ) -> Result<(), LayoutError> {
        let v = self.view_mut(view_id)?;
        if !tree::set_in_tree(&mut v.layout, slot_id, content) {
            return Err(LayoutError::SlotNotFound(slot_id));
        }
        self.forget_unknown(view_id, slot_id);
        Ok(())
    }

    fn remember_unknown(&mut self, view_id: ViewId, slot_id: Uuid, raw: Map<String, Value>) {
        self.unknown_content
            .entry(view_id)
            .or_default()
            .insert(slot_id, raw);
    }

    fn forget_unknown(&mut self, view_id: ViewId, slot_id: Uuid) {
        if let Some(slots) = self.unknown_content.get_mut(&view_id) {
            slots.remove(&slot_id);
            if slots.is_empty() {
                self.unknown_content.remove(&view_id);
            }
        }
    }

    // The only place that removes a view. The window tab lists (`windows[*].tabs`) are aligned by the caller (invariant 1).
    fn remove_view(&mut self, view: ViewId) {
        self.views.remove(&view);
        self.view_owner.remove(&view);
        self.unknown_content.remove(&view);
    }

    // Ownership/window assignment is the caller's. The name is auto — no default name is built in. (TRD S26-window-chrome §3-1)
    fn make_view(&mut self) -> ViewId {
        let id = Uuid::new_v4();
        let first_slot = LayoutNode::new_empty_slot();
        let focus = tree::first_slot_id(&first_slot);
        self.views.insert(
            id,
            View {
                id,
                name: TabName::Auto,
                layout: first_slot,
                focused_slot_id: Some(focus),
            },
        );
        id
    }

    // ── mutation ────────────────────────────────────────────────────────────

    /// If `name` is `Some`, a tab with that set name; if `None`, an auto-named tab. Empty or whitespace-only names are `EmptyName` and create nothing.
    pub fn create_tab(&mut self, label: &str, name: Option<String>) -> Result<ViewId, LayoutError> {
        if !self.windows.contains_key(label) {
            return Err(LayoutError::WindowNotFound(label.to_string()));
        }
        let name = match name {
            Some(name) => TabName::named(name).ok_or(LayoutError::EmptyName)?,
            None => TabName::Auto,
        };
        let id = self.make_view();
        self.view_mut(id).expect("the View just created").name = name;
        // Pair update (invariants 1·2). // ADR-0057
        self.view_owner.insert(id, label.to_string());
        let wt = self.windows.get_mut(label).expect("existence checked");
        wt.tabs.push(id);
        wt.active = id;
        self.bump_version();
        Ok(id)
    }

    // The label is issued by the caller (D-6). If the label has a tombstone (that window closed after the OS window opened and before inserting), `WindowClosed`: no model entry is created for a closed window, or a popout without an OS window would stay in `state.json` and restore as an empty popup at the next boot.
    // ADR-0299 (TRD S21-slot-docking §3-9)
    pub fn create_window(&mut self, label: &str) -> Result<ViewId, LayoutError> {
        if self.closed_labels.contains(label) {
            return Err(LayoutError::WindowClosed(label.to_string()));
        }
        if self.windows.contains_key(label) {
            return Err(LayoutError::WindowNotFound(label.to_string()));
        }
        let id = self.make_view();
        self.view_owner.insert(id, label.to_string());
        self.insert_new_popout(label, id);
        self.bump_version();
        Ok(id)
    }

    // keep-alive (ADR-0056), so the exposed set is unchanged — only the active mark changes.
    pub fn switch_tab(&mut self, label: &str, view: ViewId) -> Result<(), LayoutError> {
        let wt = self
            .windows
            .get_mut(label)
            .ok_or_else(|| LayoutError::WindowNotFound(label.to_string()))?;
        if !wt.tabs.contains(&view) {
            return Err(LayoutError::ViewNotFound(view));
        }
        wt.active = view;
        self.bump_version();
        Ok(())
    }

    /// [`Self::switch_tab`] only when that window's active tab is `expect`: `true` if switched, `false` if the active tab differs (touches nothing, version unchanged). A missing window or a `view` that is not that window's tab are the same errors as `switch_tab` regardless of the condition.
    pub fn switch_tab_if(
        &mut self,
        label: &str,
        view: ViewId,
        expect: ViewId,
    ) -> Result<bool, LayoutError> {
        let wt = self
            .windows
            .get(label)
            .ok_or_else(|| LayoutError::WindowNotFound(label.to_string()))?;
        if !wt.tabs.contains(&view) {
            return Err(LayoutError::ViewNotFound(view));
        }
        if wt.active != expect {
            return Ok(false);
        }
        self.switch_tab(label, view)?;
        Ok(true)
    }

    // Closes tab `view` of window `label` (§5-2 state machine, ADR-0057). Returns whether the window **must close** because of this close.
    pub fn close_tab(&mut self, label: &str, view: ViewId) -> Result<CloseTabOutcome, LayoutError> {
        let wt = self
            .windows
            .get(label)
            .ok_or_else(|| LayoutError::WindowNotFound(label.to_string()))?;
        if !wt.tabs.contains(&view) {
            return Err(LayoutError::ViewNotFound(view));
        }

        // Drops one View (invariant 1 pair update). // ADR-0057
        self.remove_view(view);
        let outcome = match self.pull_tab(label, view, None) {
            Pulled::Stayed => CloseTabOutcome::Stayed,
            Pulled::PopoutRemoved => CloseTabOutcome::WindowClosed,
            Pulled::MainEmptied => {
                self.refill_main();
                CloseTabOutcome::Stayed
            }
        };
        self.bump_version();
        Ok(outcome)
    }

    // Removes `view` from the tab list of window `label`: if it was active, the successor is `successor` if that is a remaining tab, otherwise by `succession`; if a popout empties, the window entry is removed (ADR-0057 D-3). View · `view_owner` · version are the caller's job. With `MainEmptied` main remains with an empty list and the caller fills it with `refill_main` in the same operation (invariant 4). Not in the list: no-op `Stayed`.
    // ADR-0057
    fn pull_tab(&mut self, label: &str, view: ViewId, successor: Option<ViewId>) -> Pulled {
        let Some(wt) = self.windows.get_mut(label) else {
            return Pulled::Stayed;
        };
        let Some(pos) = wt.tabs.iter().position(|v| *v == view) else {
            return Pulled::Stayed;
        };
        let heir = succession(&wt.tabs, view);
        wt.tabs.remove(pos);
        if wt.active == view {
            // ADR-0295
            if let Some(heir) = successor.filter(|s| wt.tabs.contains(s)).or(heir) {
                wt.active = heir;
            }
        }
        if !wt.tabs.is_empty() {
            Pulled::Stayed
        } else if label == MAIN_WINDOW_LABEL {
            Pulled::MainEmptied
        } else {
            // The caller closes the OS window — the number is held until its `Destroyed`.
            self.retire_window_entry(label, Os::Alive);
            Pulled::PopoutRemoved
        }
    }

    // Sets up one empty tab in an empty main, makes it active and returns its id. Must be called in the same lock operation where `pull_tab` emitted `MainEmptied`, so an empty main is never visible outside (invariant 4). version is the caller's job.
    // ADR-0057
    fn refill_main(&mut self) -> ViewId {
        let id = self.make_view();
        // Pair update (invariants 1·2). // ADR-0057
        self.view_owner.insert(id, MAIN_WINDOW_LABEL.to_string());
        let wt = self
            .windows
            .get_mut(MAIN_WINDOW_LABEL)
            .expect("main is never removed");
        wt.tabs.push(id);
        wt.active = id;
        id
    }

    // Closes window `label` entirely (§5-2; drops all tab Views + removes the windows entry); returns the dropped View ids (the command layer puts them in the Unsubscribe delta after the rebuild). Assumes the OS window still exists and holds the number until its `Destroyed`; if it is already gone use [`Self::drop_window_gone`].
    // ADR-0057
    pub fn close_window(&mut self, label: &str) -> Result<Vec<ViewId>, LayoutError> {
        self.remove_window(label, Os::Alive)
    }

    /// Like [`Self::close_window`] but that OS window is already gone: frees the number right away. For callers where that window's `Destroyed` already passed or never comes (a boot restore popout that could not be opened · a copy window that vanished before the swap · a window still in the model at `Destroyed`). Calling it while the OS window remains gives a new window the same number before that window closes.
    // ADR-0300 — window numbers (TRD S26-window-chrome §4-1)
    pub fn drop_window_gone(&mut self, label: &str) -> Result<Vec<ViewId>, LayoutError> {
        self.remove_window(label, Os::Gone)
    }

    fn remove_window(&mut self, label: &str, os: Os) -> Result<Vec<ViewId>, LayoutError> {
        if label == MAIN_WINDOW_LABEL {
            return Err(LayoutError::MainNotClosable);
        }
        let wt = self
            .retire_window_entry(label, os)
            .ok_or_else(|| LayoutError::WindowNotFound(label.to_string()))?;
        // Drops all tab Views of this window (invariant 1 pair update — iterates all tabs). // ADR-0057
        let dropped = wt.tabs.clone();
        for vid in &wt.tabs {
            self.remove_view(*vid);
        }
        self.bump_version();
        Ok(dropped)
    }

    // view-id is globally unique, so the owning window derives from view_owner.
    pub fn split_slot(
        &mut self,
        view_id: Uuid,
        slot_id: Uuid,
        dir: SplitDir,
    ) -> Result<Uuid, LayoutError> {
        let v = self.view_mut(view_id)?;
        // Representability guard: always splitting the child on the same side makes the new boundary bit-identical to the parent's end and one area's width 0. Measure the new boundary in advance with the same formula and ratio as the real geometry and split only when it lies strictly inside the area. An area that already has area 0 (only in planted trees) is refused regardless of axis (the new areas would both be 0). A split creating an area smaller than `min_pane_px` is not blocked (R11 — user decision).
        // ADR-0227
        let geo = geometry::compute(&v.layout);
        if let Some(r) = geo.slots.iter().find(|s| s.slot_id == slot_id) {
            if !has_area(r) {
                return Err(LayoutError::SplitTooDeep);
            }
            let (lo, hi) = match dir {
                SplitDir::LeftRight => (r.x0, r.x1),
                SplitDir::TopBottom => (r.y0, r.y1),
            };
            let at = geometry::boundary(lo, hi, tree::SPLIT_RATIO);
            if !(lo < at && at < hi) {
                return Err(LayoutError::SplitTooDeep);
            }
        }
        match tree::split_in_tree(&mut v.layout, slot_id, dir) {
            Some(new_id) => {
                v.focused_slot_id = Some(new_id);
                self.bump_version();
                Ok(new_id)
            }
            None => Err(LayoutError::SlotNotFound(slot_id)),
        }
    }

    // Writes the ratio of split `split_id` (side a = the share of the left/top area — ADR-0140). The decision order is the contract:
    // ① non-finite → `InvalidRatio` — this manager is the single gate for every caller (direct Rust or non-JSON transports skip bus · IPC deserialization filtering).
    // ② missing view → `ViewNotFound`, a split not in that view → `SplitNotFound` (no change).
    // ③ clamp to `[RATIO_MIN, RATIO_MAX]`.
    // ④ If both the canvas · metrics of the owning window are known, clamp again so each side of the split (whole subtree) is at least `min_pane_px` (not refused); an empty range → `TooSmall` with the current value. The screen drag's twin `ratioRange` in `src/components/layout/splitPreview.ts` computes the range with the same formula and input — changing only one side makes the confirmed value get clipped again here and jump on release. If either is unknown (before the report · right after a popout) only ③ applies and an area thinner than `min_pane_px` can result — accepted to keep ③'s limit small (ADR-0260).
    // ⑤ If the write would **newly** make an area of area 0 → `TooSmall` (blocks an ancestor ratio collapsing a deep descendant). Areas already 0 before the write are excluded from the comparison, or one such area would lock all ratio writes of this view.
    // ⑥ Equal to the current value → `Unchanged` (version unchanged). ⑦ Otherwise write and version +1 → `Applied`.
    // ADR-0227 (TRD §2c/§2f)
    pub fn set_split_ratio(
        &mut self,
        view_id: ViewId,
        split_id: Uuid,
        ratio: f64,
    ) -> Result<SplitRatioResult, LayoutError> {
        if !ratio.is_finite() {
            return Err(LayoutError::InvalidRatio(ratio.to_string()));
        }
        let px = self.px_context(view_id)?;
        let v = self
            .views
            .get(&view_id)
            .ok_or(LayoutError::ViewNotFound(view_id))?;
        let current =
            tree::split_ratio(&v.layout, split_id).ok_or(LayoutError::SplitNotFound(split_id))?;
        let before = geometry::compute(&v.layout);
        let rect = before
            .splits
            .iter()
            .find(|s| s.split_id == split_id)
            .ok_or(LayoutError::SplitNotFound(split_id))?;
        let untouched = SplitRatioResult {
            ratio: current,
            outcome: SplitRatioOutcome::TooSmall,
        };

        let mut wanted = tree::clamp_ratio(ratio);
        if let Some((canvas, metrics)) = px {
            let (canvas_len, extent) = match rect.dir {
                SplitDir::LeftRight => (canvas.w, rect.x1 - rect.x0),
                SplitDir::TopBottom => (canvas.h, rect.y1 - rect.y0),
            };
            let len = f64::from(canvas_len) * extent;
            let edge = f64::from(metrics.min_pane_px) / len;
            let lo = tree::RATIO_MIN.max(edge);
            let hi = tree::RATIO_MAX.min(1.0 - edge);
            // `f64::clamp` panics on an inverted range; a zero-width box (`len == 0`) has `edge = ∞` and falls out here as an empty range.
            if lo > hi {
                return Ok(untouched);
            }
            wanted = wanted.clamp(lo, hi);
        }

        let mut candidate = v.layout.clone();
        tree::set_ratio_in_tree(&mut candidate, split_id, wanted);
        let after = geometry::compute(&candidate);
        // Only the ratio changed, so the leaf set · preorder are the same; compare positions pairwise.
        let collapses = before
            .slots
            .iter()
            .zip(&after.slots)
            .any(|(b, a)| has_area(b) && !has_area(a));
        if collapses {
            return Ok(untouched);
        }
        if wanted == current {
            return Ok(SplitRatioResult {
                ratio: wanted,
                outcome: SplitRatioOutcome::Unchanged,
            });
        }
        self.view_mut(view_id)?.layout = candidate;
        self.bump_version();
        Ok(SplitRatioResult {
            ratio: wanted,
            outcome: SplitRatioOutcome::Applied,
        })
    }

    pub fn list_splits(&self, view_id: ViewId) -> Result<Vec<tree::SplitInfo>, LayoutError> {
        let v = self
            .views
            .get(&view_id)
            .ok_or(LayoutError::ViewNotFound(view_id))?;
        Ok(tree::list_splits(&v.layout))
    }

    // Sets `slot_id` in `view` as the focus (click-to-focus — ADR-0066 decision 1). ★Only when that slot exists in this View tree★ is `focused_slot_id` updated and version bumped; otherwise a no-op + SlotNotFound.
    // ★Backend authority (ADR-0035/0066)★: the backend owns focused_slot_id and the frontend reflects it only through emit (layout:updated); no optimistic update.
    // ADR-0066
    pub fn set_focused_slot(&mut self, view_id: Uuid, slot_id: Uuid) -> Result<(), LayoutError> {
        let v = self.view_mut(view_id)?;
        if !tree::contains_slot(&v.layout, slot_id) {
            return Err(LayoutError::SlotNotFound(slot_id));
        }
        v.focused_slot_id = Some(slot_id);
        self.bump_version();
        Ok(())
    }

    /// Sets the tab name — text identical to the currently visible auto name still becomes a set name (it no longer follows the slot · agent name). Empty or whitespace-only names are `EmptyName` with no change. Other text is held as is without trimming (trimming is the boundary's job: human UI · bus adapter). (TRD S26-window-chrome §3-1)
    pub fn rename_tab(&mut self, view_id: Uuid, name: String) -> Result<(), LayoutError> {
        let v = self.view_mut(view_id)?;
        v.name = TabName::named(name).ok_or(LayoutError::EmptyName)?;
        self.bump_version();
        Ok(())
    }

    pub fn close_slot(&mut self, view_id: Uuid, slot_id: Uuid) -> Result<(), LayoutError> {
        let v = self.view_mut(view_id)?;
        if !tree::close_in_tree(&mut v.layout, slot_id) {
            return Err(LayoutError::SlotNotFound(slot_id));
        }
        Self::fixup_focus(v);
        self.forget_unknown(view_id, slot_id);
        self.bump_version();
        Ok(())
    }

    // Assigns agent_id (a reference string) to slot slot_id in view; ★not verified to exist in the daemon (ADR-0035/0006)★.
    // ★Overwrite semantics★: an occupied slot is replaced unconditionally; the occupancy defense is in the `resolve_spawn_slot` layer (ADR-0059).
    pub fn assign_agent(
        &mut self,
        view_id: Uuid,
        slot_id: Uuid,
        agent_id: String,
    ) -> Result<(), LayoutError> {
        self.write_slot_content(view_id, slot_id, SlotContent::Agent { agent_id })?;
        self.bump_version();
        Ok(())
    }

    // Replaces the content of slot slot_id in view with `content` (§5 path, ADR-0063 placement control surface). Like assign_agent but takes the whole union (Empty/Agent/AgentList/PresetPalette/Usage). ★Overwrite semantics★: an occupied slot is replaced unconditionally (a placement command intends an explicit replacement; no occupancy defense).
    pub fn set_slot_content(
        &mut self,
        view_id: Uuid,
        slot_id: Uuid,
        content: SlotContent,
    ) -> Result<(), LayoutError> {
        self.write_slot_content(view_id, slot_id, content)?;
        self.bump_version();
        Ok(())
    }

    /// Sets slot `slot_id` up as a slot holding content `raw` this version does not know: the memory content is `Empty` and the raw text is kept in the side table. The door restore (`PersistedContent::Unknown`) calls; IPC · bus commands never reach it (unknown kinds are refused in deserialization). A missing view·slot is `Err` with no change. (TRD S21-storage §6-2)
    pub(crate) fn set_unknown_content(
        &mut self,
        view_id: ViewId,
        slot_id: Uuid,
        raw: Map<String, Value>,
    ) -> Result<(), LayoutError> {
        self.write_slot_content(view_id, slot_id, SlotContent::Empty)?;
        self.remember_unknown(view_id, slot_id, raw);
        self.bump_version();
        Ok(())
    }

    // ── move_slot_to_window 2-phase support (§5-3) ───────────────────────────

    pub fn slot_content(&self, view_id: Uuid, slot_id: Uuid) -> Result<SlotContent, LayoutError> {
        let v = self
            .views
            .get(&view_id)
            .ok_or(LayoutError::ViewNotFound(view_id))?;
        tree::find_slot(&v.layout, slot_id)
            .cloned()
            .ok_or(LayoutError::SlotNotFound(slot_id))
    }

    // ★phase A★: creates a temporary View holding the source slot content, **not yet in any window's tabs** (phase C inserts it; this prevents an orphan). The source slot is untouched (closed in phase C). The only refusals are a missing view/slot.
    // ★ADR-0064 content generalization★: every slot kind can pop out (invariant 5); from the returned SlotContent the caller decides whether an agent subscription migration (still-ours close guard) is needed (= whether it is Agent).
    // ★Empty slots are moved too — do not revive the refusal (user decision)★: the result is a new tab with one empty area. Unknown content's raw text follows into the new slot (moving is not replacing); the source-side entry is reaped by phase C's `close_slot`.
    // ADR-0064
    // ADR-0228
    pub fn prepare_detached_view(
        &mut self,
        src_view: ViewId,
        src_slot: Uuid,
    ) -> Result<(ViewId, SlotContent), LayoutError> {
        let content = self.slot_content(src_view, src_slot)?;
        let carried = self.unknown_content(src_view, src_slot).cloned();
        let id = self.make_view();
        let slot = {
            let v = self.views.get(&id).expect("the View just created");
            tree::first_slot_id(&v.layout)
        };
        // ADR-0064
        let _ = self.write_slot_content(id, slot, content.clone());
        if let Some(raw) = carried {
            self.remember_unknown(id, slot, raw);
        }
        self.bump_version();
        Ok((id, content))
    }

    // ★phase A rollback★: removes the temporary View (before window insertion, so no tabs update needed).
    pub fn drop_detached_view(&mut self, view: ViewId) {
        self.remove_view(view);
        self.bump_version();
    }

    // ★phase C — insert into an existing window★: inserts and activates the temporary View as a new tab of `to_window`. ★Re-validation (G4)★: only if `to_window` still exists, else Err (the caller rolls back).
    pub fn insert_tab_into(&mut self, to_window: &str, view: ViewId) -> Result<(), LayoutError> {
        if !self.windows.contains_key(to_window) {
            return Err(LayoutError::WindowNotFound(to_window.to_string()));
        }
        if !self.views.contains_key(&view) {
            return Err(LayoutError::ViewNotFound(view));
        }
        // Pair update (invariants 1·2). // ADR-0057
        self.view_owner.insert(view, to_window.to_string());
        let wt = self.windows.get_mut(to_window).expect("existence checked");
        wt.tabs.push(view);
        wt.active = view;
        self.bump_version();
        Ok(())
    }

    // ★phase C — create a new window with the temporary View as its first tab★. The label is issued by the caller (monotonic counter). With a tombstone on the label (that new window closed during phase B) → `WindowClosed`; no model entry is created for a closed window. (TRD S21-slot-docking 5.8th edition §1-2)
    pub fn attach_view_as_new_window(
        &mut self,
        label: &str,
        view: ViewId,
    ) -> Result<(), LayoutError> {
        if self.closed_labels.contains(label) {
            return Err(LayoutError::WindowClosed(label.to_string()));
        }
        if self.windows.contains_key(label) {
            return Err(LayoutError::WindowNotFound(label.to_string()));
        }
        if !self.views.contains_key(&view) {
            return Err(LayoutError::ViewNotFound(view));
        }
        self.view_owner.insert(view, label.to_string());
        self.insert_new_popout(label, view);
        self.bump_version();
        Ok(())
    }

    // ── Docking ──────────────────────────────

    /// Moves tab `view` to `to`. On success, removing from the source window · inserting into the target · removing an empty popout entry · moving the unknown-content side table happen in one go and `version` goes up once, except that a `TabBar` at the same position in the same window is an unchanged `Ok` (`version` unchanged; with `Reveal` and a tab that was not visible it only makes it visible, `version` +1).
    /// A popout removed because it emptied is returned in `Docked::emptied` (its OS window is the caller's job). When main's last tab leaves to another window · a new window, an empty tab appears in main in the same go, becomes main's active tab, and its id is `Docked::main_refilled` (ADR-0057 invariant 4, as when closing a tab). On `Err` nothing changes (`version` included).
    ///
    /// The decision order is the contract — once caught earlier, later ones are not looked at:
    /// 1. If the source is not a tab of any window, `TabNotFound`.
    /// 2. Destination — `Seam` target tab missing or a tombstone ([`Self::mark_closed`]) on its window: `TabNotFound` · `TabBar` window missing or
    ///    tombstoned: `WindowNotFound` · tombstone on the `NewWindow` label: `WindowClosed` · a window with that label already exists:
    ///    `WindowExists`.
    /// 3. `Seam` target = source → `SelfTarget`. A `TabBar` target window = source window is allowed as a reposition. `TabBar`'s `after` =
    ///    source → `SelfTarget` · `after` not a tab of that window → `TabNotFound(after)`.
    /// 4. If the `Seam` reference slot is not in the target tab — `SlotNotInTab` if it is in another tab, `SlotNotFound` if nowhere.
    /// 5. `Seam` — `share` not finite or outside `tree::RATIO_MIN..=dock::SHARE_MAX`: `InvalidShare` · the two trees share slot ids:
    ///    `SlotIdOverlap` · a new area-0 area in the result: `ZeroArea`.
    ///
    /// Visible tab · focus: with `Keep`, the target window's active tab and the target tab's focus do not change, except that if the source was the target window's active tab (docking within the same window) the target (`Seam` = target tab · `TabBar` = moved tab) becomes active. With `Reveal` the target (`Seam` = target tab · otherwise the moved tab) is active, and `Seam` puts the target tab's focus on the first inserted slot. If the moved tab was the source window's active one and that window remains, `mv.successor` becomes its active tab when it is a remaining tab (and, when `mv.successor_window` is set, only if the source window is that window), otherwise [`succession`] does. (TRD S21-slot-docking §2-1 · §3-3)
    pub fn dock(
        &mut self,
        view: ViewId,
        to: DockTo,
        focus: FocusPolicy,
        mv: DockMove,
    ) -> Result<Docked, DockError> {
        let (source_window, plan) = self.check_dock(view, &to)?;
        let slot_ids = tree::slot_ids(&self.views[&view].layout);
        let reveal = focus == FocusPolicy::Reveal;
        let (window, pulled) = match plan {
            DockPlan::Seam {
                target,
                target_window,
                layout,
            } => {
                let first = tree::first_slot_id(&self.views[&view].layout);
                let source_was_visible =
                    target_window == source_window && self.windows[&target_window].active == view;
                if let Some(slots) = self.unknown_content.remove(&view) {
                    self.unknown_content
                        .entry(target)
                        .or_default()
                        .extend(slots);
                }
                let pulled =
                    self.pull_docked_tab(&source_window, view, mv.successor_in(&source_window));
                self.remove_view(view);
                match self.view_mut(target) {
                    Ok(t) => {
                        t.layout = layout;
                        if reveal {
                            t.focused_slot_id = Some(first);
                        }
                    }
                    Err(e) => {
                        tracing::warn!(view = %target, "seam docking — the target tab the decision saw is gone at apply time: {e}")
                    }
                }
                if reveal || source_was_visible {
                    if let Some(wt) = self.windows.get_mut(&target_window) {
                        wt.active = target;
                    }
                }
                (target_window, pulled)
            }
            DockPlan::TabBar { window, tabs } => {
                let current = &self.windows[&window];
                if tabs == current.tabs && (!reveal || current.active == view) {
                    return Ok(Docked {
                        window,
                        slot_ids,
                        source_window,
                        emptied: None,
                        main_refilled: None,
                    });
                }
                // Within the same window the source is not removed — only the list changes, so the active tab (the source itself if it was the source) stays.
                let pulled = if window == source_window {
                    PulledDock::default()
                } else {
                    self.pull_docked_tab(&source_window, view, mv.successor_in(&source_window))
                };
                // Pair update (invariants 1·2). // ADR-0057
                self.view_owner.insert(view, window.clone());
                if let Some(wt) = self.windows.get_mut(&window) {
                    wt.tabs = tabs;
                    if reveal {
                        wt.active = view;
                    }
                }
                (window, pulled)
            }
            DockPlan::NewWindow { label } => {
                let pulled =
                    self.pull_docked_tab(&source_window, view, mv.successor_in(&source_window));
                // Pair update (invariants 1·2). // ADR-0057
                self.view_owner.insert(view, label.clone());
                self.insert_new_popout(&label, view);
                (label, pulled)
            }
        };
        self.bump_version();
        Ok(Docked {
            window,
            slot_ids,
            source_window,
            emptied: pulled.emptied,
            main_refilled: pulled.main_refilled,
        })
    }

    // `dock` decisions 1–5; `&self`, so nothing changes. Returns (source window, what to apply).
    fn check_dock(&self, view: ViewId, to: &DockTo) -> Result<(WindowLabel, DockPlan), DockError> {
        let source_window = self
            .tab_window(view)
            .ok_or(DockError::TabNotFound(view))?
            .clone();

        // A tombstoned window is removed from the model first, so this tombstone check is only a defense; it emits the same error as "missing". (TRD S21-slot-docking 5.8th edition §1-2)
        let target_window = match to {
            DockTo::Seam { target, .. } => self
                .tab_window(*target)
                .filter(|label| !self.closed_labels.contains(*label))
                .ok_or(DockError::TabNotFound(*target))?
                .clone(),
            DockTo::TabBar { window, .. } => {
                if !self.windows.contains_key(window) || self.closed_labels.contains(window) {
                    return Err(DockError::WindowNotFound(window.clone()));
                }
                window.clone()
            }
            DockTo::NewWindow { label, .. } => {
                if self.closed_labels.contains(label) {
                    return Err(DockError::WindowClosed(label.clone()));
                }
                if self.windows.contains_key(label) {
                    return Err(DockError::WindowExists(label.clone()));
                }
                label.clone()
            }
        };

        if let DockTo::TabBar {
            after: Some(after), ..
        } = to
        {
            if *after == view {
                return Err(DockError::SelfTarget);
            }
            if !self.windows[&target_window].tabs.contains(after) {
                return Err(DockError::TabNotFound(*after));
            }
        }

        if let DockTo::Seam { target, anchor, .. } = to {
            if *target == view {
                return Err(DockError::SelfTarget);
            }
            if let DockAnchor::Slot(slot) = *anchor {
                if !tree::contains_slot(&self.views[target].layout, slot) {
                    let elsewhere = self
                        .views
                        .values()
                        .any(|v| tree::contains_slot(&v.layout, slot));
                    return Err(if elsewhere {
                        DockError::SlotNotInTab { slot, tab: *target }
                    } else {
                        DockError::SlotNotFound(slot)
                    });
                }
            }
        }

        let source_layout = &self.views[&view].layout;
        let plan = match to {
            DockTo::Seam {
                target,
                anchor,
                side,
                share,
            } => {
                // The range assertions of `docked_tree` exist only in debug builds, so refuse by value: a NaN in the tree in release would make saving write the ratio as `null` and the next boot's read skip that tab (`state::codec`).
                // ADR-0295
                if !(share.is_finite() && (tree::RATIO_MIN..=dock::SHARE_MAX).contains(share)) {
                    return Err(DockError::InvalidShare(share.to_string()));
                }
                let layout = dock::docked_tree(
                    &self.views[target].layout,
                    *anchor,
                    *side,
                    source_layout,
                    *share,
                )
                .map_err(|e| match e {
                    DockTreeError::SlotNotFound(slot) => DockError::SlotNotFound(slot),
                    DockTreeError::SlotIdOverlap => DockError::SlotIdOverlap,
                    DockTreeError::ZeroArea => DockError::ZeroArea,
                })?;
                DockPlan::Seam {
                    target: *target,
                    target_window,
                    layout,
                }
            }
            DockTo::TabBar { index, after, .. } => {
                let mut tabs: Vec<ViewId> = self.windows[&target_window]
                    .tabs
                    .iter()
                    .copied()
                    .filter(|v| *v != view)
                    .collect();
                let at = match after.and_then(|a| tabs.iter().position(|v| *v == a)) {
                    Some(anchor) => anchor + 1,
                    None => index.unwrap_or(tabs.len()).min(tabs.len()),
                };
                tabs.insert(at, view);
                DockPlan::TabBar {
                    window: target_window,
                    tabs,
                }
            }
            DockTo::NewWindow { .. } => DockPlan::NewWindow {
                label: target_window,
            },
        };
        Ok((source_window, plan))
    }

    /// Phase A of the new-window drop: whether a new OS window is needed, and at what size. Changes nothing. Source not a tab of any window → `TabNotFound` (as [`Self::dock`] decision 1); source a popout's only tab → `Reuse{that window}` (moves that window, even if its normal position has not been seen yet); otherwise `Create{size}` — `size` = the normal-position size of the source popout (logical; the pre-maximize size even while maximized; `None` if not known yet), `None` for main (the window port's default popout size).
    pub fn plan_new_window(&self, view: ViewId) -> Result<NewWindowPlan, DockError> {
        let source_window = self.tab_window(view).ok_or(DockError::TabNotFound(view))?;
        let source = &self.windows[source_window];
        Ok(if source_window == MAIN_WINDOW_LABEL {
            NewWindowPlan::Create { size: None }
        } else if source.tabs.len() == 1 {
            NewWindowPlan::Reuse {
                label: source_window.clone(),
            }
        } else {
            NewWindowPlan::Create {
                size: source.attrs.bounds.map(|b| (b.w(), b.h())),
            }
        })
    }

    /// Puts a tombstone on window `label`: afterwards [`Self::dock`] (`NewWindow`) · [`Self::attach_view_as_new_window`] · [`Self::create_window`] attaching a new window with that label get `WindowClosed`, and `Seam` · `TabBar` into that window get "missing" (`TabNotFound` · `WindowNotFound`), with no change. The window entry and `version` are not touched (removing is the caller's job: `commands::popout::drop_window_in_model` calls it inside the same lock that removes the window, and `window_ctl::close` calls it for a window outside the model before the OS window is reaped).
    // ADR-0302 — window state commands
    pub(crate) fn mark_closed(&mut self, label: &str) {
        self.closed_labels.insert(label.to_string());
    }

    // The window that has `view` as a tab (View · ownership pair must hold). A windowless temporary View (`prepare_detached_view`) is `None`; if `Some`, `views[view]` · `windows[label]` exist (docking decisions read them as indexes).
    fn tab_window(&self, view: ViewId) -> Option<&WindowLabel> {
        if !self.views.contains_key(&view) {
            return None;
        }
        let label = self.view_owner.get(&view)?;
        self.windows
            .get(label)?
            .tabs
            .contains(&view)
            .then_some(label)
    }

    fn pull_docked_tab(
        &mut self,
        label: &str,
        view: ViewId,
        successor: Option<ViewId>,
    ) -> PulledDock {
        match self.pull_tab(label, view, successor) {
            Pulled::Stayed => PulledDock::default(),
            Pulled::PopoutRemoved => PulledDock {
                emptied: Some(label.to_string()),
                main_refilled: None,
            },
            Pulled::MainEmptied => PulledDock {
                emptied: None,
                main_refilled: Some(self.refill_main()),
            },
        }
    }

    // ── Runtime restore accept ─────────────────────────────

    /// Swaps the window set for `restored` (the result of `from_persisted`): main changes only tabs · active · attributes (measurements · persisted id remain), old popouts are removed with their entries, and `restored`'s popouts are taken in with their side table raw text. Returns the removed old popout labels (sorted); the caller reaps those OS windows outside the lock.
    ///
    /// - `version` · `attrs_rev` each go up by one from the current value (`restored`'s numbers are discarded); the writer only checks equality, so a rewound number equal to the last seen value would hide the changes in between.
    /// - Views without an owner (the temporary View of a `prepare_detached_view` creating a window) are left as is; that job's phase C attaches or reaps them.
    /// - On `Err` nothing changed: `restored` has no main · a popout label overlaps a window of the current model · a View id overlaps a remaining View.
    /// - Numbers: frees all numbers of the old popouts (closing windows included) and fixes `restored`'s popouts' numbers again with [`numbering::resolve_saved`] against the numbers held now (the remaining main's 1), in ascending number order. A late `Destroyed` of an old popout is a no-op since [`Self::release_closing`] looks up by label only.
    /// - Name ledger: `restored`'s ledger (seeds from the saved file) goes only into the empty entries of the current ledger ([`Self::seed_agent_names`]); the current values came from the daemon list. (TRD S21-storage §6-7 ③; TRD S26-window-chrome §4-1 · §5-6)
    pub(crate) fn adopt_restored(
        &mut self,
        restored: ViewManager,
    ) -> Result<Vec<WindowLabel>, LayoutError> {
        let ViewManager {
            views,
            view_owner,
            mut windows,
            version: _,
            attrs_rev: _,
            unknown_content,
            closed_labels: _,
            closing: _,
            agent_names: seeds,
            // ADR-0373 — the live manager's language stays; the restored model was built without the setting
            lang: _,
        } = restored;
        let main = windows
            .remove(MAIN_WINDOW_LABEL)
            .ok_or_else(|| LayoutError::WindowNotFound(MAIN_WINDOW_LABEL.to_string()))?;
        if !self.windows.contains_key(MAIN_WINDOW_LABEL) {
            return Err(LayoutError::WindowNotFound(MAIN_WINDOW_LABEL.to_string()));
        }
        if let Some(label) = windows.keys().find(|l| self.windows.contains_key(*l)) {
            return Err(LayoutError::WindowNotFound(label.clone()));
        }
        let owned: HashSet<ViewId> = self
            .windows
            .values()
            .flat_map(|w| w.tabs.iter().copied())
            .collect();
        if let Some(&view) = views
            .keys()
            .find(|v| self.views.contains_key(*v) && !owned.contains(*v))
        {
            return Err(LayoutError::ViewNotFound(view));
        }

        for view in owned {
            self.remove_view(view);
        }
        let mut removed: Vec<WindowLabel> = self
            .windows
            .keys()
            .filter(|l| l.as_str() != MAIN_WINDOW_LABEL)
            .cloned()
            .collect();
        removed.sort();
        // The old OS window still exists (the caller reaps it after showing the copy window), but its number is freed now for the copy window.
        for label in &removed {
            self.retire_window_entry(label, Os::Gone);
        }
        self.closing.clear();
        let live = self
            .windows
            .get_mut(MAIN_WINDOW_LABEL)
            .expect("checked above");
        live.tabs = main.tabs;
        live.active = main.active;
        live.attrs = main.attrs;
        live.placement_memo = PlacementMemo::default();
        self.views.extend(views);
        self.view_owner.extend(view_owner);
        self.unknown_content.extend(unknown_content);
        self.seed_agent_names(seeds);
        let mut popouts: Vec<(WindowLabel, WindowTabs)> = windows.into_iter().collect();
        popouts.sort_by_key(|(_, w)| w.number);
        let saved: Vec<Option<u32>> = popouts.iter().map(|(_, w)| Some(w.number)).collect();
        let numbers = numbering::resolve_saved(&saved, self.held_numbers());
        for ((label, mut window), number) in popouts.into_iter().zip(numbers) {
            window.number = number;
            self.insert_window_entry(label, window);
        }
        self.bump_version();
        self.attrs_rev += 1;
        Ok(removed)
    }

    // ── Window attributes ───────────────────────────────────────
    //
    // ★Bumps attrs_rev, not version★, only when changed. The caller reads the values outside the lock and passes them in (no OS calls here). (TRD S21-storage §6-3)

    pub fn window_attrs(&self, label: &str) -> Result<WindowAttrs, LayoutError> {
        self.windows
            .get(label)
            .map(|w| w.attrs)
            .ok_or_else(|| LayoutError::WindowNotFound(label.to_string()))
    }

    pub fn attrs_rev(&self) -> u64 {
        self.attrs_rev
    }

    /// `None` = clears the per-window theme (follows the global theme).
    pub fn set_window_theme(
        &mut self,
        label: &str,
        theme: Option<UiTheme>,
    ) -> Result<(), LayoutError> {
        self.update_attrs(label, |attrs| attrs.set_theme(theme))
    }

    /// Records only the maximize marker; `bounds` stays. Used when deferring or clearing the maximize of a window that is not visible (when that window is shown and the wish is applied, the position record writes the same value again). (TRD S26-window-chrome §5-2)
    pub fn set_window_maximized(
        &mut self,
        label: &str,
        maximized: bool,
    ) -> Result<(), LayoutError> {
        self.update_attrs(label, |attrs| {
            let changed = attrs.maximized != maximized;
            attrs.maximized = maximized;
            changed
        })
    }

    /// Records one set of getters read at `Moved` · `Resized` — the rule is [`WindowAttrs::observe`].
    pub fn observe_window_placement(
        &mut self,
        label: &str,
        seen: WindowPlacement,
    ) -> Result<(), LayoutError> {
        let window = self
            .windows
            .get_mut(label)
            .ok_or_else(|| LayoutError::WindowNotFound(label.to_string()))?;
        if window.attrs.observe(seen, &mut window.placement_memo) {
            self.attrs_rev += 1;
        }
        Ok(())
    }

    fn update_attrs(
        &mut self,
        label: &str,
        change: impl FnOnce(&mut WindowAttrs) -> bool,
    ) -> Result<(), LayoutError> {
        let window = self
            .windows
            .get_mut(label)
            .ok_or_else(|| LayoutError::WindowNotFound(label.to_string()))?;
        if change(&mut window.attrs) {
            self.attrs_rev += 1;
        }
        Ok(())
    }

    // ── Measurement reports (webview → shell) ───────────────────────────
    //
    // ★Does not bump version★: a measurement is not a layout change, is not in snapshots and has no notice (the apply service takes no port). Shell computations read it on the spot.
    // ADR-0227

    // A zero size is ignored and the previous value kept (`Ok` — [`reported_canvas`]).
    pub fn set_window_canvas(&mut self, label: &str, w: u32, h: u32) -> Result<(), LayoutError> {
        let wt = self
            .windows
            .get_mut(label)
            .ok_or_else(|| LayoutError::WindowNotFound(label.to_string()))?;
        if let Some(canvas) = reported_canvas(w, h) {
            wt.canvas = Some(canvas);
        }
        Ok(())
    }

    // Out of range is refused with `InvalidMetrics` and the previous value kept (range = the `UiMetrics` docs).
    pub fn set_ui_metrics(&mut self, label: &str, m: UiMetrics) -> Result<(), LayoutError> {
        let wt = self
            .windows
            .get_mut(label)
            .ok_or_else(|| LayoutError::WindowNotFound(label.to_string()))?;
        check_metrics(&m)?;
        wt.metrics = Some(m);
        Ok(())
    }

    // The canvas · metrics of the window owning view. `Ok(None)` if either is unknown or the View has no owning window (e.g. the temporary View of `prepare_detached_view`).
    // ADR-0227
    pub(crate) fn px_context(
        &self,
        view: ViewId,
    ) -> Result<Option<(CanvasPx, UiMetrics)>, LayoutError> {
        if !self.views.contains_key(&view) {
            return Err(LayoutError::ViewNotFound(view));
        }
        Ok(self
            .view_owner
            .get(&view)
            .and_then(|label| self.windows.get(label))
            .and_then(|wt| wt.canvas.zip(wt.metrics)))
    }

    // The px rectangle of area `slot`: `frame` follows `geometry::frame_px`, `content` `geometry::content_rect` (divider thickness is not subtracted; dividers are overlays). `Ok(None)` if the canvas or metrics are unknown (content would be a wrong value); a missing view·slot is `Err` before that. Hidden tabs are computed with the same window canvas.
    // ADR-0227
    pub fn slot_px(&self, view: ViewId, slot: Uuid) -> Result<Option<SlotPx>, LayoutError> {
        let v = self
            .views
            .get(&view)
            .ok_or(LayoutError::ViewNotFound(view))?;
        if !tree::contains_slot(&v.layout, slot) {
            return Err(LayoutError::SlotNotFound(slot));
        }
        let Some((canvas, metrics)) = self.px_context(view)? else {
            return Ok(None);
        };
        let geo = geometry::compute(&v.layout);
        let rect = geo
            .slots
            .iter()
            .find(|r| r.slot_id == slot)
            .ok_or(LayoutError::SlotNotFound(slot))?;
        let frame = geometry::frame_px(canvas.w, canvas.h, rect);
        Ok(Some(SlotPx {
            frame,
            content: geometry::content_rect(&frame, &metrics.frame_insets),
        }))
    }
}

// The accepted value of a canvas report: `None` if either `w`·`h` is 0 (not recorded) — zero-size observations are real, and storing one would make all of that window's areas 0 px. Reports of windows outside the model (`spare::pool`) follow the same rule.
pub(crate) fn reported_canvas(w: u32, h: u32) -> Option<CanvasPx> {
    (w != 0 && h != 0).then_some(CanvasPx { w, h })
}

// `RangeInclusive::contains` is false for NaN, so NaN · ±∞ are caught together by this one check (also used for reports of windows outside the model, `spare::pool`).
pub(crate) fn check_metrics(m: &UiMetrics) -> Result<(), LayoutError> {
    let i = &m.frame_insets;
    for (name, v) in [("t", i.t), ("r", i.r), ("b", i.b), ("l", i.l)] {
        if !(0.0..=INSET_MAX_PX).contains(&v) {
            return Err(LayoutError::InvalidMetrics(format!(
                "frame_insets.{name}={v} — must be finite and 0~{INSET_MAX_PX}"
            )));
        }
    }
    if !(1..=MIN_PANE_PX_MAX).contains(&m.min_pane_px) {
        return Err(LayoutError::InvalidMetrics(format!(
            "min_pane_px={} — must be 1~{MIN_PANE_PX_MAX}",
            m.min_pane_px
        )));
    }
    Ok(())
}

// Reasons `spawn_into` (D-7) slot resolution fails; the command layer turns them into strings. (TRD §6 G9)
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SpawnSlotError {
    #[error("slot {0} is already occupied (no overwrite — make an empty slot with slot.split and retry)")]
    SlotOccupied(Uuid),
    #[error("slot {0} not found")]
    SlotNotFound(Uuid),
    // ADR-0059
    #[error("no empty slot in this tab (slot not given — make an empty slot with slot.split or use another tab)")]
    NoEmptySlot,
    #[error("view {0} not found")]
    ViewNotFound(Uuid),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseTabOutcome {
    Stayed,
    // Last tab of a popup → the command layer must close the OS window (the agent survives).
    WindowClosed,
}

/// The tab that takes over the active tab when `removed` leaves tab list `tabs`: the right neighbor first, otherwise the left; `None` if `removed` is not in the list or is alone. Used by tab close · docking. ★Must match the web mirror `src/components/layout/tabStrip/succession.ts`★: the tab the source window shows during the drag must be the one this picks after the drop, so the screen does not change.
// ADR-0057 (TRD S21-slot-docking §3-3 · §4-2)
pub fn succession(tabs: &[ViewId], removed: ViewId) -> Option<ViewId> {
    let pos = tabs.iter().position(|v| *v == removed)?;
    tabs.get(pos + 1)
        .or_else(|| pos.checked_sub(1).and_then(|left| tabs.get(left)))
        .copied()
}

#[derive(Debug)]
enum Pulled {
    Stayed,
    PopoutRemoved,
    MainEmptied,
}

#[derive(Default)]
struct PulledDock {
    emptied: Option<WindowLabel>,
    main_refilled: Option<ViewId>,
}

// ── Docking types ────────────────────────────── (TRD S21-slot-docking §3-3 · §3-7)

#[derive(Debug, Clone, PartialEq)]
pub enum DockTo {
    /// Inserts the whole source tab layout at `side` next to `anchor` of target tab `target`. `share` = the share the source takes in the wrapped position (range · ratio: [`dock::docked_tree`]).
    Seam {
        target: ViewId,
        anchor: DockAnchor,
        side: DockSide,
        share: f64,
    },
    /// Puts it in as a tab of window `window`. `index` = the position in the resulting tab list (at the end if it overflows or `None`). `after` = right after tab `after` of that window; if set, `index` is ignored. Positions are counted in the list without the source (even if the source sits before `after` in the same window, it ends right after). The mouse always gives `None`. (TRD S28 §4-1)
    TabBar {
        window: WindowLabel,
        index: Option<usize>,
        after: Option<ViewId>,
    },
    /// Creates a new window entry `label` holding only that tab; `label` belongs to the OS window the caller already opened.
    NewWindow { label: WindowLabel },
}

/// The next active tab of the source window when the moved tab was its active one and that window remains: [`succession`] if `successor` is not a remaining tab or is `None`. If `successor_window` is set, `successor` is used only when the moved tab is in that window at apply time (otherwise [`succession`]; `successor` belongs to that window and must not pick another window's selection); `None` = any window. Canonical rule: [`ViewManager::dock`].
// ADR-0295 · ADR-0297
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DockMove {
    pub successor: Option<ViewId>,
    pub successor_window: Option<WindowLabel>,
}

impl DockMove {
    // The `successor` to use in source window `source`.
    fn successor_in(&self, source: &str) -> Option<ViewId> {
        self.successor.filter(|_| {
            self.successor_window
                .as_deref()
                .is_none_or(|window| window == source)
        })
    }
}

/// What to show after the drop: `Keep` = command (does not change the visible tab · focus) · `Reveal` = mouse (shows what was moved and focuses the first slot). Canonical rule: [`ViewManager::dock`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusPolicy {
    Keep,
    Reveal,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Docked {
    pub window: WindowLabel,
    /// Slot ids of the moved tab (tree preorder) — ids stay the same after moving.
    pub slot_ids: Vec<Uuid>,
    pub source_window: WindowLabel,
    /// The source popout emptied and its window entry was removed (that label); its OS window is still up and the caller decides outside the lock.
    pub emptied: Option<WindowLabel>,
    /// The empty tab newly set up in main when main's last tab left.
    pub main_refilled: Option<ViewId>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum NewWindowPlan {
    /// The source is a popout with only one tab — moves that window without a new one (no model change).
    Reuse { label: WindowLabel },
    /// Attaches with [`DockTo::NewWindow`] after opening a new window. `size` = logical inner (width, height) · `None` = default popout size.
    Create { size: Option<(f64, f64)> },
}

/// Docking failure reasons — on failure the model is unchanged. Text = an English reason + the kind word in parentheses (people · LLMs read it; machine branching goes by the kind). (TRD S21-slot-docking §3-4)
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DockError {
    #[error("tab not found: {0} (TabNotFound)")]
    TabNotFound(Uuid),
    #[error("window not found: {0} (WindowNotFound)")]
    WindowNotFound(String),
    #[error("slot not found: {0} (SlotNotFound)")]
    SlotNotFound(Uuid),
    #[error("slot {slot} does not belong to tab {tab} (SlotNotInTab)")]
    SlotNotInTab { slot: Uuid, tab: Uuid },
    #[error("a tab cannot be attached to itself (SelfTarget)")]
    SelfTarget,
    #[error("attaching would create a slot with area 0 (ZeroArea)")]
    ZeroArea,
    /// The new part's share of `Seam` is not finite or outside `tree::RATIO_MIN..=dock::SHARE_MAX` — a defect of the caller. Value = the received share.
    // ADR-0295
    #[error("attaching share out of range: {0} (InvalidShare)")]
    InvalidShare(String),
    /// A tombstone on the `NewWindow` label — that new window closed before it was attached.
    #[error("the new window closed before it was attached: {0} (WindowClosed)")]
    WindowClosed(String),
    /// An unreachable defense — the label issuer is monotonic, so reaching it is a shell defect.
    #[error("the new window label already exists: {0} (WindowExists)")]
    WindowExists(String),
    /// An unreachable defense — happens only when global slot id uniqueness (ADR-0035) is broken.
    #[error("the target and the source have the same slot id (SlotIdOverlap)")]
    SlotIdOverlap,
    /// The apply service could not open a new OS window, or could not move the source window to reuse.
    #[error("could not open the new window: {0} (WindowOpen)")]
    WindowOpen(String),
}

// What to apply after `ViewManager::check_dock` finished deciding.
enum DockPlan {
    Seam {
        target: ViewId,
        target_window: WindowLabel,
        layout: LayoutNode,
    },
    TabBar {
        window: WindowLabel,
        tabs: Vec<ViewId>,
    },
    NewWindow {
        label: WindowLabel,
    },
}

// For tests — the shell package has no test feature, so this is opened only with `cfg(test)`. (TRD S26-window-chrome §2-3)
#[cfg(test)]
impl ViewManager {
    /// The numbers held by closing windows (ascending).
    pub(crate) fn closing_numbers(&self) -> Vec<u32> {
        let mut numbers: Vec<u32> = self.closing.values().map(|c| c.number).collect();
        numbers.sort_unstable();
        numbers
    }

    /// The window number invariant: main = 1 · popouts ≥ 2 · no number repeats across live and closing windows · the label and persisted id of a closing window do not overlap windows in the model. Call at the end of write tests that insert and remove window entries (§10-1).
    pub(crate) fn assert_number_invariants(&self) {
        let main = self.windows.get(MAIN_WINDOW_LABEL).expect("main window");
        assert_eq!(main.number, numbering::MAIN_WINDOW_NUMBER, "main is 1");
        let mut seen = HashSet::new();
        for (label, w) in &self.windows {
            if label != MAIN_WINDOW_LABEL {
                assert!(
                    w.number > numbering::MAIN_WINDOW_NUMBER,
                    "number {} of popout {label}",
                    w.number
                );
            }
            assert!(
                seen.insert(w.number),
                "two windows held number {}",
                w.number
            );
        }
        for (label, c) in &self.closing {
            assert!(
                !self.windows.contains_key(label),
                "closing window {label} is in the model"
            );
            assert!(
                self.windows.values().all(|w| w.window_id != c.window_id),
                "the persisted id of closing window {label} is in the model"
            );
            assert!(
                c.number > numbering::MAIN_WINDOW_NUMBER,
                "number {} of closing window {label}",
                c.number
            );
            assert!(
                seen.insert(c.number),
                "another window also held number {} of closing window {label}",
                c.number
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::geometry::SlotRect;

    fn first_slot_of(mgr: &ViewManager, view_id: Uuid) -> Uuid {
        let v = mgr.views.get(&view_id).unwrap();
        tree::first_slot_id(&v.layout)
    }

    fn main_active(mgr: &ViewManager) -> ViewId {
        mgr.windows.get(MAIN_WINDOW_LABEL).unwrap().active
    }

    fn assert_invariants(mgr: &ViewManager) {
        for vid in mgr.views.keys() {
            assert!(
                mgr.view_owner.contains_key(vid),
                "invariant 2: View {vid} has no owning window"
            );
        }
        for (vid, label) in &mgr.view_owner {
            assert!(
                mgr.views.contains_key(vid),
                "view_owner points to a missing View"
            );
            assert!(
                mgr.windows.contains_key(label),
                "view_owner points to a missing window {label}"
            );
        }
        for (label, wt) in &mgr.windows {
            for vid in &wt.tabs {
                assert_eq!(
                    mgr.view_owner.get(vid).map(|s| s.as_str()),
                    Some(label.as_str()),
                    "invariant 1: ownership mismatch of {vid} in windows[{label}].tabs"
                );
            }
            assert!(
                wt.tabs.contains(&wt.active),
                "invariant 3: windows[{label}].active is outside tabs"
            );
            assert!(!wt.tabs.is_empty(), "empty windows must not exist");
        }
        for (vid, label) in &mgr.view_owner {
            let wt = mgr.windows.get(label).expect("owner window exists");
            assert!(
                wt.tabs.contains(vid),
                "invariant 1 reverse: view_owner[{vid}]={label} but not in tabs"
            );
        }
        assert!(
            mgr.windows
                .get(MAIN_WINDOW_LABEL)
                .map(|w| !w.tabs.is_empty())
                .unwrap_or(false),
            "invariant 4: main has at least 1 tab"
        );
        for (vid, slots) in &mgr.unknown_content {
            let v = mgr
                .views
                .get(vid)
                .unwrap_or_else(|| panic!("points to View {vid} without a side table"));
            assert!(
                !slots.is_empty(),
                "side table: an empty entry of View {vid} remains"
            );
            for sid in slots.keys() {
                assert_eq!(
                    tree::find_slot(&v.layout, *sid),
                    Some(&SlotContent::Empty),
                    "side table: slot {sid} of View {vid} is missing or its memory content is not Empty"
                );
            }
        }
        mgr.assert_number_invariants();
    }

    fn raw(kind: &str) -> Map<String, Value> {
        serde_json::json!({ "type": kind, "from": "a newer build" })
            .as_object()
            .unwrap()
            .clone()
    }

    // Sets the root of main's active tab up as an unknown content slot.
    fn foreign_root(mgr: &mut ViewManager) -> (ViewId, Uuid) {
        let v = main_active(mgr);
        let s = first_slot_of(mgr, v);
        mgr.set_unknown_content(v, s, raw("from_the_future"))
            .unwrap();
        (v, s)
    }

    #[test]
    fn new_has_main_with_one_tab() {
        let mgr = ViewManager::new();
        assert_eq!(mgr.views.len(), 1);
        let wt = mgr.windows.get(MAIN_WINDOW_LABEL).unwrap();
        assert_eq!(wt.tabs.len(), 1);
        assert_eq!(wt.active, wt.tabs[0]);
        assert_eq!(mgr.view_owner.get(&wt.tabs[0]).unwrap(), MAIN_WINDOW_LABEL);
        assert_eq!(mgr.version, 0);
        assert_eq!(mgr.windows.len(), 1);
        assert_invariants(&mgr);
    }

    #[test]
    fn new_main_default_layout_is_single_empty_slot_focused() {
        let mgr = ViewManager::new();
        let v = mgr.views.get(&main_active(&mgr)).unwrap();
        let LayoutNode::Slot {
            id,
            content: SlotContent::Empty,
        } = &v.layout
        else {
            panic!(
                "the boot default must be a single empty slot (no tree): {:?}",
                v.layout
            );
        };
        assert_eq!(v.focused_slot_id, Some(*id), "focus on that empty slot");
    }

    #[test]
    fn boot_view_has_the_same_shape_as_any_new_tab() {
        let mut mgr = ViewManager::new();
        let boot = main_active(&mgr);
        let t = mgr.create_tab(MAIN_WINDOW_LABEL, None).unwrap();
        for view in [boot, t] {
            let v = mgr.views.get(&view).unwrap();
            assert!(matches!(
                v.layout,
                LayoutNode::Slot {
                    content: SlotContent::Empty,
                    ..
                }
            ));
            assert_eq!(v.focused_slot_id, Some(first_slot_of(&mgr, view)));
        }
    }

    #[test]
    fn create_tab_appends_and_activates_and_bumps_version() {
        let mut mgr = ViewManager::new();
        let v0 = mgr.version;
        let id = mgr
            .create_tab(MAIN_WINDOW_LABEL, Some("Custom".into()))
            .unwrap();
        let wt = mgr.windows.get(MAIN_WINDOW_LABEL).unwrap();
        assert_eq!(wt.tabs.len(), 2);
        assert_eq!(wt.active, id, "the new tab is active");
        assert_eq!(
            mgr.views.get(&id).unwrap().name,
            TabName::Named("Custom".into())
        );
        assert_eq!(mgr.view_owner.get(&id).unwrap(), MAIN_WINDOW_LABEL);
        assert_eq!(mgr.version, v0 + 1);
        assert_invariants(&mgr);
    }

    #[test]
    fn create_tab_unknown_window_is_err() {
        let mut mgr = ViewManager::new();
        let err = mgr.create_tab("no-such", None).unwrap_err();
        assert!(matches!(err, LayoutError::WindowNotFound(_)));
        assert_invariants(&mgr);
    }

    #[test]
    fn switch_tab_changes_active_only_that_window() {
        let mut mgr = ViewManager::new();
        let main0 = main_active(&mgr);
        let t1 = mgr.create_tab(MAIN_WINDOW_LABEL, None).unwrap();
        let pv = mgr.create_window("slot-popup-1").unwrap();
        mgr.switch_tab(MAIN_WINDOW_LABEL, main0).unwrap();
        assert_eq!(main_active(&mgr), main0);
        assert_eq!(mgr.windows.get("slot-popup-1").unwrap().active, pv);
        mgr.switch_tab(MAIN_WINDOW_LABEL, t1).unwrap();
        assert_eq!(main_active(&mgr), t1);
        assert_invariants(&mgr);
    }

    #[test]
    fn switch_tab_invalid_view_is_err_noop() {
        let mut mgr = ViewManager::new();
        let ver = mgr.version;
        assert!(mgr.switch_tab(MAIN_WINDOW_LABEL, Uuid::new_v4()).is_err());
        assert_eq!(mgr.version, ver);
    }

    #[test]
    fn create_window_makes_new_window_with_one_tab() {
        let mut mgr = ViewManager::new();
        let v = mgr.create_window("slot-popup-1").unwrap();
        let wt = mgr.windows.get("slot-popup-1").unwrap();
        assert_eq!(wt.tabs, vec![v]);
        assert_eq!(wt.active, v);
        assert_eq!(mgr.view_owner.get(&v).unwrap(), "slot-popup-1");
        assert_invariants(&mgr);
    }

    #[test]
    fn create_window_duplicate_label_is_err() {
        let mut mgr = ViewManager::new();
        mgr.create_window("slot-popup-1").unwrap();
        assert_eq!(
            mgr.create_window("slot-popup-1"),
            Err(LayoutError::WindowNotFound("slot-popup-1".into()))
        );
        assert_eq!(
            mgr.create_window(MAIN_WINDOW_LABEL),
            Err(LayoutError::WindowNotFound(MAIN_WINDOW_LABEL.into()))
        );
    }

    // TRD S21-slot-docking §8-1 ⑬ — a window closed after the OS window opened · before inserting (tombstone) does not enter the model.
    #[test]
    fn create_window_refuses_a_tombstoned_label_unchanged() {
        let mut mgr = ViewManager::new();
        mgr.mark_closed("slot-popup-1");
        let before = model(&mgr);

        assert_eq!(
            mgr.create_window("slot-popup-1"),
            Err(LayoutError::WindowClosed("slot-popup-1".into()))
        );
        assert_eq!(model(&mgr), before);
        assert!(!mgr.windows.contains_key("slot-popup-1"));
        assert_invariants(&mgr);
        // A label without a tombstone is created as is.
        mgr.create_window("slot-popup-2").unwrap();
        assert_invariants(&mgr);
    }

    // ── close_tab state machine (§5-2) ─────────────────────────────────────────────

    #[test]
    fn close_active_tab_succeeds_right_neighbor() {
        let mut mgr = ViewManager::new();
        let a = main_active(&mgr);
        let b = mgr.create_tab(MAIN_WINDOW_LABEL, None).unwrap();
        let c = mgr.create_tab(MAIN_WINDOW_LABEL, None).unwrap();
        mgr.switch_tab(MAIN_WINDOW_LABEL, b).unwrap();
        let out = mgr.close_tab(MAIN_WINDOW_LABEL, b).unwrap();
        assert_eq!(out, CloseTabOutcome::Stayed);
        assert_eq!(main_active(&mgr), c, "the right tab takes over");
        let wt = mgr.windows.get(MAIN_WINDOW_LABEL).unwrap();
        assert_eq!(wt.tabs, vec![a, c]);
        assert!(!mgr.views.contains_key(&b), "the closed View is dropped");
        assert!(!mgr.view_owner.contains_key(&b));
        assert_invariants(&mgr);
    }

    #[test]
    fn close_active_last_tab_succeeds_left_neighbor() {
        let mut mgr = ViewManager::new();
        let a = main_active(&mgr);
        let b = mgr.create_tab(MAIN_WINDOW_LABEL, None).unwrap();
        mgr.close_tab(MAIN_WINDOW_LABEL, b).unwrap();
        assert_eq!(main_active(&mgr), a, "the left tab takes over (no right)");
        assert_invariants(&mgr);
    }

    #[test]
    fn close_non_active_tab_keeps_active() {
        let mut mgr = ViewManager::new();
        let a = main_active(&mgr);
        let b = mgr.create_tab(MAIN_WINDOW_LABEL, None).unwrap();
        mgr.close_tab(MAIN_WINDOW_LABEL, a).unwrap();
        assert_eq!(main_active(&mgr), b, "closing an inactive one keeps active");
        assert_invariants(&mgr);
    }

    #[test]
    fn close_main_last_tab_forces_empty_tab() {
        let mut mgr = ViewManager::new();
        let v0 = main_active(&mgr);
        let out = mgr.close_tab(MAIN_WINDOW_LABEL, v0).unwrap();
        assert_eq!(
            out,
            CloseTabOutcome::Stayed,
            "main does not close the window"
        );
        let wt = mgr.windows.get(MAIN_WINDOW_LABEL).unwrap();
        assert_eq!(wt.tabs.len(), 1, "one empty tab forced");
        assert_ne!(wt.tabs[0], v0, "a new empty View id");
        let new_id = wt.tabs[0];
        assert!(matches!(
            mgr.views.get(&new_id).unwrap().layout,
            LayoutNode::Slot {
                content: SlotContent::Empty,
                ..
            }
        ));
        assert_eq!(wt.active, new_id);
        assert_invariants(&mgr);
    }

    #[test]
    fn close_popup_last_tab_closes_window() {
        let mut mgr = ViewManager::new();
        let pv = mgr.create_window("slot-popup-1").unwrap();
        let out = mgr.close_tab("slot-popup-1", pv).unwrap();
        assert_eq!(out, CloseTabOutcome::WindowClosed);
        assert!(!mgr.windows.contains_key("slot-popup-1"), "window removed");
        assert!(!mgr.views.contains_key(&pv), "tab View dropped");
        assert!(!mgr.view_owner.contains_key(&pv));
        assert_invariants(&mgr);
    }

    #[test]
    fn close_popup_non_last_tab_stays() {
        let mut mgr = ViewManager::new();
        let p0 = mgr.create_window("slot-popup-1").unwrap();
        let p1 = mgr.create_tab("slot-popup-1", None).unwrap();
        let out = mgr.close_tab("slot-popup-1", p1).unwrap();
        assert_eq!(out, CloseTabOutcome::Stayed);
        assert!(mgr.windows.contains_key("slot-popup-1"));
        assert_eq!(mgr.windows.get("slot-popup-1").unwrap().tabs, vec![p0]);
        assert_invariants(&mgr);
    }

    #[test]
    fn close_tab_invalid_view_is_err_noop() {
        let mut mgr = ViewManager::new();
        let ver = mgr.version;
        let n = mgr.views.len();
        assert!(mgr.close_tab(MAIN_WINDOW_LABEL, Uuid::new_v4()).is_err());
        assert_eq!(mgr.version, ver);
        assert_eq!(mgr.views.len(), n);
        assert_invariants(&mgr);
    }

    #[test]
    fn close_tab_unknown_window_is_err() {
        let mut mgr = ViewManager::new();
        assert!(mgr.close_tab("no-such", Uuid::new_v4()).is_err());
    }

    // ── close_window (§5-2/G1 multi-tab cleanup) ────────────────────────────────────

    #[test]
    fn close_window_main_is_rejected() {
        let mut mgr = ViewManager::new();
        let err = mgr.close_window(MAIN_WINDOW_LABEL).unwrap_err();
        assert!(matches!(err, LayoutError::MainNotClosable));
        assert_invariants(&mgr);
    }

    #[test]
    fn close_window_multitab_drops_all_views() {
        let mut mgr = ViewManager::new();
        let p0 = mgr.create_window("slot-popup-1").unwrap();
        let p1 = mgr.create_tab("slot-popup-1", None).unwrap();
        let p2 = mgr.create_tab("slot-popup-1", None).unwrap();
        let dropped = mgr.close_window("slot-popup-1").unwrap();
        // Order-independent — set comparison.
        assert_eq!(dropped.len(), 3);
        for v in [p0, p1, p2] {
            assert!(dropped.contains(&v));
            assert!(!mgr.views.contains_key(&v), "0 View leftovers");
            assert!(!mgr.view_owner.contains_key(&v), "0 view_owner leftovers");
        }
        assert!(
            !mgr.windows.contains_key("slot-popup-1"),
            "window entry removed"
        );
        assert_invariants(&mgr);
    }

    #[test]
    fn close_window_unknown_is_err() {
        let mut mgr = ViewManager::new();
        assert!(mgr.close_window("no-such").is_err());
    }

    // ── split/close_slot/assign (view-id keys, owning window derived) ────────────────────

    #[test]
    fn split_slot_creates_new_slot_and_focuses_it() {
        let mut mgr = ViewManager::new();
        let view_id = main_active(&mgr);
        let slot = first_slot_of(&mgr, view_id);
        let new_id = mgr.split_slot(view_id, slot, SplitDir::LeftRight).unwrap();
        let v = mgr.views.get(&view_id).unwrap();
        assert!(matches!(v.layout, LayoutNode::Split { .. }));
        assert_eq!(v.focused_slot_id, Some(new_id));
        assert_invariants(&mgr);
    }

    #[test]
    fn split_invalid_view_is_err() {
        let mut mgr = ViewManager::new();
        assert!(matches!(
            mgr.split_slot(Uuid::new_v4(), Uuid::new_v4(), SplitDir::LeftRight)
                .unwrap_err(),
            LayoutError::ViewNotFound(_)
        ));
    }

    #[test]
    fn split_invalid_slot_is_err_noop() {
        let mut mgr = ViewManager::new();
        let view_id = main_active(&mgr);
        let before = mgr.views.get(&view_id).unwrap().layout.clone();
        let ver = mgr.version;
        assert!(matches!(
            mgr.split_slot(view_id, Uuid::new_v4(), SplitDir::TopBottom)
                .unwrap_err(),
            LayoutError::SlotNotFound(_)
        ));
        assert_eq!(mgr.views.get(&view_id).unwrap().layout, before);
        assert_eq!(mgr.version, ver);
    }

    #[test]
    fn close_slot_focus_fallback_to_first() {
        let mut mgr = ViewManager::new();
        let view_id = main_active(&mgr);
        let slot = first_slot_of(&mgr, view_id);
        let new_id = mgr.split_slot(view_id, slot, SplitDir::LeftRight).unwrap();
        mgr.close_slot(view_id, new_id).unwrap();
        let v = mgr.views.get(&view_id).unwrap();
        assert_eq!(v.focused_slot_id, Some(slot));
        assert_invariants(&mgr);
    }

    #[test]
    fn close_root_slot_keeps_view_empty() {
        let mut mgr = ViewManager::new();
        let view_id = main_active(&mgr);
        let slot = first_slot_of(&mgr, view_id);
        mgr.assign_agent(view_id, slot, "agent-x".into()).unwrap();
        mgr.close_slot(view_id, slot).unwrap();
        let v = mgr.views.get(&view_id).unwrap();
        assert!(matches!(
            v.layout,
            LayoutNode::Slot {
                content: SlotContent::Empty,
                ..
            }
        ));
        assert!(v.focused_slot_id.is_some());
        assert_invariants(&mgr);
    }

    #[test]
    fn close_slot_invalid_is_err_noop() {
        let mut mgr = ViewManager::new();
        let view_id = main_active(&mgr);
        let ver = mgr.version;
        assert!(mgr.close_slot(view_id, Uuid::new_v4()).is_err());
        assert_eq!(mgr.version, ver);
    }

    // ── set_focused_slot (click-to-focus — ADR-0066 decision 1) ───────────────────

    #[test]
    fn set_focused_slot_updates_focus_and_bumps_version() {
        let mut mgr = ViewManager::new();
        let view_id = main_active(&mgr);
        let left = first_slot_of(&mgr, view_id);
        let right = mgr.split_slot(view_id, left, SplitDir::LeftRight).unwrap();
        assert_eq!(
            mgr.views.get(&view_id).unwrap().focused_slot_id,
            Some(right)
        );
        let ver = mgr.version;
        mgr.set_focused_slot(view_id, left).unwrap();
        assert_eq!(
            mgr.views.get(&view_id).unwrap().focused_slot_id,
            Some(left),
            "focus moves to the clicked slot"
        );
        assert_eq!(mgr.version, ver + 1, "version +1 on success");
        assert_invariants(&mgr);
    }

    #[test]
    fn set_focused_slot_invalid_view_is_err() {
        let mut mgr = ViewManager::new();
        assert!(matches!(
            mgr.set_focused_slot(Uuid::new_v4(), Uuid::new_v4())
                .unwrap_err(),
            LayoutError::ViewNotFound(_)
        ));
    }

    #[test]
    fn set_focused_slot_absent_slot_is_err_noop() {
        let mut mgr = ViewManager::new();
        let view_id = main_active(&mgr);
        let before = mgr.views.get(&view_id).unwrap().focused_slot_id;
        let ver = mgr.version;
        assert!(matches!(
            mgr.set_focused_slot(view_id, Uuid::new_v4()).unwrap_err(),
            LayoutError::SlotNotFound(_)
        ));
        assert_eq!(
            mgr.views.get(&view_id).unwrap().focused_slot_id,
            before,
            "focused_slot_id unchanged on failure"
        );
        assert_eq!(mgr.version, ver, "version unchanged on failure (no-op)");
        assert_invariants(&mgr);
    }

    #[test]
    fn rename_tab_renames_and_bumps_version() {
        let mut mgr = ViewManager::new();
        let view_id = mgr
            .create_tab(MAIN_WINDOW_LABEL, Some("Old".to_string()))
            .unwrap();
        let ver = mgr.version;
        mgr.rename_tab(view_id, "New Name".to_string()).unwrap();
        assert_eq!(
            mgr.views.get(&view_id).unwrap().name,
            TabName::Named("New Name".into()),
            "View.name is replaced"
        );
        assert_eq!(mgr.version, ver + 1, "version +1 on success");
        assert_invariants(&mgr);
    }

    // AC36 — the model refuses an empty name even if the boundary does not filter it (name · version unchanged).
    #[test]
    fn rename_tab_refuses_a_blank_name() {
        let mut mgr = ViewManager::new();
        let view = main_active(&mgr);
        mgr.rename_tab(view, "Work".into()).unwrap();
        let ver = mgr.version;
        for blank in ["", " ", "\t\n", "\u{3000}"] {
            assert_eq!(
                mgr.rename_tab(view, blank.into()),
                Err(LayoutError::EmptyName),
                "{blank:?}"
            );
        }
        assert_eq!(mgr.views[&view].name, TabName::Named("Work".into()));
        assert_eq!(mgr.version, ver);
    }

    // Even renaming to the same text as the auto name sets that name (TRD S26-window-chrome §3-1).
    #[test]
    fn rename_tab_to_the_current_auto_name_fixes_it() {
        let mut mgr = ViewManager::new();
        let view = main_active(&mgr);
        assert_eq!(
            mgr.list_tabs(MAIN_WINDOW_LABEL).unwrap().tabs[0].name,
            "Empty slot"
        );
        mgr.rename_tab(view, "Empty slot".into()).unwrap();
        assert_eq!(mgr.views[&view].name, TabName::Named("Empty slot".into()));
        mgr.assign_agent(view, first_slot_of(&mgr, view), "a1".into())
            .unwrap();
        mgr.set_language(Lang::Ko);
        let meta = &mgr.list_tabs(MAIN_WINDOW_LABEL).unwrap().tabs[0];
        assert_eq!((meta.name.as_str(), meta.named), ("Empty slot", true));
    }

    #[test]
    fn create_tab_with_a_blank_name_is_refused_without_a_tab() {
        let mut mgr = ViewManager::new();
        let ver = mgr.version;
        assert_eq!(
            mgr.create_tab(MAIN_WINDOW_LABEL, Some("  ".into())),
            Err(LayoutError::EmptyName)
        );
        assert_eq!(mgr.views.len(), 1);
        assert_eq!(mgr.version, ver);
        assert_invariants(&mgr);
    }

    #[test]
    fn rename_tab_invalid_view_is_err_noop() {
        let mut mgr = ViewManager::new();
        let ver = mgr.version;
        assert!(matches!(
            mgr.rename_tab(Uuid::new_v4(), "X".to_string()).unwrap_err(),
            LayoutError::ViewNotFound(_)
        ));
        assert_eq!(mgr.version, ver, "version unchanged on failure (no-op)");
        assert_invariants(&mgr);
    }

    // ── Agent name ledger (TRD S26-window-chrome §4-3) ──

    fn names(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(id, name)| (id.to_string(), name.to_string()))
            .collect()
    }

    fn tab_names(mgr: &ViewManager, label: &str) -> Vec<String> {
        mgr.list_tabs(label)
            .unwrap()
            .tabs
            .into_iter()
            .map(|t| t.name)
            .collect()
    }

    // main = auto tab (a1) · set tab (a1) · popout = auto tab (a2).
    fn agents_in_two_windows() -> ViewManager {
        let mut mgr = ViewManager::new();
        let auto = main_active(&mgr);
        mgr.assign_agent(auto, first_slot_of(&mgr, auto), "a1".into())
            .unwrap();
        let named = mgr
            .create_tab(MAIN_WINDOW_LABEL, Some("Work".into()))
            .unwrap();
        mgr.assign_agent(named, first_slot_of(&mgr, named), "a1".into())
            .unwrap();
        let popout = mgr.create_window("slot-popup-1").unwrap();
        mgr.assign_agent(popout, first_slot_of(&mgr, popout), "a2".into())
            .unwrap();
        mgr
    }

    #[test]
    fn a_name_update_reports_only_windows_whose_auto_tab_name_changed_and_bumps_the_version() {
        let mut mgr = agents_in_two_windows();
        assert_eq!(tab_names(&mgr, MAIN_WINDOW_LABEL), ["Agent", "Work"]);
        let ver = mgr.version;

        assert_eq!(
            mgr.update_agent_names(&names(&[("a1", "myproj")])),
            [MAIN_WINDOW_LABEL]
        );
        assert_eq!(mgr.version, ver + 1);
        assert_eq!(tab_names(&mgr, MAIN_WINDOW_LABEL), ["myproj", "Work"]);
        assert_eq!(tab_names(&mgr, "slot-popup-1"), ["Agent"]);

        assert_eq!(
            mgr.update_agent_names(&names(&[("a1", "other"), ("a2", "beta")])),
            [MAIN_WINDOW_LABEL, "slot-popup-1"],
            "sorted · no duplicates"
        );
        assert_eq!(mgr.version, ver + 2, "one version per go");
        assert_invariants(&mgr);
    }

    #[test]
    fn a_name_update_that_changes_no_visible_name_keeps_the_version() {
        let mut mgr = agents_in_two_windows();
        mgr.update_agent_names(&names(&[("a1", "myproj")]));
        let ver = mgr.version;

        assert!(mgr
            .update_agent_names(&names(&[("a1", "myproj")]))
            .is_empty());
        assert!(
            mgr.update_agent_names(&names(&[("a9", "nowhere")]))
                .is_empty(),
            "an agent not on screen"
        );
        assert!(mgr.update_agent_names(&[]).is_empty());
        assert_eq!(mgr.version, ver);
        assert_eq!(
            mgr.agent_name("a9"),
            Some("nowhere"),
            "it is recorded in the ledger"
        );
    }

    // ADR-0373 — the same contract as a name-ledger update: only windows with a changed automatic name, one bump.
    #[test]
    fn a_language_change_reports_the_windows_with_auto_tabs_and_bumps_the_version_once() {
        let mut mgr = agents_in_two_windows();
        let ver = mgr.version;

        assert_eq!(
            mgr.set_language(Lang::Ko),
            [MAIN_WINDOW_LABEL, "slot-popup-1"]
        );
        assert_eq!(mgr.language(), Lang::Ko);
        assert_eq!(mgr.version, ver + 1);
        assert_eq!(tab_names(&mgr, MAIN_WINDOW_LABEL), ["에이전트", "Work"]); // hangul-ok: the Korean UI table
        assert_eq!(tab_names(&mgr, "slot-popup-1"), ["에이전트"]); // hangul-ok: the Korean UI table

        assert!(mgr.set_language(Lang::Ko).is_empty(), "the same language");
        assert_eq!(mgr.version, ver + 1);
        assert_invariants(&mgr);
    }

    #[test]
    fn a_language_change_over_named_tabs_and_ledger_names_keeps_the_version() {
        let mut mgr = agents_in_two_windows();
        mgr.update_agent_names(&names(&[("a1", "myproj"), ("a2", "beta")]));
        let ver = mgr.version;

        assert!(mgr.set_language(Lang::Ko).is_empty());
        assert_eq!(mgr.language(), Lang::Ko, "the language is kept even so");
        assert_eq!(mgr.version, ver);
        assert_eq!(tab_names(&mgr, MAIN_WINDOW_LABEL), ["myproj", "Work"]);
    }

    #[test]
    fn adopt_restored_keeps_the_live_language() {
        let mut live = ViewManager::new();
        live.set_language(Lang::Ko);
        let restored = ViewManager::new();
        assert_eq!(restored.language(), Lang::En);

        live.adopt_restored(restored).unwrap();

        assert_eq!(live.language(), Lang::Ko);
        assert_eq!(tab_names(&live, MAIN_WINDOW_LABEL), ["빈 칸"]); // hangul-ok: the Korean UI table
    }

    // An agent slot outside the focus is not the tab name — even if that name changes, that window is not a changed window.
    #[test]
    fn an_agent_outside_the_focused_slot_does_not_change_the_tab() {
        let mut mgr = ViewManager::new();
        let view = main_active(&mgr);
        let left = first_slot_of(&mgr, view);
        let right = mgr.split_slot(view, left, SplitDir::LeftRight).unwrap();
        mgr.assign_agent(view, left, "a1".into()).unwrap();
        mgr.set_slot_content(view, right, SlotContent::AgentList)
            .unwrap();
        let ver = mgr.version;

        assert!(mgr
            .update_agent_names(&names(&[("a1", "myproj")]))
            .is_empty());
        assert_eq!(mgr.version, ver);
        mgr.set_focused_slot(view, left).unwrap();
        assert_eq!(tab_names(&mgr, MAIN_WINDOW_LABEL), ["myproj"]);
    }

    // The slots of agents dropped from the list (dead) also use the last name — the ledger is not cleared.
    #[test]
    fn a_name_update_never_forgets_an_agent() {
        let mut mgr = agents_in_two_windows();
        mgr.update_agent_names(&names(&[("a1", "myproj")]));
        mgr.update_agent_names(&names(&[("a2", "beta")]));
        assert_eq!(tab_names(&mgr, MAIN_WINDOW_LABEL), ["myproj", "Work"]);
        assert_eq!(mgr.agent_name("a1"), Some("myproj"));
    }

    #[test]
    fn a_seed_fills_only_empty_entries_and_keeps_the_version() {
        let mut mgr = agents_in_two_windows();
        mgr.update_agent_names(&names(&[("a1", "live")]));
        let ver = mgr.version;

        mgr.seed_agent_names(names(&[("a1", "old"), ("a2", "seeded")]));

        assert_eq!(mgr.agent_name("a1"), Some("live"));
        assert_eq!(mgr.agent_name("a2"), Some("seeded"));
        assert_eq!(mgr.version, ver);
    }

    // Copy accept — the copy's ledger (the saved file's seeds) goes only into the empty entries of the current ledger (TRD S26-window-chrome §4-3).
    #[test]
    fn adopting_a_restored_copy_adds_its_seeds_only_to_empty_entries() {
        let mut mgr = ViewManager::new();
        mgr.update_agent_names(&names(&[("a1", "live")]));
        let mut copy = ViewManager::new();
        copy.seed_agent_names(names(&[("a1", "old"), ("a2", "seeded")]));

        mgr.adopt_restored(copy).unwrap();

        assert_eq!(mgr.agent_name("a1"), Some("live"));
        assert_eq!(mgr.agent_name("a2"), Some("seeded"));
    }

    #[test]
    fn a_refused_adoption_adds_no_seed() {
        let mut mgr = ViewManager::new();
        let mut no_main = ViewManager::new();
        no_main.seed_agent_names(names(&[("a1", "seeded")]));
        no_main.windows.remove(MAIN_WINDOW_LABEL);

        assert!(mgr.adopt_restored(no_main).is_err());
        assert_eq!(mgr.agent_name("a1"), None);
    }

    #[test]
    fn assign_agent_sets_ref() {
        let mut mgr = ViewManager::new();
        let view_id = main_active(&mgr);
        let slot = first_slot_of(&mgr, view_id);
        mgr.assign_agent(view_id, slot, "agent-42".into()).unwrap();
        let v = mgr.views.get(&view_id).unwrap();
        assert_eq!(
            tree::find_slot(&v.layout, slot).unwrap().agent_id(),
            Some("agent-42")
        );
        assert_invariants(&mgr);
    }

    #[test]
    fn assign_same_agent_to_two_views_is_allowed() {
        let mut mgr = ViewManager::new();
        let v1 = main_active(&mgr);
        let s1 = first_slot_of(&mgr, v1);
        mgr.assign_agent(v1, s1, "shared".into()).unwrap();
        let v2 = mgr.create_tab(MAIN_WINDOW_LABEL, None).unwrap();
        let s2 = first_slot_of(&mgr, v2);
        mgr.assign_agent(v2, s2, "shared".into()).unwrap();
        assert_eq!(mgr.slot_agent(v1, s1).unwrap().as_deref(), Some("shared"));
        assert_eq!(mgr.slot_agent(v2, s2).unwrap().as_deref(), Some("shared"));
        assert_invariants(&mgr);
    }

    #[test]
    fn assign_agent_invalid_view_is_err() {
        let mut mgr = ViewManager::new();
        assert!(mgr
            .assign_agent(Uuid::new_v4(), Uuid::new_v4(), "x".into())
            .is_err());
    }

    // ── set_slot_content (generic placement command — ADR-0063) ─────────────────────

    #[test]
    fn set_slot_content_places_agent_list() {
        let mut mgr = ViewManager::new();
        let v = mgr.create_tab(MAIN_WINDOW_LABEL, None).unwrap();
        let slot = first_slot_of(&mgr, v);
        let ver = mgr.version;
        mgr.set_slot_content(v, slot, SlotContent::AgentList)
            .unwrap();
        assert_eq!(
            tree::find_slot(&mgr.views.get(&v).unwrap().layout, slot).unwrap(),
            &SlotContent::AgentList
        );
        assert_eq!(mgr.version, ver + 1, "version +1 on success");
        assert_invariants(&mgr);
    }

    #[test]
    fn set_slot_content_can_clear_to_empty() {
        let mut mgr = ViewManager::new();
        let v = mgr.create_tab(MAIN_WINDOW_LABEL, None).unwrap();
        let slot = first_slot_of(&mgr, v);
        mgr.assign_agent(v, slot, "occupant".into()).unwrap();
        mgr.set_slot_content(v, slot, SlotContent::Empty).unwrap();
        assert!(tree::find_slot(&mgr.views.get(&v).unwrap().layout, slot)
            .unwrap()
            .is_empty());
        assert_invariants(&mgr);
    }

    #[test]
    fn set_slot_content_invalid_view_is_err() {
        let mut mgr = ViewManager::new();
        assert!(matches!(
            mgr.set_slot_content(Uuid::new_v4(), Uuid::new_v4(), SlotContent::AgentList)
                .unwrap_err(),
            LayoutError::ViewNotFound(_)
        ));
    }

    #[test]
    fn set_slot_content_invalid_slot_is_err_noop() {
        let mut mgr = ViewManager::new();
        let v = mgr.create_tab(MAIN_WINDOW_LABEL, None).unwrap();
        let ver = mgr.version;
        assert!(matches!(
            mgr.set_slot_content(v, Uuid::new_v4(), SlotContent::PresetPalette)
                .unwrap_err(),
            LayoutError::SlotNotFound(_)
        ));
        assert_eq!(mgr.version, ver, "version unchanged on failure (no-op)");
    }

    #[test]
    fn owner_of_derives_window_o1() {
        let mut mgr = ViewManager::new();
        let main = main_active(&mgr);
        assert_eq!(mgr.owner_of(main).map(|s| s.as_str()), Some("main"));
        let pv = mgr.create_window("slot-popup-1").unwrap();
        assert_eq!(mgr.owner_of(pv).map(|s| s.as_str()), Some("slot-popup-1"));
    }

    // ── list_tabs / list_windows ─────────────────────────────────────────────

    #[test]
    fn list_tabs_returns_tabs_active_version() {
        let mut mgr = ViewManager::new();
        let t1 = mgr
            .create_tab(MAIN_WINDOW_LABEL, Some("Second".into()))
            .unwrap();
        let snap = mgr.list_tabs(MAIN_WINDOW_LABEL).unwrap();
        assert_eq!(snap.tabs.len(), 2);
        assert_eq!(snap.active, t1);
        assert_eq!(snap.version, mgr.version);
        assert_eq!(snap.tabs[1].name, "Second");
        assert!(snap.tabs[1].named);
        assert_eq!(
            snap.tabs[0].name, "Empty slot",
            "auto-named tab = the word of the focused area"
        );
        assert!(!snap.tabs[0].named);
    }

    #[test]
    fn list_tabs_unknown_window_is_err() {
        let mgr = ViewManager::new();
        assert!(mgr.list_tabs("no-such").is_err());
    }

    #[test]
    fn list_windows_lists_main_only_initially() {
        let mgr = ViewManager::new();
        let ws = mgr.list_windows();
        assert_eq!(ws, vec![MAIN_WINDOW_LABEL.to_string()]);
    }

    // ── snapshot ─────────────────────────────────────────────────────────────

    #[test]
    fn snapshot_returns_version_and_layout() {
        let mut mgr = ViewManager::new();
        let view_id = main_active(&mgr);
        let slot = first_slot_of(&mgr, view_id);
        mgr.split_slot(view_id, slot, SplitDir::LeftRight).unwrap();
        let snap = mgr.snapshot(view_id).unwrap();
        assert_eq!(snap.view_id, view_id);
        assert_eq!(snap.version, mgr.version);
        assert!(matches!(snap.layout, LayoutNode::Split { .. }));
    }

    #[test]
    fn snapshot_carries_geometry_and_ratio_bounds() {
        let mut mgr = ViewManager::new();
        let view_id = main_active(&mgr);
        let slot = first_slot_of(&mgr, view_id);
        let right = mgr.split_slot(view_id, slot, SplitDir::LeftRight).unwrap();
        mgr.split_slot(view_id, right, SplitDir::TopBottom).unwrap();
        let snap = mgr.snapshot(view_id).unwrap();
        let geo = geometry::compute(&snap.layout);
        assert_eq!(snap.slot_rects, geo.slots);
        assert_eq!(snap.split_rects, geo.splits);
        assert_eq!(snap.slot_rects.len(), 3);
        assert_eq!(snap.split_rects.len(), 2);
        assert_eq!(snap.ratio_min, tree::RATIO_MIN);
        assert_eq!(snap.ratio_max, tree::RATIO_MAX);
    }

    #[test]
    fn snapshot_invalid_view_is_err() {
        let mgr = ViewManager::new();
        assert!(mgr.snapshot(Uuid::new_v4()).is_err());
    }

    // ── move_slot_to_window 2-phase support (§5-3, G4) ──────────────────────────

    #[test]
    fn prepare_detached_view_moves_agent_without_window() {
        let mut mgr = ViewManager::new();
        let src = main_active(&mgr);
        let slot = first_slot_of(&mgr, src);
        mgr.assign_agent(src, slot, "moving".into()).unwrap();
        let (tmp, content) = mgr.prepare_detached_view(src, slot).unwrap();
        assert_eq!(
            content,
            SlotContent::Agent {
                agent_id: "moving".into()
            }
        );
        let tslot = first_slot_of(&mgr, tmp);
        assert_eq!(
            mgr.slot_agent(tmp, tslot).unwrap().as_deref(),
            Some("moving")
        );
        assert!(
            mgr.view_owner.get(&tmp).is_none(),
            "phase A does not assign view_owner"
        );
        assert_eq!(
            mgr.slot_agent(src, slot).unwrap().as_deref(),
            Some("moving")
        );
    }

    #[test]
    fn prepare_detached_view_moves_agent_list_content() {
        let mut mgr = ViewManager::new();
        let src = main_active(&mgr);
        let slot = first_slot_of(&mgr, src);
        mgr.set_slot_content(src, slot, SlotContent::AgentList)
            .unwrap();
        let (tmp, content) = mgr.prepare_detached_view(src, slot).unwrap();
        assert_eq!(
            content,
            SlotContent::AgentList,
            "returned content = AgentList"
        );
        let tslot = first_slot_of(&mgr, tmp);
        assert_eq!(
            tree::find_slot(&mgr.views.get(&tmp).unwrap().layout, tslot).unwrap(),
            &SlotContent::AgentList
        );
        assert_eq!(
            tree::find_slot(&mgr.views.get(&src).unwrap().layout, slot).unwrap(),
            &SlotContent::AgentList
        );
    }

    // ADR-0228
    #[test]
    fn prepare_detached_view_moves_an_empty_slot() {
        let mut mgr = ViewManager::new();
        let src = main_active(&mgr);
        let slot = first_slot_of(&mgr, src);
        mgr.set_slot_content(src, slot, SlotContent::Empty).unwrap();
        let (tmp, content) = mgr.prepare_detached_view(src, slot).unwrap();
        assert_eq!(content, SlotContent::Empty, "returned content = Empty");
        let tslot = first_slot_of(&mgr, tmp);
        assert_eq!(
            tree::find_slot(&mgr.views.get(&tmp).unwrap().layout, tslot).unwrap(),
            &SlotContent::Empty
        );
        assert!(
            mgr.view_owner.get(&tmp).is_none(),
            "phase A does not assign view_owner"
        );
        assert_eq!(
            tree::find_slot(&mgr.views.get(&src).unwrap().layout, slot).unwrap(),
            &SlotContent::Empty
        );
    }

    #[test]
    fn prepare_detached_view_missing_slot_or_view_is_err_without_a_view() {
        let mut mgr = ViewManager::new();
        let src = main_active(&mgr);
        let views_before = mgr.views.len();
        let ghost = Uuid::new_v4();
        assert_eq!(
            mgr.prepare_detached_view(src, ghost),
            Err(LayoutError::SlotNotFound(ghost))
        );
        assert_eq!(
            mgr.prepare_detached_view(ghost, ghost),
            Err(LayoutError::ViewNotFound(ghost))
        );
        assert_eq!(
            mgr.views.len(),
            views_before,
            "a refusal leaves no temporary View"
        );
    }

    #[test]
    fn insert_tab_into_existing_window_phase_c() {
        let mut mgr = ViewManager::new();
        let src = main_active(&mgr);
        let slot = first_slot_of(&mgr, src);
        mgr.assign_agent(src, slot, "moving".into()).unwrap();
        let existing = mgr.create_window("slot-popup-1").unwrap();
        let (tmp, _content) = mgr.prepare_detached_view(src, slot).unwrap();
        mgr.insert_tab_into("slot-popup-1", tmp).unwrap();
        let wt = mgr.windows.get("slot-popup-1").unwrap();
        assert_eq!(wt.tabs, vec![existing, tmp]);
        assert_eq!(wt.active, tmp, "inserted tab activated");
        assert_eq!(mgr.view_owner.get(&tmp).unwrap(), "slot-popup-1");
        assert_invariants(&mgr);
    }

    #[test]
    fn insert_tab_into_vanished_window_is_err_for_rollback() {
        // ★G4 re-validation★: if to_window vanished during phase B, no insert and Err (the caller rolls back).
        let mut mgr = ViewManager::new();
        let src = main_active(&mgr);
        let slot = first_slot_of(&mgr, src);
        mgr.assign_agent(src, slot, "moving".into()).unwrap();
        let (tmp, _content) = mgr.prepare_detached_view(src, slot).unwrap();
        let err = mgr.insert_tab_into("gone", tmp).unwrap_err();
        assert!(matches!(err, LayoutError::WindowNotFound(_)));
        assert!(mgr.view_owner.get(&tmp).is_none());
    }

    #[test]
    fn insert_same_agent_into_window_that_has_it_is_allowed() {
        let mut mgr = ViewManager::new();
        let src = main_active(&mgr);
        let slot = first_slot_of(&mgr, src);
        mgr.assign_agent(src, slot, "shared".into()).unwrap();
        let existing = mgr.create_window("slot-popup-1").unwrap();
        let eslot = first_slot_of(&mgr, existing);
        mgr.assign_agent(existing, eslot, "shared".into()).unwrap();
        let (tmp, _content) = mgr.prepare_detached_view(src, slot).unwrap();
        mgr.insert_tab_into("slot-popup-1", tmp).unwrap();
        assert_eq!(mgr.windows.get("slot-popup-1").unwrap().tabs.len(), 2);
        assert_invariants(&mgr);
    }

    #[test]
    fn attach_view_as_new_window_phase_c() {
        let mut mgr = ViewManager::new();
        let src = main_active(&mgr);
        let slot = first_slot_of(&mgr, src);
        mgr.assign_agent(src, slot, "moving".into()).unwrap();
        let (tmp, _content) = mgr.prepare_detached_view(src, slot).unwrap();
        mgr.attach_view_as_new_window("slot-popup-1", tmp).unwrap();
        let wt = mgr.windows.get("slot-popup-1").unwrap();
        assert_eq!(wt.tabs, vec![tmp]);
        assert_eq!(wt.active, tmp);
        assert_eq!(mgr.view_owner.get(&tmp).unwrap(), "slot-popup-1");
        assert_invariants(&mgr);
    }

    #[test]
    fn drop_detached_view_rolls_back_phase_a() {
        let mut mgr = ViewManager::new();
        let src = main_active(&mgr);
        let slot = first_slot_of(&mgr, src);
        mgr.assign_agent(src, slot, "moving".into()).unwrap();
        let (tmp, _content) = mgr.prepare_detached_view(src, slot).unwrap();
        mgr.drop_detached_view(tmp);
        assert!(!mgr.views.contains_key(&tmp), "temporary View removed");
        assert_invariants(&mgr);
    }

    // ── resolve_spawn_slot (spawn_into pure slot policy — TRD §6 G9) ─────────────────

    #[test]
    fn resolve_none_targets_empty_root_slot() {
        let mut mgr = ViewManager::new();
        let v = mgr.create_tab(MAIN_WINDOW_LABEL, None).unwrap();
        let root = first_slot_of(&mgr, v);
        assert_eq!(
            mgr.resolve_spawn_slot(v, None).unwrap(),
            root,
            "slot=None → the empty root slot"
        );
    }

    #[test]
    fn resolve_none_on_single_occupied_slot_is_no_empty_slot() {
        // No automatic split or overwrite: a single occupied slot reports NoEmptySlot.
        let mut mgr = ViewManager::new();
        let v = mgr.create_tab(MAIN_WINDOW_LABEL, None).unwrap();
        let root = first_slot_of(&mgr, v);
        mgr.assign_agent(v, root, "existing".into()).unwrap();
        assert_eq!(
            mgr.resolve_spawn_slot(v, None),
            Err(SpawnSlotError::NoEmptySlot)
        );
    }

    #[test]
    fn resolve_none_on_split_tab_picks_first_empty_not_leftmost() {
        // Picks the first empty slot in tree order, not the leftmost one.
        let mut mgr = ViewManager::new();
        let v = mgr.create_tab(MAIN_WINDOW_LABEL, None).unwrap();
        let root = first_slot_of(&mgr, v);
        let right = mgr.split_slot(v, root, SplitDir::LeftRight).unwrap();
        mgr.assign_agent(v, root, "occupied".into()).unwrap();
        assert_eq!(
            mgr.resolve_spawn_slot(v, None).unwrap(),
            right,
            "slot=None → skips the occupied left one, the first empty slot (right)"
        );
    }

    #[test]
    fn resolve_none_on_fully_occupied_split_tab_is_no_empty_slot() {
        let mut mgr = ViewManager::new();
        let v = mgr.create_tab(MAIN_WINDOW_LABEL, None).unwrap();
        let root = first_slot_of(&mgr, v);
        let right = mgr.split_slot(v, root, SplitDir::LeftRight).unwrap();
        mgr.assign_agent(v, root, "a".into()).unwrap();
        mgr.assign_agent(v, right, "b".into()).unwrap();
        assert_eq!(
            mgr.resolve_spawn_slot(v, None),
            Err(SpawnSlotError::NoEmptySlot)
        );
    }

    #[test]
    fn resolve_none_after_create_tab_targets_fresh_root() {
        let mut mgr = ViewManager::new();
        let v = mgr.create_tab(MAIN_WINDOW_LABEL, None).unwrap();
        let root = first_slot_of(&mgr, v);
        assert_eq!(mgr.resolve_spawn_slot(v, None).unwrap(), root);
    }

    #[test]
    fn resolve_then_assign_holds_invariants() {
        let mut mgr = ViewManager::new();
        let v = mgr.create_tab(MAIN_WINDOW_LABEL, None).unwrap();
        let target = mgr.resolve_spawn_slot(v, None).unwrap();
        mgr.assign_agent(v, target, "spawned-agent".into()).unwrap();
        assert_eq!(
            mgr.slot_agent(v, target).unwrap().as_deref(),
            Some("spawned-agent")
        );
        assert_invariants(&mgr);
    }

    #[test]
    fn owner_of_rejects_view_from_other_window() {
        // ★The spawn_into tab=Some ownership verification predicate★: claiming a view owned by another window as a tab of the target window (window)
        //   must be refused by the owner_of mismatch (spawn_into verifies with this predicate before placement).
        let mut mgr = ViewManager::new();
        let popup_view = mgr.create_window("slot-popup-1").unwrap();
        assert_eq!(
            mgr.owner_of(popup_view).map(|s| s.as_str()),
            Some("slot-popup-1")
        );
        assert_ne!(
            mgr.owner_of(popup_view).map(|s| s.as_str()),
            Some(MAIN_WINDOW_LABEL),
            "a view owned by another window is not a tab of main (spawn_into refuses before placement)"
        );
    }

    #[test]
    fn resolve_some_empty_returns_that_slot() {
        let mut mgr = ViewManager::new();
        let v = mgr.create_tab(MAIN_WINDOW_LABEL, None).unwrap();
        let root = first_slot_of(&mgr, v);
        let new_slot = mgr.split_slot(v, root, SplitDir::LeftRight).unwrap();
        assert_eq!(
            mgr.resolve_spawn_slot(v, Some(new_slot)).unwrap(),
            new_slot,
            "slot=Some + empty → that slot"
        );
    }

    #[test]
    fn resolve_some_occupied_errors_no_overwrite() {
        let mut mgr = ViewManager::new();
        let v = mgr.create_tab(MAIN_WINDOW_LABEL, None).unwrap();
        let root = first_slot_of(&mgr, v);
        mgr.assign_agent(v, root, "existing".into()).unwrap();
        assert_eq!(
            mgr.resolve_spawn_slot(v, Some(root)),
            Err(SpawnSlotError::SlotOccupied(root))
        );
    }

    #[test]
    fn resolve_some_usage_slot_is_occupied() {
        let mut mgr = ViewManager::new();
        let v = mgr.create_tab(MAIN_WINDOW_LABEL, None).unwrap();
        let root = first_slot_of(&mgr, v);
        mgr.set_slot_content(
            v,
            root,
            SlotContent::Usage {
                show_claude: false,
                show_codex: false,
            },
        )
        .unwrap();
        assert_eq!(
            mgr.resolve_spawn_slot(v, Some(root)),
            Err(SpawnSlotError::SlotOccupied(root))
        );
        assert_eq!(
            mgr.resolve_spawn_slot(v, None),
            Err(SpawnSlotError::NoEmptySlot)
        );
    }

    #[test]
    fn resolve_some_missing_slot_errors() {
        let mut mgr = ViewManager::new();
        let v = mgr.create_tab(MAIN_WINDOW_LABEL, None).unwrap();
        let bogus = Uuid::new_v4();
        assert_eq!(
            mgr.resolve_spawn_slot(v, Some(bogus)),
            Err(SpawnSlotError::SlotNotFound(bogus))
        );
    }

    #[test]
    fn resolve_on_a_missing_view_is_view_not_found() {
        let mgr = ViewManager::new();
        let ghost = Uuid::new_v4();
        assert_eq!(
            mgr.resolve_spawn_slot(ghost, None),
            Err(SpawnSlotError::ViewNotFound(ghost))
        );
    }

    // ── Side table of unknown slot content (TRD S21-storage §6-2 · §12 R6) ────────────────────

    #[test]
    fn an_unknown_content_slot_reads_empty_but_is_not_free() {
        let mut mgr = ViewManager::new();
        let before = mgr.version;
        let (v, s) = foreign_root(&mut mgr);
        assert!(
            mgr.version > before,
            "setting it up in the side table is a change too"
        );
        assert_eq!(mgr.slot_content(v, s).unwrap(), SlotContent::Empty);
        assert_eq!(mgr.slot_is_free(v, s), Ok(false));
        assert_eq!(mgr.unknown_content(v, s), Some(&raw("from_the_future")));

        let right = mgr.split_slot(v, s, SplitDir::LeftRight).unwrap();
        assert_eq!(mgr.slot_is_free(v, right), Ok(true));
        let ghost = Uuid::new_v4();
        assert_eq!(
            mgr.slot_is_free(v, ghost),
            Err(LayoutError::SlotNotFound(ghost))
        );
        assert_eq!(
            mgr.slot_is_free(ghost, s),
            Err(LayoutError::ViewNotFound(ghost))
        );
        assert_invariants(&mgr);
    }

    #[test]
    fn set_unknown_content_writes_empty_over_an_occupied_slot() {
        let mut mgr = ViewManager::new();
        let v = main_active(&mgr);
        let s = first_slot_of(&mgr, v);
        mgr.assign_agent(v, s, "a".into()).unwrap();
        mgr.set_unknown_content(v, s, raw("x")).unwrap();
        assert_eq!(mgr.slot_content(v, s).unwrap(), SlotContent::Empty);
        assert_eq!(mgr.slot_is_free(v, s), Ok(false));
        assert_invariants(&mgr);
    }

    #[test]
    fn set_unknown_content_on_a_missing_view_or_slot_changes_nothing() {
        let mut mgr = ViewManager::new();
        let v = main_active(&mgr);
        let ghost = Uuid::new_v4();
        let before = mgr.version;
        assert_eq!(
            mgr.set_unknown_content(v, ghost, raw("x")),
            Err(LayoutError::SlotNotFound(ghost))
        );
        assert_eq!(
            mgr.set_unknown_content(ghost, ghost, raw("x")),
            Err(LayoutError::ViewNotFound(ghost))
        );
        assert_eq!(mgr.version, before);
        assert!(mgr.unknown_content.is_empty());
    }

    #[test]
    fn auto_spawn_skips_an_unknown_content_slot_for_another_empty_one() {
        let mut mgr = ViewManager::new();
        let (v, root) = foreign_root(&mut mgr);
        let right = mgr.split_slot(v, root, SplitDir::LeftRight).unwrap();
        assert_eq!(
            mgr.unknown_content(v, root),
            Some(&raw("from_the_future")),
            "splitting keeps the original slot on side a — the side table entry remains too"
        );
        assert_eq!(mgr.resolve_spawn_slot(v, None), Ok(right));
        assert_eq!(
            mgr.resolve_spawn_slot(v, Some(root)),
            Err(SpawnSlotError::SlotOccupied(root))
        );
        assert_eq!(mgr.resolve_spawn_slot(v, Some(right)), Ok(right));
    }

    #[test]
    fn auto_spawn_with_no_free_slot_besides_unknown_content_is_no_empty_slot() {
        let mut mgr = ViewManager::new();
        let (v, root) = foreign_root(&mut mgr);
        assert_eq!(
            mgr.resolve_spawn_slot(v, None),
            Err(SpawnSlotError::NoEmptySlot)
        );
        let right = mgr.split_slot(v, root, SplitDir::LeftRight).unwrap();
        mgr.assign_agent(v, right, "a".into()).unwrap();
        assert_eq!(
            mgr.resolve_spawn_slot(v, None),
            Err(SpawnSlotError::NoEmptySlot)
        );
    }

    #[test]
    fn every_content_write_clears_the_unknown_entry() {
        type Write = fn(&mut ViewManager, ViewId, Uuid);
        let writes: [(&str, Write); 6] = [
            ("assign_agent", |m, v, s| {
                m.assign_agent(v, s, "a".into()).unwrap()
            }),
            ("set_slot_content(empty)", |m, v, s| {
                m.set_slot_content(v, s, SlotContent::Empty).unwrap()
            }),
            ("set_slot_content(agent)", |m, v, s| {
                m.set_slot_content(
                    v,
                    s,
                    SlotContent::Agent {
                        agent_id: "a".into(),
                    },
                )
                .unwrap()
            }),
            ("set_slot_content(agent_list)", |m, v, s| {
                m.set_slot_content(v, s, SlotContent::AgentList).unwrap()
            }),
            ("set_slot_content(preset_palette)", |m, v, s| {
                m.set_slot_content(v, s, SlotContent::PresetPalette)
                    .unwrap()
            }),
            ("set_slot_content(usage)", |m, v, s| {
                m.set_slot_content(
                    v,
                    s,
                    SlotContent::Usage {
                        show_claude: true,
                        show_codex: true,
                    },
                )
                .unwrap()
            }),
        ];
        for (name, write) in writes {
            let mut mgr = ViewManager::new();
            let (v, s) = foreign_root(&mut mgr);
            write(&mut mgr, v, s);
            assert_eq!(mgr.unknown_content(v, s), None, "{name}");
            assert!(mgr.snapshot(v).unwrap().foreign_slots.is_empty(), "{name}");
            assert_invariants(&mgr);
        }

        let mut mgr = ViewManager::new();
        let (v, s) = foreign_root(&mut mgr);
        mgr.set_slot_content(v, s, SlotContent::Empty).unwrap();
        assert_eq!(
            mgr.slot_is_free(v, s),
            Ok(true),
            "explicitly writing empty content discards the raw text and makes an empty slot"
        );
    }

    #[test]
    fn closing_the_slot_clears_the_unknown_entry() {
        let mut mgr = ViewManager::new();
        let (v, root) = foreign_root(&mut mgr);
        let right = mgr.split_slot(v, root, SplitDir::LeftRight).unwrap();
        mgr.close_slot(v, root).unwrap();
        assert!(mgr.unknown_content.is_empty());
        assert!(mgr.snapshot(v).unwrap().foreign_slots.is_empty());
        assert_eq!(mgr.resolve_spawn_slot(v, None), Ok(right));
        assert_invariants(&mgr);

        let mut mgr = ViewManager::new();
        let (v, root) = foreign_root(&mut mgr);
        mgr.close_slot(v, root).unwrap();
        assert!(mgr.unknown_content.is_empty());
        let fresh = first_slot_of(&mgr, v);
        assert_eq!(mgr.slot_is_free(v, fresh), Ok(true));
        assert_invariants(&mgr);
    }

    #[test]
    fn closing_a_tab_or_window_drops_its_unknown_entries() {
        let mut mgr = ViewManager::new();
        let first = mgr.create_window("slot-popup-1").unwrap();
        let fs = first_slot_of(&mgr, first);
        mgr.set_unknown_content(first, fs, raw("x")).unwrap();
        let second = mgr.create_tab("slot-popup-1", None).unwrap();
        let ss = first_slot_of(&mgr, second);
        mgr.set_unknown_content(second, ss, raw("y")).unwrap();

        mgr.close_tab("slot-popup-1", second).unwrap();
        assert!(!mgr.unknown_content.contains_key(&second));
        assert_eq!(mgr.unknown_content(first, fs), Some(&raw("x")));
        mgr.close_window("slot-popup-1").unwrap();
        assert!(mgr.unknown_content.is_empty());

        // main's last tab — an empty tab is forced up and the closed tab's entries vanish.
        let (v, _) = foreign_root(&mut mgr);
        mgr.close_tab(MAIN_WINDOW_LABEL, v).unwrap();
        assert!(mgr.unknown_content.is_empty());
        assert_invariants(&mgr);
    }

    #[test]
    fn snapshot_lists_exactly_the_unknown_content_slots_in_tree_order() {
        let mut mgr = ViewManager::new();
        let v = main_active(&mgr);
        let s1 = first_slot_of(&mgr, v);
        let s2 = mgr.split_slot(v, s1, SplitDir::LeftRight).unwrap();
        let s3 = mgr.split_slot(v, s2, SplitDir::TopBottom).unwrap();
        let s4 = mgr.split_slot(v, s3, SplitDir::LeftRight).unwrap();
        assert_eq!(tree::slot_ids(&mgr.views[&v].layout), vec![s1, s2, s3, s4]);
        // Set up in reverse tree order — they must be loaded in tree order, not in the order set up.
        mgr.set_unknown_content(v, s4, raw("b")).unwrap();
        mgr.set_unknown_content(v, s1, raw("a")).unwrap();
        mgr.assign_agent(v, s2, "agent".into()).unwrap();

        assert_eq!(mgr.snapshot(v).unwrap().foreign_slots, vec![s1, s4]);
        let other = mgr.create_tab(MAIN_WINDOW_LABEL, None).unwrap();
        assert!(
            mgr.snapshot(other).unwrap().foreign_slots.is_empty(),
            "does not leak into other tabs"
        );
        assert_invariants(&mgr);
    }

    #[test]
    fn snapshot_after_unrelated_changes_still_lists_the_unknown_slot() {
        let mut mgr = ViewManager::new();
        let (v, root) = foreign_root(&mut mgr);
        let right = mgr.split_slot(v, root, SplitDir::LeftRight).unwrap();
        mgr.assign_agent(v, right, "a".into()).unwrap();
        mgr.set_focused_slot(v, right).unwrap();
        mgr.rename_tab(v, "renamed".into()).unwrap();
        let split = tree::list_splits(&mgr.views[&v].layout)[0].id;
        mgr.set_split_ratio(v, split, 0.3).unwrap();
        let other = mgr.create_tab(MAIN_WINDOW_LABEL, None).unwrap();
        mgr.switch_tab(MAIN_WINDOW_LABEL, v).unwrap();
        mgr.close_tab(MAIN_WINDOW_LABEL, other).unwrap();

        let snap = mgr.snapshot(v).unwrap();
        assert_eq!(snap.foreign_slots, vec![root]);
        assert_eq!(snap.version, mgr.version);
        assert_eq!(
            mgr.unknown_content(v, root),
            Some(&raw("from_the_future")),
            "the raw text as is"
        );
        assert_invariants(&mgr);
    }

    #[test]
    fn moving_an_unknown_content_slot_carries_its_raw_content() {
        let mut mgr = ViewManager::new();
        let (src, slot) = foreign_root(&mut mgr);
        let (tmp, content) = mgr.prepare_detached_view(src, slot).unwrap();
        assert_eq!(content, SlotContent::Empty);
        let tslot = first_slot_of(&mgr, tmp);
        assert_eq!(
            mgr.unknown_content(tmp, tslot),
            Some(&raw("from_the_future"))
        );

        mgr.attach_view_as_new_window("slot-popup-1", tmp).unwrap();
        mgr.close_slot(src, slot).unwrap();
        assert_eq!(mgr.unknown_content(src, slot), None);
        assert_eq!(mgr.snapshot(tmp).unwrap().foreign_slots, vec![tslot]);
        assert_eq!(mgr.slot_is_free(tmp, tslot), Ok(false));
        assert_invariants(&mgr);
    }

    #[test]
    fn rolling_back_a_move_drops_only_the_carried_entry() {
        let mut mgr = ViewManager::new();
        let (src, slot) = foreign_root(&mut mgr);
        let (tmp, _content) = mgr.prepare_detached_view(src, slot).unwrap();
        mgr.drop_detached_view(tmp);
        assert!(!mgr.unknown_content.contains_key(&tmp));
        assert_eq!(
            mgr.unknown_content(src, slot),
            Some(&raw("from_the_future")),
            "the source unchanged"
        );
        assert_invariants(&mgr);
    }

    // ── Window identity · window attributes (TRD S21-storage §6-3) ─────────────────────────────

    fn bounds(x: f64) -> WindowBounds {
        WindowBounds::new(x, 60.0, 1280.0, 800.0).unwrap()
    }

    fn normal_at(x: f64) -> WindowPlacement {
        WindowPlacement {
            minimized: false,
            maximized: false,
            bounds: Some(bounds(x)),
        }
    }

    #[test]
    fn main_is_main_and_each_popout_gets_its_own_uuid() {
        let mut mgr = ViewManager::new();
        assert_eq!(window(&mgr, MAIN_WINDOW_LABEL).window_id, "main");
        mgr.create_window("slot-popup-1").unwrap();
        let src = main_active(&mgr);
        let (tmp, _) = mgr
            .prepare_detached_view(src, first_slot_of(&mgr, src))
            .unwrap();
        mgr.attach_view_as_new_window("slot-popup-2", tmp).unwrap();

        let first = window(&mgr, "slot-popup-1").window_id;
        let second = window(&mgr, "slot-popup-2").window_id;
        for id in [&first, &second] {
            assert_eq!(
                Uuid::parse_str(id).unwrap().to_string(),
                *id,
                "lowercase hyphenated UUID"
            );
        }
        assert_ne!(first, second);

        mgr.create_tab("slot-popup-1", None).unwrap();
        assert_eq!(
            window(&mgr, "slot-popup-1").window_id,
            first,
            "unchanged even when tabs are added"
        );
    }

    #[test]
    fn window_bounds_accept_only_finite_values_and_a_positive_size() {
        assert!(WindowBounds::new(-1920.0, -8.5, 0.5, 1e9).is_some());
        for (w, h) in [(0.0, 1.0), (1.0, 0.0), (-1.0, 1.0), (1.0, -0.5)] {
            assert_eq!(WindowBounds::new(0.0, 0.0, w, h), None, "{w}x{h}");
        }
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert_eq!(WindowBounds::new(bad, 0.0, 1.0, 1.0), None);
            assert_eq!(WindowBounds::new(0.0, bad, 1.0, 1.0), None);
            assert_eq!(WindowBounds::new(0.0, 0.0, bad, 1.0), None);
            assert_eq!(WindowBounds::new(0.0, 0.0, 1.0, bad), None);
        }
        let b = bounds(-1920.0);
        assert_eq!((b.x(), b.y(), b.w(), b.h()), (-1920.0, 60.0, 1280.0, 800.0));
    }

    #[test]
    fn attribute_changes_bump_attrs_rev_and_not_version() {
        let mut mgr = ViewManager::new();
        let version = mgr.version;
        assert_eq!(mgr.attrs_rev(), 0);

        mgr.set_window_theme(MAIN_WINDOW_LABEL, Some(UiTheme::Light))
            .unwrap();
        assert_eq!(mgr.attrs_rev(), 1);
        mgr.observe_window_placement(MAIN_WINDOW_LABEL, normal_at(80.0))
            .unwrap();
        assert_eq!(mgr.attrs_rev(), 2);
        assert_eq!(
            mgr.version, version,
            "window attributes do not bump the layout number"
        );
        assert_eq!(
            mgr.window_attrs(MAIN_WINDOW_LABEL),
            Ok(WindowAttrs {
                theme: Some(UiTheme::Light),
                bounds: Some(bounds(80.0)),
                maximized: false,
            })
        );

        // Same value = no change — the number unchanged too.
        mgr.set_window_theme(MAIN_WINDOW_LABEL, Some(UiTheme::Light))
            .unwrap();
        mgr.observe_window_placement(MAIN_WINDOW_LABEL, normal_at(80.0))
            .unwrap();
        assert_eq!(mgr.attrs_rev(), 2);

        mgr.set_window_theme(MAIN_WINDOW_LABEL, None).unwrap();
        assert_eq!(mgr.attrs_rev(), 3);
        assert_eq!(mgr.window_attrs(MAIN_WINDOW_LABEL).unwrap().theme, None);
        assert_eq!(mgr.version, version);
    }

    #[test]
    fn attribute_setters_on_an_unknown_window_change_nothing() {
        let mut mgr = ViewManager::new();
        for label in ["no-such", "slot-popup-9"] {
            assert_eq!(
                mgr.set_window_theme(label, Some(UiTheme::Dark)),
                Err(LayoutError::WindowNotFound(label.into()))
            );
            assert_eq!(
                mgr.observe_window_placement(label, normal_at(0.0)),
                Err(LayoutError::WindowNotFound(label.into()))
            );
            assert_eq!(
                mgr.window_attrs(label),
                Err(LayoutError::WindowNotFound(label.into()))
            );
        }
        assert_eq!((mgr.attrs_rev(), mgr.version), (0, 0));
    }

    #[test]
    fn placement_records_bounds_only_in_the_normal_state() {
        let mut mgr = ViewManager::new();
        let attrs = |m: &ViewManager| m.window_attrs(MAIN_WINDOW_LABEL).unwrap();
        mgr.observe_window_placement(MAIN_WINDOW_LABEL, normal_at(80.0))
            .unwrap();

        // Maximized — only the marker; the position stays the normal position before maximizing.
        mgr.observe_window_placement(
            MAIN_WINDOW_LABEL,
            WindowPlacement {
                minimized: false,
                maximized: true,
                bounds: Some(bounds(-8.0)),
            },
        )
        .unwrap();
        assert_eq!(
            (attrs(&mgr).bounds, attrs(&mgr).maximized),
            (Some(bounds(80.0)), true)
        );

        // Minimized — changes nothing (even minimizing from maximized, the next restore is maximized).
        let rev = mgr.attrs_rev();
        mgr.observe_window_placement(
            MAIN_WINDOW_LABEL,
            WindowPlacement {
                minimized: true,
                maximized: false,
                bounds: Some(bounds(-32000.0)),
            },
        )
        .unwrap();
        assert_eq!(mgr.attrs_rev(), rev);
        assert_eq!(
            (attrs(&mgr).bounds, attrs(&mgr).maximized),
            (Some(bounds(80.0)), true)
        );

        // Back to normal — lowers the marker and records the new position.
        mgr.observe_window_placement(MAIN_WINDOW_LABEL, normal_at(300.0))
            .unwrap();
        assert_eq!(
            (attrs(&mgr).bounds, attrs(&mgr).maximized),
            (Some(bounds(300.0)), false)
        );

        // Normal but the position could not be read — keeps the previous position.
        mgr.observe_window_placement(
            MAIN_WINDOW_LABEL,
            WindowPlacement {
                minimized: false,
                maximized: false,
                bounds: None,
            },
        )
        .unwrap();
        assert_eq!(attrs(&mgr).bounds, Some(bounds(300.0)));
    }

    fn reading(maximized: bool, x: f64) -> WindowPlacement {
        WindowPlacement {
            minimized: false,
            maximized,
            bounds: Some(bounds(x)),
        }
    }

    #[test]
    fn a_user_maximize_reported_first_as_a_normal_move_keeps_the_normal_rect() {
        let mut mgr = ViewManager::new();
        let state = |m: &ViewManager| {
            let attrs = m.window_attrs(MAIN_WINDOW_LABEL).unwrap();
            (attrs.bounds, attrs.maximized)
        };
        // Normal A(80) → maximize: `Moved` with the maximized rectangle (-8) without the marker → `Resized` with the same rectangle and the marker.
        for seen in [
            reading(false, 80.0),
            reading(false, -8.0),
            reading(true, -8.0),
        ] {
            mgr.observe_window_placement(MAIN_WINDOW_LABEL, seen)
                .unwrap();
        }
        assert_eq!(state(&mgr), (Some(bounds(80.0)), true));
        mgr.observe_window_placement(MAIN_WINDOW_LABEL, reading(true, -8.0))
            .unwrap();
        assert_eq!(state(&mgr), (Some(bounds(80.0)), true));

        // Unmaximize: `Moved` with the marker still set → `Resized` to A without the marker.
        for seen in [reading(true, 80.0), reading(false, 80.0)] {
            mgr.observe_window_placement(MAIN_WINDOW_LABEL, seen)
                .unwrap();
        }
        assert_eq!(state(&mgr), (Some(bounds(80.0)), false));
    }

    #[test]
    fn a_maximize_that_sets_the_flag_first_keeps_the_last_normal_reading() {
        let mut mgr = ViewManager::new();
        // The position the builder made (20) → the position the restore applied (80) → tao `maximize` (the marker is set first, so there is no wrongly recorded read).
        for seen in [
            reading(false, 20.0),
            reading(false, 80.0),
            reading(true, -8.0),
            reading(true, -8.0),
        ] {
            mgr.observe_window_placement(MAIN_WINDOW_LABEL, seen)
                .unwrap();
        }
        let attrs = mgr.window_attrs(MAIN_WINDOW_LABEL).unwrap();
        assert_eq!((attrs.bounds, attrs.maximized), (Some(bounds(80.0)), true));
    }

    #[test]
    fn a_window_whose_first_reading_is_the_maximized_rect_has_no_normal_rect() {
        let mut mgr = ViewManager::new();
        mgr.create_window("slot-popup-1").unwrap();
        for seen in [reading(false, -8.0), reading(true, -8.0)] {
            mgr.observe_window_placement("slot-popup-1", seen).unwrap();
        }
        let attrs = mgr.window_attrs("slot-popup-1").unwrap();
        assert_eq!((attrs.bounds, attrs.maximized), (None, true));
    }

    #[test]
    fn attributes_are_per_window_and_die_with_the_window() {
        let mut mgr = ViewManager::new();
        mgr.create_window("slot-popup-1").unwrap();
        mgr.set_window_theme("slot-popup-1", Some(UiTheme::EInk))
            .unwrap();
        assert_eq!(mgr.window_attrs(MAIN_WINDOW_LABEL).unwrap().theme, None);
        mgr.close_window("slot-popup-1").unwrap();
        // The same label can stand only after the old OS window's `Destroyed` passes.
        assert!(mgr.release_closing("slot-popup-1"));
        mgr.create_window("slot-popup-1").unwrap();
        assert_eq!(
            mgr.window_attrs("slot-popup-1"),
            Ok(WindowAttrs::default()),
            "a new window with the same label does not get the old window's attributes (ADR-0167)"
        );
    }

    // ── from_restored ───────────────────────────────────────────────────────

    fn view_named(name: &str) -> View {
        let layout = LayoutNode::new_empty_slot();
        View {
            id: Uuid::new_v4(),
            name: TabName::Named(name.into()),
            focused_slot_id: Some(tree::first_slot_id(&layout)),
            layout,
        }
    }

    fn restored(tabs: Vec<View>, active: Option<ViewId>) -> RestoredWindow {
        RestoredWindow {
            attrs: WindowAttrs {
                theme: Some(UiTheme::Dark),
                bounds: Some(bounds(1.0)),
                maximized: true,
            },
            tabs,
            active,
            number: None,
        }
    }

    #[test]
    fn from_restored_rebuilds_ownership_and_keeps_order_attrs_and_ids() {
        let (a, b, p) = (view_named("a"), view_named("b"), view_named("p"));
        let popout_id = Uuid::new_v4().to_string();
        let mgr = ViewManager::from_restored(
            restored(vec![a.clone(), b.clone()], Some(b.id)),
            vec![(
                "slot-popup-1".into(),
                popout_id.clone(),
                restored(vec![p.clone()], Some(p.id)),
            )],
        )
        .unwrap();
        assert_invariants(&mgr);
        let main = window(&mgr, MAIN_WINDOW_LABEL);
        assert_eq!((main.tabs, main.active), (vec![a.id, b.id], b.id));
        assert_eq!(main.window_id, "main");
        assert_eq!(main.attrs, restored(vec![], None).attrs);
        assert_eq!((main.canvas, main.metrics), (None, None));
        let popout = window(&mgr, "slot-popup-1");
        assert_eq!((popout.tabs, popout.window_id), (vec![p.id], popout_id));
        assert_eq!(mgr.views[&a.id], a);
        assert_eq!((mgr.version, mgr.attrs_rev()), (0, 0));
    }

    #[test]
    fn from_restored_gives_an_empty_main_one_blank_tab_and_fixes_focus() {
        let mgr = ViewManager::from_restored(restored(vec![], None), vec![]).unwrap();
        assert_invariants(&mgr);
        let main = window(&mgr, MAIN_WINDOW_LABEL);
        assert_eq!(main.tabs.len(), 1);
        assert_eq!(mgr.views[&main.active].name, TabName::Auto);
        assert_eq!(
            main.attrs,
            restored(vec![], None).attrs,
            "the attributes are kept"
        );

        let mut lost = view_named("lost");
        lost.focused_slot_id = Some(Uuid::new_v4());
        let mut none = view_named("none");
        none.focused_slot_id = None;
        let mgr = ViewManager::from_restored(
            restored(vec![lost.clone(), none.clone()], Some(lost.id)),
            vec![],
        )
        .unwrap();
        for v in [&lost, &none] {
            assert_eq!(
                mgr.views[&v.id].focused_slot_id,
                Some(tree::first_slot_id(&v.layout))
            );
        }
    }

    #[test]
    fn from_restored_refuses_what_it_cannot_make_consistent() {
        let a = view_named("a");
        let p = view_named("p");
        let popout = |tabs: Vec<View>, active| {
            vec![(
                "slot-popup-1".to_string(),
                Uuid::new_v4().to_string(),
                restored(tabs, active),
            )]
        };
        let main = || restored(vec![a.clone()], Some(a.id));
        let cases = [
            ("popout without tabs", main(), popout(vec![], None)),
            ("no active tab", restored(vec![a.clone()], None), vec![]),
            (
                "active tab outside the tabs",
                restored(vec![a.clone()], Some(p.id)),
                vec![],
            ),
            (
                "popout active tab outside the tabs",
                main(),
                popout(vec![p.clone()], Some(a.id)),
            ),
            (
                "duplicate View",
                main(),
                popout(vec![a.clone()], Some(a.id)),
            ),
            (
                "duplicate View within one window",
                restored(vec![a.clone(), a.clone()], Some(a.id)),
                vec![],
            ),
            (
                "a popout with the main label",
                main(),
                vec![(
                    MAIN_WINDOW_LABEL.to_string(),
                    Uuid::new_v4().to_string(),
                    restored(vec![p.clone()], Some(p.id)),
                )],
            ),
            (
                "duplicate popout label",
                main(),
                vec![
                    (
                        "x".to_string(),
                        "1".to_string(),
                        restored(vec![p.clone()], Some(p.id)),
                    ),
                    {
                        let q = view_named("q");
                        let q_id = q.id;
                        (
                            "x".to_string(),
                            "2".to_string(),
                            restored(vec![q], Some(q_id)),
                        )
                    },
                ],
            ),
        ];
        for (name, main, popouts) in cases {
            assert!(ViewManager::from_restored(main, popouts).is_err(), "{name}");
        }
    }

    // ── adopt_restored (TRD S21-storage §6-7 ③) ─────────────────────────────

    fn fingerprint(mgr: &ViewManager) -> (u64, u64, Vec<WindowLabel>, Vec<ViewId>) {
        let mut labels = mgr.list_windows();
        labels.sort();
        let mut views: Vec<ViewId> = mgr.views.keys().copied().collect();
        views.sort();
        (mgr.version, mgr.attrs_rev(), labels, views)
    }

    fn restored_with_popout(label: &str) -> (ViewManager, View, View, View) {
        let (a, b, p) = (view_named("a"), view_named("b"), view_named("p"));
        let mgr = ViewManager::from_restored(
            restored(vec![a.clone(), b.clone()], Some(b.id)),
            vec![(
                label.into(),
                "4b7f6a7e-0d5e-4f5a-9b0e-6a8f0c2d1e3f".into(),
                restored(vec![p.clone()], Some(p.id)),
            )],
        )
        .unwrap();
        (mgr, a, b, p)
    }

    #[test]
    fn adopt_restored_swaps_the_windows_bumps_both_counters_once_and_moves_the_side_table() {
        let mut mgr = ViewManager::new();
        let old_main = main_active(&mgr);
        let old_popout = mgr.create_window("slot-popup-1").unwrap();
        let old_slot = first_slot_of(&mgr, old_popout);
        mgr.set_unknown_content(old_popout, old_slot, raw("old"))
            .unwrap();
        mgr.set_window_canvas(MAIN_WINDOW_LABEL, 800, 600).unwrap();
        mgr.set_window_theme(MAIN_WINDOW_LABEL, Some(UiTheme::Light))
            .unwrap();
        let (version, attrs_rev) = (mgr.version, mgr.attrs_rev());

        let (mut fresh, a, b, p) = restored_with_popout("slot-popup-2");
        let p_slot = first_slot_of(&fresh, p.id);
        fresh
            .set_unknown_content(p.id, p_slot, raw("future"))
            .unwrap();

        let removed = mgr.adopt_restored(fresh).unwrap();

        assert_eq!(removed, ["slot-popup-1"]);
        assert_invariants(&mgr);
        assert_eq!(
            (mgr.version, mgr.attrs_rev()),
            (version + 1, attrs_rev + 1),
            "one up from the current number — no rewind"
        );
        let main = window(&mgr, MAIN_WINDOW_LABEL);
        assert_eq!((main.tabs, main.active), (vec![a.id, b.id], b.id));
        assert_eq!(main.attrs, restored(vec![], None).attrs);
        assert_eq!(main.window_id, "main");
        assert_eq!(
            main.canvas,
            Some(CanvasPx { w: 800, h: 600 }),
            "the measurements remain"
        );
        let popout = window(&mgr, "slot-popup-2");
        assert_eq!(
            (popout.tabs, popout.window_id.as_str()),
            (vec![p.id], "4b7f6a7e-0d5e-4f5a-9b0e-6a8f0c2d1e3f")
        );
        assert!(!mgr.views.contains_key(&old_main));
        assert!(!mgr.views.contains_key(&old_popout));
        assert_eq!(
            mgr.unknown_content(old_popout, old_slot),
            None,
            "the old side table is reaped"
        );
        assert_eq!(
            mgr.unknown_content(p.id, p_slot),
            Some(&raw("future")),
            "the new side table comes with the View"
        );
        assert_eq!(mgr.snapshot(p.id).unwrap().foreign_slots, [p_slot]);
    }

    #[test]
    fn adopt_restored_refuses_a_collision_or_a_missing_main_and_changes_nothing() {
        let mut mgr = ViewManager::new();
        mgr.create_window("slot-popup-1").unwrap();
        let src = main_active(&mgr);
        let src_slot = first_slot_of(&mgr, src);
        let (tmp, _) = mgr.prepare_detached_view(src, src_slot).unwrap();
        let before = fingerprint(&mgr);

        let (live_label, ..) = restored_with_popout("slot-popup-1");
        assert_eq!(
            mgr.adopt_restored(live_label).unwrap_err(),
            LayoutError::WindowNotFound("slot-popup-1".into()),
            "the label of a window that is up"
        );

        let (mut no_main, ..) = restored_with_popout("slot-popup-2");
        no_main.windows.remove(MAIN_WINDOW_LABEL);
        assert_eq!(
            mgr.adopt_restored(no_main).unwrap_err(),
            LayoutError::WindowNotFound(MAIN_WINDOW_LABEL.into())
        );

        let mut clash = view_named("clash");
        clash.id = tmp;
        let clash = ViewManager::from_restored(restored(vec![clash], Some(tmp)), vec![]).unwrap();
        assert_eq!(
            mgr.adopt_restored(clash).unwrap_err(),
            LayoutError::ViewNotFound(tmp),
            "overlaps a remaining View (an ownerless temporary View)"
        );

        assert_eq!(fingerprint(&mgr), before);
    }

    #[test]
    fn adopt_restored_leaves_an_ownerless_detached_view_to_its_move() {
        let mut mgr = ViewManager::new();
        let src = main_active(&mgr);
        let src_slot = first_slot_of(&mgr, src);
        let (tmp, _) = mgr.prepare_detached_view(src, src_slot).unwrap();

        let (fresh, ..) = restored_with_popout("slot-popup-2");
        mgr.adopt_restored(fresh).unwrap();

        assert!(mgr.views.contains_key(&tmp));
        mgr.attach_view_as_new_window("slot-popup-3", tmp).unwrap();
        assert_invariants(&mgr);
    }

    #[test]
    fn a_restore_that_brings_back_the_same_view_id_keeps_the_restored_view() {
        let mut mgr = ViewManager::new();
        let v = main_active(&mgr);
        mgr.split_slot(v, first_slot_of(&mgr, v), SplitDir::LeftRight)
            .unwrap();
        let restored_layout = LayoutNode::new_empty_slot();
        let same_id = View {
            id: v,
            name: TabName::Auto,
            layout: restored_layout.clone(),
            focused_slot_id: None,
        };
        let restored = ViewManager::from_restored(
            RestoredWindow {
                attrs: WindowAttrs::default(),
                tabs: vec![same_id],
                active: Some(v),
                number: None,
            },
            Vec::new(),
        )
        .unwrap();
        mgr.adopt_restored(restored).unwrap();
        assert_eq!(mgr.views[&v].layout, restored_layout);
        assert_invariants(&mgr);
    }

    // ── Window numbers (TRD S26-window-chrome §4-1 · §10-1) ─────────────────────────

    fn number_of(mgr: &ViewManager, label: &str) -> u32 {
        mgr.windows[label].number
    }

    #[test]
    fn windows_are_numbered_from_1_and_a_closing_number_waits_for_destroyed() {
        let mut mgr = ViewManager::new();
        assert_eq!(number_of(&mgr, MAIN_WINDOW_LABEL), 1);
        mgr.create_window("p-1").unwrap();
        mgr.create_window("p-2").unwrap();
        mgr.create_window("p-3").unwrap();
        assert_eq!(["p-1", "p-2", "p-3"].map(|l| number_of(&mgr, l)), [2, 3, 4]);
        assert_invariants(&mgr);

        mgr.close_window("p-2").unwrap();
        assert_eq!(
            mgr.closing_numbers(),
            [3],
            "the OS window remains and holds the number"
        );
        mgr.create_window("p-4").unwrap();
        assert_eq!(
            number_of(&mgr, "p-4"),
            5,
            "skips the number of the closing window"
        );
        assert_invariants(&mgr);

        assert!(mgr.release_closing("p-2"));
        assert!(mgr.closing_numbers().is_empty());
        mgr.create_window("p-5").unwrap();
        assert_eq!(
            number_of(&mgr, "p-5"),
            3,
            "the freed number goes to the next new window"
        );
        assert_invariants(&mgr);
    }

    #[test]
    fn two_closes_before_either_destroyed_hold_both_numbers() {
        let mut mgr = ViewManager::new();
        mgr.create_window("p-1").unwrap();
        mgr.create_window("p-2").unwrap();
        mgr.close_window("p-1").unwrap();
        mgr.close_window("p-2").unwrap();
        assert_eq!(mgr.closing_numbers(), [2, 3]);
        mgr.create_window("p-3").unwrap();
        assert_eq!(number_of(&mgr, "p-3"), 4);
        assert_invariants(&mgr);

        assert!(mgr.release_closing("p-2"));
        mgr.create_window("p-4").unwrap();
        assert_eq!(number_of(&mgr, "p-4"), 3);
        assert!(mgr.release_closing("p-1"));
        mgr.create_window("p-5").unwrap();
        assert_eq!(number_of(&mgr, "p-5"), 2);
        assert_invariants(&mgr);
    }

    #[test]
    fn release_closing_of_a_label_that_is_not_closing_changes_nothing() {
        let mut mgr = ViewManager::new();
        mgr.create_window("p-1").unwrap();
        mgr.create_window("p-2").unwrap();
        mgr.close_window("p-2").unwrap();
        let version = mgr.version;

        assert!(
            !mgr.release_closing("p-9"),
            "a prewarm window — never in the model"
        );
        assert!(!mgr.release_closing("p-1"), "a live window");
        assert!(!mgr.release_closing(MAIN_WINDOW_LABEL));
        assert!(mgr.release_closing("p-2"));
        assert!(!mgr.release_closing("p-2"), "the second time is a no-op");

        assert_eq!(number_of(&mgr, "p-1"), 2);
        assert_eq!(
            mgr.version, version,
            "freeing a number does not bump version"
        );
        assert_invariants(&mgr);
    }

    #[test]
    fn drop_window_gone_frees_the_number_at_once() {
        let mut mgr = ViewManager::new();
        let p = mgr.create_window("p-1").unwrap();
        mgr.create_window("p-2").unwrap();
        let version = mgr.version;

        assert_eq!(mgr.drop_window_gone("p-1").unwrap(), [p]);
        assert!(!mgr.windows.contains_key("p-1"));
        assert!(mgr.closing_numbers().is_empty());
        assert_eq!(mgr.version, version + 1);
        mgr.create_window("p-3").unwrap();
        assert_eq!(number_of(&mgr, "p-3"), 2);
        assert_invariants(&mgr);

        assert_eq!(
            mgr.drop_window_gone(MAIN_WINDOW_LABEL),
            Err(LayoutError::MainNotClosable)
        );
        assert_eq!(
            mgr.drop_window_gone("p-1"),
            Err(LayoutError::WindowNotFound("p-1".into()))
        );
    }

    #[test]
    fn a_popout_emptied_by_its_last_tab_or_a_dock_holds_its_number() {
        let mut mgr = ViewManager::new();
        let p = mgr.create_window("p-1").unwrap();
        assert_eq!(
            mgr.close_tab("p-1", p).unwrap(),
            CloseTabOutcome::WindowClosed
        );
        assert_eq!(mgr.closing_numbers(), [2]);
        assert_invariants(&mgr);

        let q = mgr.create_window("p-2").unwrap();
        assert_eq!(number_of(&mgr, "p-2"), 3);
        let out = docked(
            &mut mgr,
            q,
            tab_bar(MAIN_WINDOW_LABEL, None),
            FocusPolicy::Keep,
        );
        assert_eq!(out.emptied.as_deref(), Some("p-2"));
        assert_eq!(mgr.closing_numbers(), [2, 3]);
        assert_invariants(&mgr);
    }

    #[test]
    fn every_new_window_path_takes_the_smallest_free_number() {
        let mut mgr = ViewManager::new();
        mgr.create_window("p-1").unwrap();
        mgr.create_window("p-2").unwrap();
        mgr.close_window("p-1").unwrap();
        mgr.release_closing("p-1");

        let src = main_active(&mgr);
        let slot = first_slot_of(&mgr, src);
        let (tmp, _) = mgr.prepare_detached_view(src, slot).unwrap();
        mgr.attach_view_as_new_window("p-3", tmp).unwrap();
        assert_eq!(number_of(&mgr, "p-3"), 2, "moving a slot to a new window");

        let extra = mgr.create_tab(MAIN_WINDOW_LABEL, None).unwrap();
        docked(&mut mgr, extra, new_window("p-4"), FocusPolicy::Keep);
        assert_eq!(number_of(&mgr, "p-4"), 4, "docking new window");
        assert_invariants(&mgr);
    }

    #[test]
    fn main_keeps_1_through_refills_and_a_refused_close() {
        let mut mgr = ViewManager::new();
        let only = main_active(&mgr);
        mgr.close_tab(MAIN_WINDOW_LABEL, only).unwrap();
        assert_eq!(
            mgr.close_window(MAIN_WINDOW_LABEL),
            Err(LayoutError::MainNotClosable)
        );
        mgr.create_window("p-1").unwrap();
        let last = main_active(&mgr);
        let out = docked(&mut mgr, last, tab_bar("p-1", None), FocusPolicy::Keep);
        assert!(out.main_refilled.is_some());
        assert_eq!(number_of(&mgr, MAIN_WINDOW_LABEL), 1);
        assert!(mgr.closing_numbers().is_empty());
        assert_invariants(&mgr);
    }

    #[test]
    fn from_restored_numbers_popouts_by_the_saved_numbers_in_order() {
        let popout = |label: &str, number: Option<u32>| {
            let v = view_named(label);
            let mut w = restored(vec![v.clone()], Some(v.id));
            w.number = number;
            (label.to_string(), Uuid::new_v4().to_string(), w)
        };
        let mut main = restored(vec![], None);
        main.number = Some(7);
        let mgr = ViewManager::from_restored(
            main,
            vec![
                popout("p-1", Some(3)),
                popout("p-2", Some(3)),
                popout("p-3", None),
                popout("p-4", Some(1)),
            ],
        )
        .unwrap();
        assert_eq!(
            number_of(&mgr, MAIN_WINDOW_LABEL),
            1,
            "main's saved value is not looked at"
        );
        assert_eq!(
            ["p-1", "p-2", "p-3", "p-4"].map(|l| number_of(&mgr, l)),
            [3, 2, 4, 5],
            "earlier windows keep their numbers and the rest get free numbers in saved order"
        );
        assert!(mgr.closing_numbers().is_empty());
        assert_invariants(&mgr);
    }

    #[test]
    fn adopt_restored_frees_every_old_number_and_keeps_the_copy_numbers() {
        let mut mgr = ViewManager::new();
        mgr.create_window("slot-popup-1").unwrap();
        mgr.create_window("slot-popup-2").unwrap();
        mgr.create_window("slot-popup-3").unwrap();
        mgr.close_window("slot-popup-3").unwrap();
        assert_eq!(mgr.closing_numbers(), [4]);

        let copy_popout = |label: &str, number: Option<u32>| {
            let v = view_named(label);
            let mut w = restored(vec![v.clone()], Some(v.id));
            w.number = number;
            (label.to_string(), Uuid::new_v4().to_string(), w)
        };
        let m = view_named("m");
        let copy = ViewManager::from_restored(
            restored(vec![m.clone()], Some(m.id)),
            vec![
                copy_popout("slot-popup-10", Some(4)),
                copy_popout("slot-popup-11", None),
            ],
        )
        .unwrap();

        let removed = mgr.adopt_restored(copy).unwrap();
        assert_eq!(removed, ["slot-popup-1", "slot-popup-2"]);
        assert!(
            mgr.closing_numbers().is_empty(),
            "closing window numbers are freed too"
        );
        assert_eq!(number_of(&mgr, "slot-popup-10"), 4);
        assert_eq!(number_of(&mgr, "slot-popup-11"), 2);
        assert_invariants(&mgr);

        // A late `Destroyed` of the old window does not touch the copy window's number.
        for old in ["slot-popup-1", "slot-popup-2", "slot-popup-3"] {
            assert!(!mgr.release_closing(old));
        }
        assert_eq!(number_of(&mgr, "slot-popup-10"), 4);
        mgr.create_window("slot-popup-12").unwrap();
        assert_eq!(number_of(&mgr, "slot-popup-12"), 3);
        assert_invariants(&mgr);
    }

    // The `Destroyed` branch in `lib.rs` without the state to call the cleanup — only this one runs (TRD S26-window-chrome §4-1).
    #[test]
    fn release_window_number_frees_a_closing_window_or_one_still_in_the_model() {
        let state = crate::layout::LayoutState::new();
        {
            let mut mgr = state.0.lock().unwrap();
            mgr.create_window("p-1").unwrap();
            mgr.create_window("p-2").unwrap();
            mgr.close_window("p-1").unwrap();
        }

        crate::layout::release_window_number(&state, "p-1");
        crate::layout::release_window_number(&state, "p-2");
        crate::layout::release_window_number(&state, "p-9");
        crate::layout::release_window_number(&state, MAIN_WINDOW_LABEL);

        let mut mgr = state.0.lock().unwrap();
        assert!(mgr.closing_numbers().is_empty());
        assert!(
            !mgr.windows.contains_key("p-2"),
            "a window that remained in the model is removed with its entry"
        );
        assert!(mgr.windows.contains_key(MAIN_WINDOW_LABEL));
        assert_invariants(&mgr);
        mgr.create_window("p-3").unwrap();
        mgr.create_window("p-4").unwrap();
        assert_eq!(["p-3", "p-4"].map(|l| number_of(&mgr, l)), [2, 3]);
    }

    #[test]
    fn list_tabs_carries_the_window_id_and_number() {
        let mut mgr = ViewManager::new();
        mgr.create_window("p-1").unwrap();
        let main = mgr.list_tabs(MAIN_WINDOW_LABEL).unwrap();
        assert_eq!((main.window_id.as_str(), main.number), ("main", 1));
        let popout = mgr.list_tabs("p-1").unwrap();
        assert_eq!(popout.number, 2);
        assert_eq!(popout.window_id, mgr.windows["p-1"].window_id);
    }

    // ── Measurement reports · slot_px (ADR-0227) ───────────────────────────────────────

    fn metrics(t: f64, r: f64, b: f64, l: f64, min_pane_px: u32) -> UiMetrics {
        UiMetrics {
            frame_insets: geometry::Insets { t, r, b, l },
            min_pane_px,
        }
    }

    fn one_px_border() -> UiMetrics {
        metrics(1.0, 1.0, 1.0, 1.0, 30)
    }

    fn window(mgr: &ViewManager, label: &str) -> WindowTabs {
        mgr.windows.get(label).expect("window exists").clone()
    }

    #[test]
    fn canvas_is_stored_per_window() {
        let mut mgr = ViewManager::new();
        mgr.create_window("popup-1").unwrap();
        mgr.set_window_canvas(MAIN_WINDOW_LABEL, 1200, 800).unwrap();
        mgr.set_window_canvas("popup-1", 640, 480).unwrap();

        assert_eq!(
            window(&mgr, MAIN_WINDOW_LABEL).canvas,
            Some(CanvasPx { w: 1200, h: 800 })
        );
        assert_eq!(
            window(&mgr, "popup-1").canvas,
            Some(CanvasPx { w: 640, h: 480 })
        );
    }

    #[test]
    fn hidden_tab_uses_the_window_canvas_for_slot_px() {
        let mut mgr = ViewManager::new();
        let hidden = main_active(&mgr);
        let shown = mgr.create_tab(MAIN_WINDOW_LABEL, None).unwrap();
        assert_ne!(
            main_active(&mgr),
            hidden,
            "premise: the first tab is hidden"
        );
        mgr.set_window_canvas(MAIN_WINDOW_LABEL, 1200, 800).unwrap();
        mgr.set_ui_metrics(MAIN_WINDOW_LABEL, one_px_border())
            .unwrap();

        let hidden_px = mgr
            .slot_px(hidden, first_slot_of(&mgr, hidden))
            .unwrap()
            .expect("hidden tabs are computed with the window canvas too");
        let shown_px = mgr
            .slot_px(shown, first_slot_of(&mgr, shown))
            .unwrap()
            .expect("active tab");
        let full = PxRect {
            x0: 0,
            y0: 0,
            x1: 1200,
            y1: 800,
        };
        assert_eq!(hidden_px.frame, full);
        assert_eq!(shown_px.frame, full);
    }

    #[test]
    fn a_new_window_starts_without_canvas_or_metrics() {
        let mut mgr = ViewManager::new();
        mgr.set_window_canvas(MAIN_WINDOW_LABEL, 1200, 800).unwrap();
        mgr.set_ui_metrics(MAIN_WINDOW_LABEL, one_px_border())
            .unwrap();

        let fresh = mgr.create_window("popup-1").unwrap();
        let src = main_active(&mgr);
        let (detached, _) = mgr
            .prepare_detached_view(src, first_slot_of(&mgr, src))
            .unwrap();
        mgr.attach_view_as_new_window("popup-2", detached).unwrap();

        for (label, view) in [("popup-1", fresh), ("popup-2", detached)] {
            let wt = window(&mgr, label);
            assert_eq!(
                wt.canvas, None,
                "{label}: no canvas until that webview reports"
            );
            assert_eq!(wt.metrics, None, "{label}: no metrics either");
            assert_eq!(
                mgr.slot_px(view, first_slot_of(&mgr, view)),
                Ok(None),
                "{label}: does not borrow main's values"
            );
        }
    }

    #[test]
    fn canvas_and_metrics_vanish_with_the_window() {
        let mut mgr = ViewManager::new();
        mgr.create_window("popup-1").unwrap();
        let tab = mgr.create_window("popup-2").unwrap();
        for label in ["popup-1", "popup-2"] {
            mgr.set_window_canvas(label, 640, 480).unwrap();
            mgr.set_ui_metrics(label, one_px_border()).unwrap();
        }

        mgr.close_window("popup-1").unwrap();
        assert_eq!(
            mgr.close_tab("popup-2", tab).unwrap(),
            CloseTabOutcome::WindowClosed
        );

        for label in ["popup-1", "popup-2"] {
            assert!(!mgr.windows.contains_key(label));
            assert_eq!(
                mgr.set_window_canvas(label, 640, 480),
                Err(LayoutError::WindowNotFound(label.to_string()))
            );
        }
    }

    #[test]
    fn reports_bump_neither_version_nor_attrs_rev() {
        let mut mgr = ViewManager::new();
        let before = (mgr.version, mgr.attrs_rev());
        mgr.set_window_canvas(MAIN_WINDOW_LABEL, 1200, 800).unwrap();
        mgr.set_window_canvas(MAIN_WINDOW_LABEL, 0, 800).unwrap();
        mgr.set_ui_metrics(MAIN_WINDOW_LABEL, one_px_border())
            .unwrap();
        let _ = mgr.set_ui_metrics(MAIN_WINDOW_LABEL, metrics(-1.0, 1.0, 1.0, 1.0, 30));
        let _ = mgr.set_window_canvas("no-such", 1, 1);
        assert_eq!((mgr.version, mgr.attrs_rev()), before);
    }

    #[test]
    fn a_zero_size_canvas_report_is_ignored_and_keeps_the_previous_value() {
        let mut mgr = ViewManager::new();
        mgr.set_window_canvas(MAIN_WINDOW_LABEL, 1200, 800).unwrap();
        for (w, h) in [(0, 800), (1200, 0), (0, 0)] {
            assert_eq!(mgr.set_window_canvas(MAIN_WINDOW_LABEL, w, h), Ok(()));
            assert_eq!(
                window(&mgr, MAIN_WINDOW_LABEL).canvas,
                Some(CanvasPx { w: 1200, h: 800 }),
                "the {w}x{h} report overwrote the previous value"
            );
        }
    }

    #[test]
    fn a_zero_size_first_report_leaves_the_canvas_unknown() {
        let mut mgr = ViewManager::new();
        mgr.set_window_canvas(MAIN_WINDOW_LABEL, 0, 0).unwrap();
        assert_eq!(window(&mgr, MAIN_WINDOW_LABEL).canvas, None);
    }

    #[test]
    fn out_of_range_metrics_are_rejected_and_keep_the_previous_value() {
        let good = one_px_border();
        let bad = [
            ("t < 0", metrics(-0.5, 1.0, 1.0, 1.0, 30)),
            ("r > 64", metrics(1.0, 64.5, 1.0, 1.0, 30)),
            ("b NaN", metrics(1.0, 1.0, f64::NAN, 1.0, 30)),
            ("l +inf", metrics(1.0, 1.0, 1.0, f64::INFINITY, 30)),
            ("t -inf", metrics(f64::NEG_INFINITY, 1.0, 1.0, 1.0, 30)),
            ("min_pane_px 0", metrics(1.0, 1.0, 1.0, 1.0, 0)),
            ("min_pane_px 1001", metrics(1.0, 1.0, 1.0, 1.0, 1001)),
        ];
        for (why, m) in bad {
            let mut mgr = ViewManager::new();
            mgr.set_ui_metrics(MAIN_WINDOW_LABEL, good).unwrap();
            assert!(
                matches!(
                    mgr.set_ui_metrics(MAIN_WINDOW_LABEL, m),
                    Err(LayoutError::InvalidMetrics(_))
                ),
                "{why}: must be refused"
            );
            assert_eq!(
                window(&mgr, MAIN_WINDOW_LABEL).metrics,
                Some(good),
                "{why}: must keep the previous value"
            );
        }
    }

    #[test]
    fn exact_metric_bounds_are_accepted() {
        let mut mgr = ViewManager::new();
        for m in [
            metrics(0.0, 0.0, 0.0, 0.0, 1),
            metrics(64.0, 64.0, 64.0, 64.0, 1000),
            metrics(0.8, 1.25, 0.0, 64.0, 30),
        ] {
            assert_eq!(mgr.set_ui_metrics(MAIN_WINDOW_LABEL, m), Ok(()));
            assert_eq!(window(&mgr, MAIN_WINDOW_LABEL).metrics, Some(m));
        }
    }

    #[test]
    fn reports_to_an_unknown_window_are_window_not_found() {
        let mut mgr = ViewManager::new();
        assert_eq!(
            mgr.set_window_canvas("no-such", 100, 100),
            Err(LayoutError::WindowNotFound("no-such".into()))
        );
        assert_eq!(
            mgr.set_ui_metrics("no-such", one_px_border()),
            Err(LayoutError::WindowNotFound("no-such".into()))
        );
    }

    #[test]
    fn slot_px_rounds_frame_edges_from_the_canvas_and_subtracts_insets() {
        let mut mgr = ViewManager::new();
        let v = main_active(&mgr);
        let left = first_slot_of(&mgr, v);
        let right = mgr.split_slot(v, left, SplitDir::LeftRight).unwrap();
        // Width 1001 × ratio 0.5 = 500.5 → a tie rounds away from 0, so 501.
        mgr.set_window_canvas(MAIN_WINDOW_LABEL, 1001, 600).unwrap();
        mgr.set_ui_metrics(MAIN_WINDOW_LABEL, metrics(1.0, 2.0, 3.0, 4.0, 30))
            .unwrap();

        let l = mgr.slot_px(v, left).unwrap().unwrap();
        let r = mgr.slot_px(v, right).unwrap().unwrap();
        assert_eq!(
            l.frame,
            PxRect {
                x0: 0,
                y0: 0,
                x1: 501,
                y1: 600
            }
        );
        assert_eq!(
            r.frame,
            PxRect {
                x0: 501,
                y0: 0,
                x1: 1001,
                y1: 600
            }
        );
        assert_eq!(
            l.content,
            RectF64 {
                x0: 4.0,
                y0: 1.0,
                x1: 499.0,
                y1: 597.0
            }
        );
        assert_eq!(
            r.content,
            RectF64 {
                x0: 505.0,
                y0: 1.0,
                x1: 999.0,
                y1: 597.0
            }
        );
    }

    #[test]
    fn slot_px_content_never_goes_below_zero() {
        let mut mgr = ViewManager::new();
        let v = main_active(&mgr);
        let s = first_slot_of(&mgr, v);
        // The frame (3×2) is narrower than the padding sums (left+right 4 · top+bottom 4).
        mgr.set_window_canvas(MAIN_WINDOW_LABEL, 3, 2).unwrap();
        mgr.set_ui_metrics(MAIN_WINDOW_LABEL, metrics(2.0, 2.0, 2.0, 2.0, 30))
            .unwrap();

        let px = mgr.slot_px(v, s).unwrap().unwrap();
        assert!(px.content.x1 - px.content.x0 >= 0.0, "{:?}", px.content);
        assert!(px.content.y1 - px.content.y0 >= 0.0, "{:?}", px.content);
        assert_eq!(px.content.x1, px.content.x0);
        assert_eq!(px.content.y1, px.content.y0);
    }

    #[test]
    fn slot_px_is_none_until_both_canvas_and_metrics_are_known() {
        let mut mgr = ViewManager::new();
        let v = main_active(&mgr);
        let s = first_slot_of(&mgr, v);
        assert_eq!(mgr.slot_px(v, s), Ok(None), "neither");

        mgr.set_window_canvas(MAIN_WINDOW_LABEL, 1200, 800).unwrap();
        assert_eq!(mgr.slot_px(v, s), Ok(None), "no metrics");
        assert_eq!(mgr.px_context(v), Ok(None));

        let mut only_metrics = ViewManager::new();
        let v2 = main_active(&only_metrics);
        let s2 = first_slot_of(&only_metrics, v2);
        only_metrics
            .set_ui_metrics(MAIN_WINDOW_LABEL, one_px_border())
            .unwrap();
        assert_eq!(only_metrics.slot_px(v2, s2), Ok(None), "no canvas");

        mgr.set_ui_metrics(MAIN_WINDOW_LABEL, one_px_border())
            .unwrap();
        assert!(mgr.slot_px(v, s).unwrap().is_some(), "both present");
        assert_eq!(
            mgr.px_context(v),
            Ok(Some((CanvasPx { w: 1200, h: 800 }, one_px_border())))
        );
    }

    #[test]
    fn slot_px_unknown_view_or_slot_is_err_even_without_canvas() {
        let mut mgr = ViewManager::new();
        let v = main_active(&mgr);
        let other = mgr.create_tab(MAIN_WINDOW_LABEL, None).unwrap();
        let bogus = Uuid::new_v4();
        assert_eq!(
            mgr.slot_px(bogus, first_slot_of(&mgr, v)),
            Err(LayoutError::ViewNotFound(bogus))
        );
        assert_eq!(mgr.slot_px(v, bogus), Err(LayoutError::SlotNotFound(bogus)));
        // An area of another tab is not an area of this view.
        let foreign = first_slot_of(&mgr, other);
        assert_eq!(
            mgr.slot_px(v, foreign),
            Err(LayoutError::SlotNotFound(foreign))
        );
        assert_eq!(mgr.px_context(bogus), Err(LayoutError::ViewNotFound(bogus)));
    }

    #[test]
    fn px_context_of_an_ownerless_detached_view_is_none() {
        let mut mgr = ViewManager::new();
        mgr.set_window_canvas(MAIN_WINDOW_LABEL, 1200, 800).unwrap();
        mgr.set_ui_metrics(MAIN_WINDOW_LABEL, one_px_border())
            .unwrap();
        let src = main_active(&mgr);
        let (tmp, _) = mgr
            .prepare_detached_view(src, first_slot_of(&mgr, src))
            .unwrap();
        assert_eq!(mgr.px_context(tmp), Ok(None));
    }

    // ── Ratio writes · split representability (ADR-0227) ──────────────────────────────

    // Preorder — the root first; for a chain dug on the same side, the deepest is last.
    fn split_ids(mgr: &ViewManager, view: ViewId) -> Vec<Uuid> {
        tree::list_splits(&mgr.views[&view].layout)
            .into_iter()
            .map(|s| s.id)
            .collect()
    }

    fn ratio_in(mgr: &ViewManager, view: ViewId, split: Uuid) -> f64 {
        tree::split_ratio(&mgr.views[&view].layout, split).expect("split must exist")
    }

    fn slot_rect(mgr: &ViewManager, view: ViewId, slot: Uuid) -> SlotRect {
        *geometry::compute(&mgr.views[&view].layout)
            .slots
            .iter()
            .find(|r| r.slot_id == slot)
            .expect("area must exist")
    }

    fn assert_every_leaf_has_area(mgr: &ViewManager, view: ViewId) {
        for r in geometry::compute(&mgr.views[&view].layout).slots {
            assert!(has_area(&r), "area-0 area: {r:?}");
        }
    }

    // A world where the first area of main's active tab is split once along `dir` — returns = (manager, tab, that split).
    fn one_split(dir: SplitDir) -> (ViewManager, ViewId, Uuid) {
        let mut mgr = ViewManager::new();
        let v = main_active(&mgr);
        let s = first_slot_of(&mgr, v);
        mgr.split_slot(v, s, dir).unwrap();
        let split = split_ids(&mgr, v)[0];
        (mgr, v, split)
    }

    fn applied(ratio: f64) -> SplitRatioResult {
        SplitRatioResult {
            ratio,
            outcome: SplitRatioOutcome::Applied,
        }
    }

    // An outcome that did not change the value — also checks the tree · version are unchanged.
    fn assert_untouched(
        mgr: &mut ViewManager,
        view: ViewId,
        split: Uuid,
        ratio: f64,
        expect: SplitRatioResult,
    ) {
        let before = mgr.views[&view].clone();
        let ver = mgr.version;
        assert_eq!(mgr.set_split_ratio(view, split, ratio), Ok(expect));
        assert_eq!(mgr.views[&view], before, "must be unchanged");
        assert_eq!(mgr.version, ver, "does not bump version");
    }

    #[test]
    fn set_split_ratio_writes_the_split_with_that_id_and_bumps_version() {
        let mut mgr = ViewManager::new();
        let v = main_active(&mgr);
        let x = first_slot_of(&mgr, v);
        let y = mgr.split_slot(v, x, SplitDir::LeftRight).unwrap();
        mgr.split_slot(v, y, SplitDir::TopBottom).unwrap();
        let ids = split_ids(&mgr, v);
        assert_eq!(ids.len(), 2, "two splits");
        let (outer, inner) = (ids[0], ids[1]);
        let ver = mgr.version;

        assert_eq!(mgr.set_split_ratio(v, inner, 0.7), Ok(applied(0.7)));
        assert_eq!(ratio_in(&mgr, v, inner), 0.7);
        assert_eq!(
            ratio_in(&mgr, v, outer),
            tree::SPLIT_RATIO,
            "the other split unchanged"
        );
        assert_eq!(mgr.version, ver + 1);
        assert_eq!(mgr.snapshot(v).unwrap().version, ver + 1);
    }

    #[test]
    fn set_split_ratio_of_a_missing_split_or_view_changes_nothing() {
        let (mut mgr, v, _split) = one_split(SplitDir::LeftRight);
        let other = mgr.create_tab(MAIN_WINDOW_LABEL, None).unwrap();
        let other_slot = first_slot_of(&mgr, other);
        mgr.split_slot(other, other_slot, SplitDir::TopBottom)
            .unwrap();
        let foreign = split_ids(&mgr, other)[0];
        let bogus = Uuid::new_v4();
        let slot = first_slot_of(&mgr, v);
        let before = mgr.views[&v].clone();
        let ver = mgr.version;

        for missing in [bogus, slot, foreign] {
            assert_eq!(
                mgr.set_split_ratio(v, missing, 0.3),
                Err(LayoutError::SplitNotFound(missing)),
                "area ids · splits of other tabs are not splits of this tab either"
            );
        }
        assert_eq!(
            mgr.set_split_ratio(bogus, foreign, 0.3),
            Err(LayoutError::ViewNotFound(bogus))
        );
        assert_eq!(mgr.views[&v], before);
        assert_eq!(ratio_in(&mgr, other, foreign), tree::SPLIT_RATIO);
        assert_eq!(mgr.version, ver);
    }

    #[test]
    fn set_split_ratio_refuses_non_finite_values_at_the_manager() {
        let (mut mgr, v, split) = one_split(SplitDir::LeftRight);
        let before = mgr.views[&v].clone();
        let ver = mgr.version;
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert!(
                matches!(
                    mgr.set_split_ratio(v, split, bad),
                    Err(LayoutError::InvalidRatio(_))
                ),
                "{bad}: must be refused"
            );
            // Caught before a missing view — the decision order is the contract.
            assert!(matches!(
                mgr.set_split_ratio(Uuid::new_v4(), split, bad),
                Err(LayoutError::InvalidRatio(_))
            ));
        }
        assert_eq!(mgr.views[&v], before);
        assert_eq!(mgr.version, ver);
    }

    #[test]
    fn set_split_ratio_clamps_to_the_ratio_bounds() {
        let (mut mgr, v, split) = one_split(SplitDir::LeftRight);
        assert_eq!(
            mgr.set_split_ratio(v, split, 0.0),
            Ok(applied(tree::RATIO_MIN))
        );
        assert_eq!(ratio_in(&mgr, v, split), tree::RATIO_MIN);
        assert_eq!(
            mgr.set_split_ratio(v, split, 1.0),
            Ok(applied(tree::RATIO_MAX))
        );
        assert_eq!(
            mgr.set_split_ratio(v, split, -5.0),
            Ok(applied(tree::RATIO_MIN))
        );
        // If the clipped value equals the current value, no change.
        assert_untouched(
            &mut mgr,
            v,
            split,
            0.005,
            SplitRatioResult {
                ratio: tree::RATIO_MIN,
                outcome: SplitRatioOutcome::Unchanged,
            },
        );
    }

    #[test]
    fn ratio_0_3_round_trips_exactly() {
        let (mut mgr, v, split) = one_split(SplitDir::TopBottom);
        let got = mgr.set_split_ratio(v, split, 0.3).unwrap();
        assert_eq!(got.ratio.to_bits(), 0.3f64.to_bits());
        assert_eq!(ratio_in(&mgr, v, split).to_bits(), 0.3f64.to_bits());
        let wire = serde_json::to_value(mgr.snapshot(v).unwrap().layout).unwrap();
        assert_eq!(wire["ratio"], serde_json::json!(0.3), "{wire}");
        assert_eq!(serde_json::to_string(&got.ratio).unwrap(), "0.3");
    }

    #[test]
    fn the_same_value_is_unchanged_without_a_version_bump() {
        let (mut mgr, v, split) = one_split(SplitDir::LeftRight);
        assert_untouched(
            &mut mgr,
            v,
            split,
            tree::SPLIT_RATIO,
            SplitRatioResult {
                ratio: tree::SPLIT_RATIO,
                outcome: SplitRatioOutcome::Unchanged,
            },
        );
        mgr.set_split_ratio(v, split, 0.3).unwrap();
        assert_untouched(
            &mut mgr,
            v,
            split,
            0.3,
            SplitRatioResult {
                ratio: 0.3,
                outcome: SplitRatioOutcome::Unchanged,
            },
        );
    }

    #[test]
    fn px_minimum_keeps_both_sides_at_least_min_pane_px_on_the_split_axis() {
        // Two horizontal splits: the outer (box width 1000px) · inside the right area (box width 500px). m = 100.
        let mut mgr = ViewManager::new();
        let v = main_active(&mgr);
        let x = first_slot_of(&mgr, v);
        let y = mgr.split_slot(v, x, SplitDir::LeftRight).unwrap();
        mgr.split_slot(v, y, SplitDir::LeftRight).unwrap();
        let ids = split_ids(&mgr, v);
        assert_eq!(ids.len(), 2, "two splits");
        let (outer, inner) = (ids[0], ids[1]);
        mgr.set_window_canvas(MAIN_WINDOW_LABEL, 1000, 400).unwrap();
        mgr.set_ui_metrics(MAIN_WINDOW_LABEL, metrics(1.0, 1.0, 1.0, 1.0, 100))
            .unwrap();

        // Inner: L = 1000 × 0.5 = 500 → allowed [0.2, 0.8]. Measuring with the whole canvas width would let 0.1 pass.
        assert_eq!(mgr.set_split_ratio(v, inner, 0.1), Ok(applied(0.2)));
        assert_eq!(ratio_in(&mgr, v, inner), 0.2);
        assert_eq!(mgr.set_split_ratio(v, inner, 0.95), Ok(applied(0.8)));
        assert_eq!(
            mgr.set_split_ratio(v, inner, 0.4),
            Ok(applied(0.4)),
            "inside the range unchanged"
        );
        // Outer: L = 1000 → m/L = 0.1 is larger than the ratio limit, so px wins.
        assert_eq!(mgr.set_split_ratio(v, outer, 0.05), Ok(applied(0.1)));
        assert_eq!(mgr.set_split_ratio(v, outer, 0.99), Ok(applied(0.9)));

        // Top/bottom splits are measured by the canvas height: L = 400 → allowed [0.25, 0.75].
        let (mut tb, tv, split) = one_split(SplitDir::TopBottom);
        tb.set_window_canvas(MAIN_WINDOW_LABEL, 1000, 400).unwrap();
        tb.set_ui_metrics(MAIN_WINDOW_LABEL, metrics(1.0, 1.0, 1.0, 1.0, 100))
            .unwrap();
        assert_eq!(tb.set_split_ratio(tv, split, 0.1), Ok(applied(0.25)));
        assert_eq!(tb.set_split_ratio(tv, split, 0.9), Ok(applied(0.75)));
    }

    // ADR-0260
    #[test]
    fn at_the_screen_minimum_either_pane_of_a_top_bottom_split_stops_at_30px() {
        // Screen minimum area = 30px (frontend `MIN_PANE_PX`). The top area's wall is the lower bound, the bottom area's wall the upper bound.
        let (mut mgr, v, split) = one_split(SplitDir::TopBottom);
        mgr.set_window_canvas(MAIN_WINDOW_LABEL, 800, 1000).unwrap();
        mgr.set_ui_metrics(MAIN_WINDOW_LABEL, metrics(1.0, 1.0, 1.0, 1.0, 30))
            .unwrap();
        let row = mgr
            .list_splits(v)
            .unwrap()
            .into_iter()
            .next()
            .expect("one split");
        let (top, bottom) = (row.a_slots[0], row.b_slots[0]);
        let height = |mgr: &ViewManager, slot| {
            let f = mgr.slot_px(v, slot).unwrap().unwrap().frame;
            f.y1 - f.y0
        };

        // L = 1000 → allowed [0.03, 0.97].
        assert_eq!(mgr.set_split_ratio(v, split, 0.01), Ok(applied(0.03)));
        assert_eq!(height(&mgr, top), 30);
        assert_untouched(
            &mut mgr,
            v,
            split,
            0.0,
            SplitRatioResult {
                ratio: 0.03,
                outcome: SplitRatioOutcome::Unchanged,
            },
        );
        assert_eq!(mgr.set_split_ratio(v, split, 0.99), Ok(applied(0.97)));
        assert_eq!(height(&mgr, bottom), 30);

        // When L exceeds 3000px, 30/L is smaller than the ratio limit, so the ratio limit wins (L = 5000 → 0.006).
        mgr.set_window_canvas(MAIN_WINDOW_LABEL, 800, 5000).unwrap();
        assert_eq!(
            mgr.set_split_ratio(v, split, 0.0),
            Ok(applied(tree::RATIO_MIN))
        );
        assert_eq!(
            mgr.set_split_ratio(v, split, 1.0),
            Ok(applied(tree::RATIO_MAX))
        );
    }

    #[test]
    fn an_empty_px_range_is_too_small_and_keeps_the_current_ratio() {
        let (mut mgr, v, split) = one_split(SplitDir::LeftRight);
        mgr.set_split_ratio(v, split, 0.3).unwrap();
        // L = 300 < 2m = 400.
        mgr.set_window_canvas(MAIN_WINDOW_LABEL, 300, 300).unwrap();
        mgr.set_ui_metrics(MAIN_WINDOW_LABEL, metrics(1.0, 1.0, 1.0, 1.0, 200))
            .unwrap();
        let too_small = SplitRatioResult {
            ratio: 0.3,
            outcome: SplitRatioOutcome::TooSmall,
        };
        for asked in [0.6, 0.3, 0.1] {
            assert_untouched(&mut mgr, v, split, asked, too_small);
        }
    }

    #[test]
    fn without_canvas_or_metrics_only_the_ratio_bounds_apply() {
        // With only one of the two the px minimum does not apply — with both it would be a world clipped to [0.2, 0.8].
        let (mut only_canvas, v1, s1) = one_split(SplitDir::LeftRight);
        only_canvas
            .set_window_canvas(MAIN_WINDOW_LABEL, 500, 500)
            .unwrap();
        assert_eq!(only_canvas.set_split_ratio(v1, s1, 0.1), Ok(applied(0.1)));
        assert_eq!(
            only_canvas.set_split_ratio(v1, s1, 0.0),
            Ok(applied(tree::RATIO_MIN))
        );

        let (mut only_metrics, v2, s2) = one_split(SplitDir::LeftRight);
        only_metrics
            .set_ui_metrics(MAIN_WINDOW_LABEL, metrics(1.0, 1.0, 1.0, 1.0, 100))
            .unwrap();
        assert_eq!(only_metrics.set_split_ratio(v2, s2, 0.1), Ok(applied(0.1)));
        assert_eq!(
            only_metrics.set_split_ratio(v2, s2, 0.001),
            Ok(applied(tree::RATIO_MIN))
        );
        assert_eq!(
            only_metrics.set_split_ratio(v2, s2, 0.999),
            Ok(applied(tree::RATIO_MAX))
        );
    }

    #[test]
    fn a_split_that_makes_panes_smaller_than_min_pane_px_is_allowed() {
        // R11: even knowing the canvas · metrics, a split is not refused by px.
        let mut mgr = ViewManager::new();
        let v = main_active(&mgr);
        let x = first_slot_of(&mgr, v);
        mgr.set_window_canvas(MAIN_WINDOW_LABEL, 40, 40).unwrap();
        mgr.set_ui_metrics(MAIN_WINDOW_LABEL, metrics(0.0, 0.0, 0.0, 0.0, 30))
            .unwrap();
        let before = mgr.views[&v].layout.clone();

        let y = mgr
            .split_slot(v, x, SplitDir::LeftRight)
            .expect("a split making an area under 30px also succeeds");
        assert_ne!(mgr.views[&v].layout, before, "the tree changes");
        for slot in [x, y] {
            let f = mgr.slot_px(v, slot).unwrap().unwrap().frame;
            assert!(f.x1 - f.x0 < 30, "{slot}: {f:?}");
        }
        mgr.split_slot(v, y, SplitDir::TopBottom)
            .expect("it splits even smaller");
    }

    #[test]
    fn repeated_same_side_splits_stop_with_split_too_deep_before_a_zero_width_slot() {
        for dir in [SplitDir::LeftRight, SplitDir::TopBottom] {
            let mut mgr = ViewManager::new();
            let v = main_active(&mgr);
            // Keeps splitting the new area (b = right/bottom) — the boundary crowds toward 1.0.
            let mut target = first_slot_of(&mgr, v);
            let mut splits = 0;
            let err = loop {
                let before = mgr.views[&v].clone();
                let ver = mgr.version;
                match mgr.split_slot(v, target, dir) {
                    Ok(new) => {
                        target = new;
                        splits += 1;
                        assert!(splits < 200, "{dir:?}: the guard does not kick in");
                    }
                    Err(e) => {
                        assert_eq!(
                            mgr.views[&v], before,
                            "{dir:?}: a refusal leaves the tree unchanged"
                        );
                        assert_eq!(mgr.version, ver);
                        break e;
                    }
                }
            };
            assert_eq!(err, LayoutError::SplitTooDeep);
            // It stopped at the f64 limit (not refused early) — a halving chain reaches it at about 54 levels.
            assert!(splits > 40, "{dir:?}: stopped at {splits} levels");
            // Right before that: the area still has width, but the next boundary equals the area's end.
            let r = slot_rect(&mgr, v, target);
            let (lo, hi) = match dir {
                SplitDir::LeftRight => (r.x0, r.x1),
                SplitDir::TopBottom => (r.y0, r.y1),
            };
            assert!(lo < hi, "{dir:?}: {r:?}");
            let at = geometry::boundary(lo, hi, tree::SPLIT_RATIO);
            assert!(at == lo || at == hi, "{dir:?}: at={at} lo={lo} hi={hi}");
            assert_every_leaf_has_area(&mgr, v);
            mgr.snapshot(v)
                .expect("a snapshot at this depth also stands without panic");
        }
    }

    // Sets every split of the chain to 0.9 — the new area being split (b) has a 10% share, so the boundary crowds toward 1.0 much faster.
    fn ninety_chain(mgr: &mut ViewManager, v: ViewId) -> (usize, LayoutError) {
        let mut target = first_slot_of(mgr, v);
        let mut levels = 0;
        loop {
            let before = mgr.views[&v].clone();
            match mgr.split_slot(v, target, SplitDir::LeftRight) {
                Ok(new) => {
                    let deepest = *split_ids(mgr, v).last().unwrap();
                    assert_eq!(
                        mgr.set_split_ratio(v, deepest, 0.9),
                        Ok(applied(0.9)),
                        "{levels} levels"
                    );
                    target = new;
                    levels += 1;
                    assert!(levels < 200, "the guard does not kick in");
                }
                Err(e) => {
                    assert_eq!(mgr.views[&v], before, "a refusal leaves the tree unchanged");
                    return (levels, e);
                }
            }
        }
    }

    #[test]
    fn a_ninety_percent_chain_also_stops_with_split_too_deep() {
        let mut mgr = ViewManager::new();
        let v = main_active(&mgr);
        let (levels, err) = ninety_chain(&mut mgr, v);
        assert_eq!(err, LayoutError::SplitTooDeep);
        // Shrinking by 10% reaches it at about 17 levels — much shallower than a halving chain.
        assert!((10..30).contains(&levels), "{levels} levels");
        assert_every_leaf_has_area(&mgr, v);
    }

    #[test]
    fn an_ancestor_ratio_that_would_collapse_a_deep_leaf_is_too_small() {
        let mut mgr = ViewManager::new();
        let v = main_active(&mgr);
        let mut target = first_slot_of(&mgr, v);
        while let Ok(new) = mgr.split_slot(v, target, SplitDir::LeftRight) {
            target = new;
        }
        let root = split_ids(&mgr, v)[0];
        // Setting the root to 0.9 makes the whole chain start at width 0.1 and the last areas become width 0. The limit end is narrower still.
        for asked in [0.9, tree::RATIO_MAX] {
            assert_untouched(
                &mut mgr,
                v,
                root,
                asked,
                SplitRatioResult {
                    ratio: tree::SPLIT_RATIO,
                    outcome: SplitRatioOutcome::TooSmall,
                },
            );
        }
        assert_every_leaf_has_area(&mgr, v);
        // Contrast: the direction that widens the chain collapses no area and is applied — the filter is not a blanket refusal.
        assert_eq!(mgr.set_split_ratio(v, root, 0.1), Ok(applied(0.1)));
        assert_eq!(
            mgr.set_split_ratio(v, root, tree::RATIO_MIN),
            Ok(applied(tree::RATIO_MIN))
        );
        assert_every_leaf_has_area(&mgr, v);
    }

    // A tree the write path cannot make: a left/right halving chain (always side b is the next level) — areas are width 0 from level 54 on.
    fn planted_half_chain(depth: usize) -> LayoutNode {
        let mut node = LayoutNode::new_empty_slot();
        for _ in 0..depth {
            node = LayoutNode::Split {
                id: Uuid::new_v4(),
                dir: SplitDir::LeftRight,
                ratio: 0.5,
                a: Box::new(LayoutNode::new_empty_slot()),
                b: Box::new(node),
            };
        }
        node
    }

    #[test]
    fn a_planted_zero_width_slot_is_too_deep_to_split_on_the_other_axis_too() {
        let mut mgr = ViewManager::new();
        let v = main_active(&mgr);
        mgr.views.get_mut(&v).unwrap().layout = planted_half_chain(60);
        let zero = geometry::compute(&mgr.views[&v].layout)
            .slots
            .into_iter()
            .find(|r| !has_area(r))
            .expect("premise: an area-0 area was planted");
        assert!(
            zero.x0 == zero.x1 && zero.y0 < zero.y1,
            "premise: only the width is 0: {zero:?}"
        );
        // Splitting top/bottom puts the new boundary strictly inside the height, but both new areas have width 0.
        let before = mgr.views[&v].clone();
        let ver = mgr.version;
        assert_eq!(
            mgr.split_slot(v, zero.slot_id, SplitDir::TopBottom),
            Err(LayoutError::SplitTooDeep)
        );
        assert_eq!(mgr.views[&v], before, "a refusal leaves the tree unchanged");
        assert_eq!(mgr.version, ver);
    }

    #[test]
    fn a_leaf_that_already_had_zero_area_does_not_lock_the_view() {
        // Top = horizontal split Q{x, y} · bottom = a planted halving chain of 60 levels.
        let q = Uuid::new_v4();
        let mut mgr = ViewManager::new();
        let v = main_active(&mgr);
        mgr.views.get_mut(&v).unwrap().layout = LayoutNode::Split {
            id: Uuid::new_v4(),
            dir: SplitDir::TopBottom,
            ratio: 0.5,
            a: Box::new(LayoutNode::Split {
                id: q,
                dir: SplitDir::LeftRight,
                ratio: 0.5,
                a: Box::new(LayoutNode::new_empty_slot()),
                b: Box::new(LayoutNode::new_empty_slot()),
            }),
            b: Box::new(planted_half_chain(60)),
        };
        let zero = geometry::compute(&mgr.views[&v].layout)
            .slots
            .iter()
            .filter(|r| !has_area(r))
            .count();
        assert!(zero > 0, "premise: an area-0 area was planted");

        assert_eq!(mgr.set_split_ratio(v, q, 0.3), Ok(applied(0.3)));
        assert_eq!(ratio_in(&mgr, v, q), 0.3);
    }

    #[test]
    fn list_splits_of_a_missing_view_is_view_not_found() {
        let (mgr, v, split) = one_split(SplitDir::LeftRight);
        let rows = mgr.list_splits(v).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, split);
        let bogus = Uuid::new_v4();
        assert_eq!(
            mgr.list_splits(bogus),
            Err(LayoutError::ViewNotFound(bogus))
        );
    }

    // ── Docking (TRD S21-slot-docking 5.8th edition §1-2 · §9 「model」 · 7th edition §8-1 ⑦) ──────

    const P1: &str = "slot-popup-1";
    const P2: &str = "slot-popup-2";
    const FRESH: &str = "slot-popup-9";

    // main [m0, m1, m2] (active m0 · m1 = two areas left/right) · P1 [p0, p1] (active p0 · p1 = two areas left/right, agent on the left) ·
    // P2 [q0].
    struct Fx {
        mgr: ViewManager,
        m: [ViewId; 3],
        p: [ViewId; 2],
        q: ViewId,
    }

    fn fixture() -> Fx {
        let mut mgr = ViewManager::new();
        let m0 = main_active(&mgr);
        let m1 = mgr.create_tab(MAIN_WINDOW_LABEL, None).unwrap();
        let m2 = mgr.create_tab(MAIN_WINDOW_LABEL, None).unwrap();
        let s = first_slot_of(&mgr, m1);
        mgr.split_slot(m1, s, SplitDir::LeftRight).unwrap();
        mgr.switch_tab(MAIN_WINDOW_LABEL, m0).unwrap();
        let p0 = mgr.create_window(P1).unwrap();
        let p1 = mgr.create_tab(P1, None).unwrap();
        let s = first_slot_of(&mgr, p1);
        mgr.split_slot(p1, s, SplitDir::LeftRight).unwrap();
        mgr.assign_agent(p1, s, "a-1".into()).unwrap();
        mgr.switch_tab(P1, p0).unwrap();
        let q0 = mgr.create_window(P2).unwrap();
        assert_invariants(&mgr);
        Fx {
            mgr,
            m: [m0, m1, m2],
            p: [p0, p1],
            q: q0,
        }
    }

    // The whole model to compare whether a failure is a no-change (sorted to drop the table iteration order).
    #[derive(Debug, PartialEq)]
    struct Model {
        version: u64,
        attrs_rev: u64,
        views: Vec<View>,
        owners: Vec<(ViewId, WindowLabel)>,
        windows: Vec<(WindowLabel, Vec<ViewId>, ViewId, String, u32, WindowAttrs)>,
        unknown: Vec<(ViewId, Vec<(Uuid, Map<String, Value>)>)>,
        closed: Vec<WindowLabel>,
        closing: Vec<u32>,
    }

    fn model(mgr: &ViewManager) -> Model {
        let mut views: Vec<View> = mgr.views.values().cloned().collect();
        views.sort_by_key(|v| v.id);
        let mut owners: Vec<_> = mgr
            .view_owner
            .iter()
            .map(|(v, l)| (*v, l.clone()))
            .collect();
        owners.sort();
        let mut windows: Vec<_> = mgr
            .windows
            .iter()
            .map(|(l, w)| {
                (
                    l.clone(),
                    w.tabs.clone(),
                    w.active,
                    w.window_id.clone(),
                    w.number,
                    w.attrs,
                )
            })
            .collect();
        windows.sort_by(|a, b| a.0.cmp(&b.0));
        let mut unknown: Vec<_> = mgr
            .unknown_content
            .iter()
            .map(|(v, slots)| {
                let mut slots: Vec<_> = slots.iter().map(|(s, m)| (*s, m.clone())).collect();
                slots.sort_by_key(|(s, _)| *s);
                (*v, slots)
            })
            .collect();
        unknown.sort_by_key(|(v, _)| *v);
        let mut closed: Vec<_> = mgr.closed_labels.iter().cloned().collect();
        closed.sort();
        Model {
            version: mgr.version,
            attrs_rev: mgr.attrs_rev,
            views,
            owners,
            windows,
            unknown,
            closed,
            closing: mgr.closing_numbers(),
        }
    }

    fn seam(target: ViewId, anchor: DockAnchor, side: DockSide) -> DockTo {
        DockTo::Seam {
            target,
            anchor,
            side,
            share: dock::DOCK_NEW_SHARE,
        }
    }

    fn tab_bar(window: &str, index: Option<usize>) -> DockTo {
        DockTo::TabBar {
            window: window.to_string(),
            index,
            after: None,
        }
    }

    fn new_window(label: &str) -> DockTo {
        DockTo::NewWindow {
            label: label.to_string(),
        }
    }

    fn layout_of(mgr: &ViewManager, view: ViewId) -> LayoutNode {
        mgr.views[&view].layout.clone()
    }

    fn slots_of(mgr: &ViewManager, view: ViewId) -> Vec<Uuid> {
        tree::slot_ids(&mgr.views[&view].layout)
    }

    fn plant(mgr: &mut ViewManager, view: ViewId, layout: LayoutNode) {
        let v = mgr.views.get_mut(&view).unwrap();
        v.layout = layout;
        ViewManager::fixup_focus(v);
    }

    // Success — version goes up exactly once and the invariants hold.
    fn docked(mgr: &mut ViewManager, view: ViewId, to: DockTo, focus: FocusPolicy) -> Docked {
        let before = mgr.version;
        let out = mgr.dock(view, to, focus, DockMove::default()).unwrap();
        assert_eq!(mgr.version, before + 1, "success bumps version once");
        assert_invariants(mgr);
        out
    }

    // Failure — the same reason under both focus policies and the whole model (version · tombstones included) unchanged.
    fn refused(mgr: &mut ViewManager, view: ViewId, to: DockTo, want: DockError) {
        let before = model(mgr);
        for focus in [FocusPolicy::Keep, FocusPolicy::Reveal] {
            assert_eq!(
                mgr.dock(view, to.clone(), focus, DockMove::default()),
                Err(want.clone()),
                "{to:?} {focus:?}"
            );
            assert_eq!(model(mgr), before, "a failure changes nothing: {want:?}");
        }
        assert_invariants(mgr);
    }

    #[test]
    fn seam_wraps_only_the_anchor_slot_and_moves_the_source_tab_whole() {
        let Fx { mut mgr, m, p, .. } = fixture();
        let target_before = layout_of(&mgr, m[1]);
        let source_before = layout_of(&mgr, p[1]);
        let LayoutNode::Split {
            id: outer_id,
            a: kept,
            b: anchor_node,
            ..
        } = &target_before
        else {
            panic!("premise: m1 has two areas left/right")
        };
        let anchor = tree::first_slot_id(anchor_node);
        let focus_before = mgr.views[&m[1]].focused_slot_id;

        let out = docked(
            &mut mgr,
            p[1],
            seam(m[1], DockAnchor::Slot(anchor), DockSide::Top),
            FocusPolicy::Keep,
        );
        assert_eq!(
            out,
            Docked {
                window: MAIN_WINDOW_LABEL.to_string(),
                slot_ids: tree::slot_ids(&source_before),
                source_window: P1.to_string(),
                emptied: None,
                main_refilled: None,
            }
        );
        // Only the reference area's position is wrapped in a split (source top 1/3 · reference bottom 2/3), and the rest · the source nodes stay with ids · ratios · contents as is.
        let LayoutNode::Split {
            id, a, b: wrapped, ..
        } = &mgr.views[&m[1]].layout
        else {
            panic!("the root split must remain")
        };
        assert_eq!((id, a), (outer_id, kept));
        let LayoutNode::Split {
            id: fresh,
            dir,
            ratio,
            a,
            b,
        } = &**wrapped
        else {
            panic!("the reference area must be wrapped in a split")
        };
        assert_eq!((*dir, *ratio), (SplitDir::TopBottom, dock::DOCK_NEW_SHARE));
        assert_eq!((&**a, &**b), (&source_before, &**anchor_node));
        let old_splits: Vec<Uuid> = tree::list_splits(&target_before)
            .into_iter()
            .chain(tree::list_splits(&source_before))
            .map(|s| s.id)
            .collect();
        assert!(
            !old_splits.contains(fresh),
            "only the wrapping split has a new id"
        );

        assert!(!mgr.views.contains_key(&p[1]), "the source tab vanishes");
        assert!(!mgr.view_owner.contains_key(&p[1]));
        assert_eq!(mgr.windows[P1].tabs, vec![p[0]]);
        assert_eq!(
            main_active(&mgr),
            m[0],
            "Keep — the target window's visible tab unchanged"
        );
        assert_eq!(
            mgr.views[&m[1]].focused_slot_id, focus_before,
            "Keep — the target focus unchanged"
        );
    }

    #[test]
    fn seam_at_the_outer_edge_wraps_the_whole_target_and_reveal_shows_it() {
        let Fx { mut mgr, m, q, .. } = fixture();
        let target_before = layout_of(&mgr, m[2]);
        let source_before = layout_of(&mgr, q);
        let out = docked(
            &mut mgr,
            q,
            seam(m[2], DockAnchor::Outer, DockSide::Bottom),
            FocusPolicy::Reveal,
        );
        let LayoutNode::Split {
            dir, ratio, a, b, ..
        } = &mgr.views[&m[2]].layout
        else {
            panic!("the root must be the new split")
        };
        assert_eq!(
            (*dir, *ratio),
            (SplitDir::TopBottom, 1.0 - dock::DOCK_NEW_SHARE)
        );
        assert_eq!((&**a, &**b), (&target_before, &source_before));
        // A popout with only one tab empties and vanishes with its window entry (ADR-0057 D-3).
        assert!(out.emptied.is_some());
        assert!(!mgr.windows.contains_key(P2));
        assert_eq!(
            main_active(&mgr),
            m[2],
            "Reveal — the target tab is visible"
        );
        assert_eq!(
            mgr.views[&m[2]].focused_slot_id,
            Some(tree::first_slot_id(&source_before)),
            "Reveal — focus on the first inserted slot"
        );
    }

    #[test]
    fn seam_carries_the_unknown_content_side_table_to_the_target_tab() {
        let Fx { mut mgr, m, p, .. } = fixture();
        let foreign = slots_of(&mgr, p[1])[1];
        mgr.set_unknown_content(p[1], foreign, raw("from_the_future"))
            .unwrap();
        docked(
            &mut mgr,
            p[1],
            seam(m[0], DockAnchor::Outer, DockSide::Left),
            FocusPolicy::Keep,
        );
        assert_eq!(
            mgr.unknown_content(m[0], foreign),
            Some(&raw("from_the_future"))
        );
        assert!(!mgr.unknown_content.contains_key(&p[1]));
        assert_eq!(mgr.snapshot(m[0]).unwrap().foreign_slots, vec![foreign]);
        assert!(!mgr.slot_is_free(m[0], foreign).unwrap());
    }

    #[test]
    fn docking_the_visible_tab_inside_its_own_window_makes_the_target_visible() {
        // ★If the source is the target window's visible tab, 「the visible tab unchanged」 cannot hold — the target is visible (TRD S21-slot-docking 5.8th edition §1-2 「visible tab」 · §11-1 #1)★.
        let Fx { mut mgr, m, .. } = fixture();
        mgr.switch_tab(MAIN_WINDOW_LABEL, m[1]).unwrap();
        assert_eq!(
            succession(&mgr.windows[MAIN_WINDOW_LABEL].tabs, m[1]),
            Some(m[2]),
            "premise: a close succession would be the right neighbor"
        );
        docked(
            &mut mgr,
            m[1],
            seam(m[0], DockAnchor::Outer, DockSide::Right),
            FocusPolicy::Keep,
        );
        assert_eq!(main_active(&mgr), m[0]);

        // If the source is not the visible tab, it stays the same even in the same window.
        let Fx { mut mgr, m, .. } = fixture();
        mgr.switch_tab(MAIN_WINDOW_LABEL, m[2]).unwrap();
        docked(
            &mut mgr,
            m[1],
            seam(m[0], DockAnchor::Outer, DockSide::Right),
            FocusPolicy::Keep,
        );
        assert_eq!(main_active(&mgr), m[2]);
    }

    #[test]
    fn the_source_window_hands_its_visible_tab_over_by_succession() {
        for (active, moved, heir) in [(0, 0, 1), (1, 1, 0)] {
            let Fx { mut mgr, m, p, .. } = fixture();
            mgr.switch_tab(P1, p[active]).unwrap();
            docked(
                &mut mgr,
                p[moved],
                seam(m[0], DockAnchor::Outer, DockSide::Left),
                FocusPolicy::Reveal,
            );
            assert_eq!(mgr.windows[P1].active, p[heir]);
        }
    }

    #[test]
    fn tab_bar_moves_the_view_whole_to_the_requested_position() {
        let Fx { mut mgr, m, p, .. } = fixture();
        let view_before = mgr.views[&p[0]].clone();
        let out = docked(
            &mut mgr,
            p[0],
            tab_bar(MAIN_WINDOW_LABEL, Some(1)),
            FocusPolicy::Keep,
        );
        assert_eq!(
            out,
            Docked {
                window: MAIN_WINDOW_LABEL.to_string(),
                slot_ids: tree::slot_ids(&view_before.layout),
                source_window: P1.to_string(),
                emptied: None,
                main_refilled: None,
            }
        );
        assert_eq!(
            mgr.windows[MAIN_WINDOW_LABEL].tabs,
            vec![m[0], p[0], m[1], m[2]]
        );
        assert_eq!(
            mgr.views[&p[0]], view_before,
            "tab id · layout · focus · name unchanged"
        );
        assert_eq!(
            mgr.owner_of(p[0]).map(String::as_str),
            Some(MAIN_WINDOW_LABEL)
        );
        assert_eq!(main_active(&mgr), m[0], "Keep — the visible tab unchanged");
        assert_eq!(mgr.windows[P1].tabs, vec![p[1]]);
        assert_eq!(mgr.windows[P1].active, p[1], "the source window succeeds");

        // An overflowing position · no position = at the end. A popout with only one tab empties and closes.
        let Fx { mut mgr, m, p, q } = fixture();
        let out = docked(
            &mut mgr,
            q,
            tab_bar(MAIN_WINDOW_LABEL, Some(99)),
            FocusPolicy::Keep,
        );
        assert!(out.emptied.is_some());
        assert!(!mgr.windows.contains_key(P2));
        docked(
            &mut mgr,
            p[1],
            tab_bar(MAIN_WINDOW_LABEL, None),
            FocusPolicy::Keep,
        );
        assert_eq!(
            mgr.windows[MAIN_WINDOW_LABEL].tabs,
            vec![m[0], m[1], m[2], q, p[1]]
        );
    }

    #[test]
    fn tab_bar_reveal_shows_the_moved_tab() {
        let Fx { mut mgr, p, .. } = fixture();
        docked(
            &mut mgr,
            p[1],
            tab_bar(MAIN_WINDOW_LABEL, Some(0)),
            FocusPolicy::Reveal,
        );
        assert_eq!(main_active(&mgr), p[1]);
    }

    #[test]
    fn tab_bar_in_the_same_window_reorders_and_keeps_the_visible_tab() {
        // Moving the visible tab keeps that tab visible (same-window TabBar — TRD S21-slot-docking 5.8th edition §1-2 「visible tab」).
        let Fx { mut mgr, m, .. } = fixture();
        let out = docked(
            &mut mgr,
            m[0],
            tab_bar(MAIN_WINDOW_LABEL, None),
            FocusPolicy::Keep,
        );
        assert!(!out.emptied.is_some());
        assert_eq!(mgr.windows[MAIN_WINDOW_LABEL].tabs, vec![m[1], m[2], m[0]]);
        assert_eq!(main_active(&mgr), m[0]);

        // Moving a tab that is not visible: Keep keeps the visible tab · Reveal shows the moved tab.
        let Fx { mut mgr, m, .. } = fixture();
        docked(
            &mut mgr,
            m[2],
            tab_bar(MAIN_WINDOW_LABEL, Some(0)),
            FocusPolicy::Keep,
        );
        assert_eq!(mgr.windows[MAIN_WINDOW_LABEL].tabs, vec![m[2], m[0], m[1]]);
        assert_eq!(main_active(&mgr), m[0]);
        docked(
            &mut mgr,
            m[1],
            tab_bar(MAIN_WINDOW_LABEL, Some(1)),
            FocusPolicy::Reveal,
        );
        assert_eq!(mgr.windows[MAIN_WINDOW_LABEL].tabs, vec![m[2], m[1], m[0]]);
        assert_eq!(main_active(&mgr), m[1]);
    }

    #[test]
    fn tab_bar_onto_the_same_position_changes_nothing_unless_reveal_must_show_it() {
        // Same position = no change · version unchanged — when Keep or the tab is already visible.
        let Fx { mut mgr, m, .. } = fixture();
        let same_spots = [
            (m[0], Some(0)),
            (m[1], Some(1)),
            (m[2], None),
            (m[2], Some(7)),
        ];
        for (view, index) in same_spots {
            let before = model(&mgr);
            let out = mgr
                .dock(
                    view,
                    tab_bar(MAIN_WINDOW_LABEL, index),
                    FocusPolicy::Keep,
                    DockMove::default(),
                )
                .unwrap();
            assert_eq!(out.window, MAIN_WINDOW_LABEL);
            assert!(!out.emptied.is_some());
            assert_eq!(model(&mgr), before, "Keep: {view} {index:?}");
        }
        let before = model(&mgr);
        mgr.dock(
            m[0],
            tab_bar(MAIN_WINDOW_LABEL, Some(0)),
            FocusPolicy::Reveal,
            DockMove::default(),
        )
        .unwrap();
        assert_eq!(model(&mgr), before, "Reveal but the tab is already visible");

        // Reveal with a tab that was not visible only makes it visible — the list unchanged · version once.
        for (at, index) in [(1, Some(1)), (2, None)] {
            let Fx { mut mgr, m, .. } = fixture();
            let view = m[at];
            let tabs = mgr.windows[MAIN_WINDOW_LABEL].tabs.clone();
            let out = docked(
                &mut mgr,
                view,
                tab_bar(MAIN_WINDOW_LABEL, index),
                FocusPolicy::Reveal,
            );
            assert!(!out.emptied.is_some());
            assert_eq!(mgr.windows[MAIN_WINDOW_LABEL].tabs, tabs);
            assert_eq!(main_active(&mgr), view);
        }

        // main's last tab staying in place is not refused either.
        let mut mgr = ViewManager::new();
        let only = main_active(&mgr);
        let before = model(&mgr);
        for focus in [FocusPolicy::Keep, FocusPolicy::Reveal] {
            mgr.dock(
                only,
                tab_bar(MAIN_WINDOW_LABEL, None),
                focus,
                DockMove::default(),
            )
            .unwrap();
            assert_eq!(model(&mgr), before);
        }
    }

    #[test]
    fn new_window_makes_a_popout_entry_holding_only_that_tab() {
        let Fx { mut mgr, p, .. } = fixture();
        let view_before = mgr.views[&p[1]].clone();
        let out = docked(&mut mgr, p[1], new_window(FRESH), FocusPolicy::Keep);
        assert_eq!(
            out,
            Docked {
                window: FRESH.to_string(),
                slot_ids: tree::slot_ids(&view_before.layout),
                source_window: P1.to_string(),
                emptied: None,
                main_refilled: None,
            }
        );
        let w = &mgr.windows[FRESH];
        assert_eq!((w.tabs.clone(), w.active), (vec![p[1]], p[1]));
        assert_ne!(w.window_id, mgr.windows[P1].window_id, "new persisted id");
        assert_eq!(w.attrs, WindowAttrs::default());
        assert_eq!((w.canvas, w.metrics), (None, None));
        assert_eq!(mgr.views[&p[1]], view_before);
        assert_eq!(mgr.windows[P1].tabs, vec![p[0]]);

        // The model holds even from a popout with only one tab (the reuse judgment is `plan_new_window`'s job).
        let Fx { mut mgr, m, q, .. } = fixture();
        assert!(docked(&mut mgr, q, new_window(FRESH), FocusPolicy::Keep)
            .emptied
            .is_some());
        assert!(!mgr.windows.contains_key(P2));
        docked(
            &mut mgr,
            m[1],
            new_window("slot-popup-10"),
            FocusPolicy::Keep,
        );
        assert_eq!(mgr.windows[MAIN_WINDOW_LABEL].tabs, vec![m[0], m[2]]);
    }

    #[test]
    fn missing_sources_and_destinations_are_refused_in_order() {
        let Fx { mut mgr, m, p, .. } = fixture();
        let (bogus, bogus2) = (Uuid::new_v4(), Uuid::new_v4());
        refused(
            &mut mgr,
            bogus,
            seam(bogus2, DockAnchor::Outer, DockSide::Left),
            DockError::TabNotFound(bogus),
        );
        refused(
            &mut mgr,
            p[0],
            seam(bogus2, DockAnchor::Slot(bogus), DockSide::Left),
            DockError::TabNotFound(bogus2),
        );
        refused(
            &mut mgr,
            p[0],
            tab_bar("no-such", None),
            DockError::WindowNotFound("no-such".into()),
        );
        refused(
            &mut mgr,
            p[0],
            new_window(P2),
            DockError::WindowExists(P2.into()),
        );
        refused(
            &mut mgr,
            p[0],
            new_window(MAIN_WINDOW_LABEL),
            DockError::WindowExists(MAIN_WINDOW_LABEL.into()),
        );
        // The tombstone comes before 「already exists」.
        mgr.mark_closed(P2);
        refused(
            &mut mgr,
            p[0],
            new_window(P2),
            DockError::WindowClosed(P2.into()),
        );
        // A windowless temporary View is not a tab — neither as a source nor as a target.
        let slot = first_slot_of(&mgr, m[0]);
        let (tmp, _) = mgr.prepare_detached_view(m[0], slot).unwrap();
        for (view, to, want) in [
            (
                tmp,
                tab_bar(MAIN_WINDOW_LABEL, None),
                DockError::TabNotFound(tmp),
            ),
            (
                p[0],
                seam(tmp, DockAnchor::Outer, DockSide::Left),
                DockError::TabNotFound(tmp),
            ),
        ] {
            let before = model(&mgr);
            assert_eq!(
                mgr.dock(view, to, FocusPolicy::Keep, DockMove::default()),
                Err(want)
            );
            assert_eq!(model(&mgr), before);
        }
        mgr.drop_detached_view(tmp);
        assert_invariants(&mgr);
    }

    #[test]
    fn self_target_and_anchor_slot_checks_are_refused_unchanged() {
        let Fx { mut mgr, m, p, .. } = fixture();
        let bogus = Uuid::new_v4();
        refused(
            &mut mgr,
            p[1],
            seam(p[1], DockAnchor::Slot(bogus), DockSide::Left),
            DockError::SelfTarget,
        );
        let elsewhere = first_slot_of(&mgr, m[2]);
        refused(
            &mut mgr,
            p[1],
            seam(m[1], DockAnchor::Slot(elsewhere), DockSide::Left),
            DockError::SlotNotInTab {
                slot: elsewhere,
                tab: m[1],
            },
        );
        let own = first_slot_of(&mgr, p[1]);
        refused(
            &mut mgr,
            p[1],
            seam(m[1], DockAnchor::Slot(own), DockSide::Left),
            DockError::SlotNotInTab {
                slot: own,
                tab: m[1],
            },
        );
        refused(
            &mut mgr,
            p[1],
            seam(m[1], DockAnchor::Slot(bogus), DockSide::Left),
            DockError::SlotNotFound(bogus),
        );

        // Only main — a refusal does not create an empty tab in main.
        let mut mgr = ViewManager::new();
        let only = main_active(&mgr);
        let p0 = mgr.create_window(P1).unwrap();
        refused(
            &mut mgr,
            only,
            seam(only, DockAnchor::Outer, DockSide::Left),
            DockError::SelfTarget,
        );
        refused(
            &mut mgr,
            only,
            seam(p0, DockAnchor::Slot(bogus), DockSide::Left),
            DockError::SlotNotFound(bogus),
        );
        assert_eq!(mgr.windows[MAIN_WINDOW_LABEL].tabs, vec![only]);
        assert_invariants(&mgr);
    }

    #[test]
    fn a_new_zero_area_slot_is_refused_but_a_wide_place_docks() {
        let Fx { mut mgr, m, p, .. } = fixture();
        plant(&mut mgr, m[1], planted_half_chain(40));
        plant(&mut mgr, p[1], planted_half_chain(20));
        let deepest = *slots_of(&mgr, m[1]).last().unwrap();
        let to = seam(m[1], DockAnchor::Slot(deepest), DockSide::Right);
        refused(&mut mgr, p[1], to, DockError::ZeroArea);
        // The same source attaches at a wide position (the outer edge).
        docked(
            &mut mgr,
            p[1],
            seam(m[1], DockAnchor::Outer, DockSide::Left),
            FocusPolicy::Keep,
        );
    }

    #[test]
    fn a_slot_id_shared_by_both_trees_is_the_internal_slot_id_overlap() {
        let Fx { mut mgr, m, p, .. } = fixture();
        let shared = first_slot_of(&mgr, m[1]);
        plant(
            &mut mgr,
            p[0],
            LayoutNode::Slot {
                id: shared,
                content: SlotContent::Empty,
            },
        );
        refused(
            &mut mgr,
            p[0],
            seam(m[1], DockAnchor::Outer, DockSide::Left),
            DockError::SlotIdOverlap,
        );
    }

    #[test]
    fn plan_new_window_reuses_a_lone_popout_and_sizes_the_rest_from_the_source() {
        let Fx { mut mgr, m, p, q } = fixture();
        let maximized = |bounds| WindowPlacement {
            minimized: false,
            maximized: true,
            bounds,
        };
        // Reuse even if the normal position has not been seen yet.
        assert_eq!(mgr.windows[P2].attrs.bounds, None, "premise");
        assert_eq!(
            mgr.plan_new_window(q),
            Ok(NewWindowPlan::Reuse { label: P2.into() })
        );
        mgr.observe_window_placement(P2, normal_at(30.0)).unwrap();
        assert_eq!(
            mgr.plan_new_window(q),
            Ok(NewWindowPlan::Reuse { label: P2.into() })
        );
        // Reuse even while maximized if a normal position is recorded.
        mgr.observe_window_placement(P2, maximized(WindowBounds::new(0.0, 0.0, 2560.0, 1400.0)))
            .unwrap();
        let attrs = mgr.windows[P2].attrs;
        assert_eq!(
            (attrs.maximized, attrs.bounds),
            (true, Some(bounds(30.0))),
            "premise"
        );
        assert_eq!(
            mgr.plan_new_window(q),
            Ok(NewWindowPlan::Reuse { label: P2.into() })
        );
        assert_eq!(
            mgr.plan_new_window(m[1]),
            Ok(NewWindowPlan::Create { size: None }),
            "main = default popout size"
        );
        assert_eq!(
            mgr.plan_new_window(p[1]),
            Ok(NewWindowPlan::Create { size: None }),
            "a position not seen yet"
        );
        mgr.observe_window_placement(P1, normal_at(10.0)).unwrap();
        assert_eq!(
            mgr.plan_new_window(p[1]),
            Ok(NewWindowPlan::Create {
                size: Some((1280.0, 800.0))
            })
        );
        // Even while maximized it is the normal position size (TRD S21-slot-docking §3-3 「size: None」 · §3-5).
        mgr.observe_window_placement(
            P1,
            WindowPlacement {
                minimized: false,
                maximized: true,
                bounds: WindowBounds::new(0.0, 0.0, 2560.0, 1400.0),
            },
        )
        .unwrap();
        let before = model(&mgr);
        assert_eq!(
            mgr.plan_new_window(p[1]),
            Ok(NewWindowPlan::Create {
                size: Some((1280.0, 800.0))
            })
        );
        assert_eq!(model(&mgr), before, "only judges");
        assert_invariants(&mgr);
    }

    #[test]
    fn plan_new_window_checks_the_source_side_like_dock() {
        let Fx { mgr, .. } = fixture();
        let bogus = Uuid::new_v4();
        assert_eq!(
            mgr.plan_new_window(bogus),
            Err(DockError::TabNotFound(bogus))
        );

        // main's last tab can also leave to a new window (PRD R14) — it only judges, so there is no empty tab yet.
        let mgr = ViewManager::new();
        let only = main_active(&mgr);
        let before = model(&mgr);
        assert_eq!(
            mgr.plan_new_window(only),
            Ok(NewWindowPlan::Create { size: None })
        );
        assert_eq!(model(&mgr), before);
        assert_invariants(&mgr);
    }

    #[test]
    fn a_tombstoned_label_refuses_new_window_attachment_on_both_paths() {
        let Fx { mut mgr, m, p, .. } = fixture();
        let before = model(&mgr);
        mgr.mark_closed(FRESH);
        assert_eq!(
            mgr.version, before.version,
            "a tombstone is not a layout change"
        );
        refused(
            &mut mgr,
            p[1],
            new_window(FRESH),
            DockError::WindowClosed(FRESH.into()),
        );

        // The phase C of a popout (`slot.popout`) also stops at a tombstone — the temporary View remains and waits for the rollback.
        let slot = first_slot_of(&mgr, m[0]);
        let (tmp, _) = mgr.prepare_detached_view(m[0], slot).unwrap();
        let version = mgr.version;
        assert_eq!(
            mgr.attach_view_as_new_window(FRESH, tmp),
            Err(LayoutError::WindowClosed(FRESH.into()))
        );
        assert_eq!(mgr.version, version);
        assert!(!mgr.windows.contains_key(FRESH));
        assert!(!mgr.view_owner.contains_key(&tmp));
        mgr.drop_detached_view(tmp);
        assert_invariants(&mgr);
        // A label without a tombstone attaches as is.
        let (tmp, _) = mgr.prepare_detached_view(m[0], slot).unwrap();
        mgr.attach_view_as_new_window("slot-popup-11", tmp).unwrap();
        assert_invariants(&mgr);
    }

    #[test]
    fn succession_gives_the_same_tab_as_closing() {
        for n in 1..=4 {
            for removed in 0..n {
                let mut mgr = ViewManager::new();
                let first = mgr.create_window(P1).unwrap();
                let mut tabs = vec![first];
                for _ in 1..n {
                    tabs.push(mgr.create_tab(P1, None).unwrap());
                }
                let gone = tabs[removed];
                mgr.switch_tab(P1, gone).unwrap();
                let want = succession(&tabs, gone);
                let out = mgr.close_tab(P1, gone).unwrap();
                match want {
                    Some(heir) => {
                        assert_eq!(mgr.windows[P1].active, heir, "n={n} removed={removed}")
                    }
                    None => assert_eq!(
                        out,
                        CloseTabOutcome::WindowClosed,
                        "alone, there is no tab to take over"
                    ),
                }
                assert_invariants(&mgr);
            }
        }
        let a = Uuid::new_v4();
        assert_eq!(succession(&[a], Uuid::new_v4()), None, "not in the list");
        assert_eq!(succession(&[], a), None);
    }

    #[test]
    fn a_tombstoned_destination_window_reads_as_missing() {
        // The tombstone stands after the model removal, so this combination (tombstone + a window remaining in the model) is a defense branch — the same reason as 「missing」.
        let Fx { mut mgr, m, q, .. } = fixture();
        mgr.mark_closed(P2);
        refused(
            &mut mgr,
            m[1],
            tab_bar(P2, None),
            DockError::WindowNotFound(P2.into()),
        );
        refused(
            &mut mgr,
            m[1],
            seam(q, DockAnchor::Outer, DockSide::Left),
            DockError::TabNotFound(q),
        );
        // A window without a tombstone is accepted as is.
        docked(&mut mgr, m[1], tab_bar(P1, None), FocusPolicy::Keep);
    }

    #[test]
    fn a_popout_tab_docks_beside_a_slot_of_another_popout() {
        // AC13 — merging popouts.
        let Fx { mut mgr, p, q, .. } = fixture();
        let anchor = slots_of(&mgr, p[1])[1];
        let moved = slots_of(&mgr, q);
        let out = docked(
            &mut mgr,
            q,
            seam(p[1], DockAnchor::Slot(anchor), DockSide::Right),
            FocusPolicy::Reveal,
        );
        assert_eq!(
            out,
            Docked {
                window: P1.to_string(),
                slot_ids: moved.clone(),
                source_window: P2.to_string(),
                emptied: Some(P2.to_string()),
                main_refilled: None,
            }
        );
        assert!(!mgr.windows.contains_key(P2));
        assert_eq!(mgr.windows[P1].tabs, vec![p[0], p[1]]);
        assert_eq!(mgr.windows[P1].active, p[1]);
        let now = slots_of(&mgr, p[1]);
        assert_eq!(now.len(), 3);
        assert!(moved.iter().all(|s| now.contains(s)));
    }

    #[test]
    fn reveal_focuses_the_first_slot_of_a_split_source_not_of_the_result() {
        let Fx { mut mgr, m, p, .. } = fixture();
        let source = slots_of(&mgr, p[1]);
        assert_eq!(source.len(), 2, "premise: a source with two areas");
        docked(
            &mut mgr,
            p[1],
            seam(m[2], DockAnchor::Outer, DockSide::Right),
            FocusPolicy::Reveal,
        );
        let result = slots_of(&mgr, m[2]);
        assert_ne!(
            result[0], source[0],
            "premise: the first area of the result is on the target side"
        );
        assert_eq!(mgr.views[&m[2]].focused_slot_id, Some(source[0]));
    }

    #[test]
    fn a_refused_dock_of_the_main_last_tab_makes_no_empty_tab() {
        let mut mgr = ViewManager::new();
        let only = main_active(&mgr);
        mgr.create_window(P1).unwrap();
        let bogus = Uuid::new_v4();
        refused(
            &mut mgr,
            only,
            tab_bar("no-such", None),
            DockError::WindowNotFound("no-such".into()),
        );
        refused(
            &mut mgr,
            only,
            seam(bogus, DockAnchor::Outer, DockSide::Left),
            DockError::TabNotFound(bogus),
        );
        refused(
            &mut mgr,
            only,
            new_window(P1),
            DockError::WindowExists(P1.into()),
        );
        assert_eq!(mgr.windows[MAIN_WINDOW_LABEL].tabs, vec![only]);
        assert_invariants(&mgr);
    }

    #[test]
    fn the_main_last_tab_leaving_for_another_window_refills_main_with_an_empty_tab() {
        // PRD R14 · ADR-0057 invariant 4 — within the same one go (one version +1) an empty tab appears in main and becomes active.
        for focus in [FocusPolicy::Keep, FocusPolicy::Reveal] {
            for mode in ["seam", "tab_bar", "new_window"] {
                let mut mgr = ViewManager::new();
                let p0 = mgr.create_window(P1).unwrap();
                let (only, _) = foreign_root(&mut mgr);
                let moved = mgr.views[&only].clone();
                let (to, window) = match mode {
                    "seam" => (seam(p0, DockAnchor::Outer, DockSide::Left), P1),
                    "tab_bar" => (tab_bar(P1, None), P1),
                    _ => (new_window(FRESH), FRESH),
                };
                let out = docked(&mut mgr, only, to, focus);
                let main = &mgr.windows[MAIN_WINDOW_LABEL];
                assert_eq!(main.tabs.len(), 1, "{mode} {focus:?}");
                let blank = main.tabs[0];
                assert_eq!(
                    out,
                    Docked {
                        window: window.to_string(),
                        slot_ids: tree::slot_ids(&moved.layout),
                        source_window: MAIN_WINDOW_LABEL.to_string(),
                        emptied: None,
                        main_refilled: Some(blank),
                    },
                    "{mode} {focus:?}"
                );
                assert_ne!(blank, only, "a new empty View: {mode} {focus:?}");
                assert_eq!(
                    main.active, blank,
                    "the empty tab is active: {mode} {focus:?}"
                );
                assert_eq!(mgr.views[&blank].name, TabName::Auto);
                assert!(matches!(
                    mgr.views[&blank].layout,
                    LayoutNode::Slot {
                        content: SlotContent::Empty,
                        ..
                    }
                ));
                assert!(!mgr.unknown_content.contains_key(&blank));
                if mode == "seam" {
                    assert!(!mgr.views.contains_key(&only));
                    assert!(
                        mgr.unknown_content.contains_key(&p0),
                        "the side table goes to the target"
                    );
                } else {
                    assert_eq!(mgr.views[&only], moved, "{mode} {focus:?}");
                    assert_eq!(
                        mgr.owner_of(only).map(String::as_str),
                        Some(window),
                        "{mode} {focus:?}"
                    );
                    assert!(
                        mgr.unknown_content.contains_key(&only),
                        "the side table goes with the View"
                    );
                }
                assert_invariants(&mgr);
            }
        }
    }

    // ── Next active · attaching share (ADR-0295 · ADR-0297) ──────────────

    #[test]
    fn the_source_window_takes_the_given_successor_when_it_remains() {
        // P1 [p0, p1, p2] · active p1 — a succession would be the right neighbor p2.
        let setup = || {
            let Fx { mut mgr, m, p, .. } = fixture();
            let p2 = mgr.create_tab(P1, None).unwrap();
            mgr.switch_tab(P1, p[1]).unwrap();
            assert_eq!(succession(&mgr.windows[P1].tabs, p[1]), Some(p2), "premise");
            (mgr, m[0], [p[0], p[1], p2])
        };
        type Pick = fn(&[ViewId; 3], ViewId) -> Option<ViewId>;
        let cases: [(Pick, usize, &str); 4] = [
            (|p, _| Some(p[0]), 0, "a remaining tab is used"),
            (|_, _| None, 2, "none means succession"),
            (
                |p, _| Some(p[1]),
                2,
                "the moved tab itself means succession",
            ),
            (
                |_, m0| Some(m0),
                2,
                "a tab of another window means succession",
            ),
        ];
        for (pick, want, why) in cases {
            for mode in ["tab_bar", "new_window", "seam"] {
                let (mut mgr, m0, p) = setup();
                let to = match mode {
                    "tab_bar" => tab_bar(MAIN_WINDOW_LABEL, None),
                    "new_window" => new_window(FRESH),
                    _ => seam(m0, DockAnchor::Outer, DockSide::Left),
                };
                let before = mgr.version;
                mgr.dock(
                    p[1],
                    to,
                    FocusPolicy::Reveal,
                    DockMove {
                        successor: pick(&p, m0),
                        ..DockMove::default()
                    },
                )
                .unwrap();
                assert_eq!(mgr.version, before + 1);
                assert_eq!(mgr.windows[P1].active, p[want], "{why} — {mode}");
                assert_invariants(&mgr);
            }
        }

        // If the moved tab was not active, that window's active tab stays.
        let (mut mgr, _, p) = setup();
        mgr.dock(
            p[0],
            tab_bar(MAIN_WINDOW_LABEL, None),
            FocusPolicy::Reveal,
            DockMove {
                successor: Some(p[2]),
                ..DockMove::default()
            },
        )
        .unwrap();
        assert_eq!(mgr.windows[P1].active, p[1]);
    }

    // ADR-0297 — the `initial` of a drop is the tab of the window that started the drag. If the dragged tab went to another window meanwhile, that window follows succession.
    #[test]
    fn the_given_successor_is_taken_only_while_the_tab_is_in_its_successor_window() {
        // P1 [p0, p1, p2] · active p1 — a succession would be the right neighbor p2.
        for (window, want) in [(Some(P1), 0), (None, 0), (Some(MAIN_WINDOW_LABEL), 2)] {
            let Fx { mut mgr, p, .. } = fixture();
            let p2 = mgr.create_tab(P1, None).unwrap();
            mgr.switch_tab(P1, p[1]).unwrap();
            let p = [p[0], p[1], p2];
            mgr.dock(
                p[1],
                tab_bar(MAIN_WINDOW_LABEL, None),
                FocusPolicy::Reveal,
                DockMove {
                    successor: Some(p[0]),
                    successor_window: window.map(str::to_owned),
                },
            )
            .unwrap();
            assert_eq!(mgr.windows[P1].active, p[want], "{window:?}");
            assert_invariants(&mgr);
        }
    }

    #[test]
    fn a_seam_share_outside_its_range_is_refused_by_value() {
        let Fx { mut mgr, m, p, .. } = fixture();
        let at = |share: f64| DockTo::Seam {
            target: m[0],
            anchor: DockAnchor::Outer,
            side: DockSide::Left,
            share,
        };
        for share in [
            f64::NAN,
            f64::INFINITY,
            f64::NEG_INFINITY,
            0.0,
            tree::RATIO_MIN / 2.0,
            0.6,
        ] {
            refused(
                &mut mgr,
                p[1],
                at(share),
                DockError::InvalidShare(share.to_string()),
            );
        }
        // Decision order — before area 0.
        plant(&mut mgr, m[1], planted_half_chain(40));
        plant(&mut mgr, p[0], planted_half_chain(20));
        let deepest = *slots_of(&mgr, m[1]).last().unwrap();
        let to = |share: f64| DockTo::Seam {
            target: m[1],
            anchor: DockAnchor::Slot(deepest),
            side: DockSide::Right,
            share,
        };
        refused(
            &mut mgr,
            p[0],
            to(dock::DOCK_NEW_SHARE),
            DockError::ZeroArea,
        );
        refused(
            &mut mgr,
            p[0],
            to(f64::NAN),
            DockError::InvalidShare("NaN".into()),
        );
    }

    #[test]
    fn a_seam_share_at_either_end_of_its_range_docks() {
        for share in [tree::RATIO_MIN, 0.5] {
            let Fx { mut mgr, m, p, .. } = fixture();
            docked(
                &mut mgr,
                p[1],
                DockTo::Seam {
                    target: m[0],
                    anchor: DockAnchor::Outer,
                    side: DockSide::Right,
                    share,
                },
                FocusPolicy::Keep,
            );
            assert_eq!(
                tree::split_ratio(&mgr.views[&m[0]].layout, split_ids(&mgr, m[0])[0]),
                Some(1.0 - share),
                "{share}"
            );
        }
    }
}
