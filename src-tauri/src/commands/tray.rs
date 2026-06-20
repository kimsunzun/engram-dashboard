//! tray 커맨드 — 트레이 메뉴 동작을 LLM/cdp 가 invoke 로 호출하는 §5 제어 표면.
//!
//! ## 왜 command 가 필요한가 (CLAUDE.md §5 — load-bearing)
//! 네이티브 트레이 팝업은 WebView DOM 이 아니라 cdp 가 직접 클릭 못 한다. 그래서 트레이 메뉴와
//! **같은 내부 함수**(tray::actions)를 command 로도 노출해 LLM/cdp 가 호출·검증하게 한다.
//! 동작 로직은 actions 에 한 벌만 — 여기는 thin wrapper(중복 금지). 데몬 켜기/끄기는 기존
//! daemon_start/daemon_stop/daemon_status(commands/discovery.rs)를 재사용하므로 여기 없음.

use crate::tray::actions;

/// main 창 보이기(show+unminimize+focus). 트레이 "UI 보이기"와 동일 동작.
#[tauri::command]
pub fn show_main_ui(app: tauri::AppHandle) {
    actions::show_main_ui(&app);
}

/// main 창 숨기기(hide). 트레이 "UI 숨기기"·X=hide 와 동일 종착.
#[tauri::command]
pub fn hide_main_ui(app: tauri::AppHandle) {
    actions::hide_main_ui(&app);
}

/// 앱 완전 종료(best-effort 데몬 graceful stop 후 exit). 트레이 "완전 종료"와 동일.
#[tauri::command]
pub fn quit_app(app: tauri::AppHandle) {
    actions::quit_app(&app);
}
