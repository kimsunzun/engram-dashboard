//! The Tauri adapter of the window state port (`layout::window_ctl::WindowControl`) and invoke `window_control` — the human path (web buttons ·
//! registry) and the bus verbs land on the same service (`layout::window_ctl`) (ADR-0081 · TRD S26-window-chrome §6-2 · §6-4).
//! The web's minimum window size report invoke `report_window_min_width` is also placed here (ledger = `crate::chrome` · §7-7).
//!
//! - ★One action = one main-thread job★ (§5-2) — `apply` · `show_all` · `hide_all` of [`TauriWindowControl`] do reading ·
//!   decision · applying inside one `run_on_main_thread` job and wait for its answer. The first-paint show is also on the main thread, so it cannot
//!   interleave with that job. The one-window operation inside that job is [`TauriOps`].
//! - ★The caller does not hold the layout lock★ — that job takes the same lock briefly (`layout::window_ctl` head).
//! - Do not put in an OS `cfg` — what differs per OS is done by platform `window` (ADR-0266 · TRD §8-3).

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tauri::{AppHandle, Manager, WebviewWindow, Window};
use ts_rs::TS;

use crate::commands::layout::RouterSubs;
use crate::commands::popout::TauriWindowHost;
use crate::daemon_client::DaemonClient;
use crate::layout::window_arg;
use crate::layout::window_ctl::{
    self, Activation, Applied, CloseOutcome, OsWindowState, WindowAction, WindowControl, WindowOps,
};
use crate::layout::{LayoutState, SubscriptionSync, ViewManager, MAIN_WINDOW_LABEL};
use crate::output_router::OutputRouter;

/// The deadline for waiting for the answer of one main-thread job. A window-state job is a few window getter · setter calls, so it is short, and if the main thread is meanwhile
/// building a window (WebView2 build — about 1 s in a debug build · measured in U4d′ of `docs/process/step-log.md`) it is delayed that much. Several times that is allowed.
/// ★Even after the deadline the handed-over job is not cancelled★ — only the answer is discarded, and the job runs when the main thread is freed.
const MAIN_THREAD_WAIT: Duration = Duration::from_secs(5);

/// The Tauri real implementation of [`WindowControl`] — answers after the deadline: `apply` = `Err` · `show_all` = false · `hide_all` = no-op (one log line).
/// It may be called from the main thread — the handed-over job runs on the spot (`layout::window_ctl::WindowControl` docs).
// ADR-0302 — window state command
pub(crate) struct TauriWindowControl {
    app: AppHandle,
}

impl TauriWindowControl {
    pub(crate) fn new(app: AppHandle) -> Self {
        Self { app }
    }
}

impl WindowControl for TauriWindowControl {
    fn apply(&self, label: &str, action: WindowAction) -> Result<Applied, String> {
        let label = label.to_owned();
        on_main_thread(&self.app, "창 상태 동작", move |app| {
            let layout = app
                .try_state::<LayoutState>()
                .ok_or_else(|| "레이아웃 상태가 등록돼 있지 않다".to_owned())?;
            window_ctl::run_action(&TauriOps::new(app), &layout, &label, action)
        })?
    }

    fn show_all(&self, how: Activation) -> bool {
        on_main_thread(&self.app, "모든 창 보이기", move |app| {
            crate::tray::actions::show_main_ui(app, how);
            app.get_webview_window(MAIN_WINDOW_LABEL)
                .is_some_and(|main| is_foreground(&main.as_ref().window()))
        })
        .unwrap_or_else(|e| {
            tracing::warn!(module = "window_ctl", "{e}");
            false
        })
    }

    fn hide_all(&self) {
        if let Err(e) = on_main_thread(&self.app, "모든 창 숨기기", |app| {
            crate::tray::actions::hide_main_ui(app)
        }) {
            tracing::warn!(module = "window_ctl", "{e}");
        }
    }

    // Read from any thread — window getters ask the main thread and get the answer, and the front-window · first-paint-wait decision is thread-independent.
    fn state(&self, label: &str) -> Option<OsWindowState> {
        read_state(&self.app, label)
    }

    // Pop-out labels only — main is always in the model, and a window with any other label is not built by the shell. Windows of the prewarmed window pool are excluded — they are windows
    //   waiting hidden for the next new window, so if one were closed or a wish recorded on it, it would carry over to the taken-in window.
    fn is_unmodeled(&self, label: &str) -> bool {
        crate::commands::popout::is_popup_label(label)
            && self.app.get_webview_window(label).is_some()
            && !crate::commands::spare::is_spare_window(&self.app, label)
    }

    fn apply_unmodeled(&self, label: &str, action: WindowAction) -> Result<Applied, String> {
        let label = label.to_owned();
        on_main_thread(&self.app, "모델 밖 창 상태 동작", move |app| {
            window_ctl::run_unmodeled_action(&TauriOps::new(app), &label, action)
        })?
    }
}

// Run `job` as one main-thread job and wait for its answer. Who owns the answer is decided by [`handoff`] — an answer discarded after the deadline is
//   logged by the job side with one line.
fn on_main_thread<T: Send + std::fmt::Debug + 'static>(
    app: &AppHandle,
    what: &str,
    job: impl FnOnce(&AppHandle) -> T + Send + 'static,
) -> Result<T, String> {
    let (slot, waiter) = handoff();
    let handle = app.clone();
    let job_what = what.to_owned();
    let dispatched = Instant::now();
    app.run_on_main_thread(move || {
        let result = job(&handle);
        if let Err(dropped) = slot.deliver(result) {
            tracing::debug!(
                module = "window_ctl",
                what = %job_what,
                elapsed_ms = dispatched.elapsed().as_millis() as u64,
                result = ?dropped,
                "메인 스레드 일이 끝났지만 부르는 쪽은 이미 시한으로 물러났다 — 그 답은 버려졌다"
            );
        }
    })
    .map_err(|e| format!("{what} — 메인 스레드에 넘기지 못했다: {e}"))?;
    match waiter.wait(MAIN_THREAD_WAIT) {
        Waited::Answered(result) => Ok(result),
        Waited::Abandoned => Err(format!(
            "{what} — 메인 스레드가 {} 초 안에 답하지 않았다(넘긴 일은 그 뒤에 돌 수 있다)",
            MAIN_THREAD_WAIT.as_secs()
        )),
        Waited::NotRun => Err(format!("{what} — 메인 스레드가 그 일을 돌지 않았다")),
    }
}

// Decide who owns one answer with a single compare-and-swap — if the job sets `DELIVERED` first, the receiver always receives it even if it passed the deadline,
//   and if the receiver sets `ABANDONED` first, the job discards the answer and logs it. ★Do not read a successful send as receipt★ — the receiver holds the channel
//   until the function ends even after the deadline, so a send right after the deadline succeeded and was still discarded.
const WAITING: u8 = 0;
const DELIVERED: u8 = 1;
const ABANDONED: u8 = 2;

// The channel has a buffer of one — if called on the main thread, that job runs on the spot (tauri-runtime-wry 2.11.3 `send_user_message`) and sends before
//   receiving, so with capacity 0 that send would block forever.
fn handoff<T>() -> (Slot<T>, Waiter<T>) {
    let (tx, rx) = mpsc::sync_channel(1);
    let state = Arc::new(AtomicU8::new(WAITING));
    (
        Slot {
            tx,
            state: state.clone(),
        },
        Waiter { rx, state },
    )
}

struct Slot<T> {
    tx: SyncSender<T>,
    state: Arc<AtomicU8>,
}

impl<T> Slot<T> {
    // `Err` = the receiver has already withdrawn, so the answer is returned.
    fn deliver(self, value: T) -> Result<(), T> {
        if !self.commit() {
            return Err(value);
        }
        self.tx.send(value).map_err(|mpsc::SendError(value)| value)
    }

    fn commit(&self) -> bool {
        self.state
            .compare_exchange(WAITING, DELIVERED, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }
}

struct Waiter<T> {
    rx: Receiver<T>,
    state: Arc<AtomicU8>,
}

#[derive(Debug, PartialEq, Eq)]
enum Waited<T> {
    Answered(T),
    Abandoned,
    NotRun,
}

impl<T> Waiter<T> {
    fn wait(self, wait: Duration) -> Waited<T> {
        match self.rx.recv_timeout(wait) {
            Ok(value) => Waited::Answered(value),
            Err(RecvTimeoutError::Disconnected) => Waited::NotRun,
            Err(RecvTimeoutError::Timeout) => {
                let abandoned = self
                    .state
                    .compare_exchange(WAITING, ABANDONED, Ordering::AcqRel, Ordering::Acquire)
                    .is_ok();
                if abandoned {
                    return Waited::Abandoned;
                }
                // The job decided to send — it is coming soon. If it decided and still could not send (the channel itself was dropped), it is the same as not having run.
                self.rx.recv().map_or(Waited::NotRun, Waited::Answered)
            }
        }
    }
}

/// Is that window the OS foreground window now — platform `window::is_foreground`, and if the OS has no means (`None`), fall back to Tauri `is_focused`.
/// The reason `is_focused` is not used on Windows = it is false while the webview holds keyboard focus (TRD
/// S26-window-chrome §6-2 「앞 창 판정」 · head of `state::placement::foreground_is_ours`). If unreadable, false. Works
/// from any thread.
pub(crate) fn is_foreground(window: &Window) -> bool {
    engram_dashboard_platform::window::is_foreground(window)
        .unwrap_or_else(|| matches!(window.is_focused(), Ok(true)))
}

/// A snapshot of one window's OS state — `None` = that label's OS window does not exist or visible · minimized · maximized could not be read. The meaning of the fields is in
/// [`OsWindowState`].
fn read_state(app: &AppHandle, label: &str) -> Option<OsWindowState> {
    let window = app.get_webview_window(label)?;
    let read = || -> tauri::Result<(bool, bool, bool)> {
        Ok((
            window.is_visible()?,
            window.is_minimized()?,
            window.is_maximized()?,
        ))
    };
    let (visible, minimized, maximized) = read()
        .map_err(|e| {
            tracing::debug!(module = "window_ctl", label, error = %e, "창 상태를 못 읽었다");
        })
        .ok()?;
    Some(OsWindowState {
        visible,
        minimized,
        maximized: maximized && !minimized,
        focused: is_foreground(&window.as_ref().window()),
        awaiting_first_paint: crate::first_paint::is_held(app, label),
    })
}

/// The Tauri real implementation of [`WindowOps`] (TRD S26-window-chrome §6-2) — calls Tauri windows · platform `window` · the first-paint gate · the deferred-maximize table.
/// ★Used only inside a main-thread job★ — a raw Win32 raise (`first_paint::raise_without_activation`) requires the main thread,
/// and reading · decision · applying must not interleave with the first-paint show (§5-2). Inside it, do not hand off to the main thread again — the handed-off job
/// runs only after the current job ends, which puts it outside this job's turn.
pub(crate) struct TauriOps<'a> {
    app: &'a AppHandle,
}

impl<'a> TauriOps<'a> {
    pub(crate) fn new(app: &'a AppHandle) -> Self {
        Self { app }
    }

    fn window(&self, label: &str) -> Option<WebviewWindow> {
        self.app.get_webview_window(label)
    }

    fn call(
        &self,
        label: &str,
        verb: &str,
        run: impl FnOnce(&WebviewWindow) -> tauri::Result<()>,
    ) -> Result<(), String> {
        let window = self
            .window(label)
            .ok_or_else(|| format!("window 없음: {label} — OS 창이 열려 있지 않다"))?;
        run(&window).map_err(|e| format!("{label} 의 {verb} 실패: {e}"))
    }
}

impl WindowOps for TauriOps<'_> {
    fn read(&self, label: &str) -> Option<OsWindowState> {
        read_state(self.app, label)
    }

    fn minimize(&self, label: &str) -> Result<(), String> {
        self.call(label, "최소화", |w| w.minimize())
    }

    fn maximize(&self, label: &str) -> Result<(), String> {
        self.call(label, "최대화", |w| w.maximize())
    }

    fn unmaximize(&self, label: &str) -> Result<(), String> {
        self.call(label, "원래 크기", |w| w.unmaximize())
    }

    fn defer_maximized(&self, label: &str, maximized: bool) {
        crate::state::placement::defer_maximize(self.app, label, maximized);
    }

    fn drop_deferred(&self, label: &str) {
        crate::state::placement::drop_deferred_maximize(self.app, label);
    }

    fn raise_without_activation(&self, label: &str) {
        if let Some(window) = self.window(label) {
            crate::first_paint::raise_without_activation(&window.as_ref().window());
        }
    }

    fn show(&self, label: &str) -> Result<(), String> {
        self.call(label, "보이기", |w| w.show())
    }

    fn unminimize(&self, label: &str) -> Result<(), String> {
        self.call(label, "펴기", |w| w.unminimize())
    }

    fn apply_deferred(&self, label: &str) {
        if let Some(window) = self.window(label) {
            crate::state::placement::apply_deferred_maximize(&window.as_ref().window());
        }
    }

    fn foreground_is_ours(&self) -> bool {
        crate::state::placement::foreground_is_ours(self.app)
    }

    fn set_focus(&self, label: &str) {
        let focused = match self.window(label) {
            Some(window) => window.set_focus().map_err(|e| e.to_string()),
            None => Err("창이 없다".to_owned()),
        };
        if let Err(e) = focused {
            tracing::debug!(
                module = "window_ctl",
                label,
                "앞으로 낸 창에 포커스를 주지 못했다: {e}"
            );
        }
    }

    fn show_all_no_steal(&self) {
        crate::tray::actions::show_main_ui_no_steal(self.app, self);
    }
}

// ── invoke `window_control` (TRD S26-window-chrome §6-4) ─────────────────────────

/// The action of invoke `window_control` — the meaning is the same as the same-named bus verb (`window.minimize` · `window.maximize` ·
/// `window.unmaximize` · `window.raise` · `window.hideAll` · `window.showAll` · `window.close`). The wire value is snake_case
/// (like `hide_all`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum WindowControlAction {
    Minimize,
    Maximize,
    Unmaximize,
    Raise,
    HideAll,
    ShowAll,
    Close,
}

/// The outcome of `close` — the meaning is the same as `outcome` of bus `window.close`: `closed` = a pop-out was closed · `hidden_all` = it was main, so it was not
/// closed and all windows were hidden · `kept` = it was `if_empty` but that window has tabs or was already closed, so it was left as is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum WindowCloseResult {
    Closed,
    HiddenAll,
    Kept,
}

/// The answer of invoke `window_control` — the meaning of the fields is the same as the bus answer, and a field that the action does not carry is false · `null`.
///
/// - `deferred` = it was an invisible window, so the wished maximize state was recorded (applied when shown) — `maximize` · `unmaximize`.
/// - `pending` = it was a window before first paint, so nothing was done (it will be visible soon) — `raise`.
/// - `shown_all` = it was a hidden window, so all windows were shown first — `raise`.
/// - `focused` = after finishing, that window (main for `show_all`) is the OS foreground window — the fact as it is (if another app is foreground, it is only
///   raised to the top and false) — `raise` · `show_all`.
/// - `close` = the outcome of `close` — `null` for other actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub struct WindowControlOutcome {
    pub deferred: bool,
    pub pending: bool,
    pub shown_all: bool,
    pub focused: bool,
    pub close: Option<WindowCloseResult>,
}

impl From<Applied> for WindowControlOutcome {
    fn from(applied: Applied) -> Self {
        Self {
            deferred: applied.deferred,
            pending: applied.pending,
            shown_all: applied.shown_all,
            focused: applied.focused,
            close: None,
        }
    }
}

impl From<CloseOutcome> for WindowCloseResult {
    fn from(outcome: CloseOutcome) -> Self {
        match outcome {
            CloseOutcome::Closed => Self::Closed,
            CloseOutcome::HiddenAll => Self::HiddenAll,
            CloseOutcome::Kept => Self::Kept,
        }
    }
}

/// One window-state action (async — it waits for a main-thread job, so it runs on the blocking pool) — `invoke('window_control', { window,
/// action, ifEmpty? })` (Tauri accepts JS-side argument names in camelCase). It is the same service as the bus verbs (ADR-0081).
///
/// - `window` = the window argument (window_id or a label within the same run — TRD §4-5). `hide_all` · `show_all` do not distinguish that window
///   (it is all windows) — the received value is only checked for numeric rejection.
/// - `if_empty` = used only for `close` (if `true`, `window` is that window's window_id — it does not go through the resolver · TRD §5-5). Giving
///   `true` to another action gives `Err`.
/// - Order (TRD §5-4) — ① an argument that is only digits is `Err` for any action (even with `if_empty`) ② `if_empty` ③ argument resolution.
/// - `Err` = that message (the same as the bus `INVALID_ARGUMENT` · `CONFLICT` messages).
#[tauri::command]
pub async fn window_control(
    app: AppHandle,
    window: String,
    action: WindowControlAction,
    if_empty: Option<bool>,
) -> Result<WindowControlOutcome, String> {
    tauri::async_runtime::spawn_blocking(move || {
        control(&app, &window, action, if_empty.unwrap_or(false))
    })
    .await
    .map_err(|e| format!("창 상태 동작이 끝나지 못했다: {e}"))?
}

fn control(
    app: &AppHandle,
    window: &str,
    action: WindowControlAction,
    if_empty: bool,
) -> Result<WindowControlOutcome, String> {
    window_arg::reject_number(window).map_err(|e| e.to_string())?;
    if if_empty && action != WindowControlAction::Close {
        return Err("if_empty 는 close 에만 쓴다".to_owned());
    }
    let state = app
        .try_state::<LayoutState>()
        .ok_or_else(|| "레이아웃 상태가 등록돼 있지 않다".to_owned())?;
    let control = TauriWindowControl::new(app.clone());
    let act = |action| {
        window_ctl::act(&state, &control, window, action)
            .map(WindowControlOutcome::from)
            .map_err(|e| e.to_string())
    };
    match action {
        WindowControlAction::Minimize => act(WindowAction::Minimize),
        WindowControlAction::Maximize => act(WindowAction::Maximize),
        WindowControlAction::Unmaximize => act(WindowAction::Unmaximize),
        WindowControlAction::Raise => act(WindowAction::Raise),
        WindowControlAction::HideAll => {
            control.hide_all();
            Ok(WindowControlOutcome::default())
        }
        WindowControlAction::ShowAll => Ok(WindowControlOutcome {
            focused: control.show_all(Activation::NoSteal),
            ..WindowControlOutcome::default()
        }),
        WindowControlAction::Close => {
            let host = TauriWindowHost { app };
            // main close (= hide all windows) and `if_empty` close (`close_if_empty` does not take a subscription port) do not recount subscriptions
            //   — they run even when there is no daemon client.
            let no_subs = if_empty
                || window_ctl::resolve(&state, window)
                    .is_ok_and(|label| label == MAIN_WINDOW_LABEL);
            let outcome = if no_subs {
                window_ctl::close(&state, &NoSubs, &host, &control, window, if_empty)
            } else {
                let router = app
                    .try_state::<Arc<OutputRouter>>()
                    .ok_or_else(|| "출력 라우터가 등록돼 있지 않다".to_owned())?;
                let client = app
                    .try_state::<Arc<DaemonClient>>()
                    .ok_or_else(|| "데몬 클라이언트가 등록돼 있지 않다".to_owned())?;
                window_ctl::close(
                    &state,
                    &RouterSubs {
                        router: &router,
                        client: &client,
                    },
                    &host,
                    &control,
                    window,
                    if_empty,
                )
            }
            .map_err(|e| e.to_string())?;
            Ok(WindowControlOutcome {
                close: Some(outcome.into()),
                ..WindowControlOutcome::default()
            })
        }
    }
}

/// The window webview reports the minimum window size it counted from its own title row (integer CSS px = logical px · based on the normal state) —
/// `invoke('report_window_min_width', { width, height, seq, epoch })` (TRD S26-window-chrome §7-7).
///
/// - It does not take the window label as an argument — the [`Window`] that Tauri injects is the sole authority for that value (the same
///   rule as `report_window_canvas`). It is not a bus command — it is a measurement report, so not a value for the LLM to forge (ADR-0227).
/// - `seq` = a sequence number that rises on every send within that web page (retries included) · `epoch` = the wall-clock ms at which that page was loaded
///   (`Math.floor(performance.timeOrigin)` — the later it is reloaded, the larger). If it has an `epoch` older than the accepted one, or in the same `epoch` is not newer than the
///   accepted sequence, it is not recorded and `Ok` — even if an old attempt the web left on a deadline (including one from the page before reload) arrives late,
///   it cannot overwrite the last sent value.
/// - If either width or height is outside 1~4096, `Err` and that window's previous value is kept.
/// - Do not change it to async — it must run on the main thread so that `Destroyed` cannot come in between the window check and the recording (`crate::chrome`).
// ADR-0303 — chrome-style title bar
#[tauri::command]
pub fn report_window_min_width(
    app: AppHandle,
    window: Window,
    width: u32,
    height: u32,
    seq: u32,
    epoch: u64,
) -> Result<(), String> {
    crate::chrome::report_min_size(
        &app,
        window.label(),
        width,
        height,
        crate::chrome::MinOrder { epoch, seq },
    )
}

// The empty subscription port passed to main close · `if_empty` close — neither branch calls it. If it was called, it means the subscription is stale, so it is logged.
struct NoSubs;

impl SubscriptionSync for NoSubs {
    fn resync(&self, _mgr: &ViewManager) {
        tracing::warn!(
            module = "window_ctl",
            "main · if_empty 닫기에 구독 재계산이 불렸다 — 데몬 구독을 다시 셈하지 않았다"
        );
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    // ── handing over the answer ──

    #[test]
    fn an_answer_delivered_in_time_is_received() {
        let (slot, waiter) = handoff();
        assert_eq!(slot.deliver(7), Ok(()));
        assert_eq!(waiter.wait(Duration::ZERO), Waited::Answered(7));
    }

    #[test]
    fn an_answer_after_the_caller_gave_up_comes_back_to_the_job() {
        let (slot, waiter) = handoff::<i32>();
        assert_eq!(waiter.wait(Duration::ZERO), Waited::Abandoned);
        assert_eq!(slot.deliver(7), Err(7));
    }

    #[test]
    fn a_job_committed_before_the_timeout_is_still_received() {
        // The send is put on another thread so the receiver usually meets the deadline (0) first — the answer must be received in either order.
        let (slot, waiter) = handoff();
        assert!(slot.commit());
        let sender = std::thread::spawn(move || slot.tx.send(7).unwrap());
        assert_eq!(waiter.wait(Duration::ZERO), Waited::Answered(7));
        sender.join().unwrap();
    }

    #[test]
    fn a_job_committed_but_dropped_unsent_reads_as_not_run() {
        let (slot, waiter) = handoff::<i32>();
        assert!(slot.commit());
        let dropper = std::thread::spawn(move || drop(slot));
        assert_eq!(waiter.wait(Duration::ZERO), Waited::NotRun);
        dropper.join().unwrap();
    }

    #[test]
    fn a_job_dropped_uncommitted_reads_as_not_run() {
        let (slot, waiter) = handoff::<i32>();
        drop(slot);
        assert_eq!(waiter.wait(Duration::from_secs(5)), Waited::NotRun);
    }

    #[test]
    fn the_action_and_the_outcome_spell_the_wire_names() {
        let actions: Vec<WindowControlAction> = serde_json::from_value(json!([
            "minimize",
            "maximize",
            "unmaximize",
            "raise",
            "hide_all",
            "show_all",
            "close"
        ]))
        .unwrap();
        assert_eq!(
            actions,
            [
                WindowControlAction::Minimize,
                WindowControlAction::Maximize,
                WindowControlAction::Unmaximize,
                WindowControlAction::Raise,
                WindowControlAction::HideAll,
                WindowControlAction::ShowAll,
                WindowControlAction::Close,
            ]
        );
        assert_eq!(
            serde_json::to_value(WindowControlOutcome {
                shown_all: true,
                focused: true,
                ..WindowControlOutcome::default()
            })
            .unwrap(),
            json!({ "deferred": false, "pending": false, "shown_all": true, "focused": true, "close": null })
        );
        assert_eq!(
            serde_json::to_value(WindowControlOutcome {
                close: Some(CloseOutcome::HiddenAll.into()),
                ..WindowControlOutcome::default()
            })
            .unwrap()["close"],
            json!("hidden_all")
        );
    }
}
