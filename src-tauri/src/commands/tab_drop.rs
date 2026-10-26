//! 탭 놓기 invoke 껍데기 둘(`tab_drop` · `window_frame`)과 그 Tauri 어댑터 — 끌기가 끝난 뒤의 입력 읽기(platform) ·
//! 창 게터 · 사건 내기(`tab_drop:done` · `window:frame`). 처리 · 판정 · 장부는 `crate::tab_drop` 이 한다.
//! `window:frame` 은 자리 기록 말고 그 창의 포커스 사건([`push_window_focus`])에도 밀리고, 몸에 앞 창 칸(`focused`)과
//! 제목줄 모양(`caption` — platform `caption_style`)을 싣는다(TRD S26-window-chrome §6-5).
//!
//! 둘 다 웹뷰 안 배선이라 버스 명령표에 올리지 않는다 — 놓기는 마우스 놓기의 끝이고 입력 상태를 읽는 갈래가 있어 LLM
//! 이 흉내 낼 것이 아니다. 같은 동작의 LLM 명령은 같은 `layout::apply::dock_tab` 위의 T-65 `tab.dock` 이다.
// ADR-0296
// TRD S21-slot-docking §3

use std::sync::Arc;

use engram_dashboard_platform::{input, window as os};
use tauri::{AppHandle, Emitter, Manager, State, Window};

use crate::commands::layout::{RouterSubs, TauriEvents};
use crate::commands::popout::{PopupCounter, TauriWindowHost};
use crate::daemon_client::DaemonClient;
use crate::layout::{LayoutState, WindowAttrs};
use crate::output_router::OutputRouter;
use crate::tab_drop::plan::{InputNow, PhysPoint, TopAt};
use crate::tab_drop::wire::{
    CaptionChrome, DropTo, TabDrop, TabDropDone, TabDropReply, WindowFrame,
};
use crate::tab_drop::{
    self, Arrival, DropBook, DropPorts, DropShell, FrameBook, FrameGeom, FrameSeen,
};

/// 놓기 처리마다 출처 창(명령의 `from`)에만 — 몸 = [`TabDropDone`].
pub const EVT_TAB_DROP_DONE: &str = "tab_drop:done";
/// 그 창에만 · 값이 바뀔 때만 — 몸 = [`WindowFrame`].
pub const EVT_WINDOW_FRAME: &str = "window:frame";

/// 탭 놓기(async — 새 창을 만들 수 있다) — `invoke('tab_drop', { drop })`. 인자 · 답의 계약은 [`TabDrop`] ·
/// [`TabDropReply`]. 부른 창 = Tauri 가 넣어 주는 창. 결과는 답과 별개로 `tab_drop:done` 으로 출처 창에 간다(같은
/// 끌기의 중복 · 맡아 둔 `drop` 에 자리를 내준 거절은 빼고 — [`TabDropDone`]). 늘 `Ok` 다 — 실패도 답의 갈래다.
///
/// 입력은 받자마자, 락 · 장부보다 먼저 읽는다 — 출처 dragend 가 보낸 명령이면 커서 · 주 버튼 · Esc · 커서 밑 맨 위 창,
/// 우리 웹뷰의 drop 이면 커서만. 끌기가 끝난 뒤의 호출이다(끌기 명령은 놓을 때만 — ADR-0296 · ADR-0297).
// TRD S21-slot-docking §3-2
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn tab_drop(
    window: Window,
    app: AppHandle,
    layout: State<'_, LayoutState>,
    book: State<'_, Arc<DropBook>>,
    router: State<'_, Arc<OutputRouter>>,
    counter: State<'_, Arc<PopupCounter>>,
    client: State<'_, Arc<DaemonClient>>,
    drop: TabDrop,
) -> Result<TabDropReply, String> {
    let caller = window.label().to_owned();
    let places = matches!(drop.to, DropTo::NewWindow { .. });
    let read = read_input(&app, Arrival::of(&drop.to), places);
    // 출처 창 틀은 입력을 읽은 뒤에 읽는다 — 게터 왕복이 놓은 순간의 입력 읽기를 늦추지 않게.
    let frame = if places {
        let source = app
            .get_webview_window(&drop.from)
            .or_else(|| app.get_webview_window(&caller));
        let geom = source.and_then(|w| geom_of(&w.as_ref().window()));
        tab_drop::source_frame(geom.as_ref())
    } else {
        tab_drop::source_frame(None)
    };
    let input = InputNow {
        cursor: read.cursor,
        primary_down: read.primary_down,
        escape_down: read.escape_down,
        top: read.top,
        scale: read.monitor_scale.unwrap_or(frame.scale),
    };
    let ports = DropPorts {
        state: &layout,
        subs: &RouterSubs {
            router: &router,
            client: &client,
        },
        events: &TauriEvents { app: &app },
        host: &TauriWindowHost { app: &app },
        labels: &**counter,
        shell: &TauriDropShell { app: &app },
    };
    Ok(tab_drop::handle(
        &book,
        &ports,
        drop,
        &caller,
        &input,
        &frame,
        read.top_class.as_deref(),
    ))
}

/// 부른 창의 창 사실(동기) — `invoke('window_frame')`. 웹뷰가 마운트 때 한 번 당기고, 그 뒤 바뀜은 `window:frame` 이
/// 민다. 복원 크기 · 최대화 = 모델 값 · 틀 = 지금 게터로 센 값(규칙 = `tab_drop::pull_frame`) · 앞 창 = 지금 OS 전경 창
/// 판정. 탭 모델에 없는 창이면 `Err`.
// TRD S21-slot-docking §3-8 · TRD S26-window-chrome §6-5
#[tauri::command]
pub fn window_frame(
    window: Window,
    layout: State<'_, LayoutState>,
    frames: State<'_, Arc<FrameBook>>,
) -> Result<WindowFrame, String> {
    let label = window.label();
    let attrs = layout
        .0
        .lock()
        .map_err(|e| e.to_string())?
        .window_attrs(label)
        .map_err(|e| e.to_string())?; // ← 레이아웃 락 드롭 — 게터는 그 밖이다
    let minimized = window.is_minimized().unwrap_or(false);
    let seen = FrameSeen {
        minimized,
        attrs,
        geom: geom_of(&window),
        focused: crate::commands::window_ctl::is_foreground(&window),
        caption: caption_chrome(),
    };
    Ok(tab_drop::pull_frame(&frames, label, seen))
}

/// 자리 기록 경로(`state::placement::record`)가 모델에 적은 뒤 부른다 — 그 창의 창 사실이 바뀌었을 때만 그 창에
/// `window:frame` 을 낸다(규칙 = `tab_drop::push_frame`). 앞 창 = 지금 OS 전경 창 판정. ★어느 락도 쥐지 않고
/// 부른다★ — 게터를 부른다.
// TRD S21-slot-docking §3-8
pub(crate) fn push_window_frame(window: &Window, minimized: bool, attrs: WindowAttrs) {
    let focused = crate::commands::window_ctl::is_foreground(window);
    push(window, minimized, attrs, focused);
}

/// 그 창의 포커스 사건(`WindowEvent::Focused(focused)`)에 부른다 — 앞 창 칸은 ★사건의 값★이다: 사건이 날 때 OS 전경 창
/// 판정이 아직 바뀌지 않았을 수 있다(차례 미검 · TRD S26-window-chrome §6-5 「`focused` 값의 출처」 · S12). 탭 모델에
/// 없는 창(예열 창 · 닫힌 뒤 늦게 온 사건)은 건너뛴다. ★어느 락도 쥐지 않고 부른다★.
pub(crate) fn push_window_focus(window: &Window, focused: bool) {
    let Some(layout) = window.app_handle().try_state::<LayoutState>() else {
        return;
    };
    // 레이아웃 락은 값만 읽고 곧바로 놓는다 — 게터는 그 밖이다.
    let attrs = match layout.0.lock() {
        Ok(mgr) => mgr.window_attrs(window.label()).ok(),
        Err(_) => {
            // 독 든 락 로그는 debug 다 — `state::placement::record` 와 같은 까닭.
            tracing::debug!(
                module = "tab_drop",
                label = window.label(),
                "레이아웃 락에 독이 들어 창 포커스를 밀지 않는다"
            );
            None
        }
    };
    let Some(attrs) = attrs else {
        return;
    };
    let minimized = window.is_minimized().unwrap_or(false);
    push(window, minimized, attrs, focused);
}

fn push(window: &Window, minimized: bool, attrs: WindowAttrs, focused: bool) {
    let app = window.app_handle();
    let Some(frames) = app.try_state::<Arc<FrameBook>>() else {
        return;
    };
    let label = window.label();
    let geom = if minimized { None } else { geom_of(window) };
    let seen = FrameSeen {
        minimized,
        attrs,
        geom,
        focused,
        caption: caption_chrome(),
    };
    tab_drop::push_frame(&frames, label, seen, |frame| {
        if let Err(e) = app.emit_to(label, EVT_WINDOW_FRAME, frame) {
            tracing::warn!(
                module = "dock",
                window = label,
                "{EVT_WINDOW_FRAME} emit 실패: {e}"
            );
        }
    });
}

// 창 사실의 크롬 모양 — platform 의 OS 상수에 셸 빌드의 스냅 오버레이 여부를 더한다(TRD S26-window-chrome §3-2 · §8-4).
fn caption_chrome() -> CaptionChrome {
    let style = os::caption_style();
    CaptionChrome {
        buttons: style.drawn_buttons,
        leading_inset: style.leading_inset,
        // 셸 기능 `snap-overlay` 로 지은 빌드에서만 참(기본 꺼짐 — §8-4) — 참일 때만 웹이 `report_max_button_rect` 를 부른다.
        snap_overlay: cfg!(feature = "snap-overlay"),
    }
}

/// 창이 사라진 뒤(`Destroyed`) 부른다 — 그 창의 마지막 창 사실을 잊는다(규칙 = `tab_drop::FrameBook::forget`).
pub(crate) fn forget_window_frame(app: &AppHandle, label: &str) {
    if let Some(frames) = app.try_state::<Arc<FrameBook>>() {
        frames.forget(label);
    }
}

// 끌기가 끝난 뒤 읽은 입력 — `top_class` = 맨 위가 다른 앱 창이면 그 창 클래스(로그 몫 — 끌기 그림 창을 가르는 실측 재료).
struct ReadInput {
    cursor: Option<PhysPoint>,
    primary_down: bool,
    escape_down: bool,
    top: TopAt,
    top_class: Option<String>,
    monitor_scale: Option<f64>,
}

// 차례 = 커서 → 주 버튼 → Esc → 커서 밑 맨 위 창 → 커서 밑 모니터 배율. drop 은 커서만 읽는다(drop 사건이 놓기의
//   증거다). `places` = 새 창 자리를 셀 일이 있다(모니터 배율을 읽는다).
fn read_input(app: &AppHandle, arrival: Arrival, places: bool) -> ReadInput {
    let cursor = input::cursor_pos().map(|p| PhysPoint { x: p.x, y: p.y });
    let (primary_down, escape_down, top, top_class) = match arrival {
        Arrival::DragEnd => {
            let primary_down = input::primary_button_down();
            let escape_down = input::escape_down();
            let (top, class) = match cursor {
                Some(at) => top_at(app, at),
                None => (TopAt::Unknown, None),
            };
            (primary_down, escape_down, top, class)
        }
        Arrival::Drop => (false, false, TopAt::Unknown, None),
    };
    let monitor_scale = match cursor {
        Some(at) if places => app
            .monitor_from_point(f64::from(at.x), f64::from(at.y))
            .map_err(|e| {
                tracing::debug!(
                    module = "dock",
                    "커서 밑 모니터를 못 읽었다 — 출처 창 배율로 센다: {e}"
                )
            })
            .ok()
            .flatten()
            .map(|m| m.scale_factor()),
        _ => None,
    };
    ReadInput {
        cursor,
        primary_down,
        escape_down,
        top,
        top_class,
        monitor_scale,
    }
}

// 점 밑 맨 위 최상위 창 — 우리 웹뷰 창이면 그 label, 아니면 다른 앱(그 창 클래스와 함께), 못 읽으면 `Unknown`.
// TRD S21-slot-docking §3-2 · §3-5
fn top_at(app: &AppHandle, at: PhysPoint) -> (TopAt, Option<String>) {
    let Some(hit) = os::top_level_at(os::PhysPoint { x: at.x, y: at.y }) else {
        return (TopAt::Unknown, None);
    };
    let ours = app
        .webview_windows()
        .into_iter()
        .find(|(_, w)| os::NativeWindow::of(w) == Some(hit))
        .map(|(label, _)| label);
    match ours {
        Some(label) => (TopAt::Ours(label), None),
        None => (
            TopAt::Foreign,
            os::describe_window(hit).map(|facts| facts.class),
        ),
    }
}

// 창 게터 넷과 배율 — 하나라도 못 읽으면 `None`(debug 한 줄).
fn geom_of(window: &Window) -> Option<FrameGeom> {
    let read = || -> tauri::Result<FrameGeom> {
        let (outer_pos, outer_size) = (window.outer_position()?, window.outer_size()?);
        let (inner_pos, inner_size) = (window.inner_position()?, window.inner_size()?);
        Ok(FrameGeom {
            outer_pos: (outer_pos.x, outer_pos.y),
            outer_size: (outer_size.width, outer_size.height),
            inner_pos: (inner_pos.x, inner_pos.y),
            inner_size: (inner_size.width, inner_size.height),
            scale: window.scale_factor()?,
        })
    };
    read()
        .map_err(|e| {
            tracing::debug!(
                module = "dock",
                window = window.label(),
                "창 기하를 못 읽었다: {e}"
            )
        })
        .ok()
}

struct TauriDropShell<'a> {
    app: &'a AppHandle,
}

impl DropShell for TauriDropShell<'_> {
    // 이 길의 끌기 동안에는 키 감시가 돌지 않는다 — `set_focus` 가
    //   합성하는 Alt 를 놓기로 읽을 쪽이 없다.
    fn focus(&self, label: &str) {
        let focused = match self.app.get_webview_window(label) {
            Some(window) => window.set_focus().map_err(|e| e.to_string()),
            None => Err("창이 없다".to_owned()),
        };
        if let Err(e) = focused {
            tracing::debug!(
                module = "dock",
                window = label,
                "옮긴 탭이 든 창에 포커스를 못 줬다: {e}"
            );
        }
    }

    fn done(&self, source: &str, done: &TabDropDone) {
        // 없는 label 로의 `emit_to` 는 오류 없이 아무에게도 닿지 않는다 — 건너뜀을 로그로 남기려고 먼저 본다.
        if self.app.get_webview_window(source).is_none() {
            tracing::debug!(
                module = "dock",
                window = source,
                drag = %done.drag,
                "출처 창이 없어 {EVT_TAB_DROP_DONE} 을 건너뛴다"
            );
            return;
        }
        if let Err(e) = self.app.emit_to(source, EVT_TAB_DROP_DONE, done) {
            tracing::warn!(
                module = "dock",
                window = source,
                drag = %done.drag,
                "{EVT_TAB_DROP_DONE} emit 실패: {e}"
            );
        }
    }
}
