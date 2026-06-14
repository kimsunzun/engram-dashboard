//! discovery 커맨드 — LLM/프론트가 데몬 발견을 호출하는 thin wrapper(§5 제어 표면).
//!
//! 비즈니스 로직 없음 — discovery::ensure_daemon 호출만. 실제 부팅 자동 호출 배선은
//! phase4 DaemonClient(WS) 와 함께 한다(이번 단위는 command 노출까지).

use std::time::Duration;

use crate::discovery::{self, locate_daemon_exe};

/// 프론트로 넘기는 DaemonInfo. token 을 그대로 노출(로컬 IPC) 하나 **로그 금지**.
#[derive(serde::Serialize)]
pub struct DaemonInfoDto {
    pub pid: u32,
    pub host: String,
    pub port: u16,
    pub token: String,
    pub protocol_version: u32,
}

impl From<engram_dashboard_protocol::DaemonInfo> for DaemonInfoDto {
    fn from(i: engram_dashboard_protocol::DaemonInfo) -> Self {
        Self {
            pid: i.pid,
            host: i.host,
            port: i.port,
            token: i.token,
            protocol_version: i.protocol_version,
        }
    }
}

/// 데몬을 발견(없으면 WMI spawn)하고 접속 정보를 반환한다.
///
/// data_dir 은 호출자가 안 줘도 되게 Tauri app_data_dir 을 쓴다(Embedded 와 동일 경로).
/// timeout_ms 미지정 시 5초.
#[tauri::command]
pub async fn discover_daemon(
    app: tauri::AppHandle,
    timeout_ms: Option<u64>,
) -> Result<DaemonInfoDto, String> {
    use tauri::Manager;

    let data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("app_data_dir 조회 실패: {e}"))?;
    let exe = locate_daemon_exe().map_err(|e| e.to_string())?;
    let timeout = Duration::from_millis(timeout_ms.unwrap_or(5000));

    // blocking(폴링·sleep 포함) — async executor 보호 위해 tauri 런타임의 spawn_blocking 사용
    // (tokio 직접 의존 없이 tauri::async_runtime 경유).
    tauri::async_runtime::spawn_blocking(move || discovery::ensure_daemon(&data_dir, &exe, timeout))
        .await
        .map_err(|e| format!("discover_daemon join 실패: {e}"))?
        .map(DaemonInfoDto::from)
        // 보안: 에러 메시지엔 token 이 없다(DiscoveryError 는 token 미포함).
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dto_from_maps_all_fields() {
        // 순수 변환 — 각 필드가 올바르게 매핑되는지 단언. start_time 은 내부 IPC 전용이라
        // DTO 에는 싣지 않는다(프론트는 liveness 판정에 쓰지 않음).
        let info = engram_dashboard_protocol::DaemonInfo {
            pid: 4321,
            host: "127.0.0.1".into(),
            port: 51000,
            token: "c".repeat(64),
            protocol_version: 7,
            start_time: 999, // DTO 로 안 넘어가는 것까지 확인(컴파일 + 무시)
        };
        let dto = DaemonInfoDto::from(info);
        assert_eq!(dto.pid, 4321);
        assert_eq!(dto.host, "127.0.0.1");
        assert_eq!(dto.port, 51000);
        assert_eq!(dto.token, "c".repeat(64));
        assert_eq!(dto.protocol_version, 7);
    }
}
