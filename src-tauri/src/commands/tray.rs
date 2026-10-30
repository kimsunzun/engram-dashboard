//! tray 커맨드 — 트레이 메뉴 동작을 LLM/cdp 가 invoke 로 호출하는 §5 제어 표면.
//!
//! 데몬 켜기/끄기는 기존 daemon_start/daemon_stop/daemon_status(commands/discovery.rs)를 재사용하므로
//! 여기 없음.

use crate::commands::window_ctl::TauriWindowControl;
use crate::layout::window_ctl::{Activation, WindowControl};
use crate::tray::actions;

/// 메인과 팝아웃 창 전부를 되살린다 — 버스 `window.showAll` 과 같은 포커스를 빼앗지 않는 보이기다(ADR-0229 · TRD
/// S26-window-chrome §6-2 「누가 어느 갈래인가」): 첫 화면 전인 창은 건너뛰고, 포커스는 우리 앱이 이미 전경일 때 main 에만
/// 준다. ★트레이 좌클릭(누름 갈래)으로 되돌리지 말 것★ — LLM · cdp 가 부르는 표면이라, 누름 갈래면 남의 앱 전경을
/// 빼앗는 두 번째 길이 된다.
#[tauri::command]
pub fn show_main_ui(app: tauri::AppHandle) {
    TauriWindowControl::new(app).show_all(Activation::NoSteal);
}

/// 메인과 팝아웃 창 전부를 숨긴다 — 메인 X 와 같은 동작(ADR-0229).
#[tauri::command]
pub fn hide_main_ui(app: tauri::AppHandle) {
    actions::hide_main_ui(&app);
}

// 앱 완전 종료(best-effort 데몬 graceful stop 후 exit). 트레이 "완전 종료"와 동일.
#[tauri::command]
pub fn quit_app(app: tauri::AppHandle) {
    actions::quit_app(&app);
}
