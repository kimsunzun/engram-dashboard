//! Two tab-drop invoke shells (`tab_drop` · `window_frame`) and their Tauri adapter — reading input after a drag ends (platform) ·
//! window getters · emitting events (`tab_drop:done` · `window:frame`). Processing · decision · ledger are done by `crate::tab_drop`.
//! `window:frame` is pushed not only for place recording but also on that window's focus event ([`push_window_focus`]), and carries in its body the front-window field (`focused`) and
//! the caption look (`caption` — platform `caption_style`) (TRD S26-window-chrome §6-5).
//!
//! Both are wiring inside the webview, so they are not put in the bus command table — a drop is the end of a mouse release and has a branch that reads input state, so the LLM
//! must not imitate it. The LLM command for the same action is T-65 `tab.dock` on the same `layout::apply::dock_tab`.
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

/// On each drop handling, only to the source window (the command's `from`) — body = [`TabDropDone`].
pub const EVT_TAB_DROP_DONE: &str = "tab_drop:done";
/// Only to that window · only when the value changes — body = [`WindowFrame`].
pub const EVT_WINDOW_FRAME: &str = "window:frame";

/// Tab drop (async — it may create a new window) — `invoke('tab_drop', { drop })`. The contract of argument · answer is [`TabDrop`] ·
/// [`TabDropReply`]. Calling window = the window Tauri injects. The result goes, separately from the answer, to the source window as `tab_drop:done` (excluding duplicates of the same
/// drag · a rejection that yielded its place to a `drop` held in custody — [`TabDropDone`]). It is always `Ok` — failure is also a branch of the answer.
///
/// Input is read as soon as it is received, before the lock · ledger — for a command sent by the source dragend: the cursor · primary button · Esc · the topmost window under the cursor;
/// for a drop in our webview: the cursor only. It is a call after the drag has ended (the drag command only on release — ADR-0296 · ADR-0297).
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
    // The source window frame is read after the input is read — so the getter round trip does not delay reading the input at the moment of release.
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

/// The calling window's window facts (synchronous) — `invoke('window_frame')`. The webview pulls once at mount, and later changes are pushed by `window:frame`.
/// Restore size · maximize = model values · frame = value counted by the getters now (rule = `tab_drop::pull_frame`) · front window = the current OS
/// foreground-window decision. If the window is not in the tab model, `Err`.
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
        .map_err(|e| e.to_string())?; // ← layout lock dropped — the getters are outside it
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

/// Called after the place-recording path (`state::placement::record`) has written to the model — emits `window:frame` to that window only when that window's window facts changed
/// (rule = `tab_drop::push_frame`). Front window = the current OS foreground-window decision. ★Called without holding any lock★
/// — it calls the getters.
// TRD S21-slot-docking §3-8
pub(crate) fn push_window_frame(window: &Window, minimized: bool, attrs: WindowAttrs) {
    let focused = crate::commands::window_ctl::is_foreground(window);
    push(window, minimized, attrs, focused);
}

/// Called on that window's focus event (`WindowEvent::Focused(focused)`) — the front-window field is ★the event's value★: when the event fires the OS foreground-window
/// decision may not have changed yet (order unverified · TRD S26-window-chrome §6-5 「`focused` 값의 출처」 · S12). A window not in the tab
/// model (a prewarmed window · an event that arrived late after closing) is skipped. ★Called without holding any lock★.
pub(crate) fn push_window_focus(window: &Window, focused: bool) {
    let Some(layout) = window.app_handle().try_state::<LayoutState>() else {
        return;
    };
    // The layout lock only reads the value and is released right away — the getters are outside it.
    let attrs = match layout.0.lock() {
        Ok(mgr) => mgr.window_attrs(window.label()).ok(),
        Err(_) => {
            // The dock-held-lock log is debug — the same reason as `state::placement::record`.
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

// The chrome look of the window facts — platform's OS constant plus whether the shell build has the snap overlay (TRD S26-window-chrome §3-2 · §8-4).
fn caption_chrome() -> CaptionChrome {
    let style = os::caption_style();
    CaptionChrome {
        buttons: style.drawn_buttons,
        leading_inset: style.leading_inset,
        // True only in a build made with the shell feature `snap-overlay` (default off — §8-4) — only when true does the web call `report_max_button_rect`.
        snap_overlay: cfg!(feature = "snap-overlay"),
    }
}

/// Called after the window is gone (`Destroyed`) — forget that window's last window facts (rule = `tab_drop::FrameBook::forget`).
pub(crate) fn forget_window_frame(app: &AppHandle, label: &str) {
    if let Some(frames) = app.try_state::<Arc<FrameBook>>() {
        frames.forget(label);
    }
}

// Input read after the drag ended — `top_class` = if the top is another app's window, that window's class (the log's share — measured material to tell the drag-picture window apart).
struct ReadInput {
    cursor: Option<PhysPoint>,
    primary_down: bool,
    escape_down: bool,
    top: TopAt,
    top_class: Option<String>,
    monitor_scale: Option<f64>,
}

// Order = cursor → primary button → Esc → topmost window under the cursor → monitor scale under the cursor. A drop reads only the cursor (the drop event is the
//   evidence of the release). `places` = there is a new window place to be counted (reads the monitor scale).
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

// The topmost top-level window under a point — if it is our webview window, that label; otherwise another app (with that window's class), and if unreadable, `Unknown`.
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

// The four window getters and the scale — if even one cannot be read, `None` (one debug line).
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
    // During a drag on this path the key watch does not run — there is no one to read the Alt that `set_focus`
    //   synthesizes as a release.
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
        // `emit_to` to a nonexistent label reaches no one without an error — checked first so that the skip can be logged.
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
