//! The layout apply service — the transport-neutral single path where human clicks (Tauri invoke) and relayed LLM calls land on **the same function**
//! (ADR-0081 decision 3). The shape ADR-0081 「Rejected alternatives」 rejected = the caller locking `ViewManager`
//! directly and calling it (the human path and the LLM path split into 2 code paths) — this module is the only alternative to that.
//!
//! ★0 Tauri dependencies★: all are free functions with `AppHandle` · `State<..>` taken out of the arguments — because standalone calls ·
//! tests must work without a window (TRD S20 §7 「shell apply service」). It reaches the outside world (frontend notices · OS windows · daemon round trips ·
//! subscription tables) only through the 5 ports below. The `#[tauri::command]` wrapper is `commands/layout.rs`.
//!
//! ## ★Lock discipline★
//! Write functions take the ViewManager lock **briefly to mutate + copy the needed data (snapshots · tab lists)** and notify **after
//! dropping the lock**. The lock drop is enforced by scope (`{ ... }`). The only port called while holding the lock is
//! `SubscriptionSync` (synchronous · non-blocking contract), and the other four (`LayoutEvents` · `WindowHost` · `LabelSource` ·
//! `AgentSpawner`) are always outside the lock. **The two rules have different grounds:**
//! - **0 external calls while holding the lock** (notices · OS windows · daemon round trips · await) = ADR-0006's principle ("no external calls while holding a
//!   lock") applied to this lock. ★That ADR's body only covers the core's sessions/status locks — do not look there for the layout lock or
//!   subscription delta clauses.★
//! - **Only the subscription resync inside the lock** = grounded not in an ADR but in the call contract of `output_router::rebuild` (RMW serialization of
//!   load→compute→store) and the F1/F2 fixes on top of it. Canonical = that function's comment.
//!
//! invalid view_id/slot_id/window → no-op + Err (no panic · no partial change). Errors are strings, and the ones whose kind can be told apart are
//! four: docking (`dock_tab`)'s `DockError` · target-version docking (`dock_target`)'s `TargetDockError` · target-version area · tab writes (`*_at`)'s
//! `TargetError` · word-version spawn (`spawn_into_at`)'s `SpawnAtError`.
//!
//! ## The ports a function receives = what that command touches
//! A port absent from the arguments **means that path does not touch it** — `focus_slot` · `rename_tab` ·
//! `set_split_ratio` lacking `SubscriptionSync` is not an omission but an expression of routing invariance (ADR-0066), and
//! `close_window` lacking `LayoutEvents` is the fact that that path fires nothing at the frontend.
//!
//! The 8 read-only ones (`get_view` · `list_tabs` · `list_windows` · `resolve_spatial` · `list_splits` · `layout_rows` ·
//! `layout_rows_of` · `agent_places`) have no mutation and take no port at all (the first four joined the v1 command scope by ADR-0156 decision 2
//! · `list_splits` by ADR-0227 · the last three by TRD S28 §4-5).
//!
//! The 2 measurement reports (`report_window_canvas` · `report_ui_metrics`) take no port either — they write to shell
//! state but touch none of version · notices · subscriptions (measurement, not control — ADR-0227).
//!
//! ## Target versions (`*_at` · `dock_target`)
//! Beside the writes taking ids there are versions taking a [`Target`] (agent · spatial word · id) — LLM commands use them (TRD S28 §5-2). The target is
//! resolved **inside the same lock that changes it** (`target::locate`), so a write cutting in between resolving and applying cannot make it change the wrong tab.
//! Target versions that change the layout also capture the layout rows of the changed tab in that lock and put them in the outcome ([`Wrote`] · [`DockOutcome::rows`]) (TRD S28 §7).
//! The id versions are unchanged — mouse · web · drop commands call them. The lock · port discipline of the two versions is the same.

use std::fmt::Display;
use std::future::Future;
use std::pin::Pin;

use engram_dashboard_base::sync;
use engram_dashboard_protocol::AgentBackendKind;
use uuid::Uuid;

use super::dock::{DockAnchor, DockSide};
use super::manager::{
    CloseTabOutcome, DockError, DockMove, DockTo, Docked, FocusPolicy, NewWindowPlan, ViewId,
    ViewManager, WindowLabel, WindowTabsSnapshot, MAIN_WINDOW_LABEL,
};
use super::rows::{self, LayoutRow, RowFilter};
use super::spatial::{resolve_spatial as resolve_spatial_token, SpatialToken};
use super::tab_name::window_title;
use super::target::{self, locate, seam_share, LocateError, Located, Place, Target};
use super::tree::{self, SplitInfo};
use super::types::{
    SlotContent, SplitDir, SplitRatioApplied, SplitRatioOutcome, UiMetrics, ViewMeta, ViewSnapshot,
};
use super::LayoutState;
use crate::ui_text::Lang;

/// Per-window tab bar notice payload (ADR-0057). The frontend reacts only when `label` matches its own window (§7-1) and discards stale notices by `version`. Field names are the wire contract; the frontend reads by these names.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct WindowTabsPayload {
    pub label: String,
    /// That window's persisted identity — main = `main`, popout = UUID. Unlike the label, it is the same window across runs.
    pub window_id: String,
    /// Window number — main = 1, popouts ≥ 2. Does not change while the window entry is in the model. (TRD S26-window-chrome §4-1)
    pub number: u32,
    /// That window's OS title, built from the active tab's visible name and the number. The web does not read it; the shell's notice adapter applies it to the OS window together with `version` (`crate::chrome`). (TRD S26-window-chrome §4-4)
    pub title: String,
    pub tabs: Vec<ViewMeta>,
    pub active: Uuid,
    pub version: u64,
}

impl From<WindowTabsSnapshot> for WindowTabsPayload {
    fn from(s: WindowTabsSnapshot) -> Self {
        let active_name = s
            .tabs
            .iter()
            .find(|meta| meta.id == s.active)
            .map(|meta| meta.name.as_str());
        let title = window_title(active_name, Some(s.number));
        WindowTabsPayload {
            label: s.label,
            window_id: s.window_id,
            number: s.number,
            title,
            tabs: s.tabs,
            active: s.active,
            version: s.version,
        }
    }
}

/// A port that bundles the routing table recompute + firing the unsubscribe of agents no longer visible in any window into one action.
///
/// ★Call convention★: the apply service calls it **while holding the ViewManager lock**; the recompute and the firing must be in one critical section for "compute order = firing order" to hold, or a stale unsubscribe kills an agent re-added in the gap (F1/F2). The ground is the call contract of `output_router::rebuild` (RMW serialization), canonical in that function's comment, not an ADR clause (ADR-0006 has no subscription delta clause). The implementation must be synchronous and non-blocking (no await · network).
///
/// ★The recompute of the usage interest derived from the layout happens here too★: the writes receiving this port are exactly the writes that change slot content · tabs · windows, so every place the interest can change comes here. What that recompute does (in-memory computation · non-blocking nudge · starting a timer task) is within the contract above. (TRD S21 usage-limit-slot §1-7)
pub trait SubscriptionSync: Send + Sync {
    fn resync(&self, mgr: &ViewManager);
}

/// The frontend notice port — the production implementation also applies the tab list's `title` to the OS window (`crate::chrome`). ★Called only while not holding the lock★ (ADR-0006). The implementation swallows failures: a lost notice is no reason to revert a layout mutation (the frontend recovers by a read-only pull; the OS title is fixed by that window's next tab notice). (TRD S26-window-chrome §4-4)
pub trait LayoutEvents: Send + Sync {
    fn layout_updated(&self, snapshot: &ViewSnapshot);
    fn window_tabs_updated(&self, tabs: &WindowTabsPayload);
}

/// Where the window port places a window (a new window · the window to move in docking).
///
/// - `pos` = **physical** outer top-left (the unit of `outer_position`; a logical position points at two monitors with one value when scales differ — `state::placement` head). `None` = no position picked.
/// - `size` = **logical** inner size. `None` = no size picked. (TRD S21-slot-docking §3-3 · §3-5)
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WindowPlace {
    pub pos: Option<(i32, i32)>,
    pub size: Option<(f64, f64)>,
}

/// One setter step of the window port, applied in this order (position → size).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoveStep {
    Position,
    Size,
}

/// The step that could not be applied and its reason (the sentence the OS or runtime gave).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MoveFailed {
    pub step: MoveStep,
    pub error: String,
}

/// The claim ticket of a hidden prewarm window the window port handed over to one caller. [`WindowHost::claim_spare`] gives it and the caller returns it once with [`WindowHost::committed`] or [`WindowHost::release_spare`] (if `open` · `open_at` of the handed-over window is `Err`, neither is called — the window port already reaped it).
///
/// - `label` = the handed-over window's label; the new window opens with it (no [`LabelSource::next_label`] call).
/// - `generation` = the adoption number drawn anew at every handover. The window port listens only when the ticket matches the current handover in both label and number, so if the same label was handed over again after being taken back, the earlier ticket is stale and a no-op.
// I27
// ADR-0299 (TRD S21-slot-docking §3-9)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpareClaim {
    pub label: WindowLabel,
    pub generation: u32,
}

/// The OS window host port. ★All called only outside the lock★ — window creation · setters · getters are OS calls, and moving `open` · `record_placement` · `close` · `committed` inside the lock deadlocks right there.
///
/// - `open`: the window build requires the event loop · lock.
/// - `record_placement`: the implementation calls the window getters (OS calls) and then takes the same ViewManager lock to record.
/// - `committed`: the implementation takes the same ViewManager lock and moves the measurements the handed-over window holds into the model.
/// - `close`: when the implementation destroys the OS window, that window's `Destroyed` handler takes **the same ViewManager lock** again (`popout::destroy_window` → `Destroyed` → `cleanup_popup_window` → `state.0.lock()`); a worker thread calling it holding the guard makes destroy wait for the event loop while the event loop waits for the guard. An already closed label is a no-op.
/// - `is_open`: whether the **OS window** is alive, not the model. The two can diverge (the OS window vanished without a close request but the `Destroyed` cleanup has not removed the model entry yet); only this answer filters that moment, and the model re-validation of `insert_tab_into` lets that state pass.
///
/// Pop-out releases the lock once between the window check and the model mutation. No debris remains if the user closes the target window in that gap because a popout's close request first removes the window from the model and tombstones the label in one lock (`commands::popout::close_requested_popup_window`; `window.close` of a window outside the model does the same — `window_ctl::close`). An existing-window target then fails at phase C's `insert_tab_into`, a new-window target at `attach_view_as_new_window`, and both roll back (the slot to move stays at the source). That premise (a popout's close request is not reverted) and its check command are in the `CloseRequested` handler of `lib.rs`.
///
/// ★No default body for `open_at` · `move_window` · the three prewarm ones (`claim_spare` · `committed` · `release_spare`)★: the forwarding adapter (`OwnedWindowHost`) would inherit it and silently drop position · size · activation · prewarm windows. (TRD S21-slot-docking §3-9; TRD S21-slot-docking 5.8th edition §1-2)
pub trait WindowHost: Send + Sync {
    fn open(&self, label: &str) -> Result<(), String>;
    /// Like [`Self::open`] but picks position · activation. `at.pos` absent or overlapping no monitor → the label's default position; `at.size` absent → the default popout size. `activate = false` = create without taking focus. `Err` = the window could not be created, or its picked position could not be applied and the window was reaped; either way no window is up and the caller ends without changing the model.
    fn open_at(&self, label: &str, at: &WindowPlace, activate: bool) -> Result<(), String>;
    fn close(&self, label: &str);
    fn is_open(&self, label: &str) -> bool;
    /// Moves an up window to `at`: position → size, ★stops at the first failure★ and returns that step. An empty field or a position overlapping no monitor skips that step. Overlap is measured with the size the window will have after moving (`at.size`, else its normal size). A missing window fails the position step. For a maximized window the position step unmaximizes: a visible window shows at normal size, a hidden one flickers briefly with `SW_RESTORE` then `SW_HIDE` and stays hidden (tao 0.35.3 `platform_impl/windows/window_state.rs` `apply_diff`; not GUI-verified). On failure the window may be half moved or unmaximized and there is no port to revert it (reuse docking leaves it where it was dropped — [`dock_tab`]).
    fn move_window(&self, label: &str, at: &WindowPlace) -> Result<(), MoveFailed>;
    /// Called once after a window opened with `open` · `open_at` enters the model; records its first position in the model (so even a never-dragged window opens there at the next boot). Also called once after reuse docking moves a window (`dock_tab`). Default = does nothing. (TRD S21-storage §6-3)
    fn record_placement(&self, _label: &str) {}
    /// Asked before opening a window with a new label: a hidden prewarm window handed over to this caller, or `None` (build a new one). `open` · `open_at` of that label then place that window instead of building (same `Err` contract, after reaping it); the window stays hidden outside the model meanwhile. ★Do not call from the main thread★: it waits for a prewarm window being built, and that build runs on the main thread. (TRD S21-slot-docking §3-9)
    fn claim_spare(&self) -> Option<SpareClaim>;
    /// The handed-over window's entry entered the model and its notice was emitted, so it may now draw and show. Called before `record_placement`, and even if that label left the model meanwhile (closing it is the implementation's job). No-op for a stale ticket.
    fn committed(&self, claim: &SpareClaim);
    /// Takes back the handed-over window before the confirm (a rollback that could not insert into the model); it can be handed over again while hidden. No-op for a stale ticket. Rolling back a newly built window is [`Self::close`], which always closes, even for a prewarm window label.
    fn release_spare(&self, claim: &SpareClaim);
}

/// The new window label issuing port. ★Monotonic★: reusing a closed label makes a window of that label impossible to create again. Split from `WindowHost` because paths that only close windows (`close_tab` · `close_window`) should not depend on the issuer.
///
/// A new window's first tab is not given a name (it is auto-named); building one from the label form would make the OS title · tab bar show "Popup N" instead of the slot content.
///
/// ★Called only outside the lock★. Do not rely on today's implementation being pure: an implementation touching state inside the lock deadlocks with `Destroyed → cleanup_popup_window → state.0.lock()`. (TRD S26-window-chrome §3-1)
pub trait LabelSource: Send + Sync {
    fn next_label(&self) -> String;
}

/// Agent spawn port — creates a new agent for `cwd` and starts it. ★Awaited with no lock held★ (a daemon round trip).
///
/// The refusal goes out to the caller as is (fail-loud). Reading the reply (which frame is success) is the
/// implementation's job — that is a transport contract this service does not know.
///
/// `backend` is **the wire vocabulary as is**: a vocabulary of its own here could let the word the shell passes and
/// the word the daemon knows drift apart, and that drift shows only as a different backend starting silently.
/// `None` = not chosen — **the daemon refuses it** (the shell invents no default).
pub trait AgentSpawner: Send + Sync {
    fn spawn_by_cwd<'a>(
        &'a self,
        cwd: String,
        backend: Option<AgentBackendKind>,
    ) -> Pin<Box<dyn Future<Output = Result<SpawnedAgent, SpawnRefusal>> + Send + 'a>>;
}

/// A started agent. `notices` = the daemon's launch notice rows (`AgentEvent::Spawned.notices`, `[]` when absent),
/// passed on undecoded — the shell does not know the agent types (ADR-0270).
// ADR-0348
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpawnedAgent {
    pub id: String,
    pub notices: Vec<serde_json::Value>,
}

/// Why a spawn-and-place did not end placed. `message` is the whole text (`Display`); the other fields are the
/// structured part a bus answer carries as `args`:
/// - `launch_blocked` = the daemon's `{"reasons": […]}` object, verbatim;
/// - `created_agent_id` = the daemon added this agent to the roster and it did not start — it is asleep and not placed;
/// - `live_agent_id` = a spawn succeeded before the placement failed — this agent runs and is not placed;
/// - `notices` = the rows of that spawn — the daemon may have recorded them as shown, so they must reach the caller
///   with the failure.
///
/// A refusal of the shell's own (validation, placement) has only `message` and, after a spawn, `live_agent_id` and
/// `notices`.
// ADR-0348
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct SpawnRefusal {
    pub message: String,
    pub launch_blocked: Option<serde_json::Map<String, serde_json::Value>>,
    pub created_agent_id: Option<engram_dashboard_protocol::AgentId>,
    pub live_agent_id: Option<String>,
    pub notices: Vec<serde_json::Value>,
}

impl From<String> for SpawnRefusal {
    fn from(message: String) -> Self {
        SpawnRefusal {
            message,
            launch_blocked: None,
            created_agent_id: None,
            live_agent_id: None,
            notices: Vec::new(),
        }
    }
}

// Window already gone → None → skip the tab notice (e.g. closing a popup's last tab just removed the entry).
fn tabs_payload(mgr: &ViewManager, label: &str) -> Option<WindowTabsPayload> {
    mgr.list_tabs(label).ok().map(Into::into)
}

fn owner_tabs(mgr: &ViewManager, view_id: Uuid) -> Option<WindowTabsPayload> {
    mgr.owner_of(view_id)
        .cloned()
        .and_then(|label| tabs_payload(mgr, &label))
}

fn notify(
    events: &dyn LayoutEvents,
    layout: Option<ViewSnapshot>,
    tabs: Option<WindowTabsPayload>,
) {
    announce(events, layout.as_ref(), tabs.as_ref());
}

/// Emits the notice bundle of one write: layout snapshots → window tab lists. ★Call only after releasing the lock★. Every notifying write passes through here, the restore accept (`state::restore`) too.
pub(crate) fn announce<'a>(
    events: &dyn LayoutEvents,
    layouts: impl IntoIterator<Item = &'a ViewSnapshot>,
    tabs: impl IntoIterator<Item = &'a WindowTabsPayload>,
) {
    for snapshot in layouts {
        events.layout_updated(snapshot);
    }
    for payload in tabs {
        events.window_tabs_updated(payload);
    }
}

// ── The handover part of the three new-window places ─────────────
//
// The three places that open a window with a new label (`create_window` · `move_slot_to_window` · docking new window) ask the window port for a prewarm window first, and if one was handed over carry that claim ticket to the end, either confirm (`committed`) or take back (`release_spare`) exactly once. If the window's open is `Err`, neither is called (the window port already reaped it — `SpareClaim` docs).
// ADR-0299

// The new window's label: the handed-over prewarm window's if any, otherwise a new one from the issuer. (TRD S21-slot-docking §3-9)
fn new_window_label(
    host: &dyn WindowHost,
    labels: &dyn LabelSource,
) -> (WindowLabel, Option<SpareClaim>) {
    match host.claim_spare() {
        Some(claim) => (claim.label.clone(), Some(claim)),
        None => (labels.next_label(), None),
    }
}

// The opened window could not be put into the model: a handed-over prewarm window is taken back and a newly built window is closed. ★Do not reap the handed-over window with `close`★: it always closes, discarding a sound prewarm window and forcing a rebuild (take back only with the claim ticket). (TRD §3-9 I27)
fn withdraw_new_window(host: &dyn WindowHost, label: &str, claim: Option<&SpareClaim>) {
    match claim {
        Some(claim) => host.release_spare(claim),
        None => host.close(label),
    }
}

// The new window entry entered the model and was notified: if it is a handed-over window, confirm it, then record the first position (the first `window:frame` comes after the confirm). (TRD §3-8)
fn settle_new_window(host: &dyn WindowHost, label: &str, claim: Option<&SpareClaim>) {
    if let Some(claim) = claim {
        host.committed(claim);
    }
    host.record_placement(label);
}

/// One newly opened window (or a window that received a tab) — the answer of invoke `create_window`, and the bus `window.create` · `slot.popout` and the docking
/// outcome carry the same three (TRD S26-window-chrome §3-2 · §5-3).
///
/// - `window` = label — valid only within that run.
/// - `window_id` = persisted id (main = `main` · popout = UUID) — the same window across restarts, and the value to pass as the window argument.
/// - `number` = the visible window number (main = 1 · popouts ≥ 2) — closed windows' numbers are reused, so it is not used to point at a window
///   (given as a window argument, it is refused).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, ts_rs::TS)]
#[ts(export)]
pub struct NewWindow {
    pub window: String,
    pub window_id: String,
    pub number: u32,
}

impl NewWindow {
    // The identity fields of the tab list payload, from what was captured inside the lock that inserted it.
    fn of(tabs: &WindowTabsPayload) -> Self {
        NewWindow {
            window: tabs.label.clone(),
            window_id: tabs.window_id.clone(),
            number: tabs.number,
        }
    }

    // The identity of window `label` read directly from the window entry, without capturing the tab list payload; `None` if not in the model.
    fn in_model(mgr: &ViewManager, label: &str) -> Option<Self> {
        mgr.windows.get(label).map(|w| NewWindow {
            window: label.to_string(),
            window_id: w.window_id.clone(),
            number: w.number,
        })
    }
}

// Builds the answer from the tab list captured inside the lock that inserted the window (or gave it the tab). It was inserted inside that lock so the list always exists; if not, only the answer is emitted as a failure (insertion and notice already happened).
fn window_answer(label: &str, tabs: Option<&WindowTabsPayload>) -> Result<NewWindow, String> {
    tabs.map(NewWindow::of).ok_or_else(|| {
        tracing::warn!(
            label,
            "new window answer — the inserted window has no tab list"
        );
        format!(
            "window not found: {label} — could not read that window's tab list after inserting it"
        )
    })
}

// ── Writes ─────────────────────────────────────────────────────────────────────

pub fn create_tab(
    state: &LayoutState,
    subs: &dyn SubscriptionSync,
    events: &dyn LayoutEvents,
    window: &str,
    name: Option<String>,
) -> Result<Uuid, String> {
    let (id, layout, tabs) = {
        let mut mgr = state.0.lock().map_err(|e| e.to_string())?;
        let id = mgr.create_tab(window, name).map_err(|e| e.to_string())?;
        let layout = mgr.snapshot(id).ok();
        let tabs = tabs_payload(&mgr, window);
        subs.resync(&mgr);
        (id, layout, tabs)
    }; // ← lock dropped
    notify(events, layout, tabs);
    Ok(id)
}

// An empty new window (1 empty tab) + an OS window. On success: the new window's label · persisted id · number.
//
// ★Window count cap proximity log (ADR-0056 · §4-2)★: approaching the visible slot cap (≤16) is only logged, not a hard block (an exceeding layout = frontend onContextLoss → DOM graceful degrade + ADR-0056 review trigger).
//
// ★Notifies the tab list of the inserted new window★: the web pulls and draws on mount (§3-3), but the OS window title is applied only through this notice (TRD S26-window-chrome §2-4 ② · §4-4). Confirm · first position record follow the notice (same order as the other two new-window places).
//
// ★Inserts into the model after opening the OS window★ (same order as `move_slot_to_window` · docking new window — TRD S21-slot-docking §3-9 「order per position」). If the window closes between opening and inserting (close request · `Destroyed`), that cleanup tombstones the label, the insert is rejected as `WindowClosed`, and the opened window is reaped (no-op if already closed).
// ADR-0299 (TRD S26-window-chrome §5-3)
pub fn create_window(
    state: &LayoutState,
    subs: &dyn SubscriptionSync,
    events: &dyn LayoutEvents,
    host: &dyn WindowHost,
    labels: &dyn LabelSource,
) -> Result<NewWindow, String> {
    let (label, claim) = new_window_label(host, labels);
    host.open(&label)?;

    let inserted = state
        .0
        .lock()
        .map_err(|e| e.to_string())
        .and_then(|mut mgr| {
            mgr.create_window(&label).map_err(|e| e.to_string())?;
            subs.resync(&mgr);
            Ok((mgr.windows.len(), tabs_payload(&mgr, &label)))
        });
    let (n_windows, tabs) = match inserted {
        Ok(inserted) => inserted,
        Err(e) => {
            withdraw_new_window(host, &label, claim.as_ref());
            tracing::warn!(label = %label, "create_window: could not put the opened window into the model; reaping it — {e}");
            return Err(e);
        }
    };
    if n_windows >= 3 {
        tracing::info!(
            windows = n_windows,
            "create_window: window count grew — watch the visible slot cap (<=16, ADR-0056)"
        );
    }
    announce(events, None::<&ViewSnapshot>, tabs.as_ref());
    settle_new_window(host, &label, claim.as_ref());

    tracing::info!(label = %label, "empty new window created (create_window)");
    window_answer(&label, tabs.as_ref())
}

// keep-alive keeps the exposed set unchanged (only the active mark changes), but by contract it is re-synced.
pub fn switch_tab(
    state: &LayoutState,
    subs: &dyn SubscriptionSync,
    events: &dyn LayoutEvents,
    window: &str,
    view: Uuid,
) -> Result<(), String> {
    let (layout, tabs) = {
        let mut mgr = state.0.lock().map_err(|e| e.to_string())?;
        mgr.switch_tab(window, view).map_err(|e| e.to_string())?;
        let layout = mgr.snapshot(view).ok();
        let tabs = tabs_payload(&mgr, window);
        subs.resync(&mgr);
        (layout, tabs)
    };
    notify(events, layout, tabs);
    Ok(())
}

/// [`switch_tab`] only when that window's active tab is `expect`: `true` if it switched (notification and re-sync as `switch_tab`); if the active tab differs, `false` and nothing in the model, subscriptions or notifications is touched. Errors (window not found · a `view` that is not that window's tab) are as `switch_tab`.
///
/// A web → shell command only for restoring the original selection after a tab drag, so the restore does not overwrite a tab the user or LLM chose before it arrived. Not in the LLM bus table (the LLM uses `tab.switch`). (TRD S21-slot-docking §4-6; TRD §3-6)
pub fn switch_tab_if(
    state: &LayoutState,
    subs: &dyn SubscriptionSync,
    events: &dyn LayoutEvents,
    window: &str,
    view: Uuid,
    expect: Uuid,
) -> Result<bool, String> {
    let (layout, tabs) = {
        let mut mgr = state.0.lock().map_err(|e| e.to_string())?;
        if !mgr
            .switch_tab_if(window, view, expect)
            .map_err(|e| e.to_string())?
        {
            drop(mgr);
            tracing::debug!(
                window,
                view = %view,
                expect = %expect,
                "switch_tab_if: the active tab is not expect; not switching"
            );
            return Ok(false);
        }
        let layout = mgr.snapshot(view).ok();
        let tabs = tabs_payload(&mgr, window);
        subs.resync(&mgr);
        (layout, tabs)
    };
    notify(events, layout, tabs);
    Ok(true)
}

// ★Closing the OS window when the last tab's close makes the window vanish is single-sourced here (§5-2)★: `close_tab` already erased the `windows` entry. The later Destroyed event cleans the registry/Channel remains; no separate `view:closed` is fired to the frontend (avoids double firing).
pub fn close_tab(
    state: &LayoutState,
    subs: &dyn SubscriptionSync,
    events: &dyn LayoutEvents,
    host: &dyn WindowHost,
    window: &str,
    view: Uuid,
) -> Result<(), String> {
    let (outcome, layout, tabs) = {
        let mut mgr = state.0.lock().map_err(|e| e.to_string())?;
        let outcome = mgr.close_tab(window, view).map_err(|e| e.to_string())?;
        let (layout, tabs) = match outcome {
            CloseTabOutcome::Stayed => {
                let active = mgr.list_tabs(window).ok().map(|s| s.active);
                let layout = active.and_then(|a| mgr.snapshot(a).ok());
                (layout, tabs_payload(&mgr, window))
            }
            CloseTabOutcome::WindowClosed => (None, None),
        };
        subs.resync(&mgr);
        (outcome, layout, tabs)
    };

    match outcome {
        CloseTabOutcome::Stayed => notify(events, layout, tabs),
        CloseTabOutcome::WindowClosed => host.close(window),
    }
    Ok(())
}

// No main-window refusal guard here: the model (`ViewManager`) is the SSOT, so its Err is passed on as a string.
pub fn close_window(
    state: &LayoutState,
    subs: &dyn SubscriptionSync,
    host: &dyn WindowHost,
    window: &str,
) -> Result<(), String> {
    {
        let mut mgr = state.0.lock().map_err(|e| e.to_string())?;
        close_window_in_model(&mut mgr, subs, window)?;
    }
    host.close(window);
    Ok(())
}

/// The model part of [`close_window`]: called by the party holding the layout lock; its OS window is reaped after the lock is released ([`WindowHost::close`]).
pub(crate) fn close_window_in_model(
    mgr: &mut ViewManager,
    subs: &dyn SubscriptionSync,
    window: &str,
) -> Result<(), String> {
    mgr.close_window(window).map_err(|e| e.to_string())?;
    subs.resync(mgr);
    Ok(())
}

pub fn split_slot(
    state: &LayoutState,
    subs: &dyn SubscriptionSync,
    events: &dyn LayoutEvents,
    view_id: Uuid,
    slot_id: Uuid,
    dir: SplitDir,
) -> Result<Uuid, String> {
    let (new_id, layout, tabs) = {
        let mut mgr = state.0.lock().map_err(|e| e.to_string())?;
        let new_id = mgr
            .split_slot(view_id, slot_id, dir)
            .map_err(|e| e.to_string())?;
        let layout = mgr.snapshot(view_id).ok();
        let tabs = owner_tabs(&mgr, view_id);
        subs.resync(&mgr);
        (new_id, layout, tabs)
    };
    notify(events, layout, tabs);
    Ok(new_id)
}

pub fn close_slot(
    state: &LayoutState,
    subs: &dyn SubscriptionSync,
    events: &dyn LayoutEvents,
    view_id: Uuid,
    slot_id: Uuid,
) -> Result<(), String> {
    let (layout, tabs) = {
        let mut mgr = state.0.lock().map_err(|e| e.to_string())?;
        mgr.close_slot(view_id, slot_id)
            .map_err(|e| e.to_string())?;
        let layout = mgr.snapshot(view_id).ok();
        let tabs = owner_tabs(&mgr, view_id);
        subs.resync(&mgr);
        (layout, tabs)
    };
    notify(events, layout, tabs);
    Ok(())
}

// ★Routing unchanged → no re-sync (so no `SubscriptionSync`)★: moving focus does not change which slot sees which agent (= output routing).
// ADR-0066
pub fn focus_slot(
    state: &LayoutState,
    events: &dyn LayoutEvents,
    view_id: Uuid,
    slot_id: Uuid,
) -> Result<(), String> {
    let (layout, tabs) = {
        let mut mgr = state.0.lock().map_err(|e| e.to_string())?;
        mgr.set_focused_slot(view_id, slot_id)
            .map_err(|e| e.to_string())?;
        let layout = mgr.snapshot(view_id).ok();
        // ★Do not drop the tab notification★: the visible name of an auto-named tab is decided by the focus slot, so moving focus changes the tab bar name and the OS window title. (TRD S26-window-chrome §4-2 · §2-4 ②)
        let tabs = owner_tabs(&mgr, view_id);
        (layout, tabs)
    };
    notify(events, layout, tabs);
    Ok(())
}

// Where a person's divider drag (Tauri `set_split_ratio`) and the LLM's `split.setRatio` land together.
// ★Layout notification only for `Applied`, no tab notification★: announcing an unchanged outcome (`Unchanged` · `TooSmall`) would send the screen a snapshot that did not change. Output routing is unchanged, so no re-sync either (as `focus_slot`).
// ★The returned version is that of the snapshot taken in the same critical section★: re-reading the global counter after releasing the lock could return another write's version, and the screen would wait for a version that is never notified.
// ADR-0227
pub fn set_split_ratio(
    state: &LayoutState,
    events: &dyn LayoutEvents,
    view_id: Uuid,
    split_id: Uuid,
    ratio: f64,
) -> Result<SplitRatioApplied, String> {
    let (applied, layout) = {
        let mut mgr = state.0.lock().map_err(|e| e.to_string())?;
        let result = mgr
            .set_split_ratio(view_id, split_id, ratio)
            .map_err(|e| e.to_string())?;
        let layout = match result.outcome {
            SplitRatioOutcome::Applied => mgr.snapshot(view_id).ok(),
            SplitRatioOutcome::Unchanged | SplitRatioOutcome::TooSmall => None,
        };
        let version = layout.as_ref().map_or(mgr.version, |snap| snap.version);
        let applied = SplitRatioApplied {
            ratio: result.ratio,
            outcome: result.outcome,
            version,
        };
        (applied, layout)
    };
    notify(events, layout, None);
    Ok(applied)
}

// ★Tab notification only★: the name lives only in `ViewMeta.name` (the tab payload), not in `ViewSnapshot`, so no layout snapshot is fired; rename changes neither output routing nor the layout tree, so no re-sync.
// ADR-0057
pub fn rename_tab(
    state: &LayoutState,
    events: &dyn LayoutEvents,
    view_id: Uuid,
    name: String,
) -> Result<(), String> {
    let tabs = {
        let mut mgr = state.0.lock().map_err(|e| e.to_string())?;
        mgr.rename_tab(view_id, name).map_err(|e| e.to_string())?;
        owner_tabs(&mgr, view_id)
    };
    announce(events, None::<&ViewSnapshot>, tabs.as_ref());
    Ok(())
}

/// Overwrites the daemon agent list names `names` (agent id, display name) into the name ledger and notifies, at a new version, only the windows whose auto-named tab's visible name actually changed. If no tab changed, the version stays and nothing is emitted. The name is used as the daemon made it (uniquifying · canonicalizing are the daemon's).
// ★Tab notification only★ (the name is only in the tab payload, as `rename_tab`; routing and tree unchanged).
// ADR-0301 — tab name · OS title (TRD S26-window-chrome §2-4 ② · §4-3)
pub fn agent_names_updated(
    state: &LayoutState,
    events: &dyn LayoutEvents,
    names: &[(String, String)],
) -> Result<(), String> {
    announce_renamed_windows(state, events, |mgr| mgr.update_agent_names(names))
}

/// Makes automatic tab names (and so OS window titles) in `lang` and announces, at a new version, only the windows where an automatic tab's visible name actually changed (same shape as [`agent_names_updated`]). Nothing changed (same language, or only named tabs) = no bump, no notification. Not [`sync_titles`]: that does not bump, so the web and the OS title would drop the payload as not newer.
// ★Tab notifications only★, like `agent_names_updated`.
// ADR-0373
pub fn language_changed(
    state: &LayoutState,
    events: &dyn LayoutEvents,
    lang: Lang,
) -> Result<(), String> {
    announce_renamed_windows(state, events, |mgr| mgr.set_language(lang))
}

// Under the layout lock `rename` changes what automatic tab names are made of and returns the windows whose name changed (the manager already bumped their version); their tab lists are announced after the lock is dropped.
fn announce_renamed_windows(
    state: &LayoutState,
    events: &dyn LayoutEvents,
    rename: impl FnOnce(&mut ViewManager) -> Vec<WindowLabel>,
) -> Result<(), String> {
    let tabs: Vec<WindowTabsPayload> = {
        let mut mgr = state.0.lock().map_err(|e| e.to_string())?;
        rename(&mut mgr)
            .iter()
            .filter_map(|label| tabs_payload(&mgr, label))
            .collect()
    }; // ← the lock is dropped here
    announce(events, None::<&ViewSnapshot>, &tabs);
    Ok(())
}

pub fn assign_agent(
    state: &LayoutState,
    subs: &dyn SubscriptionSync,
    events: &dyn LayoutEvents,
    view_id: Uuid,
    slot_id: Uuid,
    agent_id: String,
) -> Result<(), String> {
    let (layout, tabs) = {
        let mut mgr = state.0.lock().map_err(|e| e.to_string())?;
        mgr.assign_agent(view_id, slot_id, agent_id)
            .map_err(|e| e.to_string())?;
        let layout = mgr.snapshot(view_id).ok();
        let tabs = owner_tabs(&mgr, view_id);
        subs.resync(&mgr);
        (layout, tabs)
    };
    notify(events, layout, tabs);
    Ok(())
}

pub fn set_slot_content(
    state: &LayoutState,
    subs: &dyn SubscriptionSync,
    events: &dyn LayoutEvents,
    view_id: Uuid,
    slot_id: Uuid,
    content: SlotContent,
) -> Result<(), String> {
    let (layout, tabs) = {
        let mut mgr = state.0.lock().map_err(|e| e.to_string())?;
        mgr.set_slot_content(view_id, slot_id, content)
            .map_err(|e| e.to_string())?;
        let layout = mgr.snapshot(view_id).ok();
        let tabs = owner_tabs(&mgr, view_id);
        subs.resync(&mgr);
        (layout, tabs)
    };
    notify(events, layout, tabs);
    Ok(())
}

/// Makes a slot a usage view; a missing field (`None`) is the current value if the slot is already `Usage`, otherwise `true`. ★Reading the current value and writing it happen in one lock★: merging with a value the caller read earlier would overwrite another toggle that came in between.
pub fn set_usage_slot(
    state: &LayoutState,
    subs: &dyn SubscriptionSync,
    events: &dyn LayoutEvents,
    view_id: Uuid,
    slot_id: Uuid,
    show_claude: Option<bool>,
    show_codex: Option<bool>,
) -> Result<(), String> {
    let (layout, tabs) = {
        let mut mgr = state.0.lock().map_err(|e| e.to_string())?;
        let (claude_now, codex_now) = match mgr
            .slot_content(view_id, slot_id)
            .map_err(|e| e.to_string())?
        {
            SlotContent::Usage {
                show_claude,
                show_codex,
            } => (show_claude, show_codex),
            _ => (true, true),
        };
        let content = SlotContent::Usage {
            show_claude: show_claude.unwrap_or(claude_now),
            show_codex: show_codex.unwrap_or(codex_now),
        };
        mgr.set_slot_content(view_id, slot_id, content)
            .map_err(|e| e.to_string())?;
        let layout = mgr.snapshot(view_id).ok();
        let tabs = owner_tabs(&mgr, view_id);
        subs.resync(&mgr);
        (layout, tabs)
    };
    notify(events, layout, tabs);
    Ok(())
}

// ★spawn_into (D-7)★: spawn + tab creation (if needed) + slot assignment in one go. On success, the new agent id.
//
// ## ★ordering (ADR-0006 concurrency contract — CRITICAL)★
// Spawn is a daemon round trip (async), so it finishes first **without holding the ViewManager lock**. Only afterwards is the lock taken and (tab/slot resolution + occupancy check + assignment + re-sync) run as one critical section; notifications go out after the lock is dropped.
//
// ## ★Slot policy (G9 — no guessing)★
// - `tab=None`: first create a new tab (an empty root slot) and assign there. A `slot=Some` with it is ★rejected before the spawn★ (the new tab does not have that slot, so an orphan tab would arise).
//
// ## ★Failure visibility (§5 — spawn-first)★
// Even if the later placement fails (occupied slot · invalid view/window · lock acquisition failure), **the agent is not killed** (no hard rollback): it is alive in the daemon and can be re-attached by listing. Every early-return after the spawn embeds the living agent id with `alive_err`, so no agent is invisible.
//
// ## backend — the shell blocks only unknown words (a typo net)
// A typo (`codx`) would otherwise cost one spawn round trip before the daemon rejects it, and an old daemon would ignore the field
// and quietly start a different backend (ADR-0058 protects against exactly that). The net is not a promise that the backend comes
// up (the daemon decides). The vocabulary is asked from the wire vocabulary itself, not written by hand, so it cannot diverge.
// An unspecified backend is not blocked here: the daemon refuses absence and issues the text naming the field to fill.
//
// ## ★The policy axis (even for a known word, is it created now) is not in the shell — the daemon's `SpawnByCwd` handler bears it★
// The daemon asks the backend creation policy (agent's `LLM_BACKEND_POLICY`) before the spawn (ADR-0279); that rejection returns through the `?` of 1) the spawn step, never reaches 2) placement, and the layout does not change. The daemon's text goes without a prefix.
// ★Do not revive the policy check here★: it would make the shell depend on the agent crate again (ADR-0270 decision 1 · CI gate `shell gate 1`).
// ★Do not lump the two rejections into one text★: the daemon's policy rejection says "a known word, but … not created", this net says "does not know …"; lumped, the caller would try to fix a typo that is not there.
/// Typo net — ★the vocabulary is not written by hand★: the decision and the "what works" text both come from the wire enum (serde's unknown-variant error lists the expected words), so this and the daemon cannot diverge.
// ADR-0270 · ADR-0279 (TRD §6)
fn parse_backend(word: &str) -> Result<AgentBackendKind, String> {
    serde_json::from_value::<AgentBackendKind>(serde_json::Value::String(word.to_string()))
        .map_err(|e| format!("unknown backend '{word}' ({e}). Not spawned."))
}

pub async fn spawn_into(
    state: &LayoutState,
    subs: &dyn SubscriptionSync,
    events: &dyn LayoutEvents,
    spawner: &dyn AgentSpawner,
    window: &str,
    tab: Option<Uuid>,
    slot: Option<Uuid>,
    backend: Option<String>,
    cwd: String,
) -> Result<String, SpawnRefusal> {
    spawn_into_placed(
        state,
        subs,
        events,
        spawner,
        Some(window),
        tab,
        slot,
        backend,
        cwd,
    )
    .await
    .map(|placed| placed.out.id)
}

/// The outcome form of [`spawn_into`] — the started agent (`out`, with its launch notices), where it was placed and
/// that tab's layout rows (taken under the lock that placed it · TRD S28 §7). `window` = the window to place in — `None`
/// = the tab's window when `tab` is given, else main. The refusal texts are those of `spawn_into`.
// ADR-0307 · ADR-0348
pub async fn spawn_into_placed(
    state: &LayoutState,
    subs: &dyn SubscriptionSync,
    events: &dyn LayoutEvents,
    spawner: &dyn AgentSpawner,
    window: Option<&str>,
    tab: Option<Uuid>,
    slot: Option<Uuid>,
    backend: Option<String>,
    cwd: String,
) -> Result<Wrote<SpawnedAgent>, SpawnRefusal> {
    // ── 0) Pre-spawn validation (before the agent exists, so alive_err is unnecessary) ──────────────
    let backend = match backend.as_deref().map(str::trim).filter(|b| !b.is_empty()) {
        None => None,
        Some(word) => Some(parse_backend(word)?),
    };
    if tab.is_none() && slot.is_some() {
        return Err(
            "cannot specify a particular slot for a tab that will be newly created — omit slot or specify tab. Not spawned."
                .to_string()
                .into(),
        );
    }
    // If a tab is given, place it in that tab's window, and do not bring it up if that window is not the tab's owner (learning that afterwards would leave an agent with nowhere to be placed). The same decision in the placement lock below covers the tab moving during the spawn.
    let window = {
        let mgr = state.0.lock().map_err(|e| e.to_string())?;
        spawn_window(&mgr, window, tab)?
    };
    let window = window.as_str();

    spawn_then_place(state, subs, events, spawner, window, backend, cwd, |mgr| {
        let view_id = match tab {
            Some(v) => {
                if mgr.owner_of(v).map(|l| l.as_str()) != Some(window) {
                    return Err(format!("view {v} is not a tab of window {window}"));
                }
                v
            }
            // tab=None → a new tab (one empty root). slot is confirmed None by the guard above, so resolve always succeeds on that empty root; create_tab is the only failure point (window absent). (FIX 3)
            None => mgr.create_tab(window, None).map_err(|e| e.to_string())?,
        };
        let target_slot = mgr
            .resolve_spawn_slot(view_id, slot)
            .map_err(|e| e.to_string())?;
        Ok((view_id, target_slot))
    })
    .await
}

// The window to bring it up in: the owner window of `tab` if given (a given `window` must be that window), otherwise `window`, otherwise main.
fn spawn_window(
    mgr: &ViewManager,
    window: Option<&str>,
    tab: Option<ViewId>,
) -> Result<WindowLabel, String> {
    let Some(view) = tab else {
        return Ok(window.unwrap_or(MAIN_WINDOW_LABEL).to_string());
    };
    let owner = mgr
        .owner_of(view)
        .ok_or_else(|| format!("view {view} not found. Not spawned."))?;
    match window {
        Some(window) if window != owner.as_str() => Err(format!(
            "view {view} is not a tab of window {window} (that tab is in window {owner}). Not spawned."
        )),
        _ => Ok(owner.clone()),
    }
}

/// The rejection of [`spawn_into_at`].
// ADR-0307
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SpawnAtError {
    /// Before the spawn: the word picked no area in that window's visible tab (including window not found) or the area cannot host it (occupied). Nothing was brought up.
    #[error("{0}. Not spawned.")]
    Unplaced(TargetError),
    /// Otherwise the text has the same form as [`spawn_into`] (a pre-spawn rejection is "Not spawned", a post-spawn placement failure carries the living agent's id).
    #[error("{0}")]
    Failed(SpawnRefusal),
}

/// The spatial-word form of [`spawn_into`] — starts the agent in the slot `at` picks in window `window`'s shown tab.
/// That slot must be empty (`resolve_spawn_slot`'s occupancy check). Outcome = the started agent (`out`), the resolved
/// place and that tab's layout rows (taken under the lock that placed it).
///
/// The word is resolved twice: in the pre-spawn lock (cannot pick or occupied → ends without spawning) and again in the placement lock (the first lock after the spawn). If the layout changed during the spawn and it now cannot pick, is occupied, or ★picks a different area than the one confirmed★ (a relative word follows focus), it is not placed in another area. The spawn already happened, so the text is the same as the placement failure of [`spawn_into`] (the living agent's id).
// ADR-0307 (TRD S28 §5-2)
pub async fn spawn_into_at(
    state: &LayoutState,
    subs: &dyn SubscriptionSync,
    events: &dyn LayoutEvents,
    spawner: &dyn AgentSpawner,
    window: &str,
    at: SpatialToken,
    backend: Option<String>,
    cwd: String,
) -> Result<Wrote<SpawnedAgent>, SpawnAtError> {
    let backend = match backend.as_deref().map(str::trim).filter(|b| !b.is_empty()) {
        None => None,
        Some(word) => Some(parse_backend(word).map_err(|e| SpawnAtError::Failed(e.into()))?),
    };
    let target = Target::At {
        token: at,
        window: Some(window.to_string()),
    };
    let checked = {
        let mgr = lock_for(state).map_err(SpawnAtError::Unplaced)?;
        spawn_slot_at(&mgr, &target).map_err(SpawnAtError::Unplaced)?
    };
    spawn_then_place(state, subs, events, spawner, window, backend, cwd, |mgr| {
        let now = spawn_slot_at(mgr, &target).map_err(|e| e.to_string())?;
        if now != checked {
            return Err(format!(
                "'{}' picks a different area while spawning — {} instead of the confirmed area {}",
                at.as_str(),
                checked.1,
                now.1
            ));
        }
        Ok(now)
    })
    .await
    .map_err(SpawnAtError::Failed)
}

// Whether the area that target `target` picks is an empty area that can host a spawn — (tab, area). Rejection texts for occupied · nonexistent areas are `resolve_spawn_slot`'s as is (`TargetError::Apply`).
fn spawn_slot_at(mgr: &ViewManager, target: &Target) -> Result<(ViewId, Uuid), TargetError> {
    let (at, slot) = locate_slot(mgr, target)?;
    let slot = mgr
        .resolve_spawn_slot(at.view, Some(slot))
        .map_err(applied)?;
    Ok((at.view, slot))
}

// Spawn (async with no lock held — a daemon round trip), then place (one critical section under the lock). `place`
//   picks (tab, empty slot) under that lock. A placement failure names the live agent id (spawn-first — failure
//   visibility above) and keeps the spawn's notices (M11). Outcome = the started agent · where it was placed · that tab's layout
//   rows (same lock).
// ADR-0348
async fn spawn_then_place(
    state: &LayoutState,
    subs: &dyn SubscriptionSync,
    events: &dyn LayoutEvents,
    spawner: &dyn AgentSpawner,
    window: &str,
    backend: Option<AgentBackendKind>,
    cwd: String,
    place: impl FnOnce(&mut ViewManager) -> Result<(ViewId, Uuid), String>,
) -> Result<Wrote<SpawnedAgent>, SpawnRefusal> {
    // ── 1) Spawn (async, no lock — daemon round trip) ──────────────────────────────────────────
    let spawned = spawner.spawn_by_cwd(cwd, backend).await?;
    let agent_id = spawned.id.clone();

    // ── 2) Placement (one critical section under the lock) ────────────────────────────────────────────────────────
    let alive_err = |detail: String| {
        SpawnRefusal {
        message: format!(
            "placement failed ({detail}) — agent {agent_id} is alive (do not start it again; attach it with layout.setSlotContent <slot> --content Agent --assign {agent_id} · it is in agent.list)"
        ),
        launch_blocked: None,
        created_agent_id: None,
        live_agent_id: Some(agent_id.clone()),
        notices: spawned.notices.clone(),
    }
    };
    let (view_id, target_slot, layout, tabs, rows) = {
        // A lock acquisition failure (mutex poison) also keeps the living agent id. (FIX 4)
        let mut mgr = state
            .0
            .lock()
            .map_err(|e| alive_err(format!("failed to acquire the layout lock: {e}")))?;

        let (view_id, target_slot) = place(&mut mgr).map_err(alive_err)?;

        // The occupancy check was already done above; assign is reached only after an empty slot is confirmed.
        mgr.assign_agent(view_id, target_slot, agent_id.clone())
            .map_err(|e| alive_err(e.to_string()))?;

        let layout = mgr.snapshot(view_id).ok();
        let tabs = tabs_payload(&mgr, window);
        subs.resync(&mgr);
        let rows = rows::rows_of_views(&mgr, &[view_id]);
        (view_id, target_slot, layout, tabs, rows)
    };
    notify(events, layout, tabs);

    tracing::info!(agent = %agent_id, window = %window, view = %view_id, "spawn_into done (spawn + placement)");
    Ok(Wrote {
        at: Located {
            window: window.to_string(),
            view: view_id,
            slot: Some(target_slot),
        },
        out: spawned,
        rows,
    })
}

/// Outcome of a slot detach (pop-out): the label of the window the content landed in + the View id of the **new tab** carrying it + that window's persistent id · number (the window's own, new or existing — as [`NewWindow`]).
///
/// The field names are the wire contract: the frontend (`viewStore.moveSlotToWindow`) reads `{window, tab, window_id, number}`. (TRD S26-window-chrome §3-2)
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct SlotMove {
    pub window: String,
    pub tab: Uuid,
    pub window_id: String,
    pub number: u32,
}

// ★MOVE a slot to a new tab of another window (not a mirror)★: moves the **content (SlotContent)** of the original slot to the new tab and removes the original slot from the original View. The agent process is untouched; only the display surface moves.
// `to_window` unspecified → a new window (label = the handed-over prewarmed window's, otherwise a new one from the issuer); specified → a new tab of that existing window.
//
// ★ADR-0064 all slot content★: agent_list/usage and empty slots (ADR-0228) move too. Non-agent content has no backend output subscription, so re-sync is naturally a no-op. The only rejections are a nonexistent view/slot, returned as the `LayoutError` reason.
//
// ## ★2-phase rollback + no orphan for an existing-window target (G4)★
// While the lock is released the target window may be destroyed/closed concurrently, so **tab insertion is deferred to phase C**, and the target window is re-checked there (absent → rollback). The source detach is closed in the 2nd lock with a still-ours guard.
// ADR-0057
// ADR-0064
// ADR-0228
pub fn move_slot_to_window(
    state: &LayoutState,
    subs: &dyn SubscriptionSync,
    events: &dyn LayoutEvents,
    host: &dyn WindowHost,
    labels: &dyn LabelSource,
    view_id: Uuid,
    slot_id: Uuid,
    to_window: Option<String>,
) -> Result<SlotMove, String> {
    move_slot(
        state,
        subs,
        events,
        host,
        labels,
        |_| Ok((view_id, slot_id, ())),
        to_window,
        false,
    )
    .map(|(moved, (), _)| moved)
    .map_err(|e| e.to_string())
}

/// The target edition of [`move_slot_to_window`]: resolves the area to move inside phase A (the lock that takes the content). Outcome = the resolved place · the move outcome (`out`) · the layout rows of the original tab and the new tab (taken in phase C's inserting lock; only the new tab's if the original tab vanished in between). Rejection texts as the id edition (`TargetError::Apply`).
// ADR-0307
pub fn move_slot_to_window_at(
    state: &LayoutState,
    subs: &dyn SubscriptionSync,
    events: &dyn LayoutEvents,
    host: &dyn WindowHost,
    labels: &dyn LabelSource,
    target: &Target,
    to_window: Option<String>,
) -> Result<Wrote<SlotMove>, TargetError> {
    move_slot(
        state,
        subs,
        events,
        host,
        labels,
        |mgr| {
            let (at, slot) = locate_slot(mgr, target)?;
            Ok((at.view, slot, at))
        },
        to_window,
        true,
    )
    .map(|(out, at, rows)| Wrote { at, out, rows })
}

// `pick` = inside the phase A lock, picks the (tab, area) to move and what to return to the caller. `with_rows` = whether the phase C lock takes the layout rows of the original and new tab (target edition only).
fn move_slot<T>(
    state: &LayoutState,
    subs: &dyn SubscriptionSync,
    events: &dyn LayoutEvents,
    host: &dyn WindowHost,
    labels: &dyn LabelSource,
    pick: impl FnOnce(&ViewManager) -> Result<(ViewId, Uuid, T), TargetError>,
    to_window: Option<String>,
    with_rows: bool,
) -> Result<(SlotMove, T, Vec<LayoutRow>), TargetError> {
    let is_new_window = to_window.is_none();
    let (target_label, claim) = match to_window {
        Some(label) => (label, None),
        None => new_window_label(host, labels),
    };

    // ── phase A (lock): source content → temporary View (not yet in any window's tabs — prevents an orphan) ──────────
    // ★SlotContent is carried out of the lock★ (MOVE atomicity): while the window build releases the lock, the original slot may be reassigned; in the 2nd lock, compare this value with the re-queried one and close the original "only if it is still the content that was moved" (never delete wrong content).
    //
    // ★Why an owner-less tmp_view is safe in `views` during phase B (unlocked)★: it is in `views` but in neither `view_owner` nor `windows[*].tabs` ("every View has one owner" is temporarily violated). Safe because ① only this op holds the view id ② it is in no window's tabs, so the routing walk (window → tabs) misses it ③ the source content keeps showing since the source slot is alive ④ the end point is always phase C attach (owner granted) or rollback (`drop_detached_view`). ⚠️ Code added later that walks all views and assumes an owner must account for this temporary owner-less View (assuming an owner = panic during this op).
    let detached = lock_for(state).and_then(|mut mgr| {
        let (view_id, slot_id, picked) = pick(&mgr)?;
        let detached = mgr
            .prepare_detached_view(view_id, slot_id)
            .map_err(applied)?;
        subs.resync(&mgr);
        Ok((view_id, slot_id, picked, detached))
    });
    let (view_id, slot_id, picked, (tmp_view, src_content)) = match detached {
        Ok(detached) => detached,
        Err(e) => {
            // No window opened yet; only the handed-over prewarmed window is returned.
            if let Some(claim) = &claim {
                host.release_spare(claim);
            }
            return Err(e);
        }
    };

    // ── phase B (unlocked): a new-window target builds the webview; an existing-window target only confirms existence ─────────────────
    if is_new_window {
        if let Err(e) = host.open(&target_label) {
            rollback_detached(state, subs, tmp_view);
            return Err(TargetError::Apply(e));
        }
    } else if !host.is_open(&target_label) {
        rollback_detached(state, subs, tmp_view);
        return Err(TargetError::Apply(format!(
            "target window not found: {target_label}"
        )));
    }

    // ── phase C (lock): insert the temporary View as a tab of the target window (★re-verify an existing window★) + close the source slot ───────────
    let (src_tabs, tgt_tabs, src_layout, rows) = {
        // If the lock is poisoned the temporary View cannot be reaped (as `rollback_detached`), but the opened window is; on `Err` no floating new window remains (window port contract).
        let mut mgr = match lock_for(state) {
            Ok(mgr) => mgr,
            Err(e) => {
                tracing::warn!(
                    window = %target_label,
                    view = %tmp_view,
                    new_window = is_new_window,
                    "slot MOVE: lock poisoned — skipping the temporary View rollback · reaping if it is a new window"
                );
                if is_new_window {
                    withdraw_new_window(host, &target_label, claim.as_ref());
                }
                return Err(e);
            }
        };

        let inserted = if is_new_window {
            mgr.attach_view_as_new_window(&target_label, tmp_view)
        } else {
            mgr.insert_tab_into(&target_label, tmp_view)
        };
        if let Err(e) = inserted {
            // ★Two rollback branches★: for an existing-window target, `insert_tab_into` fails if the target vanished from the model during phase B. For a new-window target the label is monotonic so it cannot collide, but if that window closed during phase B and a tombstone stands (`ViewManager::mark_closed`), `attach_view_as_new_window` fails with `WindowClosed`; then the just-opened window is reaped (no-op if already closed; returned if it is a handed-over prewarmed window).
            mgr.drop_detached_view(tmp_view);
            subs.resync(&mgr);
            drop(mgr);
            if is_new_window {
                withdraw_new_window(host, &target_label, claim.as_ref());
            }
            return Err(TargetError::Apply(format!(
                "tab insertion failed (rollback): {e}"
            )));
        }

        // ★MOVE→COPY degradation is an intended best-effort★: if the source slot is reassigned during phase B (unlocked), still_ours=false and the close is skipped. "Not deleting reassigned wrong content" has top priority, at the price of the original content remaining in both the target tab and the source slot; harmless because the model allows two Views with the same content (progress independent — ADR-0046), so no strict rollback. The same comparison applies when what moved is an empty area: if it was filled in between it mismatches and the filled content survives (ADR-0228).
        // ★load-bearing★: if the source View itself vanished in the gap (tab/window closed), `slot_content` gives `Err`, the comparison fails and the close is skipped; this prevents a malfunction/panic from closing an already-vanished source (do not modify).
        let still_ours = matches!(
            mgr.slot_content(view_id, slot_id),
            Ok(ref c) if *c == src_content
        );
        if still_ours {
            let _ = mgr.close_slot(view_id, slot_id);
        } else {
            tracing::warn!(
                view = %view_id, slot = %slot_id,
                "the original slot was reassigned/removed during window creation — skipping MOVE's close (the target tab stays)"
            );
        }

        let src_tabs = owner_tabs(&mgr, view_id);
        let tgt_tabs = tabs_payload(&mgr, &target_label);
        let src_layout = mgr.snapshot(view_id).ok();
        subs.resync(&mgr);
        let rows = if with_rows {
            rows::rows_of_views(&mgr, &[view_id, tmp_view])
        } else {
            Vec::new()
        };
        (src_tabs, tgt_tabs, src_layout, rows)
    };

    announce(
        events,
        src_layout.as_ref(),
        [src_tabs.as_ref(), tgt_tabs.as_ref()].into_iter().flatten(),
    );
    // Confirmation · the first place record come after the notification: the window fact (`window:frame`) that record emits must follow the new window's tab notification (same order as docking new window).
    // ADR-0299 (TRD S21-slot-docking §3-9)
    if is_new_window {
        settle_new_window(host, &target_label, claim.as_ref());
    }

    tracing::info!(window = %target_label, view = %tmp_view, "slot MOVE done (detach)");
    let target = window_answer(&target_label, tgt_tabs.as_ref()).map_err(TargetError::Apply)?;
    Ok((
        SlotMove {
            window: target.window,
            tab: tmp_view,
            window_id: target.window_id,
            number: target.number,
        },
        picked,
        rows,
    ))
}

// Rollback of the phase A temporary View (before window insertion, so no tabs update is needed). The source slot is kept, so the user does not lose it. If the lock cannot be taken (poison) the rollback is abandoned: a panic here would take the process down over an op that already failed.
fn rollback_detached(state: &LayoutState, subs: &dyn SubscriptionSync, tmp_view: Uuid) {
    let Ok(mut mgr) = state.0.lock() else {
        tracing::warn!("rollback_detached: lock poisoned — skipping rollback");
        return;
    };
    mgr.drop_detached_view(tmp_view);
    subs.resync(&mgr);
}

// ── Docking ─────────────────────────────────────────────────────────────────────

/// A request to move one tab, received by [`dock_tab`]. For `focus` · `mv` see [`ViewManager::dock`].
#[derive(Debug, Clone, PartialEq)]
pub struct DockRequest {
    pub view: ViewId,
    pub to: DockDest,
    pub focus: FocusPolicy,
    pub mv: DockMove,
}

/// Where to move. `Seam` · `TabBar` are the same as the same-named [`DockTo`] variants.
#[derive(Debug, Clone, PartialEq)]
pub enum DockDest {
    Seam {
        target: ViewId,
        anchor: DockAnchor,
        side: DockSide,
        share: f64,
    },
    TabBar {
        window: WindowLabel,
        index: Option<usize>,
        after: Option<ViewId>,
    },
    /// A window holding only that tab; the label is chosen by the apply service. If the source is a pop-out with a single tab, that window is moved to `at` without a new window (reuse). If `at.size` is absent, the new window takes the source pop-out's normal size (the popout default size if the source is main); reuse keeps that window's size.
    NewWindow { at: WindowPlace },
}

/// The outcome of [`dock_tab`] · [`dock_target`].
#[derive(Debug, Clone, PartialEq)]
pub struct DockOutcome {
    /// Whether the model changed; false when dropping on the tab bar of the same window at the same place, and for reuse (only the window moves). (TRD S28 §4-4)
    pub changed: bool,
    /// The resulting tab: `Seam` = the surviving target tab (the moved tab went into it and was erased), otherwise the moved tab.
    pub tab: ViewId,
    /// Tabs whose layout changed or that became newly visible: `tab` first, then the tabs newly visible in the source window (including an empty tab newly set up in main). Just `tab` if nothing changed.
    pub views: Vec<ViewId>,
    pub window: WindowLabel,
    /// The slot ids of the moved tab (tree preorder); unchanged by the move.
    pub slot_ids: Vec<Uuid>,
    /// The source pop-out that became empty and was erased from the model with its OS window closed; `None` on the reuse path.
    pub emptied: Option<WindowLabel>,
    pub main_refilled: Option<ViewId>,
    /// The source window was moved without a new window: the model is unchanged (`version` unchanged, no notification).
    pub reused: bool,
    /// The new window was opened as a prewarmed window handed over by the window port ([`WindowHost::claim_spare`]); always false if `reused`.
    // ADR-0299
    pub prewarmed: bool,
    /// The identity of `window` (as [`NewWindow`]), taken in the lock that makes the outcome: always `Some` for a new window · `Seam` · `TabBar`; for reuse `None` if that window closed while being moved. The outcome is a success even if `None` (everything to move was already moved). (TRD S26-window-chrome §3-2; TRD S28 §4-4)
    pub identity: Option<NewWindow>,
    /// The layout rows of `views`, taken in the **same lock** as `window` · `tab` · `identity` so a command answer is a single moment's picture. Only the target edition ([`dock_target`]) fills it; the id edition ([`dock_tab`]) leaves it empty (the mouse path does not read rows).
    // ADR-0307 (TRD S28)
    pub rows: Vec<LayoutRow>,
}

/// Moves tab `req.view` to `req.to`. On `Err` the model is unchanged, no notification is sent, and a new window opened in between is reaped. The canonical decision · application rules: [`ViewManager::dock`] · [`ViewManager::plan_new_window`].
///
/// - `Seam` · `TabBar`: change in one lock, notify outside the lock, then close the window of the emptied-and-erased source pop-out.
/// - `NewWindow` (new window): open the window first outside the lock, then redo the same decision and attach.
/// - `NewWindow` (reuse): move the source window outside the lock (no model change); it neither re-decides nor reverts.
///
/// ★Do not call `NewWindow` from the main thread or inside a synchronous command★: the new window build deadlocks (same reason `commands::popout::move_slot_to_window` is async). `Seam` · `TabBar` open no window and can be called from any thread (an emptied source window is only closed). (TRD S21-slot-docking §3-2 ④; TRD S21-slot-docking §3-2 ④ · §3-3)
pub fn dock_tab(
    state: &LayoutState,
    subs: &dyn SubscriptionSync,
    events: &dyn LayoutEvents,
    host: &dyn WindowHost,
    labels: &dyn LabelSource,
    req: DockRequest,
) -> Result<DockOutcome, DockError> {
    let DockRequest {
        view,
        to,
        focus,
        mv,
    } = req;
    let ask = DockAsk {
        focus,
        mv,
        rows: false,
    };
    let (mode, result) = match to {
        DockDest::Seam {
            target,
            anchor,
            side,
            share,
        } => (
            "seam",
            dock_in_place(state, subs, events, host, &ask, |_| {
                Ok::<_, DockError>((
                    view,
                    DockTo::Seam {
                        target,
                        anchor,
                        side,
                        share,
                    },
                ))
            }),
        ),
        DockDest::TabBar {
            window,
            index,
            after,
        } => (
            "tab_bar",
            dock_in_place(state, subs, events, host, &ask, |_| {
                Ok((
                    view,
                    DockTo::TabBar {
                        window,
                        index,
                        after,
                    },
                ))
            }),
        ),
        DockDest::NewWindow { at } => (
            "new_window",
            dock_into_window(
                state,
                subs,
                events,
                host,
                labels,
                &ask,
                at,
                |_| Ok(view),
                |_, _, _| Ok(()),
            ),
        ),
    };
    log_dock(mode, &view, &result);
    result.map(|(out, _)| out)
}

fn log_dock<E: Display>(
    mode: &str,
    subject: &dyn std::fmt::Debug,
    result: &Result<(DockOutcome, DockNotes), E>,
) {
    match result {
        Ok((out, notes)) => tracing::info!(
            view = %notes.view,
            mode = if out.reused { "reuse" } else { mode },
            window = %out.window,
            emptied = out.emptied.as_deref(),
            reused = out.reused,
            main_refilled = out.main_refilled.is_some(),
            changed = notes.unchanged.then_some(false),
            "docking applied"
        ),
        Err(e) => {
            tracing::debug!(view = ?subject, mode, reason = %e, "docking refused — model unchanged")
        }
    }
}

struct DockAsk {
    focus: FocusPolicy,
    mv: DockMove,
    // Whether to put layout rows in the outcome (`DockOutcome::rows`); target edition only.
    rows: bool,
}

impl DockAsk {
    // The layout rows of the outcome, called inside the very lock that makes the outcome; if `views` is empty (nothing changed), the rows of the single `shown`.
    fn rows(&self, mgr: &ViewManager, views: &[ViewId], shown: ViewId) -> Vec<LayoutRow> {
        if !self.rows {
            return Vec::new();
        }
        if views.is_empty() {
            return rows::rows_of_views(mgr, &[shown]);
        }
        rows::rows_of_views(mgr, views)
    }
}

// The end-log part: what is not put in the outcome (`DockOutcome`).
struct DockNotes {
    view: ViewId,
    // Dropping on the tab bar of the same window at the same place — the model is unchanged.
    unchanged: bool,
}

// What is used to pick what to announce from the picture just before `ViewManager::dock`, taken inside the same lock.
struct BeforeDock {
    version: u64,
    source_active: Option<ViewId>,
}

impl BeforeDock {
    fn take(mgr: &ViewManager, view: ViewId) -> Self {
        BeforeDock {
            version: mgr.version,
            source_active: mgr
                .owner_of(view)
                .and_then(|label| mgr.windows.get(label))
                .map(|w| w.active),
        }
    }
}

// The notification a successful `dock` will emit, copied inside the lock. Empty = no change (dropping on the tab bar of the same window at the same place). `views` = the tabs of `layouts`, in that order (it becomes the outcome's `views`).
#[derive(Default)]
struct DockNews {
    views: Vec<ViewId>,
    layouts: Vec<ViewSnapshot>,
    tabs: Vec<WindowTabsPayload>,
}

impl DockNews {
    // `shown` = the tab to be visible at the target: the target tab for `Seam`, otherwise the moved tab.
    // ★If the active tab of the remaining source window changed, that tab's layout is carried too★: the frontend would otherwise get that tab id only in the tab list, with no layout (as `close_tab` carries the remaining window's active layout); this includes the empty tab set up because main's last tab left.
    // ★The tab list is not carried for an emptied-and-erased source★: a webview that receives a 0-tab notification closes its own window.
    fn collect(mgr: &ViewManager, before: &BeforeDock, docked: &Docked, shown: ViewId) -> Self {
        if mgr.version == before.version {
            return DockNews::default();
        }
        let mut views = vec![shown];
        let mut windows = vec![docked.window.as_str()];
        if docked.source_window != docked.window && docked.emptied.is_none() {
            windows.push(&docked.source_window);
            let active = mgr.windows.get(&docked.source_window).map(|w| w.active);
            if let Some(active) = active.filter(|a| Some(*a) != before.source_active && *a != shown)
            {
                views.push(active);
            }
        }
        DockNews {
            layouts: views.iter().filter_map(|v| mgr.snapshot(*v).ok()).collect(),
            views,
            tabs: windows
                .into_iter()
                .filter_map(|label| tabs_payload(mgr, label))
                .collect(),
        }
    }
}

// The outside-the-lock part of a docking that changed the model: notification → closing the OS window of the emptied-and-erased source pop-out (the model erased it first, as `close_tab`). Returns the outcome. `shown` = the resulting tab (`DockNews::collect`).
// ADR-0057
fn finish_docked(
    events: &dyn LayoutEvents,
    host: &dyn WindowHost,
    docked: Docked,
    news: DockNews,
    identity: Option<NewWindow>,
    shown: ViewId,
    rows: Vec<LayoutRow>,
) -> DockOutcome {
    announce(events, &news.layouts, &news.tabs);
    if let Some(emptied) = &docked.emptied {
        host.close(emptied);
    }
    let changed = !news.views.is_empty();
    DockOutcome {
        changed,
        tab: shown,
        views: if changed { news.views } else { vec![shown] },
        window: docked.window,
        slot_ids: docked.slot_ids,
        emptied: docked.emptied,
        main_refilled: docked.main_refilled,
        reused: false,
        prewarmed: false,
        identity,
        rows,
    }
}

// `Seam` · `TabBar`: target picking · decision · change · snapshot · re-sync are one lock. `pick` picks (the tab to move, the place to move to) inside that lock; the target edition resolves the target there. (TRD S28 §5-2)
fn dock_in_place<E: From<DockError>>(
    state: &LayoutState,
    subs: &dyn SubscriptionSync,
    events: &dyn LayoutEvents,
    host: &dyn WindowHost,
    ask: &DockAsk,
    pick: impl FnOnce(&ViewManager) -> Result<(ViewId, DockTo), E>,
) -> Result<(DockOutcome, DockNotes), E> {
    let (docked, news, identity, view, shown, rows) = {
        let mut mgr = sync::lock(&state.0);
        let (view, to) = pick(&mgr)?;
        let shown = match &to {
            DockTo::Seam { target, .. } => *target,
            _ => view,
        };
        let before = BeforeDock::take(&mgr, view);
        let docked = mgr.dock(view, to, ask.focus, ask.mv.clone())?;
        let news = DockNews::collect(&mgr, &before, &docked, shown);
        if mgr.version != before.version {
            subs.resync(&mgr);
        }
        let identity = NewWindow::in_model(&mgr, &docked.window);
        let rows = ask.rows(&mgr, &news.views, shown);
        (docked, news, identity, view, shown, rows)
    };
    let out = finish_docked(events, host, docked, news, identity, shown, rows);
    let unchanged = !out.changed;
    Ok((out, DockNotes { view, unchanged }))
}

// `NewWindow` phase A (lock): picks the tab to move (`pick`) and whether a new window is needed. `recheck` runs in the phase C lock: whether that tab is still the tab to move; if not, the opened window is reaped with that error (target edition). The third argument = the window that tab should be in (`None` for a new window, where it is removed from the current window when attached; for reuse the reused window; if the tab left it or it closed, the outcome cannot name that window). (TRD S28 §4-3)
fn dock_into_window<E: From<DockError> + Display>(
    state: &LayoutState,
    subs: &dyn SubscriptionSync,
    events: &dyn LayoutEvents,
    host: &dyn WindowHost,
    labels: &dyn LabelSource,
    ask: &DockAsk,
    at: WindowPlace,
    pick: impl FnOnce(&ViewManager) -> Result<ViewId, E>,
    recheck: impl FnOnce(&ViewManager, ViewId, Option<&str>) -> Result<(), E>,
) -> Result<(DockOutcome, DockNotes), E> {
    let (view, plan) = {
        let mgr = sync::lock(&state.0);
        let view = pick(&mgr)?;
        (view, mgr.plan_new_window(view)?)
    };
    match plan {
        NewWindowPlan::Reuse { label } => {
            reuse_source_window(state, host, ask, view, label, &at, recheck)
        }
        NewWindowPlan::Create { size } => {
            let place = WindowPlace {
                pos: at.pos,
                size: at.size.or(size),
            };
            let out = open_new_window(
                state, subs, events, host, labels, ask, view, &place, recheck,
            )?;
            Ok((
                out,
                DockNotes {
                    view,
                    unchanged: false,
                },
            ))
        }
    }
}

// New window: phase B (outside the lock) opens the window → phase C (lock) redoes the same decision and attaches. If phase C refuses (the window closed in between and a tombstone stands, or the source tab vanished), the opened window is reaped; the source window was never touched.
// ★A prewarmed window is asked about regardless of `focus`★: even with `Keep` the handed-over window is used, and whether it is activated is written into that handover by `open_at`.
// ★`recheck` comes before `dock` inside the attaching lock★: if it fails there, only the opened window is reaped, as when `dock` refused.
// ADR-0299 (TRD S21-slot-docking §3-9 I4)
fn open_new_window<E: From<DockError> + Display>(
    state: &LayoutState,
    subs: &dyn SubscriptionSync,
    events: &dyn LayoutEvents,
    host: &dyn WindowHost,
    labels: &dyn LabelSource,
    ask: &DockAsk,
    view: ViewId,
    place: &WindowPlace,
    recheck: impl FnOnce(&ViewManager, ViewId, Option<&str>) -> Result<(), E>,
) -> Result<DockOutcome, E> {
    let (label, claim) = new_window_label(host, labels);
    if let Err(e) = host.open_at(&label, place, ask.focus == FocusPolicy::Reveal) {
        tracing::warn!(label = %label, "could not open the docking new window — model unchanged: {e}");
        return Err(DockError::WindowOpen(e).into());
    }

    let committed = {
        let mut mgr = sync::lock(&state.0);
        recheck(&mgr, view, None).and_then(|()| {
            let before = BeforeDock::take(&mgr, view);
            mgr.dock(
                view,
                DockTo::NewWindow {
                    label: label.clone(),
                },
                ask.focus,
                ask.mv.clone(),
            )
            .map(|docked| {
                let news = DockNews::collect(&mgr, &before, &docked, view);
                let identity = NewWindow::in_model(&mgr, &label);
                subs.resync(&mgr);
                let rows = ask.rows(&mgr, &news.views, view);
                (docked, news, identity, rows)
            })
            .map_err(E::from)
        })
    };
    let (docked, news, identity, rows) = match committed {
        Ok(done) => done,
        Err(e) => {
            withdraw_new_window(host, &label, claim.as_ref());
            tracing::warn!(label = %label, reason = %e, "docking new window — the decision changed before attaching; reaping the opened window");
            return Err(e);
        }
    };
    let out = DockOutcome {
        prewarmed: claim.is_some(),
        ..finish_docked(events, host, docked, news, identity, view, rows)
    };
    settle_new_window(host, &label, claim.as_ref());
    Ok(out)
}

// Reuse: B′ (outside the lock) moves the source window → inside the lock read only what goes in the outcome (model change 0).
//   ★It neither re-decides nor reverts★: if the drop stands, the window stays where it was put. If another path changed the source while moving (added a tab · slot, closed a tab), the model is as that write left it: with tabs grown, the whole window was moved; a closed tab's close reaps the window too.
// `recheck` runs in that reading lock: whether the target is still that tab of window `label`; if not, only the outcome is that error (target edition; the moved window is not reverted, and the target edition is called with the place emptied so nothing was moved). The id edition always passes (if that window closed: `Ok` + `identity: None`). (TRD S21-slot-docking §3-3 · §3-5)
fn reuse_source_window<E: From<DockError>>(
    state: &LayoutState,
    host: &dyn WindowHost,
    ask: &DockAsk,
    view: ViewId,
    label: WindowLabel,
    at: &WindowPlace,
    recheck: impl FnOnce(&ViewManager, ViewId, Option<&str>) -> Result<(), E>,
) -> Result<(DockOutcome, DockNotes), E> {
    if let Err(failed) = host.move_window(&label, at) {
        tracing::warn!(
            label = %label,
            step = ?failed.step,
            "reuse docking — could not move the source window (not reverted): {}",
            failed.error
        );
        return Err(DockError::WindowOpen(format!(
            "moving the window {:?} failed — {}",
            failed.step, failed.error
        ))
        .into());
    }

    let (slot_ids, identity, still, rows) = {
        let mgr = sync::lock(&state.0);
        let slot_ids = mgr
            .views
            .get(&view)
            .map(|v| tree::slot_ids(&v.layout))
            .unwrap_or_default();
        let still = recheck(&mgr, view, Some(&label));
        // The rows of an outcome that will be refused are not taken.
        let rows = if still.is_ok() {
            ask.rows(&mgr, &[view], view)
        } else {
            Vec::new()
        };
        (slot_ids, NewWindow::in_model(&mgr, &label), still, rows)
    };

    // A window that closed in between and is not in the model has no entry to record.
    if identity.is_some() {
        host.record_placement(&label);
    }
    still?;
    Ok((
        DockOutcome {
            changed: false,
            tab: view,
            views: vec![view],
            window: label,
            slot_ids,
            emptied: None,
            main_refilled: None,
            reused: true,
            prewarmed: false,
            identity,
            rows,
        },
        DockNotes {
            view,
            unchanged: false,
        },
    ))
}

// ── Docking — target edition ──────────────────────────────────────────

/// An LLM request to move one tab, received by [`dock_target`]. `tab` = the tab to move (if the target does not pick a tab, the tab holding that target); `focus` · `mv` as [`DockRequest`]. (TRD S28 §4 · §5-2)
#[derive(Debug, Clone, PartialEq)]
pub struct TargetDockRequest {
    pub tab: Target,
    pub to: TargetDest,
    pub focus: FocusPolicy,
    pub mv: DockMove,
}

/// The place to move, stated as a target; each variant matches the same-named [`DockDest`] variant.
#[derive(Debug, Clone, PartialEq)]
pub enum TargetDest {
    /// The `side` side of the area `beside` points at; if `whole`, the outer edge of the whole tab holding that area (`DockAnchor::Outer`; then `beside` may point at a tab). `share` = the attaching side's share, `None` = [`target::seam_share`].
    Seam {
        beside: Target,
        whole: bool,
        side: DockSide,
        share: Option<f64>,
    },
    TabBar(TabBarSpot),
    NewWindow {
        at: WindowPlace,
    },
}

/// A place in the tab row.
#[derive(Debug, Clone, PartialEq)]
pub enum TabBarSpot {
    /// Place `index` of window `window` (label; `None` = the current window of the tab to move); `index` as [`DockTo::TabBar`].
    At {
        window: Option<WindowLabel>,
        index: Option<usize>,
    },
    /// Right after the tab holding this target (counted in that tab's window, in the list without the source); `SelfTarget` if that tab is the tab to move.
    After(Target),
}

/// The rejection of [`dock_target`] — the model is unchanged. The text is that of the contained error as is.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TargetDockError {
    /// The tab to move (`tab`) could not be resolved.
    #[error("{0}")]
    Subject(LocateError),
    /// The place to move to (`Seam.beside` · `TabBarSpot::After`) could not be resolved.
    #[error("{0}")]
    Dest(LocateError),
    /// `beside` of a non-`whole` `Seam` pointed at a tab, not an area.
    #[error(
        "pointed at a tab, not at the area to attach to — give a target that points at an area, or attach to the whole tab (NotASlot)"
    )]
    NotASlot,
    /// While opening a new window (or moving the window for reuse) between two locks, the target to move became a different tab or vanished. The opened window was reaped and the model is unchanged; a reuse window already moved is not reverted (the place was emptied, so nothing was moved). (TRD S28 §4-3)
    #[error(
        "the tab to move changed while the new window was opening — nothing was moved (Moved)"
    )]
    Moved,
    #[error("{0}")]
    Dock(#[from] DockError),
}

/// The target edition of [`dock_tab`]: resolves the target (the tab to move · the area to attach to · the tab to insert after) inside the very lock that changes it. Thread · notification · reaping rules as [`dock_tab`] (★do not call `NewWindow` from the main thread or inside a synchronous command★).
///
/// - `NewWindow` takes the lock twice; in the second lock (the attaching lock for a new window, the outcome-reading lock for reuse) the target to move is resolved again, and a different tab than at first, or none, is `Moved` (the opened window is reaped for a new window).
/// - If `Seam`'s `share` is absent, [`target::seam_share`] is computed inside the same lock.
/// - [`DockOutcome::rows`] is taken in the lock that makes the outcome, so window · tab · identity · rows are one moment's picture.
// ADR-0307
pub fn dock_target(
    state: &LayoutState,
    subs: &dyn SubscriptionSync,
    events: &dyn LayoutEvents,
    host: &dyn WindowHost,
    labels: &dyn LabelSource,
    req: TargetDockRequest,
) -> Result<DockOutcome, TargetDockError> {
    let TargetDockRequest { tab, to, focus, mv } = req;
    let ask = DockAsk {
        focus,
        mv,
        rows: true,
    };
    let subject = |mgr: &ViewManager| locate(mgr, &tab).map_err(TargetDockError::Subject);
    let (mode, result) = match to {
        TargetDest::Seam {
            beside,
            whole,
            side,
            share,
        } => (
            "seam",
            dock_in_place(state, subs, events, host, &ask, |mgr| {
                let source = subject(mgr)?;
                let place = locate(mgr, &beside).map_err(TargetDockError::Dest)?;
                let anchor = if whole {
                    DockAnchor::Outer
                } else {
                    DockAnchor::Slot(place.slot.ok_or(TargetDockError::NotASlot)?)
                };
                let share =
                    share.unwrap_or_else(|| seam_share(mgr, source.view, place.view, anchor, side));
                Ok((
                    source.view,
                    DockTo::Seam {
                        target: place.view,
                        anchor,
                        side,
                        share,
                    },
                ))
            }),
        ),
        TargetDest::TabBar(spot) => (
            "tab_bar",
            dock_in_place(state, subs, events, host, &ask, |mgr| {
                let source = subject(mgr)?;
                let to = match spot {
                    TabBarSpot::At { window, index } => DockTo::TabBar {
                        window: window.unwrap_or(source.window),
                        index,
                        after: None,
                    },
                    TabBarSpot::After(anchor) => {
                        let anchor = locate(mgr, &anchor).map_err(TargetDockError::Dest)?;
                        DockTo::TabBar {
                            window: anchor.window,
                            index: None,
                            after: Some(anchor.view),
                        }
                    }
                };
                Ok((source.view, to))
            }),
        ),
        TargetDest::NewWindow { at } => (
            "new_window",
            dock_into_window(
                state,
                subs,
                events,
                host,
                labels,
                &ask,
                at,
                |mgr| subject(mgr).map(|source| source.view),
                |mgr, view, window| match subject(mgr) {
                    Ok(now) if now.view == view && window.is_none_or(|w| now.window == w) => Ok(()),
                    _ => Err(TargetDockError::Moved),
                },
            ),
        ),
    };
    log_dock(mode, &tab, &result);
    result.map(|(out, _)| out)
}

// ── Area · tab writes — target edition ─────────────────────────────────────────

/// The rejection of target-edition area · tab writes — the model is unchanged. (TRD S28 §7)
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TargetError {
    #[error("{0}")]
    Locate(#[from] LocateError),
    /// A write that changes an area was given a target that points at a tab, not an area (`Tab` · `TabOf`).
    #[error("pointed at a tab, not an area — give a target that points at an area (NotASlot)")]
    NotASlot,
    /// The target was resolved but the application was refused or the lock was poisoned — the text is that of the same id edition as is.
    #[error("{0}")]
    Apply(String),
}

/// The outcome of a target-edition write: `at` = the resolved place · `out` = that write's answer · `rows` = the layout rows of the tab that write changed, taken in the very lock that wrote (one moment's picture, as [`DockOutcome::rows`]).
// ADR-0307 (TRD S28 §7)
#[derive(Debug, Clone, PartialEq)]
pub struct Wrote<T> {
    pub at: Located,
    pub out: T,
    pub rows: Vec<LayoutRow>,
}

fn lock_for(state: &LayoutState) -> Result<std::sync::MutexGuard<'_, ViewManager>, TargetError> {
    state
        .0
        .lock()
        .map_err(|e| TargetError::Apply(e.to_string()))
}

fn applied(e: impl Display) -> TargetError {
    TargetError::Apply(e.to_string())
}

// A target that points at an area — (the resolved place, that area).
fn locate_slot(mgr: &ViewManager, target: &Target) -> Result<(Located, Uuid), TargetError> {
    let at = locate(mgr, target)?;
    let slot = at.slot.ok_or(TargetError::NotASlot)?;
    Ok((at, slot))
}

// One area write of tab `at.view` inside the very lock that resolved one target: re-sync (if `subs` exists), that tab's layout, that window's tab list and that tab's layout rows are in the same lock; the notification is outside the lock (same order as the id edition).
fn slot_write_at<T>(
    state: &LayoutState,
    subs: Option<&dyn SubscriptionSync>,
    events: &dyn LayoutEvents,
    target: &Target,
    act: impl FnOnce(&mut ViewManager, &Located, Uuid) -> Result<T, TargetError>,
) -> Result<Wrote<T>, TargetError> {
    let (wrote, layout, tabs) = {
        let mut mgr = lock_for(state)?;
        let (at, slot) = locate_slot(&mgr, target)?;
        let out = act(&mut mgr, &at, slot)?;
        let layout = mgr.snapshot(at.view).ok();
        let tabs = owner_tabs(&mgr, at.view);
        if let Some(subs) = subs {
            subs.resync(&mgr);
        }
        let rows = rows::rows_of_views(&mgr, &[at.view]);
        (Wrote { at, out, rows }, layout, tabs)
    };
    notify(events, layout, tabs);
    Ok(wrote)
}

/// The target edition of [`focus_slot`] (rows = that tab's areas, with the focus area changed).
// ADR-0066 · ADR-0307
pub fn focus_slot_at(
    state: &LayoutState,
    events: &dyn LayoutEvents,
    target: &Target,
) -> Result<Wrote<()>, TargetError> {
    slot_write_at(state, None, events, target, |mgr, at, slot| {
        mgr.set_focused_slot(at.view, slot).map_err(applied)
    })
}

/// The target edition of [`close_slot`] (rows = that tab's remaining areas).
pub fn close_slot_at(
    state: &LayoutState,
    subs: &dyn SubscriptionSync,
    events: &dyn LayoutEvents,
    target: &Target,
) -> Result<Wrote<()>, TargetError> {
    slot_write_at(state, Some(subs), events, target, |mgr, at, slot| {
        mgr.close_slot(at.view, slot).map_err(applied)
    })
}

/// The target edition of [`split_slot`] — `out` = the new area id.
pub fn split_slot_at(
    state: &LayoutState,
    subs: &dyn SubscriptionSync,
    events: &dyn LayoutEvents,
    target: &Target,
    dir: SplitDir,
) -> Result<Wrote<Uuid>, TargetError> {
    slot_write_at(state, Some(subs), events, target, |mgr, at, slot| {
        mgr.split_slot(at.view, slot, dir).map_err(applied)
    })
}

/// The target edition of [`assign_agent`]. It does not check that the agent exists in the daemon (the caller's job). (TRD S28 §7)
pub fn assign_agent_at(
    state: &LayoutState,
    subs: &dyn SubscriptionSync,
    events: &dyn LayoutEvents,
    target: &Target,
    agent_id: String,
) -> Result<Wrote<()>, TargetError> {
    slot_write_at(state, Some(subs), events, target, |mgr, at, slot| {
        mgr.assign_agent(at.view, slot, agent_id).map_err(applied)
    })
}

/// The target edition of [`set_slot_content`].
pub fn set_slot_content_at(
    state: &LayoutState,
    subs: &dyn SubscriptionSync,
    events: &dyn LayoutEvents,
    target: &Target,
    content: SlotContent,
) -> Result<Wrote<()>, TargetError> {
    slot_write_at(state, Some(subs), events, target, |mgr, at, slot| {
        mgr.set_slot_content(at.view, slot, content)
            .map_err(applied)
    })
}

/// The target edition of [`set_usage_slot`]; reading the current value of a missing field and writing share one lock.
pub fn set_usage_slot_at(
    state: &LayoutState,
    subs: &dyn SubscriptionSync,
    events: &dyn LayoutEvents,
    target: &Target,
    show_claude: Option<bool>,
    show_codex: Option<bool>,
) -> Result<Wrote<()>, TargetError> {
    slot_write_at(state, Some(subs), events, target, |mgr, at, slot| {
        let (claude_now, codex_now) = match mgr.slot_content(at.view, slot).map_err(applied)? {
            SlotContent::Usage {
                show_claude,
                show_codex,
            } => (show_claude, show_codex),
            _ => (true, true),
        };
        let content = SlotContent::Usage {
            show_claude: show_claude.unwrap_or(claude_now),
            show_codex: show_codex.unwrap_or(codex_now),
        };
        mgr.set_slot_content(at.view, slot, content)
            .map_err(applied)
    })
}

/// The target edition of [`switch_tab`]: makes the tab holding `target` the visible tab of its window. If `window` is given it must be a tab of that window (otherwise the same error as [`switch_tab`]). Returns the resolved place.
pub fn switch_tab_at(
    state: &LayoutState,
    subs: &dyn SubscriptionSync,
    events: &dyn LayoutEvents,
    window: Option<&str>,
    target: &Target,
) -> Result<Located, TargetError> {
    let (at, layout, tabs) = {
        let mut mgr = lock_for(state)?;
        let at = locate(&mgr, target)?;
        let window = window.unwrap_or(&at.window).to_string();
        mgr.switch_tab(&window, at.view).map_err(applied)?;
        let layout = mgr.snapshot(at.view).ok();
        let tabs = tabs_payload(&mgr, &window);
        subs.resync(&mgr);
        (at, layout, tabs)
    };
    notify(events, layout, tabs);
    Ok(at)
}

/// The target edition of [`close_tab`]; `window` as [`switch_tab_at`]. `out` = the tab now visible in that window (`None` if the window closed along with it) · `rows` = that tab's layout rows (empty if the window closed).
pub fn close_tab_at(
    state: &LayoutState,
    subs: &dyn SubscriptionSync,
    events: &dyn LayoutEvents,
    host: &dyn WindowHost,
    window: Option<&str>,
    target: &Target,
) -> Result<Wrote<Option<ViewId>>, TargetError> {
    let (at, window, outcome, shown, layout, tabs, rows) = {
        let mut mgr = lock_for(state)?;
        let at = locate(&mgr, target)?;
        let window = window.unwrap_or(&at.window).to_string();
        let outcome = mgr.close_tab(&window, at.view).map_err(applied)?;
        let (shown, layout, tabs) = match outcome {
            CloseTabOutcome::Stayed => {
                let shown = mgr.list_tabs(&window).ok().map(|s| s.active);
                let layout = shown.and_then(|a| mgr.snapshot(a).ok());
                (shown, layout, tabs_payload(&mgr, &window))
            }
            CloseTabOutcome::WindowClosed => (None, None, None),
        };
        subs.resync(&mgr);
        let rows = shown
            .map(|view| rows::rows_of_views(&mgr, &[view]))
            .unwrap_or_default();
        (at, window, outcome, shown, layout, tabs, rows)
    };
    match outcome {
        CloseTabOutcome::Stayed => notify(events, layout, tabs),
        CloseTabOutcome::WindowClosed => host.close(&window),
    }
    Ok(Wrote {
        at,
        out: shown,
        rows,
    })
}

/// The target edition of [`rename_tab`]. Returns the resolved place.
pub fn rename_tab_at(
    state: &LayoutState,
    events: &dyn LayoutEvents,
    target: &Target,
    name: String,
) -> Result<Located, TargetError> {
    let (at, tabs) = {
        let mut mgr = lock_for(state)?;
        let at = locate(&mgr, target)?;
        mgr.rename_tab(at.view, name).map_err(applied)?;
        let tabs = owner_tabs(&mgr, at.view);
        (at, tabs)
    };
    announce(events, None::<&ViewSnapshot>, tabs.as_ref());
    Ok(at)
}

// ── Layout rows · place lookup ──────────────────────────────────────────

/// The display name of agent `agent_id` (shell name ledger); `None` if not in the ledger or the lock is poisoned. ★For display such as error text only★: do not use it to pick an agent by name (the resolver is the daemon alone — ADR-0132 · ADR-0308). (TRD S28 §4-5)
pub fn agent_display_name(state: &LayoutState, agent_id: Uuid) -> Option<String> {
    let mgr = state.0.lock().ok()?;
    mgr.agent_name(&agent_id.to_string()).map(str::to_owned)
}

/// Takes [`rows::layout_rows`] in one lock. ★Query only★: no mutation, no notification.
// ADR-0307
pub fn layout_rows(state: &LayoutState, filter: &RowFilter) -> Result<Vec<LayoutRow>, String> {
    let mgr = state.0.lock().map_err(|e| e.to_string())?;
    Ok(rows::layout_rows(&mgr, filter))
}

/// Takes [`rows::rows_of_views`] in one lock. ★Do not use it for a docking command's answer★: it is a different lock from the docking, so writes in between can mix in (use the outcome's [`DockOutcome::rows`], same lock).
pub fn layout_rows_of(state: &LayoutState, views: &[ViewId]) -> Result<Vec<LayoutRow>, String> {
    let mgr = state.0.lock().map_err(|e| e.to_string())?;
    Ok(rows::rows_of_views(&mgr, views))
}

/// Takes [`target::agent_places`] in one lock. ★Do not use it to pick the apply target★: it runs after the lock was released and may change in between (picking belongs to the target-edition write inside its own lock).
pub fn agent_places(state: &LayoutState, agent: Uuid) -> Result<Vec<Place>, String> {
    let mgr = state.0.lock().map_err(|e| e.to_string())?;
    Ok(target::agent_places(&mgr, agent))
}

// ── Boot title ────────────────────────────────────────────────────────────────

/// Notifies once, at the current version, the tab list of every window of the model — the part that first applies the OS window title after the boot has built all windows
/// (TRD S26-window-chrome §4-4 ③). It does not change the model so it does not raise the version — a window the web already pulled has the same version, so that notification
/// is discarded (version guard).
pub fn sync_titles(state: &LayoutState, events: &dyn LayoutEvents) -> Result<(), String> {
    let tabs: Vec<WindowTabsPayload> = {
        let mgr = state.0.lock().map_err(|e| e.to_string())?;
        mgr.list_windows()
            .iter()
            .filter_map(|label| tabs_payload(&mgr, label))
            .collect()
    };
    announce(events, None::<&ViewSnapshot>, &tabs);
    Ok(())
}

// ── Two measurement reports ────────────────────────────────────────────────────────────
//
// `window` is the window label of the webview that reported — the thin wrapper takes it from the window Tauri supplied and passes it (not received as an argument).
// Ignoring 0 size · range rejection and keeping the previous value are borne by the manager (`ViewManager::set_window_canvas` · `set_ui_metrics`)
// .
// ADR-0227

pub fn report_window_canvas(
    state: &LayoutState,
    window: &str,
    w: u32,
    h: u32,
) -> Result<(), String> {
    let mut mgr = state.0.lock().map_err(|e| e.to_string())?;
    mgr.set_window_canvas(window, w, h)
        .map_err(|e| e.to_string())
}

pub fn report_ui_metrics(
    state: &LayoutState,
    window: &str,
    metrics: UiMetrics,
) -> Result<(), String> {
    let mut mgr = state.0.lock().map_err(|e| e.to_string())?;
    mgr.set_ui_metrics(window, metrics)
        .map_err(|e| e.to_string())
}

// ── Five read-only ────────────────────────────────────────────────────────────

// The path by which the LLM finds a split id — the snapshot (`get_view`) is a recursive type, so it cannot be carried on the command bus.
// ADR-0227
pub fn list_splits(state: &LayoutState, view_id: Uuid) -> Result<Vec<SplitInfo>, String> {
    let mgr = state.0.lock().map_err(|e| e.to_string())?;
    mgr.list_splits(view_id).map_err(|e| e.to_string())
}

// For the popup pull↔listen race. ★Query only★ — no mutation, no notification (version not raised).
pub fn get_view(state: &LayoutState, view_id: Uuid) -> Result<ViewSnapshot, String> {
    let mgr = state.0.lock().map_err(|e| e.to_string())?;
    mgr.snapshot(view_id).map_err(|e| e.to_string())
}

// Why needed: when a window mounts it must settle its own active tab (a popup knows its own window only by its own window label, and the
// active tab is backend authority), but the write path notifies only right after a change → right after boot/mount it takes
// `{tabs,active,version}` with this query and does the initial render (§3-3/G3).
pub fn list_tabs(state: &LayoutState, window: &str) -> Result<WindowTabsPayload, String> {
    let mgr = state.0.lock().map_err(|e| e.to_string())?;
    mgr.list_tabs(window)
        .map(Into::into)
        .map_err(|e| e.to_string())
}

pub fn list_windows(state: &LayoutState) -> Result<Vec<String>, String> {
    let mgr = state.0.lock().map_err(|e| e.to_string())?;
    Ok(mgr.list_windows())
}

// ★Spatial/direction token → slot id resolution (backend-authority resolver)★. If `view_id` is given, that View; if not,
// the active tab View of `window` (main if unspecified) is the target. Unknown token / nonexistent View → Err (fail-loud).
// ADR-0068
pub fn resolve_spatial(
    state: &LayoutState,
    token: &str,
    window: Option<&str>,
    view_id: Option<Uuid>,
) -> Result<Option<Uuid>, String> {
    let tok = SpatialToken::parse(token)
        .ok_or_else(|| format!("unknown spatial token: '{token}' (top-left/top-right/bottom-left/bottom-right/left/right/up/down)"))?;
    let mgr = state.0.lock().map_err(|e| e.to_string())?;
    let vid = match view_id {
        Some(v) => v,
        None => {
            let label = window.unwrap_or(MAIN_WINDOW_LABEL);
            mgr.list_tabs(label).map_err(|e| e.to_string())?.active
        }
    };
    let v = mgr
        .views
        .get(&vid)
        .ok_or_else(|| format!("view not found: {vid}"))?;
    Ok(resolve_spatial_token(&v.layout, v.focused_slot_id, tok))
}
