//! autostart 커맨드 — §5 LLM/cdp 제어 표면.
//!
//! ## 무엇 (ADR-0027 §53~55, ADR-0029)
//! - `set_autostart`/`get_autostart`: 부팅 자동 시작(레지스트리 Run, Windows) 토글/조회. 트레이
//!   CheckMenuItem 과 **같은 상태**(autolaunch() State)를 만진다 — 두 조작 주체(사람 트레이 클릭 /
//!   LLM invoke)가 같은 레지스트리 키를 본다.
//!
//! ADR-0029: 모드 개념 제거(embedded/daemon 통일). 옛 `set_mode`(모드 self-relaunch 전환)는 삭제됐다.

use tauri_plugin_autostart::ManagerExt;

/// 부팅 자동 시작 켜기/끄기(§5 LLM/트레이 공용 표면).
///
/// autolaunch() State 는 init() 등록 시의 args(`--hidden`)로 레지스트리 Run 엔트리를 구성한다 —
/// enable=등록, disable=삭제. 플러그인 등록 ≠ 활성화: 기본 OFF, 이 command/트레이 토글로만 켠다.
#[tauri::command]
pub fn set_autostart(app: tauri::AppHandle, enable: bool) -> Result<(), String> {
    let mgr = app.autolaunch();
    if enable { mgr.enable() } else { mgr.disable() }.map_err(|e| e.to_string())
}

/// 부팅 자동 시작 활성 여부 조회(레지스트리 Run 엔트리 존재).
#[tauri::command]
pub fn get_autostart(app: tauri::AppHandle) -> Result<bool, String> {
    app.autolaunch().is_enabled().map_err(|e| e.to_string())
}
