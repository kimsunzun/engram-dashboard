//! tray 커맨드 — 트레이 메뉴 동작을 LLM/cdp 가 invoke 로 호출하는 §5 제어 표면.
//!
//! 데몬 켜기/끄기는 기존 daemon_start/daemon_stop/daemon_status(commands/discovery.rs)를 재사용하므로
//! 여기 없음.

use crate::tray::actions;

// main 창 보이기(show+unminimize+focus). 트레이 "UI 보이기"와 동일 동작.
#[tauri::command]
pub fn show_main_ui(app: tauri::AppHandle) {
    actions::show_main_ui(&app);
}

// 트레이 "UI 숨기기"·X=hide 와 동일 종착.
#[tauri::command]
pub fn hide_main_ui(app: tauri::AppHandle) {
    actions::hide_main_ui(&app);
}

// 앱 완전 종료(best-effort 데몬 graceful stop 후 exit). 트레이 "완전 종료"와 동일.
#[tauri::command]
pub fn quit_app(app: tauri::AppHandle) {
    actions::quit_app(&app);
}
