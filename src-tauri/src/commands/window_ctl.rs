//! 창 상태 포트(`layout::window_ctl::WindowControl`)의 Tauri 어댑터와 invoke `window_control` — 사람 길(웹 단추 ·
//! 레지스트리)과 버스 동사가 같은 서비스(`layout::window_ctl`)에 떨어진다(ADR-0081 · TRD S26-window-chrome §6-2 · §6-4).
//! 웹의 최소 창 크기 보고 invoke `report_window_min_width` 도 여기 둔다(장부 = `crate::chrome` · §7-7).
//!
//! - ★동작 하나 = 메인 스레드 일 하나★(§5-2) — [`TauriWindowControl`] 의 `apply` · `show_all` · `hide_all` 은 읽기 ·
//!   판정 · 입히기를 한 `run_on_main_thread` 일 안에서 하고 그 답을 기다린다. 첫 화면 보이기도 메인 스레드라 그 일과
//!   끼어들 수 없다. 그 일 안의 창 하나 조작이 [`TauriOps`] 다.
//! - ★부르는 쪽은 레이아웃 락을 쥐지 않는다★ — 그 일이 같은 락을 잠깐 잡는다(`layout::window_ctl` 머리).
//! - OS `cfg` 를 두지 않는다 — OS 마다 다른 일은 platform `window` 가 한다(ADR-0266 · TRD §8-3).

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

/// 메인 스레드 일 하나의 답을 기다리는 시한. 창 상태 일은 창 게터 · 세터 몇 번이라 짧고, 메인 스레드가 그동안 창 하나를
/// 짓고 있으면(WebView2 짓기 — debug 빌드 약 1 s · `docs/process/step-log.md` 의 U4d′ 실측) 그만큼 늦는다. 그 몇 배를 둔다.
/// ★시한이 지나도 넘긴 일은 취소되지 않는다★ — 답만 버리고, 그 일은 메인 스레드가 풀리면 돈다.
const MAIN_THREAD_WAIT: Duration = Duration::from_secs(5);

/// [`WindowControl`] 의 Tauri 실물 — 시한 뒤의 답: `apply` = `Err` · `show_all` = 거짓 · `hide_all` = 무동작(로그 한 줄).
/// 메인 스레드에서 불러도 된다 — 넘긴 일이 그 자리에서 돈다(`layout::window_ctl::WindowControl` 문서).
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

    // 어느 스레드에서나 읽는다 — 창 게터는 메인 스레드에 물어 답을 받고, 앞 창 · 첫 화면 대기 판정은 스레드와 무관하다.
    fn state(&self, label: &str) -> Option<OsWindowState> {
        read_state(&self.app, label)
    }

    // 팝아웃 label 만 — main 은 늘 모델에 있고, 그 밖 label 의 창은 셸이 짓지 않는다. 예열 창 풀의 창은 뺀다 — 숨은 채
    //   다음 새 창을 기다리는 창이라 닫거나 바람을 적으면 들인 창에 옮아간다.
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

// `job` 을 메인 스레드 일 하나로 돌고 그 답을 기다린다. 답의 주인은 [`handoff`] 가 가른다 — 시한 뒤 버려진 답은 일 쪽이
//   한 줄 남긴다.
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

// 답 하나의 주인을 한 칸의 비교-교체로 가른다 — 일이 먼저 `DELIVERED` 를 세우면 받는 쪽이 시한을 넘겼어도 반드시 받고,
//   받는 쪽이 먼저 `ABANDONED` 를 세우면 일이 답을 버리며 남긴다. ★보내기 성공을 받음으로 읽지 말 것★ — 받는 쪽은 시한
//   뒤에도 함수가 끝날 때까지 채널을 쥐고 있어, 시한 직후의 보내기는 성공하고도 버려졌다.
const WAITING: u8 = 0;
const DELIVERED: u8 = 1;
const ABANDONED: u8 = 2;

// 채널은 버퍼 하나다 — 메인 스레드에서 부르면 그 일이 이 자리에서 돌아(tauri-runtime-wry 2.11.3 `send_user_message`) 받기
//   전에 보내므로, 용량 0 이면 그 보내기가 영영 선다.
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
    // `Err` = 받는 쪽이 이미 물러나 그 답을 돌려준다.
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
                // 일이 보내기로 정했다 — 곧 온다. 정하고도 못 보냈으면(채널째 버려짐) 돌지 않은 것과 같다.
                self.rx.recv().map_or(Waited::NotRun, Waited::Answered)
            }
        }
    }
}

/// 그 창이 지금 OS 전경 창인가 — platform `window::is_foreground` 이고, 그 OS 에 수단이 없으면(`None`) Tauri `is_focused`
/// 로 물러난다. Windows 에서 `is_focused` 를 쓰지 않는 까닭 = 웹뷰가 키보드 포커스를 쥔 동안 거짓이다(TRD
/// S26-window-chrome §6-2 「앞 창 판정」 · `state::placement::foreground_is_ours` 머리). 못 읽으면 거짓. 어느 스레드에서나
/// 된다.
pub(crate) fn is_foreground(window: &Window) -> bool {
    engram_dashboard_platform::window::is_foreground(window)
        .unwrap_or_else(|| matches!(window.is_focused(), Ok(true)))
}

/// 창 하나의 OS 상태 순간값 — `None` = 그 label 의 OS 창이 없거나 보임 · 최소화 · 최대화를 못 읽었다. 칸의 뜻은
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

/// [`WindowOps`] 의 Tauri 실물(TRD S26-window-chrome §6-2) — Tauri 창 · platform `window` · 첫 화면 문 · 미룬 최대화 표를
/// 부른다. ★메인 스레드 일 안에서만 쓴다★ — 날 Win32 올리기(`first_paint::raise_without_activation`)가 메인 스레드를
/// 요구하고, 읽기 · 판정 · 입히기가 첫 화면 보이기와 끼어들지 않아야 한다(§5-2). 그 안에서 메인 스레드로 다시 넘기지
/// 않는다 — 넘긴 일은 지금 일이 끝난 뒤에야 돌아 이 일의 차례 밖이 된다.
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

/// invoke `window_control` 의 동작 — 뜻은 같은 이름의 버스 동사(`window.minimize` · `window.maximize` ·
/// `window.unmaximize` · `window.raise` · `window.hideAll` · `window.showAll` · `window.close`)와 같다. wire 값은 snake_case
/// (`hide_all` 따위).
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

/// `close` 의 결말 — 뜻은 버스 `window.close` 의 `outcome` 과 같다: `closed` = 팝아웃을 닫았다 · `hidden_all` = main 이라
/// 닫지 않고 모든 창을 숨겼다 · `kept` = `if_empty` 인데 그 창에 탭이 있거나 이미 닫혀 그대로 뒀다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum WindowCloseResult {
    Closed,
    HiddenAll,
    Kept,
}

/// invoke `window_control` 의 답 — 칸의 뜻은 버스 답과 같고, 그 동작이 싣지 않는 칸은 거짓 · `null` 이다.
///
/// - `deferred` = 보이지 않는 창이라 바랄 최대화 상태를 적어 뒀다(보일 때 입힌다) — `maximize` · `unmaximize`.
/// - `pending` = 첫 화면 전인 창이라 아무것도 하지 않았다(곧 보인다) — `raise`.
/// - `shown_all` = 숨은 창이라 먼저 모든 창을 보였다 — `raise`.
/// - `focused` = 끝난 뒤 그 창(`show_all` 이면 main)이 OS 전경 창이다 — 사실 그대로다(남의 앱이 전경이면 맨 위로만
///   올리고 거짓) — `raise` · `show_all`.
/// - `close` = `close` 의 결말 — 그 밖의 동작은 `null`.
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

/// 창 상태 동작 하나(async — 메인 스레드 일을 기다리므로 블로킹 풀에서 돈다) — `invoke('window_control', { window,
/// action, ifEmpty? })`(Tauri 는 JS 쪽 인자 이름을 camelCase 로 받는다). 버스 동사와 같은 서비스다(ADR-0081).
///
/// - `window` = 창 인자(window_id 또는 같은 실행 안 label — TRD §4-5). `hide_all` · `show_all` 은 그 창을 가리지 않는다
///   (모든 창이다) — 받은 값은 숫자 거절만 본다.
/// - `if_empty` = `close` 에만 쓴다(`true` 면 `window` 는 그 창의 window_id — 해석기를 거치지 않는다 · TRD §5-5). 다른
///   동작에 `true` 를 주면 `Err`.
/// - 차례(TRD §5-4) — ① 숫자만인 인자는 어느 동작이든 `Err`(`if_empty` 여도) ② `if_empty` ③ 인자 풀기.
/// - `Err` = 그 문구(버스의 `INVALID_ARGUMENT` · `CONFLICT` 문구와 같다).
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
            // main 닫기(= 모든 창 숨기기)와 `if_empty` 닫기(`close_if_empty` 는 구독 포트를 받지 않는다)는 구독을 다시 셈하지
            //   않는다 — 데몬 클라이언트가 없을 때도 돈다.
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

/// 창 웹뷰가 자기 제목 줄로 센 최소 창 크기(정수 CSS px = 논리 px · 보통 상태 기준)를 알린다 —
/// `invoke('report_window_min_width', { width, height, seq, epoch })`(TRD S26-window-chrome §7-7).
///
/// - 창 label 을 인자로 받지 않는다 — Tauri 가 넣어 주는 [`Window`] 가 그 값의 유일한 권위다(`report_window_canvas` 와 같은
///   규칙). 버스 명령이 아니다 — 측정 보고라 LLM 이 위조할 값이 아니다(ADR-0227).
/// - `seq` = 그 웹 페이지 안에서 보낼 때마다(재시도 포함) 오르는 순번 · `epoch` = 그 페이지를 읽은 벽시계 ms
///   (`Math.floor(performance.timeOrigin)` — 다시 읽을수록 크다). 받아들인 것보다 옛 `epoch` 이거나 같은 `epoch` 에서
///   받아들인 순번보다 새 것이 아니면 적지 않고 `Ok` 다 — 웹이 시한에 놓은 옛 시도(다시 읽기 전 페이지의 것 포함)가 늦게
///   닿아도 마지막으로 보낸 값을 덮지 못한다.
/// - 너비 · 높이 둘 중 하나라도 1~4096 밖이면 `Err` 이고 그 창의 직전 값을 유지한다.
/// - 비동기로 바꾸지 말 것 — 메인 스레드에서 돌아야 창 확인과 적기 사이에 `Destroyed` 가 끼지 않는다(`crate::chrome`).
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

// main 닫기 · `if_empty` 닫기에 넘기는 빈 구독 포트 — 두 갈래 다 부르지 않는다. 불렸으면 구독이 낡았다는 뜻이라 남긴다.
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

    // ── 답 넘기기 ──

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
        // 보내기를 다른 스레드에 둬 받는 쪽이 대개 먼저 시한(0)을 맞는다 — 어느 차례로 와도 답은 받혀야 한다.
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
