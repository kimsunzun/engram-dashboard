//! discovery 커맨드 — LLM/프론트가 데몬 발견을 호출하는 thin wrapper(§5 제어 표면).
//!
//! 비즈니스 로직 없음 — discovery::ensure_daemon 호출만. 실제 부팅 자동 호출 배선은
//! phase4 DaemonClient(WS) 와 함께 한다(이번 단위는 command 노출까지).
//!
//! ADR-0029: 모드 제거 → AppState 없음. data_dir 은 `default_data_dir()`(무인자, debug=repo 루트
//! walk-up / release=appdata)로 산출 — 데몬과 같은 폴더를 본다(daemon.json 공유).

use std::sync::OnceLock;
use std::time::Duration;

use tauri::async_runtime::Mutex;

use crate::discovery::{self, locate_daemon_exe};

/// ensure(spawn 포함) 직렬화 락 — 프로세스 전역(OnceLock+tokio async Mutex).
///
/// ★왜 필요한가★: 창 3개(main/tree/popup)의 각 WebView 가 부팅 시 동시에
/// discover_daemon/daemon_start(=ensure_internal)를 호출한다(StrictMode 로 2회씩 더). daemon.json
/// 이 아직 없으니 각 호출이 제각각 데몬을 WMI spawn → 데몬 mutex 가 1개만 살리고 나머지는 즉시 exit.
/// debug 데몬은 console 앱이라 콘솔 창이 여러 개 깜빡인다(losers).
///
/// 이 락으로 ensure 구간을 직렬화하면 첫 호출만 spawn+daemon.json 을 발행하고, 직렬화로 뒤따르는
/// 호출은 ensure_daemon 이 기존 daemon.json 을 찾아 spawn 없이 attach 한다 → losers spawn 자체가
/// 안 생긴다. 결과(살아있는 데몬 1개)는 동일, 다중 WMI spawn 만 사라진다. 이미 살아있으면 ensure 는
/// 즉시 attach 반환이라 락 보유는 짧다. (async Mutex 라 락 보유 중 await 가능 — spawn_blocking await 를
/// 락 안에서 해도 executor 를 막지 않는다.)
///
/// ★범위 한정(load-bearing)★: 이 락은 **command 경로(discover_daemon/daemon_start=ensure_internal)
/// 한정** 직렬화다. 트레이 "데몬 켜기"(`tray/mod.rs` spawn_daemon_action)는 동기 spawn_blocking 워커라
/// 이 async 락을 안 거치고 `discovery::ensure_daemon` 을 직접 부른다 — 즉 트레이-켜기와 부팅 ensure 가
/// 동시에 나면 직렬화 밖이라 다중 spawn 이 날 수 있다. **그래도 정합성은 데몬 named mutex
/// (`Global\EngramDashboardDaemon-<user>`, daemon instance.rs)가 최종 1개를 보장**한다 — 이 락은
/// 정합성 수단이 아니라 부팅 다중-WebView 동시 ensure 의 콘솔 깜빡임(UX)을 없애는 보강이다. 트레이
/// 경로까지 묶으려면 락을 ensure_daemon(discovery crate)으로 내리거나 트레이를 command 경유로 — 실익
/// (트레이 켜기는 단발 사용자 클릭이라 부팅 race 와 시점 분리) 대비 비용이 커 현재는 named mutex 에 위임.
fn ensure_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

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

/// ★T7c: TauriTransport.start() 진입점(spawn 허용)★. Rust DaemonClient.connect() 를 호출한다.
/// 프론트 TauriTransport.start() 가 invoke 로 부른다(WsTransport.openSocket(true) 대응).
#[tauri::command]
pub async fn daemon_connect(
    client: tauri::State<'_, std::sync::Arc<crate::daemon_client::DaemonClient>>,
) -> Result<(), String> {
    client.connect().await.map_err(|e| e.to_string())
}

/// ★T7c: TauriTransport.ensureReady() 진입점(attach-only, no-spawn)★. Rust DaemonClient.ensure() 를 호출한다.
/// 프론트 TauriTransport.ensureReady() 가 invoke 로 부른다(WsTransport.ensureReady() 대응).
#[tauri::command]
pub async fn daemon_ensure(
    client: tauri::State<'_, std::sync::Arc<crate::daemon_client::DaemonClient>>,
) -> Result<(), String> {
    client.ensure().await.map_err(|e| e.to_string())
}

/// ★T7c: TauriTransport.close() 진입점★. Rust DaemonClient.close() 를 호출한다.
#[tauri::command]
pub fn daemon_close(client: tauri::State<'_, std::sync::Arc<crate::daemon_client::DaemonClient>>) {
    client.close();
}

/// ★리로드 자가복구 pull 조회(Fix-D)★. 현재 DaemonClient 연결 상태를 문자열로 반환한다.
///
/// ★왜 필요한가★: Rust DaemonClient 는 `daemon-connection-state` 이벤트를 상태 *전이* 시에만 emit
/// 한다 — connect()/ensure() 는 이미 Connected 면 emit 없이 Ok 로 단락한다(connection.rs 는 전이에서만
/// app.emit). 그래서 웹뷰가 리로드되면(TauriTransport 재생성, _state='down') 새 창은 "이미 연결됨"을
/// 알 방법이 없어 출력 Channel 을 등록하지 못하고 replay/live 가 전부 두절된다(창 단위 사각지대).
/// 프론트 self-heal 이 리스너 등록 후 이 command 를 1회 pull 해 현재 상태를 확인한다.
///
/// ★문자열 어휘 정합★: `daemon-connection-state` 이벤트 payload 와 **동일한 어휘**("connected"/
/// "reconnecting"/"down")로 직렬화한다 — 프론트가 이벤트 핸들러와 같은 코드 경로로 먹일 수 있게.
/// Connecting(전이 중)은 이벤트로 emit 되지 않으므로(connection.rs 는 connected/reconnecting/down 만
/// emit) 여기서도 "down"으로 접는다 — 프론트 ConnectionState 어휘(connected/reconnecting/down)와 일치.
#[tauri::command]
pub fn daemon_connection_state(
    client: tauri::State<'_, std::sync::Arc<crate::daemon_client::DaemonClient>>,
) -> String {
    use crate::daemon_client::ConnectionState;
    match client.state() {
        ConnectionState::Connected => "connected",
        ConnectionState::Reconnecting => "reconnecting",
        // Connecting/Down 은 이벤트 어휘에 없거나 down 으로 접힌다 — 프론트가 non-connected 로 처리.
        ConnectionState::Connecting | ConnectionState::Down => "down",
    }
    .to_string()
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

    // ★다중 spawn 직렬화★: 다중 WebView 동시 ensure 를 프로세스 전역 락으로 줄 세운다(ensure_lock
    // 주석 참조). 락은 ensure(spawn 포함) 구간만 — 첫 호출이 spawn+daemon.json 발행, 뒤따르는
    // 호출은 attach 만(spawn 안 함). ADR-0024 C1(데몬 직접 spawn 금지)과 무관 — WMI 경로는 그대로.
    let _guard = ensure_lock().lock().await;

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
