//! discovery 커맨드 — LLM/프론트가 데몬 발견을 호출하는 thin wrapper(§5 제어 표면).
//!
//! 비즈니스 로직 없음 — discovery::ensure_daemon 호출만. 실제 부팅 자동 호출 배선은
//! phase4 DaemonClient(WS) 와 함께 한다(이번 단위는 command 노출까지).
//!
//! ADR-0029: 모드 제거 → AppState 없음. data_dir 은 `default_data_dir()`(무인자, debug=repo 루트
//! walk-up / release=appdata)로 산출 — 데몬과 같은 폴더를 본다(daemon.json 공유).

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
/// data_dir 은 default_data_dir()(데몬과 같은 폴더 단일 출처, ADR-0024/0029)로 산출한다.
/// timeout_ms 미지정 시 5초. spawn 시 windowless(콘솔 창 없음) — 콘솔 가시화는 daemon_start(console=true).
#[tauri::command]
pub async fn discover_daemon(timeout_ms: Option<u64>) -> Result<DaemonInfoDto, String> {
    ensure_internal(timeout_ms, false).await
}

/// 데몬 alive/pid/port 조회(§5 LLM 제어 표면). daemon.json + PID liveness 로 판정.
#[derive(serde::Serialize)]
pub struct DaemonStatusDto {
    pub alive: bool,
    pub pid: Option<u32>,
    pub port: Option<u16>,
}

/// ADR-0021 §5: 데몬 명시 시작(ensure). 이미 살아있으면 attach(그 접속 정보 반환), 없으면 spawn.
/// `console=true` 면 콘솔 창과 함께 spawn(디버그 로그 가시화), 기본(false/미지정) windowless.
/// ★재연결과 분리★: 이 command 만 spawn 을 유발한다 — 프론트 재연결 루프는 호출하지 않는다.
#[tauri::command]
pub async fn daemon_start(
    timeout_ms: Option<u64>,
    console: Option<bool>,
) -> Result<DaemonInfoDto, String> {
    ensure_internal(timeout_ms, console.unwrap_or(false)).await
}

/// 데몬 상태 조회(§5). 살아있는 데몬이 있는지 + pid/port.
///
/// data_dir 은 default_data_dir()(데몬과 같은 폴더 단일 출처, ADR-0024/0029)로 산출한다.
#[tauri::command]
pub fn daemon_status() -> Result<DaemonStatusDto, String> {
    let data_dir = discovery::default_data_dir();
    let s = discovery::daemon_status(&data_dir);
    Ok(DaemonStatusDto {
        alive: s.alive,
        pid: s.pid,
        port: s.port,
    })
}

/// 살아있는 데몬의 접속 정보(token 포함)를 daemon.json 에서 읽어 반환. ★spawn 안 함★(ADR-0021:
/// 재연결은 깨우지 않는다 — 단지 데몬이 옮겨갔으면 새 주소를 따라가게 한다). 데몬 없거나 죽었으면 None.
///
/// ★재연결 hot-swap 추적용★: daemon_stop→daemon_start(통째 교체)나 크래시-재spawn 으로 데몬이 새
/// port/token 으로 뜨면, 프론트 재연결 루프(wsTransport)가 캐시된 옛 주소 대신 이 command 로 현재
/// daemon.json 을 재조회해 새 주소로 attach 한다. read-only 라 discover_daemon(spawn 동반)과 분리된다.
/// data_dir 은 default_data_dir()(데몬과 같은 폴더 단일 출처, ADR-0024/0029).
///
/// daemon.json 없음/깨짐/죽은 PID/버전 불일치 → Ok(None)(보수). 살아있는 호환 데몬이면 Ok(Some(DTO)).
/// token 은 DTO 에 실리나 **로그 금지**(DaemonInfoDto 규약).
#[tauri::command]
pub fn read_daemon_info() -> Result<Option<DaemonInfoDto>, String> {
    let data_dir = discovery::default_data_dir();
    Ok(discovery::read_live_daemon(&data_dir).map(DaemonInfoDto::from))
}

/// 데몬 종료 fallback(§5). daemon.json 의 pid 를 taskkill /F.
///
/// ★graceful 우선★: 연결을 쥔 프론트는 먼저 StopDaemon AgentCommand(graceful, 자식 정리 후 자진
/// 종료)를 보내야 한다. 이 command 는 연결이 없거나 graceful 이 안 먹을 때의 fallback 이다.
/// 반환: kill 시도한 pid(없으면 None).
#[tauri::command]
pub async fn daemon_stop() -> Result<Option<u32>, String> {
    let data_dir = discovery::default_data_dir();
    tauri::async_runtime::spawn_blocking(move || discovery::daemon_stop(&data_dir))
        .await
        .map_err(|e| format!("daemon_stop join 실패: {e}"))?
        .map_err(|e| e.to_string())
}

/// discover/start 공통 — ensure_daemon 을 blocking 으로 호출하고 DTO 로 변환.
async fn ensure_internal(timeout_ms: Option<u64>, console: bool) -> Result<DaemonInfoDto, String> {
    // ADR-0024/0029: data_dir 은 default_data_dir() 단일 출처(데몬과 동일 폴더).
    let data_dir = discovery::default_data_dir();
    let exe = locate_daemon_exe().map_err(|e| e.to_string())?;
    let timeout = Duration::from_millis(timeout_ms.unwrap_or(5000));

    // blocking(폴링·sleep 포함) — async executor 보호 위해 tauri 런타임의 spawn_blocking 사용
    // (tokio 직접 의존 없이 tauri::async_runtime 경유).
    tauri::async_runtime::spawn_blocking(move || {
        discovery::ensure_daemon(&data_dir, &exe, timeout, console)
    })
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
