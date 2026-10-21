//! 도킹의 셸 쪽 창 관리 — 끌기 세션 밖에서 도는 것들(TRD S21-slot-docking §2-5): 웹뷰 측정 보고
//! ([`report_dock_chrome`]) · 창 역할 질의([`window_role`]) · 대기 창 명부와 안내 창의 `Destroyed` 갈래 · 예비 창 ·
//! 안내 창 짓기 · 앱 종료 표시([`on_app_exiting`]). 끌기 세션 자체는 `commands::drag` 가 세운다.
//!
//! ★락 규율★ — 대기 창 명부 락(`DockState::parked`)은 잎이다. 가드는 문장 하나 안에서 쓰고 놓고, 쥔 채 레이아웃 락 ·
//! emit · 창 호출 · 창 짓기를 하지 않는다. 레이아웃 락을 쥔 채 잡는 것은 된다(차례 = `DockState` 문서).

use std::sync::Arc;
use std::time::Instant;

use tauri::{AppHandle, Manager, State, WebviewUrl, WebviewWindow, WebviewWindowBuilder, Window};

use crate::commands::popout::PopupCounter;
use crate::dock::parked::{Measure, ParkedRole};
use crate::dock::wire::WindowRoleReply;
use crate::dock::DockState;
use crate::layout::apply;
use crate::layout::chrome::DockChrome;
use crate::layout::{LabelSource, LayoutState};
use crate::webview_env::{Backdrop, WebviewEnv};

/// 창 웹뷰가 탭 끌기 영역 · 캔버스 원점을 알린다(동기 · 측정) — `invoke('report_dock_chrome', { chrome })`. 계약은
/// [`DockChrome`] 문서. 대기 창 명부의 창이면 모델 대신 명부에 적는다(TRD §2-5). 탭 모델에도 명부에도 없는 창이면
/// `Err` 이고 직전 값을 유지한다. 버스 명령이 아니다(ADR-0227 — 측정은 위조할 수 없게 웹뷰만 보고한다).
#[tauri::command]
pub fn report_dock_chrome(
    window: Window,
    dock: State<'_, Arc<DockState>>,
    layout: State<'_, LayoutState>,
    chrome: DockChrome,
) -> Result<(), String> {
    match dock.record_parked(window.label(), Measure::Chrome(chrome.clone())) {
        Some(recorded) => recorded.map_err(|e| e.to_string())?,
        None => apply::report_dock_chrome(&layout, window.label(), chrome)?,
    }
    Ok(())
}

/// 부른 창의 지금 역할(동기) — `invoke('window_role')`. 웹뷰가 마운트 때 묻고 `parked` 면 대기 모드로 그린다(TRD §4 ·
/// §2-5). 웹뷰 안 배선이라 버스 명령표에 올리지 않는다.
#[tauri::command]
pub fn window_role(
    window: Window,
    dock: State<'_, Arc<DockState>>,
    layout: State<'_, LayoutState>,
) -> WindowRoleReply {
    let label = window.label();
    let role = dock.window_role(label, || {
        apply::list_windows(&layout).is_ok_and(|windows| windows.iter().any(|w| w == label))
    });
    WindowRoleReply { role, drag: None }
}

/// 대기 창 명부의 창인가 — 트레이 보이기 · 숨기기 · `--hidden` 과 자리 기록이 빼는 창(TRD §2-5).
pub(crate) fn is_parked_window(app: &AppHandle, label: &str) -> bool {
    app.try_state::<Arc<DockState>>()
        .is_some_and(|dock| dock.is_parked(label))
}

/// 창 `Destroyed` 의 명부 갈래(TRD §2-5 「사라지면」) — 그 창을 명부에서 빼고, 예비 창이었으면 명부에 예비(짓는 중
/// 포함)가 없을 때 작업 스레드에서 새로 짓는다(예비는 늘 하나 · G11). 닫는 창의 명부 항목을 지우는 곳은 여기 하나다.
/// 레이아웃 락 밖에서 부른다(`lib.rs`).
// ADR-0295
pub(crate) fn on_parked_window_destroyed(app: &AppHandle, label: &str) {
    let Some(dock) = app.try_state::<Arc<DockState>>() else {
        return;
    };
    let Some(gone) = dock.parked().remove(label) else {
        return;
    };
    match gone.role {
        ParkedRole::Spare if dock.exiting() => {
            tracing::debug!(
                module = "dock",
                label,
                "앱이 끝나는 중에 예비 창이 사라졌다 — 다시 짓지 않는다"
            );
        }
        ParkedRole::Spare => {
            tracing::warn!(module = "dock", label, "예비 창을 잃어 명부에서 뺐다");
            let has_spare = dock.parked().has_spare();
            if !has_spare {
                spawn_spare_build(app);
            }
        }
        ParkedRole::Kept => {
            tracing::debug!(module = "dock", label, "남긴 창이 사라져 명부에서 뺐다");
        }
    }
}

/// 창 `Destroyed` 의 안내 창 갈래(TRD §2-5 「안내 창」 「사라지면」) — 작업 스레드에서 다시 짓는다.
// ADR-0295
pub(crate) fn on_overlay_destroyed(app: &AppHandle, label: &str) {
    if !is_overlay_label(label) {
        return;
    }
    if exiting(app) {
        tracing::debug!(
            module = "dock",
            label,
            "앱이 끝나는 중에 안내 창이 사라졌다 — 다시 짓지 않는다"
        );
        return;
    }
    tracing::warn!(module = "dock", label, "안내 창이 사라져 다시 짓는다");
    spawn_overlay_build(app);
}

/// 앱 종료 요청 — 이 뒤로 사라지는 예비 창 · 안내 창은 다시 짓지 않는다(`lib.rs` 의 `run` 사건).
pub(crate) fn on_app_exiting(app: &AppHandle) {
    if let Some(dock) = app.try_state::<Arc<DockState>>() {
        dock.begin_exit();
    }
}

// ── 예비 창 · 안내 창 짓기(TRD §2-5 · §2-4) ─────────────────────────────────────
//
// 둘 다 미리 하나씩 지어 ★숨긴 채★ 기다린다 — 화면 밖 대기 자리를 두지 않는다(사용자 결정 2026-10-19). 숨긴 최상위 창은
// 작업 표시줄 · Alt+Tab 에 나오지 않는다. 대기 창 명부의 남긴 창(`Kept`)도 붙는 순간의 날 `SW_HIDE` 그대로
// 숨긴 채 기다린다 — 화면 밖으로 옮기지 않는다.
// ★숨긴 채 기다린 창의 첫 보이기 모습은 잰 적이 없다★ — 스파이크 S8 의 미리 만든 창(깜빡임 0)은 화면 밖에 보이는
// 채였다. 새면 증상 = 뗄 때 · 안내 창을 처음 그릴 때 검정 · 흰 · 지난 그림이 몇 프레임 보인다.
// ★작업 스레드에서 짓는다★ — 동기 IPC 처리기 안에서 지으면 Windows 에서 교착이고(스파이크), 메인 스레드의 사건
// 처리기(`Destroyed`)도 부르는 쪽이다.

/// 안내 창 label — 팝아웃 label 이 아니라 트레이 · 닫기 요청 · 명부의 대상이 아니다(TRD §2-5 · G10).
pub(crate) const DOCK_OVERLAY_LABEL: &str = "dock-overlay";
// 안내 창 경로(G10). ★웹뷰에 이 경로의 페이지가 없다★ — 이 창을 지으면 탭 창 앱(데몬 부팅 · 버스 명령 보고 등 전역 배선)이
//   돈다. 부팅은 짓지 않고 다시 짓기는 안내 창이 사라질 때만 돈다.
// TODO(U4c′-1): 안내 창 짓기 · 다시 짓기 몫을 걷는다(TRD S21-slot-docking §7).
const DOCK_OVERLAY_URL: &str = "index.html#/dock-overlay";
// 안내 창의 지을 때 크기(논리 px) — 그릴 때 대상 창 안쪽에 맞춰 다시 놓으므로 기다리는 동안의 값일 뿐이다(스파이크
//   setup 의 G 400 × 300 그대로).
const DOCK_OVERLAY_SIZE: (f64, f64) = (400.0, 300.0);

pub(crate) fn is_overlay_label(label: &str) -> bool {
    label == DOCK_OVERLAY_LABEL
}

/// 예비 창 하나를 작업 스레드에서 짓는다 — 명부에 예비(짓는 중 포함)가 이미 있거나 앱이 끝나는 중이면 짓지 않는다.
pub(crate) fn spawn_spare_build(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || build_spare(&app));
}

/// 안내 창을 작업 스레드에서 짓는다 — 앱이 끝나는 중이면 짓지 않는다.
pub(crate) fn spawn_overlay_build(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || build_overlay(&app));
}

fn exiting(app: &AppHandle) -> bool {
    app.try_state::<Arc<DockState>>()
        .is_some_and(|dock| dock.exiting())
}

fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

// 예비 창 — 명부에 `Spare` 로 적고(짓기 전 — 짓는 동안의 트레이 · `--hidden` 도 그 창을 보지 않게) · 숨긴 채 짓고 ·
//   지어짐을 세운다. 짓기에 실패하면 명부에서 뺀다 — 두면 「예비 있음」이 늘 참이라 다시 짓지 못한다. 기다리는 동안 이
//   창에는 창 호출을 하지 않는다 — tao 보임 깃발(거짓)이 OS 보임과 맞은 채다.
//
// ★들이는 차례(세션 포트 `place_detached` — 어댑터 `commands::drag_window`)★ — tao 보임 깃발이 늘 OS 보임과 맞게:
// 1. 숨긴 채 자리 · 크기 = platform `place_window(rect: Some(바깥 사각형), z: Keep, show: false, activate: false)`.
//    `SWP_SHOWWINDOW` 없는 `SetWindowPos` 한 번이라 창은 숨은 채고 tao 깃발도 건드리지 않으며, 값이 §5-5 셈의
//    단위(물리 px 바깥 사각형) 그대로다. Tauri `set_position` · `set_size` 는 쓰지 않는다 — 자리 · 크기가 따로 두 번
//    바뀌고 `set_size` 는 안쪽 크기다.
// 2. 보이기 = Tauri `show()` — tao 보임 깃발이 참이 돼 그 뒤 트레이 `show` · `hide` 가 그대로 돈다. 날 Win32 로 보이면
//    깃발이 거짓으로 남아 트레이 숨기기가 아무것도 하지 않는다(바뀐 깃발이 없으면 tao 0.35.3 `apply_diff` 가 돌아간다).
//    포커스 없이 지은 창이라 tao 는 활성화하지 않고 보인다(`SW_SHOWNOACTIVATE`).
// 3. 활성 = platform `place_window(rect: None, z: Keep, show: false, activate: true)` — Tauri `set_focus` 는 쓰지 않는다
//    (tao 가 Alt 를 합성해 키 감시가 놓기로 읽는다 — TRD F7 a).
// 예비로 되돌릴 때(`give_back`)는 Tauri `hide()` 로 숨긴다 — 날 `SW_HIDE` 면 tao 보임 깃발이 참으로 남아 다음
//   들이기의 2 가 아무것도 하지 않는다.
// ADR-0295
fn build_spare(app: &AppHandle) {
    let started = Instant::now();
    let (Some(dock), Some(labels)) = (
        app.try_state::<Arc<DockState>>(),
        app.try_state::<Arc<PopupCounter>>(),
    ) else {
        tracing::warn!(
            module = "dock",
            "도킹 상태 · 팝아웃 번호가 등록돼 있지 않아 예비 창을 짓지 못했다"
        );
        return;
    };
    if dock.exiting() {
        tracing::debug!(module = "dock", "앱이 끝나는 중이라 예비 창을 짓지 않는다");
        return;
    }
    let label = LabelSource::next_label(&**labels);
    let reserved = dock.parked().reserve_spare(label.clone());
    if !reserved {
        tracing::debug!(module = "dock", label, "예비 창이 이미 있어 짓지 않는다");
        return;
    }
    if let Err(e) = crate::commands::popout::build_spare_window(app, &label) {
        dock.parked().remove(&label);
        tracing::warn!(module = "dock", label, "예비 창을 짓지 못했다: {e}");
        return;
    }
    let built = dock.mark_built(&label);
    let build_ms = elapsed_ms(started);
    if built {
        tracing::debug!(
            module = "dock",
            label,
            role = "spare",
            build_ms,
            "예비 창을 지었다"
        );
    } else {
        tracing::debug!(
            module = "dock",
            label,
            role = "spare",
            build_ms,
            "예비 창이 짓는 동안 사라졌다"
        );
    }
}

// 안내 창 — 투명 · 테두리 · 그림자 없음 · 크기 고정 · 활성화 안 함 · 항상 위 · 클릭 통과(TRD §2-5 · 스파이크 G). 숨긴
//   채 짓고 보이지 않는다 — 끄는 동안의 보이기 · 숨기기 · 자리 · z 는 platform 함수(날 Win32)로만 한다. 그래서 tao 보임
//   깃발은 늘 거짓이다.
// ★지은 뒤로 이 창에는 tao · Tauri 창 깃발 setter 를 부르지 않는다★ — 깃발이 하나라도 바뀌면 tao 가 제 보임 깃발대로
//   창을 숨기고(`SW_HIDE`) `GWL_EXSTYLE` 을 제 깃발로 통째 다시 써 Alt+Tab 빼기를 되돌린다(TRD §2-5 「tao 깃발
//   불변식」). 지금 그런 호출은 아래 짓기 걸음뿐이다.
// ADR-0295
fn build_overlay(app: &AppHandle) {
    let started = Instant::now();
    if exiting(app) {
        tracing::debug!(module = "dock", "앱이 끝나는 중이라 안내 창을 짓지 않는다");
        return;
    }
    // 웹뷰 환경 없이 만들지 않는다 — 마무리를 건너뛴 창은 다른 창과 환경이 갈린다(ADR-0054).
    let Some(env) = app.try_state::<Arc<WebviewEnv>>() else {
        tracing::warn!(
            module = "dock",
            "웹뷰 환경이 등록돼 있지 않아 안내 창을 짓지 못했다"
        );
        return;
    };
    let (w, h) = DOCK_OVERLAY_SIZE;
    let built = env
        .finish(
            WebviewWindowBuilder::new(
                app,
                DOCK_OVERLAY_LABEL,
                WebviewUrl::App(DOCK_OVERLAY_URL.into()),
            )
            .title(format!("Engram — {DOCK_OVERLAY_LABEL}"))
            .inner_size(w, h)
            .visible(false)
            .focused(false)
            .transparent(true)
            .decorations(false)
            .shadow(false)
            .resizable(false)
            .always_on_top(true),
            Backdrop::Unpainted,
        )
        .build();
    let window = match built {
        Ok(window) => window,
        Err(e) => {
            tracing::warn!(module = "dock", "안내 창을 짓지 못했다: {e}");
            return;
        }
    };
    let posted = app.run_on_main_thread(move || {
        exclude_overlay(&window);
        tracing::debug!(
            module = "dock",
            label = window.label(),
            role = "overlay",
            build_ms = elapsed_ms(started),
            "안내 창을 지었다"
        );
    });
    if let Err(e) = posted {
        tracing::warn!(
            module = "dock",
            "안내 창을 다 짓지 못했다(메인 스레드에 못 넘김): {e}"
        );
    }
}

// 지은 안내 창의 깃발 — 메인 스레드에서 한 걸음에. tao 쪽(클릭 통과 · 작업 표시줄 — tao `skip_taskbar` 는 깃발을 바꾸지
//   않는다)을 다 한 뒤 Alt+Tab(platform `set_switcher_hidden`)에서 뺀다(E2). 숨은 동안에는 쓸모가 없지만 끄는 동안 날
//   Win32 로 보일 때 이미 서 있어야 한다. ★메인 스레드여야 하는 까닭★ — 다른 스레드에서 부른 Tauri setter 는
//   이벤트 루프 줄로 넘어가 나중에 돌아, 날 Win32 인 Alt+Tab 빼기가 먼저 돌면 그 뒤의 tao 깃발 바꿈이 `GWL_EXSTYLE` 을
//   통째 다시 써 되돌린다. 실패는 한 줄로 남기고 넘어간다 — 창은 그대로 안내 창이다.
// ADR-0295
fn exclude_overlay(window: &WebviewWindow) {
    let mut failed: Vec<String> = Vec::new();
    if let Err(e) = window.set_ignore_cursor_events(true) {
        failed.push(format!("click_through: {e}"));
    }
    if let Err(e) = window.set_skip_taskbar(true) {
        failed.push(format!("skip_taskbar: {e}"));
    }
    if let Err(e) = engram_dashboard_platform::window::set_switcher_hidden(window, true) {
        failed.push(format!("switcher_hidden: {e}"));
    }
    if !failed.is_empty() {
        tracing::warn!(
            module = "dock",
            label = window.label(),
            ?failed,
            "안내 창의 깃발을 다 세우지 못했다"
        );
    }
}
