//! 예열 창의 셸 쪽 창 관리(TRD S21-slot-docking §3-9): 창 역할 질의([`window_role`]) · 창 `Destroyed` 의 풀 갈래 ·
//! 예열 창 짓기 · 앱 종료 표시([`on_app_exiting`]). 상태 기계는 `crate::spare` 에 있고 여기는 Tauri 어댑터다.
//!
//! ★락 차례 = 레이아웃 → 예열 창 풀 → 첫 화면 문★(`SpareState` 문서) — 풀 락을 쥔 채 잡는 것은 첫 화면 문의 잎 락
//! 하나뿐이고, 쥔 채 레이아웃 락 · emit · 창 호출 · 창 짓기를 하지 않는다.

use std::sync::Arc;

use tauri::{AppHandle, Manager, State, Window};

use crate::commands::popout::PopupCounter;
use crate::layout::apply;
use crate::layout::LayoutState;
pub(crate) use crate::spare::elapsed_ms;
use crate::spare::wire::{WindowRole, WindowRoleReply};
use crate::spare::SpareState;

/// 부른 창의 지금 역할(동기) — `invoke('window_role')`. 웹뷰가 마운트 때 묻고 `spare` 면 대기 모드로 그린다(TRD
/// §3-9). 채택 뒤 보임을 기다리는 창이면 그 채택 번호도 싣는다(I26). 웹뷰 안 배선이라 버스 명령표에 올리지 않는다.
#[tauri::command]
pub fn window_role(
    window: Window,
    spare: State<'_, Arc<SpareState>>,
    layout: State<'_, LayoutState>,
) -> WindowRoleReply {
    let label = window.label();
    let role = spare.window_role(label, || {
        apply::list_windows(&layout).is_ok_and(|windows| windows.iter().any(|w| w == label))
    });
    // 번호는 역할을 본 뒤 읽고 모델 창에만 싣는다 — 확정 ① 전의 `spare` 답에 ① 뒤의 번호가 붙지 않는다.
    let generation = match role {
        WindowRole::Model => spare.reveal_generation(label),
        WindowRole::Spare | WindowRole::Unknown => None,
    };
    WindowRoleReply { role, generation }
}

/// 예열 창 풀의 창인가 — 트레이 보이기 · 숨기기 · `--hidden` 과 자리 기록이 빼는 창(TRD §3-9).
pub(crate) fn is_spare_window(app: &AppHandle, label: &str) -> bool {
    app.try_state::<Arc<SpareState>>()
        .is_some_and(|spare| spare.is_spare(label))
}

/// 창 `Destroyed` 의 풀 갈래(TRD §3-9 「다시 짓기」) — 풀의 창이면 풀에서 빼고 다시 짓고, 보임을 기다리던 채택 창이면
/// 다시 짓기 완료다(둘 다 종료 중 · 풀 꺼짐이면 짓지 않는다 · 판정 = `SpareState::on_destroyed`). 닫는 창의 풀 항목을
/// 지우는 곳은 여기 하나다. 레이아웃 락 밖에서 부른다(`lib.rs`).
// ADR-0295 · ADR-0299
pub(crate) fn on_spare_window_destroyed(app: &AppHandle, label: &str) {
    let Some(spare) = app.try_state::<Arc<SpareState>>() else {
        return;
    };
    if spare.on_destroyed(label) {
        spawn_spare_build(app);
    }
}

/// 앱 종료 요청 — 이 뒤로 사라지는 예열 창은 다시 짓지 않는다(`lib.rs` 의 `run` 사건).
pub(crate) fn on_app_exiting(app: &AppHandle) {
    if let Some(spare) = app.try_state::<Arc<SpareState>>() {
        spare.begin_exit();
    }
}

// ── 예열 창 짓기(TRD §3-9 「부팅」 · 「다시 짓기」) ─────────────────────────────────────
//
// 미리 하나 지어 ★숨긴 채★ 기다린다 — 화면 밖 대기 자리를 두지 않는다(사용자 결정 2026-10-19). 숨긴 최상위 창은
// 작업 표시줄 · Alt+Tab 에 나오지 않는다.
// ★숨긴 채 기다린 창의 첫 보이기 모습은 잰 적이 없다★ — 스파이크 S8 의 미리 만든 창(깜빡임 0)은 화면 밖에 보이는
// 채였다. 새면 증상 = 그 창을 처음 보일 때 검정 · 흰 · 지난 그림이 몇 프레임 보인다.
// ★작업 스레드에서 짓는다★ — 동기 IPC 처리기 안에서 지으면 Windows 에서 교착이고(스파이크), 메인 스레드의 사건
// 처리기(`Destroyed`)도 부르는 쪽이다.

/// 예열 창 하나를 작업 스레드에서 짓는다 — 풀에 예열 창(짓는 중 포함)이 이미 있거나 · 보임을 기다리는 채택 창이
/// 있거나 · 앱이 끝나는 중 · 풀이 꺼졌으면 짓지 않고 팝아웃 번호도 받지 않는다(`SpareState::build`).
pub(crate) fn spawn_spare_build(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || build_spare(&app));
}

// 기다리는 동안 이 창에는 보임을 바꾸는 창 호출을 하지 않는다 — tao 보임 깃발(거짓)이 OS 보임과 맞은 채다.
// ★보일 때는 Tauri `show()` 로★ — 날 Win32 로 보이면 깃발이 거짓으로 남아 트레이 숨기기가 아무것도 하지 않는다(바뀐
//   깃발이 없으면 tao 0.35.3 `apply_diff` 가 돌아간다). 포커스 없이 지은 창이라 tao 는 활성화하지 않고 보인다
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
