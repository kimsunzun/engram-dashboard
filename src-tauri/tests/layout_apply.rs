//! Integration test of the layout apply service (`layout::apply`) — zero windows · daemon · Tauri (ADR-0012 seam isolation).
//!
//! ★Why this file is in `tests/` (an integration target)★: what it measures is **wiring that cannot be set up without a real socket · a real `AppHandle`**,
//! so an integration harness that cuts that place with ports is the right place (so the harness itself uses no windows · daemon · Tauri
//! at all). That is also the placement criterion for new assertions — **pure units go in `#[cfg(test)]` next to the module** (those run with
//! `--test lib_unit` · the decision that set up that target = ADR-0174 · status = `.claude/skill-bindings/qa.md` 「모듈별 명령」),
//! **wiring goes here.**
//! Run: `cargo test -p engram-dashboard --test layout_apply` — ★do not add `-- --test-threads=4`★
//! (this suite starts no child process at all. The canonical decision rule for that flag = `.claude/skill-bindings/qa.md` 「모듈별 명령」)
//!
//! CI also has a dedicated step that calls only this target (`.github/workflows/ci.yml`).
//!
//! Window label issuing uses the **real `PopupCounter`** (only a probe shell is wrapped around it), and subscription resync uses the **real `OutputRouter`**
//! — if imitated with fakes, one verifies the fake, not the label monotonicity · routing contract. The remaining three
//! ports (notification · OS window · spawn) are recording fakes.
//!
//! ## ★Lock position probe (the core of this harness)★
//! The invariant this refactor wants to keep is "which port is called **inside** the lock and which **outside**",
//! and counting calls alone does not catch it — moving the position leaves the count the same. So each fake port checks directly whether the lock is held with `try_lock` at
//! **the very place** it is called:
//! - `LayoutEvents`·`WindowHost`·`LabelSource`·`AgentSpawner` → not `WouldBlock` (must be **outside** the lock — a poisoned
//!   lock counts as outside).
//!   If `WindowHost::close` is moved inside the lock, it is a commit deadlock in production (`w.destroy()` → `Destroyed` → re-acquiring the same
//!   lock).
//! - `SubscriptionSync::resync` → `is_err()` (must be **inside** the lock). If moved outside, the F1/F2 stale-unsubscribe
//!   defect comes back.
//!
//! Determinism basis: std `Mutex::try_lock` gives **`WouldBlock` without blocking even if the same thread already holds it**
//! → it splits with zero threads · timing. ★But this decision presumes the test runs on a single thread★ (if another
//! thread holds the lock, `is_err()` can pass falsely — this file creates no threads).
//! The real Tauri adapter needs a window and cannot run here (that is the share of the GUI measurement).

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

// ── lock position probe ───────────────────────────────────────────────────────

// Shares the same `Arc<Mutex<ViewManager>>` and sees whether the lock is held at the time of the call (header §Lock position probe).
struct Probe(LayoutState);

impl Probe {
    fn new(state: &LayoutState) -> Self {
        Probe(state.clone())
    }

    // A poisoned lock is held by nobody — the lock poisoning test also sees port calls after the poisoning with this probe.
    fn assert_outside(&self, who: &str) {
        assert!(
            !matches!(self.0 .0.try_lock(), Err(TryLockError::WouldBlock)),
            "{who} 는 락을 드롭한 뒤에 불려야 한다(락 안이면 창 destroy·emit 이 교착·재진입한다)"
        );
    }

    fn assert_inside(&self, who: &str) {
        assert!(
            self.0 .0.try_lock().is_err(),
            "{who} 는 락 보유 중에 불려야 한다(밖이면 계산~발화 사이 재추가로 stale 해제 — F1/F2)"
        );
    }
}

// ── fake ports ────────────────────────────────────────────────────────────────

/// The order across the window port and the notification port — the two share one record (TRD S21-slot-docking §8-1 ⑬ per-place order). Window port =
/// `"open:<label>"` · `"open_at:<label>"` · `"close:<label>"` · `"place:<label>"` · the three prewarmed window entries `"claim:<label>"` (only when
/// handed over) · `"committed:<label>"` · `"release:<label>"`, notification = `"layout"` · `"tabs:<label>"`.
#[derive(Clone, Default)]
struct Journal(Arc<Mutex<Vec<String>>>);

impl Journal {
    fn note(&self, entry: String) {
        self.0.lock().unwrap().push(entry);
    }

    /// Returns the record so far while emptying it.
    fn take(&self) -> Vec<String> {
        std::mem::take(&mut *self.0.lock().unwrap())
    }
}

struct Recorder {
    probe: Probe,
    layout: Mutex<Vec<ViewSnapshot>>,
    tabs: Mutex<Vec<WindowTabsPayload>>,
    /// The order notifications came in — `"layout"` · `"tabs"`.
    order: Mutex<Vec<&'static str>>,
    journal: Journal,
    /// Once outside the lock at the first tab notification — sets up the state where another path changed the model after the new window entry entered the model · before the commit.
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

    /// Returns the order that has come so far while emptying it.
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

/// The interjection that `open` calls once outside the lock — sets up the state where another command changed the model during the window build (phase B).
/// The interjection of `move_window` (`during_move`) has the same shape.
type DuringOpen = Box<dyn FnOnce(&LayoutState) + Send>;

struct Host {
    probe: Probe,
    /// Both `open` and `open_at` fail.
    fail_open: bool,
    /// Labels of opened windows — both `open` and `open_at`.
    opened: Mutex<Vec<String>>,
    closed: Mutex<Vec<String>>,
    placed: Mutex<Vec<String>>,
    during_open: Mutex<Option<DuringOpen>>,
    /// The `(label, 자리, 활성)` (label, placement, activation) that `open_at` received.
    opened_at: Mutex<Vec<(String, WindowPlace, bool)>>,
    /// The `(label, 자리)` (label, placement) that `move_window` received.
    moves: Mutex<Vec<(String, WindowPlace)>>,
    /// The setter steps `move_window` actually called (in order) — failures are planted with `fail_steps`.
    steps: Mutex<Vec<MoveStep>>,
    fail_steps: Mutex<Vec<MoveStep>>,
    /// Once outside the lock before `move_window` calls the setters — sets up the state where another path changed the source while moving.
    during_move: Mutex<Option<DuringOpen>>,
    journal: Journal,
    /// Label of the hidden prewarmed window to hand over — empties when handed over and comes back when taken back (TRD S21-slot-docking §8-1 ⑬). The pool's state machine (build ·
    /// waiting · rebuild) is not imitated — only the call order and the claim ticket are measured.
    spare: Mutex<Option<String>>,
    /// The claim ticket currently handed over — commit · take-back must return a claim ticket equal to this exactly once.
    claimed: Mutex<Option<SpareClaim>>,
    /// Number of times `claim_spare` was called — it also counts the case that gave `None` for lack of a window to hand over.
    claims_asked: AtomicUsize,
    generation: AtomicU32,
    /// Label the test deleted from the model after it entered the model · before the commit — `record_placement` does not require a model entry for it.
    vanished: Mutex<Vec<String>>,
    /// If true, `close` does nothing (only leaves a record) — sets up 「모델에서만 빠지고 OS 창은 남은 창」 (a window removed only from the model while its OS window remains) (TRD
    /// S26-window-chrome §2-3 「시험 몫 도우미」).
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

    // The handed-over window came back with that claim ticket — blows up if the same claim ticket is received twice or a different claim ticket is received.
    fn take_claim(&self, claim: &SpareClaim, who: &str) {
        let held = self.claimed.lock().unwrap().take();
        assert_eq!(
            held.as_ref(),
            Some(claim),
            "{who} 는 맡긴 맡김표 그대로 한 번만 받아야 한다"
        );
    }

    // The window opening part — shared by `open` and `open_at`.
    fn opening(&self, kind: &str, label: &str) -> Result<(), String> {
        assert!(
            !self.probe.0 .0.lock().unwrap().windows.contains_key(label),
            "{kind}({label}) 은 창이 모델에 들기 전에 불려야 한다"
        );
        self.journal.note(format!("{kind}:{label}"));
        if let Some(hook) = self.during_open.lock().unwrap().take() {
            hook(&self.probe.0);
        }
        self.opened.lock().unwrap().push(label.to_string());
        if self.fail_open {
            // Port contract — if the handed-over prewarmed window could not be dropped, it withdraws that window and gives `Err`. The withdrawn handover is finished.
            let mut claimed = self.claimed.lock().unwrap();
            if claimed.as_ref().is_some_and(|c| c.label == label) {
                *claimed = None;
            }
            return Err("창 생성 실패(테스트)".to_string());
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
                error: format!("{step:?} 세터 실패(테스트)"),
            });
        }
        Ok(())
    }
}

impl WindowHost for Host {
    // ★It must be before entering the model★ — all three places that open a new window open the window first and put it in the model (TRD S21-slot-docking §3-9
    // 「자리별 차례」 · ADR-0299).
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

    // It is accepted even if that label is not in the model — closing a window that dropped out before commit is the implementation's job (port docs).
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

    // Contract = port docs — position → size, stops at the first failure. An empty field skips that step.
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

    // ★It must be after entering the model★ — the production implementation writes the placement into that label's model entry, so if called first, the first placement is discarded.
    fn record_placement(&self, label: &str) {
        self.probe.assert_outside("WindowHost::record_placement");
        assert!(
            self.probe.0 .0.lock().unwrap().windows.contains_key(label)
                || self.vanished.lock().unwrap().iter().any(|l| l == label),
            "record_placement({label}) 는 창이 모델에 든 뒤에 불려야 한다"
        );
        self.journal.note(format!("place:{label}"));
        self.placed.lock().unwrap().push(label.to_string());
    }

    // ★Answers true with main as a special case★ — it is a static config window so this fake has never opened it, but in the real app it is always up
    // (returning a popup slot to main is the real usage path). The rest are only those this fake opened and did not close.
    fn is_open(&self, label: &str) -> bool {
        self.probe.assert_outside("WindowHost::is_open");
        label == MAIN_WINDOW_LABEL
            || (self.opened.lock().unwrap().iter().any(|l| l == label)
                && !self.closed.lock().unwrap().iter().any(|l| l == label))
    }
}

// ★It **wraps** the real issuer (does not replace it)★: the inner real one keeps bearing label monotonicity, and this shell only adds a view of the call
// **position**. Today's implementation is pure, so it does not blow up even if called inside the lock, and what pins that place is this probe alone
// — the day it changes to an implementation that touches state, it deadlocks with `Destroyed → cleanup_popup_window → state.0.lock()`.
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
    /// ★Did the chosen backend reach the port★ — the spawn call count alone cannot tell whether that value flowed or not.
    backends: Mutex<Vec<Option<AgentBackendKind>>>,
    /// Once during the spawn (outside the lock) — sets up the state where another write changed the layout while spawning.
    during: Mutex<Option<DuringOpen>>,
}

impl Spawner {
    fn ok(state: &LayoutState, agent: Uuid) -> Self {
        Spawner::new(state, Ok(agent.to_string()))
    }

    fn failing(state: &LayoutState) -> Self {
        Spawner::new(state, Err("spawn 실패: 데몬 거절".to_string()))
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
            // If one awaits inside the lock, the command future loses Send and does not even compile, but whether that wall stands is
            // also checked here (the same probe as the sibling port — with one wall it silently collapses).
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
    // Real issuer (only a probe wrapped around it) — the prefix (`slot-popup-`) pairs with the capabilities/popup.json glob and
    // monotonicity is the contract that prevents a closed-label re-build error.
    labels: Labels,
    /// The order record shared by `ev` and `host`.
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
            .expect("main 창은 항상 있다")
            .active
    }

    fn snapshot(&self, view: Uuid) -> ViewSnapshot {
        apply::get_view(&self.state, view).expect("view 존재")
    }

    fn slots(&self, view: Uuid) -> Vec<Uuid> {
        self.snapshot(view)
            .slot_spatial
            .iter()
            .map(|s| s.slot_id)
            .collect()
    }

    fn empty_slot(&self, view: Uuid) -> Uuid {
        tree::first_empty_slot_id(&self.snapshot(view).layout).expect("빈 슬롯 존재")
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
            .expect("탭 알림")
    }

    fn unsubscribed(&self) -> Vec<Uuid> {
        self.subs.unsubscribed.lock().unwrap().clone()
    }

    // One popup window (1 empty tab) — the premise of the WindowClosed/close_window path.
    fn popup(&self) -> String {
        apply::create_window(&self.state, &self.subs, &self.ev, &self.host, &self.labels)
            .expect("창 생성")
            .window
    }

    // One hidden prewarmed window for the window port to hand over — as in production, it takes a label from the issuer when built. Returns = that label.
    fn prewarm(&self) -> String {
        let label = LabelSource::next_label(&self.labels.inner);
        *self.host.spare.lock().unwrap() = Some(label.clone());
        label
    }

    // Does the window port still hold that handover (none of commit · take-back · withdrawal has happened).
    fn claim_outstanding(&self) -> bool {
        self.host.claimed.lock().unwrap().is_some()
    }

    fn claims_asked(&self) -> usize {
        self.host.claims_asked.load(Ordering::Relaxed)
    }

    // A path that does not open a new window does not ask for a prewarmed window either — an unfinished handover blocks the one-field pool (TRD S21-slot-docking
    //   §3-9 I27). `asked` = the number of asks before calling that path.
    fn assert_spare_not_asked(&self, spare: &str, asked: usize) {
        assert_eq!(
            self.claims_asked(),
            asked,
            "새 창이 아닌 길이 예열 창을 물었다"
        );
        assert_eq!(
            self.host.spare.lock().unwrap().as_deref(),
            Some(spare),
            "예열 창은 맡겨지지 않은 채 남는다"
        );
    }
}

// Poisons the lock right after opening — imitates another write that panicked while holding it (same thread · released by unwinding).
fn poison(state: &LayoutState) {
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _guard = state.0.lock().unwrap();
        panic!("락 오염(테스트)");
    }));
    assert!(state.0.is_poisoned());
}

// ── verifying the probe itself ────────────────────────────────────────────────

// ★Evidence that the probe really bites in both directions★. The 「밖」 (outside) probe was tried by moving the service code inside the lock and watching it fail
// (measured on WindowHost · LayoutEvents), but for the 「안」 (inside) probe **a compilable counterexample cannot be made** — `resync` takes
// `&ViewManager`, so there is no way to call it without a guard (the type already guards that half). So
// we directly prove here that the probe is not a dead assertion: what is left for this probe is to catch the moment the port shape changes
// (e.g. if it came to take an owned snapshot, it could be called without a guard).
#[test]
fn probe_fires_on_both_sides_of_the_lock() {
    use std::panic::{catch_unwind, AssertUnwindSafe};

    let state = LayoutState::new();
    let probe = Probe::new(&state);

    probe.assert_outside("락 미보유");
    assert!(
        catch_unwind(AssertUnwindSafe(|| probe.assert_inside("거짓 주장"))).is_err(),
        "락 미보유인데 「안」이라 주장하면 프로브가 터져야 한다"
    );

    let guard = state.0.lock().unwrap();
    probe.assert_inside("락 보유");
    assert!(
        catch_unwind(AssertUnwindSafe(|| probe.assert_outside("거짓 주장"))).is_err(),
        "락 보유 중인데 「밖」이라 주장하면 프로브가 터져야 한다"
    );
    drop(guard);
}

// The docking tests rely on this fake's step record · failure planting, so we first pin down that the fake imitates the port contract (moving = stops at the first failure · an empty field
// is skipped).
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
        "첫 실패 뒤 크기 걸음은 안 불린다"
    );
    let size_only = WindowPlace {
        pos: None,
        size: Some((300.0, 200.0)),
    };
    *w.host.during_move.lock().unwrap() = Some(Box::new(|state: &LayoutState| {
        assert!(state.0.try_lock().is_ok(), "끼어들기는 락 밖");
    }));
    w.host.move_window(&label, &size_only).unwrap();
    assert_eq!(
        w.host.steps()[1..],
        [MoveStep::Size],
        "빈 칸은 그 걸음을 건너뛴다"
    );
    assert_eq!(w.host.moves.lock().unwrap().len(), 2);

    w.host.open_at("slot-popup-99", &at, false).unwrap();
    assert!(w.host.is_open("slot-popup-99"), "open_at 도 연 창이다");
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
    assert_eq!(tabs.active, id, "새 탭이 활성");
    assert_eq!(tabs.tabs.len(), 2);
    assert_eq!(w.layout_events(), 1);
    assert_eq!(w.tab_events(), 1);
    assert_eq!(w.resyncs(), 1);
}

#[test]
fn create_tab_unknown_window_is_err_without_notify() {
    let w = World::new();
    let err = apply::create_tab(&w.state, &w.subs, &w.ev, "no-such", None).unwrap_err();
    assert!(err.contains("window 없음"), "err={err}");
    assert_eq!(w.layout_events(), 0);
    assert_eq!(w.tab_events(), 0);
    assert_eq!(w.resyncs(), 0, "변형 실패 = 재동기 없음");
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
        "첫 자리 기록 1회"
    );
    assert!(apply::list_windows(&w.state).unwrap().contains(&label));
    assert_eq!(
        apply::list_tabs(&w.state, &label).unwrap().tabs.len(),
        1,
        "빈 탭 1개"
    );
    // One tab list of a new window — the material to apply the OS title (TRD S26-window-chrome §2-4 ② · §4-4). The layout is pulled by the web at mount.
    assert_eq!(w.layout_events(), 0);
    assert_eq!(w.tab_events(), 1);
    let notified = w.last_tabs();
    assert_eq!(notified, apply::list_tabs(&w.state, &label).unwrap());
    assert_eq!(notified.title, "빈 칸 - Engram:2");
    assert!(notified.version > version, "새 판으로 알린다");
}

// TRD S21-slot-docking §8-1 ⑬ — the order of a fresh build: `open` → put into the model → tab notification → `record_placement`. That it opens before entering the model
// is measured by the fake `open` itself. The branch where putting in is rejected (tombstone) is measured by the shell unit test of `commands::popout` — the function that sets the tombstone
// is not reachable from this crate.
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
            "여는 동안 모델은 그 창을 모른다"
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
    assert_eq!(w.resyncs(), 1, "넣기 한 번");
}

// Port contract — if `open` is `Err`, there is no window up. The model was not touched, so there is nothing to roll back and no window to withdraw.
#[test]
fn create_window_open_failure_leaves_the_model_untouched() {
    let w = World::failing_host();
    let version = apply::list_tabs(&w.state, MAIN_WINDOW_LABEL)
        .unwrap()
        .version;

    let err = apply::create_window(&w.state, &w.subs, &w.ev, &w.host, &w.labels).unwrap_err();
    assert!(err.contains("창 생성 실패"), "err={err}");

    let label = w.host.opened.lock().unwrap()[0].clone();
    assert!(
        !apply::list_windows(&w.state).unwrap().contains(&label),
        "못 연 창은 모델에 들지 않는다 — 유령 창 금지"
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
        "닫기 · 자리 기록 없음"
    );
}

// ── create_window: prewarmed window (TRD S21-slot-docking §8-1 ⑬ · ADR-0299) ──

// Prewarm order `claim_spare` → `open` → put into the model → tab notification → `committed` → `record_placement`.
#[test]
fn create_window_opens_the_prewarmed_window_inserts_then_commits_before_the_placement() {
    let w = World::new();
    let spare = w.prewarm();

    let label = apply::create_window(&w.state, &w.subs, &w.ev, &w.host, &w.labels)
        .unwrap()
        .window;

    assert_eq!(label, spare, "맡긴 창의 label 로 연다");
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
    assert_eq!(next, popup_label(2), "맡길 창이 없으면 발급기의 새 label");
}

// If the handed-over window's `open` is `Err`, the window port has already withdrawn that window — no commit, no take-back, and the model is unchanged.
#[test]
fn create_window_whose_prewarmed_window_fails_to_open_neither_commits_nor_releases() {
    let w = World::failing_host();
    let spare = w.prewarm();

    let err = apply::create_window(&w.state, &w.subs, &w.ev, &w.host, &w.labels).unwrap_err();

    assert!(err.contains("창 생성 실패"), "err={err}");
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
        "닫기 · 확정 · 자리 기록 없음"
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

// ★Passes through the real `PopupCounter`★: the prefix pairs with the glob of capabilities/popup.json (with another label
// the Destroyed cleanup gate is skipped and routing · subscriptions · Channels leak), and monotonicity prevents a closed-label re-build error.
// There is a separate unit test of this contract in `popout.rs`, but this assertion is left as it is — ★**what is measured through the service is
// a different layer**★, so it is not a duplicate.
#[test]
fn create_window_issues_monotonic_popup_labels_from_real_counter() {
    let w = World::new();
    let a = w.popup();
    let b = w.popup();
    assert_eq!(a, popup_label(1));
    assert_eq!(b, popup_label(2));

    apply::close_window(&w.state, &w.subs, &w.host, &a).unwrap();
    let c = w.popup();
    assert_eq!(c, popup_label(3), "닫힌 label 을 되돌려 쓰지 않는다");
    assert!(apply::list_windows(&w.state).unwrap().contains(&c));
}

// ── window number (TRD S26-window-chrome §4-1 · §10-2) ──

fn number_of(w: &World, label: &str) -> u32 {
    apply::list_tabs(&w.state, label).unwrap().number
}

// The first tab notification of a new window carries the number · the title carrying that number · the version just written (TRD S26-window-chrome §10-2). `tab` = the visible name of that window's active
//   tab.
fn assert_new_window_announced(w: &World, label: &str, tab: &str) {
    let notified = w.notified_tabs_of(label);
    let number = number_of(w, label);
    assert_eq!(notified.number, number);
    assert_eq!(notified.title, format!("{tab} - Engram:{number}"));
    assert_eq!(notified.version, w.version(), "{label} — 새 창을 넣은 판");
}

// The identity in the new-window answer equals that window's tab list — persistent id · number (TRD S26-window-chrome §5-3 · §10-2).
fn assert_answer_names(w: &World, answer: &NewWindow) {
    let tabs = apply::list_tabs(&w.state, &answer.window).unwrap();
    assert_eq!(
        (answer.window_id.as_str(), answer.number),
        (tabs.window_id.as_str(), tabs.number),
        "{} — 답의 영속 id · 번호",
        answer.window
    );
}

// The three new-window places, whether a prewarmed window or a newly built window, get the smallest free number, the tab list payload carries the number · title · persistent id,
// and the answer carries that window's persistent id · number.
#[test]
fn every_new_window_path_numbers_the_window_and_the_tabs_payload_carries_it() {
    let w = World::new();
    let main = apply::list_tabs(&w.state, MAIN_WINDOW_LABEL).unwrap();
    assert_eq!((main.number, main.window_id.as_str()), (1, "main"));
    assert_eq!(main.title, "빈 칸 - Engram:1");

    let mut labels = Vec::new();
    for prewarmed in [true, false] {
        let spare = prewarmed.then(|| w.prewarm());
        let created = apply::create_window(&w.state, &w.subs, &w.ev, &w.host, &w.labels).unwrap();
        assert_new_window_announced(&w, &created.window, "빈 칸");
        assert_answer_names(&w, &created);
        if let Some(spare) = spare {
            assert_eq!(created.window, spare, "예열 창의 label 로 답한다");
        }
        labels.push(created.window);

        let spare = prewarmed.then(|| w.prewarm());
        let (view, slot, _) = w.view_with_filled_slot();
        let moved = apply::move_slot_to_window(
            &w.state, &w.subs, &w.ev, &w.host, &w.labels, view, slot, None,
        )
        .unwrap();
        assert_new_window_announced(&w, &moved.window, "에이전트");
        assert_answer_names(
            &w,
            &NewWindow {
                window: moved.window.clone(),
                window_id: moved.window_id.clone(),
                number: moved.number,
            },
        );
        if let Some(spare) = spare {
            assert_eq!(moved.window, spare, "예열 창의 label 로 답한다");
        }
        labels.push(moved.window);

        let spare = prewarmed.then(|| w.prewarm());
        let source = apply::create_tab(&w.state, &w.subs, &w.ev, MAIN_WINDOW_LABEL, None).unwrap();
        let out = w
            .dock(source, DockDest::NewWindow { at: AT }, FocusPolicy::Keep)
            .unwrap();
        assert_eq!(out.prewarmed, prewarmed);
        assert_new_window_announced(&w, &out.window, "빈 칸");
        let opened = out
            .identity
            .clone()
            .expect("새 창 길은 그 창의 신원을 싣는다");
        assert_eq!(opened.window, out.window);
        assert_answer_names(&w, &opened);
        if let Some(spare) = spare {
            assert_eq!(out.window, spare, "예열 창의 label 로 답한다");
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
    assert_eq!(ids.len(), labels.len(), "영속 id 는 창마다 다르다");
}

// While the OS window of a closed popout still exists (the fake `close` does not emit `Destroyed`), that number does not go to a new window,
// and once the number release of the `Destroyed` part frees it, the next new window gets it (AC20 ②). What can be called from outside this crate is only the
// `release_window_number` of the branch that cannot call the cleanup — the cleanup branch (`drop_window_in_model`) is measured by the shell unit test.
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
    assert_eq!(number_of(&w, &c), 4, "닫히는 두 창의 번호를 건너뛴다");

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
    assert_eq!(w.last_tabs().active, first, "탭 알림이 활성 탭을 나른다");
}

#[test]
fn switch_tab_foreign_view_is_err() {
    let w = World::new();
    let label = w.popup();
    let foreign = apply::list_tabs(&w.state, &label).unwrap().active;
    let before = w.main_active();

    let err = apply::switch_tab(&w.state, &w.subs, &w.ev, MAIN_WINDOW_LABEL, foreign).unwrap_err();
    assert!(err.contains("view 없음"), "err={err}");
    assert_eq!(
        w.main_active(),
        before,
        "타 창 탭으로 전환 불가 — 활성 불변"
    );
}

// ── switch_tab_if (drag revert — TRD S21-slot-docking §4-6) ───────────────────

#[test]
fn switch_tab_if_switches_when_active_matches() {
    let w = World::new();
    let first = w.main_active();
    let dragged = apply::create_tab(&w.state, &w.subs, &w.ev, MAIN_WINDOW_LABEL, None).unwrap();
    assert_eq!(w.main_active(), dragged, "전제: 새 탭이 활성");
    w.ev.take_order();
    let resyncs = w.resyncs();

    let switched =
        apply::switch_tab_if(&w.state, &w.subs, &w.ev, MAIN_WINDOW_LABEL, first, dragged).unwrap();
    assert!(switched);
    assert_eq!(w.main_active(), first);
    assert_eq!(w.last_tabs().active, first, "탭 알림이 활성 탭을 나른다");
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
        "전제: 끄는 탭이 아닌 다른 탭이 활성"
    );
    w.ev.take_order();
    let (resyncs, layouts, tabs) = (w.resyncs(), w.layout_events(), w.tab_events());
    let version = apply::list_tabs(&w.state, MAIN_WINDOW_LABEL)
        .unwrap()
        .version;

    let switched =
        apply::switch_tab_if(&w.state, &w.subs, &w.ev, MAIN_WINDOW_LABEL, first, dragged).unwrap();
    assert!(!switched);
    assert_eq!(w.main_active(), chosen, "고른 탭이 그대로");
    assert_eq!(
        apply::list_tabs(&w.state, MAIN_WINDOW_LABEL)
            .unwrap()
            .version,
        version
    );
    assert_eq!(
        (w.resyncs(), w.layout_events(), w.tab_events()),
        (resyncs, layouts, tabs),
        "재동기 · 알림 없음"
    );
    assert!(w.ev.take_order().is_empty(), "알림도 없음");
}

#[test]
fn switch_tab_if_errors_like_switch_tab() {
    let w = World::new();
    let label = w.popup();
    let foreign = apply::list_tabs(&w.state, &label).unwrap().active;
    let active = w.main_active();

    let err = apply::switch_tab_if(&w.state, &w.subs, &w.ev, "no-such-window", active, active)
        .unwrap_err();
    assert!(err.contains("window 없음"), "err={err}");
    // A tab of another window is an error whether the condition matches or not.
    for expect in [active, foreign] {
        let err =
            apply::switch_tab_if(&w.state, &w.subs, &w.ev, MAIN_WINDOW_LABEL, foreign, expect)
                .unwrap_err();
        assert!(err.contains("view 없음"), "err={err}");
    }
    assert_eq!(w.main_active(), active, "활성 불변");
}

// ── close_tab ────────────────────────────────────────────────────────────────

#[test]
fn close_tab_keeps_main_alive_with_fresh_tab() {
    let w = World::new();
    let only = w.main_active();

    apply::close_tab(&w.state, &w.subs, &w.ev, &w.host, MAIN_WINDOW_LABEL, only).unwrap();

    let tabs = apply::list_tabs(&w.state, MAIN_WINDOW_LABEL).unwrap();
    assert_eq!(tabs.tabs.len(), 1, "main 은 최소 1탭(불변식 4)");
    assert_ne!(tabs.active, only, "새 빈 탭으로 교체");
    assert!(w.host.closed.lock().unwrap().is_empty(), "main OS 창 유지");
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
        "창이 사라진 뒤엔 그 창 탭 알림을 쏘지 않는다"
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
    assert!(err.contains("view 없음"), "err={err}");
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
        "어느 창에도 안 남은 agent 는 구독 해제"
    );
}

#[test]
fn close_window_main_is_rejected_and_os_window_untouched() {
    let w = World::new();
    let err = apply::close_window(&w.state, &w.subs, &w.host, MAIN_WINDOW_LABEL).unwrap_err();
    assert!(err.contains("메인 창은 닫을 수 없음"), "err={err}");
    assert!(
        w.host.closed.lock().unwrap().is_empty(),
        "거부 시 OS 창 close 0"
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
    assert!(w.host.is_open(&label), "OS 창은 남았다");
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
        "새 창이 그 번호를 다시 받았다 — 번호가 아니라 영속 id 로 고른다"
    );
    assert_eq!(
        window_ctl::close_if_empty(&w.state, &w.host, &window_id_of(&w, &newcomer)),
        Ok(CloseOutcome::Kept)
    );
    assert!(w.host.closed.lock().unwrap().is_empty());
    assert_eq!(
        window_ctl::close_if_empty(&w.state, &w.host, &left_id),
        Ok(CloseOutcome::Kept),
        "번호를 푼 창(그 OS 창의 `Destroyed` 가 지나갔다)은 닫힐 것이 없다"
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
    assert!(err.contains("slot 없음"), "err={err}");
    assert_eq!(w.layout_events(), 0);
}

#[test]
fn close_slot_removes_slot_and_unsubscribes_vanished_agent() {
    let w = World::new();
    let view = w.main_active();
    let slot = w.empty_slot(view);
    let agent = Uuid::new_v4();
    apply::assign_agent(&w.state, &w.subs, &w.ev, view, slot, agent.to_string()).unwrap();
    assert!(w.unsubscribed().is_empty(), "배정 직후엔 해제 없음");

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

    assert!(err.contains("view 없음"), "err={err}");
    // Looking only at the error string would let through "it failed but before that the resync · notification went out" — a rejection must be a no-op.
    assert_eq!(w.slots(view), slots_before, "부분변경 금지");
    assert_eq!(w.resyncs(), 0);
    assert_eq!(w.layout_events() + w.tab_events(), 0);
}

// ── focus_slot (ADR-0066 routing unchanged) ───────────────────────────────────

#[test]
fn focus_slot_notifies_without_routing_resync() {
    let w = World::new();
    let view = w.main_active();
    let target = w.empty_slot(view);
    // The split takes focus to the new area, so `target` is now outside focus — only then is the move measured.
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
        "포커스는 출력 라우팅을 안 바꾼다"
    );
}

#[test]
fn focus_slot_unknown_slot_is_err() {
    let w = World::new();
    let view = w.main_active();
    let before = w.snapshot(view).focused_slot_id;
    let err = apply::focus_slot(&w.state, &w.ev, view, Uuid::new_v4()).unwrap_err();
    assert!(err.contains("slot 없음"), "err={err}");
    assert_eq!(w.snapshot(view).focused_slot_id, before, "부분변경 금지");
}

// ── rename_tab (ADR-0057 tab notification only) ───────────────────────────────

#[test]
fn rename_tab_notifies_tabs_only() {
    let w = World::new();
    let view = w.main_active();

    apply::rename_tab(&w.state, &w.ev, view, "새 이름".to_string()).unwrap();

    assert_eq!(w.layout_events(), 0, "이름은 뷰 스냅샷에 없다");
    assert_eq!(w.tab_events(), 1);
    assert_eq!(w.resyncs(), 0);
    let named = w.last_tabs();
    assert_eq!(
        named.tabs.iter().find(|t| t.id == view).map(|t| &t.name),
        Some(&"새 이름".to_string())
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
    assert!(err.contains("view 없음"), "err={err}");
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

    assert_eq!(w.tab_events(), tabs + 1, "바뀐 창 하나만");
    let sent = w.last_tabs();
    assert_eq!(sent.label, MAIN_WINDOW_LABEL);
    assert_eq!(sent.tabs[0].name, "myproj");
    assert!(
        sent.version > version,
        "새 판 번호 — 같은 판이면 웹이 버린다"
    );
    assert_eq!(w.layout_events(), layouts, "이름은 뷰 스냅숏에 없다");
    assert_eq!(w.resyncs(), resyncs, "출력 라우팅은 그대로");
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
            (Uuid::new_v4().to_string(), "화면에 없음".into()),
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
        "판도 그대로"
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
    assert!(err.contains("slot 없음"), "err={err}");
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
    assert!(err.contains("view 없음"), "err={err}");
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
    assert!(err.contains("slot 없음"), "err={err}");
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
    assert!(err.contains("view 없음"), "err={err}");
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

    // ★The only gauge that measures rollback★: unlike the window list, `views` also counts temporary Views with no owner. If one counts windows,
    // on the failure path no window entry is made in the first place, so it is green even if rollback is deleted (that assertion measures nothing).
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
        "새 창은 모델에 든 뒤 첫 자리를 적는다"
    );
    assert!(!w.slots(view).contains(&slot), "원본 슬롯은 닫힌다(MOVE)");
    let landed = apply::list_tabs(&w.state, &moved.window).unwrap();
    assert_eq!(landed.active, moved.tab, "새 탭이 그 창의 활성 탭이다");
    let landed_slot = w.slots(moved.tab)[0];
    assert_eq!(
        tree::find_slot(&w.snapshot(moved.tab).layout, landed_slot),
        Some(&content),
        "콘텐츠가 그대로 새 탭에 실린다"
    );
}

/// ★If the notification does not go out, the screen draws the old layout★ — this command changes **two** windows (the source window that lost its content,
/// the target window that gained a tab), so there are also two tab bar notifications. If the target window's part is missed, the new window opens with an empty tab bar.
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
        "원본 뷰의 새 배치 1회"
    );
    let tab_payloads = w.ev.tabs.lock().unwrap();
    let sent: Vec<&str> = tab_payloads[tabs_before..]
        .iter()
        .map(|t| t.label.as_str())
        .collect();
    assert_eq!(
        sent,
        vec![MAIN_WINDOW_LABEL, moved.window.as_str()],
        "원본 창 → 대상 창 순으로 탭바가 갱신된다"
    );
}

// The answer of moving to an existing window carries that window's persistent id · number — for main, `main` · 1 (TRD S26-window-chrome §3-2).
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

// TRD S21-slot-docking §8-1 ⑬ — the order of a fresh build: stage A (temporary View) → `open` → stage C → notification → `record_placement`. The first
// placement record comes after the notification.
#[test]
fn move_slot_to_window_into_a_new_window_records_the_placement_after_the_notifications() {
    let w = World::new();
    let (view, slot, _) = w.view_with_filled_slot();
    let views_before = w.view_count();
    *w.host.during_open.lock().unwrap() = Some(Box::new(move |state: &LayoutState| {
        assert_eq!(
            state.0.lock().unwrap().views.len(),
            views_before + 1,
            "단계 A 의 임시 View 가 열기 전에 선다"
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

// Prewarm order `claim_spare` → stage A → `open` → stage C → notification → `committed` → `record_placement`.
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
            "단계 A 의 임시 View 가 열기 전에 선다"
        );
    }));
    w.journal.take();

    let moved = apply::move_slot_to_window(
        &w.state, &w.subs, &w.ev, &w.host, &w.labels, view, slot, None,
    )
    .unwrap();

    assert_eq!(moved.window, spare, "맡긴 창의 label 로 연다");
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
        ("에이전트", false),
        "새 창의 탭은 자동 이름 — 옮긴 칸의 이름이다(장부에 없는 에이전트)"
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

    assert!(err.contains("slot 없음"), "err={err}");
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
    assert_eq!(w.view_count(), views_before, "임시 View 를 거뒀다");
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

    assert!(err.contains("탭 삽입 실패"), "err={err}");
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

// Even if that window drops out of the model while notifying (the first tab notification) · before the commit, the commit passes the claim ticket on as it is — closing that window is the window port's job (TRD
//   §3-9 「자리별 차례」 · I27 · I31).
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
        "적용 서비스는 닫지 않는다"
    );
}

// Even if the lock is poisoned at stage C, no opened window remains — a newly built window is closed, and a handed-over window is taken back (the temporary View cannot be
//   withdrawn — same as `rollback_detached`).
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

/// ★The subscription is not cut while switching windows★: the agent disappears from the source slot but at the same moment
/// appears in the target tab. If the resync is computed before attach or runs only after close, 1→0 is caught and a live subscription dies.
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
        "옮겨간 창에서 계속 보이므로 해제할 것이 없다: {:?}",
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

    assert_eq!(moved.window, target, "새 창을 열지 않는다");
    assert_eq!(
        w.host.placed.lock().unwrap().len(),
        placed_before,
        "이미 있는 창의 자리는 사건이 적는다"
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
        apply::split_slot(&w.state, &w.subs, &w.ev, view, slot, SplitDir::LeftRight).expect("분할");

    let moved = apply::move_slot_to_window(
        &w.state, &w.subs, &w.ev, &w.host, &w.labels, view, slot, None,
    )
    .unwrap();

    assert_eq!(&*w.host.opened.lock().unwrap(), &[moved.window.clone()]);
    assert_eq!(
        w.slots(view),
        vec![sibling],
        "원본 슬롯은 닫히고 형제가 자리를 물려받는다"
    );
    let landed = apply::list_tabs(&w.state, &moved.window).unwrap();
    assert_eq!(landed.active, moved.tab);
    let landed_slots = w.slots(moved.tab);
    assert_eq!(landed_slots.len(), 1);
    assert_eq!(
        tree::find_slot(&w.snapshot(moved.tab).layout, landed_slots[0]),
        Some(&SlotContent::Empty),
        "새 탭은 빈 칸 하나다"
    );
    assert!(w.unsubscribed().is_empty(), "{:?}", w.unsubscribed());
}

// If the tab has only one area, a new empty area stands in the closed place (same rule as `close_slot`) — the source tab does not become empty.
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
    assert_ne!(left[0], slot, "옮긴 칸이 아니라 새로 선 칸이다");
    assert_eq!(
        tree::find_slot(&w.snapshot(view).layout, left[0]),
        Some(&SlotContent::Empty)
    );
}

// ★The rejection reason names the real cause★ — if a nonexistent area · nonexistent tab is called an 「빈 슬롯」 (empty slot), the caller fixes the wrong thing.
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
    assert!(err.contains("slot 없음"), "err={err}");
    assert!(!err.contains("빈 슬롯"), "err={err}");

    let err = apply::move_slot_to_window(
        &w.state, &w.subs, &w.ev, &w.host, &w.labels, ghost, ghost, None,
    )
    .unwrap_err();
    assert!(err.contains("view 없음"), "err={err}");

    assert!(w.host.opened.lock().unwrap().is_empty(), "창도 안 연다");
    assert_eq!(w.view_count(), views_before, "임시 View 를 남기지 않는다");
}

/// ★If the empty area being moved is filled during the window build, the source is not closed★ — even when the comparison base is `Empty`, 「옮긴 그대로일
/// 때만 닫는다」 (close only if it is as moved) holds. If closed, the content someone filled in the meantime stays on no screen.
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
        "그 사이 채운 콘텐츠를 지우지 않는다"
    );
    let landed_slot = w.slots(moved.tab)[0];
    assert_eq!(
        tree::find_slot(&w.snapshot(moved.tab).layout, landed_slot),
        Some(&SlotContent::Empty),
        "대상 탭은 옮긴 그대로(빈 칸) 남는다"
    );
}

/// ★Measures that the temporary View is actually reclaimed★ — if one counts windows, on the failure path no window entry is made, so it is green even if the whole
/// rollback is deleted, and in the meantime Views with no owner pile up in `views` endlessly with every retry
/// (the ④ endpoint of the `apply.rs` 「owner-less tmp_view」 invariant disappears).
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

    assert!(err.contains("창 생성 실패"), "err={err}");
    assert!(
        w.host.placed.lock().unwrap().is_empty(),
        "못 연 창은 적지 않는다"
    );
    assert_eq!(
        w.view_count(),
        views_before,
        "임시 View 가 회수되지 않았다 — 실패한 pop-out 마다 유령 View 가 쌓인다"
    );
    assert_eq!(
        w.resyncs() - resyncs_before,
        2,
        "phase A 재동기 + 롤백 재동기 = 2(롤백이 빠지면 1)"
    );
    assert_eq!(
        tree::find_slot(&w.snapshot(view).layout, slot),
        Some(&content),
        "빌드 실패 시 소스 슬롯은 그대로 — 사용자가 콘텐츠를 잃지 않는다"
    );
}

/// ★Sets up the state that only `is_open` can filter out★: the moment the target window is **alive in the model** but its OS window has already vanished
/// (the `Destroyed` cleanup has not yet deleted the model). The model re-validation of `insert_tab_into` lets this through, so
/// without `is_open` the content goes in as a tab of a window with no screen and the source slot is closed, leaving it on no screen.
#[test]
fn move_slot_to_window_rejects_a_target_whose_os_window_is_gone() {
    let w = World::new();
    let target = w.popup();
    // Only the OS window vanished — the model entry is as it was.
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

    assert!(err.contains("대상 창 없음"), "err={err}");
    assert_eq!(
        tree::find_slot(&w.snapshot(view).layout, slot),
        Some(&content),
        "거절은 무행위여야 한다 — 소스 슬롯을 닫으면 콘텐츠가 어느 화면에도 안 남는다"
    );
    assert_eq!(
        apply::list_tabs(&w.state, &target).unwrap().tabs.len(),
        tabs_before,
        "화면 없는 창에 탭을 밀어 넣지 않는다"
    );
    assert_eq!(w.view_count(), views_before, "임시 View 도 회수된다");
}

// A label that is not even in the model is filtered by `is_open` first and goes out as 「대상 창 없음」 (no target window) (it does not reach the model re-validation).
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

    assert!(err.contains("대상 창 없음"), "err={err}");
    assert_eq!(
        tree::find_slot(&w.snapshot(view).layout, slot),
        Some(&content),
        "거절은 무행위여야 한다"
    );
    assert_eq!(w.view_count(), views_before, "임시 View 회수");
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
    assert_eq!(slots.len(), 1, "새 탭 = 빈 root 1개");
    assert_eq!(
        tree::find_slot(&w.snapshot(placed).layout, slots[0]),
        Some(&SlotContent::Agent {
            agent_id: agent.to_string()
        })
    );
    assert_eq!(w.layout_events(), 1);
}

/// ★It blocks only unknown words★ — if a typo passes, that round trip dies in the daemon (a new daemon) or silently starts another backend
/// (an old daemon that does not know the field). The rejection message takes the vocabulary from the wire enum rather than writing it by hand.
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

    assert!(err.contains("스폰 안 함"), "err={err}");
    assert!(
        err.contains("claude") && err.contains("codex"),
        "무엇이 통하는지 말해야: {err}"
    );
    assert_eq!(spawner.calls(), 0, "★스폰 전에 거부★");
}

/// A known word **goes to the port as it is** — if this line is green, the blanket rejection of ADR-0058 has really been lifted.
///
/// ★The shell does not look at policy — every word the wire knows stands here★ (policy is in the daemon's `SpawnByCwd` handling part · ADR-0279).
/// If a word is removed from that list, this file no longer measures anywhere whether that word actually reaches the port.
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

    // Whether a blank backend passed as "unspecified" is not caught by the spawn call alone — only after the layout step too does the
    // pass path split from the rejection path (the rejection path has zero spawns and zero layouts).
    assert_eq!(spawner.calls(), 1);
    // ★The shell does not invent defaults★ — unspecified goes to the port as unspecified (rejecting that absence is the
    //   daemon's job, and that way the 「어느 칸을 채워라」 (fill which field) message comes from one place only).
    assert_eq!(&*spawner.backends.lock().unwrap(), &[None]);
    assert_eq!(id, agent.to_string());
    let placed = w.main_active();
    assert_eq!(
        apply::list_tabs(&w.state, MAIN_WINDOW_LABEL)
            .unwrap()
            .tabs
            .len(),
        tabs_before + 1,
        "tab=None → 새 탭"
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

    assert!(err.contains("스폰 안 함"), "err={err}");
    assert_eq!(spawner.calls(), 0, "orphan 탭 방지 가드 = pre-spawn");
}

// Gives the layout failure as an occupied area — a tab of another window is now rejected before the spawn (test below).
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

    assert_eq!(spawner.calls(), 1, "스폰은 이미 일어났다(하드 롤백 없음)");
    assert!(
        err.contains(&agent.to_string()),
        "생존 agent id 를 오류에 박아 invisible 에이전트를 막는다: err={err}"
    );
}

/// If a tab is given, it is placed in that tab's window — without a window it is that tab's window, and for another window · a nonexistent tab it does not launch (so that an agent with no place to be placed does not
/// survive — TRD S28 §7).
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
        assert_eq!(never.calls(), 0, "{tab}: 스폰하지 않는다");
        assert!(err.contains("스폰 안 함"), "err={err}");
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
    assert_eq!(placed.rows.len(), 1, "놓인 탭의 줄");
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

    assert_eq!(err, "spawn 실패: 데몬 거절");
    assert_eq!(
        apply::list_tabs(&w.state, MAIN_WINDOW_LABEL)
            .unwrap()
            .tabs
            .len(),
        1,
        "스폰 실패면 탭도 안 만든다"
    );
    assert_eq!(w.resyncs(), 0);
}

/// ★A policy rejection of a known word makes one trip to the daemon and comes back as it is with no layout★ (ADR-0279) — the shell sends that word
/// through only the net to the port, returns the port's `Err` with its message as it is, and does not touch the layout.
/// `spawn_into_propagates_spawn_failure_untouched` has no word, so it cannot measure 「낱말이 데몬까지 갔다가 거절되어
/// 돌아온다」 (the word goes to the daemon, is rejected and comes back).
/// The message imitates the shape of a daemon policy rejection — the shell does not read that text, so anything is the same.
#[tokio::test]
async fn spawn_into_passes_a_daemon_refusal_through_without_placing() {
    let w = World::new();
    let refusal = "backend 'codex' 는 아는 낱말이지만 이 표면으로는 지금 만들지 않는다 — 시험용 사유. 스폰 안 함.";
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

    assert_eq!(spawner.calls(), 1, "셸은 정책을 안 보고 데몬에 묻는다");
    assert_eq!(
        &*spawner.backends.lock().unwrap(),
        &[Some(AgentBackendKind::Codex)],
        "고른 낱말이 포트까지 갔다"
    );
    assert_eq!(err, refusal, "데몬 문구가 그대로 돌아온다");
    assert_eq!(
        apply::list_tabs(&w.state, MAIN_WINDOW_LABEL)
            .unwrap()
            .tabs
            .len(),
        1,
        "거절이면 탭도 안 만든다"
    );
    assert_eq!(w.resyncs(), 0);
}

// ── set_split_ratio (ADR-0227 — the layout notification only on Applied) ──────

impl World {
    // A world where the first area of main's active tab is split left/right — returns = (tab, that split).
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
            .expect("레이아웃 알림")
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
        "레이아웃 통지 정확히 1"
    );
    assert_eq!(w.tab_events(), tabs_before, "탭 통지 0");
    assert_eq!(w.resyncs(), resyncs_before, "라우팅 불변 — 재동기 없음");

    let snap = w.last_layout();
    assert_eq!(snap.view_id, view);
    assert_eq!(
        got.version, snap.version,
        "반환 version == 통지된 스냅샷 version"
    );
    let LayoutNode::Split { ratio, .. } = &snap.layout else {
        panic!("분할 트리여야 한다: {:?}", snap.layout)
    };
    assert_eq!(*ratio, 0.3);
    let rect = snap
        .split_rects
        .iter()
        .find(|r| r.split_id == split)
        .expect("분할 사각형");
    assert_eq!(rect.at, 0.3, "사각형도 새 비율로 계산된다");
    let left = snap
        .slot_rects
        .iter()
        .find(|r| r.x0 == 0.0)
        .expect("왼쪽 칸");
    assert_eq!(left.x1, 0.3, "a = 왼쪽 칸이 30%");
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

    // Split width L = 300px < 2 × 200px — the px range is empty.
    apply::report_window_canvas(&w.state, MAIN_WINDOW_LABEL, 300, 300).unwrap();
    let mut big_min = one_px_border();
    big_min.min_pane_px = 200;
    apply::report_ui_metrics(&w.state, MAIN_WINDOW_LABEL, big_min).unwrap();
    let small = apply::set_split_ratio(&w.state, &w.ev, view, split, 0.3).unwrap();
    assert_eq!(small.outcome, SplitRatioOutcome::TooSmall);
    assert_eq!(small.ratio, 0.5, "지금 값 그대로");

    assert_eq!(
        (w.layout_events(), w.tab_events()),
        events_before,
        "무변경 결말은 아무것도 안 알린다"
    );
    assert_eq!(w.snapshot(view).version, version, "version 불변");
}

#[test]
fn set_split_ratio_unknown_split_is_err_without_notify() {
    let w = World::new();
    let (view, split) = w.split_view();
    let events_before = (w.layout_events(), w.tab_events());

    let err = apply::set_split_ratio(&w.state, &w.ev, view, Uuid::new_v4(), 0.3).unwrap_err();
    assert!(err.contains("split 없음"), "err={err}");
    let err = apply::set_split_ratio(&w.state, &w.ev, view, split, f64::NAN).unwrap_err();
    assert!(err.contains("비율 거절"), "err={err}");

    assert_eq!((w.layout_events(), w.tab_events()), events_before);
}

/// A port that interjects another write at the very place it receives the notification — if the implementation releases the lock and reads the global counter again,
/// the returned version is off by the value of this interjected write.
struct Meddler {
    inner: Recorder,
    state: LayoutState,
}

impl LayoutEvents for Meddler {
    fn layout_updated(&self, snapshot: &ViewSnapshot) {
        self.inner.layout_updated(snapshot);
        let mut mgr = self.state.0.lock().unwrap();
        mgr.rename_tab(snapshot.view_id, "끼어든 쓰기".to_string())
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
        .expect("알림 1");
    assert_eq!(got.version, notified.version);
    assert!(
        w.snapshot(view).version > got.version,
        "전제: 끼어든 쓰기가 전역 카운터를 올렸다"
    );
}

// One write emitted one or more notifications — empties that order.
fn assert_notified(w: &World, write: &str) {
    assert!(!w.ev.take_order().is_empty(), "{write}: 알림이 있어야 한다");
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
    apply::rename_tab(&w.state, &w.ev, main, "이름".to_string()).unwrap();
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
    // The last tab is closed and the whole window vanishes — there is no window to notify.
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

// The picture before one write — [`TitleCheck::after`] measures invariant 2: for every window whose title changed (including a new window), that write announced that window's tab
//   list with a larger version than before the write, and that window's last notification carries the current title.
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

    // `changes` = which window's title this write changes — seen as a pair so that running only writes that do not change titles does not pass vacuously.
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
            "{write}: 제목이 바뀐 창 {changed:?}"
        );
        for label in changed {
            let last = sent
                .iter()
                .rev()
                .find(|t| &t.label == label)
                .unwrap_or_else(|| panic!("{write}: 제목이 바뀐 {label} 을 알리지 않았다"));
            assert_eq!(last.title, after[label], "{write}: {label}");
            assert!(
                last.version > self.version,
                "{write}: {label} 을 옛 판으로 알렸다"
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
    assert_eq!(titles(&w)[MAIN_WINDOW_LABEL], "빈 칸 - Engram:1");

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
            Some("작업".to_string()),
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
        apply::rename_tab(&w.state, &w.ev, first, "이름".to_string()).unwrap()
    });
    title_write(&w, "close_tab", true, || {
        apply::close_tab(&w.state, &w.subs, &w.ev, &w.host, MAIN_WINDOW_LABEL, first).unwrap()
    });
    assert_eq!(titles(&w)[MAIN_WINDOW_LABEL], "작업 - Engram:1");
    let work_slot = w.empty_slot(work);
    title_write(&w, "move_slot_to_window(새 창)", true, || {
        apply::move_slot_to_window(
            &w.state, &w.subs, &w.ev, &w.host, &w.labels, work, work_slot, None,
        )
        .unwrap()
    });
    let (view, filled, _) = w.view_with_filled_slot();
    title_write(&w, "move_slot_to_window(있는 창)", true, || {
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
    title_write(&w, "dock_tab(탭바)", true, || {
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
    assert_eq!(titles(&w)[popup.as_str()], "작업 - Engram:2");
    title_write(&w, "dock_tab(새 창)", true, || {
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
    assert_eq!(titles(&w)[MAIN_WINDOW_LABEL], "에이전트 - Engram:1");
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
                "빈 칸 - Engram:1".to_string(),
                version
            ),
            (popup, "빈 칸 - Engram:2".to_string(), version),
        ]
    );
    assert_eq!(w.layout_events(), 0);
    assert_eq!(w.version(), version, "판을 올리지 않는다");
    assert_eq!(w.resyncs(), 1, "재동기 없음 — 앞의 1 은 창 만들기 몫");
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
// The two functions take no notification · resync ports at all, so 「통지 0」 (zero notifications) is guarded by the signature first. What is measured here is the
// rest — that the version is unchanged, and that whether it succeeds or fails, no sibling command's notification goes out with it.

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

    assert_eq!(w.layout_events(), 0, "레이아웃 통지 0");
    assert_eq!(w.tab_events(), 0, "탭 통지 0");
    assert_eq!(w.resyncs(), 0, "라우팅 불변 — 재동기 없음");
    assert_eq!(
        apply::list_tabs(&w.state, MAIN_WINDOW_LABEL)
            .unwrap()
            .version,
        before,
        "측정 보고는 version 을 올리지 않는다"
    );

    // It was saved — the shell computation reads that value.
    let view = w.main_active();
    let slot = w.slots(view)[0];
    let px = w
        .state
        .0
        .lock()
        .unwrap()
        .slot_px(view, slot)
        .unwrap()
        .expect("캔버스·지표 둘 다 보고됨");
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
    assert!(err.contains("window 없음"), "err={err}");
    let err = apply::report_ui_metrics(&w.state, "no-such", one_px_border()).unwrap_err();
    assert!(err.contains("window 없음"), "err={err}");
    let mut bad = one_px_border();
    bad.frame_insets.l = f64::NAN;
    let err = apply::report_ui_metrics(&w.state, MAIN_WINDOW_LABEL, bad).unwrap_err();
    assert!(err.contains("ui 지표 거절"), "err={err}");
    // Zero size is not an error but an ignore.
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
    assert!(err.contains("view 없음"), "err={err}");
}

#[test]
fn list_tabs_returns_tabs_active_and_version() {
    let w = World::new();
    let before = apply::list_tabs(&w.state, MAIN_WINDOW_LABEL).unwrap();
    apply::create_tab(&w.state, &w.subs, &w.ev, MAIN_WINDOW_LABEL, None).unwrap();
    let after = apply::list_tabs(&w.state, MAIN_WINDOW_LABEL).unwrap();

    assert_eq!(after.label, MAIN_WINDOW_LABEL);
    assert_eq!(after.tabs.len(), before.tabs.len() + 1);
    assert!(after.version > before.version, "변경마다 version 증가");
}

#[test]
fn list_tabs_unknown_window_is_err() {
    let w = World::new();
    let err = apply::list_tabs(&w.state, "no-such").unwrap_err();
    assert!(err.contains("window 없음"), "err={err}");
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
    assert!(err.contains("알 수 없는 공간 토큰"), "err={err}");
}

#[test]
fn resolve_spatial_unknown_view_is_err() {
    let w = World::new();
    let err = apply::resolve_spatial(&w.state, "left", None, Some(Uuid::new_v4())).unwrap_err();
    assert!(err.contains("view 없음"), "err={err}");
}

#[test]
fn resolve_spatial_unknown_window_is_err() {
    let w = World::new();
    let err = apply::resolve_spatial(&w.state, "left", Some("no-such"), None).unwrap_err();
    assert!(err.contains("window 없음"), "err={err}");
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
        "새 탭은 단일 슬롯 — 오른쪽 이웃 없음"
    );
}

// ── dock_tab (TRD S21-slot-docking §3-3) ──────────────────────
//
// What is not measured here: the rule that if a new window's place straddles no monitor it goes to the default place belongs to the Tauri implementation of the window port, so
// the `lib_unit` unit test of `resolve_place` in `state::placement` measures it. The branch where the new window is closed before attaching and a tombstone stands · the branch where a window closes during the reuse
// move belong to the model part of the close request, a function inside the crate, so the `commands::popout` tests (`lib_unit`) measure them.

// ADR-0057 ①–④ — the ownership pair matches on both sides · one owner window per View · the active one is a tab of that window · main tabs ≥ 1.
fn assert_invariants(w: &World) {
    let mgr = w.state.0.lock().unwrap();
    let mut tabs = 0;
    for (label, wt) in &mgr.windows {
        assert!(
            wt.tabs.contains(&wt.active),
            "③ {label} 의 활성이 그 창 탭이 아니다"
        );
        for view in &wt.tabs {
            assert_eq!(mgr.view_owner.get(view), Some(label), "① 탭 {view}");
            assert!(mgr.views.contains_key(view), "① 탭 {view} 의 View 가 없다");
        }
        tabs += wt.tabs.len();
    }
    assert_eq!(mgr.views.len(), tabs, "② 창 없는 View 나 두 창의 탭");
    assert_eq!(mgr.view_owner.len(), tabs, "① 남은 소유 쌍");
    assert!(
        mgr.windows
            .get(MAIN_WINDOW_LABEL)
            .is_some_and(|w| !w.tabs.is_empty()),
        "④ main 탭 ≥ 1"
    );
}

const AT: WindowPlace = WindowPlace {
    pos: Some((300, 200)),
    size: None,
};

fn bounds(x: f64, y: f64, w: f64, h: f64) -> Option<WindowBounds> {
    WindowBounds::new(x, y, w, h)
}

// At every notification, sees whether the window is still not closed — the eye that measures whether closing an empty source popout comes after the notification bundle.
struct ClosesAfter<'a> {
    inner: &'a Recorder,
    host: &'a Host,
}

impl ClosesAfter<'_> {
    fn not_closed_yet(&self) {
        assert!(
            self.host.closed.lock().unwrap().is_empty(),
            "빈 출처 팝아웃은 알림을 다 낸 뒤에 닫는다"
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

    // A popout with `n` empty tabs — returns = (label, tab order). The last tab is active.
    fn popup_with(&self, n: usize) -> (String, Vec<Uuid>) {
        let label = self.popup();
        let mut tabs = vec![apply::list_tabs(&self.state, &label).unwrap().active];
        for _ in 1..n {
            tabs.push(apply::create_tab(&self.state, &self.subs, &self.ev, &label, None).unwrap());
        }
        (label, tabs)
    }

    // Writes the window's model place — the same path as production's `Moved` · `Resized` handlers.
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

    // Empties the record the preparation writes left — the later assertions see only one docking. ★The opened-window list (`opened`) is kept★ — this fake's
    // `is_open` · revert look at it to see whether the window is up.
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
            .unwrap_or_else(|| panic!("{label} 탭 알림"))
    }

    // The snapshots · tab lists it emitted are all at the current `version` — they were taken in the same critical section as the change.
    fn assert_notified_at_current_version(&self) {
        let version = self.version();
        for s in self.ev.layout.lock().unwrap().iter() {
            assert_eq!(s.version, version, "스냅숏 {}", s.view_id);
        }
        for t in self.ev.tabs.lock().unwrap().iter() {
            assert_eq!(t.version, version, "탭 목록 {}", t.label);
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
    assert_eq!(w.resyncs(), resyncs + 1, "재동기 한 번 — 락 안(프로브)");
    assert_eq!(
        w.subs.router.targets(agent).to_vec(),
        [MAIN_WINDOW_LABEL.to_string()],
        "옮긴 에이전트의 출력은 대상 창으로"
    );
    assert!(w.unsubscribed().is_empty(), "구독은 끊기지 않는다");
    assert!(w.slots(target).contains(&slot), "대상 탭에 끼웠다");
    assert_eq!(w.owner(source), None, "출처 View 는 지워진다");
    assert_eq!(
        w.notified_views(),
        [target, tabs[0]],
        "대상 배치 + 출처 창의 새 활성 배치"
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
    assert_eq!(main.active, source, "Reveal — 옮긴 탭이 보인다");
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
        "닫아도 지운 항목은 돌려준다"
    );
    assert_eq!(&*w.host.closed.lock().unwrap(), &[label.clone()], "한 번");
    assert!(!w.window_exists(&label), "모델에서 먼저 지웠다");
    assert_eq!(
        w.notified_windows(),
        [MAIN_WINDOW_LABEL],
        "지운 창의 탭 목록은 없다"
    );
    assert_notified(&w, "dock_tab(빈 출처)");
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
    assert_eq!(w.resyncs(), resyncs, "실패 = 재동기 없음");
    assert_eq!(w.ev.take_order(), Vec::<&str>::new(), "알림 0");
    assert!(
        w.host.opened_at.lock().unwrap().is_empty(),
        "새 창도 안 연다"
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
    assert!(!out.changed, "결말이 무변경을 말한다");
    assert_eq!(out.views, [tabs[1]]);
    assert_eq!(w.version(), version);
    assert_eq!(w.resyncs(), resyncs);
    assert_eq!(
        w.ev.take_order(),
        Vec::<&str>::new(),
        "무변경 결말은 알리지 않는다"
    );
}

#[test]
fn dock_tab_of_mains_last_tab_announces_the_fresh_empty_tab() {
    // Tab bar (another window) · new window — in both, an empty tab is created in main and its id is returned, and that layout · tab list ride in the notification.
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
        assert_eq!(main.len(), 1, "{mode}: main 에 빈 탭 하나");
        let fresh = main[0];
        assert_ne!(fresh, only);
        assert_eq!(out.main_refilled, Some(fresh), "{mode}");
        let payload = w.notified_tabs_of(MAIN_WINDOW_LABEL);
        assert_eq!(payload.active, fresh, "빈 탭이 main 의 활성");
        assert_eq!(
            payload.tabs.iter().map(|m| m.id).collect::<Vec<_>>(),
            [fresh]
        );
        assert!(
            w.notified_views().contains(&fresh),
            "빈 탭의 배치도 싣는다 — 탭 id 만 받으면 그릴 배치가 없다"
        );
        assert_eq!(
            out.views,
            [only, fresh],
            "{mode}: 결말의 바뀐 탭 = 옮긴 탭 + main 에 새로 세운 빈 탭"
        );
        w.assert_notified_at_current_version();
        assert_notified(&w, "dock_tab(main 마지막 탭)");
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
    // Popout [t0, t1, t2] · active t1 — if it were succession, the right neighbor t2.
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
            "{pick:?}: 새 활성의 배치도 싣는다"
        );
        assert_notified(&w, "dock_tab(다음 활성)");
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
        LayoutNode::Split { ratio, .. } => assert_eq!(ratio, 0.75, "새것 몫 1/4 · 오른쪽"),
        other => panic!("새 분할이어야: {other:?}"),
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
        "main 출처 = 크기 없음(팝아웃 기본)"
    );
    assert_eq!(&*w.host.placed.lock().unwrap(), &[label.clone()]);
    assert!(w.host.closed.lock().unwrap().is_empty());
    assert_eq!(w.tabs_of(&label), [source]);
    assert_eq!(w.owner(source).as_deref(), Some(label.as_str()));
    assert_eq!(w.resyncs(), resyncs + 1);
    assert_eq!(w.notified_views(), [source, first]);
    assert_eq!(w.notified_windows(), [label.as_str(), MAIN_WINDOW_LABEL]);
    // TRD S21-slot-docking §8-1 ⑬ — `open_at` → `dock` → notification → `record_placement`.
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
    assert_notified(&w, "dock_tab(새 창)");
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
        "Keep = 포커스를 가져가지 않는다"
    );
    assert_eq!(
        w.host.claims_asked.load(Ordering::Relaxed),
        1,
        "Keep 이어도 예열 창을 묻는다(TRD §3-9 I4)"
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
        "못 연 창은 닫을 것도 없다"
    );
    assert!(w.host.placed.lock().unwrap().is_empty());
    assert_invariants(&w);
}

#[test]
fn dock_tab_new_window_closes_the_opened_window_when_phase_c_refuses() {
    let w = World::new();
    let source = apply::create_tab(&w.state, &w.subs, &w.ev, MAIN_WINDOW_LABEL, None).unwrap();
    w.forget();
    // While opening the window, another path closes the source tab.
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
        "방금 연 창을 거둔다"
    );
    assert!(!w.window_exists(&opened[0]));
    assert_eq!(w.owner(source), None, "닫힌 탭은 새 창에 붙지 않는다");
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

    assert_eq!(out.window, spare, "맡긴 창의 label 로 연다");
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

// The new window of `Keep` also uses a prewarmed window — not activating is received by the handed-over window's `open_at` (TRD §3-9 I4).
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
    assert!(!w.claim_outstanding(), "창 포트가 거뒀다");
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
        "닫기 · 알림 · 자리 기록 없음"
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
        "적용 서비스는 닫지 않는다"
    );
}

// ── dock_tab: reuse ──

impl World {
    // A popout with only one tab — moves that window rather than a new window (`plan_new_window`). Also writes down the normal place. Returns = (label, tab).
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
    assert!(w.host.opened_at.lock().unwrap().is_empty(), "새 창 없음");
    assert_eq!(&*w.host.placed.lock().unwrap(), &[label.clone()], "한 번");
    assert_eq!(w.version(), version, "모델 변경 0");
    assert_eq!(w.resyncs(), resyncs);
    assert_eq!(w.ev.take_order(), Vec::<&str>::new(), "알릴 것이 없다");
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
        "전제: 보통 자리를 아직 못 봤다"
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
    assert!(w.host.opened_at.lock().unwrap().is_empty(), "새 창 없음");
    assert!(w.host.closed.lock().unwrap().is_empty());
    assert_eq!(&*w.host.placed.lock().unwrap(), &[label.clone()]);
    assert_eq!(w.version(), version);
    assert_eq!(w.ev.take_order(), Vec::<&str>::new());
    assert_invariants(&w);
}

#[test]
fn dock_tab_reuse_finishes_the_drop_even_if_the_source_changes_during_the_move() {
    // User decision (2026-10-16) — once the drag stands, it stays at the place it was dropped. Writes during the move are neither re-decided nor reverted:
    // it ends with the model that write left. (what, the interjecting write, whether that window remains in the model)
    type Meddle = fn(&mut ViewManager, &str, Uuid, Uuid);
    let cases: [(&str, Meddle, bool); 3] = [
        (
            "출처 탭에 슬롯 더함",
            |mgr, _, view, slot| {
                mgr.split_slot(view, slot, SplitDir::TopBottom).unwrap();
            },
            true,
        ),
        (
            "그 팝아웃에 탭 더함",
            |mgr, label, _, _| {
                mgr.create_tab(label, None).unwrap();
            },
            true,
        ),
        (
            "출처 탭 닫음(창째 사라짐)",
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
            "{what}: 옮기는 사이 닫힌 창은 신원이 없다"
        );
        assert_eq!(w.host.moves.lock().unwrap().len(), 1, "{what}");
        assert!(w.host.opened_at.lock().unwrap().is_empty(), "{what}");
        assert!(
            w.host.closed.lock().unwrap().is_empty(),
            "{what}: 거두지 않는다"
        );
        assert_eq!(w.window_exists(&label), kept, "{what}");
        let placed = w.host.placed.lock().unwrap().clone();
        assert_eq!(
            placed,
            if kept { vec![label.clone()] } else { vec![] },
            "{what}: 모델에 남은 창만 자리를 적는다"
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
            .filter(|l| l.message.contains("출처 창을 못 옮겼다"))
            .collect();
        assert_eq!(ends.len(), 1, "{failing:?}: 끝 줄 하나 — {lines:?}");
        assert_eq!(ends[0].level, tracing::Level::WARN, "{failing:?}");
        let step = format!("{failing:?}");
        assert_eq!(ends[0].field("step"), Some(step.as_str()));
        assert_eq!(ends[0].field("label"), Some(label.as_str()));
        assert_eq!(w.host.steps(), called, "{failing:?}: 첫 실패에서 멈춘다");
        assert_eq!(
            w.host.moves.lock().unwrap().len(),
            1,
            "{failing:?}: 다시 옮기지 않는다"
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

/// Collects all events emitted on this thread (the same shape as the same-named fake of the `settings` tests). On creation, it sets up [`ASK_ALWAYS`].
#[derive(Clone)]
struct Captured(std::sync::Arc<Mutex<Vec<Line>>>);

impl Default for Captured {
    fn default() -> Self {
        LazyLock::force(&ASK_ALWAYS);
        Self(std::sync::Arc::default())
    }
}

// tracing-core 0.1.36: if only one subscriber was alive at the last subscriber registration, the first registration of a callsite asks only **that thread's** default subscriber for interest
//   (`Dispatchers::rebuilder` → `JustOne`). If a test thread outside the capture hits a shared place such as 「도킹 적용」 (docking applied) for the first time during a capture,
//   「관심 없음」 (no interest) is pinned globally and that capture comes up empty-handed (measured — 38 of 120 runs with 6 in parallel). If this is held for the whole binary, the first
//   registration asks all live subscribers, and since this is always 「때마다 묻기」 (ask every time), 「관심 없음」 does not get pinned. The remaining gap = one same race at the
//   moment this is first set up (untested — if it leaks, the symptom is the one above).
static ASK_ALWAYS: LazyLock<tracing::Dispatch> =
    LazyLock::new(|| tracing::Dispatch::new(AskAlways));

/// Receives nothing and answers every callsite only with 「때마다 묻기」 (ask every time) — [`ASK_ALWAYS`].
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
        .filter(|l| l.message.contains("도킹 적용"))
        .map(|l| (l.level, l.field("changed")))
        .collect();
    assert_eq!(
        applied,
        [
            (tracing::Level::INFO, Some("false")),
            (tracing::Level::INFO, None)
        ],
        "무변경 결말만 그것을 적는다"
    );
}

// ── drop command — find the target again at drop time (ADR-0297 · TRD S21-slot-docking §3-3 · §8-1 ⑦) ─────────────────────
//
// The drop command (`tab_drop`) carries what was decided with the web store — even if that store is behind the model, if what the drop pointed to (the dragged tab · target
// tab · target area · tab bar window) exists now, the same `dock_tab` applies it to the current state, and if not, it rejects. The request is built with the same
// pure decision as production (`tab_drop::plan::plan`).

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

    // Decides and applies one drop command — the same path as production's `tab_drop` (the input is a drop in our webview).
    fn tab_drop(&self, drop: &TabDrop, caller: &str) -> Result<DockOutcome, DockError> {
        match drop_plan::plan(drop, caller, &dropped_at(500, 400), &FLAT_FRAME) {
            Plan::Dock(req) => self.dock_req(req),
            other => panic!("도킹 요청이어야: {other:?}"),
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

    // Seam drop — the right half of `anchor` of `target`.
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
        assert_eq!(self.version(), version, "{what}: 모델 그대로");
    }
}

// main = [m0, t] · popout = [p] — after starting to drag `t` from main, the LLM moved `t` to the popout (D9).
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
        .expect("탭바");
    assert_eq!(out.window, MAIN_WINDOW_LABEL);
    assert_eq!(w.tabs_of(MAIN_WINDOW_LABEL), [t, m0]);
    assert_eq!(w.tabs_of(&popout), [p], "지금 창(팝아웃)에서 뺐다");
    assert_invariants(&w);

    let (w, _, t, popout, p) = setup();
    let out = w
        .tab_drop(
            &w.seam_drop(t, MAIN_WINDOW_LABEL, None, p, DropAnchor::Outer),
            &popout,
        )
        .expect("이음새");
    assert_eq!(out.window, popout);
    assert_eq!(w.tabs_of(&popout), [p]);
    assert_eq!(w.owner(t), None, "t 는 p 에 붙어 사라졌다");
    assert_invariants(&w);

    let (w, _, t, popout, p) = setup();
    let out = w
        .tab_drop(&w.new_window_drop(t, MAIN_WINDOW_LABEL), &popout)
        .expect("새 창");
    assert!(!out.reused, "팝아웃에 탭이 둘이라 새 창");
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
    assert_eq!(w.tabs_of(&popout), [i, p, t], "전제");
    assert_eq!(w.active_of(&popout), t, "전제");
    w.tab_drop(&drop, MAIN_WINDOW_LABEL).expect("받는다");
    assert_eq!(w.tabs_of(&popout), [i, p]);
    assert_eq!(
        w.active_of(&popout),
        p,
        "승계(왼쪽 이웃) — initial 이 아니다"
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
        "창을 열기 전에 거절"
    );
    assert!(w.host.moves.lock().unwrap().is_empty());
    assert_eq!(w.ev.take_order(), Vec::<&str>::new(), "알림 없음");
}

#[test]
fn a_tab_bar_drop_lands_in_the_tab_list_as_it_is_now_and_clamps_to_its_end() {
    // Tabs increased after the decision — the position is counted in the current list.
    let w = World::new();
    let m0 = w.main_active();
    let a = apply::create_tab(&w.state, &w.subs, &w.ev, MAIN_WINDOW_LABEL, None).unwrap();
    let (popout, tabs) = w.popup_with(2);
    let t = tabs[1];
    let drop = w.tab_bar_drop(t, &popout, Some(tabs[0]), 1);
    let c = apply::create_tab(&w.state, &w.subs, &w.ev, MAIN_WINDOW_LABEL, None).unwrap();
    w.tab_drop(&drop, MAIN_WINDOW_LABEL).expect("받는다");
    assert_eq!(w.tabs_of(MAIN_WINDOW_LABEL), [m0, t, a, c]);
    assert_eq!(w.active_of(MAIN_WINDOW_LABEL), t, "놓은 탭이 보인다");
    assert_invariants(&w);

    // Tabs decreased after the decision — the overflowed position is the end position of the current list.
    let w = World::new();
    let m0 = w.main_active();
    let a = apply::create_tab(&w.state, &w.subs, &w.ev, MAIN_WINDOW_LABEL, None).unwrap();
    let b = apply::create_tab(&w.state, &w.subs, &w.ev, MAIN_WINDOW_LABEL, None).unwrap();
    let (popout, tabs) = w.popup_with(2);
    let t = tabs[1];
    let drop = w.tab_bar_drop(t, &popout, Some(tabs[0]), 3);
    apply::close_tab(&w.state, &w.subs, &w.ev, &w.host, MAIN_WINDOW_LABEL, b).unwrap();
    w.tab_drop(&drop, MAIN_WINDOW_LABEL).expect("받는다");
    assert_eq!(w.tabs_of(MAIN_WINDOW_LABEL), [m0, a, t]);
    assert_eq!(w.tabs_of(&popout), [tabs[0]]);
    assert_invariants(&w);

    // Same window — counted in the current list without the dragged tab.
    let w = World::new();
    let m0 = w.main_active();
    let a = apply::create_tab(&w.state, &w.subs, &w.ev, MAIN_WINDOW_LABEL, None).unwrap();
    let t = apply::create_tab(&w.state, &w.subs, &w.ev, MAIN_WINDOW_LABEL, None).unwrap();
    let b = apply::create_tab(&w.state, &w.subs, &w.ev, MAIN_WINDOW_LABEL, None).unwrap();
    let drop = w.tab_bar_drop(t, MAIN_WINDOW_LABEL, Some(m0), 3);
    apply::close_tab(&w.state, &w.subs, &w.ev, &w.host, MAIN_WINDOW_LABEL, a).unwrap();
    w.tab_drop(&drop, MAIN_WINDOW_LABEL).expect("받는다");
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
        "닫힌 창",
    );
    assert_eq!(w.owner(t).as_deref(), Some(MAIN_WINDOW_LABEL));
}

#[test]
fn a_seam_drop_lands_in_the_target_layout_as_it_is_now() {
    // Target = tab q of the popout · dragged tab = t of main.
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
    apply::rename_tab(&w.state, &w.ev, q, "새 이름".into()).unwrap();
    let out = w.tab_drop(&drop, &popout).expect("받는다");
    assert_eq!(out.window, popout);
    let slots = w.slots(q);
    assert!(
        [s, n].iter().chain(&moved).all(|x| slots.contains(x)),
        "나뉜 두 칸 + 붙은 칸: {slots:?}"
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
        "기준 칸 없음",
    );

    // The target tab vanished in the meantime — rejected.
    let (w, t, popout, q, _) = setup();
    let other = apply::create_tab(&w.state, &w.subs, &w.ev, &popout, None).unwrap();
    let drop = w.seam_drop(t, MAIN_WINDOW_LABEL, None, q, DropAnchor::Outer);
    apply::close_tab(&w.state, &w.subs, &w.ev, &w.host, &popout, q).unwrap();
    w.assert_refused(
        w.tab_drop(&drop, &popout),
        DockError::TabNotFound(q),
        "대상 탭 없음",
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
    let out = w.tab_drop(&drop, MAIN_WINDOW_LABEL).expect("받는다");
    assert_eq!(out.window, popout);
    assert_eq!(w.active_of(&popout), q, "대상 탭이 보인다");
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
        assert_eq!(w.active_of(MAIN_WINDOW_LABEL), d, "누른 탭이 활성");
        (w, i, d)
    };

    // The drop arrives first — active = d.
    let (w, i, d) = setup();
    let drop = w.seam_drop(d, MAIN_WINDOW_LABEL, Some(i), i, DropAnchor::Outer);
    assert!(w.tab_drop(&drop, MAIN_WINDOW_LABEL).is_ok());
    assert_eq!(w.tabs_of(MAIN_WINDOW_LABEL), [i]);
    assert_invariants(&w);

    // The source's original-selection revert (`switch_tab_if(initial)`) arrives first — active = i.
    let (w, i, d) = setup();
    let drop = w.seam_drop(d, MAIN_WINDOW_LABEL, Some(i), i, DropAnchor::Outer);
    apply::switch_tab(&w.state, &w.subs, &w.ev, MAIN_WINDOW_LABEL, i).unwrap();
    let out = w.tab_drop(&drop, MAIN_WINDOW_LABEL).unwrap();
    assert_eq!(out.window, MAIN_WINDOW_LABEL);
    assert_eq!(
        w.tabs_of(MAIN_WINDOW_LABEL),
        [i],
        "d 가 i 의 이음새에 붙었다"
    );

    // The LLM showed a third tab — it still attaches, and the target tab is shown.
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
        "끈 자리로 옮기고 크기는 고르지 않는다"
    );
    assert!(w.host.opened_at.lock().unwrap().is_empty(), "새 창 없음");
    assert_eq!(w.version(), version, "모델 변경 0");
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

    // Puts a new agent into an area — returns = that id.
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
        "칸 id 만으로도 그 탭 · 창을 찾는다"
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
        "한 탭 안 두 칸이면 탭은 하나다"
    );

    let LocateError::AgentAmbiguous { agent, places } =
        w.locate(&Target::SlotOf(twice)).unwrap_err()
    else {
        panic!("두 칸이면 모호하다");
    };
    assert_eq!(agent, twice);
    assert_eq!(places.iter().map(|p| p.slot).collect::<Vec<_>>(), [m1, m2]);

    let spread_err = w.locate(&Target::TabOf(spread)).unwrap_err();
    assert_eq!(spread_err.by(), TargetBy::Agent);
    let LocateError::AgentAmbiguous { places, .. } = &spread_err else {
        panic!("두 탭이면 모호하다: {spread_err}");
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
        "창 번호 순 → 탭 줄 순"
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
        "떠 있는 에이전트 — 겹침 없이 · 이름이 없으면 id"
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
        panic!("없는 에이전트");
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
        "포커스 칸 기준 방향"
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
        "숨은 탭이 아니라 보이는 탭에서 푼다"
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

// ── target version: interjection — resolving and applying are the same lock ──

// ★This test is sequential — it cannot catch the regression of 「한 락에서 풀고 다른 락에서 옮기는」 (resolving in one lock and moving in another)★. That beside-area · tab-row docking does the resolving and
//   the moving in one lock is kept by the structure — the closure that resolves the target receives the model (`&ViewManager`) of the lock the apply function took only as an argument and
//   cannot be called outside that lock (`dock_in_place` of `apply.rs`). What really races between locks is only new-window
//   docking, which takes the lock twice (the two tests below — window port · window move interjection). No test-only interjection door was put in production code — where that door is placed is
//   exactly the gap the test measures, so if regressed code moves that door too, it would not be caught.
#[test]
fn a_tab_the_agent_left_after_its_name_was_resolved_is_never_moved() {
    let w = World::new();
    let (label, tabs) = w.popup_with(2);
    let slot = w.empty_slot(tabs[0]);
    // After the daemon resolved the name to this id · before applying, another write changed that area to another agent. If the path applied with the resolved tab id,
    //   it would move tabs[0], which that agent has left.
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
    assert_eq!(w.tabs_of(&label), tabs, "그 에이전트가 떠난 탭은 제자리");
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
        assert!(!w.window_exists(&opened[0]), "연 창은 모델에 들지 않는다");
        let journal = w.journal.take();
        match &spare {
            Some(spare) => {
                assert!(journal.contains(&format!("release:{spare}")), "{journal:?}");
                assert!(!w.claim_outstanding());
            }
            None => assert_eq!(
                &*w.host.closed.lock().unwrap(),
                &opened,
                "방금 연 창을 거둔다"
            ),
        }
        let want = if closes {
            vec![main]
        } else {
            vec![main, first]
        };
        assert_eq!(w.tabs_of(MAIN_WINDOW_LABEL), want, "아무것도 옮기지 않았다");
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
    assert!(w.host.opened_at.lock().unwrap().is_empty(), "새 창 없음");
    assert_eq!(w.tabs_of(&label), [lone], "재사용한 창은 그대로");
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
        "새 창의 탭 + main 에서 새로 보이게 된 탭"
    );
    let opened = w.host.opened_at.lock().unwrap().clone();
    assert_eq!(
        opened,
        [(out.window.clone(), NO_PLACE, false)],
        "활성 없이 연다"
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

    // Same window · source first: [s, a, x] → [a, s, x] (counted in the list without the source — no off-by-one)
    let out = w
        .dock_t(Target::Tab(s), after(agent), FocusPolicy::Keep)
        .unwrap();
    assert!(out.changed);
    assert_eq!(out.window, MAIN_WINDOW_LABEL);
    assert_eq!(w.tabs_of(MAIN_WINDOW_LABEL), [a, s, x]);

    // Same window · source after: [a, s, x] → [a, x, s]
    w.dock_t(Target::Tab(x), after(agent), FocusPolicy::Keep)
        .unwrap();
    assert_eq!(w.tabs_of(MAIN_WINDOW_LABEL), [a, x, s]);

    // Already at that position
    let out = w
        .dock_t(Target::Tab(x), after(agent), FocusPolicy::Keep)
        .unwrap();
    assert!(!out.changed);

    // Another window — goes to that tab's window
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
        "제 뒤에 넣기"
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
        "id 판 — 그 창의 탭이 아닌 기준"
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
    assert!(!out.changed, "같은 창 같은 자리");
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
    assert!(out.reused && !out.changed, "이미 혼자인 팝아웃");
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
    // Splits the target tab left/right first so the area frame (left half) and the whole canvas give different shares — beside an area it is the area frame, and for the whole tab
    //   whether it was counted by the canvas is told apart. Original size = source window canvas (width 200).
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

        assert_eq!(out.tab, target_tab, "결과 탭 = 살아남은 대상 탭");
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
        "칸 쓰기에 탭을 짚는 대상"
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
        "창을 주면 그 창의 탭이어야 한다"
    );

    apply::rename_tab_at(&w.state, &w.ev, &Target::TabOf(agent), "작업".into()).unwrap();
    let names = apply::list_tabs(&w.state, MAIN_WINDOW_LABEL).unwrap();
    assert_eq!(names.tabs[0].name, "작업");

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
    assert_eq!(closed.out, Some(t), "그 창에 이제 보이는 탭");
    assert_eq!(w.tabs_of(MAIN_WINDOW_LABEL), [t]);

    let only = w.active_of(&label);
    let closed =
        apply::close_tab_at(&w.state, &w.subs, &w.ev, &w.host, None, &Target::Tab(only)).unwrap();
    assert_eq!(closed.out, None, "마지막 탭이면 창째 닫힌다");
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
        "창을 열지 않는다"
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
        assert_eq!(never.calls(), 0, "{token:?} {window}: 스폰하지 않는다");
        assert!(matches!(err, apply::SpawnAtError::Unplaced(_)), "{err}");
        assert!(err.to_string().contains("스폰 안 함"), "{err}");
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
    assert_eq!(raced.calls(), 1, "스폰은 됐다");
    assert!(
        matches!(err, apply::SpawnAtError::Failed(_)) && err.to_string().contains("살아있음"),
        "점유 칸 — 살아 있는 id 를 남긴다: {err}"
    );
    assert_eq!(
        w.content(main, left),
        SlotContent::Agent {
            agent_id: filler.to_string()
        },
        "채운 칸을 덮지 않는다"
    );
}

// A future the command handler waits for across threads — if a lock guard is held across an await, compilation stops here.
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
        "창 번호 → 탭 줄 → 트리 전위 · 좌우와 위아래가 좌표로 갈린다"
    );
    for row in &rows {
        let listed = apply::list_tabs(&w.state, &row.window).unwrap();
        let meta = &listed.tabs[row.tab_index as usize];
        assert_eq!(row.tab, meta.id);
        assert_eq!(row.tab_name, meta.name, "탭 이름 = 탭 줄의 보이는 이름");
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
        "준 탭만 · 순서는 모델 순"
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
        "대상 탭 + 출처 창에서 새로 보이게 된 탭"
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
    assert!(rows[1].y > rows[0].y, "아래에 붙었다");
    assert_eq!(
        out.rows, rows,
        "결말의 줄 = 그 탭들의 줄(끼어든 쓰기가 없으면)"
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
        "새 창 갈래 — 옮긴 탭 + 출처 창에 새로 보이게 된 탭의 줄(창 번호 순)"
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
        "재사용 갈래 — 그 탭의 줄"
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
    assert!(out.rows.is_empty(), "id 판은 줄을 싣지 않는다");
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

    assert_eq!(spawner.calls(), 1, "스폰은 됐다");
    assert!(
        matches!(err, apply::SpawnAtError::Failed(_)) && err.to_string().contains("살아있음"),
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
                "혼자인 팝아웃이 비어 모델에서 빠진다"
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
                "대상판 — 없는 창을 결말로 말하지 않는다"
            );
        } else {
            let out = w
                .dock(
                    lone,
                    DockDest::NewWindow { at: NO_PLACE },
                    FocusPolicy::Keep,
                )
                .unwrap();
            assert!(out.reused, "id 판은 그대로 — 성공 + 신원 없음");
            assert_eq!(out.window, label);
            assert_eq!(out.identity, None);
        }
        assert!(!w.window_exists(&label));
        assert_eq!(w.owner(lone).as_deref(), Some(MAIN_WINDOW_LABEL));
        assert!(
            w.host.placed.lock().unwrap().is_empty(),
            "닫힌 창은 적지 않는다"
        );
        assert_invariants(&w);
    }
}
