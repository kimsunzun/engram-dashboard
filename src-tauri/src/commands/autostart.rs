//! autostart command — §5 LLM/cdp control surface.
//!
//! ## What (ADR-0027 §53~55, ADR-0029)
//! - `set_autostart`/`get_autostart`: toggle/query start at boot (registry Run, Windows). Touches the **same state**
//!   (autolaunch() State) as the tray CheckMenuItem — the two acting parties (a human's tray click / an
//!   LLM invoke) see the same registry key.
//!
//! ADR-0029: the mode concept was removed (embedded/daemon unified). The old `set_mode` (mode self-relaunch switch) was deleted.

use tauri_plugin_autostart::ManagerExt;

// The autolaunch() State builds the registry Run entry from the args given at init() registration (`--hidden`) —
// enable=register, disable=delete.
#[tauri::command]
pub fn set_autostart(app: tauri::AppHandle, enable: bool) -> Result<(), String> {
    let mgr = app.autolaunch();
    if enable { mgr.enable() } else { mgr.disable() }.map_err(|e| e.to_string())
}

// Query whether start at boot is active (whether the registry Run entry exists).
#[tauri::command]
pub fn get_autostart(app: tauri::AppHandle) -> Result<bool, String> {
    app.autolaunch().is_enabled().map_err(|e| e.to_string())
}
