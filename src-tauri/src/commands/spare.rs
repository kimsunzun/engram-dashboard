//! The shell-side window management of prewarmed windows (TRD S21-slot-docking §3-9): the window role query ([`window_role`]) · the window port's claim · commit ·
//! release share ([`claim`] · [`commit`] · [`release`] — window placing is the window port of `commands::popout`) · rebuild completion (the entrusted window's
//! showing · window `Destroyed` · safety deadline) · building prewarmed windows · the app-exit mark ([`on_app_exiting`]). The state machine is in
//! `crate::spare` and this is the Tauri adapter.
//!
//! ★Lock order = layout → prewarmed window pool → first-paint gate★ (`SpareState` docs) — the only thing taken while holding the pool lock is the first-paint gate's leaf lock,
//! and while holding the pool lock, do not take the layout lock · emit · call a window · build a window. All window calls · emits in this file are outside both locks.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use tauri::{AppHandle, Emitter, Manager, State, Window};

use crate::commands::popout::PopupCounter;
use crate::layout::apply;
use crate::layout::{LayoutState, SpareClaim};
pub(crate) use crate::spare::elapsed_ms;
use crate::spare::pool::Opening;
use crate::spare::wire::{SpareAdopted, WindowRoleReply, EVT_SPARE_ADOPTED};
use crate::spare::{CommitPort, SpareState};

/// The current role of the calling window (synchronous) — `invoke('window_role', { adoptable })`. The webview, at mount, sets up listening for `spare:adopted`, then
/// asks, and if `spare` draws in waiting mode (TRD §3-9). `adoptable` = that listening was set up — a pool window with `false` is taken out of the pool and closed and
/// is `unknown` (I28). If it is a window waiting to be shown after adoption, it also carries that adoption number (I26). It is wiring inside the webview, so it is not put in the bus
/// command table.
// ADR-0299
#[tauri::command]
pub fn window_role(
    app: AppHandle,
    window: Window,
    adoptable: bool,
    spare: State<'_, Arc<SpareState>>,
    layout: State<'_, LayoutState>,
) -> WindowRoleReply {
    let label = window.label();
    let answer = spare.answer_role(label, adoptable, || {
        apply::list_windows(&layout).is_ok_and(|windows| windows.iter().any(|w| w == label))
    });
    if answer.close {
        // A synchronous command runs on the main thread — the calling webview is not withdrawn inside its IPC handler but handed to a worker thread. It is not rebuilt
        //   immediately — it was taken out of the pool first, so its `Destroyed` cannot find a pool entry (the next claim builds it · I16).
        let label = label.to_owned();
        tauri::async_runtime::spawn_blocking(move || {
            crate::commands::popout::destroy_window(&app, &label)
        });
    }
    answer.reply
}

/// Is it a window of the prewarmed window pool — a window that tray show · hide · `--hidden` and place recording exclude (TRD §3-9).
pub(crate) fn is_spare_window(app: &AppHandle, label: &str) -> bool {
    app.try_state::<Arc<SpareState>>()
        .is_some_and(|spare| spare.is_spare(label))
}

/// The pool branch of window `Destroyed` (TRD §3-9 「다시 짓기」) — if it is a pool window, take it out of the pool and rebuild; if it is an adoption window that was waiting to be shown,
/// it is rebuild completion (both: not built while exiting · pool off · decision = `SpareState::on_destroyed`). This is the only place that
/// erases the pool entry of a closing window. Called outside the layout lock (`lib.rs`).
// ADR-0295 · ADR-0299
pub(crate) fn on_spare_window_destroyed(app: &AppHandle, label: &str) {
    let Some(spare) = app.try_state::<Arc<SpareState>>() else {
        return;
    };
    if spare.on_destroyed(label) {
        spawn_spare_build(app);
    }
}

/// The entrusted prewarmed window became visible at first paint (the notification or its deadline — the entrusted-window show of `first_paint`) — that is rebuild completion. Called
/// on the main thread (building is handed to a worker thread).
// ADR-0299
pub(crate) fn on_adopted_window_shown(app: &AppHandle, label: &str) {
    if app
        .try_state::<Arc<SpareState>>()
        .is_some_and(|spare| spare.on_shown(label))
    {
        spawn_spare_build(app);
    }
}

// ── the window port's prewarmed-window share (TRD §3-9 「포트」 · 「맡기기」 · 「확정」) ─────────────────────────────

/// The real implementation of window port `claim_spare` — entrusts the prewarmed window to be entrusted (after waiting if it is being built). If there is nothing to entrust and the pool wants one, it launches
/// a build (I16). If no pool is registered, `None`. ★Do not call it from the main thread★ (I15 — `SpareState::claim`).
// ADR-0299
pub(crate) fn claim(app: &AppHandle) -> Option<SpareClaim> {
    let spare = app.try_state::<Arc<SpareState>>()?;
    let reply = spare.claim();
    if reply.build {
        spawn_spare_build(app);
    }
    let wait_ms = reply
        .waited
        .map(|waited| u64::try_from(waited.as_millis()).unwrap_or(u64::MAX));
    match &reply.claim {
        Some(claim) => tracing::info!(
            module = "spare",
            label = claim.label,
            generation = claim.generation,
            wait_ms = ?wait_ms,
            "창 포트 — 예열 창을 맡겼다(spare_claim)"
        ),
        None => tracing::debug!(
            module = "spare",
            build = reply.build,
            wait_ms = ?wait_ms,
            "창 포트 — 맡길 예열 창이 없어 새로 짓는다"
        ),
    }
    reply.claim
}

/// When the window port opens (places) `label` — how the pool sees that label (meaning = `SpareState::opening`). If no pool is registered,
/// `Fresh`.
pub(crate) fn opening(app: &AppHandle, label: &str, activate: bool) -> Opening {
    app.try_state::<Arc<SpareState>>()
        .map_or(Opening::Fresh, |spare| spare.opening(label, activate))
}

/// The real implementation of window port `committed` — binds the commit (`SpareState::commit`) to the Tauri window-side job ([`TauriCommitPort`]). Called
/// outside the lock.
// ADR-0299
pub(crate) fn commit(app: &AppHandle, claim: &SpareClaim) {
    let (Some(spare), Some(layout)) = (
        app.try_state::<Arc<SpareState>>(),
        app.try_state::<LayoutState>(),
    ) else {
        tracing::warn!(
            module = "spare",
            label = claim.label,
            "예열 창 상태 · 레이아웃이 등록돼 있지 않아 맡긴 창을 확정하지 못했다"
        );
        return;
    };
    // The opening mark of the taken-in window comes before the take-in notification (`spare:adopted`) — the minimum-size report that the page which received the notification · tab sends first
    //   while drawing the number marker · window buttons widens and pulls in (TRD S26-window-chrome §7-7). A waiting page has no window number, so it does not
    //   report (`TitleRow`) — so a waiting-time report that was in flight cannot withdraw this mark first (it is the same page, so the report sequence cannot tell a waiting-time
    //   report from a post-take-in report). If the commit ends by closing, that window's `Destroyed` forgets the mark.
    crate::chrome::mark_opening(app, &claim.label);
    spare.commit(&layout, claim, &TauriCommitPort { app });
}

/// The real implementation of window port `release_spare` — if the claim ticket is the current claim, give it back to the pool hidden (otherwise no-op + warn).
pub(crate) fn release(app: &AppHandle, claim: &SpareClaim) {
    if let Some(spare) = app.try_state::<Arc<SpareState>>() {
        spare.release(claim);
    }
}

/// Placing the entrusted window failed and withdrawing it was attempted, but even that withdrawal failed — if that window's `Destroyed` does not come, the claim never ends
/// and that run's prewarming stops. It is treated as gone, taken out of the pool, and rebuilt (one hidden window remains — the same as the same
/// failure of a newly built window).
// ADR-0299
pub(crate) fn abandon_claimed(app: &AppHandle, label: &str) {
    tracing::warn!(
        module = "spare",
        label,
        "맡긴 예열 창을 거두지 못해 사라진 것으로 치고 풀에서 뺀다"
    );
    on_spare_window_destroyed(app, label);
}

/// The Tauri window-side job after commit ③ ④ (`crate::spare::CommitPort`) — called outside the layout lock · pool lock.
struct TauriCommitPort<'a> {
    app: &'a AppHandle,
}

impl CommitPort for TauriCommitPort<'_> {
    fn adopted(&self, label: &str, generation: u32) {
        if let Err(e) = self
            .app
            .emit_to(label, EVT_SPARE_ADOPTED, SpareAdopted { generation })
        {
            tracing::warn!(
                module = "spare",
                label,
                generation,
                "{EVT_SPARE_ADOPTED} emit 실패 — 그 창은 첫 화면 시한에 보인다: {e}"
            );
        }
    }

    fn reveal_deadline(&self, label: &str, after: Duration) {
        let Some(spare) = self
            .app
            .try_state::<Arc<SpareState>>()
            .map(|spare| spare.inner().clone())
        else {
            return;
        };
        let app = self.app.clone();
        let label = label.to_owned();
        tauri::async_runtime::spawn(complete_when_overdue(spare, label, after, move || {
            spawn_spare_build(&app)
        }));
    }

    fn destroy(&self, label: &str) -> Result<(), String> {
        crate::commands::popout::try_destroy_window(self.app, label)
    }
}

// Safety deadline (I8) — if the show-wait of that label is still standing after `after`, end it and build. If a completion (show · `Destroyed`) came earlier
//   it is a no-op (once per label — `SpareState::on_reveal_overdue`).
fn complete_when_overdue(
    spare: Arc<SpareState>,
    label: String,
    after: Duration,
    build: impl FnOnce() + Send + 'static,
) -> impl Future<Output = ()> + Send + 'static {
    async move {
        tokio::time::sleep(after).await;
        if spare.on_reveal_overdue(&label) {
            tracing::info!(
                module = "spare",
                label,
                "채택한 창이 안전 시한까지 보이지 않았다(UI 숨기기 · 보이기 실패) — 다시 짓는다"
            );
            build();
        }
    }
}

/// App exit request — prewarmed windows that disappear after this are not rebuilt (the `run` event of `lib.rs`).
pub(crate) fn on_app_exiting(app: &AppHandle) {
    if let Some(spare) = app.try_state::<Arc<SpareState>>() {
        spare.begin_exit();
    }
}

// ── building prewarmed windows (TRD §3-9 「부팅」 · 「다시 짓기」) ─────────────────────────────────────
//
// Build one in advance and wait ★hidden★ — no off-screen waiting place is kept (user decision 2026-10-19). A hidden top-level window
// does not appear in the taskbar · Alt+Tab.
// ★The look of the first show of a window that waited hidden has never been measured★ — the prebuilt window of spike S8 (0 flicker) was one that stayed visible
// off-screen. If it leaks, the symptom = a few frames of black · white · the previous picture show when that window is first shown.
// ★Build on a worker thread★ — building inside a synchronous IPC handler deadlocks on Windows (spike), and the main thread's event
// handler (`Destroyed`) is also a caller.

/// Build one prewarmed window on a worker thread — if the pool already has a prewarmed window (including one being built), or there is an adoption window waiting to be shown, or
/// the app is exiting, or the pool is off, it neither builds nor takes a pop-out number (`SpareState::build`).
pub(crate) fn spawn_spare_build(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || build_spare(&app));
}

// While waiting, no window call that changes visibility is made on this window — the tao visibility flag (false) stays matched with the OS visibility.
// ★When showing, use Tauri `show()`★ — if shown with raw Win32, the flag stays false and tray hide does nothing (with no changed
//   flag, tao 0.35.3 `apply_diff` returns without doing anything). It is a window built without focus, so tao shows it without activating
//   (`SW_SHOWNOACTIVATE`).
// ADR-0295
fn build_spare(app: &AppHandle) {
    let (Some(spare), Some(labels)) = (
        app.try_state::<Arc<SpareState>>(),
        app.try_state::<Arc<PopupCounter>>(),
    ) else {
        tracing::warn!(
            module = "spare",
            "예열 창 상태 · 팝아웃 번호가 등록돼 있지 않아 예열 창을 짓지 못했다"
        );
        return;
    };
    spare.build(&**labels, |label| {
        crate::commands::popout::build_spare_window(app, label).map(drop)
    });
}

// ── tests: safety deadline (TRD S21-slot-docking §8-1 ⑫ 「다시 짓기」) — stands without a window ──────────
#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::Instant;

    use super::*;
    use crate::layout::LabelSource;
    use crate::spare::{CommitOutcome, PaintGate, SpareBuild, SpareSeams};

    struct OpenGate;

    impl PaintGate for OpenGate {
        fn hold_unarmed(&self, _label: &str) {}
        fn cancel(&self, _label: &str) {}
        fn reveal_if_held(
            &self,
            _label: &str,
            _generation: u32,
            _activate: bool,
            _opened: Option<Instant>,
        ) -> bool {
            true
        }
    }

    struct NoWindows;

    impl CommitPort for NoWindows {
        fn adopted(&self, _label: &str, _generation: u32) {}
        fn reveal_deadline(&self, _label: &str, _after: Duration) {}
        fn destroy(&self, _label: &str) -> Result<(), String> {
            Ok(())
        }
    }

    #[derive(Default)]
    struct Labels(AtomicU32);

    impl LabelSource for Labels {
        fn next_label(&self) -> String {
            format!("slot-popup-{}", self.0.fetch_add(1, Ordering::SeqCst) + 1)
        }
    }

    // One window that was adopted and is waiting to be shown.
    fn adopted() -> (Arc<SpareState>, String) {
        let spare = Arc::new(SpareState::new(SpareSeams::new(Arc::new(OpenGate))));
        let SpareBuild::Built { label } = spare.build(&Labels::default(), |_| Ok(())) else {
            panic!("지어야 한다");
        };
        let claim = spare.claim().claim.expect("맡긴다");
        let layout = LayoutState::new();
        layout.0.lock().unwrap().create_window(&label).unwrap();
        assert_eq!(
            spare.commit(&layout, &claim, &NoWindows),
            CommitOutcome::Adopted { revealed: true }
        );
        (spare, label)
    }

    fn counter() -> (Arc<AtomicU32>, impl FnOnce() + Send + 'static) {
        let built = Arc::new(AtomicU32::new(0));
        let sink = built.clone();
        (built, move || {
            sink.fetch_add(1, Ordering::SeqCst);
        })
    }

    #[tokio::test(start_paused = true)]
    async fn a_window_still_unrevealed_at_the_safety_deadline_is_completed_and_rebuilt() {
        let (spare, label) = adopted();
        let (built, build) = counter();
        let after = Duration::from_secs(3);
        let deadline = tokio::spawn(complete_when_overdue(spare.clone(), label, after, build));
        tokio::time::sleep(after - Duration::from_millis(1)).await;
        assert_eq!(built.load(Ordering::SeqCst), 0, "시한 전에는 짓지 않는다");
        deadline.await.unwrap();
        assert_eq!(built.load(Ordering::SeqCst), 1);
        assert_eq!(spare.awaiting_reveal(), None);
    }

    #[tokio::test(start_paused = true)]
    async fn the_safety_deadline_after_the_reveal_builds_nothing() {
        let (spare, label) = adopted();
        let (built, build) = counter();
        assert!(spare.on_shown(&label), "보임이 먼저 짓는다");
        complete_when_overdue(spare, label, Duration::from_secs(3), build).await;
        assert_eq!(built.load(Ordering::SeqCst), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn the_safety_deadline_after_the_window_is_gone_builds_nothing() {
        let (spare, label) = adopted();
        let (built, build) = counter();
        assert!(spare.on_destroyed(&label), "사라짐이 먼저 짓는다");
        complete_when_overdue(spare, label, Duration::from_secs(3), build).await;
        assert_eq!(built.load(Ordering::SeqCst), 0);
    }
}
