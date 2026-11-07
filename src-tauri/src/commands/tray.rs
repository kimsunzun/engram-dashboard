//! tray command — a §5 control surface through which the LLM/cdp invokes tray menu actions.
//!
//! Daemon on/off reuses the existing daemon_start/daemon_stop/daemon_status (commands/discovery.rs), so it is
//! not here.

use crate::commands::window_ctl::TauriWindowControl;
use crate::layout::window_ctl::{Activation, WindowControl};
use crate::tray::actions;

/// Revive main and all pop-out windows — a show that does not steal focus, the same as bus `window.showAll` (ADR-0229 · TRD
/// S26-window-chrome §6-2 「누가 어느 갈래인가」): a window before first paint is skipped, and focus is given only to main when our app is already the foreground.
/// ★Do not revert it to the tray left-click (press branch)★ — this is a surface called by the LLM · cdp, and the press branch would become a second way to steal
/// another app's foreground.
#[tauri::command]
pub fn show_main_ui(app: tauri::AppHandle) {
    TauriWindowControl::new(app).show_all(Activation::NoSteal);
}

/// Hide main and all pop-out windows — the same action as main X (ADR-0229).
#[tauri::command]
pub fn hide_main_ui(app: tauri::AppHandle) {
    actions::hide_main_ui(&app);
}

// Full app quit (best-effort daemon graceful stop, then exit). Same as tray "완전 종료" (full quit).
#[tauri::command]
pub fn quit_app(app: tauri::AppHandle) {
    actions::quit_app(&app);
}
