//! tray 커맨드 — 트레이 메뉴 동작을 LLM/cdp 가 invoke 로 호출하는 §5 제어 표면.
//!
//! 데몬 켜기/끄기는 기존 daemon_start/daemon_stop/daemon_status(commands/discovery.rs)를 재사용하므로
//! 여기 없음.

use crate::tray::actions;

/// 메인과 팝아웃 창 전부를 화면 맨 앞으로 되살리고 마지막에 메인에 포커스 — 트레이 좌클릭과 같은
/// 동작(ADR-0229).
#[tauri::command]
pub fn show_main_ui(app: tauri::AppHandle) {
    actions::show_main_ui(&app);
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
