//! discovery 커맨드 — LLM/프론트가 데몬 발견을 호출하는 thin wrapper(§5 제어 표면).
//!
//! ADR-0029: 모드 제거 → AppState 없음. data_dir 은 `default_data_dir()`(무인자, debug=repo 루트
//! walk-up / release=exe 폴더 하위 `data`)로 산출 — 데몬과 같은 폴더를 본다(daemon.json 공유).

use std::sync::OnceLock;
use std::time::Duration;

use tauri::async_runtime::Mutex;

use crate::discovery::{self, locate_daemon_exe};

// ensure(spawn 포함) 직렬화 락 — 프로세스 전역(OnceLock+tokio async Mutex).
//
// ★왜 필요한가★: 창의 WebView 가 부팅 시 discover_daemon/daemon_start(=ensure_internal)를 부르고
// StrictMode 가 그것을 두 번씩 불러 호출이 겹친다(창이 여럿이면 더 겹친다). daemon.json
// 이 아직 없으니 각 호출이 제각각 데몬을 WMI spawn → 데몬 잠금이 1개만 살리고 나머지는 즉시 exit.
// debug 데몬은 console 앱이라 콘솔 창이 여러 개 깜빡인다(losers).
//
// 이 락으로 ensure 구간을 직렬화하면 첫 호출만 spawn+daemon.json 을 발행하고, 직렬화로 뒤따르는
// 호출은 ensure_daemon 이 기존 daemon.json 을 찾아 spawn 없이 attach 한다 → losers spawn 자체가
// 안 생긴다. 결과(살아있는 데몬 1개)는 동일, 다중 WMI spawn 만 사라진다. 이미 살아있으면 ensure 는
// 즉시 attach 반환이라 락 보유는 짧다. (async Mutex 라 락 보유 중 await 가능 — spawn_blocking await 를
// 락 안에서 해도 executor 를 막지 않는다.)
//
// ★범위 한정(load-bearing)★: 이 락은 **command 경로(discover_daemon/daemon_start=ensure_internal)
// 한정** 직렬화다. 트레이 "데몬 켜기"(`tray/mod.rs` spawn_daemon_action)는 동기 spawn_blocking 워커라
// 이 async 락을 안 거치고 `discovery::ensure_daemon` 을 직접 부른다 — 즉 트레이-켜기와 부팅 ensure 가
// 동시에 나면 직렬화 밖이라 다중 spawn 이 날 수 있다. **그래도 정합성은 데몬의 단일 인스턴스 잠금
// (데이터 폴더 안 daemon.json 을 붙잡는 것, net instance.rs — ADR-0134/0135)이 최종 1개를 보장**한다 — 이 락은
// 정합성 수단이 아니라 부팅 다중-WebView 동시 ensure 의 콘솔 깜빡임(UX)을 없애는 보강이다. 트레이
// 경로까지 묶으려면 락을 ensure_daemon(`crate::discovery`)으로 내리거나 트레이를 command 경유로 — 실익
// (트레이 켜기는 단발 사용자 클릭이라 부팅 race 와 시점 분리) 대비 비용이 커 현재는 그 잠금에 위임.
fn ensure_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

/// The daemon's address as handed to the WebView by `discover_daemon` · `daemon_start` · `read_daemon_info`.
///
/// Never carries the daemon token: any script in the WebView (`csp: null`) can read a command's result, and the token
/// would let it leave the WebView (another process · a direct socket). The shell reads the token from `daemon.json`
/// for its own link; the WebView never needs it.
// ADR-0310: D23 ① — the token stays out of this DTO; do not add it back.
#[derive(serde::Serialize)]
pub struct DaemonInfoDto {
    pub pid: u32,
    pub host: String,
    pub port: u16,
    pub protocol_version: u32,
}

impl From<engram_dashboard_protocol::DaemonInfo> for DaemonInfoDto {
    fn from(i: engram_dashboard_protocol::DaemonInfo) -> Self {
        Self {
            pid: i.pid,
            host: i.host,
            port: i.port,
            protocol_version: i.protocol_version,
        }
    }
}

// 데몬을 발견(없으면 WMI spawn)하고 접속 정보를 반환한다.
// timeout_ms 미지정 시 5초. 콘솔 가시화는 daemon_start(console=true).
#[tauri::command]
pub async fn discover_daemon(timeout_ms: Option<u64>) -> Result<DaemonInfoDto, String> {
    ensure_internal(timeout_ms, false).await
}

// daemon.json + PID liveness 로 판정.
#[derive(serde::Serialize)]
pub struct DaemonStatusDto {
    pub alive: bool,
    pub pid: Option<u32>,
    pub port: Option<u16>,
}

// ADR-0021 §5: 데몬 명시 시작(ensure). 이미 살아있으면 attach(그 접속 정보 반환), 없으면 spawn.
// `console=true` 면 콘솔 창과 함께 spawn(디버그 로그 가시화).
// ★재연결과 분리★: 이 command 만 spawn 을 유발한다 — 프론트 재연결 루프는 호출하지 않는다.
#[tauri::command]
pub async fn daemon_start(
    timeout_ms: Option<u64>,
    console: Option<bool>,
) -> Result<DaemonInfoDto, String> {
    ensure_internal(timeout_ms, console.unwrap_or(false)).await
}

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

// Reads the live daemon's address from daemon.json. ★Never spawns★ (ADR-0021: reconnect never wakes a daemon — it
// only follows a daemon that moved to its new address). No daemon or a dead one → None.
//
// ★For hot-swap tracking on reconnect★: when daemon_stop→daemon_start (full replacement) or a crash respawn brings the
// daemon up on a new port, the frontend reconnect loop (wsTransport) re-reads the current daemon.json through this
// command instead of its cached old address and attaches at the new one. Read-only, so it is kept apart from
// discover_daemon (which spawns).
//
// daemon.json 없음/깨짐/죽은 PID/버전 불일치 → Ok(None)(보수). 살아있는 호환 데몬이면 Ok(Some(DTO)).
#[tauri::command]
pub fn read_daemon_info() -> Result<Option<DaemonInfoDto>, String> {
    let data_dir = discovery::default_data_dir();
    Ok(discovery::read_live_daemon(&data_dir).map(DaemonInfoDto::from))
}

// ★T7c: TauriTransport.start() 진입점(spawn 허용)★.
// 프론트 TauriTransport.start() 가 invoke 로 부른다(WsTransport.openSocket(true) 대응).
#[tauri::command]
pub async fn daemon_connect(
    client: tauri::State<'_, std::sync::Arc<crate::daemon_client::DaemonClient>>,
) -> Result<(), String> {
    client.connect().await.map_err(|e| e.to_string())
}

// ★T7c: TauriTransport.ensureReady() 진입점(attach-only, no-spawn)★.
// 프론트 TauriTransport.ensureReady() 가 invoke 로 부른다(WsTransport.ensureReady() 대응).
#[tauri::command]
pub async fn daemon_ensure(
    client: tauri::State<'_, std::sync::Arc<crate::daemon_client::DaemonClient>>,
) -> Result<(), String> {
    client.ensure().await.map_err(|e| e.to_string())
}

// ★T7c: TauriTransport.close() 진입점★.
#[tauri::command]
pub fn daemon_close(client: tauri::State<'_, std::sync::Arc<crate::daemon_client::DaemonClient>>) {
    client.close();
}

// ★Reload self-heal pull (Fix-D)★.
//
// ★Why it is needed★: the Rust DaemonClient emits `daemon-connection-state` only on state *transitions* — a
// connect()/ensure() on an already connected link answers Ok with no emit (the driver's routing emits on transitions
// only). So after a webview reload (TauriTransport recreated, _state='down') the new window has no way to learn "already
// connected", cannot register its output Channel, and replay/live are cut off (a per-window blind spot). The frontend
// self-heal pulls this command once after registering its listeners to read the current state.
//
// ★Same vocabulary★: serialized with **the same words** as the `daemon-connection-state` payload ("connected" /
// "reconnecting" / "down") so the frontend feeds it through the event handler's path — the frontend ConnectionState
// vocabulary.
#[tauri::command]
pub fn daemon_connection_state(
    client: tauri::State<'_, std::sync::Arc<crate::daemon_client::DaemonClient>>,
) -> String {
    use crate::daemon_client::ConnectionState;
    match client.state() {
        ConnectionState::Connected => "connected",
        ConnectionState::Reconnecting => "reconnecting",
        ConnectionState::Down => "down",
    }
    .to_string()
}

// 데몬 종료 fallback(§5). daemon.json 의 pid 를 자식 트리째 강제로 끈다(`/F /T` — platform `process::kill_tree`).
//
// ★graceful 우선★: 연결을 쥔 프론트는 먼저 StopDaemon AgentCommand(graceful, 자식 정리 후 자진
// 종료)를 보내야 한다. 이 command 는 연결이 없거나 graceful 이 안 먹을 때의 fallback 이다.
// 반환: kill 시도한 pid(없으면 None).
#[tauri::command]
pub async fn daemon_stop() -> Result<Option<u32>, String> {
    let data_dir = discovery::default_data_dir();
    tauri::async_runtime::spawn_blocking(move || discovery::daemon_stop(&data_dir))
        .await
        .map_err(|e| format!("daemon_stop join 실패: {e}"))?
        .map_err(|e| e.to_string())
}

async fn ensure_internal(timeout_ms: Option<u64>, console: bool) -> Result<DaemonInfoDto, String> {
    let data_dir = discovery::default_data_dir();
    let exe = locate_daemon_exe().map_err(|e| {
        tracing::error!("데몬 실행 파일을 찾지 못함: {e}");
        e.to_string()
    })?;
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
    //
    // ★문자열만 프론트로 넘기고 끝내지 마라★: 그러면 릴리즈에서 원인이 배너 하나로만 존재하고,
    //   배너를 닫는 순간 사라진다. 기본 레벨에 남겨 로그 파일이 그 배너를 대신 설명하게 한다.
    .map_err(|e| {
        tracing::warn!("데몬 ensure 실패 — 프론트에 문자열로 전달: {e}");
        e.to_string()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dto_from_maps_all_fields() {
        // start_time 은 내부 IPC 전용이라 DTO 에는 싣지 않는다(프론트는 liveness 판정에 쓰지 않음).
        let info = engram_dashboard_protocol::DaemonInfo {
            pid: 4321,
            host: "127.0.0.1".into(),
            port: 51000,
            token: engram_dashboard_protocol::Secret::new("c".repeat(64)),
            protocol_version: 7,
            start_time: 999,
        };
        let dto = DaemonInfoDto::from(info);
        assert_eq!(dto.pid, 4321);
        assert_eq!(dto.host, "127.0.0.1");
        assert_eq!(dto.port, 51000);
        assert_eq!(dto.protocol_version, 7);
    }

    // ADR-0310: D23 ① — pins the three commands' return type to `DaemonInfoDto`; returning anything else (e.g. the
    // protocol `DaemonInfo`, which serializes the token) stops this module from compiling.
    #[test]
    fn the_three_commands_return_the_tokenless_dto() {
        fn returns_dto<A, Fut>(_: impl Fn(Option<u64>, A) -> Fut)
        where
            Fut: std::future::Future<Output = Result<DaemonInfoDto, String>>,
        {
        }
        returns_dto(|timeout_ms, ()| discover_daemon(timeout_ms));
        returns_dto(daemon_start);
        let _: fn() -> Result<Option<DaemonInfoDto>, String> = read_daemon_info;
    }

    // ADR-0310: D23 ① — all three commands (`discover_daemon` · `daemon_start` · `read_daemon_info`) return this DTO,
    // so its serialized form is what the WebView receives.
    #[test]
    fn serialized_dto_carries_no_token() {
        let token = "c".repeat(64);
        let info = engram_dashboard_protocol::DaemonInfo {
            pid: 4321,
            host: "127.0.0.1".into(),
            port: 51000,
            token: engram_dashboard_protocol::Secret::new(token.clone()),
            protocol_version: 7,
            start_time: 999,
        };
        let json = serde_json::to_value(DaemonInfoDto::from(info)).expect("DTO serializes");
        let fields = json.as_object().expect("DTO serializes to an object");
        assert!(!fields.contains_key("token"), "token field leaked: {json}");
        assert!(
            !json.to_string().contains(&token),
            "token value leaked: {json}"
        );
        let mut keys: Vec<&str> = fields.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(keys, ["host", "pid", "port", "protocol_version"]);
    }
}
