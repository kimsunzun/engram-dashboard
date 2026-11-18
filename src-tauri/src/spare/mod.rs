//! Prewarmed window state (TRD S21-slot-docking §3-9) — ties the pool ([`pool`], a pure state machine) to a lock · condition variable · an injected first-paint gate ([`PaintGate`]) · two deadlines ([`SpareSeams`]). The wire with that window's webview = [`wire`]. Knows nothing of Tauri — building · closing windows · events · deadline setting are passed in by the caller (`commands/spare.rs` · the window port · [`CommitPort`]); to the layout authority (`crate::layout` · ADR-0035) it writes only the commit moving the measurements it held.
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

/// Upper bound on waiting for a prewarmed window being built (I2) — beyond it, build a new one. The window being built holds the main thread so even a new build queues behind it, hence more generous than one build (≈160 ms). A judgment, not a measurement.
pub const BUILD_WAIT: Duration = Duration::from_secs(1);

/// Safety deadline for an adopted window's rebuild to complete (I8) — the first-paint deadline runs from commit ③, so the main-thread queueing share is added; firing first would overlap the shown window and the build (≈160 ms) on the main thread (prewarm F2). The 1 s margin is a judgment.
pub const REVEAL_SAFETY: Duration =
    crate::first_paint::FALLBACK_REVEAL.saturating_add(Duration::from_secs(1));

/// Upper bound by which the handover ends a show-wait the safety deadline did not end — a net for that deadline not set or not firing; twice is a judgment (room for the deadline job running late).
pub const REVEAL_BACKSTOP: Duration = REVEAL_SAFETY.saturating_mul(2);

/// First-paint gate handle (`first_paint` — I17 · I23 · I26), passed in when the pool is made. `hold_unarmed` · `cancel` are called holding the pool lock — the implementation takes only its own leaf lock and never calls the pool (order layout → pool → first-paint gate). Handover state and placement must change in one lock section so the UI hide in between finds what to withdraw, and a take-back's withdrawal does not erase the placement of one handed over again right after.
pub trait PaintGate: Send + Sync {
    /// Puts on that label a placement that only buffers — notifications · the deadline do not show it (`reveal_if_held` decides). An existing placement is left as is (buffered notifications kept).
    fn hold_unarmed(&self, label: &str);
    fn cancel(&self, label: &str);
    /// Commit ③ — if that label's placement remains, places it in the waiting state and gives `true` (afterwards it takes only notifications carrying the adoption number `generation`; `activate` = whether to activate on show; `opened` = when the window was last opened (dropped) — the open → show part of the show log); if none (the UI hide withdrew it) does nothing and gives `false`. Called outside the pool lock · layout lock.
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
    pub build_wait: Duration,
    pub reveal_safety: Duration,
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

/// Window-side work after commit ③ ④ (shell adapter). All called outside the layout lock · pool lock — `destroy` waits for the event loop and that `Destroyed` handler takes the layout lock · pool lock.
pub trait CommitPort {
    fn adopted(&self, label: &str, generation: u32);
    /// Sets the safety deadline: after `after`, calls [`SpareState::on_reveal_overdue`] and kicks off a build if it answers true.
    fn reveal_deadline(&self, label: &str, after: Duration);
    fn destroy(&self, label: &str) -> Result<(), String>;
}

/// The answer of [`SpareState::claim`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimReply {
    pub claim: Option<SpareClaim>,
    pub build: bool,
    pub waited: Option<Duration>,
}

/// The answer of [`SpareState::answer_role`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RoleAnswer {
    pub reply: WindowRoleReply,
    pub close: bool,
}

/// The result of [`SpareState::build`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpareBuild {
    Refused(NoBuild),
    Failed { label: WindowLabel, error: String },
    Built { label: WindowLabel },
    Vanished { label: WindowLabel },
}

/// The result of [`SpareState::commit`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommitOutcome {
    Adopted { revealed: bool },
    Closed,
    CloseFailed,
    Stale,
}

/// The stage after commit ① ② — after releasing the layout lock [`Self::finish`] does ③ ④ (or closing). The two halves (`commit_locked` · `finish`) are not given out of this module; the outside calls only [`SpareState::commit`]. Dropped in between, `awaiting_reveal` stays set forever (prewarming off for the run) or the field stops at closing leaving a hidden window.
#[must_use = "call finish after releasing the layout lock to complete the confirmation"]
#[derive(Debug)]
struct CommitStep {
    claim: SpareClaim,
    decision: CommitDecision,
    carried: Result<(), LayoutError>,
}

/// The prewarmed window state, one per shell. Lock order = layout → pool → first-paint gate — the only thing taken under the pool lock is the first-paint gate's leaf lock ([`PaintGate`]); under it do not take the layout lock · emit · call windows · log. Taking the pool while holding the layout lock is fine (commit ① — [`Self::commit`]).
pub struct SpareState {
    pool: Mutex<SparePool>,
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

    pub fn is_spare(&self, label: &str) -> bool {
        self.pool().contains(label)
    }

    pub fn awaiting_reveal(&self) -> Option<(WindowLabel, u32)> {
        self.pool()
            .awaiting_reveal()
            .map(|(label, generation)| (label.to_owned(), generation))
    }

    pub fn reveal_generation(&self, label: &str) -> Option<u32> {
        self.pool().reveal_generation(label)
    }

    pub fn begin_exit(&self) {
        self.pool().begin_exit();
        self.changed.notify_all();
    }

    pub fn exiting(&self) -> bool {
        self.pool().exiting()
    }

    /// Role of window `label` — `Spare` for a pool window, else `Model` · `Unknown` by `in_model` (called after the pool lock is released, so it may take the layout lock).
    pub fn window_role(&self, label: &str, in_model: impl FnOnce() -> bool) -> WindowRole {
        if self.is_spare(label) {
            WindowRole::Spare
        } else if in_model() {
            WindowRole::Model
        } else {
            WindowRole::Unknown
        }
    }

    /// The answer of `window_role{adoptable}` (I26 · I28): a pool window that could not set up listening for the adoption event is removed from the pool ([`Self::on_listen`]) and answered `unknown` + close; otherwise the role of [`Self::window_role`], and a model window carries the adoption number it waits to be shown with. `in_model` is called after the pool lock is released.
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

    /// Records the measurements a pool window sent — `None` = not a pool window (the caller records in the model; meaning = [`SparePool::record`]). The pool lock is released before returning.
    pub fn record_measure(&self, label: &str, measure: Measure) -> Option<Result<(), LayoutError>> {
        self.pool().record(label, measure)
    }

    /// Canvas report of a window webview — recorded in the pool for a pool window, else in the model. The pool is checked first and the layout lock taken after releasing it, so a report arriving after commit ① waits for that commit's layout lock and is recorded on top of the moved value.
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

    // ── build (TRD §3-9 「boot」 · 「rebuild」 · I13) ──

    /// Builds one prewarmed window — secures the field place first (one already existing · awaiting show · shutting down · off → no number, `Refused`), then takes a number from `labels` and builds hidden with `build`. Called outside the lock, and `build` runs outside it (window building — worker thread).
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
                tracing::debug!(module = "spare", ?reason, "not building a prewarmed window");
                return SpareBuild::Refused(reason);
            }
        };
        let label = labels.next_label();
        if !self.pool().assign(label.clone()) {
            tracing::warn!(
                module = "spare",
                label,
                "the claimed build place changed while receiving a number — not building"
            );
            return SpareBuild::Vanished { label };
        }
        if let Err(error) = build(&label) {
            if self.pool().build_failed(&label) {
                self.changed.notify_all();
            }
            tracing::warn!(
                module = "spare",
                label,
                "could not build the prewarmed window: {error}"
            );
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
                "prewarmed window built"
            );
            SpareBuild::Built { label }
        } else {
            tracing::debug!(
                module = "spare",
                label,
                build_ms,
                "the prewarmed window vanished while being built"
            );
            SpareBuild::Vanished { label }
        }
    }

    // ── handover · open · take-back (I2 · I4 · I16 · I17 · I27) ──

    /// Hands over the built prewarmed window — in the same pool lock puts a buffering-only first-paint placement on that label. If being built, waits until built (upper bound `build_wait`) — built · build failed · vanished · shutdown wake it and only the first to take the lock takes it; if the awaited build failed or vanished it does not follow another build that took the place and gives `None` at once. A show-wait standing past `reveal_backstop` first gets that rebuild completion (the field is then empty, so this handover kicks off a build). Call outside the lock and off the main thread (I15 — that build runs on the main thread).
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
                "the adopted window's wait to be shown did not end even past the safety deadline, so the entrust counted it as rebuild complete"
            );
        }
        let waited = waited.then(|| started.elapsed());
        if timed_out {
            tracing::warn!(
                module = "spare",
                wait_ms = elapsed_ms(started),
                "the prewarmed window being built was not finished within the deadline — building a new one"
            );
        }
        ClaimReply {
            claim,
            build,
            waited,
        }
    }

    /// Opening (dropping) a handed-over label — a window handed over now: writes `activate` into that handover and gives `Claimed` (dropped without building); vanished while handed over, or a prewarmed label not handed over: `Unavailable` (`Err` without building anew — I20); unrelated to the pool: `Fresh`.
    pub fn opening(&self, label: &str, activate: bool) -> Opening {
        self.pool().opening(label, activate, Instant::now())
    }

    /// Take-back before commit — only if the claim ticket is the current handover, returns it to the field hidden and withdraws the placement in the same pool lock (`true`); a stale ticket is a no-op + warn (`false`).
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
                "took back the handed-over prewarmed window"
            );
        } else {
            tracing::warn!(
                module = "spare",
                label = claim.label,
                generation = claim.generation,
                "hand-back of an entrust ticket that is not the current entrust — no-op"
            );
        }
        released
    }

    // ── commit (TRD §3-9 「`committed`」 · I6 · I21 · I25 · I27 · I31) ──

    /// Once, after the handed-over window's new entry is in the model — does ① ② inside one layout lock and after releasing it ③ ④ or closing. Called outside the lock.
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

    /// Commit ① ② — `mgr` = the layout lock the caller holds. ① If the claim ticket is the current handover, empties the field and sets `awaiting_reveal` in the same pool lock (I25); if that label is not in the model, instead of emptying decides to close and withdraws the placement in the same pool lock (I27). ② Records the held measurements into the model inside the same layout lock — a report arriving during the commit waits for it and lands on top (I6).
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

    /// The adopted window was shown (first paint released or its deadline) — `true` = the caller kicks off a build. Once per label (`false` if another completion — vanishing · the safety deadline — came first, or the label is not awaiting show).
    pub fn on_shown(&self, label: &str) -> bool {
        self.complete_reveal(label, "shown")
    }

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
                "the adopted window's wait to be shown ended"
            );
        }
        done.unwrap_or(false)
    }

    /// The window vanished (`Destroyed` — all windows): if it is the field's window, empties the field (withdrawing the placement in the same pool lock if handed over) and gives `true` (rebuild) unless shutting down · off; an adopted window awaiting show counts as a rebuild completion; otherwise `false`.
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
                        "the prewarmed window that the confirmation decided to close was closed and removed from the pool"
                    );
                } else if rebuild {
                    tracing::warn!(
                        module = "spare",
                        label,
                        claimed,
                        "the prewarmed window vanished and was removed from the pool — rebuilding"
                    );
                } else {
                    tracing::debug!(
                        module = "spare",
                        label,
                        claimed,
                        "the prewarmed window vanished and was removed from the pool — shutting down or disabled, so not rebuilding"
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
                    "the adopted window's wait to be shown ended"
                );
                build
            }
        }
    }

    // ── listening failure (I28) · tray show ──

    /// The pool part of `window_role{adoptable}` — if a pool window could not set up listening, empties the field (withdrawing the placement in the same pool lock if handed over; its `Destroyed` does not rebuild); the caller closes the window outside the lock. [`pool::LISTEN_FAILURE_LIMIT`] failures in a row turn the pool off for the run; a pool window that did set up listening resets the count to 0.
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
                "the prewarmed window could not attach its adoption-event listener and was removed from the pool — closing it and not rebuilding right away"
            );
            if disabled_now {
                tracing::warn!(
                    module = "spare",
                    failures = pool::LISTEN_FAILURE_LIMIT,
                    "listener failures on prewarmed windows repeated, so prewarmed windows are not used for the rest of this run"
                );
            }
        }
        listen
    }

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
    /// Commit ③ ④ or closing — called after releasing the layout lock · pool lock. Adoption: decides ③ (place the remaining placement), ④ emits the adoption event and sets the safety deadline. Closing: closes, once more on failure, and if still failing empties the field (I31).
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
                    "confirmation of an entrust ticket that is not the current entrust — no-op"
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
                        "could not carry over to the model what the prewarmed window held: {e}"
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
                    "adopted the handed-over prewarmed window (committed)"
                );
                CommitOutcome::Adopted { revealed }
            }
            CommitDecision::Close => {
                tracing::warn!(
                    module = "spare",
                    label,
                    generation,
                    "at confirmation the model has no such window — closing it without taking it back"
                );
                let Err(first) = port.destroy(label) else {
                    return CommitOutcome::Closed;
                };
                let Err(second) = port.destroy(label) else {
                    tracing::warn!(
                        module = "spare",
                        label,
                        "could not close the prewarmed window once; closed it again: {first}"
                    );
                    return CommitOutcome::Closed;
                };
                let freed = spare.close_failed(&claim);
                tracing::warn!(
                    module = "spare",
                    label,
                    freed,
                    "could not close the prewarmed window either time, so emptying the field — the next entrust builds it: {first} / {second}"
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
