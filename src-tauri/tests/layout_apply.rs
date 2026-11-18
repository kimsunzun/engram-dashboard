//! Integration test of the layout apply service (`layout::apply`) — zero windows · daemon · Tauri (ADR-0012).
//! Lives in `tests/` because it measures wiring that needs a real socket / `AppHandle`, so the harness cuts those with ports; pure units go in `#[cfg(test)]` beside the module (run with
// hangul-ok-begin
//! `--test lib_unit`, ADR-0174, `.claude/skill-bindings/qa.md` 「모듈별 명령」), wiring goes here.
// hangul-ok-end
//! Run: `cargo test -p engram-dashboard --test layout_apply`, without `-- --test-threads=4`
// hangul-ok-begin
//! (no child process; rule = `.claude/skill-bindings/qa.md` 「모듈별 명령」). CI has a dedicated step for this target (`.github/workflows/ci.yml`).
// hangul-ok-end
//! Label issuing uses the real `PopupCounter` (only a probe shell around it) and subscription resync the real `OutputRouter` — fakes would verify the fake, not label monotonicity · routing. The other three ports (notification · OS window · spawn) are recording fakes.
//!
//! ## Lock position probe
//! The invariant is which port is called inside the lock and which outside; counting calls cannot catch a moved position. Each fake port checks `try_lock` at the place it is called:
//! - `LayoutEvents` · `WindowHost` · `LabelSource` · `AgentSpawner` → not `WouldBlock` (outside the lock; a poisoned lock counts as outside). `WindowHost::close` inside the lock is a production commit deadlock (`w.destroy()` → `Destroyed` → re-acquiring the lock).
//! - `SubscriptionSync::resync` → `is_err()` (inside the lock); outside brings back the F1/F2 stale-unsubscribe defect.
//!
//! `try_lock` gives `WouldBlock` without blocking even when the same thread holds the lock, so this needs no threads or timing — but it presumes a single-threaded test (another thread holding the lock could pass `is_err()` falsely). The real Tauri adapter needs a window (GUI measurement).

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock, Mutex, TryLockError};

use engram_dashboard_protocol::AgentBackendKind;
use uuid::Uuid;

use engram_dashboard_lib::commands::popout::{popup_label, PopupCounter};
use engram_dashboard_lib::layout::apply::{
    self, AgentSpawner, DockDest, DockOutcome, DockRequest, LabelSource, LayoutEvents, MoveFailed,
    MoveStep, NewWindow, SpareClaim, SubscriptionSync, TabBarSpot, TargetDest, TargetDockError,
    TargetDockRequest, TargetError, WindowHost, WindowPlace, WindowTabsPayload,
};
use engram_dashboard_lib::layout::bands::{attach_share, PointCss, SizeCss};
use engram_dashboard_lib::layout::dock::{DockAnchor, DockSide, DOCK_NEW_SHARE};
use engram_dashboard_lib::layout::geometry::{Insets, PxRect};
use engram_dashboard_lib::layout::manager::{DockError, DockMove, DockTo, FocusPolicy};
use engram_dashboard_lib::layout::rows::{LayoutRow, RowFilter, SlotKind};
use engram_dashboard_lib::layout::target::{
    self as target, LocateError, Located, Target, TargetBy,
};
use engram_dashboard_lib::layout::window_ctl::{self, CloseOutcome};
use engram_dashboard_lib::layout::{
    tree, LayoutNode, LayoutState, SlotContent, SpatialToken, SplitDir, SplitRatioOutcome,
    UiMetrics, ViewManager, ViewSnapshot, WindowBounds, WindowPlacement, MAIN_WINDOW_LABEL,
};
use engram_dashboard_lib::output_router::OutputRouter;
use engram_dashboard_lib::tab_drop::plan::{
    self as drop_plan, InputNow, PhysPoint, Plan, SourceFrame, TopAt,
};
use engram_dashboard_lib::tab_drop::wire::{DropAnchor, DropTo, NewWindowBy, TabDrop};
use engram_dashboard_lib::ui_text::Lang;

// ── lock position probe ───────────────────────────────────────────────────────

struct Probe(LayoutState);

impl Probe {
    fn new(state: &LayoutState) -> Self {
        Probe(state.clone())
    }

    fn assert_outside(&self, who: &str) {
        assert!(
            !matches!(self.0 .0.try_lock(), Err(TryLockError::WouldBlock)),
            "{who} must be called after the lock is dropped (inside the lock, window destroy · emit deadlocks or re-enters)"
        );
    }

    fn assert_inside(&self, who: &str) {
        assert!(
            self.0 .0.try_lock().is_err(),
            "{who} must be called while holding the lock (outside it, a re-add between computation and firing releases a stale one — F1/F2)"
        );
    }
}

// ── fake ports ────────────────────────────────────────────────────────────────

/// Order across the window port and the notification port (one shared record; TRD S21-slot-docking §8-1 ⑬). Window port: `"open:<label>"` · `"open_at:<label>"` · `"close:<label>"` · `"place:<label>"` · prewarm `"claim:<label>"` (only when handed over) · `"committed:<label>"` · `"release:<label>"`; notification: `"layout"` · `"tabs:<label>"`.
#[derive(Clone, Default)]
struct Journal(Arc<Mutex<Vec<String>>>);

impl Journal {
    fn note(&self, entry: String) {
        self.0.lock().unwrap().push(entry);
    }

    fn take(&self) -> Vec<String> {
        std::mem::take(&mut *self.0.lock().unwrap())
    }
}

struct Recorder {
    probe: Probe,
    layout: Mutex<Vec<ViewSnapshot>>,
    tabs: Mutex<Vec<WindowTabsPayload>>,
    order: Mutex<Vec<&'static str>>,
    journal: Journal,
    /// Runs once outside the lock at the first tab notification — another path changed the model after the new window entered it, before the commit.
    on_tabs: Mutex<Option<DuringOpen>>,
}

impl Recorder {
    fn new(state: &LayoutState, journal: Journal) -> Self {
        Recorder {
            probe: Probe::new(state),
            layout: Mutex::new(Vec::new()),
            tabs: Mutex::new(Vec::new()),
            order: Mutex::new(Vec::new()),
            journal,
            on_tabs: Mutex::new(None),
        }
    }

    fn take_order(&self) -> Vec<&'static str> {
        std::mem::take(&mut *self.order.lock().unwrap())
    }
}

impl LayoutEvents for Recorder {
    fn layout_updated(&self, snapshot: &ViewSnapshot) {
        self.probe.assert_outside("LayoutEvents::layout_updated");
        self.layout.lock().unwrap().push(snapshot.clone());
        self.order.lock().unwrap().push("layout");
        self.journal.note("layout".to_string());
    }

    fn window_tabs_updated(&self, tabs: &WindowTabsPayload) {
        self.probe
            .assert_outside("LayoutEvents::window_tabs_updated");
        self.tabs.lock().unwrap().push(tabs.clone());
        self.order.lock().unwrap().push("tabs");
        self.journal.note(format!("tabs:{}", tabs.label));
        if let Some(hook) = self.on_tabs.lock().unwrap().take() {
            hook(&self.probe.0);
        }
    }
}

struct Subs {
    probe: Probe,
    router: OutputRouter,
    resyncs: AtomicUsize,
    unsubscribed: Mutex<Vec<Uuid>>,
}

impl Subs {
    fn new(state: &LayoutState) -> Self {
        Subs {
            probe: Probe::new(state),
            router: OutputRouter::new(),
            resyncs: AtomicUsize::new(0),
            unsubscribed: Mutex::new(Vec::new()),
        }
    }
}

impl SubscriptionSync for Subs {
    fn resync(&self, mgr: &ViewManager) {
        self.probe.assert_inside("SubscriptionSync::resync");
        self.resyncs.fetch_add(1, Ordering::Relaxed);
        let delta = self.router.rebuild(mgr);
        self.unsubscribed
            .lock()
            .unwrap()
            .extend(delta.to_unsubscribe);
    }
}

/// Interjection `open` calls once outside the lock — another command changed the model during the window build (phase B). `move_window`'s `during_move` has the same shape.
type DuringOpen = Box<dyn FnOnce(&LayoutState) + Send>;

struct Host {
    probe: Probe,
    fail_open: bool,
    opened: Mutex<Vec<String>>,
    closed: Mutex<Vec<String>>,
    placed: Mutex<Vec<String>>,
    during_open: Mutex<Option<DuringOpen>>,
    opened_at: Mutex<Vec<(String, WindowPlace, bool)>>,
    moves: Mutex<Vec<(String, WindowPlace)>>,
    steps: Mutex<Vec<MoveStep>>,
    fail_steps: Mutex<Vec<MoveStep>>,
    /// Runs once outside the lock before `move_window` calls the setters — another path changed the source while moving.
    during_move: Mutex<Option<DuringOpen>>,
    journal: Journal,
    /// Label of the hidden prewarmed window to hand over (empties on handover, returns on take-back; TRD S21-slot-docking §8-1 ⑬). The pool state machine is not imitated — only call order and the claim ticket.
    spare: Mutex<Option<String>>,
    /// Claim ticket currently handed over — commit · take-back must return an equal one exactly once.
    claimed: Mutex<Option<SpareClaim>>,
    claims_asked: AtomicUsize,
    generation: AtomicU32,
    /// Label the test removed from the model between entering it and the commit — `record_placement` does not require a model entry.
    vanished: Mutex<Vec<String>>,
    /// If true, `close` only records — a window removed from the model while its OS window remains (TRD S26-window-chrome §2-3).
    close_does_nothing: AtomicBool,
}

impl Host {
    fn new(state: &LayoutState, fail_open: bool, journal: Journal) -> Self {
        Host {
            probe: Probe::new(state),
            journal,
            fail_open,
            opened: Mutex::new(Vec::new()),
            closed: Mutex::new(Vec::new()),
            placed: Mutex::new(Vec::new()),
            during_open: Mutex::new(None),
            opened_at: Mutex::new(Vec::new()),
            moves: Mutex::new(Vec::new()),
            steps: Mutex::new(Vec::new()),
            fail_steps: Mutex::new(Vec::new()),
            during_move: Mutex::new(None),
            spare: Mutex::new(None),
            claimed: Mutex::new(None),
            claims_asked: AtomicUsize::new(0),
            generation: AtomicU32::new(0),
            vanished: Mutex::new(Vec::new()),
            close_does_nothing: AtomicBool::new(false),
        }
    }

    // Blows up if the same claim ticket or a different one is received.
    fn take_claim(&self, claim: &SpareClaim, who: &str) {
        let held = self.claimed.lock().unwrap().take();
        assert_eq!(
            held.as_ref(),
            Some(claim),
            "{who} must receive the handed-over entrust ticket as is, exactly once"
        );
    }

    fn opening(&self, kind: &str, label: &str) -> Result<(), String> {
        assert!(
            !self.probe.0 .0.lock().unwrap().windows.contains_key(label),
            "{kind}({label}) must be called before the window enters the model"
        );
        self.journal.note(format!("{kind}:{label}"));
        if let Some(hook) = self.during_open.lock().unwrap().take() {
            hook(&self.probe.0);
        }
        self.opened.lock().unwrap().push(label.to_string());
        if self.fail_open {
            // Port contract: if the handed-over prewarmed window could not be dropped, it withdraws that window and gives `Err`.
            let mut claimed = self.claimed.lock().unwrap();
            if claimed.as_ref().is_some_and(|c| c.label == label) {
                *claimed = None;
            }
            return Err("window creation failed (test)".to_string());
        }
        Ok(())
    }

    fn fail_step(&self, step: MoveStep) {
        self.fail_steps.lock().unwrap().push(step);
    }

    fn steps(&self) -> Vec<MoveStep> {
        self.steps.lock().unwrap().clone()
    }

    fn setter(&self, step: MoveStep) -> Result<(), MoveFailed> {
        self.steps.lock().unwrap().push(step);
        if self.fail_steps.lock().unwrap().contains(&step) {
            return Err(MoveFailed {
                step,
                error: format!("{step:?} setter failed (test)"),
            });
        }
        Ok(())
    }
}

impl WindowHost for Host {
    // Must come before entering the model: all three new-window places open the window first (TRD S21-slot-docking §3-9 · ADR-0299).
    fn open(&self, label: &str) -> Result<(), String> {
        self.probe.assert_outside("WindowHost::open");
        self.opening("open", label)
    }

    fn open_at(&self, label: &str, at: &WindowPlace, activate: bool) -> Result<(), String> {
        self.probe.assert_outside("WindowHost::open_at");
        self.opened_at
            .lock()
            .unwrap()
            .push((label.to_string(), *at, activate));
        self.opening("open_at", label)
    }

    fn claim_spare(&self) -> Option<SpareClaim> {
        self.probe.assert_outside("WindowHost::claim_spare");
        self.claims_asked.fetch_add(1, Ordering::Relaxed);
        let label = self.spare.lock().unwrap().take()?;
        let claim = SpareClaim {
            label: label.clone(),
            generation: self.generation.fetch_add(1, Ordering::Relaxed) + 1,
        };
        *self.claimed.lock().unwrap() = Some(claim.clone());
        self.journal.note(format!("claim:{label}"));
        Some(claim)
    }

    fn committed(&self, claim: &SpareClaim) {
        self.probe.assert_outside("WindowHost::committed");
        self.take_claim(claim, "committed");
        self.journal.note(format!("committed:{}", claim.label));
    }

    fn release_spare(&self, claim: &SpareClaim) {
        self.probe.assert_outside("WindowHost::release_spare");
        self.take_claim(claim, "release_spare");
        self.journal.note(format!("release:{}", claim.label));
        *self.spare.lock().unwrap() = Some(claim.label.clone());
    }

    // Contract = port docs: position → size, stops at the first failure; an empty field skips that step.
    fn move_window(&self, label: &str, at: &WindowPlace) -> Result<(), MoveFailed> {
        self.probe.assert_outside("WindowHost::move_window");
        self.moves.lock().unwrap().push((label.to_string(), *at));
        if let Some(hook) = self.during_move.lock().unwrap().take() {
            hook(&self.probe.0);
        }
        if at.pos.is_some() {
            self.setter(MoveStep::Position)?;
        }
        if at.size.is_some() {
            self.setter(MoveStep::Size)?;
        }
        Ok(())
    }

    fn close(&self, label: &str) {
        self.probe.assert_outside("WindowHost::close");
        self.journal.note(format!("close:{label}"));
        if self.close_does_nothing.load(Ordering::Relaxed) {
            return;
        }
        self.closed.lock().unwrap().push(label.to_string());
    }

    // Must come after entering the model: production writes the placement into that label's model entry, so calling first discards the first placement.
    fn record_placement(&self, label: &str) {
        self.probe.assert_outside("WindowHost::record_placement");
        assert!(
            self.probe.0 .0.lock().unwrap().windows.contains_key(label)
                || self.vanished.lock().unwrap().iter().any(|l| l == label),
            "record_placement({label}) must be called after the window enters the model"
        );
        self.journal.note(format!("place:{label}"));
        self.placed.lock().unwrap().push(label.to_string());
    }

    // True for main (a static config window this fake never opened but always up in the real app; returning a popup slot to main uses it); otherwise only windows this fake opened and did not close.
    fn is_open(&self, label: &str) -> bool {
        self.probe.assert_outside("WindowHost::is_open");
        label == MAIN_WINDOW_LABEL
            || (self.opened.lock().unwrap().iter().any(|l| l == label)
                && !self.closed.lock().unwrap().iter().any(|l| l == label))
    }
}

// Wraps the real issuer rather than replacing it: the inner one keeps label monotonicity and this shell only adds a view of the call position. Today's implementation is pure, so only this probe pins that place — a state-touching one would deadlock with `Destroyed → cleanup_popup_window → state.0.lock()`.
struct Labels {
    probe: Probe,
    inner: PopupCounter,
}

impl LabelSource for Labels {
    fn next_label(&self) -> String {
        self.probe.assert_outside("LabelSource::next_label");
        LabelSource::next_label(&self.inner)
    }
}

struct Spawner {
    probe: Probe,
    reply: Result<String, String>,
    calls: AtomicUsize,
    cwds: Mutex<Vec<String>>,
    /// Did the chosen backend reach the port — the call count alone cannot tell.
    backends: Mutex<Vec<Option<AgentBackendKind>>>,
    during: Mutex<Option<DuringOpen>>,
}

impl Spawner {
    fn ok(state: &LayoutState, agent: Uuid) -> Self {
        Spawner::new(state, Ok(agent.to_string()))
    }

    fn failing(state: &LayoutState) -> Self {
        Spawner::new(state, Err("spawn failed: daemon refused".to_string()))
    }

    fn new(state: &LayoutState, reply: Result<String, String>) -> Self {
        Spawner {
            probe: Probe::new(state),
            reply,
            calls: AtomicUsize::new(0),
            cwds: Mutex::new(Vec::new()),
            backends: Mutex::new(Vec::new()),
            during: Mutex::new(None),
        }
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::Relaxed)
    }
}

impl AgentSpawner for Spawner {
    fn spawn_by_cwd<'a>(
        &'a self,
        cwd: String,
        backend: Option<AgentBackendKind>,
    ) -> Pin<Box<dyn Future<Output = Result<String, String>> + Send + 'a>> {
        Box::pin(async move {
            // An await inside the lock would make the command future lose `Send` and not compile; this probe checks the same wall as the sibling port.
            self.probe.assert_outside("AgentSpawner::spawn_by_cwd");
            if let Some(hook) = self.during.lock().unwrap().take() {
                hook(&self.probe.0);
            }
            self.calls.fetch_add(1, Ordering::Relaxed);
            self.cwds.lock().unwrap().push(cwd);
            self.backends.lock().unwrap().push(backend);
            self.reply.clone()
        })
    }
}

// ── harness ───────────────────────────────────────────────────────────────────

struct World {
    state: LayoutState,
    subs: Subs,
    ev: Recorder,
    host: Host,
    // Real issuer (probe wrapped): the prefix `slot-popup-` pairs with the capabilities/popup.json glob; monotonicity prevents a closed-label re-build error.
    labels: Labels,
    journal: Journal,
}

impl World {
    fn new() -> Self {
        World::build(false)
    }

    fn failing_host() -> Self {
        World::build(true)
    }

    fn build(fail_open: bool) -> Self {
        let state = LayoutState::new();
        let journal = Journal::default();
        World {
            subs: Subs::new(&state),
            ev: Recorder::new(&state, journal.clone()),
            host: Host::new(&state, fail_open, journal.clone()),
            labels: Labels {
                probe: Probe::new(&state),
                inner: PopupCounter::default(),
            },
            journal,
            state,
        }
    }

    fn main_active(&self) -> Uuid {
        apply::list_tabs(&self.state, MAIN_WINDOW_LABEL)
            .expect("the main window always exists")
            .active
    }

    fn snapshot(&self, view: Uuid) -> ViewSnapshot {
        apply::get_view(&self.state, view).expect("view exists")
    }

    fn slots(&self, view: Uuid) -> Vec<Uuid> {
        self.snapshot(view)
            .slot_spatial
            .iter()
            .map(|s| s.slot_id)
            .collect()
    }

    fn empty_slot(&self, view: Uuid) -> Uuid {
        tree::first_empty_slot_id(&self.snapshot(view).layout).expect("an empty slot exists")
    }

    fn resyncs(&self) -> usize {
        self.subs.resyncs.load(Ordering::Relaxed)
    }

    fn layout_events(&self) -> usize {
        self.ev.layout.lock().unwrap().len()
    }

    fn tab_events(&self) -> usize {
        self.ev.tabs.lock().unwrap().len()
    }

    fn last_tabs(&self) -> WindowTabsPayload {
        self.ev
            .tabs
            .lock()
            .unwrap()
            .last()
            .cloned()
            .expect("tab notification")
    }

    fn unsubscribed(&self) -> Vec<Uuid> {
        self.subs.unsubscribed.lock().unwrap().clone()
    }

    fn popup(&self) -> String {
        apply::create_window(&self.state, &self.subs, &self.ev, &self.host, &self.labels)
            .expect("window creation")
            .window
    }

    // One hidden prewarmed window for the port to hand over; takes a label from the issuer as in production. Returns that label.
    fn prewarm(&self) -> String {
        let label = LabelSource::next_label(&self.labels.inner);
        *self.host.spare.lock().unwrap() = Some(label.clone());
        label
    }

    fn claim_outstanding(&self) -> bool {
        self.host.claimed.lock().unwrap().is_some()
    }

    fn claims_asked(&self) -> usize {
        self.host.claims_asked.load(Ordering::Relaxed)
    }

    // A path that opens no new window must not ask for a prewarmed one — an unfinished handover blocks the one-field pool (TRD S21-slot-docking §3-9 I27). `asked` = asks before the call.
    fn assert_spare_not_asked(&self, spare: &str, asked: usize) {
        assert_eq!(
            self.claims_asked(),
            asked,
            "a path that is not a new window asked for a prewarmed window"
        );
        assert_eq!(
            self.host.spare.lock().unwrap().as_deref(),
            Some(spare),
            "the prewarmed window stays without being handed over"
        );
    }
}

fn poison(state: &LayoutState) {
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _guard = state.0.lock().unwrap();
        panic!("lock poisoned (test)");
    }));
    assert!(state.0.is_poisoned());
}

// ── verifying the probe itself ────────────────────────────────────────────────

// Evidence the probe bites both ways. The "outside" probe was verified by moving service code inside the lock and watching it fail (WindowHost · LayoutEvents); for the "inside" probe no compilable counterexample exists (`resync` takes `&ViewManager`, so the type already guards it), so this proves the probe is not a dead assertion — what remains is catching the port shape changing (e.g. an owned snapshot callable without a guard).
#[test]
fn probe_fires_on_both_sides_of_the_lock() {
    use std::panic::{catch_unwind, AssertUnwindSafe};

    let state = LayoutState::new();
    let probe = Probe::new(&state);

    probe.assert_outside("lock not held");
    assert!(
        catch_unwind(AssertUnwindSafe(|| probe.assert_inside("false claim"))).is_err(),
        "claiming 「inside」 while the lock is not held must make the probe blow up"
    );

    let guard = state.0.lock().unwrap();
    probe.assert_inside("lock held");
    assert!(
        catch_unwind(AssertUnwindSafe(|| probe.assert_outside("false claim"))).is_err(),
        "claiming 「outside」 while the lock is held must make the probe blow up"
    );
    drop(guard);
}

// Pin that the fake imitates the port contract (moving stops at the first failure; an empty field is skipped) — docking tests rely on its step record · failure planting.
#[test]
fn the_fake_window_host_keeps_the_port_contract() {
    let w = World::new();
    let label = w.popup();
    let at = WindowPlace {
        pos: Some((10, 20)),
        size: Some((300.0, 200.0)),
    };

    w.host.fail_step(MoveStep::Position);
    let err = w.host.move_window(&label, &at).unwrap_err();
    assert_eq!(err.step, MoveStep::Position);
    assert_eq!(
        w.host.steps(),
        [MoveStep::Position],
        "the size step is not called after the first failure"
    );
    let size_only = WindowPlace {
        pos: None,
        size: Some((300.0, 200.0)),
    };
    *w.host.during_move.lock().unwrap() = Some(Box::new(|state: &LayoutState| {
        assert!(state.0.try_lock().is_ok(), "the interjection is outside the lock");
    }));
    w.host.move_window(&label, &size_only).unwrap();
    assert_eq!(
        w.host.steps()[1..],
        [MoveStep::Size],
        "an empty field skips that step"
    );
    assert_eq!(w.host.moves.lock().unwrap().len(), 2);

    w.host.open_at("slot-popup-99", &at, false).unwrap();
    assert!(w.host.is_open("slot-popup-99"), "open_at also counts as an opened window");
    assert_eq!(
        w.host.opened_at.lock().unwrap().last(),
        Some(&("slot-popup-99".to_string(), at, false))
    );
}

// ── create_tab ───────────────────────────────────────────────────────────────

#[test]
fn create_tab_adds_tab_and_notifies() {
    let w = World::new();
    let id = apply::create_tab(&w.state, &w.subs, &w.ev, MAIN_WINDOW_LABEL, None).unwrap();

    let tabs = apply::list_tabs(&w.state, MAIN_WINDOW_LABEL).unwrap();
    assert_eq!(tabs.active, id, "the new tab is active");
    assert_eq!(tabs.tabs.len(), 2);
    assert_eq!(w.layout_events(), 1);
    assert_eq!(w.tab_events(), 1);
    assert_eq!(w.resyncs(), 1);
}

#[test]
fn create_tab_unknown_window_is_err_without_notify() {
    let w = World::new();
    let err = apply::create_tab(&w.state, &w.subs, &w.ev, "no-such", None).unwrap_err();
    assert!(err.contains("window not found"), "err={err}");
    assert_eq!(w.layout_events(), 0);
    assert_eq!(w.tab_events(), 0);
    assert_eq!(w.resyncs(), 0, "mutation failure = no re-sync");
}

// ── create_window ────────────────────────────────────────────────────────────

#[test]
fn create_window_registers_model_and_opens_os_window() {
    let w = World::new();
    let version = w.version();
    let label = apply::create_window(&w.state, &w.subs, &w.ev, &w.host, &w.labels)
        .unwrap()
        .window;

    assert_eq!(&*w.host.opened.lock().unwrap(), &[label.clone()]);
    assert_eq!(
        &*w.host.placed.lock().unwrap(),
        &[label.clone()],
        "first place record once"
    );
    assert!(apply::list_windows(&w.state).unwrap().contains(&label));
    assert_eq!(
        apply::list_tabs(&w.state, &label).unwrap().tabs.len(),
        1,
        "one empty tab"
    );
    // One tab list of a new window — material for the OS title (TRD S26-window-chrome §2-4 ② · §4-4). The web pulls the layout at mount.
    assert_eq!(w.layout_events(), 0);
    assert_eq!(w.tab_events(), 1);
    let notified = w.last_tabs();
    assert_eq!(notified, apply::list_tabs(&w.state, &label).unwrap());
    assert_eq!(notified.title, "Empty slot - Engram:2");
    assert!(notified.version > version, "notified with a new version");
}

// TRD S21-slot-docking §8-1 ⑬ — fresh-build order: `open` → into the model → tab notification → `record_placement`. Opening before entering the model is measured by the fake `open`; the rejected-put branch (tombstone) is measured by the `commands::popout` unit test (the tombstone setter is not reachable from this crate).
#[test]
fn create_window_opens_then_inserts_then_records_the_placement() {
    let w = World::new();
    *w.host.during_open.lock().unwrap() = Some(Box::new(|state: &LayoutState| {
        assert!(
            !state
                .0
                .lock()
                .unwrap()
                .windows
                .contains_key(&popup_label(1)),
            "while opening, the model does not know that window"
        );
    }));

    let label = apply::create_window(&w.state, &w.subs, &w.ev, &w.host, &w.labels)
        .unwrap()
        .window;

    assert_eq!(
        w.journal.take(),
        [
            format!("open:{label}"),
            format!("tabs:{label}"),
            format!("place:{label}")
        ]
    );
    assert_eq!(w.resyncs(), 1, "one insertion");
}

#[test]
fn create_window_open_failure_leaves_the_model_untouched() {
    let w = World::failing_host();
    let version = apply::list_tabs(&w.state, MAIN_WINDOW_LABEL)
        .unwrap()
        .version;

    let err = apply::create_window(&w.state, &w.subs, &w.ev, &w.host, &w.labels).unwrap_err();
    assert!(err.contains("window creation failed"), "err={err}");

    let label = w.host.opened.lock().unwrap()[0].clone();
    assert!(
        !apply::list_windows(&w.state).unwrap().contains(&label),
        "a window that failed to open does not enter the model — no ghost windows"
    );
    assert_eq!(
        apply::list_tabs(&w.state, MAIN_WINDOW_LABEL)
            .unwrap()
            .version,
        version
    );
    assert_eq!(w.resyncs(), 0);
    assert_eq!(
        w.journal.take(),
        [format!("open:{label}")],
        "no close · no place record"
    );
}

// ── create_window: prewarmed window (TRD S21-slot-docking §8-1 ⑬ · ADR-0299) ──

#[test]
fn create_window_opens_the_prewarmed_window_inserts_then_commits_before_the_placement() {
    let w = World::new();
    let spare = w.prewarm();

    let label = apply::create_window(&w.state, &w.subs, &w.ev, &w.host, &w.labels)
        .unwrap()
        .window;

    assert_eq!(label, spare, "opens with the handed-over window's label");
    assert_eq!(
        w.journal.take(),
        [
            format!("claim:{label}"),
            format!("open:{label}"),
            format!("tabs:{label}"),
            format!("committed:{label}"),
            format!("place:{label}"),
        ]
    );
    assert!(apply::list_windows(&w.state).unwrap().contains(&label));
    assert!(!w.claim_outstanding());
    let next = apply::create_window(&w.state, &w.subs, &w.ev, &w.host, &w.labels)
        .unwrap()
        .window;
    assert_eq!(next, popup_label(2), "with no window to hand over, the issuer's new label");
}

// If the handed-over window's `open` is `Err`, the window port has already withdrawn that window — no commit, no take-back, and the model is unchanged.
#[test]
fn create_window_whose_prewarmed_window_fails_to_open_neither_commits_nor_releases() {
    let w = World::failing_host();
    let spare = w.prewarm();

    let err = apply::create_window(&w.state, &w.subs, &w.ev, &w.host, &w.labels).unwrap_err();

    assert!(err.contains("window creation failed"), "err={err}");
    assert_eq!(
        w.journal.take(),
        [format!("claim:{spare}"), format!("open:{spare}")]
    );
    assert!(!apply::list_windows(&w.state).unwrap().contains(&spare));
    assert_eq!(w.resyncs(), 0);
}

// If putting into the model is rejected (a window of the same label stood in the model while opening), the handed-over window is not closed but taken back.
#[test]
fn create_window_insert_refusal_gives_the_prewarmed_window_back() {
    let w = World::new();
    let spare = w.prewarm();
    let taken = spare.clone();
    *w.host.during_open.lock().unwrap() = Some(Box::new(move |state: &LayoutState| {
        state.0.lock().unwrap().create_window(&taken).unwrap();
    }));

    assert!(apply::create_window(&w.state, &w.subs, &w.ev, &w.host, &w.labels).is_err());

    assert_eq!(
        w.journal.take(),
        [
            format!("claim:{spare}"),
            format!("open:{spare}"),
            format!("release:{spare}"),
        ],
        "no close · confirm · place record"
    );
    assert!(!w.claim_outstanding());
}

// Even if the lock is poisoned after opening and it cannot be put in, no opened window remains — a newly built window is closed, and a handed-over window is taken back.
#[test]
fn create_window_on_a_poisoned_lock_withdraws_the_window_it_opened() {
    for prewarmed in [false, true] {
        let w = World::new();
        let spare = prewarmed.then(|| w.prewarm());
        *w.host.during_open.lock().unwrap() = Some(Box::new(poison));

        assert!(apply::create_window(&w.state, &w.subs, &w.ev, &w.host, &w.labels).is_err());

        let journal = w.journal.take();
        let expected = match spare {
            Some(label) => vec![
                format!("claim:{label}"),
                format!("open:{label}"),
                format!("release:{label}"),
            ],
            None => vec![
                format!("open:{}", popup_label(1)),
                format!("close:{}", popup_label(1)),
            ],
        };
        assert_eq!(journal, expected, "prewarmed={prewarmed}");
        assert!(w.host.placed.lock().unwrap().is_empty());
    }
}

// Passes through the real `PopupCounter`: the prefix pairs with the capabilities/popup.json glob (another label skips the Destroyed cleanup gate and leaks routing · subscriptions · Channels), and monotonicity prevents a closed-label re-build error. `popout.rs` has its own unit test of this contract; this one measures a different layer (through the service).
#[test]
fn create_window_issues_monotonic_popup_labels_from_real_counter() {
    let w = World::new();
    let a = w.popup();
    let b = w.popup();
    assert_eq!(a, popup_label(1));
    assert_eq!(b, popup_label(2));

    apply::close_window(&w.state, &w.subs, &w.host, &a).unwrap();
    let c = w.popup();
    assert_eq!(c, popup_label(3), "a closed label is not reused");
    assert!(apply::list_windows(&w.state).unwrap().contains(&c));
}

// ── window number (TRD S26-window-chrome §4-1 · §10-2) ──

fn number_of(w: &World, label: &str) -> u32 {
    apply::list_tabs(&w.state, label).unwrap().number
}

// First tab notification of a new window carries number · title with that number · the version just written (TRD S26-window-chrome §10-2). `tab` = the active tab's visible name.
fn assert_new_window_announced(w: &World, label: &str, tab: &str) {
    let notified = w.notified_tabs_of(label);
    let number = number_of(w, label);
    assert_eq!(notified.number, number);
    assert_eq!(notified.title, format!("{tab} - Engram:{number}"));
    assert_eq!(notified.version, w.version(), "{label} — the version that inserted the new window");
}

// The identity in the new-window answer equals that window's tab list — persistent id · number (TRD S26-window-chrome §5-3 · §10-2).
fn assert_answer_names(w: &World, answer: &NewWindow) {
    let tabs = apply::list_tabs(&w.state, &answer.window).unwrap();
    assert_eq!(
        (answer.window_id.as_str(), answer.number),
        (tabs.window_id.as_str(), tabs.number),
        "{} — persistent id · number of the answer",
        answer.window
    );
}

// The three new-window places (prewarmed or newly built) get the smallest free number; the tab list payload carries number · title · persistent id, and the answer the persistent id · number.
#[test]
fn every_new_window_path_numbers_the_window_and_the_tabs_payload_carries_it() {
    let w = World::new();
    let main = apply::list_tabs(&w.state, MAIN_WINDOW_LABEL).unwrap();
    assert_eq!((main.number, main.window_id.as_str()), (1, "main"));
    assert_eq!(main.title, "Empty slot - Engram:1");

    let mut labels = Vec::new();
    for prewarmed in [true, false] {
        let spare = prewarmed.then(|| w.prewarm());
        let created = apply::create_window(&w.state, &w.subs, &w.ev, &w.host, &w.labels).unwrap();
        assert_new_window_announced(&w, &created.window, "Empty slot");
        assert_answer_names(&w, &created);
        if let Some(spare) = spare {
            assert_eq!(created.window, spare, "answers with the prewarmed window's label");
        }
        labels.push(created.window);

        let spare = prewarmed.then(|| w.prewarm());
        let (view, slot, _) = w.view_with_filled_slot();
        let moved = apply::move_slot_to_window(
            &w.state, &w.subs, &w.ev, &w.host, &w.labels, view, slot, None,
        )
        .unwrap();
        assert_new_window_announced(&w, &moved.window, "Agent");
        assert_answer_names(
            &w,
            &NewWindow {
                window: moved.window.clone(),
                window_id: moved.window_id.clone(),
                number: moved.number,
            },
        );
        if let Some(spare) = spare {
            assert_eq!(moved.window, spare, "answers with the prewarmed window's label");
        }
        labels.push(moved.window);

        let spare = prewarmed.then(|| w.prewarm());
        let source = apply::create_tab(&w.state, &w.subs, &w.ev, MAIN_WINDOW_LABEL, None).unwrap();
        let out = w
            .dock(source, DockDest::NewWindow { at: AT }, FocusPolicy::Keep)
            .unwrap();
        assert_eq!(out.prewarmed, prewarmed);
        assert_new_window_announced(&w, &out.window, "Empty slot");
        let opened = out
            .identity
            .clone()
            .expect("the new-window path carries that window's identity");
        assert_eq!(opened.window, out.window);
        assert_answer_names(&w, &opened);
        if let Some(spare) = spare {
            assert_eq!(out.window, spare, "answers with the prewarmed window's label");
        }
        labels.push(out.window);
    }

    let numbers: Vec<u32> = labels.iter().map(|l| number_of(&w, l)).collect();
    assert_eq!(numbers, [2, 3, 4, 5, 6, 7]);
    let mut ids: Vec<String> = labels
        .iter()
        .map(|l| apply::list_tabs(&w.state, l).unwrap().window_id)
        .collect();
    assert!(ids.iter().all(|id| Uuid::parse_str(id).is_ok()), "{ids:?}");
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), labels.len(), "the persistent id differs per window");
}

// While a closed popout's OS window still exists (the fake `close` emits no `Destroyed`) its number is not reused; once the `Destroyed` number release frees it, the next new window gets it (AC20 ②). Only `release_window_number` of the cleanup-less branch is callable from outside; the cleanup branch (`drop_window_in_model`) is measured by the shell unit test.
#[test]
fn a_closed_popouts_number_waits_for_its_destroyed() {
    let w = World::new();
    let a = w.popup();
    let b = w.popup();
    let only = apply::list_tabs(&w.state, &b).unwrap().active;
    apply::close_window(&w.state, &w.subs, &w.host, &a).unwrap();
    apply::close_tab(&w.state, &w.subs, &w.ev, &w.host, &b, only).unwrap();
    assert_eq!(&*w.host.closed.lock().unwrap(), &[a.clone(), b.clone()]);

    let c = w.popup();
    assert_eq!(number_of(&w, &c), 4, "skips the numbers of the two closing windows");

    engram_dashboard_lib::layout::release_window_number(&w.state, &b);
    let d = w.popup();
    assert_eq!(number_of(&w, &d), 3);
    engram_dashboard_lib::layout::release_window_number(&w.state, &a);
    let e = w.popup();
    assert_eq!(number_of(&w, &e), 2);
}

// ── switch_tab ───────────────────────────────────────────────────────────────

#[test]
fn switch_tab_changes_active_tab() {
    let w = World::new();
    let first = w.main_active();
    let second = apply::create_tab(&w.state, &w.subs, &w.ev, MAIN_WINDOW_LABEL, None).unwrap();

    apply::switch_tab(&w.state, &w.subs, &w.ev, MAIN_WINDOW_LABEL, first).unwrap();
    assert_eq!(w.main_active(), first);
    assert_ne!(w.main_active(), second);
    assert_eq!(w.last_tabs().active, first, "the tab notification carries the active tab");
}

#[test]
fn switch_tab_foreign_view_is_err() {
    let w = World::new();
    let label = w.popup();
    let foreign = apply::list_tabs(&w.state, &label).unwrap().active;
    let before = w.main_active();

    let err = apply::switch_tab(&w.state, &w.subs, &w.ev, MAIN_WINDOW_LABEL, foreign).unwrap_err();
    assert!(err.contains("view not found"), "err={err}");
    assert_eq!(
        w.main_active(),
        before,
        "cannot switch to another window's tab — active unchanged"
    );
}

// ── switch_tab_if (drag revert — TRD S21-slot-docking §4-6) ───────────────────

#[test]
fn switch_tab_if_switches_when_active_matches() {
    let w = World::new();
    let first = w.main_active();
    let dragged = apply::create_tab(&w.state, &w.subs, &w.ev, MAIN_WINDOW_LABEL, None).unwrap();
    assert_eq!(w.main_active(), dragged, "precondition: the new tab is active");
    w.ev.take_order();
    let resyncs = w.resyncs();

    let switched =
        apply::switch_tab_if(&w.state, &w.subs, &w.ev, MAIN_WINDOW_LABEL, first, dragged).unwrap();
    assert!(switched);
    assert_eq!(w.main_active(), first);
    assert_eq!(w.last_tabs().active, first, "the tab notification carries the active tab");
    assert_eq!(w.resyncs(), resyncs + 1);
    assert_notified(&w, "switch_tab_if");
}

// Before the revert arrives, the user · LLM picked another tab — it does not overwrite that choice and touches nothing.
#[test]
fn switch_tab_if_mismatch_is_noop_without_notification() {
    let w = World::new();
    let first = w.main_active();
    let dragged = apply::create_tab(&w.state, &w.subs, &w.ev, MAIN_WINDOW_LABEL, None).unwrap();
    let chosen = apply::create_tab(&w.state, &w.subs, &w.ev, MAIN_WINDOW_LABEL, None).unwrap();
    assert_eq!(
        w.main_active(),
        chosen,
        "precondition: a tab other than the dragged one is active"
    );
    w.ev.take_order();
    let (resyncs, layouts, tabs) = (w.resyncs(), w.layout_events(), w.tab_events());
    let version = apply::list_tabs(&w.state, MAIN_WINDOW_LABEL)
        .unwrap()
        .version;

    let switched =
        apply::switch_tab_if(&w.state, &w.subs, &w.ev, MAIN_WINDOW_LABEL, first, dragged).unwrap();
    assert!(!switched);
    assert_eq!(w.main_active(), chosen, "the chosen tab stays");
    assert_eq!(
        apply::list_tabs(&w.state, MAIN_WINDOW_LABEL)
            .unwrap()
            .version,
        version
    );
    assert_eq!(
        (w.resyncs(), w.layout_events(), w.tab_events()),
        (resyncs, layouts, tabs),
        "no re-sync · no notification"
    );
    assert!(w.ev.take_order().is_empty(), "no notification either");
}

#[test]
fn switch_tab_if_errors_like_switch_tab() {
    let w = World::new();
    let label = w.popup();
    let foreign = apply::list_tabs(&w.state, &label).unwrap().active;
    let active = w.main_active();

    let err = apply::switch_tab_if(&w.state, &w.subs, &w.ev, "no-such-window", active, active)
        .unwrap_err();
    assert!(err.contains("window not found"), "err={err}");
    // A tab of another window is an error whether the condition matches or not.
    for expect in [active, foreign] {
        let err =
            apply::switch_tab_if(&w.state, &w.subs, &w.ev, MAIN_WINDOW_LABEL, foreign, expect)
                .unwrap_err();
        assert!(err.contains("view not found"), "err={err}");
    }
    assert_eq!(w.main_active(), active, "active unchanged");
}

// ── close_tab ────────────────────────────────────────────────────────────────

#[test]
fn close_tab_keeps_main_alive_with_fresh_tab() {
    let w = World::new();
    let only = w.main_active();

    apply::close_tab(&w.state, &w.subs, &w.ev, &w.host, MAIN_WINDOW_LABEL, only).unwrap();

    let tabs = apply::list_tabs(&w.state, MAIN_WINDOW_LABEL).unwrap();
    assert_eq!(tabs.tabs.len(), 1, "main has at least 1 tab (invariant 4)");
    assert_ne!(tabs.active, only, "replaced by a new empty tab");
    assert!(w.host.closed.lock().unwrap().is_empty(), "main OS window kept");
    assert_eq!(w.tab_events(), 1);
}

#[test]
fn close_tab_last_popup_tab_closes_os_window_without_notify() {
    let w = World::new();
    let label = w.popup();
    let view = apply::list_tabs(&w.state, &label).unwrap().active;
    let events_before = w.layout_events() + w.tab_events();

    apply::close_tab(&w.state, &w.subs, &w.ev, &w.host, &label, view).unwrap();

    assert_eq!(&*w.host.closed.lock().unwrap(), &[label.clone()]);
    assert!(!apply::list_windows(&w.state).unwrap().contains(&label));
    assert_eq!(
        w.layout_events() + w.tab_events(),
        events_before,
        "after the window vanishes, its tab notification is not fired"
    );
}

#[test]
fn close_tab_unknown_view_is_err() {
    let w = World::new();
    let err = apply::close_tab(
        &w.state,
        &w.subs,
        &w.ev,
        &w.host,
        MAIN_WINDOW_LABEL,
        Uuid::new_v4(),
    )
    .unwrap_err();
    assert!(err.contains("view not found"), "err={err}");
    assert!(w.host.closed.lock().unwrap().is_empty());
}

// ── close_window ─────────────────────────────────────────────────────────────

#[test]
fn close_window_drops_model_closes_os_window_and_unsubscribes() {
    let w = World::new();
    let label = w.popup();
    let view = apply::list_tabs(&w.state, &label).unwrap().active;
    let slot = w.empty_slot(view);
    let agent = Uuid::new_v4();
    apply::assign_agent(&w.state, &w.subs, &w.ev, view, slot, agent.to_string()).unwrap();

    apply::close_window(&w.state, &w.subs, &w.host, &label).unwrap();

    assert!(!apply::list_windows(&w.state).unwrap().contains(&label));
    assert_eq!(&*w.host.closed.lock().unwrap(), &[label]);
    assert_eq!(
        w.unsubscribed(),
        vec![agent],
        "an agent that remains in no window is unsubscribed"
    );
}

#[test]
fn close_window_main_is_rejected_and_os_window_untouched() {
    let w = World::new();
    let err = apply::close_window(&w.state, &w.subs, &w.host, MAIN_WINDOW_LABEL).unwrap_err();
    assert!(err.contains("the main window cannot be closed"), "err={err}");
    assert!(
        w.host.closed.lock().unwrap().is_empty(),
        "on refusal, no OS window close"
    );
    assert!(apply::list_windows(&w.state)
        .unwrap()
        .contains(&MAIN_WINDOW_LABEL.to_string()));
}

// ── 0-tab self-close (TRD S26-window-chrome §5-5 · §10-2) ──

fn window_id_of(w: &World, label: &str) -> String {
    apply::list_tabs(&w.state, label).unwrap().window_id
}

// For a popout that was removed only from the model while its OS window remained because close did nothing, close-if-empty closes it by that persistent id (the 0-tab branch of AC24).
#[test]
fn close_if_empty_closes_a_popout_that_left_the_model_with_its_os_window_open() {
    let w = World::new();
    let label = w.popup();
    let id = window_id_of(&w, &label);
    w.host.close_does_nothing.store(true, Ordering::Relaxed);
    apply::close_window(&w.state, &w.subs, &w.host, &label).unwrap();
    w.host.close_does_nothing.store(false, Ordering::Relaxed);
    assert!(w.host.is_open(&label), "the OS window remained");
    assert!(!apply::list_windows(&w.state).unwrap().contains(&label));

    assert_eq!(
        window_ctl::close_if_empty(&w.state, &w.host, &id),
        Ok(CloseOutcome::Closed)
    );
    assert_eq!(&*w.host.closed.lock().unwrap(), &[label]);
}

// A window with tabs (a window a tab came into in the meantime) is not closed, and since it picks only by persistent id, it does not touch a new window that got the same number either (AC32).
#[test]
fn close_if_empty_keeps_a_window_with_tabs_and_never_reaches_a_new_window() {
    let w = World::new();
    let kept = w.popup();
    apply::create_tab(&w.state, &w.subs, &w.ev, &kept, None).unwrap();
    assert_eq!(
        window_ctl::close_if_empty(&w.state, &w.host, &window_id_of(&w, &kept)),
        Ok(CloseOutcome::Kept)
    );

    let left = w.popup();
    let left_id = window_id_of(&w, &left);
    w.host.close_does_nothing.store(true, Ordering::Relaxed);
    apply::close_window(&w.state, &w.subs, &w.host, &left).unwrap();
    w.host.close_does_nothing.store(false, Ordering::Relaxed);
    engram_dashboard_lib::layout::release_window_number(&w.state, &left);
    let newcomer = w.popup();
    assert_eq!(
        number_of(&w, &newcomer),
        3,
        "a new window received that number again — pick by persistent id, not by number"
    );
    assert_eq!(
        window_ctl::close_if_empty(&w.state, &w.host, &window_id_of(&w, &newcomer)),
        Ok(CloseOutcome::Kept)
    );
    assert!(w.host.closed.lock().unwrap().is_empty());
    assert_eq!(
        window_ctl::close_if_empty(&w.state, &w.host, &left_id),
        Ok(CloseOutcome::Kept),
        "a window whose number was released (that OS window's `Destroyed` has passed) has nothing to close"
    );
}

// ── split_slot / close_slot ──────────────────────────────────────────────────

#[test]
fn split_slot_returns_new_slot_and_notifies() {
    let w = World::new();
    let view = w.main_active();
    let before = w.slots(view).len();
    let target = w.empty_slot(view);

    let new_id =
        apply::split_slot(&w.state, &w.subs, &w.ev, view, target, SplitDir::TopBottom).unwrap();

    let slots = w.slots(view);
    assert_eq!(slots.len(), before + 1);
    assert!(slots.contains(&new_id));
    assert_eq!(w.layout_events(), 1);
    assert_eq!(w.tab_events(), 1);
}

#[test]
fn split_slot_unknown_slot_is_err() {
    let w = World::new();
    let view = w.main_active();
    let err = apply::split_slot(
        &w.state,
        &w.subs,
        &w.ev,
        view,
        Uuid::new_v4(),
        SplitDir::TopBottom,
    )
    .unwrap_err();
    assert!(err.contains("slot not found"), "err={err}");
    assert_eq!(w.layout_events(), 0);
}

#[test]
fn close_slot_removes_slot_and_unsubscribes_vanished_agent() {
    let w = World::new();
    let view = w.main_active();
    let slot = w.empty_slot(view);
    let agent = Uuid::new_v4();
    apply::assign_agent(&w.state, &w.subs, &w.ev, view, slot, agent.to_string()).unwrap();
    assert!(w.unsubscribed().is_empty(), "no unsubscribe right after assignment");

    apply::close_slot(&w.state, &w.subs, &w.ev, view, slot).unwrap();

    assert!(!w.slots(view).contains(&slot));
    assert_eq!(w.unsubscribed(), vec![agent]);
}

#[test]
fn close_slot_unknown_view_is_err() {
    let w = World::new();
    let view = w.main_active();
    let slots_before = w.slots(view);

    let err =
        apply::close_slot(&w.state, &w.subs, &w.ev, Uuid::new_v4(), Uuid::new_v4()).unwrap_err();

    assert!(err.contains("view not found"), "err={err}");
    assert_eq!(w.slots(view), slots_before, "no partial change");
    assert_eq!(w.resyncs(), 0);
    assert_eq!(w.layout_events() + w.tab_events(), 0);
}

// ── focus_slot (ADR-0066 routing unchanged) ───────────────────────────────────

#[test]
fn focus_slot_notifies_without_routing_resync() {
    let w = World::new();
    let view = w.main_active();
    let target = w.empty_slot(view);
    apply::split_slot(&w.state, &w.subs, &w.ev, view, target, SplitDir::LeftRight).unwrap();
    assert_ne!(w.snapshot(view).focused_slot_id, Some(target));
    let (layout_before, tabs_before, resyncs_before) =
        (w.layout_events(), w.tab_events(), w.resyncs());

    apply::focus_slot(&w.state, &w.ev, view, target).unwrap();

    assert_eq!(w.snapshot(view).focused_slot_id, Some(target));
    assert_eq!(w.layout_events(), layout_before + 1);
    assert_eq!(w.tab_events(), tabs_before + 1);
    assert_eq!(
        w.resyncs(),
        resyncs_before,
        "focus does not change output routing"
    );
}

#[test]
fn focus_slot_unknown_slot_is_err() {
    let w = World::new();
    let view = w.main_active();
    let before = w.snapshot(view).focused_slot_id;
    let err = apply::focus_slot(&w.state, &w.ev, view, Uuid::new_v4()).unwrap_err();
    assert!(err.contains("slot not found"), "err={err}");
    assert_eq!(w.snapshot(view).focused_slot_id, before, "no partial change");
}

// ── rename_tab (ADR-0057 tab notification only) ───────────────────────────────

#[test]
fn rename_tab_notifies_tabs_only() {
    let w = World::new();
    let view = w.main_active();

    apply::rename_tab(&w.state, &w.ev, view, "new name".to_string()).unwrap();

    assert_eq!(w.layout_events(), 0, "the name is not in the view snapshot");
    assert_eq!(w.tab_events(), 1);
    assert_eq!(w.resyncs(), 0);
    let named = w.last_tabs();
    assert_eq!(
        named.tabs.iter().find(|t| t.id == view).map(|t| &t.name),
        Some(&"new name".to_string())
    );
}

// AC36 — the model rejects an empty name (even if the boundary does not filter it) · no notification · name unchanged.
#[test]
fn rename_tab_refuses_a_blank_name_without_notifying() {
    let w = World::new();
    let view = w.main_active();
    let before = apply::list_tabs(&w.state, MAIN_WINDOW_LABEL).unwrap();

    for blank in ["", "   ", "\t\n"] {
        assert!(apply::rename_tab(&w.state, &w.ev, view, blank.to_string()).is_err());
    }

    assert_eq!(w.tab_events(), 0);
    assert_eq!(
        apply::list_tabs(&w.state, MAIN_WINDOW_LABEL).unwrap(),
        before
    );
}

#[test]
fn rename_tab_unknown_view_is_err() {
    let w = World::new();
    let err = apply::rename_tab(&w.state, &w.ev, Uuid::new_v4(), "x".to_string()).unwrap_err();
    assert!(err.contains("view not found"), "err={err}");
    assert_eq!(w.tab_events(), 0);
}

// ── agent_names_updated (TRD S26-window-chrome §4-3 — tab notification only · only changed windows · new version) ──

#[test]
fn agent_names_updated_announces_only_the_window_whose_tab_name_changed() {
    let w = World::new();
    let view = w.main_active();
    let agent = Uuid::new_v4().to_string();
    apply::assign_agent(
        &w.state,
        &w.subs,
        &w.ev,
        view,
        w.empty_slot(view),
        agent.clone(),
    )
    .unwrap();
    let popup = w.popup();
    let (layouts, tabs, resyncs) = (w.layout_events(), w.tab_events(), w.resyncs());
    let version = apply::list_tabs(&w.state, MAIN_WINDOW_LABEL)
        .unwrap()
        .version;

    apply::agent_names_updated(&w.state, &w.ev, &[(agent.clone(), "myproj".into())]).unwrap();

    assert_eq!(w.tab_events(), tabs + 1, "only the one changed window");
    let sent = w.last_tabs();
    assert_eq!(sent.label, MAIN_WINDOW_LABEL);
    assert_eq!(sent.tabs[0].name, "myproj");
    assert!(
        sent.version > version,
        "a new version number — the web discards the same version"
    );
    assert_eq!(w.layout_events(), layouts, "the name is not in the view snapshot");
    assert_eq!(w.resyncs(), resyncs, "output routing unchanged");
    assert_eq!(
        apply::list_tabs(&w.state, &popup).unwrap().version,
        sent.version
    );
}

#[test]
fn agent_names_updated_without_a_visible_change_announces_nothing() {
    let w = World::new();
    let view = w.main_active();
    let agent = Uuid::new_v4().to_string();
    apply::assign_agent(
        &w.state,
        &w.subs,
        &w.ev,
        view,
        w.empty_slot(view),
        agent.clone(),
    )
    .unwrap();
    apply::agent_names_updated(&w.state, &w.ev, &[(agent.clone(), "myproj".into())]).unwrap();
    let tabs = w.tab_events();
    let version = apply::list_tabs(&w.state, MAIN_WINDOW_LABEL)
        .unwrap()
        .version;

    apply::agent_names_updated(
        &w.state,
        &w.ev,
        &[
            (agent, "myproj".into()),
            (Uuid::new_v4().to_string(), "not on screen".into()),
        ],
    )
    .unwrap();
    apply::agent_names_updated(&w.state, &w.ev, &[]).unwrap();

    assert_eq!(w.tab_events(), tabs);
    assert_eq!(
        apply::list_tabs(&w.state, MAIN_WINDOW_LABEL)
            .unwrap()
            .version,
        version,
        "version unchanged too"
    );
}

#[test]
fn agent_names_updated_on_a_poisoned_lock_is_err_without_notify() {
    let w = World::new();
    poison(&w.state);
    assert!(apply::agent_names_updated(&w.state, &w.ev, &[("a1".into(), "x".into())]).is_err());
    assert_eq!(w.tab_events(), 0);
}

// ── assign_agent / set_slot_content ──────────────────────────────────────────

#[test]
fn assign_agent_binds_slot_and_resyncs() {
    let w = World::new();
    let view = w.main_active();
    let slot = w.empty_slot(view);
    let agent = Uuid::new_v4();

    apply::assign_agent(&w.state, &w.subs, &w.ev, view, slot, agent.to_string()).unwrap();

    assert_eq!(
        tree::find_slot(&w.snapshot(view).layout, slot),
        Some(&SlotContent::Agent {
            agent_id: agent.to_string()
        })
    );
    assert_eq!(w.resyncs(), 1);
    assert_eq!(w.layout_events(), 1);
}

#[test]
fn assign_agent_unknown_slot_is_err() {
    let w = World::new();
    let view = w.main_active();
    let err = apply::assign_agent(
        &w.state,
        &w.subs,
        &w.ev,
        view,
        Uuid::new_v4(),
        Uuid::new_v4().to_string(),
    )
    .unwrap_err();
    assert!(err.contains("slot not found"), "err={err}");
    assert_eq!(w.resyncs(), 0);
}

#[test]
fn set_slot_content_replaces_content() {
    let w = World::new();
    let view = w.main_active();
    let slot = w.empty_slot(view);

    apply::set_slot_content(&w.state, &w.subs, &w.ev, view, slot, SlotContent::AgentList).unwrap();

    assert_eq!(
        tree::find_slot(&w.snapshot(view).layout, slot),
        Some(&SlotContent::AgentList)
    );
    assert_eq!(w.layout_events(), 1);
}

#[test]
fn set_slot_content_unknown_view_is_err() {
    let w = World::new();
    let err = apply::set_slot_content(
        &w.state,
        &w.subs,
        &w.ev,
        Uuid::new_v4(),
        Uuid::new_v4(),
        SlotContent::AgentList,
    )
    .unwrap_err();
    assert!(err.contains("view not found"), "err={err}");
    assert_eq!(w.layout_events(), 0);
}

// ── set_usage_slot (TRD S21 usage-limit-slot §1-7) ──────────────────────────

fn usage_of(w: &World, view: Uuid, slot: Uuid) -> Option<SlotContent> {
    tree::find_slot(&w.snapshot(view).layout, slot).cloned()
}

/// A slot that was not usage — the area taken out is `true`. Overwriting an agent slot releases that subscription (the resync runs inside the lock).
#[test]
fn set_usage_slot_over_other_content_defaults_missing_flags_to_true() {
    let w = World::new();
    let view = w.main_active();
    let slot = w.empty_slot(view);
    let agent = Uuid::new_v4();
    apply::assign_agent(&w.state, &w.subs, &w.ev, view, slot, agent.to_string()).unwrap();
    let before = w.resyncs();

    apply::set_usage_slot(&w.state, &w.subs, &w.ev, view, slot, None, Some(false)).unwrap();

    assert_eq!(
        usage_of(&w, view, slot),
        Some(SlotContent::Usage {
            show_claude: true,
            show_codex: false
        })
    );
    assert_eq!(w.resyncs(), before + 1);
    assert_eq!(w.unsubscribed(), vec![agent]);
    assert_eq!(w.layout_events(), 2);
}

/// If it is already usage, the area taken out is at its current value as is — toggling one area does not revert another.
#[test]
fn set_usage_slot_keeps_the_current_value_of_a_missing_flag() {
    let w = World::new();
    let view = w.main_active();
    let slot = w.empty_slot(view);
    apply::set_usage_slot(
        &w.state,
        &w.subs,
        &w.ev,
        view,
        slot,
        Some(false),
        Some(true),
    )
    .unwrap();

    apply::set_usage_slot(&w.state, &w.subs, &w.ev, view, slot, None, Some(false)).unwrap();
    assert_eq!(
        usage_of(&w, view, slot),
        Some(SlotContent::Usage {
            show_claude: false,
            show_codex: false
        })
    );

    apply::set_usage_slot(&w.state, &w.subs, &w.ev, view, slot, Some(true), None).unwrap();
    assert_eq!(
        usage_of(&w, view, slot),
        Some(SlotContent::Usage {
            show_claude: true,
            show_codex: false
        })
    );
}

#[test]
fn set_usage_slot_unknown_slot_is_err_without_resync() {
    let w = World::new();
    let view = w.main_active();
    let err = apply::set_usage_slot(
        &w.state,
        &w.subs,
        &w.ev,
        view,
        Uuid::new_v4(),
        Some(true),
        Some(true),
    )
    .unwrap_err();
    assert!(err.contains("slot not found"), "err={err}");
    assert_eq!(w.resyncs(), 0);
    assert_eq!(w.layout_events(), 0);
}

#[test]
fn set_usage_slot_unknown_view_is_err_without_notify() {
    let w = World::new();
    let err = apply::set_usage_slot(
        &w.state,
        &w.subs,
        &w.ev,
        Uuid::new_v4(),
        Uuid::new_v4(),
        None,
        Some(true),
    )
    .unwrap_err();
    assert!(err.contains("view not found"), "err={err}");
    assert_eq!(w.resyncs(), 0);
    assert_eq!(w.layout_events(), 0);
}

/// When two toggles in a popup come in a row before the broadcast — only one area is written each, so the end is the same in whichever order they arrive.
#[test]
fn set_usage_slot_single_field_writes_from_both_vendors_land_in_either_order() {
    type Write = (Option<bool>, Option<bool>);
    let codex_on: Write = (None, Some(true));
    let claude_off: Write = (Some(false), None);
    for order in [[codex_on, claude_off], [claude_off, codex_on]] {
        let w = World::new();
        let view = w.main_active();
        let slot = w.empty_slot(view);
        apply::set_usage_slot(
            &w.state,
            &w.subs,
            &w.ev,
            view,
            slot,
            Some(true),
            Some(false),
        )
        .unwrap();
        for (claude, codex) in order {
            apply::set_usage_slot(&w.state, &w.subs, &w.ev, view, slot, claude, codex).unwrap();
        }
        assert_eq!(
            usage_of(&w, view, slot),
            Some(SlotContent::Usage {
                show_claude: false,
                show_codex: true
            }),
            "order={order:?}"
        );
        assert_eq!(w.layout_events(), 3);
    }
}

// ── move_slot_to_window ──────────────────────────────────────────────────────

impl World {
    fn view_with_filled_slot(&self) -> (Uuid, Uuid, SlotContent) {
        let view = self.main_active();
        let slot = self.empty_slot(view);
        let content = SlotContent::Agent {
            agent_id: Uuid::new_v4().to_string(),
        };
        apply::set_slot_content(
            &self.state,
            &self.subs,
            &self.ev,
            view,
            slot,
            content.clone(),
        )
        .unwrap();
        (view, slot, content)
    }

    // The only gauge that measures rollback: `views` also counts owner-less temporary Views, whereas on the failure path no window entry exists, so counting windows stays green even with rollback deleted.
    fn view_count(&self) -> usize {
        self.state.0.lock().unwrap().views.len()
    }
}

#[test]
fn move_slot_to_window_detaches_content_into_a_new_window() {
    let w = World::new();
    let (view, slot, content) = w.view_with_filled_slot();

    let moved = apply::move_slot_to_window(
        &w.state, &w.subs, &w.ev, &w.host, &w.labels, view, slot, None,
    )
    .unwrap();

    assert_eq!(&*w.host.opened.lock().unwrap(), &[moved.window.clone()]);
    assert_eq!(
        &*w.host.placed.lock().unwrap(),
        &[moved.window.clone()],
        "a new window records its first place after entering the model"
    );
    assert!(!w.slots(view).contains(&slot), "the original slot is closed (MOVE)");
    let landed = apply::list_tabs(&w.state, &moved.window).unwrap();
    assert_eq!(landed.active, moved.tab, "the new tab is that window's active tab");
    let landed_slot = w.slots(moved.tab)[0];
    assert_eq!(
        tree::find_slot(&w.snapshot(moved.tab).layout, landed_slot),
        Some(&content),
        "the content is carried to the new tab as is"
    );
}

/// If the notification does not go out the screen draws the old layout: this command changes two windows (source lost content, target gained a tab), so two tab bar notifications — missing the target's leaves a new window with an empty tab bar.
#[test]
fn move_slot_to_window_notifies_both_windows() {
    let w = World::new();
    let (view, slot, _) = w.view_with_filled_slot();
    let layout_before = w.layout_events();
    let tabs_before = w.tab_events();

    let moved = apply::move_slot_to_window(
        &w.state, &w.subs, &w.ev, &w.host, &w.labels, view, slot, None,
    )
    .unwrap();

    assert_eq!(
        w.layout_events() - layout_before,
        1,
        "one new layout of the original view"
    );
    let tab_payloads = w.ev.tabs.lock().unwrap();
    let sent: Vec<&str> = tab_payloads[tabs_before..]
        .iter()
        .map(|t| t.label.as_str())
        .collect();
    assert_eq!(
        sent,
        vec![MAIN_WINDOW_LABEL, moved.window.as_str()],
        "the tab bars update in the order original window → target window"
    );
}

// Moving to an existing window answers that window's persistent id · number (TRD S26-window-chrome §3-2).
#[test]
fn move_slot_to_window_into_an_existing_window_answers_that_windows_id_and_number() {
    let w = World::new();
    let popout = w.popup();
    let (view, slot, _) = w.view_with_filled_slot();
    let moved = apply::move_slot_to_window(
        &w.state,
        &w.subs,
        &w.ev,
        &w.host,
        &w.labels,
        view,
        slot,
        Some(popout.clone()),
    )
    .unwrap();
    let tabs = apply::list_tabs(&w.state, &popout).unwrap();
    assert_eq!(
        (
            moved.window.as_str(),
            moved.window_id.as_str(),
            moved.number
        ),
        (popout.as_str(), tabs.window_id.as_str(), 2)
    );

    let (view, slot) = {
        let back = moved.tab;
        (back, w.slots(back)[0])
    };
    let home = apply::move_slot_to_window(
        &w.state,
        &w.subs,
        &w.ev,
        &w.host,
        &w.labels,
        view,
        slot,
        Some(MAIN_WINDOW_LABEL.to_string()),
    )
    .unwrap();
    assert_eq!(
        (home.window.as_str(), home.window_id.as_str(), home.number),
        (MAIN_WINDOW_LABEL, "main", 1)
    );
}

// TRD S21-slot-docking §8-1 ⑬ — fresh-build order: stage A (temporary View) → `open` → stage C → notification → `record_placement` (the first placement record comes after the notification).
#[test]
fn move_slot_to_window_into_a_new_window_records_the_placement_after_the_notifications() {
    let w = World::new();
    let (view, slot, _) = w.view_with_filled_slot();
    let views_before = w.view_count();
    *w.host.during_open.lock().unwrap() = Some(Box::new(move |state: &LayoutState| {
        assert_eq!(
            state.0.lock().unwrap().views.len(),
            views_before + 1,
            "the phase A temporary View stands before opening"
        );
    }));
    w.journal.take();

    let moved = apply::move_slot_to_window(
        &w.state, &w.subs, &w.ev, &w.host, &w.labels, view, slot, None,
    )
    .unwrap();

    let label = moved.window;
    assert_eq!(
        w.journal.take(),
        [
            format!("open:{label}"),
            "layout".to_string(),
            format!("tabs:{MAIN_WINDOW_LABEL}"),
            format!("tabs:{label}"),
            format!("place:{label}"),
        ]
    );
}

// ── move_slot_to_window: prewarmed window (TRD S21-slot-docking §8-1 ⑬ · ADR-0299) ──

#[test]
fn move_slot_to_window_opens_the_prewarmed_window_and_commits_after_the_notifications() {
    let w = World::new();
    let (view, slot, content) = w.view_with_filled_slot();
    let spare = w.prewarm();
    let views_before = w.view_count();
    *w.host.during_open.lock().unwrap() = Some(Box::new(move |state: &LayoutState| {
        assert_eq!(
            state.0.lock().unwrap().views.len(),
            views_before + 1,
            "the phase A temporary View stands before opening"
        );
    }));
    w.journal.take();

    let moved = apply::move_slot_to_window(
        &w.state, &w.subs, &w.ev, &w.host, &w.labels, view, slot, None,
    )
    .unwrap();

    assert_eq!(moved.window, spare, "opens with the handed-over window's label");
    assert_eq!(
        w.journal.take(),
        [
            format!("claim:{spare}"),
            format!("open:{spare}"),
            "layout".to_string(),
            format!("tabs:{MAIN_WINDOW_LABEL}"),
            format!("tabs:{spare}"),
            format!("committed:{spare}"),
            format!("place:{spare}"),
        ]
    );
    let tab = &apply::list_tabs(&w.state, &spare).unwrap().tabs[0];
    assert_eq!(
        (tab.name.as_str(), tab.named),
        ("Agent", false),
        "the new window's tab is auto-named — the name of the moved area (an agent not in the ledger)"
    );
    let moved_layout = w.snapshot(moved.tab).layout;
    assert_eq!(
        tree::find_slot(&moved_layout, tree::first_slot_id(&moved_layout)),
        Some(&content)
    );
}

// If stage A rejects (a nonexistent slot), there is no opened window — only the handed-over window is taken back.
#[test]
fn move_slot_to_window_refused_before_the_open_gives_the_prewarmed_window_back() {
    let w = World::new();
    let view = w.main_active();
    let spare = w.prewarm();
    w.journal.take();

    let err = apply::move_slot_to_window(
        &w.state,
        &w.subs,
        &w.ev,
        &w.host,
        &w.labels,
        view,
        Uuid::new_v4(),
        None,
    )
    .unwrap_err();

    assert!(err.contains("slot not found"), "err={err}");
    assert_eq!(
        w.journal.take(),
        [format!("claim:{spare}"), format!("release:{spare}")]
    );
    assert!(w.host.opened.lock().unwrap().is_empty());
}

// If the handed-over window's `open` is `Err`, the window port has withdrawn that window — only the temporary View is reverted, and there is no commit · take-back.
#[test]
fn move_slot_to_window_whose_prewarmed_window_fails_to_open_rolls_back_without_a_release() {
    let w = World::failing_host();
    let (view, slot, content) = w.view_with_filled_slot();
    let spare = w.prewarm();
    let views_before = w.view_count();
    w.journal.take();

    assert!(apply::move_slot_to_window(
        &w.state, &w.subs, &w.ev, &w.host, &w.labels, view, slot, None,
    )
    .is_err());

    assert_eq!(
        w.journal.take(),
        [format!("claim:{spare}"), format!("open:{spare}")]
    );
    assert_eq!(w.view_count(), views_before, "the temporary View was reaped");
    assert_eq!(
        tree::find_slot(&w.snapshot(view).layout, slot),
        Some(&content)
    );
}

// If stage C rejects (a window of the same label stood in the model while opening), the handed-over window is not closed but taken back.
#[test]
fn move_slot_to_window_refused_at_the_attach_gives_the_prewarmed_window_back() {
    let w = World::new();
    let (view, slot, content) = w.view_with_filled_slot();
    let spare = w.prewarm();
    let taken = spare.clone();
    *w.host.during_open.lock().unwrap() = Some(Box::new(move |state: &LayoutState| {
        state.0.lock().unwrap().create_window(&taken).unwrap();
    }));
    w.journal.take();

    let err = apply::move_slot_to_window(
        &w.state, &w.subs, &w.ev, &w.host, &w.labels, view, slot, None,
    )
    .unwrap_err();

    assert!(err.contains("tab insertion failed"), "err={err}");
    assert_eq!(
        w.journal.take(),
        [
            format!("claim:{spare}"),
            format!("open:{spare}"),
            format!("release:{spare}"),
        ]
    );
    assert_eq!(
        tree::find_slot(&w.snapshot(view).layout, slot),
        Some(&content)
    );
}

// Even if the window drops out of the model while notifying (first tab notification) before the commit, the commit passes the claim ticket on — closing it is the window port's job (TRD §3-9 · I27 · I31).
#[test]
fn move_slot_to_window_commits_the_claim_even_if_the_window_left_the_model_before_it() {
    let w = World::new();
    let (view, slot, _) = w.view_with_filled_slot();
    let spare = w.prewarm();
    let gone = spare.clone();
    *w.ev.on_tabs.lock().unwrap() = Some(Box::new(move |state: &LayoutState| {
        state.0.lock().unwrap().close_window(&gone).unwrap();
    }));
    w.host.vanished.lock().unwrap().push(spare.clone());
    w.journal.take();

    apply::move_slot_to_window(
        &w.state, &w.subs, &w.ev, &w.host, &w.labels, view, slot, None,
    )
    .unwrap();

    let journal = w.journal.take();
    assert_eq!(
        journal[journal.len() - 2..],
        [format!("committed:{spare}"), format!("place:{spare}")]
    );
    assert!(!w.window_exists(&spare));
    assert!(
        w.host.closed.lock().unwrap().is_empty(),
        "the apply service does not close"
    );
}

// A poisoned lock at stage C leaves no opened window: a new one is closed, a handed-over one taken back (the temporary View cannot be withdrawn — as in `rollback_detached`).
#[test]
fn move_slot_to_window_on_a_poisoned_lock_withdraws_the_window_it_opened() {
    for prewarmed in [false, true] {
        let w = World::new();
        let (view, slot, _) = w.view_with_filled_slot();
        let spare = prewarmed.then(|| w.prewarm());
        *w.host.during_open.lock().unwrap() = Some(Box::new(poison));
        w.journal.take();

        assert!(apply::move_slot_to_window(
            &w.state, &w.subs, &w.ev, &w.host, &w.labels, view, slot, None,
        )
        .is_err());

        let expected = match spare {
            Some(label) => vec![
                format!("claim:{label}"),
                format!("open:{label}"),
                format!("release:{label}"),
            ],
            None => vec![
                format!("open:{}", popup_label(1)),
                format!("close:{}", popup_label(1)),
            ],
        };
        assert_eq!(w.journal.take(), expected, "prewarmed={prewarmed}");
        assert!(w.host.placed.lock().unwrap().is_empty());
    }
}

/// The subscription is not cut while switching windows: the agent leaves the source slot and appears in the target tab at the same moment; a resync computed before attach or run after close would catch 1→0 and kill a live subscription.
#[test]
fn move_slot_to_window_keeps_the_agent_subscribed_across_the_move() {
    let w = World::new();
    let (view, slot, _) = w.view_with_filled_slot();

    apply::move_slot_to_window(
        &w.state, &w.subs, &w.ev, &w.host, &w.labels, view, slot, None,
    )
    .unwrap();

    assert!(
        w.unsubscribed().is_empty(),
        "still visible in the window it moved to, so there is nothing to unsubscribe: {:?}",
        w.unsubscribed()
    );
}

#[test]
fn move_slot_to_window_into_an_existing_window_adds_a_tab() {
    let w = World::new();
    let target = w.popup();
    let tabs_before = apply::list_tabs(&w.state, &target).unwrap().tabs.len();
    let placed_before = w.host.placed.lock().unwrap().len();
    let (view, slot, _) = w.view_with_filled_slot();
    let spare = w.prewarm();
    let asked = w.claims_asked();

    let moved = apply::move_slot_to_window(
        &w.state,
        &w.subs,
        &w.ev,
        &w.host,
        &w.labels,
        view,
        slot,
        Some(target.clone()),
    )
    .unwrap();

    assert_eq!(moved.window, target, "does not open a new window");
    assert_eq!(
        w.host.placed.lock().unwrap().len(),
        placed_before,
        "the place of an existing window is recorded by the event"
    );
    let tabs = apply::list_tabs(&w.state, &target).unwrap();
    assert_eq!(tabs.tabs.len(), tabs_before + 1);
    assert_eq!(tabs.active, moved.tab);
    w.assert_spare_not_asked(&spare, asked);
}

// ADR-0228
#[test]
fn move_slot_to_window_moves_an_empty_slot_and_promotes_its_sibling() {
    let w = World::new();
    let view = w.main_active();
    let slot = w.empty_slot(view);
    let sibling =
        apply::split_slot(&w.state, &w.subs, &w.ev, view, slot, SplitDir::LeftRight).expect("split");

    let moved = apply::move_slot_to_window(
        &w.state, &w.subs, &w.ev, &w.host, &w.labels, view, slot, None,
    )
    .unwrap();

    assert_eq!(&*w.host.opened.lock().unwrap(), &[moved.window.clone()]);
    assert_eq!(
        w.slots(view),
        vec![sibling],
        "the original slot is closed and the sibling inherits the place"
    );
    let landed = apply::list_tabs(&w.state, &moved.window).unwrap();
    assert_eq!(landed.active, moved.tab);
    let landed_slots = w.slots(moved.tab);
    assert_eq!(landed_slots.len(), 1);
    assert_eq!(
        tree::find_slot(&w.snapshot(moved.tab).layout, landed_slots[0]),
        Some(&SlotContent::Empty),
        "the new tab is a single empty area"
    );
    assert!(w.unsubscribed().is_empty(), "{:?}", w.unsubscribed());
}

#[test]
fn move_slot_to_window_of_a_lone_empty_slot_leaves_a_fresh_empty_slot() {
    let w = World::new();
    let view = w.main_active();
    let slot = w.empty_slot(view);

    apply::move_slot_to_window(
        &w.state, &w.subs, &w.ev, &w.host, &w.labels, view, slot, None,
    )
    .unwrap();

    let left = w.slots(view);
    assert_eq!(left.len(), 1);
    assert_ne!(left[0], slot, "it is a newly standing area, not the moved one");
    assert_eq!(
        tree::find_slot(&w.snapshot(view).layout, left[0]),
        Some(&SlotContent::Empty)
    );
}

// The rejection names the real cause — calling a nonexistent area/tab an "empty slot" makes the caller fix the wrong thing.
#[test]
fn move_slot_to_window_refuses_a_missing_slot_or_view_with_the_real_cause() {
    let w = World::new();
    let view = w.main_active();
    let ghost = Uuid::new_v4();
    let views_before = w.view_count();

    let err = apply::move_slot_to_window(
        &w.state, &w.subs, &w.ev, &w.host, &w.labels, view, ghost, None,
    )
    .unwrap_err();
    assert!(err.contains("slot not found"), "err={err}");
    assert!(!err.contains("empty slot"), "err={err}");

    let err = apply::move_slot_to_window(
        &w.state, &w.subs, &w.ev, &w.host, &w.labels, ghost, ghost, None,
    )
    .unwrap_err();
    assert!(err.contains("view not found"), "err={err}");

    assert!(w.host.opened.lock().unwrap().is_empty(), "does not open a window either");
    assert_eq!(w.view_count(), views_before, "leaves no temporary View");
}

/// If the empty area being moved is filled during the window build the source is not closed (even with an `Empty` comparison base, "close only if as moved" holds) — else the content filled meanwhile stays on no screen.
#[test]
fn move_slot_to_window_keeps_an_empty_source_that_was_filled_during_the_window_build() {
    let w = World::new();
    let view = w.main_active();
    let slot = w.empty_slot(view);
    *w.host.during_open.lock().unwrap() = Some(Box::new(move |state: &LayoutState| {
        state
            .0
            .lock()
            .unwrap()
            .set_slot_content(view, slot, SlotContent::AgentList)
            .unwrap();
    }));

    let moved = apply::move_slot_to_window(
        &w.state, &w.subs, &w.ev, &w.host, &w.labels, view, slot, None,
    )
    .unwrap();

    assert_eq!(
        tree::find_slot(&w.snapshot(view).layout, slot),
        Some(&SlotContent::AgentList),
        "the content filled in the meantime is not deleted"
    );
    let landed_slot = w.slots(moved.tab)[0];
    assert_eq!(
        tree::find_slot(&w.snapshot(moved.tab).layout, landed_slot),
        Some(&SlotContent::Empty),
        "the target tab stays as moved (an empty area)"
    );
}

/// Measures that the temporary View is actually reclaimed: counting windows stays green with the whole rollback deleted while owner-less Views pile up in `views` with every retry (the ④ endpoint of the `apply.rs` owner-less tmp_view invariant).
#[test]
fn move_slot_to_window_rolls_back_when_the_window_cannot_be_built() {
    let w = World::failing_host();
    let (view, slot, content) = w.view_with_filled_slot();
    let views_before = w.view_count();
    let resyncs_before = w.resyncs();

    let err = apply::move_slot_to_window(
        &w.state, &w.subs, &w.ev, &w.host, &w.labels, view, slot, None,
    )
    .unwrap_err();

    assert!(err.contains("window creation failed"), "err={err}");
    assert!(
        w.host.placed.lock().unwrap().is_empty(),
        "a window that failed to open is not recorded"
    );
    assert_eq!(
        w.view_count(),
        views_before,
        "the temporary View was not reclaimed — a ghost View would pile up for every failed pop-out"
    );
    assert_eq!(
        w.resyncs() - resyncs_before,
        2,
        "phase A re-sync + rollback re-sync = 2 (1 if the rollback is missing)"
    );
    assert_eq!(
        tree::find_slot(&w.snapshot(view).layout, slot),
        Some(&content),
        "on build failure the source slot stays — the user does not lose content"
    );
}

/// Sets up the state only `is_open` can filter: the target window is alive in the model but its OS window vanished (`Destroyed` cleanup not yet run). The model re-validation of `insert_tab_into` lets it through, so without `is_open` the content becomes a tab of a screenless window and the source slot closes onto no screen.
#[test]
fn move_slot_to_window_rejects_a_target_whose_os_window_is_gone() {
    let w = World::new();
    let target = w.popup();
    w.host.closed.lock().unwrap().push(target.clone());
    let (view, slot, content) = w.view_with_filled_slot();
    let views_before = w.view_count();
    let tabs_before = apply::list_tabs(&w.state, &target).unwrap().tabs.len();

    let err = apply::move_slot_to_window(
        &w.state,
        &w.subs,
        &w.ev,
        &w.host,
        &w.labels,
        view,
        slot,
        Some(target.clone()),
    )
    .unwrap_err();

    assert!(err.contains("target window not found"), "err={err}");
    assert_eq!(
        tree::find_slot(&w.snapshot(view).layout, slot),
        Some(&content),
        "a refusal must be a no-op — closing the source slot would leave the content on no screen"
    );
    assert_eq!(
        apply::list_tabs(&w.state, &target).unwrap().tabs.len(),
        tabs_before,
        "no tab is pushed into a window with no screen"
    );
    assert_eq!(w.view_count(), views_before, "the temporary View is reclaimed too");
}

#[test]
fn move_slot_to_window_rejects_an_unknown_target_label() {
    let w = World::new();
    let (view, slot, content) = w.view_with_filled_slot();
    let views_before = w.view_count();

    let err = apply::move_slot_to_window(
        &w.state,
        &w.subs,
        &w.ev,
        &w.host,
        &w.labels,
        view,
        slot,
        Some("no-such".to_string()),
    )
    .unwrap_err();

    assert!(err.contains("target window not found"), "err={err}");
    assert_eq!(
        tree::find_slot(&w.snapshot(view).layout, slot),
        Some(&content),
        "a refusal must be a no-op"
    );
    assert_eq!(w.view_count(), views_before, "temporary View reclaimed");
}

// ── spawn_into ───────────────────────────────────────────────────────────────

#[tokio::test]
async fn spawn_into_creates_tab_and_places_agent() {
    let w = World::new();
    let agent = Uuid::new_v4();
    let spawner = Spawner::ok(&w.state, agent);

    let id = apply::spawn_into(
        &w.state,
        &w.subs,
        &w.ev,
        &spawner,
        MAIN_WINDOW_LABEL,
        None,
        None,
        None,
        "C:/tmp".to_string(),
    )
    .await
    .unwrap();

    assert_eq!(id, agent.to_string());
    assert_eq!(spawner.calls(), 1);
    assert_eq!(&*spawner.cwds.lock().unwrap(), &["C:/tmp".to_string()]);
    let placed = w.main_active();
    let slots = w.slots(placed);
    assert_eq!(slots.len(), 1, "new tab = one empty root");
    assert_eq!(
        tree::find_slot(&w.snapshot(placed).layout, slots[0]),
        Some(&SlotContent::Agent {
            agent_id: agent.to_string()
        })
    );
    assert_eq!(w.layout_events(), 1);
}

/// Blocks only unknown words — a typo passing would die in the daemon (new) or silently start another backend (old daemon unaware of the field). The rejection message takes its vocabulary from the wire enum.
#[tokio::test]
async fn spawn_into_rejects_an_unknown_backend_before_spawning() {
    let w = World::new();
    let spawner = Spawner::ok(&w.state, Uuid::new_v4());

    let err = apply::spawn_into(
        &w.state,
        &w.subs,
        &w.ev,
        &spawner,
        MAIN_WINDOW_LABEL,
        None,
        None,
        Some("codx".to_string()),
        "C:/tmp".to_string(),
    )
    .await
    .unwrap_err();

    assert!(err.contains("Not spawned"), "err={err}");
    assert!(
        err.contains("claude") && err.contains("codex"),
        "must say what works: {err}"
    );
    assert_eq!(spawner.calls(), 0, "★rejected before the spawn★");
}

/// A known word goes to the port as is — green means the blanket rejection of ADR-0058 is really lifted. The shell does not look at policy (the daemon's `SpawnByCwd` handling does, ADR-0279), so every word the wire knows stands here; a word removed from the list is no longer measured anywhere.
#[tokio::test]
async fn spawn_into_forwards_a_known_backend() {
    for (word, expected) in [
        ("claude", AgentBackendKind::Claude),
        (" claude ", AgentBackendKind::Claude),
        ("codex", AgentBackendKind::Codex),
        (" codex ", AgentBackendKind::Codex),
    ] {
        let w = World::new();
        let agent = Uuid::new_v4();
        let spawner = Spawner::ok(&w.state, agent);

        let id = apply::spawn_into(
            &w.state,
            &w.subs,
            &w.ev,
            &spawner,
            MAIN_WINDOW_LABEL,
            None,
            None,
            Some(word.to_string()),
            "C:/tmp".to_string(),
        )
        .await
        .unwrap();

        assert_eq!(id, agent.to_string());
        assert_eq!(
            &*spawner.backends.lock().unwrap(),
            &[Some(expected)],
            "{word}"
        );
    }
}

#[tokio::test]
async fn spawn_into_accepts_blank_backend_as_unspecified() {
    let w = World::new();
    let agent = Uuid::new_v4();
    let spawner = Spawner::ok(&w.state, agent);
    let tabs_before = apply::list_tabs(&w.state, MAIN_WINDOW_LABEL)
        .unwrap()
        .tabs
        .len();

    let id = apply::spawn_into(
        &w.state,
        &w.subs,
        &w.ev,
        &spawner,
        MAIN_WINDOW_LABEL,
        None,
        None,
        Some("   ".to_string()),
        "C:/tmp".to_string(),
    )
    .await
    .unwrap();

    // A blank backend passed as "unspecified" is told from the rejection path only after the layout step too (rejection: zero spawns, zero layouts).
    assert_eq!(spawner.calls(), 1);
    // The shell invents no default — unspecified goes to the port as unspecified (rejecting the absence is the daemon's job, one source for the "fill which field" message).
    assert_eq!(&*spawner.backends.lock().unwrap(), &[None]);
    assert_eq!(id, agent.to_string());
    let placed = w.main_active();
    assert_eq!(
        apply::list_tabs(&w.state, MAIN_WINDOW_LABEL)
            .unwrap()
            .tabs
            .len(),
        tabs_before + 1,
        "tab=None → a new tab"
    );
    let slots = w.slots(placed);
    assert_eq!(
        tree::find_slot(&w.snapshot(placed).layout, slots[0]),
        Some(&SlotContent::Agent {
            agent_id: agent.to_string()
        })
    );
}

#[tokio::test]
async fn spawn_into_rejects_slot_without_tab_before_spawning() {
    let w = World::new();
    let spawner = Spawner::ok(&w.state, Uuid::new_v4());

    let err = apply::spawn_into(
        &w.state,
        &w.subs,
        &w.ev,
        &spawner,
        MAIN_WINDOW_LABEL,
        None,
        Some(Uuid::new_v4()),
        None,
        "C:/tmp".to_string(),
    )
    .await
    .unwrap_err();

    assert!(err.contains("Not spawned"), "err={err}");
    assert_eq!(spawner.calls(), 0, "orphan-tab guard = pre-spawn");
}

#[tokio::test]
async fn spawn_into_reports_live_agent_when_placement_fails() {
    let w = World::new();
    let main = w.main_active();
    let taken = w.empty_slot(main);
    w.put_agent(main, taken);
    let agent = Uuid::new_v4();
    let spawner = Spawner::ok(&w.state, agent);

    let err = apply::spawn_into(
        &w.state,
        &w.subs,
        &w.ev,
        &spawner,
        MAIN_WINDOW_LABEL,
        Some(main),
        Some(taken),
        None,
        "C:/tmp".to_string(),
    )
    .await
    .unwrap_err();

    assert_eq!(spawner.calls(), 1, "the spawn already happened (no hard rollback)");
    assert!(
        err.contains(&agent.to_string()),
        "the living agent id is embedded in the error to prevent an invisible agent: err={err}"
    );
}

/// A given tab places the agent in that tab's window; for another window or a nonexistent tab nothing launches, so an agent with no place does not survive (TRD S28 §7).
#[tokio::test]
async fn spawn_into_takes_the_window_from_the_tab_and_refuses_a_foreign_one_before_spawning() {
    let w = World::new();
    let label = w.popup();
    let foreign_tab = apply::list_tabs(&w.state, &label).unwrap().active;

    for (window, tab) in [
        (MAIN_WINDOW_LABEL, foreign_tab),
        (MAIN_WINDOW_LABEL, Uuid::new_v4()),
    ] {
        let never = Spawner::ok(&w.state, Uuid::new_v4());
        let version = w.version();
        let err = apply::spawn_into(
            &w.state,
            &w.subs,
            &w.ev,
            &never,
            window,
            Some(tab),
            None,
            None,
            "C:/tmp".to_string(),
        )
        .await
        .unwrap_err();
        assert_eq!(never.calls(), 0, "{tab}: does not spawn");
        assert!(err.contains("Not spawned"), "err={err}");
        assert_eq!(w.version(), version);
    }

    let agent = Uuid::new_v4();
    let spawner = Spawner::ok(&w.state, agent);
    let placed = apply::spawn_into_placed(
        &w.state,
        &w.subs,
        &w.ev,
        &spawner,
        None,
        Some(foreign_tab),
        None,
        None,
        "C:/tmp".to_string(),
    )
    .await
    .unwrap();
    assert_eq!(placed.out, agent.to_string());
    assert_eq!(
        (placed.at.window.as_str(), placed.at.view),
        (label.as_str(), foreign_tab)
    );
    assert_eq!(placed.rows.len(), 1, "the placed tab's row");
    assert_invariants(&w);
}

#[tokio::test]
async fn spawn_into_propagates_spawn_failure_untouched() {
    let w = World::new();
    let spawner = Spawner::failing(&w.state);

    let err = apply::spawn_into(
        &w.state,
        &w.subs,
        &w.ev,
        &spawner,
        MAIN_WINDOW_LABEL,
        None,
        None,
        None,
        "C:/tmp".to_string(),
    )
    .await
    .unwrap_err();

    assert_eq!(err, "spawn failed: daemon refused");
    assert_eq!(
        apply::list_tabs(&w.state, MAIN_WINDOW_LABEL)
            .unwrap()
            .tabs
            .len(),
        1,
        "if the spawn fails, no tab is made either"
    );
    assert_eq!(w.resyncs(), 0);
}

/// A known word's policy rejection makes one trip to the daemon and returns as is with no layout (ADR-0279): the shell passes the word through only the port net, returns the port's `Err` message untouched and leaves the layout. `spawn_into_propagates_spawn_failure_untouched` has no word, so it cannot measure that. The message imitates a daemon policy refusal; the shell does not read it.
#[tokio::test]
async fn spawn_into_passes_a_daemon_refusal_through_without_placing() {
    let w = World::new();
    let refusal = "backend 'codex' is a known word, but this surface does not create it now — test reason. Not spawned.";
    let spawner = Spawner::new(&w.state, Err(refusal.to_string()));

    let err = apply::spawn_into(
        &w.state,
        &w.subs,
        &w.ev,
        &spawner,
        MAIN_WINDOW_LABEL,
        None,
        None,
        Some("codex".to_string()),
        "C:/tmp".to_string(),
    )
    .await
    .unwrap_err();

    assert_eq!(spawner.calls(), 1, "the shell does not look at the policy and asks the daemon");
    assert_eq!(
        &*spawner.backends.lock().unwrap(),
        &[Some(AgentBackendKind::Codex)],
        "the chosen word reached the port"
    );
    assert_eq!(err, refusal, "the daemon text comes back as is");
    assert_eq!(
        apply::list_tabs(&w.state, MAIN_WINDOW_LABEL)
            .unwrap()
            .tabs
            .len(),
        1,
        "on refusal no tab is made either"
    );
    assert_eq!(w.resyncs(), 0);
}

// ── set_split_ratio (ADR-0227 — the layout notification only on Applied) ──────

impl World {
    fn split_view(&self) -> (Uuid, Uuid) {
        let view = self.main_active();
        let slot = self.empty_slot(view);
        apply::split_slot(
            &self.state,
            &self.subs,
            &self.ev,
            view,
            slot,
            SplitDir::LeftRight,
        )
        .unwrap();
        (view, self.snapshot(view).split_rects[0].split_id)
    }

    fn last_layout(&self) -> ViewSnapshot {
        self.ev
            .layout
            .lock()
            .unwrap()
            .last()
            .cloned()
            .expect("layout notification")
    }
}

#[test]
fn set_split_ratio_applied_notifies_the_layout_once_with_the_new_ratio_and_rects() {
    let w = World::new();
    let (view, split) = w.split_view();
    let (layout_before, tabs_before, resyncs_before) =
        (w.layout_events(), w.tab_events(), w.resyncs());

    let got = apply::set_split_ratio(&w.state, &w.ev, view, split, 0.3).unwrap();

    assert_eq!(got.outcome, SplitRatioOutcome::Applied);
    assert_eq!(got.ratio, 0.3);
    assert_eq!(
        w.layout_events(),
        layout_before + 1,
        "exactly 1 layout notification"
    );
    assert_eq!(w.tab_events(), tabs_before, "0 tab notifications");
    assert_eq!(w.resyncs(), resyncs_before, "routing unchanged — no re-sync");

    let snap = w.last_layout();
    assert_eq!(snap.view_id, view);
    assert_eq!(
        got.version, snap.version,
        "returned version == notified snapshot version"
    );
    let LayoutNode::Split { ratio, .. } = &snap.layout else {
        panic!("should be a split tree: {:?}", snap.layout)
    };
    assert_eq!(*ratio, 0.3);
    let rect = snap
        .split_rects
        .iter()
        .find(|r| r.split_id == split)
        .expect("split rectangle");
    assert_eq!(rect.at, 0.3, "the rectangle is computed with the new ratio too");
    let left = snap
        .slot_rects
        .iter()
        .find(|r| r.x0 == 0.0)
        .expect("left area");
    assert_eq!(left.x1, 0.3, "a = the left area is 30%");
}

#[test]
fn set_split_ratio_unchanged_or_too_small_notifies_nothing() {
    let w = World::new();
    let (view, split) = w.split_view();
    let events_before = (w.layout_events(), w.tab_events());
    let version = w.snapshot(view).version;

    let same = apply::set_split_ratio(&w.state, &w.ev, view, split, 0.5).unwrap();
    assert_eq!(same.outcome, SplitRatioOutcome::Unchanged);
    assert_eq!(same.ratio, 0.5);

    apply::report_window_canvas(&w.state, MAIN_WINDOW_LABEL, 300, 300).unwrap();
    let mut big_min = one_px_border();
    big_min.min_pane_px = 200;
    apply::report_ui_metrics(&w.state, MAIN_WINDOW_LABEL, big_min).unwrap();
    let small = apply::set_split_ratio(&w.state, &w.ev, view, split, 0.3).unwrap();
    assert_eq!(small.outcome, SplitRatioOutcome::TooSmall);
    assert_eq!(small.ratio, 0.5, "the current value as is");

    assert_eq!(
        (w.layout_events(), w.tab_events()),
        events_before,
        "an unchanged outcome announces nothing"
    );
    assert_eq!(w.snapshot(view).version, version, "version unchanged");
}

#[test]
fn set_split_ratio_unknown_split_is_err_without_notify() {
    let w = World::new();
    let (view, split) = w.split_view();
    let events_before = (w.layout_events(), w.tab_events());

    let err = apply::set_split_ratio(&w.state, &w.ev, view, Uuid::new_v4(), 0.3).unwrap_err();
    assert!(err.contains("split not found"), "err={err}");
    let err = apply::set_split_ratio(&w.state, &w.ev, view, split, f64::NAN).unwrap_err();
    assert!(err.contains("ratio refused"), "err={err}");

    assert_eq!((w.layout_events(), w.tab_events()), events_before);
}

/// A port that interjects another write right where it receives the notification — a lock released and the global counter re-read would make the returned version off by this write.
struct Meddler {
    inner: Recorder,
    state: LayoutState,
}

impl LayoutEvents for Meddler {
    fn layout_updated(&self, snapshot: &ViewSnapshot) {
        self.inner.layout_updated(snapshot);
        let mut mgr = self.state.0.lock().unwrap();
        mgr.rename_tab(snapshot.view_id, "interleaved write".to_string())
            .unwrap();
    }

    fn window_tabs_updated(&self, tabs: &WindowTabsPayload) {
        self.inner.window_tabs_updated(tabs);
    }
}

#[test]
fn set_split_ratio_returns_the_notified_snapshots_version_not_a_later_one() {
    let w = World::new();
    let (view, split) = w.split_view();
    let meddler = Meddler {
        inner: Recorder::new(&w.state, Journal::default()),
        state: w.state.clone(),
    };

    let got = apply::set_split_ratio(&w.state, &meddler, view, split, 0.3).unwrap();

    let notified = meddler
        .inner
        .layout
        .lock()
        .unwrap()
        .last()
        .cloned()
        .expect("notification 1");
    assert_eq!(got.version, notified.version);
    assert!(
        w.snapshot(view).version > got.version,
        "precondition: the interleaved write raised the global counter"
    );
}

fn assert_notified(w: &World, write: &str) {
    assert!(!w.ev.take_order().is_empty(), "{write}: there must be a notification");
}

#[tokio::test]
async fn every_notifying_write_notifies() {
    let w = World::new();
    let main = w.main_active();
    let slot = w.empty_slot(main);

    let tab = apply::create_tab(&w.state, &w.subs, &w.ev, MAIN_WINDOW_LABEL, None).unwrap();
    assert_notified(&w, "create_tab");
    apply::switch_tab(&w.state, &w.subs, &w.ev, MAIN_WINDOW_LABEL, main).unwrap();
    assert_notified(&w, "switch_tab");
    let added =
        apply::split_slot(&w.state, &w.subs, &w.ev, main, slot, SplitDir::LeftRight).unwrap();
    assert_notified(&w, "split_slot");
    apply::focus_slot(&w.state, &w.ev, main, added).unwrap();
    assert_notified(&w, "focus_slot");
    let split = w.snapshot(main).split_rects[0].split_id;
    let ratio = apply::set_split_ratio(&w.state, &w.ev, main, split, 0.3).unwrap();
    assert_eq!(ratio.outcome, SplitRatioOutcome::Applied);
    assert_notified(&w, "set_split_ratio");
    apply::rename_tab(&w.state, &w.ev, main, "name".to_string()).unwrap();
    assert_notified(&w, "rename_tab");
    apply::assign_agent(
        &w.state,
        &w.subs,
        &w.ev,
        main,
        slot,
        Uuid::new_v4().to_string(),
    )
    .unwrap();
    assert_notified(&w, "assign_agent");
    apply::set_slot_content(
        &w.state,
        &w.subs,
        &w.ev,
        main,
        added,
        SlotContent::AgentList,
    )
    .unwrap();
    assert_notified(&w, "set_slot_content");
    apply::set_usage_slot(&w.state, &w.subs, &w.ev, main, added, Some(false), None).unwrap();
    assert_notified(&w, "set_usage_slot");
    apply::close_slot(&w.state, &w.subs, &w.ev, main, added).unwrap();
    assert_notified(&w, "close_slot");
    apply::close_tab(&w.state, &w.subs, &w.ev, &w.host, MAIN_WINDOW_LABEL, tab).unwrap();
    assert_notified(&w, "close_tab");
    apply::move_slot_to_window(
        &w.state, &w.subs, &w.ev, &w.host, &w.labels, main, slot, None,
    )
    .unwrap();
    assert_notified(&w, "move_slot_to_window");
    let spawner = Spawner::ok(&w.state, Uuid::new_v4());
    apply::spawn_into(
        &w.state,
        &w.subs,
        &w.ev,
        &spawner,
        MAIN_WINDOW_LABEL,
        None,
        None,
        None,
        "C:/tmp".to_string(),
    )
    .await
    .unwrap();
    assert_notified(&w, "spawn_into");
}

#[test]
fn writes_that_change_nothing_notify_nothing() {
    let w = World::new();
    let (view, split) = w.split_view();
    let label = w.popup();
    let popup_tab = apply::list_tabs(&w.state, &label).unwrap().active;
    w.ev.take_order();

    apply::create_tab(&w.state, &w.subs, &w.ev, "no-such", None).unwrap_err();
    apply::rename_tab(&w.state, &w.ev, Uuid::new_v4(), "x".to_string()).unwrap_err();
    apply::focus_slot(&w.state, &w.ev, view, Uuid::new_v4()).unwrap_err();
    let same = apply::set_split_ratio(&w.state, &w.ev, view, split, 0.5).unwrap();
    assert_eq!(same.outcome, SplitRatioOutcome::Unchanged);
    apply::move_slot_to_window(
        &w.state,
        &w.subs,
        &w.ev,
        &w.host,
        &w.labels,
        view,
        Uuid::new_v4(),
        None,
    )
    .unwrap_err();
    apply::close_tab(&w.state, &w.subs, &w.ev, &w.host, &label, popup_tab).unwrap();

    assert_eq!(w.ev.take_order(), Vec::<&str>::new());
}

// ── OS window title (TRD S26-window-chrome §2-4 ② · §4-4 · §10-2) ──

// Current title per window — picked up by lookup.
fn titles(w: &World) -> BTreeMap<String, String> {
    apply::list_windows(&w.state)
        .unwrap()
        .into_iter()
        .map(|label| {
            let title = apply::list_tabs(&w.state, &label).unwrap().title;
            (label, title)
        })
        .collect()
}

// The picture before one write — [`TitleCheck::after`] measures invariant 2: every window whose title changed (including a new one) was announced by that write with a larger tab-list version, and its last notification carries the current title.
struct TitleCheck {
    titles: BTreeMap<String, String>,
    version: u64,
    seen: usize,
}

impl TitleCheck {
    fn before(w: &World) -> Self {
        TitleCheck {
            titles: titles(w),
            version: w.version(),
            seen: w.ev.tabs.lock().unwrap().len(),
        }
    }

    // `changes` = which window's title this write changes — a pair so that running only non-title writes does not pass vacuously.
    fn after(self, w: &World, write: &str, changes: bool) {
        let after = titles(w);
        let sent = w.ev.tabs.lock().unwrap()[self.seen..].to_vec();
        let changed: Vec<&String> = after
            .iter()
            .filter(|(label, title)| self.titles.get(*label) != Some(*title))
            .map(|(label, _)| label)
            .collect();
        assert_eq!(
            !changed.is_empty(),
            changes,
            "{write}: windows whose title changed {changed:?}"
        );
        for label in changed {
            let last = sent
                .iter()
                .rev()
                .find(|t| &t.label == label)
                .unwrap_or_else(|| panic!("{write}: did not announce {label}, whose title changed"));
            assert_eq!(last.title, after[label], "{write}: {label}");
            assert!(
                last.version > self.version,
                "{write}: announced {label} with an old version"
            );
        }
    }
}

fn title_write<T>(w: &World, write: &str, changes: bool, run: impl FnOnce() -> T) -> T {
    let check = TitleCheck::before(w);
    let out = run();
    check.after(w, write, changes);
    out
}

// Every write that changes a title (window · tab · slot · name ledger · move · docking · spawn) announces that window with a new version.
#[tokio::test]
async fn every_write_that_changes_a_window_title_announces_that_window_at_a_new_version() {
    let w = World::new();
    let first = w.main_active();
    let slot = w.empty_slot(first);
    let agent = Uuid::new_v4().to_string();
    assert_eq!(titles(&w)[MAIN_WINDOW_LABEL], "Empty slot - Engram:1");

    let popup = title_write(&w, "create_window", true, || {
        apply::create_window(&w.state, &w.subs, &w.ev, &w.host, &w.labels)
            .unwrap()
            .window
    });
    let work = title_write(&w, "create_tab", true, || {
        apply::create_tab(
            &w.state,
            &w.subs,
            &w.ev,
            MAIN_WINDOW_LABEL,
            Some("work".to_string()),
        )
        .unwrap()
    });
    title_write(&w, "switch_tab", true, || {
        apply::switch_tab(&w.state, &w.subs, &w.ev, MAIN_WINDOW_LABEL, first).unwrap()
    });
    title_write(&w, "switch_tab_if", true, || {
        apply::switch_tab_if(&w.state, &w.subs, &w.ev, MAIN_WINDOW_LABEL, work, first).unwrap()
    });
    title_write(&w, "switch_tab", true, || {
        apply::switch_tab(&w.state, &w.subs, &w.ev, MAIN_WINDOW_LABEL, first).unwrap()
    });
    title_write(&w, "assign_agent", true, || {
        apply::assign_agent(&w.state, &w.subs, &w.ev, first, slot, agent.clone()).unwrap()
    });
    title_write(&w, "agent_names_updated", true, || {
        apply::agent_names_updated(&w.state, &w.ev, &[(agent.clone(), "myproj".to_string())])
            .unwrap()
    });
    assert_eq!(titles(&w)[MAIN_WINDOW_LABEL], "myproj - Engram:1");
    let added = title_write(&w, "split_slot", true, || {
        apply::split_slot(&w.state, &w.subs, &w.ev, first, slot, SplitDir::LeftRight).unwrap()
    });
    title_write(&w, "focus_slot", true, || {
        apply::focus_slot(&w.state, &w.ev, first, slot).unwrap()
    });
    title_write(&w, "set_slot_content", true, || {
        apply::set_slot_content(
            &w.state,
            &w.subs,
            &w.ev,
            first,
            slot,
            SlotContent::AgentList,
        )
        .unwrap()
    });
    title_write(&w, "set_usage_slot", true, || {
        apply::set_usage_slot(&w.state, &w.subs, &w.ev, first, slot, Some(false), None).unwrap()
    });
    title_write(&w, "close_slot", true, || {
        apply::close_slot(&w.state, &w.subs, &w.ev, first, slot).unwrap()
    });
    assert_eq!(w.snapshot(first).focused_slot_id, Some(added));
    title_write(&w, "rename_tab", true, || {
        apply::rename_tab(&w.state, &w.ev, first, "name".to_string()).unwrap()
    });
    title_write(&w, "close_tab", true, || {
        apply::close_tab(&w.state, &w.subs, &w.ev, &w.host, MAIN_WINDOW_LABEL, first).unwrap()
    });
    assert_eq!(titles(&w)[MAIN_WINDOW_LABEL], "work - Engram:1");
    let work_slot = w.empty_slot(work);
    title_write(&w, "move_slot_to_window(new window)", true, || {
        apply::move_slot_to_window(
            &w.state, &w.subs, &w.ev, &w.host, &w.labels, work, work_slot, None,
        )
        .unwrap()
    });
    let (view, filled, _) = w.view_with_filled_slot();
    title_write(&w, "move_slot_to_window(existing window)", true, || {
        apply::move_slot_to_window(
            &w.state,
            &w.subs,
            &w.ev,
            &w.host,
            &w.labels,
            view,
            filled,
            Some(popup.clone()),
        )
        .unwrap()
    });
    title_write(&w, "dock_tab(tab bar)", true, || {
        w.dock(
            work,
            DockDest::TabBar {
                window: popup.clone(),
                index: None,
                after: None,
            },
            FocusPolicy::Reveal,
        )
        .unwrap()
    });
    assert_eq!(titles(&w)[popup.as_str()], "work - Engram:2");
    title_write(&w, "dock_tab(new window)", true, || {
        w.dock(work, DockDest::NewWindow { at: AT }, FocusPolicy::Reveal)
            .unwrap()
    });
    let spawner = Spawner::ok(&w.state, Uuid::new_v4());
    let check = TitleCheck::before(&w);
    apply::spawn_into(
        &w.state,
        &w.subs,
        &w.ev,
        &spawner,
        MAIN_WINDOW_LABEL,
        None,
        None,
        None,
        "C:/tmp".to_string(),
    )
    .await
    .unwrap();
    check.after(&w, "spawn_into", true);
    assert_eq!(titles(&w)[MAIN_WINDOW_LABEL], "Agent - Engram:1");
    title_write(&w, "language_changed", true, || {
        apply::language_changed(&w.state, &w.ev, Lang::Ko).unwrap()
    });
    assert_eq!(titles(&w)[MAIN_WINDOW_LABEL], "에이전트 - Engram:1"); // hangul-ok: the Korean UI table
}

// ADR-0373 — a language change renames automatic tabs, so it is a title write: a new version, every affected window announced.
#[test]
fn a_language_change_with_auto_named_tabs_bumps_and_announces_every_affected_window() {
    let w = World::new();
    let popup = w.popup();
    w.forget();
    let version = w.version();

    apply::language_changed(&w.state, &w.ev, Lang::Ko).unwrap();

    let mut sent: Vec<(String, String, u64)> =
        w.ev.tabs
            .lock()
            .unwrap()
            .iter()
            .map(|t| (t.label.clone(), t.title.clone(), t.version))
            .collect();
    sent.sort();
    assert_eq!(
        sent,
        [
            (
                MAIN_WINDOW_LABEL.to_string(),
                "빈 칸 - Engram:1".to_string(), // hangul-ok: the Korean UI table
                version + 1
            ),
            (popup, "빈 칸 - Engram:2".to_string(), version + 1), // hangul-ok: the Korean UI table
        ]
    );
    assert_eq!(w.version(), version + 1, "one bump for the whole change");
    assert_eq!(w.layout_events(), 0, "tab notifications only");

    w.forget();
    apply::language_changed(&w.state, &w.ev, Lang::Ko).unwrap();
    assert_eq!(w.tab_events(), 0, "the same language changes nothing");
    assert_eq!(w.version(), version + 1);
}

#[test]
fn a_language_change_with_only_named_tabs_does_not_bump() {
    let w = World::new();
    let main = w.main_active();
    apply::rename_tab(&w.state, &w.ev, main, "work".to_string()).unwrap();
    w.forget();
    let version = w.version();

    apply::language_changed(&w.state, &w.ev, Lang::Ko).unwrap();

    assert_eq!(w.tab_events(), 0);
    assert_eq!(w.version(), version);
    assert_eq!(titles(&w)[MAIN_WINDOW_LABEL], "work - Engram:1");
}

// One time at boot — announces all windows at the current version as it is and changes neither the version nor the model (TRD S26-window-chrome §4-4 ③).
#[test]
fn sync_titles_announces_every_window_once_at_the_current_version() {
    let w = World::new();
    let popup = w.popup();
    w.forget();
    let version = w.version();

    apply::sync_titles(&w.state, &w.ev).unwrap();

    let mut sent: Vec<(String, String, u64)> =
        w.ev.tabs
            .lock()
            .unwrap()
            .iter()
            .map(|t| (t.label.clone(), t.title.clone(), t.version))
            .collect();
    sent.sort();
    assert_eq!(
        sent,
        [
            (
                MAIN_WINDOW_LABEL.to_string(),
                "Empty slot - Engram:1".to_string(),
                version
            ),
            (popup, "Empty slot - Engram:2".to_string(), version),
        ]
    );
    assert_eq!(w.layout_events(), 0);
    assert_eq!(w.version(), version, "does not raise the version");
    assert_eq!(w.resyncs(), 1, "no re-sync — the earlier 1 belongs to window creation");
}

#[test]
fn sync_titles_on_a_poisoned_lock_is_err_without_notify() {
    let w = World::new();
    poison(&w.state);

    assert!(apply::sync_titles(&w.state, &w.ev).is_err());
    assert_eq!(w.tab_events(), 0);
}

// ── two measurement reports (ADR-0227) ────────────────────────────────────────
//
// These take no notification · resync ports, so "zero notifications" is guarded by the signature; measured here: the version is unchanged and no sibling command's notification goes out with success or failure.

fn one_px_border() -> UiMetrics {
    UiMetrics {
        frame_insets: Insets {
            t: 1.0,
            r: 1.0,
            b: 1.0,
            l: 1.0,
        },
        min_pane_px: 30,
    }
}

#[test]
fn canvas_and_metrics_reports_notify_nothing_and_keep_version() {
    let w = World::new();
    let before = apply::list_tabs(&w.state, MAIN_WINDOW_LABEL)
        .unwrap()
        .version;

    apply::report_window_canvas(&w.state, MAIN_WINDOW_LABEL, 1200, 800).unwrap();
    apply::report_ui_metrics(&w.state, MAIN_WINDOW_LABEL, one_px_border()).unwrap();

    assert_eq!(w.layout_events(), 0, "0 layout notifications");
    assert_eq!(w.tab_events(), 0, "0 tab notifications");
    assert_eq!(w.resyncs(), 0, "routing unchanged — no re-sync");
    assert_eq!(
        apply::list_tabs(&w.state, MAIN_WINDOW_LABEL)
            .unwrap()
            .version,
        before,
        "a measurement report does not raise version"
    );

    let view = w.main_active();
    let slot = w.slots(view)[0];
    let px = w
        .state
        .0
        .lock()
        .unwrap()
        .slot_px(view, slot)
        .unwrap()
        .expect("both canvas and metrics reported");
    assert_eq!(
        px.frame,
        PxRect {
            x0: 0,
            y0: 0,
            x1: 1200,
            y1: 800
        }
    );
}

#[test]
fn rejected_reports_notify_nothing_and_keep_version() {
    let w = World::new();
    let before = apply::list_tabs(&w.state, MAIN_WINDOW_LABEL)
        .unwrap()
        .version;

    let err = apply::report_window_canvas(&w.state, "no-such", 1200, 800).unwrap_err();
    assert!(err.contains("window not found"), "err={err}");
    let err = apply::report_ui_metrics(&w.state, "no-such", one_px_border()).unwrap_err();
    assert!(err.contains("window not found"), "err={err}");
    let mut bad = one_px_border();
    bad.frame_insets.l = f64::NAN;
    let err = apply::report_ui_metrics(&w.state, MAIN_WINDOW_LABEL, bad).unwrap_err();
    assert!(err.contains("ui metrics refused"), "err={err}");
    apply::report_window_canvas(&w.state, MAIN_WINDOW_LABEL, 0, 800).unwrap();

    assert_eq!(w.layout_events() + w.tab_events(), 0);
    assert_eq!(w.resyncs(), 0);
    assert_eq!(
        apply::list_tabs(&w.state, MAIN_WINDOW_LABEL)
            .unwrap()
            .version,
        before
    );
}

// ── four read-only ───────────────────────────────────────────────────────────

#[test]
fn get_view_unknown_is_err() {
    let w = World::new();
    let err = apply::get_view(&w.state, Uuid::new_v4()).unwrap_err();
    assert!(err.contains("view not found"), "err={err}");
}

#[test]
fn list_tabs_returns_tabs_active_and_version() {
    let w = World::new();
    let before = apply::list_tabs(&w.state, MAIN_WINDOW_LABEL).unwrap();
    apply::create_tab(&w.state, &w.subs, &w.ev, MAIN_WINDOW_LABEL, None).unwrap();
    let after = apply::list_tabs(&w.state, MAIN_WINDOW_LABEL).unwrap();

    assert_eq!(after.label, MAIN_WINDOW_LABEL);
    assert_eq!(after.tabs.len(), before.tabs.len() + 1);
    assert!(after.version > before.version, "version increases with every change");
}

#[test]
fn list_tabs_unknown_window_is_err() {
    let w = World::new();
    let err = apply::list_tabs(&w.state, "no-such").unwrap_err();
    assert!(err.contains("window not found"), "err={err}");
}

#[test]
fn list_windows_lists_main_and_runtime_windows() {
    let w = World::new();
    assert_eq!(
        apply::list_windows(&w.state).unwrap(),
        vec![MAIN_WINDOW_LABEL.to_string()]
    );
    let label = w.popup();
    let mut got = apply::list_windows(&w.state).unwrap();
    got.sort();
    assert_eq!(got, vec![MAIN_WINDOW_LABEL.to_string(), label]);
}

#[test]
fn resolve_spatial_finds_neighbor_of_focused_slot() {
    let w = World::new();
    let view = w.main_active();
    let left = w.empty_slot(view);
    let right =
        apply::split_slot(&w.state, &w.subs, &w.ev, view, left, SplitDir::LeftRight).unwrap();
    assert_eq!(w.snapshot(view).focused_slot_id, Some(right));

    let got = apply::resolve_spatial(&w.state, "left", None, Some(view)).unwrap();
    assert_eq!(got, Some(left));
}

#[test]
fn resolve_spatial_without_view_uses_window_active_tab() {
    let w = World::new();
    let view = w.main_active();
    let by_view = apply::resolve_spatial(&w.state, "top-left", None, Some(view)).unwrap();

    // window unspecified → main's active tab.
    let by_default = apply::resolve_spatial(&w.state, "top-left", None, None).unwrap();
    let by_label =
        apply::resolve_spatial(&w.state, "top-left", Some(MAIN_WINDOW_LABEL), None).unwrap();

    assert_eq!(by_default, by_view);
    assert_eq!(by_label, by_view);
    assert!(by_view.is_some());
}

#[test]
fn resolve_spatial_unknown_token_is_err() {
    let w = World::new();
    let err = apply::resolve_spatial(&w.state, "sideways", None, None).unwrap_err();
    assert!(err.contains("unknown spatial token"), "err={err}");
}

#[test]
fn resolve_spatial_unknown_view_is_err() {
    let w = World::new();
    let err = apply::resolve_spatial(&w.state, "left", None, Some(Uuid::new_v4())).unwrap_err();
    assert!(err.contains("view not found"), "err={err}");
}

#[test]
fn resolve_spatial_unknown_window_is_err() {
    let w = World::new();
    let err = apply::resolve_spatial(&w.state, "left", Some("no-such"), None).unwrap_err();
    assert!(err.contains("window not found"), "err={err}");
}

// ── a lookup sees the value just written (read-your-writes — TRD S20 §7) ──────

#[test]
fn reads_observe_writes_immediately() {
    let w = World::new();
    let tab = apply::create_tab(
        &w.state,
        &w.subs,
        &w.ev,
        MAIN_WINDOW_LABEL,
        Some("T".into()),
    )
    .unwrap();
    let label = w.popup();
    let slot = w.empty_slot(tab);
    let agent = Uuid::new_v4();
    apply::assign_agent(&w.state, &w.subs, &w.ev, tab, slot, agent.to_string()).unwrap();
    apply::rename_tab(&w.state, &w.ev, tab, "R".to_string()).unwrap();
    apply::focus_slot(&w.state, &w.ev, tab, slot).unwrap();

    let snap = apply::get_view(&w.state, tab).unwrap();
    assert_eq!(snap.focused_slot_id, Some(slot));
    assert_eq!(
        tree::find_slot(&snap.layout, slot),
        Some(&SlotContent::Agent {
            agent_id: agent.to_string()
        })
    );
    let tabs = apply::list_tabs(&w.state, MAIN_WINDOW_LABEL).unwrap();
    assert_eq!(tabs.active, tab);
    assert_eq!(
        tabs.tabs.iter().find(|t| t.id == tab).map(|t| &t.name),
        Some(&"R".to_string())
    );
    assert!(apply::list_windows(&w.state).unwrap().contains(&label));
    assert_eq!(
        apply::resolve_spatial(&w.state, "right", None, Some(tab)).unwrap(),
        None,
        "a new tab is a single slot — no right neighbour"
    );
}

// ── dock_tab (TRD S21-slot-docking §3-3) ──────────────────────
//
// Not measured here: a new window's place straddling no monitor going to the default place (Tauri window port → `lib_unit` test of `resolve_place` in `state::placement`); the new window closed before attaching with a tombstone, and a window closing during the reuse move (model part of the close request → `commands::popout` tests in `lib_unit`).

// ADR-0057 ①–④ — the ownership pair matches on both sides · one owner window per View · the active one is a tab of that window · main tabs ≥ 1.
fn assert_invariants(w: &World) {
    let mgr = w.state.0.lock().unwrap();
    let mut tabs = 0;
    for (label, wt) in &mgr.windows {
        assert!(
            wt.tabs.contains(&wt.active),
            "③ the active tab of {label} is not that window's tab"
        );
        for view in &wt.tabs {
            assert_eq!(mgr.view_owner.get(view), Some(label), "① tab {view}");
            assert!(mgr.views.contains_key(view), "① the View of tab {view} is missing");
        }
        tabs += wt.tabs.len();
    }
    assert_eq!(mgr.views.len(), tabs, "② a View with no window, or a tab of two windows");
    assert_eq!(mgr.view_owner.len(), tabs, "① leftover ownership pairs");
    assert!(
        mgr.windows
            .get(MAIN_WINDOW_LABEL)
            .is_some_and(|w| !w.tabs.is_empty()),
        "④ main tabs >= 1"
    );
}

const AT: WindowPlace = WindowPlace {
    pos: Some((300, 200)),
    size: None,
};

fn bounds(x: f64, y: f64, w: f64, h: f64) -> Option<WindowBounds> {
    WindowBounds::new(x, y, w, h)
}

// At every notification, whether the window is still not closed — measures that closing an empty source popout comes after the notification bundle.
struct ClosesAfter<'a> {
    inner: &'a Recorder,
    host: &'a Host,
}

impl ClosesAfter<'_> {
    fn not_closed_yet(&self) {
        assert!(
            self.host.closed.lock().unwrap().is_empty(),
            "an emptied source pop-out is closed after all notifications are out"
        );
    }
}

impl LayoutEvents for ClosesAfter<'_> {
    fn layout_updated(&self, snapshot: &ViewSnapshot) {
        self.not_closed_yet();
        self.inner.layout_updated(snapshot);
    }

    fn window_tabs_updated(&self, tabs: &WindowTabsPayload) {
        self.not_closed_yet();
        self.inner.window_tabs_updated(tabs);
    }
}

impl World {
    fn version(&self) -> u64 {
        self.state.0.lock().unwrap().version
    }

    fn layout_of(&self, view: Uuid) -> LayoutNode {
        self.snapshot(view).layout
    }

    fn owner(&self, view: Uuid) -> Option<String> {
        self.state.0.lock().unwrap().owner_of(view).cloned()
    }

    fn tabs_of(&self, label: &str) -> Vec<Uuid> {
        apply::list_tabs(&self.state, label)
            .map(|t| t.tabs.iter().map(|m| m.id).collect())
            .unwrap_or_default()
    }

    fn window_exists(&self, label: &str) -> bool {
        self.state.0.lock().unwrap().windows.contains_key(label)
    }

    fn popup_with(&self, n: usize) -> (String, Vec<Uuid>) {
        let label = self.popup();
        let mut tabs = vec![apply::list_tabs(&self.state, &label).unwrap().active];
        for _ in 1..n {
            tabs.push(apply::create_tab(&self.state, &self.subs, &self.ev, &label, None).unwrap());
        }
        (label, tabs)
    }

    fn observe(&self, label: &str, maximized: bool, bounds: Option<WindowBounds>) {
        self.state
            .0
            .lock()
            .unwrap()
            .observe_window_placement(
                label,
                WindowPlacement {
                    minimized: false,
                    maximized,
                    bounds,
                },
            )
            .unwrap();
    }

    // Empties the record the preparation writes left so later assertions see one docking. The opened-window list (`opened`) is kept — this fake's `is_open` · revert look at it.
    fn forget(&self) {
        self.ev.take_order();
        self.ev.layout.lock().unwrap().clear();
        self.ev.tabs.lock().unwrap().clear();
        let h = &self.host;
        h.closed.lock().unwrap().clear();
        h.placed.lock().unwrap().clear();
        h.opened_at.lock().unwrap().clear();
        h.moves.lock().unwrap().clear();
        h.steps.lock().unwrap().clear();
        self.journal.take();
    }

    fn dock(&self, view: Uuid, to: DockDest, focus: FocusPolicy) -> Result<DockOutcome, DockError> {
        self.dock_via(&self.ev, view, to, focus)
    }

    fn dock_via(
        &self,
        events: &dyn LayoutEvents,
        view: Uuid,
        to: DockDest,
        focus: FocusPolicy,
    ) -> Result<DockOutcome, DockError> {
        apply::dock_tab(
            &self.state,
            &self.subs,
            events,
            &self.host,
            &self.labels,
            DockRequest {
                view,
                to,
                focus,
                mv: DockMove::default(),
            },
        )
    }

    fn notified_views(&self) -> Vec<Uuid> {
        self.ev
            .layout
            .lock()
            .unwrap()
            .iter()
            .map(|s| s.view_id)
            .collect()
    }

    fn notified_windows(&self) -> Vec<String> {
        self.ev
            .tabs
            .lock()
            .unwrap()
            .iter()
            .map(|t| t.label.clone())
            .collect()
    }

    fn notified_tabs_of(&self, label: &str) -> WindowTabsPayload {
        self.ev
            .tabs
            .lock()
            .unwrap()
            .iter()
            .find(|t| t.label == label)
            .cloned()
            .unwrap_or_else(|| panic!("{label} tab notification"))
    }

    fn assert_notified_at_current_version(&self) {
        let version = self.version();
        for s in self.ev.layout.lock().unwrap().iter() {
            assert_eq!(s.version, version, "snapshot {}", s.view_id);
        }
        for t in self.ev.tabs.lock().unwrap().iter() {
            assert_eq!(t.version, version, "tab list {}", t.label);
        }
    }
}

// ── dock_tab: seam · tab bar ──

#[test]
fn dock_tab_seam_moves_the_tab_under_one_lock_and_notifies_after_it() {
    let w = World::new();
    let target = w.main_active();
    let (label, tabs) = w.popup_with(2);
    let source = tabs[1];
    let slot = w.empty_slot(source);
    let agent = Uuid::new_v4();
    apply::assign_agent(&w.state, &w.subs, &w.ev, source, slot, agent.to_string()).unwrap();
    let resyncs = w.resyncs();
    w.forget();

    let out = w
        .dock(
            source,
            DockDest::Seam {
                target,
                anchor: DockAnchor::Outer,
                side: DockSide::Left,
                share: DOCK_NEW_SHARE,
            },
            FocusPolicy::Reveal,
        )
        .unwrap();

    assert_eq!(
        out,
        DockOutcome {
            changed: true,
            tab: target,
            views: vec![target, tabs[0]],
            window: MAIN_WINDOW_LABEL.to_string(),
            slot_ids: vec![slot],
            emptied: None,
            main_refilled: None,
            reused: false,
            prewarmed: false,
            identity: Some(NewWindow {
                window: MAIN_WINDOW_LABEL.to_string(),
                window_id: MAIN_WINDOW_LABEL.to_string(),
                number: 1,
            }),
            rows: Vec::new(),
        }
    );
    assert_eq!(w.resyncs(), resyncs + 1, "one re-sync — inside the lock (probe)");
    assert_eq!(
        w.subs.router.targets(agent).to_vec(),
        [MAIN_WINDOW_LABEL.to_string()],
        "the moved agent's output goes to the target window"
    );
    assert!(w.unsubscribed().is_empty(), "the subscription is not cut");
    assert!(w.slots(target).contains(&slot), "inserted into the target tab");
    assert_eq!(w.owner(source), None, "the source View is erased");
    assert_eq!(
        w.notified_views(),
        [target, tabs[0]],
        "target layout + the new active layout of the source window"
    );
    assert_eq!(w.notified_windows(), [MAIN_WINDOW_LABEL, label.as_str()]);
    assert_eq!(w.notified_tabs_of(&label).active, tabs[0]);
    w.assert_notified_at_current_version();
    assert_notified(&w, "dock_tab(seam)");
    assert!(w.host.closed.lock().unwrap().is_empty());
    assert_invariants(&w);
}

#[test]
fn dock_tab_tab_bar_moves_the_tab_and_notifies_both_windows() {
    let w = World::new();
    let main_tab = w.main_active();
    let (label, tabs) = w.popup_with(2);
    let source = tabs[1];
    let resyncs = w.resyncs();
    let spare = w.prewarm();
    let asked = w.claims_asked();
    w.forget();

    let out = w
        .dock(
            source,
            DockDest::TabBar {
                window: MAIN_WINDOW_LABEL.to_string(),
                index: Some(0),
                after: None,
            },
            FocusPolicy::Reveal,
        )
        .unwrap();

    w.assert_spare_not_asked(&spare, asked);
    assert!(!out.prewarmed);
    assert_eq!(out.window, MAIN_WINDOW_LABEL);
    assert_eq!(out.emptied, None);
    assert_eq!(w.resyncs(), resyncs + 1);
    assert_eq!(w.tabs_of(MAIN_WINDOW_LABEL), [source, main_tab]);
    assert_eq!(w.owner(source).as_deref(), Some(MAIN_WINDOW_LABEL));
    let main = w.notified_tabs_of(MAIN_WINDOW_LABEL);
    assert_eq!(main.active, source, "Reveal — the moved tab is shown");
    assert_eq!(
        main.tabs.iter().map(|m| m.id).collect::<Vec<_>>(),
        [source, main_tab]
    );
    assert_eq!(w.notified_tabs_of(&label).active, tabs[0]);
    assert_eq!(w.notified_views(), [source, tabs[0]]);
    w.assert_notified_at_current_version();
    assert_notified(&w, "dock_tab(tab_bar)");
    assert_invariants(&w);
}

#[test]
fn dock_tab_closes_an_emptied_source_popout_once_after_its_notifications() {
    let w = World::new();
    let target = w.main_active();
    let (label, tabs) = w.popup_with(1);
    let source = tabs[0];
    w.forget();
    let events = ClosesAfter {
        inner: &w.ev,
        host: &w.host,
    };

    let out = w
        .dock_via(
            &events,
            source,
            DockDest::Seam {
                target,
                anchor: DockAnchor::Outer,
                side: DockSide::Right,
                share: DOCK_NEW_SHARE,
            },
            FocusPolicy::Reveal,
        )
        .unwrap();

    assert_eq!(
        out.emptied,
        Some(label.clone()),
        "even when closing, the erased entry is returned"
    );
    assert_eq!(&*w.host.closed.lock().unwrap(), &[label.clone()], "once");
    assert!(!w.window_exists(&label), "erased from the model first");
    assert_eq!(
        w.notified_windows(),
        [MAIN_WINDOW_LABEL],
        "there is no tab list for the erased window"
    );
    assert_notified(&w, "dock_tab(empty source)");
    assert_invariants(&w);
}

#[test]
fn dock_tab_refusals_change_nothing_and_notify_nothing() {
    let w = World::new();
    let (_, tabs) = w.popup_with(2);
    let source = tabs[1];
    let bogus = Uuid::new_v4();
    let cases = [
        (
            source,
            DockDest::Seam {
                target: source,
                anchor: DockAnchor::Outer,
                side: DockSide::Top,
                share: DOCK_NEW_SHARE,
            },
            DockError::SelfTarget,
        ),
        (
            source,
            DockDest::TabBar {
                window: "no-such".into(),
                index: None,
                after: None,
            },
            DockError::WindowNotFound("no-such".into()),
        ),
        (
            bogus,
            DockDest::NewWindow { at: AT },
            DockError::TabNotFound(bogus),
        ),
    ];
    let version = w.version();
    let resyncs = w.resyncs();
    w.forget();

    for (view, to, want) in cases {
        assert_eq!(w.dock(view, to, FocusPolicy::Reveal), Err(want));
    }

    assert_eq!(w.version(), version);
    assert_eq!(w.resyncs(), resyncs, "failure = no re-sync");
    assert_eq!(w.ev.take_order(), Vec::<&str>::new(), "0 notifications");
    assert!(
        w.host.opened_at.lock().unwrap().is_empty(),
        "does not open a new window either"
    );
    assert!(w.host.closed.lock().unwrap().is_empty());
    assert_invariants(&w);
}

#[test]
fn dock_tab_onto_the_same_place_is_ok_and_silent() {
    let w = World::new();
    let (label, tabs) = w.popup_with(2);
    let version = w.version();
    let resyncs = w.resyncs();
    w.forget();

    let out = w
        .dock(
            tabs[1],
            DockDest::TabBar {
                window: label.clone(),
                index: Some(1),
                after: None,
            },
            FocusPolicy::Keep,
        )
        .unwrap();

    assert_eq!(out.window, label);
    assert!(!out.changed, "the outcome says unchanged");
    assert_eq!(out.views, [tabs[1]]);
    assert_eq!(w.version(), version);
    assert_eq!(w.resyncs(), resyncs);
    assert_eq!(
        w.ev.take_order(),
        Vec::<&str>::new(),
        "an unchanged outcome is not announced"
    );
}

#[test]
fn dock_tab_of_mains_last_tab_announces_the_fresh_empty_tab() {
    for mode in ["tab_bar", "new_window"] {
        let w = World::new();
        let only = w.main_active();
        let (label, _) = w.popup_with(1);
        w.forget();
        let to = match mode {
            "tab_bar" => DockDest::TabBar {
                window: label.clone(),
                index: None,
                after: None,
            },
            _ => DockDest::NewWindow { at: AT },
        };

        let out = w.dock(only, to, FocusPolicy::Keep).unwrap();

        let main = w.tabs_of(MAIN_WINDOW_LABEL);
        assert_eq!(main.len(), 1, "{mode}: one empty tab in main");
        let fresh = main[0];
        assert_ne!(fresh, only);
        assert_eq!(out.main_refilled, Some(fresh), "{mode}");
        let payload = w.notified_tabs_of(MAIN_WINDOW_LABEL);
        assert_eq!(payload.active, fresh, "the empty tab is main's active tab");
        assert_eq!(
            payload.tabs.iter().map(|m| m.id).collect::<Vec<_>>(),
            [fresh]
        );
        assert!(
            w.notified_views().contains(&fresh),
            "the empty tab's layout is carried too — with only the tab id there is no layout to draw"
        );
        assert_eq!(
            out.views,
            [only, fresh],
            "{mode}: the outcome's changed tabs = the moved tab + the empty tab newly set up in main"
        );
        w.assert_notified_at_current_version();
        assert_notified(&w, "dock_tab(main's last tab)");
        assert_invariants(&w);
    }
}

// ── dock_tab: next active · attach share (ADR-0295) ──

impl World {
    fn dock_req(&self, req: DockRequest) -> Result<DockOutcome, DockError> {
        apply::dock_tab(
            &self.state,
            &self.subs,
            &self.ev,
            &self.host,
            &self.labels,
            req,
        )
    }
}

#[test]
fn dock_tab_hands_the_source_window_to_the_given_successor() {
    for (pick, want) in [(Some(0), 0), (None, 2)] {
        let w = World::new();
        let (label, tabs) = w.popup_with(3);
        apply::switch_tab(&w.state, &w.subs, &w.ev, &label, tabs[1]).unwrap();
        w.forget();

        w.dock_req(DockRequest {
            view: tabs[1],
            to: DockDest::TabBar {
                window: MAIN_WINDOW_LABEL.into(),
                index: None,
                after: None,
            },
            focus: FocusPolicy::Reveal,
            mv: DockMove {
                successor: pick.map(|i: usize| tabs[i]),
                ..DockMove::default()
            },
        })
        .unwrap();

        assert_eq!(w.notified_tabs_of(&label).active, tabs[want], "{pick:?}");
        assert!(
            w.notified_views().contains(&tabs[want]),
            "{pick:?}: the new active tab's layout is carried too"
        );
        assert_notified(&w, "dock_tab(next active)");
        assert_invariants(&w);
    }
}

#[test]
fn dock_tab_seam_wraps_the_target_with_the_requested_share() {
    let w = World::new();
    let target = w.main_active();
    let (_, tabs) = w.popup_with(2);

    w.dock(
        tabs[1],
        DockDest::Seam {
            target,
            anchor: DockAnchor::Outer,
            side: DockSide::Right,
            share: 0.25,
        },
        FocusPolicy::Keep,
    )
    .unwrap();

    match w.layout_of(target) {
        LayoutNode::Split { ratio, .. } => assert_eq!(ratio, 0.75, "incoming share 1/4 · right"),
        other => panic!("should be a new split: {other:?}"),
    }
}

#[test]
fn dock_tab_seam_with_a_share_outside_its_range_changes_nothing() {
    let w = World::new();
    let target = w.main_active();
    let (_, tabs) = w.popup_with(2);
    let version = w.version();
    let resyncs = w.resyncs();
    w.forget();

    let out = w.dock(
        tabs[1],
        DockDest::Seam {
            target,
            anchor: DockAnchor::Outer,
            side: DockSide::Left,
            share: f64::NAN,
        },
        FocusPolicy::Reveal,
    );

    assert_eq!(out, Err(DockError::InvalidShare("NaN".into())));
    assert_eq!(w.version(), version);
    assert_eq!(w.resyncs(), resyncs);
    assert_eq!(w.ev.take_order(), Vec::<&str>::new());
    assert_invariants(&w);
}

// ── dock_tab: new window ──

#[test]
fn dock_tab_new_window_opens_then_attaches_and_records_the_placement() {
    let w = World::new();
    let first = w.main_active();
    let source = apply::create_tab(&w.state, &w.subs, &w.ev, MAIN_WINDOW_LABEL, None).unwrap();
    let resyncs = w.resyncs();
    w.forget();

    let out = w
        .dock(source, DockDest::NewWindow { at: AT }, FocusPolicy::Reveal)
        .unwrap();

    let label = out.window.clone();
    assert!(!out.reused);
    assert!(!out.prewarmed);
    assert_eq!(out.emptied, None);
    assert_eq!(
        &*w.host.opened_at.lock().unwrap(),
        &[(label.clone(), AT, true)],
        "main source = no size (pop-out default)"
    );
    assert_eq!(&*w.host.placed.lock().unwrap(), &[label.clone()]);
    assert!(w.host.closed.lock().unwrap().is_empty());
    assert_eq!(w.tabs_of(&label), [source]);
    assert_eq!(w.owner(source).as_deref(), Some(label.as_str()));
    assert_eq!(w.resyncs(), resyncs + 1);
    assert_eq!(w.notified_views(), [source, first]);
    assert_eq!(w.notified_windows(), [label.as_str(), MAIN_WINDOW_LABEL]);
    // TRD S21-slot-docking §8-1 ⑬ — order `open_at` → `dock` → notification → `record_placement`.
    assert_eq!(
        w.journal.take(),
        [
            format!("open_at:{label}"),
            "layout".to_string(),
            "layout".to_string(),
            format!("tabs:{label}"),
            format!("tabs:{MAIN_WINDOW_LABEL}"),
            format!("place:{label}"),
        ]
    );
    w.assert_notified_at_current_version();
    assert_notified(&w, "dock_tab(new window)");
    assert_invariants(&w);
}

#[test]
fn dock_tab_new_window_activates_only_when_revealing() {
    let w = World::new();
    let source = apply::create_tab(&w.state, &w.subs, &w.ev, MAIN_WINDOW_LABEL, None).unwrap();
    w.forget();

    let out = w
        .dock(source, DockDest::NewWindow { at: AT }, FocusPolicy::Keep)
        .unwrap();

    assert_eq!(
        &*w.host.opened_at.lock().unwrap(),
        &[(out.window, AT, false)],
        "Keep = does not take focus"
    );
    assert_eq!(
        w.host.claims_asked.load(Ordering::Relaxed),
        1,
        "even with Keep a prewarmed window is asked for (TRD §3-9 I4)"
    );
    assert!(!out.prewarmed);
}

#[test]
fn dock_tab_new_window_takes_the_source_popouts_normal_size_unless_given_one() {
    let w = World::new();
    let (label, tabs) = w.popup_with(3);
    w.observe(&label, false, bounds(50.0, 60.0, 640.0, 480.0));
    w.forget();

    w.dock(tabs[2], DockDest::NewWindow { at: AT }, FocusPolicy::Reveal)
        .unwrap();
    let given = WindowPlace {
        pos: AT.pos,
        size: Some((400.0, 300.0)),
    };
    w.dock(
        tabs[1],
        DockDest::NewWindow { at: given },
        FocusPolicy::Reveal,
    )
    .unwrap();

    let sizes: Vec<_> = w
        .host
        .opened_at
        .lock()
        .unwrap()
        .iter()
        .map(|(_, at, _)| (at.pos, at.size))
        .collect();
    assert_eq!(
        sizes,
        [
            (AT.pos, Some((640.0, 480.0))),
            (AT.pos, Some((400.0, 300.0)))
        ]
    );
}

#[test]
fn dock_tab_new_window_open_failure_changes_nothing() {
    let w = World::failing_host();
    let source = apply::create_tab(&w.state, &w.subs, &w.ev, MAIN_WINDOW_LABEL, None).unwrap();
    let version = w.version();
    let resyncs = w.resyncs();
    w.forget();

    let out = w.dock(source, DockDest::NewWindow { at: AT }, FocusPolicy::Reveal);

    assert!(matches!(out, Err(DockError::WindowOpen(_))), "{out:?}");
    assert_eq!(w.version(), version);
    assert_eq!(w.resyncs(), resyncs);
    assert_eq!(w.owner(source).as_deref(), Some(MAIN_WINDOW_LABEL));
    assert_eq!(w.ev.take_order(), Vec::<&str>::new());
    assert!(
        w.host.closed.lock().unwrap().is_empty(),
        "a window that failed to open has nothing to close"
    );
    assert!(w.host.placed.lock().unwrap().is_empty());
    assert_invariants(&w);
}

#[test]
fn dock_tab_new_window_closes_the_opened_window_when_phase_c_refuses() {
    let w = World::new();
    let source = apply::create_tab(&w.state, &w.subs, &w.ev, MAIN_WINDOW_LABEL, None).unwrap();
    w.forget();
    *w.host.during_open.lock().unwrap() = Some(Box::new(move |state: &LayoutState| {
        state
            .0
            .lock()
            .unwrap()
            .close_tab(MAIN_WINDOW_LABEL, source)
            .unwrap();
    }));
    let resyncs = w.resyncs();

    let out = w.dock(source, DockDest::NewWindow { at: AT }, FocusPolicy::Reveal);

    assert_eq!(out, Err(DockError::TabNotFound(source)));
    let opened: Vec<String> = w
        .host
        .opened_at
        .lock()
        .unwrap()
        .iter()
        .map(|(label, _, _)| label.clone())
        .collect();
    assert_eq!(opened.len(), 1);
    assert_eq!(
        &*w.host.closed.lock().unwrap(),
        &opened,
        "reaps the window just opened"
    );
    assert!(!w.window_exists(&opened[0]));
    assert_eq!(w.owner(source), None, "a closed tab does not attach to the new window");
    assert!(w.host.placed.lock().unwrap().is_empty());
    assert_eq!(w.resyncs(), resyncs);
    assert_eq!(w.ev.take_order(), Vec::<&str>::new());
    assert_invariants(&w);
}

// ── dock_tab: new window — prewarmed window (TRD S21-slot-docking §8-1 ⑬ · ADR-0299) ──

// Prewarm order `claim_spare` → `open_at` → `dock` → notification → `committed` → `record_placement`.
#[test]
fn dock_tab_new_window_opens_the_prewarmed_window_and_commits_after_the_notifications() {
    let w = World::new();
    let source = apply::create_tab(&w.state, &w.subs, &w.ev, MAIN_WINDOW_LABEL, None).unwrap();
    let spare = w.prewarm();
    w.forget();

    let out = w
        .dock(source, DockDest::NewWindow { at: AT }, FocusPolicy::Reveal)
        .unwrap();

    assert_eq!(out.window, spare, "opens with the handed-over window's label");
    assert!(out.prewarmed && !out.reused);
    assert_eq!(
        &*w.host.opened_at.lock().unwrap(),
        &[(spare.clone(), AT, true)]
    );
    assert_eq!(
        w.journal.take(),
        [
            format!("claim:{spare}"),
            format!("open_at:{spare}"),
            "layout".to_string(),
            "layout".to_string(),
            format!("tabs:{spare}"),
            format!("tabs:{MAIN_WINDOW_LABEL}"),
            format!("committed:{spare}"),
            format!("place:{spare}"),
        ]
    );
    assert_eq!(w.tabs_of(&spare), [source]);
    assert!(!w.claim_outstanding());
    assert_invariants(&w);
}

// `Keep` also uses a prewarmed window — not activating is received by the handed-over `open_at` (TRD §3-9 I4).
#[test]
fn dock_tab_new_window_uses_the_prewarmed_window_without_focus_too() {
    let w = World::new();
    let source = apply::create_tab(&w.state, &w.subs, &w.ev, MAIN_WINDOW_LABEL, None).unwrap();
    let spare = w.prewarm();
    w.forget();

    let out = w
        .dock(source, DockDest::NewWindow { at: AT }, FocusPolicy::Keep)
        .unwrap();

    assert!(out.prewarmed);
    assert_eq!(&*w.host.opened_at.lock().unwrap(), &[(spare, AT, false)]);
}

// If the handed-over window's `open_at` is `Err`, the window port has withdrawn that window — it is `WindowOpen` and there is no commit · take-back.
#[test]
fn dock_tab_new_window_whose_prewarmed_window_fails_to_open_changes_nothing() {
    let w = World::failing_host();
    let source = apply::create_tab(&w.state, &w.subs, &w.ev, MAIN_WINDOW_LABEL, None).unwrap();
    let version = w.version();
    let spare = w.prewarm();
    w.forget();

    let out = w.dock(source, DockDest::NewWindow { at: AT }, FocusPolicy::Reveal);

    assert!(matches!(out, Err(DockError::WindowOpen(_))), "{out:?}");
    assert_eq!(
        w.journal.take(),
        [format!("claim:{spare}"), format!("open_at:{spare}")]
    );
    assert_eq!(w.version(), version);
    assert!(!w.claim_outstanding(), "the window port reaped it");
}

// If the decision before attaching changes (the source tab closed while opening), the handed-over window is not closed but taken back.
#[test]
fn dock_tab_new_window_refused_at_the_attach_gives_the_prewarmed_window_back() {
    let w = World::new();
    let source = apply::create_tab(&w.state, &w.subs, &w.ev, MAIN_WINDOW_LABEL, None).unwrap();
    let spare = w.prewarm();
    w.forget();
    *w.host.during_open.lock().unwrap() = Some(Box::new(move |state: &LayoutState| {
        state
            .0
            .lock()
            .unwrap()
            .close_tab(MAIN_WINDOW_LABEL, source)
            .unwrap();
    }));

    let out = w.dock(source, DockDest::NewWindow { at: AT }, FocusPolicy::Reveal);

    assert_eq!(out, Err(DockError::TabNotFound(source)));
    assert_eq!(
        w.journal.take(),
        [
            format!("claim:{spare}"),
            format!("open_at:{spare}"),
            format!("release:{spare}"),
        ],
        "no close · notification · place record"
    );
    assert_eq!(w.owner(source), None);
    assert_invariants(&w);
}

// Even if that window drops out of the model while notifying (the first tab notification) · before the commit, the commit passes the claim ticket on as it is (TRD §3-9 · I27 · I31).
#[test]
fn dock_tab_new_window_commits_the_claim_even_if_the_window_left_the_model_before_it() {
    let w = World::new();
    let source = apply::create_tab(&w.state, &w.subs, &w.ev, MAIN_WINDOW_LABEL, None).unwrap();
    let spare = w.prewarm();
    let gone = spare.clone();
    *w.ev.on_tabs.lock().unwrap() = Some(Box::new(move |state: &LayoutState| {
        state.0.lock().unwrap().close_window(&gone).unwrap();
    }));
    w.host.vanished.lock().unwrap().push(spare.clone());
    w.forget();

    let out = w
        .dock(source, DockDest::NewWindow { at: AT }, FocusPolicy::Reveal)
        .unwrap();

    assert!(out.prewarmed);
    let journal = w.journal.take();
    assert_eq!(
        journal[journal.len() - 2..],
        [format!("committed:{spare}"), format!("place:{spare}")]
    );
    assert!(!w.window_exists(&spare));
    assert!(
        w.host.closed.lock().unwrap().is_empty(),
        "the apply service does not close"
    );
}

// ── dock_tab: reuse ──

impl World {
    fn lone_popout(&self) -> (String, Uuid) {
        let (label, tabs) = self.popup_with(1);
        self.observe(&label, false, bounds(40.0, 30.0, 800.0, 600.0));
        (label, tabs[0])
    }
}

#[test]
fn dock_tab_reuse_moves_the_source_window_once_and_keeps_the_model() {
    let w = World::new();
    let (label, view) = w.lone_popout();
    let slot = w.empty_slot(view);
    let source = apply::list_tabs(&w.state, &label).unwrap();
    let version = w.version();
    let resyncs = w.resyncs();
    let spare = w.prewarm();
    let asked = w.claims_asked();
    w.forget();

    let out = w
        .dock(view, DockDest::NewWindow { at: AT }, FocusPolicy::Reveal)
        .unwrap();

    w.assert_spare_not_asked(&spare, asked);
    assert_eq!(
        out,
        DockOutcome {
            changed: false,
            tab: view,
            views: vec![view],
            window: label.clone(),
            slot_ids: vec![slot],
            emptied: None,
            main_refilled: None,
            reused: true,
            prewarmed: false,
            identity: Some(NewWindow {
                window: label.clone(),
                window_id: source.window_id,
                number: source.number,
            }),
            rows: Vec::new(),
        }
    );
    assert_eq!(&*w.host.moves.lock().unwrap(), &[(label.clone(), AT)]);
    assert!(w.host.opened_at.lock().unwrap().is_empty(), "no new window");
    assert_eq!(&*w.host.placed.lock().unwrap(), &[label.clone()], "once");
    assert_eq!(w.version(), version, "model change 0");
    assert_eq!(w.resyncs(), resyncs);
    assert_eq!(w.ev.take_order(), Vec::<&str>::new(), "nothing to announce");
    assert_eq!(w.owner(view).as_deref(), Some(label.as_str()));
    assert_invariants(&w);
}

#[test]
fn dock_tab_a_lone_popout_without_a_normal_place_is_reused_too() {
    let w = World::new();
    let (label, tabs) = w.popup_with(1);
    let view = tabs[0];
    assert_eq!(
        w.state
            .0
            .lock()
            .unwrap()
            .window_attrs(&label)
            .unwrap()
            .bounds,
        None,
        "precondition: the normal place has not been seen yet"
    );
    let version = w.version();
    w.forget();

    let out = w
        .dock(view, DockDest::NewWindow { at: AT }, FocusPolicy::Reveal)
        .unwrap();

    assert!(out.reused);
    assert_eq!(out.window, label);
    assert_eq!(out.emptied, None);
    assert_eq!(&*w.host.moves.lock().unwrap(), &[(label.clone(), AT)]);
    assert!(w.host.opened_at.lock().unwrap().is_empty(), "no new window");
    assert!(w.host.closed.lock().unwrap().is_empty());
    assert_eq!(&*w.host.placed.lock().unwrap(), &[label.clone()]);
    assert_eq!(w.version(), version);
    assert_eq!(w.ev.take_order(), Vec::<&str>::new());
    assert_invariants(&w);
}

#[test]
fn dock_tab_reuse_finishes_the_drop_even_if_the_source_changes_during_the_move() {
    // User decision (2026-10-16): once the drag stands it stays where dropped; writes during the move are neither re-decided nor reverted — it ends with the model that write left. (what, the interjecting write, whether that window remains in the model)
    type Meddle = fn(&mut ViewManager, &str, Uuid, Uuid);
    let cases: [(&str, Meddle, bool); 3] = [
        (
            "slot added to the source tab",
            |mgr, _, view, slot| {
                mgr.split_slot(view, slot, SplitDir::TopBottom).unwrap();
            },
            true,
        ),
        (
            "tab added to that pop-out",
            |mgr, label, _, _| {
                mgr.create_tab(label, None).unwrap();
            },
            true,
        ),
        (
            "source tab closed (the window vanishes with it)",
            |mgr, label, view, _| {
                mgr.close_tab(label, view).unwrap();
            },
            false,
        ),
    ];
    for (what, meddle, kept) in cases {
        let w = World::new();
        let (label, view) = w.lone_popout();
        let slot = w.empty_slot(view);
        w.forget();
        let hooked = label.clone();
        *w.host.during_move.lock().unwrap() = Some(Box::new(move |state: &LayoutState| {
            meddle(&mut state.0.lock().unwrap(), &hooked, view, slot);
        }));

        let out = w
            .dock(view, DockDest::NewWindow { at: AT }, FocusPolicy::Reveal)
            .unwrap_or_else(|e| panic!("{what}: {e}"));

        assert!(out.reused, "{what}");
        assert_eq!(out.window, label, "{what}");
        assert_eq!(
            out.identity.as_ref().map(|w| w.window.as_str()),
            kept.then_some(label.as_str()),
            "{what}: a window closed while moving has no identity"
        );
        assert_eq!(w.host.moves.lock().unwrap().len(), 1, "{what}");
        assert!(w.host.opened_at.lock().unwrap().is_empty(), "{what}");
        assert!(
            w.host.closed.lock().unwrap().is_empty(),
            "{what}: not reaped"
        );
        assert_eq!(w.window_exists(&label), kept, "{what}");
        let placed = w.host.placed.lock().unwrap().clone();
        assert_eq!(
            placed,
            if kept { vec![label.clone()] } else { vec![] },
            "{what}: only a window remaining in the model records its place"
        );
        assert_eq!(w.ev.take_order(), Vec::<&str>::new(), "{what}");
        assert_invariants(&w);
    }
}

#[test]
fn dock_tab_reuse_move_failure_is_window_open_and_undoes_nothing() {
    let at = WindowPlace {
        pos: AT.pos,
        size: Some((500.0, 400.0)),
    };
    for (failing, called) in [
        (MoveStep::Position, vec![MoveStep::Position]),
        (MoveStep::Size, vec![MoveStep::Position, MoveStep::Size]),
    ] {
        let w = World::new();
        let (label, view) = w.lone_popout();
        let version = w.version();
        w.forget();
        w.host.fail_step(failing);
        let captured = Captured::default();

        let out = tracing::subscriber::with_default(captured.clone(), || {
            w.dock(view, DockDest::NewWindow { at }, FocusPolicy::Reveal)
        });

        assert!(
            matches!(out, Err(DockError::WindowOpen(_))),
            "{failing:?}: {out:?}"
        );
        let lines = captured.0.lock().unwrap().clone();
        let ends: Vec<&Line> = lines
            .iter()
            .filter(|l| l.message.contains("could not move the source window"))
            .collect();
        assert_eq!(ends.len(), 1, "{failing:?}: one end line — {lines:?}");
        assert_eq!(ends[0].level, tracing::Level::WARN, "{failing:?}");
        let step = format!("{failing:?}");
        assert_eq!(ends[0].field("step"), Some(step.as_str()));
        assert_eq!(ends[0].field("label"), Some(label.as_str()));
        assert_eq!(w.host.steps(), called, "{failing:?}: stops at the first failure");
        assert_eq!(
            w.host.moves.lock().unwrap().len(),
            1,
            "{failing:?}: does not move again"
        );
        assert!(w.host.placed.lock().unwrap().is_empty());
        assert!(w.host.closed.lock().unwrap().is_empty());
        assert_eq!(w.version(), version);
        assert_eq!(w.owner(view).as_deref(), Some(label.as_str()));
        assert_invariants(&w);
    }
}

// ── dock_tab: end log ──

#[derive(Debug, Clone)]
struct Line {
    level: tracing::Level,
    message: String,
    fields: Vec<(String, String)>,
}

/// Collects all events emitted on this thread (same shape as the same-named fake of the `settings` tests); creation sets up [`ASK_ALWAYS`].
#[derive(Clone)]
struct Captured(std::sync::Arc<Mutex<Vec<Line>>>);

impl Default for Captured {
    fn default() -> Self {
        LazyLock::force(&ASK_ALWAYS);
        Self(std::sync::Arc::default())
    }
}

// tracing-core 0.1.36: if only one subscriber was alive at the last registration, a callsite's first registration asks only that thread's default subscriber (`Dispatchers::rebuilder` → `JustOne`). A test thread outside the capture hitting a shared place such as "docking applied" first during a capture pins "no interest" globally and the capture comes up empty (38 of 120 runs at 6 parallel). Held for the whole binary, the first registration asks all live subscribers, and this one always answers "ask every time". The remaining gap — the same race at its first setup — is untested.
static ASK_ALWAYS: LazyLock<tracing::Dispatch> =
    LazyLock::new(|| tracing::Dispatch::new(AskAlways));

struct AskAlways;

impl tracing::Subscriber for AskAlways {
    fn register_callsite(
        &self,
        _: &'static tracing::Metadata<'static>,
    ) -> tracing::subscriber::Interest {
        tracing::subscriber::Interest::sometimes()
    }
    fn enabled(&self, _: &tracing::Metadata<'_>) -> bool {
        false
    }
    fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        tracing::span::Id::from_u64(1)
    }
    fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}
    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
    fn event(&self, _: &tracing::Event<'_>) {}
    fn enter(&self, _: &tracing::span::Id) {}
    fn exit(&self, _: &tracing::span::Id) {}
}

impl Line {
    fn field(&self, name: &str) -> Option<&str> {
        self.fields
            .iter()
            .find(|(field, _)| field == name)
            .map(|(_, value)| value.as_str())
    }
}

struct Fields<'a>(&'a mut Line);

impl tracing::field::Visit for Fields<'_> {
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        self.0
            .fields
            .push((field.name().to_string(), value.to_string()));
    }

    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        let text = format!("{value:?}");
        if field.name() == "message" {
            self.0.message = text;
        } else {
            self.0.fields.push((field.name().to_string(), text));
        }
    }
}

impl tracing::Subscriber for Captured {
    fn register_callsite(
        &self,
        _: &'static tracing::Metadata<'static>,
    ) -> tracing::subscriber::Interest {
        tracing::subscriber::Interest::sometimes()
    }
    fn enabled(&self, _: &tracing::Metadata<'_>) -> bool {
        true
    }
    fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        tracing::span::Id::from_u64(1)
    }
    fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}
    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
    fn event(&self, event: &tracing::Event<'_>) {
        let mut line = Line {
            level: *event.metadata().level(),
            message: String::new(),
            fields: Vec::new(),
        };
        event.record(&mut Fields(&mut line));
        self.0.lock().unwrap().push(line);
    }
    fn enter(&self, _: &tracing::span::Id) {}
    fn exit(&self, _: &tracing::span::Id) {}
}

#[test]
fn dock_tab_onto_the_same_place_logs_that_nothing_changed() {
    let w = World::new();
    let (label, tabs) = w.popup_with(2);
    let captured = Captured::default();

    tracing::subscriber::with_default(captured.clone(), || {
        w.dock(
            tabs[1],
            DockDest::TabBar {
                window: label.clone(),
                index: Some(1),
                after: None,
            },
            FocusPolicy::Keep,
        )
        .unwrap();
        w.dock(
            tabs[1],
            DockDest::TabBar {
                window: label.clone(),
                index: Some(0),
                after: None,
            },
            FocusPolicy::Keep,
        )
        .unwrap();
    });

    let lines = captured.0.lock().unwrap().clone();
    let applied: Vec<(tracing::Level, Option<&str>)> = lines
        .iter()
        .filter(|l| l.message.contains("docking applied"))
        .map(|l| (l.level, l.field("changed")))
        .collect();
    assert_eq!(
        applied,
        [
            (tracing::Level::INFO, Some("false")),
            (tracing::Level::INFO, None)
        ],
        "only an unchanged outcome records it"
    );
}

// ── drop command — find the target again at drop time (ADR-0297 · TRD S21-slot-docking §3-3 · §8-1 ⑦) ─────────────────────
//
// `tab_drop` carries what was decided with the web store; even if the store lags, if what the drop pointed to (dragged tab · target tab · target area · tab bar window) exists now the same `dock_tab` applies it to the current state, else rejects. The request is built with production's pure decision (`tab_drop::plan::plan`).

const FLAT_FRAME: SourceFrame = SourceFrame {
    insets: Insets {
        t: 0.0,
        r: 0.0,
        b: 0.0,
        l: 0.0,
    },
    scale: 1.0,
};

fn dropped_at(x: i32, y: i32) -> InputNow {
    InputNow {
        cursor: Some(PhysPoint { x, y }),
        primary_down: false,
        escape_down: false,
        top: TopAt::Unknown,
        scale: 1.0,
    }
}

impl World {
    fn active_of(&self, label: &str) -> Uuid {
        apply::list_tabs(&self.state, label).unwrap().active
    }

    fn tab_drop(&self, drop: &TabDrop, caller: &str) -> Result<DockOutcome, DockError> {
        match drop_plan::plan(drop, caller, &dropped_at(500, 400), &FLAT_FRAME) {
            Plan::Dock(req) => self.dock_req(req),
            other => panic!("should be a docking request: {other:?}"),
        }
    }

    fn tab_bar_drop(&self, view: Uuid, from: &str, initial: Option<Uuid>, index: u32) -> TabDrop {
        TabDrop {
            drag: Uuid::new_v4(),
            view,
            from: from.into(),
            initial,
            to: DropTo::TabBar { index },
        }
    }

    fn seam_drop(
        &self,
        view: Uuid,
        from: &str,
        initial: Option<Uuid>,
        target: Uuid,
        anchor: DropAnchor,
    ) -> TabDrop {
        TabDrop {
            drag: Uuid::new_v4(),
            view,
            from: from.into(),
            initial,
            to: DropTo::Seam {
                target,
                anchor,
                side: DockSide::Right,
                share: 0.5,
            },
        }
    }

    fn new_window_drop(&self, view: Uuid, from: &str) -> TabDrop {
        TabDrop {
            drag: Uuid::new_v4(),
            view,
            from: from.into(),
            initial: None,
            to: DropTo::NewWindow {
                by: NewWindowBy::Drop,
                size: None,
                grab: PointCss { x: 0.0, y: 0.0 },
            },
        }
    }

    fn assert_refused(&self, got: Result<DockOutcome, DockError>, want: DockError, what: &str) {
        let version = self.version();
        assert_eq!(got, Err(want), "{what}");
        assert_eq!(self.version(), version, "{what}: model unchanged");
    }
}

#[test]
fn a_drop_whose_tab_left_its_source_window_moves_it_from_where_it_is_now() {
    let setup = || {
        let w = World::new();
        let m0 = w.main_active();
        let t = apply::create_tab(&w.state, &w.subs, &w.ev, MAIN_WINDOW_LABEL, None).unwrap();
        let popout = w.popup();
        let p = w.active_of(&popout);
        w.dock(
            t,
            DockDest::TabBar {
                window: popout.clone(),
                index: None,
                after: None,
            },
            FocusPolicy::Keep,
        )
        .unwrap();
        w.forget();
        (w, m0, t, popout, p)
    };

    let (w, m0, t, popout, p) = setup();
    let out = w
        .tab_drop(
            &w.tab_bar_drop(t, MAIN_WINDOW_LABEL, Some(m0), 0),
            MAIN_WINDOW_LABEL,
        )
        .expect("tab bar");
    assert_eq!(out.window, MAIN_WINDOW_LABEL);
    assert_eq!(w.tabs_of(MAIN_WINDOW_LABEL), [t, m0]);
    assert_eq!(w.tabs_of(&popout), [p], "removed from the current window (pop-out)");
    assert_invariants(&w);

    let (w, _, t, popout, p) = setup();
    let out = w
        .tab_drop(
            &w.seam_drop(t, MAIN_WINDOW_LABEL, None, p, DropAnchor::Outer),
            &popout,
        )
        .expect("seam");
    assert_eq!(out.window, popout);
    assert_eq!(w.tabs_of(&popout), [p]);
    assert_eq!(w.owner(t), None, "t attached to p and vanished");
    assert_invariants(&w);

    let (w, _, t, popout, p) = setup();
    let out = w
        .tab_drop(&w.new_window_drop(t, MAIN_WINDOW_LABEL), &popout)
        .expect("new window");
    assert!(!out.reused, "the pop-out has two tabs, so a new window");
    assert_eq!(w.owner(t).as_deref(), Some(out.window.as_str()));
    assert_eq!(w.tabs_of(&popout), [p]);
    assert_eq!(w.host.opened_at.lock().unwrap().len(), 1);
    assert_invariants(&w);
}

// `initial` is the selection of the window the drag started in (main) — the window the dragged tab went to in the meantime (the popout) follows the shell succession even if `initial` is there.
#[test]
fn a_drop_whose_tab_left_its_source_window_leaves_that_window_to_the_shell_succession() {
    let w = World::new();
    let i = w.main_active();
    let t = apply::create_tab(&w.state, &w.subs, &w.ev, MAIN_WINDOW_LABEL, None).unwrap();
    let drop = w.tab_bar_drop(t, MAIN_WINDOW_LABEL, Some(i), 0);
    let popout = w.popup();
    let p = w.active_of(&popout);
    for (tab, index) in [(i, Some(0)), (t, None)] {
        w.dock(
            tab,
            DockDest::TabBar {
                window: popout.clone(),
                index,
                after: None,
            },
            FocusPolicy::Reveal,
        )
        .unwrap();
    }
    assert_eq!(w.tabs_of(&popout), [i, p, t], "precondition");
    assert_eq!(w.active_of(&popout), t, "precondition");
    w.tab_drop(&drop, MAIN_WINDOW_LABEL).expect("accepts");
    assert_eq!(w.tabs_of(&popout), [i, p]);
    assert_eq!(
        w.active_of(&popout),
        p,
        "succession (left neighbour) — not initial"
    );
    assert_eq!(w.active_of(MAIN_WINDOW_LABEL), t);
    assert_invariants(&w);
}

#[test]
fn a_drop_whose_tab_is_gone_is_refused_before_opening_anything() {
    let w = World::new();
    let m0 = w.main_active();
    let t = apply::create_tab(&w.state, &w.subs, &w.ev, MAIN_WINDOW_LABEL, None).unwrap();
    let drops = [
        w.tab_bar_drop(t, MAIN_WINDOW_LABEL, Some(m0), 0),
        w.seam_drop(t, MAIN_WINDOW_LABEL, Some(m0), m0, DropAnchor::Outer),
        w.new_window_drop(t, MAIN_WINDOW_LABEL),
    ];
    apply::close_tab(&w.state, &w.subs, &w.ev, &w.host, MAIN_WINDOW_LABEL, t).unwrap();
    w.forget();
    for drop in &drops {
        w.assert_refused(
            w.tab_drop(drop, MAIN_WINDOW_LABEL),
            DockError::TabNotFound(t),
            &format!("{:?}", drop.to),
        );
    }
    assert!(
        w.host.opened_at.lock().unwrap().is_empty(),
        "refused before opening the window"
    );
    assert!(w.host.moves.lock().unwrap().is_empty());
    assert_eq!(w.ev.take_order(), Vec::<&str>::new(), "no notification");
}

#[test]
fn a_tab_bar_drop_lands_in_the_tab_list_as_it_is_now_and_clamps_to_its_end() {
    let w = World::new();
    let m0 = w.main_active();
    let a = apply::create_tab(&w.state, &w.subs, &w.ev, MAIN_WINDOW_LABEL, None).unwrap();
    let (popout, tabs) = w.popup_with(2);
    let t = tabs[1];
    let drop = w.tab_bar_drop(t, &popout, Some(tabs[0]), 1);
    let c = apply::create_tab(&w.state, &w.subs, &w.ev, MAIN_WINDOW_LABEL, None).unwrap();
    w.tab_drop(&drop, MAIN_WINDOW_LABEL).expect("accepts");
    assert_eq!(w.tabs_of(MAIN_WINDOW_LABEL), [m0, t, a, c]);
    assert_eq!(w.active_of(MAIN_WINDOW_LABEL), t, "the dropped tab is shown");
    assert_invariants(&w);

    let w = World::new();
    let m0 = w.main_active();
    let a = apply::create_tab(&w.state, &w.subs, &w.ev, MAIN_WINDOW_LABEL, None).unwrap();
    let b = apply::create_tab(&w.state, &w.subs, &w.ev, MAIN_WINDOW_LABEL, None).unwrap();
    let (popout, tabs) = w.popup_with(2);
    let t = tabs[1];
    let drop = w.tab_bar_drop(t, &popout, Some(tabs[0]), 3);
    apply::close_tab(&w.state, &w.subs, &w.ev, &w.host, MAIN_WINDOW_LABEL, b).unwrap();
    w.tab_drop(&drop, MAIN_WINDOW_LABEL).expect("accepts");
    assert_eq!(w.tabs_of(MAIN_WINDOW_LABEL), [m0, a, t]);
    assert_eq!(w.tabs_of(&popout), [tabs[0]]);
    assert_invariants(&w);

    let w = World::new();
    let m0 = w.main_active();
    let a = apply::create_tab(&w.state, &w.subs, &w.ev, MAIN_WINDOW_LABEL, None).unwrap();
    let t = apply::create_tab(&w.state, &w.subs, &w.ev, MAIN_WINDOW_LABEL, None).unwrap();
    let b = apply::create_tab(&w.state, &w.subs, &w.ev, MAIN_WINDOW_LABEL, None).unwrap();
    let drop = w.tab_bar_drop(t, MAIN_WINDOW_LABEL, Some(m0), 3);
    apply::close_tab(&w.state, &w.subs, &w.ev, &w.host, MAIN_WINDOW_LABEL, a).unwrap();
    w.tab_drop(&drop, MAIN_WINDOW_LABEL).expect("accepts");
    assert_eq!(w.tabs_of(MAIN_WINDOW_LABEL), [m0, b, t]);
    assert_invariants(&w);
}

#[test]
fn a_tab_bar_drop_into_a_window_that_closed_is_refused() {
    let w = World::new();
    let t = apply::create_tab(&w.state, &w.subs, &w.ev, MAIN_WINDOW_LABEL, None).unwrap();
    let popout = w.popup();
    let drop = w.tab_bar_drop(t, MAIN_WINDOW_LABEL, None, 0);
    apply::close_window(&w.state, &w.subs, &w.host, &popout).unwrap();
    w.forget();
    w.assert_refused(
        w.tab_drop(&drop, &popout),
        DockError::WindowNotFound(popout.clone()),
        "closed window",
    );
    assert_eq!(w.owner(t).as_deref(), Some(MAIN_WINDOW_LABEL));
}

#[test]
fn a_seam_drop_lands_in_the_target_layout_as_it_is_now() {
    let setup = || {
        let w = World::new();
        let t = apply::create_tab(&w.state, &w.subs, &w.ev, MAIN_WINDOW_LABEL, None).unwrap();
        let popout = w.popup();
        let q = w.active_of(&popout);
        let s = w.empty_slot(q);
        (w, t, popout, q, s)
    };

    // The reference area was split in the meantime — that area remains, so it attaches to that area.
    let (w, t, popout, q, s) = setup();
    let moved = w.slots(t);
    let drop = w.seam_drop(t, MAIN_WINDOW_LABEL, None, q, DropAnchor::Slot { id: s });
    let n = apply::split_slot(&w.state, &w.subs, &w.ev, q, s, SplitDir::TopBottom).unwrap();
    apply::rename_tab(&w.state, &w.ev, q, "new name".into()).unwrap();
    let out = w.tab_drop(&drop, &popout).expect("accepts");
    assert_eq!(out.window, popout);
    let slots = w.slots(q);
    assert!(
        [s, n].iter().chain(&moved).all(|x| slots.contains(x)),
        "two split areas + the attached area: {slots:?}"
    );
    assert_eq!(slots.len(), 2 + moved.len());
    assert_eq!(w.owner(t), None);
    assert_invariants(&w);

    // Outer side — attaches to the whole target tab that was split in the meantime.
    let (w, t, popout, q, s) = setup();
    let drop = w.seam_drop(t, MAIN_WINDOW_LABEL, None, q, DropAnchor::Outer);
    apply::split_slot(&w.state, &w.subs, &w.ev, q, s, SplitDir::LeftRight).unwrap();
    assert!(w.tab_drop(&drop, &popout).is_ok());
    assert_invariants(&w);

    // The reference area vanished in the meantime — rejected.
    let (w, t, popout, q, s) = setup();
    let n = apply::split_slot(&w.state, &w.subs, &w.ev, q, s, SplitDir::TopBottom).unwrap();
    let drop = w.seam_drop(t, MAIN_WINDOW_LABEL, None, q, DropAnchor::Slot { id: n });
    apply::close_slot(&w.state, &w.subs, &w.ev, q, n).unwrap();
    w.assert_refused(
        w.tab_drop(&drop, &popout),
        DockError::SlotNotFound(n),
        "no reference area",
    );

    // The target tab vanished in the meantime — rejected.
    let (w, t, popout, q, _) = setup();
    let other = apply::create_tab(&w.state, &w.subs, &w.ev, &popout, None).unwrap();
    let drop = w.seam_drop(t, MAIN_WINDOW_LABEL, None, q, DropAnchor::Outer);
    apply::close_tab(&w.state, &w.subs, &w.ev, &w.host, &popout, q).unwrap();
    w.assert_refused(
        w.tab_drop(&drop, &popout),
        DockError::TabNotFound(q),
        "no target tab",
    );
    assert_eq!(w.tabs_of(&popout), [other]);
}

#[test]
fn a_seam_drop_onto_a_tab_that_moved_to_another_window_docks_where_that_tab_is_now() {
    let w = World::new();
    let t = apply::create_tab(&w.state, &w.subs, &w.ev, MAIN_WINDOW_LABEL, None).unwrap();
    let q = apply::create_tab(&w.state, &w.subs, &w.ev, MAIN_WINDOW_LABEL, None).unwrap();
    let drop = w.seam_drop(t, MAIN_WINDOW_LABEL, None, q, DropAnchor::Outer);
    let popout = w.popup();
    w.dock(
        q,
        DockDest::TabBar {
            window: popout.clone(),
            index: None,
            after: None,
        },
        FocusPolicy::Keep,
    )
    .unwrap();
    let out = w.tab_drop(&drop, MAIN_WINDOW_LABEL).expect("accepts");
    assert_eq!(out.window, popout);
    assert_eq!(w.active_of(&popout), q, "the target tab is shown");
    assert_eq!(w.owner(t), None);
    assert_invariants(&w);
}

// main = [i, d] — pressing d (active = d) shows the remaining tab i during the drag, and the user drops at i's seam (same window).
#[test]
fn a_same_window_seam_applies_whichever_tab_the_window_shows_now() {
    let setup = || {
        let w = World::new();
        let i = w.main_active();
        let d = apply::create_tab(&w.state, &w.subs, &w.ev, MAIN_WINDOW_LABEL, None).unwrap();
        assert_eq!(w.active_of(MAIN_WINDOW_LABEL), d, "the pressed tab is active");
        (w, i, d)
    };

    let (w, i, d) = setup();
    let drop = w.seam_drop(d, MAIN_WINDOW_LABEL, Some(i), i, DropAnchor::Outer);
    assert!(w.tab_drop(&drop, MAIN_WINDOW_LABEL).is_ok());
    assert_eq!(w.tabs_of(MAIN_WINDOW_LABEL), [i]);
    assert_invariants(&w);

    let (w, i, d) = setup();
    let drop = w.seam_drop(d, MAIN_WINDOW_LABEL, Some(i), i, DropAnchor::Outer);
    apply::switch_tab(&w.state, &w.subs, &w.ev, MAIN_WINDOW_LABEL, i).unwrap();
    let out = w.tab_drop(&drop, MAIN_WINDOW_LABEL).unwrap();
    assert_eq!(out.window, MAIN_WINDOW_LABEL);
    assert_eq!(
        w.tabs_of(MAIN_WINDOW_LABEL),
        [i],
        "d attached to i's seam"
    );

    let (w, i, d) = setup();
    let x = apply::create_tab(&w.state, &w.subs, &w.ev, MAIN_WINDOW_LABEL, None).unwrap();
    apply::switch_tab(&w.state, &w.subs, &w.ev, MAIN_WINDOW_LABEL, d).unwrap();
    let drop = w.seam_drop(d, MAIN_WINDOW_LABEL, Some(i), i, DropAnchor::Outer);
    apply::switch_tab(&w.state, &w.subs, &w.ev, MAIN_WINDOW_LABEL, x).unwrap();
    assert!(w.tab_drop(&drop, MAIN_WINDOW_LABEL).is_ok());
    assert_eq!(w.tabs_of(MAIN_WINDOW_LABEL), [i, x]);
    assert_eq!(w.active_of(MAIN_WINDOW_LABEL), i);
    assert_invariants(&w);
}

#[test]
fn a_lone_popout_new_window_drop_reuses_the_window_and_keeps_its_size() {
    let w = World::new();
    let (label, view) = w.lone_popout();
    let version = w.version();
    w.forget();
    let drop = TabDrop {
        drag: Uuid::new_v4(),
        view,
        from: label.clone(),
        initial: Some(view),
        to: DropTo::NewWindow {
            by: NewWindowBy::Drop,
            size: None,
            grab: PointCss { x: 30.0, y: 12.0 },
        },
    };
    let out = w.tab_drop(&drop, &label).unwrap();
    assert!(out.reused);
    assert_eq!(out.window, label);
    assert_eq!(
        &*w.host.moves.lock().unwrap(),
        &[(
            label.clone(),
            WindowPlace {
                pos: Some((470, 388)),
                size: None,
            }
        )],
        "moves to the dragged place and does not pick a size"
    );
    assert!(w.host.opened_at.lock().unwrap().is_empty(), "no new window");
    assert_eq!(w.version(), version, "model change 0");
    assert_invariants(&w);
}

// ── target version — resolved inside the same lock that changes the target (TRD S28 §5-2 · ADR-0307) ─────────────────────────────

impl World {
    fn put(&self, view: Uuid, slot: Uuid, agent: Uuid) {
        apply::assign_agent(
            &self.state,
            &self.subs,
            &self.ev,
            view,
            slot,
            agent.to_string(),
        )
        .unwrap();
    }

    fn put_agent(&self, view: Uuid, slot: Uuid) -> Uuid {
        let agent = Uuid::new_v4();
        self.put(view, slot, agent);
        agent
    }

    fn split(&self, view: Uuid, slot: Uuid, dir: SplitDir) -> Uuid {
        apply::split_slot(&self.state, &self.subs, &self.ev, view, slot, dir).unwrap()
    }

    fn new_tab(&self, label: &str) -> Uuid {
        apply::create_tab(&self.state, &self.subs, &self.ev, label, None).unwrap()
    }

    fn name(&self, agent: Uuid, name: &str) {
        apply::agent_names_updated(
            &self.state,
            &self.ev,
            &[(agent.to_string(), name.to_string())],
        )
        .unwrap();
    }

    fn locate(&self, target: &Target) -> Result<Located, LocateError> {
        target::locate(&self.state.0.lock().unwrap(), target)
    }

    fn content(&self, view: Uuid, slot: Uuid) -> SlotContent {
        self.state
            .0
            .lock()
            .unwrap()
            .slot_content(view, slot)
            .unwrap()
    }

    fn first_slot(&self, view: Uuid) -> Uuid {
        tree::first_slot_id(&self.snapshot(view).layout)
    }

    fn dock_t(
        &self,
        tab: Target,
        to: TargetDest,
        focus: FocusPolicy,
    ) -> Result<DockOutcome, TargetDockError> {
        apply::dock_target(
            &self.state,
            &self.subs,
            &self.ev,
            &self.host,
            &self.labels,
            TargetDockRequest {
                tab,
                to,
                focus,
                mv: DockMove::default(),
            },
        )
    }
}

fn at(window: &str, view: Uuid, slot: Option<Uuid>) -> Located {
    Located {
        window: window.to_string(),
        view,
        slot,
    }
}

fn tab_bar(window: Option<&str>, index: Option<usize>) -> TargetDest {
    TargetDest::TabBar(TabBarSpot::At {
        window: window.map(str::to_string),
        index,
    })
}

fn after(agent: Uuid) -> TargetDest {
    TargetDest::TabBar(TabBarSpot::After(Target::TabOf(agent)))
}

const NO_PLACE: WindowPlace = WindowPlace {
    pos: None,
    size: None,
};

// ── target version: resolving ──

#[test]
fn locate_finds_a_tab_and_a_slot_by_id() {
    let w = World::new();
    let main = w.main_active();
    let left = w.empty_slot(main);
    let (label, tabs) = w.popup_with(1);
    let pop_slot = w.empty_slot(tabs[0]);

    assert_eq!(
        w.locate(&Target::Tab(tabs[0])),
        Ok(at(&label, tabs[0], None))
    );
    assert_eq!(
        w.locate(&Target::Slot {
            view: Some(main),
            slot: left
        }),
        Ok(at(MAIN_WINDOW_LABEL, main, Some(left)))
    );
    assert_eq!(
        w.locate(&Target::Slot {
            view: None,
            slot: pop_slot
        }),
        Ok(at(&label, tabs[0], Some(pop_slot))),
        "finds that tab · window from the area id alone"
    );

    let bogus = Uuid::new_v4();
    let misses = [
        (Target::Tab(bogus), LocateError::TabNotFound(bogus)),
        (
            Target::Slot {
                view: Some(bogus),
                slot: left,
            },
            LocateError::TabNotFound(bogus),
        ),
        (
            Target::Slot {
                view: Some(main),
                slot: pop_slot,
            },
            LocateError::SlotNotInTab {
                slot: pop_slot,
                tab: main,
            },
        ),
        (
            Target::Slot {
                view: None,
                slot: bogus,
            },
            LocateError::SlotNotFound(bogus),
        ),
    ];
    for (target, want) in misses {
        let got = w.locate(&target).unwrap_err();
        assert_eq!(got, want);
        assert_eq!(got.by(), TargetBy::Id, "{want}");
        assert!(got.candidates().is_empty());
    }
}

#[test]
fn locate_by_agent_takes_one_place_and_refuses_several_or_none_with_candidates() {
    let w = World::new();
    // main tab a = [m0 one | m1 twice | m2 twice] · main tab b = [b0 spread] · popout = [p0 spread]
    let a = w.main_active();
    let m0 = w.empty_slot(a);
    let m1 = w.split(a, m0, SplitDir::LeftRight);
    let m2 = w.split(a, m1, SplitDir::TopBottom);
    let one = w.put_agent(a, m0);
    let twice = w.put_agent(a, m1);
    w.put(a, m2, twice);
    let b = w.new_tab(MAIN_WINDOW_LABEL);
    let b0 = w.empty_slot(b);
    let spread = w.put_agent(b, b0);
    let (label, tabs) = w.popup_with(1);
    let p0 = w.empty_slot(tabs[0]);
    w.put(tabs[0], p0, spread);
    w.name(one, "alpha");
    w.name(twice, "beta");

    assert_eq!(
        w.locate(&Target::SlotOf(one)),
        Ok(at(MAIN_WINDOW_LABEL, a, Some(m0)))
    );
    assert_eq!(
        w.locate(&Target::TabOf(one)),
        Ok(at(MAIN_WINDOW_LABEL, a, None))
    );
    assert_eq!(
        w.locate(&Target::TabOf(twice)),
        Ok(at(MAIN_WINDOW_LABEL, a, None)),
        "two areas inside one tab means one tab"
    );

    let LocateError::AgentAmbiguous { agent, places } =
        w.locate(&Target::SlotOf(twice)).unwrap_err()
    else {
        panic!("two areas are ambiguous");
    };
    assert_eq!(agent, twice);
    assert_eq!(places.iter().map(|p| p.slot).collect::<Vec<_>>(), [m1, m2]);

    let spread_err = w.locate(&Target::TabOf(spread)).unwrap_err();
    assert_eq!(spread_err.by(), TargetBy::Agent);
    let LocateError::AgentAmbiguous { places, .. } = &spread_err else {
        panic!("two tabs are ambiguous: {spread_err}");
    };
    assert_eq!(
        places
            .iter()
            .map(|p| (p.window.as_str(), p.number, p.view, p.tab_index, p.slot))
            .collect::<Vec<_>>(),
        [
            (MAIN_WINDOW_LABEL, 1, b, 1, b0),
            (label.as_str(), 2, tabs[0], 0, p0)
        ],
        "window number order → tab row order"
    );
    let candidates = spread_err.candidates();
    assert_eq!(candidates.len(), 2);
    assert!(candidates[0].contains(&b0.to_string()) && candidates[0].contains(MAIN_WINDOW_LABEL));
    assert!(candidates[1].contains(&p0.to_string()) && candidates[1].contains(&label));

    let ghost = Uuid::new_v4();
    let missing = w.locate(&Target::SlotOf(ghost)).unwrap_err();
    assert_eq!(
        missing,
        LocateError::AgentNotShown {
            agent: ghost,
            shown: vec!["alpha".into(), "beta".into(), spread.to_string()],
        },
        "shown agents — without overlap · the id if there is no name"
    );
    assert_eq!(missing.by(), TargetBy::Agent);
    assert_eq!(missing.candidates().len(), 3);
}

#[test]
fn not_shown_candidates_stop_at_five() {
    let w = World::new();
    let main = w.main_active();
    let mut slot = w.empty_slot(main);
    for i in 0..6 {
        let agent = w.put_agent(main, slot);
        w.name(agent, &format!("a{i}"));
        if i < 5 {
            slot = w.split(main, slot, SplitDir::LeftRight);
        }
    }
    let LocateError::AgentNotShown { shown, .. } =
        w.locate(&Target::TabOf(Uuid::new_v4())).unwrap_err()
    else {
        panic!("a nonexistent agent");
    };
    assert_eq!(shown, ["a0", "a1", "a2", "a3", "a4"]);
}

#[test]
fn locate_by_spatial_word_reads_the_visible_tab_of_the_window() {
    let w = World::new();
    let main = w.main_active();
    let left = w.empty_slot(main);
    let right = w.split(main, left, SplitDir::LeftRight); // focus = right

    let word = |token, window: Option<&str>| {
        w.locate(&Target::At {
            token,
            window: window.map(str::to_string),
        })
    };
    assert_eq!(
        word(SpatialToken::TopRight, None),
        Ok(at(MAIN_WINDOW_LABEL, main, Some(right)))
    );
    assert_eq!(
        word(SpatialToken::Left, None),
        Ok(at(MAIN_WINDOW_LABEL, main, Some(left))),
        "direction relative to the focus area"
    );
    let miss = word(SpatialToken::Right, None).unwrap_err();
    assert_eq!(
        miss,
        LocateError::NoSlotAt {
            token: SpatialToken::Right,
            window: MAIN_WINDOW_LABEL.into()
        }
    );
    assert_eq!(miss.by(), TargetBy::Spatial);

    let fresh = w.new_tab(MAIN_WINDOW_LABEL);
    assert_eq!(
        word(SpatialToken::TopRight, None).unwrap().view,
        fresh,
        "resolved in the visible tab, not a hidden one"
    );
    let label = w.popup();
    let popout_tab = w.active_of(&label);
    assert_eq!(
        word(SpatialToken::TopLeft, Some(&label)).unwrap(),
        at(&label, popout_tab, Some(w.first_slot(popout_tab)))
    );
    let gone = word(SpatialToken::TopLeft, Some("no-such")).unwrap_err();
    assert_eq!(gone, LocateError::WindowNotFound("no-such".into()));
    assert_eq!(gone.by(), TargetBy::Spatial);
}


// Sequential, so it cannot catch the regression "resolve in one lock, move in another". That beside-area · tab-row docking resolves and moves in one lock is kept by structure: the resolving closure receives the lock's `&ViewManager` only as an argument (`dock_in_place` of `apply.rs`). What really races between locks is only new-window docking (two locks; the two tests below — window port · window move interjection). No test-only interjection door exists in production code: where that door sits is exactly the gap measured, so regressed code could move it too.
#[test]
fn a_tab_the_agent_left_after_its_name_was_resolved_is_never_moved() {
    let w = World::new();
    let (label, tabs) = w.popup_with(2);
    let slot = w.empty_slot(tabs[0]);
    // After the daemon resolved the name to this id and before applying, another write changed that area to another agent; applying by the resolved tab id would move tabs[0], which the agent left.
    let resolved = w.put_agent(tabs[0], slot);
    w.put(tabs[0], slot, Uuid::new_v4());
    let version = w.version();
    let resyncs = w.resyncs();
    w.forget();

    let err = w
        .dock_t(
            Target::TabOf(resolved),
            tab_bar(Some(MAIN_WINDOW_LABEL), None),
            FocusPolicy::Keep,
        )
        .unwrap_err();

    assert!(
        matches!(&err, TargetDockError::Subject(LocateError::AgentNotShown { agent, .. }) if *agent == resolved),
        "{err:?}"
    );
    assert_eq!(w.tabs_of(&label), tabs, "the tab that agent left stays in place");
    assert_eq!(w.version(), version);
    assert_eq!(w.resyncs(), resyncs);
    assert_eq!(w.ev.take_order(), Vec::<&str>::new());

    // If that agent moved to another tab, the tab it is in now is moved — not the old tab resolved earlier.
    w.put(tabs[1], w.empty_slot(tabs[1]), resolved);
    let out = w
        .dock_t(
            Target::TabOf(resolved),
            tab_bar(Some(MAIN_WINDOW_LABEL), None),
            FocusPolicy::Keep,
        )
        .unwrap();
    assert_eq!(out.tab, tabs[1]);
    assert_eq!(w.tabs_of(&label), [tabs[0]]);
    assert_eq!(w.owner(tabs[1]).as_deref(), Some(MAIN_WINDOW_LABEL));
    assert_invariants(&w);
}

#[test]
fn a_detach_whose_target_changes_while_the_window_opens_withdraws_it_and_moves_nothing() {
    // (prewarmed window handover, the tab given by id closed) — otherwise that agent moves to another tab.
    for (prewarmed, closes) in [(false, false), (true, false), (false, true)] {
        let w = World::new();
        let main = w.main_active();
        let first = w.new_tab(MAIN_WINDOW_LABEL);
        let slot = w.empty_slot(main);
        let other = w.empty_slot(first);
        let agent = w.put_agent(main, slot);
        let spare = prewarmed.then(|| w.prewarm());
        let target = if closes {
            Target::Tab(first)
        } else {
            Target::TabOf(agent)
        };
        *w.host.during_open.lock().unwrap() = Some(Box::new(move |state: &LayoutState| {
            let mut mgr = state.0.lock().unwrap();
            if closes {
                mgr.close_tab(MAIN_WINDOW_LABEL, first).unwrap();
            } else {
                mgr.set_slot_content(main, slot, SlotContent::Empty)
                    .unwrap();
                mgr.assign_agent(first, other, agent.to_string()).unwrap();
            }
        }));
        let resyncs = w.resyncs();
        w.forget();

        let out = w.dock_t(target, TargetDest::NewWindow { at: AT }, FocusPolicy::Keep);

        assert_eq!(out, Err(TargetDockError::Moved), "{prewarmed} {closes}");
        let opened: Vec<String> = w
            .host
            .opened_at
            .lock()
            .unwrap()
            .iter()
            .map(|(label, _, _)| label.clone())
            .collect();
        assert_eq!(opened.len(), 1);
        assert!(!w.window_exists(&opened[0]), "the opened window does not enter the model");
        let journal = w.journal.take();
        match &spare {
            Some(spare) => {
                assert!(journal.contains(&format!("release:{spare}")), "{journal:?}");
                assert!(!w.claim_outstanding());
            }
            None => assert_eq!(
                &*w.host.closed.lock().unwrap(),
                &opened,
                "reaps the window just opened"
            ),
        }
        let want = if closes {
            vec![main]
        } else {
            vec![main, first]
        };
        assert_eq!(w.tabs_of(MAIN_WINDOW_LABEL), want, "nothing was moved");
        assert_eq!(w.resyncs(), resyncs);
        assert_eq!(w.ev.take_order(), Vec::<&str>::new());
        assert!(w.host.placed.lock().unwrap().is_empty());
        assert_invariants(&w);
    }
}

#[test]
fn a_reused_detach_whose_agent_leaves_while_the_window_moves_answers_moved() {
    let w = World::new();
    let main = w.main_active();
    let main_slot = w.empty_slot(main);
    let (label, lone) = w.lone_popout();
    let lone_slot = w.empty_slot(lone);
    let agent = w.put_agent(lone, lone_slot);
    // While moving the window, that agent moves to a main tab — the tab of the reused window is no longer that agent's tab.
    *w.host.during_move.lock().unwrap() = Some(Box::new(move |state: &LayoutState| {
        let mut mgr = state.0.lock().unwrap();
        mgr.set_slot_content(lone, lone_slot, SlotContent::Empty)
            .unwrap();
        mgr.assign_agent(main, main_slot, agent.to_string())
            .unwrap();
    }));
    let resyncs = w.resyncs();
    w.forget();

    let out = w.dock_t(
        Target::TabOf(agent),
        TargetDest::NewWindow { at: NO_PLACE },
        FocusPolicy::Keep,
    );

    assert_eq!(out, Err(TargetDockError::Moved));
    assert_eq!(&*w.host.moves.lock().unwrap(), &[(label.clone(), NO_PLACE)]);
    assert!(w.host.opened_at.lock().unwrap().is_empty(), "no new window");
    assert_eq!(w.tabs_of(&label), [lone], "the reused window stays as is");
    assert_eq!(w.tabs_of(MAIN_WINDOW_LABEL), [main]);
    assert_eq!(w.resyncs(), resyncs);
    assert_eq!(w.ev.take_order(), Vec::<&str>::new());
    assert_invariants(&w);
}

#[test]
fn a_detach_by_agent_opens_a_window_for_its_tab_without_taking_focus() {
    let w = World::new();
    let main = w.main_active();
    let t = w.new_tab(MAIN_WINDOW_LABEL);
    let agent = w.put_agent(t, w.empty_slot(t));
    w.forget();

    let out = w
        .dock_t(
            Target::TabOf(agent),
            TargetDest::NewWindow { at: NO_PLACE },
            FocusPolicy::Keep,
        )
        .unwrap();

    assert!(out.changed && !out.reused);
    assert_eq!(out.tab, t);
    assert_eq!(w.owner(t), Some(out.window.clone()));
    assert_eq!(
        out.views,
        [t, main],
        "the new window's tab + the tab newly visible in main"
    );
    let opened = w.host.opened_at.lock().unwrap().clone();
    assert_eq!(
        opened,
        [(out.window.clone(), NO_PLACE, false)],
        "opens without activation"
    );
    assert_invariants(&w);
}

// ── target version: tab row position ──

#[test]
fn a_tab_bar_move_after_an_agents_tab_lands_right_behind_it() {
    let w = World::new();
    let s = w.main_active();
    let a = w.new_tab(MAIN_WINDOW_LABEL);
    let x = w.new_tab(MAIN_WINDOW_LABEL);
    let agent = w.put_agent(a, w.empty_slot(a));

    let out = w
        .dock_t(Target::Tab(s), after(agent), FocusPolicy::Keep)
        .unwrap();
    assert!(out.changed);
    assert_eq!(out.window, MAIN_WINDOW_LABEL);
    assert_eq!(w.tabs_of(MAIN_WINDOW_LABEL), [a, s, x]);

    w.dock_t(Target::Tab(x), after(agent), FocusPolicy::Keep)
        .unwrap();
    assert_eq!(w.tabs_of(MAIN_WINDOW_LABEL), [a, x, s]);

    let out = w
        .dock_t(Target::Tab(x), after(agent), FocusPolicy::Keep)
        .unwrap();
    assert!(!out.changed);

    let (label, ptabs) = w.popup_with(2);
    let there = w.put_agent(ptabs[0], w.empty_slot(ptabs[0]));
    let out = w
        .dock_t(Target::Tab(s), after(there), FocusPolicy::Keep)
        .unwrap();
    assert_eq!(out.window, label);
    assert_eq!(w.tabs_of(&label), [ptabs[0], s, ptabs[1]]);
    assert_eq!(w.tabs_of(MAIN_WINDOW_LABEL), [a, x]);
    assert_invariants(&w);
}

#[test]
fn a_tab_bar_move_after_its_own_or_an_ambiguous_tab_is_refused_unchanged() {
    let w = World::new();
    let s = w.main_active();
    let other = w.new_tab(MAIN_WINDOW_LABEL);
    let mine = w.put_agent(s, w.empty_slot(s));
    let spread = w.put_agent(other, w.empty_slot(other));
    let (_, ptabs) = w.popup_with(1);
    w.put(ptabs[0], w.empty_slot(ptabs[0]), spread);
    let version = w.version();
    w.forget();

    assert_eq!(
        w.dock_t(Target::Tab(s), after(mine), FocusPolicy::Keep),
        Err(TargetDockError::Dock(DockError::SelfTarget)),
        "insert after itself"
    );
    let err = w
        .dock_t(Target::Tab(s), after(spread), FocusPolicy::Keep)
        .unwrap_err();
    assert!(
        matches!(
            err,
            TargetDockError::Dest(LocateError::AgentAmbiguous { .. })
        ),
        "{err:?}"
    );
    assert_eq!(
        w.dock(
            s,
            DockDest::TabBar {
                window: MAIN_WINDOW_LABEL.into(),
                index: None,
                after: Some(ptabs[0]),
            },
            FocusPolicy::Keep,
        ),
        Err(DockError::TabNotFound(ptabs[0])),
        "id edition — a reference that is not that window's tab"
    );
    assert_eq!(w.version(), version);
    assert_eq!(w.ev.take_order(), Vec::<&str>::new());
    assert_invariants(&w);
}

#[test]
fn a_target_dock_that_changes_nothing_says_so() {
    let w = World::new();
    let (label, tabs) = w.popup_with(2);
    let version = w.version();

    let out = w
        .dock_t(
            Target::Tab(tabs[1]),
            tab_bar(None, Some(1)),
            FocusPolicy::Keep,
        )
        .unwrap();
    assert!(!out.changed, "the same window, the same place");
    assert_eq!(out.window, label);
    assert_eq!(out.views, [tabs[1]]);
    assert_eq!(w.version(), version);

    let (lone_label, lone) = w.lone_popout();
    let version = w.version();
    let out = w
        .dock_t(
            Target::Tab(lone),
            TargetDest::NewWindow { at: NO_PLACE },
            FocusPolicy::Keep,
        )
        .unwrap();
    assert!(out.reused && !out.changed, "a pop-out that is already alone");
    assert_eq!(out.window, lone_label);
    assert_eq!(w.version(), version);

    let out = w
        .dock_t(
            Target::Tab(tabs[1]),
            tab_bar(None, Some(0)),
            FocusPolicy::Keep,
        )
        .unwrap();
    assert!(out.changed);
    assert_eq!(w.tabs_of(&label), [tabs[1], tabs[0]]);
}

// ── target version: attaching beside an area · default share ──

#[test]
fn a_seam_dock_without_a_share_takes_the_mouse_formula_or_half_when_unmeasured() {
    // Splits the target tab left/right first so the area frame (left half) and the whole canvas give different shares — tells whether a beside-area drop counted by the area frame and a whole-tab drop by the canvas. Original size = source window canvas (width 200).
    let slot_frame = PxRect {
        x0: 0,
        y0: 0,
        x1: 500,
        y1: 600,
    };
    let canvas = PxRect {
        x0: 0,
        y0: 0,
        x1: 1000,
        y1: 600,
    };
    let original = SizeCss { w: 200.0, h: 150.0 };
    // (measured?, whole tab, share)
    for (measured, whole, want) in [
        (true, false, 0.4),
        (true, true, 0.2),
        (false, false, target::UNMEASURED_SHARE),
        (false, true, target::UNMEASURED_SHARE),
    ] {
        let w = World::new();
        let target_tab = w.main_active();
        let beside = w.empty_slot(target_tab);
        w.split(target_tab, beside, SplitDir::LeftRight);
        let host_agent = w.put_agent(target_tab, beside);
        let original_split = apply::list_splits(&w.state, target_tab).unwrap()[0].id;
        let (label, tabs) = w.popup_with(2);
        let source = tabs[1];
        if measured {
            apply::report_window_canvas(&w.state, MAIN_WINDOW_LABEL, 1000, 600).unwrap();
            apply::report_window_canvas(&w.state, &label, 200, 150).unwrap();
        }

        let out = w
            .dock_t(
                Target::Tab(source),
                TargetDest::Seam {
                    beside: Target::SlotOf(host_agent),
                    whole,
                    side: DockSide::Left,
                    share: None,
                },
                FocusPolicy::Keep,
            )
            .unwrap();

        assert_eq!(out.tab, target_tab, "resulting tab = the surviving target tab");
        assert_eq!(w.owner(source), None);
        let added: Vec<f64> = apply::list_splits(&w.state, target_tab)
            .unwrap()
            .into_iter()
            .filter(|s| s.id != original_split)
            .map(|s| s.ratio)
            .collect();
        assert_eq!(added, [want], "{measured} {whole}");
        if measured {
            let frame = if whole { canvas } else { slot_frame };
            assert_eq!(attach_share(original, DockSide::Left, frame), want);
        }
        assert_invariants(&w);
    }
}

#[test]
fn a_seam_dock_beside_a_tab_needs_whole_and_beside_its_own_slot_is_self() {
    let w = World::new();
    let target_tab = w.main_active();
    let (_, tabs) = w.popup_with(2);
    let source_slot = w.empty_slot(tabs[1]);
    let version = w.version();
    let seam = |beside, whole| TargetDest::Seam {
        beside,
        whole,
        side: DockSide::Right,
        share: Some(DOCK_NEW_SHARE),
    };

    assert_eq!(
        w.dock_t(
            Target::Tab(tabs[1]),
            seam(Target::Tab(target_tab), false),
            FocusPolicy::Keep
        ),
        Err(TargetDockError::NotASlot)
    );
    assert_eq!(
        w.dock_t(
            Target::Tab(tabs[1]),
            seam(
                Target::Slot {
                    view: None,
                    slot: source_slot
                },
                false
            ),
            FocusPolicy::Keep
        ),
        Err(TargetDockError::Dock(DockError::SelfTarget))
    );
    assert_eq!(w.version(), version);
    w.dock_t(
        Target::Tab(tabs[1]),
        seam(Target::Tab(target_tab), true),
        FocusPolicy::Keep,
    )
    .unwrap();
    assert!(w.slots(target_tab).contains(&source_slot));
    assert_invariants(&w);
}

// ── target version: area · tab writes ──

#[test]
fn slot_writes_by_target_resolve_inside_their_own_lock() {
    let w = World::new();
    let main = w.main_active();
    let left = w.empty_slot(main);
    let agent = w.put_agent(main, left);
    w.forget();

    let split = apply::split_slot_at(
        &w.state,
        &w.subs,
        &w.ev,
        &Target::SlotOf(agent),
        SplitDir::LeftRight,
    )
    .unwrap();
    let (found, right) = (split.at, split.out);
    assert_eq!(found, at(MAIN_WINDOW_LABEL, main, Some(left)));
    assert_eq!(w.slots(main), [left, right]);
    assert_notified(&w, "split_slot_at");

    apply::focus_slot_at(
        &w.state,
        &w.ev,
        &Target::At {
            token: SpatialToken::TopLeft,
            window: None,
        },
    )
    .unwrap();
    assert_eq!(w.snapshot(main).focused_slot_id, Some(left));
    assert_notified(&w, "focus_slot_at");

    let right_only = Target::Slot {
        view: None,
        slot: right,
    };
    apply::set_slot_content_at(
        &w.state,
        &w.subs,
        &w.ev,
        &right_only,
        SlotContent::AgentList,
    )
    .unwrap();
    assert_eq!(w.content(main, right), SlotContent::AgentList);
    apply::set_usage_slot_at(&w.state, &w.subs, &w.ev, &right_only, Some(false), None).unwrap();
    assert_eq!(
        w.content(main, right),
        SlotContent::Usage {
            show_claude: false,
            show_codex: true
        }
    );
    let other = Uuid::new_v4();
    apply::assign_agent_at(
        &w.state,
        &w.subs,
        &w.ev,
        &Target::At {
            token: SpatialToken::TopRight,
            window: None,
        },
        other.to_string(),
    )
    .unwrap();
    assert_eq!(
        w.content(main, right),
        SlotContent::Agent {
            agent_id: other.to_string()
        }
    );

    let version = w.version();
    let resyncs = w.resyncs();
    w.forget();
    assert_eq!(
        apply::close_slot_at(&w.state, &w.subs, &w.ev, &Target::TabOf(agent)),
        Err(TargetError::NotASlot),
        "a target that points at a tab for an area write"
    );
    let ghost = Uuid::new_v4();
    assert!(matches!(
        apply::close_slot_at(&w.state, &w.subs, &w.ev, &Target::SlotOf(ghost)),
        Err(TargetError::Locate(LocateError::AgentNotShown { .. }))
    ));
    assert_eq!(w.version(), version);
    assert_eq!(w.resyncs(), resyncs);
    assert_eq!(w.ev.take_order(), Vec::<&str>::new());

    let closed = apply::close_slot_at(&w.state, &w.subs, &w.ev, &Target::SlotOf(other)).unwrap();
    assert_eq!(closed.at.slot, Some(right));
    assert_eq!(w.slots(main), [left]);
    assert_invariants(&w);
}

#[test]
fn tab_writes_by_target_take_the_window_from_the_tab() {
    let w = World::new();
    let main = w.main_active();
    let t = w.new_tab(MAIN_WINDOW_LABEL);
    let agent = w.put_agent(main, w.empty_slot(main));
    let label = w.popup();
    w.forget();

    let found =
        apply::switch_tab_at(&w.state, &w.subs, &w.ev, None, &Target::TabOf(agent)).unwrap();
    assert_eq!(found, at(MAIN_WINDOW_LABEL, main, None));
    assert_eq!(w.active_of(MAIN_WINDOW_LABEL), main);
    assert_notified(&w, "switch_tab_at");
    assert!(
        matches!(
            apply::switch_tab_at(
                &w.state,
                &w.subs,
                &w.ev,
                Some(label.as_str()),
                &Target::TabOf(agent)
            ),
            Err(TargetError::Apply(_))
        ),
        "if a window is given it must be a tab of that window"
    );

    apply::rename_tab_at(&w.state, &w.ev, &Target::TabOf(agent), "work".into()).unwrap();
    let names = apply::list_tabs(&w.state, MAIN_WINDOW_LABEL).unwrap();
    assert_eq!(names.tabs[0].name, "work");

    let closed = apply::close_tab_at(
        &w.state,
        &w.subs,
        &w.ev,
        &w.host,
        None,
        &Target::TabOf(agent),
    )
    .unwrap();
    assert_eq!(closed.at.view, main);
    assert_eq!(closed.out, Some(t), "the tab now visible in that window");
    assert_eq!(w.tabs_of(MAIN_WINDOW_LABEL), [t]);

    let only = w.active_of(&label);
    let closed =
        apply::close_tab_at(&w.state, &w.subs, &w.ev, &w.host, None, &Target::Tab(only)).unwrap();
    assert_eq!(closed.out, None, "if it is the last tab the window closes with it");
    assert_eq!(&*w.host.closed.lock().unwrap(), &[label.clone()]);
    assert_invariants(&w);
}

#[test]
fn a_slot_popout_by_agent_moves_that_slot_and_a_miss_gives_the_prewarmed_window_back() {
    let w = World::new();
    let main = w.main_active();
    let left = w.empty_slot(main);
    let right = w.split(main, left, SplitDir::LeftRight);
    let agent = w.put_agent(main, right);

    let popped = apply::move_slot_to_window_at(
        &w.state,
        &w.subs,
        &w.ev,
        &w.host,
        &w.labels,
        &Target::SlotOf(agent),
        None,
    )
    .unwrap();
    let (found, moved) = (popped.at, popped.out);
    assert_eq!(found, at(MAIN_WINDOW_LABEL, main, Some(right)));
    assert_eq!(w.slots(main), [left]);
    assert_eq!(w.owner(moved.tab), Some(moved.window.clone()));
    assert_eq!(
        w.content(moved.tab, w.first_slot(moved.tab)),
        SlotContent::Agent {
            agent_id: agent.to_string()
        }
    );

    let spare = w.prewarm();
    let opened = w.host.opened.lock().unwrap().len();
    let err = apply::move_slot_to_window_at(
        &w.state,
        &w.subs,
        &w.ev,
        &w.host,
        &w.labels,
        &Target::SlotOf(Uuid::new_v4()),
        None,
    )
    .unwrap_err();
    assert!(matches!(
        err,
        TargetError::Locate(LocateError::AgentNotShown { .. })
    ));
    assert_eq!(
        w.host.opened.lock().unwrap().len(),
        opened,
        "does not open a window"
    );
    assert!(!w.claim_outstanding());
    assert_eq!(
        w.host.spare.lock().unwrap().as_deref(),
        Some(spare.as_str())
    );
    assert_invariants(&w);
}

#[tokio::test]
async fn spawn_into_at_places_the_agent_in_the_slot_the_word_picks() {
    let w = World::new();
    let main = w.main_active();
    let left = w.empty_slot(main);
    let right = w.split(main, left, SplitDir::LeftRight);
    let agent = Uuid::new_v4();
    let spawner = Spawner::ok(&w.state, agent);

    let placed = apply::spawn_into_at(
        &w.state,
        &w.subs,
        &w.ev,
        &spawner,
        MAIN_WINDOW_LABEL,
        SpatialToken::TopRight,
        None,
        "C:/work".into(),
    )
    .await
    .unwrap();
    let (id, found) = (placed.out, placed.at);
    assert_eq!(id, agent.to_string());
    assert_eq!(found, at(MAIN_WINDOW_LABEL, main, Some(right)));
    assert_eq!(
        w.content(main, right),
        SlotContent::Agent {
            agent_id: agent.to_string()
        }
    );

    // Filters before the spawn — occupied area · a word that picks no area (the right of focus = right) · a nonexistent window.
    for (token, window) in [
        (SpatialToken::TopRight, MAIN_WINDOW_LABEL),
        (SpatialToken::Right, MAIN_WINDOW_LABEL),
        (SpatialToken::TopLeft, "no-such"),
    ] {
        let never = Spawner::ok(&w.state, Uuid::new_v4());
        let version = w.version();
        let err = apply::spawn_into_at(
            &w.state,
            &w.subs,
            &w.ev,
            &never,
            window,
            token,
            None,
            "C:/work".into(),
        )
        .await
        .unwrap_err();
        assert_eq!(never.calls(), 0, "{token:?} {window}: does not spawn");
        assert!(matches!(err, apply::SpawnAtError::Unplaced(_)), "{err}");
        assert!(err.to_string().contains("Not spawned"), "{err}");
        assert_eq!(w.version(), version);
    }

    // That area was filled during the spawn — it resolves again under the layout lock and rejects, leaving the live id.
    let raced = Spawner::ok(&w.state, Uuid::new_v4());
    let filler = Uuid::new_v4();
    *raced.during.lock().unwrap() = Some(Box::new(move |state: &LayoutState| {
        state
            .0
            .lock()
            .unwrap()
            .assign_agent(main, left, filler.to_string())
            .unwrap();
    }));
    let call = apply::spawn_into_at(
        &w.state,
        &w.subs,
        &w.ev,
        &raced,
        MAIN_WINDOW_LABEL,
        SpatialToken::TopLeft,
        None,
        "C:/work".into(),
    );
    assert_send(&call);
    let err = call.await.unwrap_err();
    assert_eq!(raced.calls(), 1, "the spawn happened");
    assert!(
        matches!(err, apply::SpawnAtError::Failed(_)) && err.to_string().contains("is alive"),
        "an occupied area — leaves the living id: {err}"
    );
    assert_eq!(
        w.content(main, left),
        SlotContent::Agent {
            agent_id: filler.to_string()
        },
        "does not overwrite the filled area"
    );
}

// A future the handler waits for across threads — a lock guard held across an await stops compilation here.
fn assert_send<T: Send>(_: &T) {}

// ── target version: layout rows ──

type RowShape = (
    String,
    u32,
    u32,
    bool,
    Uuid,
    SlotKind,
    Option<String>,
    bool,
    [f64; 4],
);

fn row_shape(r: &LayoutRow) -> RowShape {
    (
        r.window.clone(),
        r.window_number,
        r.tab_index,
        r.tab_active,
        r.slot,
        r.content,
        r.agent.clone(),
        r.focused,
        [r.x, r.y, r.w, r.h],
    )
}

#[test]
fn layout_rows_list_every_slot_in_window_tab_tree_order_with_normalized_boxes() {
    let w = World::new();
    let main = w.main_active();
    let l = w.empty_slot(main);
    let r = w.split(main, l, SplitDir::LeftRight); // focus = r
    let second = w.new_tab(MAIN_WINDOW_LABEL); // empty tab · visible tab
    let s = w.empty_slot(second);
    let (label, ptabs) = w.popup_with(1);
    let top = w.empty_slot(ptabs[0]);
    let bottom = w.split(ptabs[0], top, SplitDir::TopBottom); // focus = bottom
    let agent = w.put_agent(main, l);
    w.put(ptabs[0], bottom, agent); // the same agent in two windows
    w.name(agent, "alpha");
    let main_label = MAIN_WINDOW_LABEL.to_string();
    let alpha = Some("alpha".to_string());

    let rows = apply::layout_rows(&w.state, &RowFilter::default()).unwrap();

    let want: Vec<RowShape> = vec![
        (
            main_label.clone(),
            1,
            0,
            false,
            l,
            SlotKind::Agent,
            alpha.clone(),
            false,
            [0.0, 0.0, 0.5, 1.0],
        ),
        (
            main_label.clone(),
            1,
            0,
            false,
            r,
            SlotKind::Empty,
            None,
            true,
            [0.5, 0.0, 0.5, 1.0],
        ),
        (
            main_label.clone(),
            1,
            1,
            true,
            s,
            SlotKind::Empty,
            None,
            true,
            [0.0, 0.0, 1.0, 1.0],
        ),
        (
            label.clone(),
            2,
            0,
            true,
            top,
            SlotKind::Empty,
            None,
            false,
            [0.0, 0.0, 1.0, 0.5],
        ),
        (
            label.clone(),
            2,
            0,
            true,
            bottom,
            SlotKind::Agent,
            alpha.clone(),
            true,
            [0.0, 0.5, 1.0, 0.5],
        ),
    ];
    assert_eq!(
        rows.iter().map(row_shape).collect::<Vec<_>>(),
        want,
        "window number → tab row → tree preorder · left-right and top-bottom are told apart by coordinates"
    );
    for row in &rows {
        let listed = apply::list_tabs(&w.state, &row.window).unwrap();
        let meta = &listed.tabs[row.tab_index as usize];
        assert_eq!(row.tab, meta.id);
        assert_eq!(row.tab_name, meta.name, "tab name = the visible name of the tab row");
        assert_eq!(row.window_id, listed.window_id);
    }
    assert_eq!(rows[0].agent_id, Some(agent.to_string()));
    assert_eq!(rows[1].agent_id, None);

    let only_popout = apply::layout_rows(
        &w.state,
        &RowFilter {
            window: Some(label.clone()),
            agent: None,
        },
    )
    .unwrap();
    assert_eq!(
        only_popout.iter().map(|r| r.slot).collect::<Vec<_>>(),
        [top, bottom]
    );
    let of_agent = apply::layout_rows(
        &w.state,
        &RowFilter {
            window: None,
            agent: Some(agent),
        },
    )
    .unwrap();
    assert_eq!(
        of_agent.iter().map(|r| r.slot).collect::<Vec<_>>(),
        [l, bottom]
    );

    let picked = apply::layout_rows_of(&w.state, &[ptabs[0], main, Uuid::new_v4()]).unwrap();
    assert_eq!(
        picked.iter().map(|r| r.slot).collect::<Vec<_>>(),
        [l, r, top, bottom],
        "only the given tabs · order is the model's order"
    );
}

#[test]
fn a_dock_outcomes_views_give_the_changed_tabs_rows() {
    let w = World::new();
    let target_tab = w.main_active();
    let beside = w.empty_slot(target_tab);
    let (label, tabs) = w.popup_with(2);
    let moved_slot = w.empty_slot(tabs[1]);
    let left_behind = w.empty_slot(tabs[0]);

    let out = w
        .dock_t(
            Target::Tab(tabs[1]),
            TargetDest::Seam {
                beside: Target::Slot {
                    view: None,
                    slot: beside,
                },
                whole: false,
                side: DockSide::Bottom,
                share: Some(DOCK_NEW_SHARE),
            },
            FocusPolicy::Keep,
        )
        .unwrap();

    assert_eq!(
        out.views,
        [target_tab, tabs[0]],
        "target tab + the tab newly visible in the source window"
    );
    let rows = apply::layout_rows_of(&w.state, &out.views).unwrap();
    assert_eq!(
        rows.iter()
            .map(|r| (r.window.as_str(), r.slot))
            .collect::<Vec<_>>(),
        [
            (MAIN_WINDOW_LABEL, beside),
            (MAIN_WINDOW_LABEL, moved_slot),
            (label.as_str(), left_behind)
        ]
    );
    assert!(rows[1].y > rows[0].y, "attached below");
    assert_eq!(
        out.rows, rows,
        "the outcome's rows = those tabs' rows (when there is no interleaved write)"
    );
}

/// The outcome rows (`DockOutcome::rows`) carry only the target version — the new-window · reuse branches are also that tab's rows, and the id version (mouse) leaves them empty.
#[test]
fn a_target_dock_carries_the_rows_of_its_views_and_the_id_dock_does_not() {
    let w = World::new();
    let main = w.main_active();
    let (label, tabs) = w.popup_with(2);

    let out = w
        .dock_t(
            Target::Tab(tabs[1]),
            TargetDest::NewWindow { at: NO_PLACE },
            FocusPolicy::Keep,
        )
        .unwrap();
    assert!(!out.reused);
    assert_eq!(
        out.rows
            .iter()
            .map(|r| (r.window.clone(), r.tab))
            .collect::<Vec<_>>(),
        [(label, tabs[0]), (out.window.clone(), tabs[1])],
        "new-window branch — the moved tab + the row of the tab newly visible in the source window (window number order)"
    );

    let (lone_label, lone) = w.lone_popout();
    let out = w
        .dock_t(
            Target::Tab(lone),
            TargetDest::NewWindow { at: NO_PLACE },
            FocusPolicy::Keep,
        )
        .unwrap();
    assert!(out.reused);
    assert_eq!(
        out.rows
            .iter()
            .map(|r| (r.window.clone(), r.tab))
            .collect::<Vec<_>>(),
        [(lone_label, lone)],
        "reuse branch — that tab's row"
    );

    let out = w
        .dock(
            tabs[0],
            DockDest::TabBar {
                window: MAIN_WINDOW_LABEL.to_string(),
                index: None,
                after: Some(main),
            },
            FocusPolicy::Keep,
        )
        .unwrap();
    assert!(out.changed);
    assert!(out.rows.is_empty(), "the id edition carries no rows");
}

#[tokio::test]
async fn spawn_into_at_does_not_place_elsewhere_when_the_word_picks_another_slot_after_the_spawn() {
    // [a | b | c] · focus = b → `left` = a. If focus goes to c during the spawn, `left` = b (also an empty area) but it is not placed there.
    let w = World::new();
    let main = w.main_active();
    let a = w.empty_slot(main);
    let b = w.split(main, a, SplitDir::LeftRight);
    let c = w.split(main, b, SplitDir::LeftRight);
    apply::focus_slot(&w.state, &w.ev, main, b).unwrap();
    let spawner = Spawner::ok(&w.state, Uuid::new_v4());
    *spawner.during.lock().unwrap() = Some(Box::new(move |state: &LayoutState| {
        state.0.lock().unwrap().set_focused_slot(main, c).unwrap();
    }));

    let err = apply::spawn_into_at(
        &w.state,
        &w.subs,
        &w.ev,
        &spawner,
        MAIN_WINDOW_LABEL,
        SpatialToken::Left,
        None,
        "C:/work".into(),
    )
    .await
    .unwrap_err();

    assert_eq!(spawner.calls(), 1, "the spawn happened");
    assert!(
        matches!(err, apply::SpawnAtError::Failed(_)) && err.to_string().contains("is alive"),
        "{err}"
    );
    for slot in [a, b, c] {
        assert_eq!(w.content(main, slot), SlotContent::Empty, "{slot}");
    }
}

#[test]
fn a_reused_detach_whose_tab_left_and_whose_window_closed_answers_moved_by_target_and_ok_by_id() {
    // While moving the window, that tab moves to main and the popout that was alone becomes empty and closes — the tab is resolved as is but there is no reused window.
    for by_target in [true, false] {
        let w = World::new();
        let (label, lone) = w.lone_popout();
        *w.host.during_move.lock().unwrap() = Some(Box::new(move |state: &LayoutState| {
            let mut mgr = state.0.lock().unwrap();
            let docked = mgr
                .dock(
                    lone,
                    DockTo::TabBar {
                        window: MAIN_WINDOW_LABEL.into(),
                        index: None,
                        after: None,
                    },
                    FocusPolicy::Keep,
                    DockMove::default(),
                )
                .unwrap();
            assert!(
                docked.emptied.is_some(),
                "a lone pop-out becomes empty and leaves the model"
            );
        }));
        w.forget();

        if by_target {
            assert_eq!(
                w.dock_t(
                    Target::Tab(lone),
                    TargetDest::NewWindow { at: NO_PLACE },
                    FocusPolicy::Keep
                ),
                Err(TargetDockError::Moved),
                "target edition — does not name a nonexistent window in the outcome"
            );
        } else {
            let out = w
                .dock(
                    lone,
                    DockDest::NewWindow { at: NO_PLACE },
                    FocusPolicy::Keep,
                )
                .unwrap();
            assert!(out.reused, "the id edition stays as is — success + no identity");
            assert_eq!(out.window, label);
            assert_eq!(out.identity, None);
        }
        assert!(!w.window_exists(&label));
        assert_eq!(w.owner(lone).as_deref(), Some(MAIN_WINDOW_LABEL));
        assert!(
            w.host.placed.lock().unwrap().is_empty(),
            "a closed window is not recorded"
        );
        assert_invariants(&w);
    }
}
