//! autostart / 모드전환 커맨드 — §5 LLM/cdp 제어 표면.
//!
//! ## 무엇 (ADR-0027 §53~55)
//! - `set_autostart`/`get_autostart`: 부팅 자동 시작(레지스트리 Run, Windows) 토글/조회. 트레이
//!   CheckMenuItem 과 **같은 상태**(autolaunch() State)를 만진다 — 두 조작 주체(사람 트레이 클릭 /
//!   LLM invoke)가 같은 레지스트리 키를 본다.
//! - `set_mode`: 모드 즉시 전환(embedded↔daemon) = Windows self-relaunch(current_exe 를 깨끗한 argv 로
//!   재기동 + 자신 exit). embedded 엔 트레이가 없어(ADR-0027 B안) command 가 1차 전환 진입.

use tauri_plugin_autostart::ManagerExt;

/// 부팅 자동 시작 켜기/끄기(§5 LLM/트레이 공용 표면).
///
/// autolaunch() State 는 init() 등록 시의 args(`--mode=daemon --hidden`)로 레지스트리 Run 엔트리를
/// 구성한다 — enable=등록, disable=삭제. 플러그인 등록 ≠ 활성화: 기본 OFF, 이 command/트레이 토글로만 켠다.
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

/// 모드 전환(Windows self-relaunch, ADR-0027 §53). 현재 exe 를 깨끗한 argv(--mode=<target> 만)로
/// 새로 띄우고 자신은 exit — 원래 argv 의 --mode/--hidden 을 떼어내므로 부팅 경로(autostart 포함)와
/// 무관하게 항상 요청 모드로 전환된다(set_var+restart 는 재전달 argv 의 --mode 가 env 를 이겨 실패했음).
/// 모드는 항상 뒤집히므로(embedded↔daemon) new 와 old 의 single-instance 키 공간이 달라
/// (embedded=Global\Engram-<hash> vs daemon=플러그인 Local) 핸드오프 중 same-key 충돌 없음(reviewer 확인).
/// §5: LLM/cdp/(미래)UI 공용 진입(embedded엔 트레이 없어 command 가 1차).
/// ★데몬 exe 직접 spawn 금지(ADR-0024 C1)는 데몬 한정 — 여기는 앱 자신 재기동이라 무관.★
/// ★한계(주석 유지): 새 프로세스 부팅과 old 의 shutdown_all 이 겹치는 짧은 창에 agents.json 핸드오프
///   경합 가능(atomic write 로 완화, self-relaunch 의 본질적 전이라 set_var+restart 도 동일). flip 후 재검토.★
/// ★QA 실측 필요: 재기동 후 window.__ENGRAM_MODE__ 가 요청 모드인지.★
#[tauri::command]
pub fn set_mode(app: tauri::AppHandle, mode: String) -> Result<(), String> {
    match mode.as_str() {
        "embedded" | "daemon" => {
            let exe = std::env::current_exe().map_err(|e| e.to_string())?;
            // 깨끗한 argv: --mode=<target> 만(원래 --mode/--hidden 제거). 다른 인자는 현재 우리 앱이
            // 부팅 인자를 안 쓰므로 불필요 — 새로 추가되면 여기서 보존 정책 재고.
            std::process::Command::new(exe)
                .arg(format!("--mode={mode}"))
                .spawn()
                .map_err(|e| format!("self-relaunch spawn 실패: {e}"))?;
            // 새 프로세스 spawn 성공 → 자신 종료(ExitRequested → shutdown_all graceful). exit 는 이벤트
            // 루프에서 처리되므로 이 command 는 Ok 반환 후 곧 종료된다.
            app.exit(0);
            Ok(())
        }
        other => Err(format!("unknown mode: {other}")),
    }
}

#[cfg(test)]
mod tests {
    /// set_mode 의 검증 분기(unknown mode → Err) 만 순수 추출해 테스트한다.
    /// 정식 set_mode 는 spawn+app.exit(OS 부수효과)라 단위로는 분기 진입만 확인 가능 → 동일 match 로직을 분리 단언.
    fn validate_mode(mode: &str) -> Result<(), String> {
        match mode {
            "embedded" | "daemon" => Ok(()),
            other => Err(format!("unknown mode: {other}")),
        }
    }

    #[test]
    fn known_modes_ok() {
        assert!(validate_mode("embedded").is_ok());
        assert!(validate_mode("daemon").is_ok());
    }

    #[test]
    fn unknown_mode_err() {
        assert_eq!(validate_mode("xxx"), Err("unknown mode: xxx".into()));
        assert_eq!(validate_mode(""), Err("unknown mode: ".into()));
    }
}
