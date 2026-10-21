//! 도킹의 셸 쪽 창 관리(TRD S21-slot-docking §2-5): 창 역할 질의([`window_role`]) · 대기 창 명부의 `Destroyed`
//! 갈래 · 예비 창 짓기 · 앱 종료 표시([`on_app_exiting`]).
//!
//! ★락 규율★ — 대기 창 명부 락(`DockState::parked`)은 잎이다. 가드는 문장 하나 안에서 쓰고 놓고, 쥔 채 레이아웃 락 ·
//! emit · 창 호출 · 창 짓기를 하지 않는다. 레이아웃 락을 쥔 채 잡는 것은 된다(차례 = `DockState` 문서).

use std::sync::Arc;
use std::time::Instant;

use tauri::{AppHandle, Manager, State, Window};

use crate::commands::popout::PopupCounter;
use crate::dock::wire::WindowRoleReply;
use crate::dock::DockState;
use crate::layout::apply;
use crate::layout::{LabelSource, LayoutState};

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

/// 대기 창 명부의 창 모드가 바뀌었다 — 몸 = [`SpareEvent`](crate::dock::wire::SpareEvent) · 받는 쪽 = 그 창 웹뷰(웹뷰
/// 쪽 이름 = `spareMode.ts` 의 `EVT_SPARE`). ★지금 내는 쪽이 없다★ — 예열 창 채택(TRD
/// S21-slot-docking §3-9 · U4d′)이 낸다.
pub const EVT_SPARE: &str = "dock:spare";

/// 대기 창 명부의 창인가 — 트레이 보이기 · 숨기기 · `--hidden` 과 자리 기록이 빼는 창(TRD §2-5).
pub(crate) fn is_parked_window(app: &AppHandle, label: &str) -> bool {
    app.try_state::<Arc<DockState>>()
        .is_some_and(|dock| dock.is_parked(label))
}

/// 창 `Destroyed` 의 명부 갈래(TRD §2-5 「사라지면」) — 그 창을 명부에서 빼고, 앱이 끝나는 중이 아니면 명부에
/// 예비(짓는 중 포함)가 없을 때 작업 스레드에서 새로 짓는다(예비는 늘 하나 · G11). 닫는 창의 명부 항목을 지우는 곳은 여기 하나다.
/// 레이아웃 락 밖에서 부른다(`lib.rs`).
// ADR-0295
pub(crate) fn on_parked_window_destroyed(app: &AppHandle, label: &str) {
    let Some(dock) = app.try_state::<Arc<DockState>>() else {
        return;
    };
    if dock.parked().remove(label).is_none() {
        return;
    }
    if dock.exiting() {
        tracing::debug!(
            module = "dock",
            label,
            "앱이 끝나는 중에 예비 창이 사라졌다 — 다시 짓지 않는다"
        );
        return;
    }
    tracing::warn!(module = "dock", label, "예비 창을 잃어 명부에서 뺐다");
    let has_spare = dock.parked().has_spare();
    if !has_spare {
        spawn_spare_build(app);
    }
}

/// 앱 종료 요청 — 이 뒤로 사라지는 예비 창은 다시 짓지 않는다(`lib.rs` 의 `run` 사건).
pub(crate) fn on_app_exiting(app: &AppHandle) {
    if let Some(dock) = app.try_state::<Arc<DockState>>() {
        dock.begin_exit();
    }
}

// ── 예비 창 짓기(TRD §2-5 · §2-4) ─────────────────────────────────────
//
// 미리 하나 지어 ★숨긴 채★ 기다린다 — 화면 밖 대기 자리를 두지 않는다(사용자 결정 2026-10-19). 숨긴 최상위 창은
// 작업 표시줄 · Alt+Tab 에 나오지 않는다.
// ★숨긴 채 기다린 창의 첫 보이기 모습은 잰 적이 없다★ — 스파이크 S8 의 미리 만든 창(깜빡임 0)은 화면 밖에 보이는
// 채였다. 새면 증상 = 그 창을 처음 보일 때 검정 · 흰 · 지난 그림이 몇 프레임 보인다.
// ★작업 스레드에서 짓는다★ — 동기 IPC 처리기 안에서 지으면 Windows 에서 교착이고(스파이크), 메인 스레드의 사건
// 처리기(`Destroyed`)도 부르는 쪽이다.

/// 예비 창 하나를 작업 스레드에서 짓는다 — 명부에 예비(짓는 중 포함)가 이미 있거나 앱이 끝나는 중이면 짓지 않는다.
pub(crate) fn spawn_spare_build(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || build_spare(&app));
}

fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

// 예비 창 — 명부에 적고(짓기 전 — 짓는 동안의 트레이 · `--hidden` 도 그 창을 보지 않게) · 숨긴 채 짓고 ·
//   지어짐을 세운다. 짓기에 실패하면 명부에서 뺀다 — 두면 「예비 있음」이 늘 참이라 다시 짓지 못한다. 기다리는 동안 이
//   창에는 보임을 바꾸는 창 호출을 하지 않는다 — tao 보임 깃발(거짓)이 OS 보임과 맞은 채다.
// ★보일 때는 Tauri `show()` 로★ — 날 Win32 로 보이면 깃발이 거짓으로 남아 트레이 숨기기가 아무것도 하지 않는다(바뀐
//   깃발이 없으면 tao 0.35.3 `apply_diff` 가 돌아간다). 포커스 없이 지은 창이라 tao 는 활성화하지 않고 보인다
//   (`SW_SHOWNOACTIVATE`).
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
