//! Prewarmed window state (TRD S21-slot-docking §3-9) — ties the pool ([`pool`] — a pure state machine) to a lock · condition variable · an injected first-paint gate
//! handle ([`PaintGate`]) · two deadlines ([`SpareSeams`]). The wire with that window's webview = [`wire`].
//!
//! It knows nothing of Tauri — building · closing windows · events · setting deadlines are passed in by the caller (`commands/spare.rs` · the window port ·
//! [`CommitPort`]). To the layout authority (`crate::layout` · ADR-0035) it writes nothing except the commit moving the measurements it held.
// ADR-0299

pub mod pool;
pub mod wire;

use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use engram_dashboard_base::sync;

use pool::{
    ClaimStep, CommitDecision, Gone, Listen, Measure, NoBuild, Opening, SpareMeasures, SparePool,
    TrayShow,
};
use wire::{WindowRole, WindowRoleReply};

use crate::layout::manager::WindowLabel;
use crate::layout::{
    apply, LabelSource, LayoutError, LayoutState, SpareClaim, UiMetrics, ViewManager,
};

/// Upper bound on waiting for a prewarmed window that is being built (I2) — beyond it, build a new one. The window being built holds the main thread, so even a new build stands behind it,
/// so it is set more generously than one build (≈ 160 ms). It is a judgment, not a measurement.
pub const BUILD_WAIT: Duration = Duration::from_secs(1);

/// Safety deadline for waiting for the rebuild of an adopted window to complete (I8) — the first-paint deadline runs from commit ③, so the main thread
/// queueing share is added on top of it. If it fires first, the window to be shown and the build (≈ 160 ms) overlap on the main thread (prewarm F2). The 1 s margin is a judgment.
pub const REVEAL_SAFETY: Duration =
    crate::first_paint::FALLBACK_REVEAL.saturating_add(Duration::from_secs(1));

/// Upper bound by which the handover ends a show-wait that even the safety deadline did not end — a net for when that deadline was not set or did not fire.
/// Twice is a judgment (it leaves room for the deadline job running late).
pub const REVEAL_BACKSTOP: Duration = REVEAL_SAFETY.saturating_mul(2);

/// First-paint gate handle (`first_paint` — I17 · I23 · I26). Passed in when the pool is made.
///
/// ★`hold_unarmed` · `cancel` are called while holding the pool lock★ — the implementation takes only its own leaf lock and does not call the pool (order =
/// layout → pool → first-paint gate). The handover state and the placement must change together in one lock section so that the UI hide in between finds what to withdraw,
/// and so that the withdrawal of a take-back does not erase the placement of the one that handed over again right after.
pub trait PaintGate: Send + Sync {
    /// Puts on that label a placement that only buffers — notifications · the deadline do not show that window (whether to show is decided by `reveal_if_held`).
    /// If a placement already exists, it is left as it is (it does not erase buffered notifications).
    fn hold_unarmed(&self, label: &str);
    /// Withdraws the placement of that label — a no-op if none.
    fn cancel(&self, label: &str);
    /// Commit ③ — if the placement of that label remains, places it in the waiting state and gives `true` (after that it receives only notifications carrying the adoption number `generation` ·
    /// `activate` = whether to choose activation after showing · `opened` = when that window was last opened (dropped) — the open → show part of the show log),
    /// and if none (the UI hide withdrew it) it does nothing and gives `false`. Called outside the pool lock · layout lock.
    fn reveal_if_held(
        &self,
        label: &str,
        generation: u32,
        activate: bool,
        opened: Option<Instant>,
    ) -> bool;
}

/// Seam passed in when the pool is made (I23) — tests pass short deadlines and a fake gate.
pub struct SpareSeams {
    pub gate: Arc<dyn PaintGate>,
    /// Upper bound on waiting for a prewarmed window being built ([`BUILD_WAIT`]).
    pub build_wait: Duration,
    /// Safety deadline for the rebuild to complete after the commit ([`REVEAL_SAFETY`]) — passed to [`CommitPort::reveal_deadline`].
    pub reveal_safety: Duration,
    /// Upper bound that ends a show-wait whose handover is not finished ([`REVEAL_BACKSTOP`]).
    pub reveal_backstop: Duration,
}

impl SpareSeams {
    pub fn new(gate: Arc<dyn PaintGate>) -> Self {
        Self {
            gate,
            build_wait: BUILD_WAIT,
            reveal_safety: REVEAL_SAFETY,
            reveal_backstop: REVEAL_BACKSTOP,
        }
    }
}

/// Window-side work after commit ③ ④ (shell adapter). ★All are called outside the layout lock · pool lock★ — `destroy` waits for the event loop and that
/// `Destroyed` handler takes the layout lock · pool lock.
pub trait CommitPort {
    /// ④ — emits `spare:adopted { generation }` to the webview of that label.
    fn adopted(&self, label: &str, generation: u32);
    /// Sets the safety deadline — after `after` calls [`SpareState::on_reveal_overdue`], and if its answer is true, kicks off a build.
    fn reveal_deadline(&self, label: &str, after: Duration);
    /// Closes that window (destroy). `Err` = could not close.
    fn destroy(&self, label: &str) -> Result<(), String>;
}

/// The answer of [`SpareState::claim`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimReply {
    /// The prewarmed window handed over — if `None`, the caller builds a new one with a new label.
    pub claim: Option<SpareClaim>,
    /// The caller kicks off building a prewarmed window (I16) — true only when `claim = None`.
    pub build: bool,
    /// Time spent waiting for a window being built (log `wait_ms`) — `None` if it did not wait.
    pub waited: Option<Duration>,
}

/// The answer of [`SpareState::answer_role`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RoleAnswer {
    pub reply: WindowRoleReply,
    /// It is a pool window that could not set up listening, so it was removed from the pool — the caller closes that window (outside the lock · outside a synchronous handler · I28).
    pub close: bool,
}

/// The result of [`SpareState::build`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpareBuild {
    /// Could not secure a place so it did not build — no number was received (I13).
    Refused(NoBuild),
    Failed {
        label: WindowLabel,
        error: String,
    },
    Built {
        label: WindowLabel,
    },
    /// Built, but meanwhile left the field (`Destroyed` · listening failure).
    Vanished {
        label: WindowLabel,
    },
}

/// The result of [`SpareState::commit`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommitOutcome {
    /// Adopted — `revealed` = ③ placed the remaining placement (`false` = the UI hide withdrew it · it becomes a model window while hidden).
    Adopted { revealed: bool },
    /// That label is not in the model so it was closed — its `Destroyed` empties the field and builds again.
    Closed,
    /// Could not close it either time so the field was emptied (I31) — the next handover builds.
    CloseFailed,
    /// A stale handover ticket — nothing was done (I27).
    Stale,
}

/// The stage where commit ① ② are done — after releasing the layout lock, [`Self::finish`] does ③ ④ (or closing).
///
/// ★The two halves (`commit_locked` · `finish`) are not given out of this module★ — the outside calls through [`SpareState::commit`] alone.
/// If the stage is dropped in between, `awaiting_reveal` stays set forever and prewarming is off for the rest of that run, or the field stops at closing and a hidden window is left.
#[must_use = "레이아웃 락을 놓은 뒤 finish 를 불러야 확정이 끝난다"]
#[derive(Debug)]
struct CommitStep {
    claim: SpareClaim,
    decision: CommitDecision,
    carried: Result<(), LayoutError>,
}

/// The prewarmed window state, one per shell.
///
/// ★Lock order = layout → pool → first-paint gate★ — the only thing taken while holding the pool lock is the first-paint gate's leaf lock ([`PaintGate`]).
/// While holding it, do not take the layout lock · emit · call windows · log. Taking the pool while holding the layout lock is fine (commit ① —
/// [`Self::commit`]).
pub struct SpareState {
    pool: Mutex<SparePool>,
    /// The field that was being built changed (built · build failed · vanished · shutdown) — wakes the waiting handovers.
    changed: Condvar,
    gate: Arc<dyn PaintGate>,
    build_wait: Duration,
    reveal_safety: Duration,
    reveal_backstop: Duration,
}

impl SpareState {
    pub fn new(seams: SpareSeams) -> Self {
        Self {
            pool: Mutex::new(SparePool::default()),
            changed: Condvar::new(),
            gate: seams.gate,
            build_wait: seams.build_wait,
            reveal_safety: seams.reveal_safety,
            reveal_backstop: seams.reveal_backstop,
        }
    }

    fn pool(&self) -> MutexGuard<'_, SparePool> {
        sync::lock(&self.pool)
    }

    /// Is it a pool window (being built · built · handed over · closing) — the windows that tray show · hide · `--hidden` and placement recording skip.
    pub fn is_spare(&self, label: &str) -> bool {
        self.pool().contains(label)
    }

    /// Labels of adopted windows waiting to be shown and their adoption numbers (I26).
    pub fn awaiting_reveal(&self) -> Option<(WindowLabel, u32)> {
        self.pool()
            .awaiting_reveal()
            .map(|(label, generation)| (label.to_owned(), generation))
    }

    /// If that label is an adopted window waiting to be shown, its adoption number — `window_role` puts it in the answer (I26).
    pub fn reveal_generation(&self, label: &str) -> Option<u32> {
        self.pool().reveal_generation(label)
    }

    /// The app is ending — nothing is handed over, built or rebuilt. Once set, it is not lowered.
    pub fn begin_exit(&self) {
        self.pool().begin_exit();
        self.changed.notify_all();
    }

    pub fn exiting(&self) -> bool {
        self.pool().exiting()
    }

    /// Role of window `label` — `Spare` if it is a pool window, otherwise `Model` · `Unknown` by `in_model`. `in_model` is called after the pool lock is
    /// released, so it may take the layout lock.
    pub fn window_role(&self, label: &str, in_model: impl FnOnce() -> bool) -> WindowRole {
        if self.is_spare(label) {
            WindowRole::Spare
        } else if in_model() {
            WindowRole::Model
        } else {
            WindowRole::Unknown
        }
    }

    /// The answer of `window_role{adoptable}` (I26 · I28) — if it is a pool window that could not set up listening for the adoption event, remove it from the pool ([`Self::on_listen`])
    /// and give `unknown` + close. Otherwise it is the role of [`Self::window_role`], and for a model window it carries the adoption number it is waiting to be shown with.
    /// `in_model` is called after the pool lock is released.
    pub fn answer_role(
        &self,
        label: &str,
        adoptable: bool,
        in_model: impl FnOnce() -> bool,
    ) -> RoleAnswer {
        if let Listen::Removed { .. } = self.on_listen(label, adoptable) {
            return RoleAnswer {
                reply: WindowRoleReply {
                    role: WindowRole::Unknown,
                    generation: None,
                },
                close: true,
            };
        }
        let role = self.window_role(label, in_model);
        // The number is read after looking at the role — a `spare` answer from before commit ① does not get a number from after ①.
        let generation = match role {
            WindowRole::Model => self.reveal_generation(label),
            WindowRole::Spare | WindowRole::Unknown => None,
        };
        RoleAnswer {
            reply: WindowRoleReply { role, generation },
            close: false,
        }
    }

    // ── measurement report (I6) ──

    /// Records the measurements a pool window sent — `None` = not a pool window (the caller records it in the model · meaning = [`SparePool::record`]).
    /// The pool lock is released before returning.
    pub fn record_measure(&self, label: &str, measure: Measure) -> Option<Result<(), LayoutError>> {
        self.pool().record(label, measure)
    }

    /// Canvas report of a window webview — recorded in the pool if it is a pool window, otherwise in the model. It looks at the pool first and takes the layout lock after releasing it, so
    /// a report that arrives after commit ① waits for that commit's layout lock and is recorded on top of the moved value.
    pub fn report_canvas(
        &self,
        layout: &LayoutState,
        label: &str,
        w: u32,
        h: u32,
    ) -> Result<(), String> {
        match self.record_measure(label, Measure::Canvas { w, h }) {
            Some(recorded) => recorded.map_err(|e| e.to_string()),
            None => apply::report_window_canvas(layout, label, w, h),
        }
    }

    /// The UI metrics counterpart of [`Self::report_canvas`].
    pub fn report_metrics(
        &self,
        layout: &LayoutState,
        label: &str,
        metrics: UiMetrics,
    ) -> Result<(), String> {
        match self.record_measure(label, Measure::Metrics(metrics)) {
            Some(recorded) => recorded.map_err(|e| e.to_string()),
            None => apply::report_ui_metrics(layout, label, metrics),
        }
    }

    // ── build (TRD §3-9 「부팅」 · 「다시 짓기」 · I13) ──

    /// Builds one prewarmed window — secures the field place first (if one already exists · is waiting to be shown · is shutting down · is off, it does not take a number
    /// and gives `Refused`), then takes a number from `labels` and builds hidden with `build`. Called outside the lock, and `build` runs outside the lock
    /// (window building — worker thread).
    // ADR-0299
    pub fn build(
        &self,
        labels: &dyn LabelSource,
        build: impl FnOnce(&str) -> Result<(), String>,
    ) -> SpareBuild {
        let started = Instant::now();
        let reserved = self.pool().reserve();
        let revealed_at = match reserved {
            Ok(at) => at,
            Err(reason) => {
                tracing::debug!(module = "spare", ?reason, "예열 창을 짓지 않는다");
                return SpareBuild::Refused(reason);
            }
        };
        let label = labels.next_label();
        if !self.pool().assign(label.clone()) {
            tracing::warn!(
                module = "spare",
                label,
                "잡은 짓기 자리가 번호를 받는 사이 바뀌었다 — 짓지 않는다"
            );
            return SpareBuild::Vanished { label };
        }
        if let Err(error) = build(&label) {
            if self.pool().build_failed(&label) {
                self.changed.notify_all();
            }
            tracing::warn!(module = "spare", label, "예열 창을 짓지 못했다: {error}");
            return SpareBuild::Failed { label, error };
        }
        let built = self.pool().mark_built(&label);
        self.changed.notify_all();
        let build_ms = elapsed_ms(started);
        if built {
            tracing::info!(
                module = "spare",
                label,
                build_ms,
                after_reveal_ms = ?revealed_at.map(elapsed_ms),
                "예열 창을 지었다"
            );
            SpareBuild::Built { label }
        } else {
            tracing::debug!(
                module = "spare",
                label,
                build_ms,
                "예열 창이 짓는 동안 사라졌다"
            );
            SpareBuild::Vanished { label }
        }
    }

    // ── handover · open · take-back (I2 · I4 · I16 · I17 · I27) ──

    /// Hands over the built prewarmed window — inside the same pool lock, puts on that label a first-paint placement that only buffers. If it is being built,
    /// waits until it is built (upper bound = `build_wait`) — built · build failed · vanished · shutdown wake it, and after waking only the
    /// first to take the lock takes it. If the build it was waiting for failed or vanished, it does not follow another build that took the place afterward and gives
    /// `None` right away. If a show-wait has stood past `reveal_backstop`, it first does that rebuild completion (after that the field is empty, so
    /// this handover kicks off a build). ★Call it outside the lock · outside the main thread★ (I15 — that build runs on the main thread).
    // ADR-0299
    pub fn claim(&self) -> ClaimReply {
        let started = Instant::now();
        let deadline = started + self.build_wait;
        let mut waited = false;
        let mut awaited = None;
        let mut pool = self.pool();
        let expired = pool.expire_reveal(started, self.reveal_backstop);
        let (claim, build, timed_out) = loop {
            match pool.take_built(awaited) {
                ClaimStep::Claimed(claim) => {
                    self.gate.hold_unarmed(&claim.label);
                    break (Some(claim), false, false);
                }
                ClaimStep::Fresh { build } => break (None, build, false),
                ClaimStep::Wait { build } => {
                    let left = deadline.saturating_duration_since(Instant::now());
                    if left.is_zero() {
                        break (None, false, true);
                    }
                    waited = true;
                    awaited = Some(build);
                    pool.park();
                    pool = sync::wait_timeout(&self.changed, pool, left).0;
                    pool.unpark();
                }
            }
        };
        drop(pool);
        if let Some((label, generation)) = expired {
            tracing::warn!(
                module = "spare",
                label,
                generation,
                backstop_ms = u64::try_from(self.reveal_backstop.as_millis()).unwrap_or(u64::MAX),
                "채택한 창의 보임 기다림이 안전 시한을 넘겨도 끝나지 않아 맡기기가 다시 짓기 완료로 쳤다"
            );
        }
        let waited = waited.then(|| started.elapsed());
        if timed_out {
            tracing::warn!(
                module = "spare",
                wait_ms = elapsed_ms(started),
                "짓는 중인 예열 창이 시한 안에 지어지지 않았다 — 새로 짓는다"
            );
        }
        ClaimReply {
            claim,
            build,
            waited,
        }
    }

    /// When opening (dropping) a handed-over label — if it is a window handed over now, write `activate` into that handover and give `Claimed` (drop it without building). If it vanished while handed over
    /// or is a prewarmed window label that was not handed over, `Unavailable` (`Err` without building anew — I20), and if unrelated to the pool, `Fresh`.
    pub fn opening(&self, label: &str, activate: bool) -> Opening {
        self.pool().opening(label, activate, Instant::now())
    }

    /// Take-back before commit — only if the claim ticket is the current handover, return it to the field hidden and withdraw the placement inside the same pool lock (`true`).
    /// If it is a stale claim ticket, no-op + warn (`false`).
    pub fn release(&self, claim: &SpareClaim) -> bool {
        let released = {
            let mut pool = self.pool();
            let released = pool.release(claim);
            if released {
                self.gate.cancel(&claim.label);
            }
            released
        };
        if released {
            tracing::debug!(
                module = "spare",
                label = claim.label,
                generation = claim.generation,
                "맡긴 예열 창을 돌려받았다"
            );
        } else {
            tracing::warn!(
                module = "spare",
                label = claim.label,
                generation = claim.generation,
                "지금의 맡김이 아닌 맡김표의 되물림 — 무동작"
            );
        }
        released
    }

    // ── commit (TRD §3-9 「`committed`」 · I6 · I21 · I25 · I27 · I31) ──

    /// Once, after the new window entry of the handed-over window is in the model — does ① ② inside one layout lock, and after releasing the lock does ③ ④ or closing.
    /// Called outside the lock.
    pub fn commit(
        &self,
        layout: &LayoutState,
        claim: &SpareClaim,
        port: &dyn CommitPort,
    ) -> CommitOutcome {
        let step = {
            let mut mgr = sync::lock(&layout.0);
            self.commit_locked(&mut mgr, claim)
        };
        step.finish(self, port)
    }

    /// Commit ① ② — `mgr` = the layout lock the caller holds. ① If the claim ticket is the current handover, empties the field and sets `awaiting_reveal` inside the same pool lock
    /// (I25), and if that label is not in the model, does not empty but decides to close and withdraws the placement inside the same pool lock
    /// (I27). ② Records the measurements it held into the model inside the same layout lock — a report that arrives during the commit waits for that lock and is recorded on top of this value
    /// (I6).
    // ADR-0299
    fn commit_locked(&self, mgr: &mut ViewManager, claim: &SpareClaim) -> CommitStep {
        let in_model = mgr.window_attrs(&claim.label).is_ok();
        let decision = {
            let mut pool = self.pool();
            let decision = pool.commit(claim, in_model, Instant::now());
            if decision == CommitDecision::Close {
                self.gate.cancel(&claim.label);
            }
            decision
        };
        let carried = match &decision {
            CommitDecision::Adopt { measures, .. } => carry_measures(mgr, &claim.label, measures),
            CommitDecision::Close | CommitDecision::Stale => Ok(()),
        };
        CommitStep {
            claim: claim.clone(),
            decision,
            carried,
        }
    }

    fn close_failed(&self, claim: &SpareClaim) -> bool {
        self.pool().close_failed(claim)
    }

    // ── rebuild completion (I8 · I25) · `Destroyed` ──

    /// The adopted window was shown (first paint released or its deadline) — `true` = the caller kicks off a build. Once per label (`false` if another completion of that
    /// label — vanishing · the safety deadline — came first). `false` if the label is not a window waiting to be shown.
    pub fn on_shown(&self, label: &str) -> bool {
        self.complete_reveal(label, "shown")
    }

    /// The safety deadline fired — the meaning is the same as [`Self::on_shown`].
    pub fn on_reveal_overdue(&self, label: &str) -> bool {
        self.complete_reveal(label, "overdue")
    }

    fn complete_reveal(&self, label: &str, cause: &'static str) -> bool {
        let done = self.pool().complete_reveal(label, Instant::now());
        if let Some(build) = done {
            tracing::debug!(
                module = "spare",
                label,
                cause,
                build,
                "채택한 창의 보임 기다림이 끝났다"
            );
        }
        done.unwrap_or(false)
    }

    /// The window vanished (`Destroyed` — all windows) — if it is the field's window, empties the field (if handed over, withdraws the placement inside the same pool lock)
    /// and gives `true` (rebuild) unless shutting down · off. If it was an adopted window waiting to be shown, it is a rebuild completion. Otherwise `false`.
    pub fn on_destroyed(&self, label: &str) -> bool {
        let gone = {
            let mut pool = self.pool();
            let gone = pool.destroyed(label, Instant::now());
            if let Gone::Removed { claimed: true, .. } = gone {
                self.gate.cancel(label);
            }
            gone
        };
        match gone {
            Gone::NotOurs => false,
            Gone::Removed {
                claimed,
                closing,
                rebuild,
            } => {
                self.changed.notify_all();
                if closing {
                    tracing::debug!(
                        module = "spare",
                        label,
                        rebuild,
                        "확정이 닫기로 정한 예열 창이 닫혀 풀에서 뺐다"
                    );
                } else if rebuild {
                    tracing::warn!(
                        module = "spare",
                        label,
                        claimed,
                        "예열 창이 사라져 풀에서 뺐다 — 다시 짓는다"
                    );
                } else {
                    tracing::debug!(
                        module = "spare",
                        label,
                        claimed,
                        "예열 창이 사라져 풀에서 뺐다 — 종료 중이거나 꺼져 다시 짓지 않는다"
                    );
                }
                rebuild
            }
            Gone::RevealDone { build } => {
                tracing::debug!(
                    module = "spare",
                    label,
                    cause = "destroyed",
                    build,
                    "채택한 창의 보임 기다림이 끝났다"
                );
                build
            }
        }
    }

    // ── listening failure (I28) · tray show ──

    /// The pool part of `window_role{adoptable}` — if a pool window could not set up listening, empties the field (if handed over, withdraws the placement inside the same pool lock
    /// · its `Destroyed` does not rebuild). The caller closes that window outside the lock. If it happens
    /// [`pool::LISTEN_FAILURE_LIMIT`] times in a row, the pool is turned off for the whole run. If a pool window did set up listening, the count is reset to 0.
    pub fn on_listen(&self, label: &str, adoptable: bool) -> Listen {
        let listen = {
            let mut pool = self.pool();
            let listen = pool.listen(label, adoptable);
            if let Listen::Removed { claimed: true, .. } = listen {
                self.gate.cancel(label);
            }
            listen
        };
        if let Listen::Removed {
            claimed,
            disabled_now,
        } = listen
        {
            self.changed.notify_all();
            tracing::warn!(
                module = "spare",
                label,
                claimed,
                "예열 창이 채택 사건 듣기를 못 걸어 풀에서 뺐다 — 닫고 곧바로 다시 짓지 않는다"
            );
            if disabled_now {
                tracing::warn!(
                    module = "spare",
                    failures = pool::LISTEN_FAILURE_LIMIT,
                    "예열 창 듣기 실패가 잇달아 일어나 이 실행 내내 예열 창을 쓰지 않는다"
                );
            }
        }
        listen
    }

    /// For each prewarmed window label skipped by the predicate when the hidden UI is shown again — if handed over, sets the placement again inside the same pool lock.
    pub fn on_tray_show(&self, label: &str) -> TrayShow {
        let pool = self.pool();
        let show = pool.tray_show(label);
        if show == TrayShow::Rehold {
            self.gate.hold_unarmed(label);
        }
        show
    }
}

impl CommitStep {
    /// Commit ③ ④ or closing — ★called after releasing the layout lock · pool lock★. For adoption, decides ③ to place the remaining placement, ④ emits the adoption
    /// event and sets the safety deadline. For closing, closes, and if it cannot close, once more, and if it still cannot, empties the field (I31).
    fn finish(self, spare: &SpareState, port: &dyn CommitPort) -> CommitOutcome {
        let Self {
            claim,
            decision,
            carried,
        } = self;
        let label = claim.label.as_str();
        let generation = claim.generation;
        match decision {
            CommitDecision::Stale => {
                tracing::warn!(
                    module = "spare",
                    label,
                    generation,
                    "지금의 맡김이 아닌 맡김표의 확정 — 무동작"
                );
                CommitOutcome::Stale
            }
            CommitDecision::Adopt {
                activate,
                opened_at,
                ..
            } => {
                if let Err(e) = carried {
                    tracing::warn!(
                        module = "spare",
                        label,
                        "예열 창이 쥐던 재기를 모델에 옮기지 못했다: {e}"
                    );
                }
                let revealed = spare
                    .gate
                    .reveal_if_held(label, generation, activate, opened_at);
                port.adopted(label, generation);
                port.reveal_deadline(label, spare.reveal_safety);
                tracing::info!(
                    module = "spare",
                    label,
                    generation,
                    since_open_ms = ?opened_at.map(elapsed_ms),
                    revealed,
                    "맡긴 예열 창을 채택했다(committed)"
                );
                CommitOutcome::Adopted { revealed }
            }
            CommitDecision::Close => {
                tracing::warn!(
                    module = "spare",
                    label,
                    generation,
                    "확정 때 모델에 그 창이 없다 — 돌려받지 않고 닫는다"
                );
                let Err(first) = port.destroy(label) else {
                    return CommitOutcome::Closed;
                };
                let Err(second) = port.destroy(label) else {
                    tracing::warn!(
                        module = "spare",
                        label,
                        "예열 창을 한 번 못 닫아 다시 닫았다: {first}"
                    );
                    return CommitOutcome::Closed;
                };
                let freed = spare.close_failed(&claim);
                tracing::warn!(
                    module = "spare",
                    label,
                    freed,
                    "예열 창을 두 번 다 못 닫아 칸을 비운다 — 다음 맡기기가 짓는다: {first} / {second}"
                );
                CommitOutcome::CloseFailed
            }
        }
    }
}

// A field with no model value is not touched — a new window entry carries no measurements, and if the pool has no value there is nothing to leave.
fn carry_measures(
    mgr: &mut ViewManager,
    label: &str,
    measures: &SpareMeasures,
) -> Result<(), LayoutError> {
    if let Some(canvas) = measures.canvas {
        mgr.set_window_canvas(label, canvas.w, canvas.h)?;
    }
    if let Some(metrics) = measures.metrics {
        mgr.set_ui_metrics(label, metrics)?;
    }
    Ok(())
}

pub fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests;
