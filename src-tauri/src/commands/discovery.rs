//! discovery commands — thin wrappers by which the LLM/frontend calls daemon discovery (§5 control surface).
//!
//! ADR-0029: mode removed → no AppState. data_dir is computed with `default_data_dir()` (no arguments, debug=repo root
//! walk-up / release=`data` under the exe folder) — it looks at the same folder as the daemon (daemon.json shared).

use std::sync::OnceLock;
use std::time::Duration;

use tauri::async_runtime::Mutex;

use crate::discovery::{self, locate_daemon_exe};

// The ensure (spawn included) serialization lock — process-global (OnceLock+tokio async Mutex).
//
// ★Why it is needed★: a window's WebView calls discover_daemon/daemon_start (=ensure_internal) at boot and
// StrictMode calls it twice, so the calls overlap (more with several windows). daemon.json
// does not exist yet so each call WMI-spawns a daemon on its own → the daemon lock keeps only 1 alive and the rest exit at once.
// A debug daemon is a console app so several console windows flash (the losers).
//
// If the ensure section is serialized with this lock, only the first call does spawn + publishes daemon.json, and the calls that follow thanks to serialization
// find the existing daemon.json in ensure_daemon and attach without spawning → the losers spawn itself
// never happens. The result (1 live daemon) is the same, only the multiple WMI spawns disappear. If one is already alive, ensure
// returns attach immediately so the lock is held briefly. (Being an async Mutex, awaiting while holding the lock is OK — awaiting spawn_blocking
// inside the lock does not block the executor.)
//
// ★Scope limit (load-bearing)★: this lock serializes **the command path only (discover_daemon/daemon_start=ensure_internal)**.
// The tray "데몬 켜기" (start daemon) (`tray/mod.rs` spawn_daemon_action) is a synchronous spawn_blocking worker so
// it does not go through this async lock and calls `discovery::ensure_daemon` directly — that is, if tray-start and the boot ensure
// happen at the same time they are outside serialization so multiple spawns can occur. **Even so, consistency is guaranteed down to the final 1 by the daemon's single-instance lock
// (holding daemon.json in the data folder, net instance.rs — ADR-0134/0135)** — this lock is not a
// consistency mechanism but an addendum that removes the console flicker (UX) of simultaneous multi-WebView ensure at boot. To bind the tray
// path too, push the lock down into ensure_daemon (`crate::discovery`) or route the tray through the command — the cost is large against the benefit
// (tray start is a one-off user click so it is separated in time from the boot race), so for now we delegate to that lock.
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

// Discovers the daemon (WMI spawn if absent) and returns the connection info.
// 5 seconds if timeout_ms is unspecified. Console visibility is daemon_start (console=true).
#[tauri::command]
pub async fn discover_daemon(timeout_ms: Option<u64>) -> Result<DaemonInfoDto, String> {
    ensure_internal(timeout_ms, false).await
}

// Decided by daemon.json + PID liveness.
#[derive(serde::Serialize)]
pub struct DaemonStatusDto {
    pub alive: bool,
    pub pid: Option<u32>,
    pub port: Option<u16>,
}

// ADR-0021 §5: explicit daemon start (ensure). If already alive, attach (return its connection info), otherwise spawn.
// If `console=true`, spawn together with a console window (making debug logs visible).
// ★Separated from reconnect★: only this command triggers a spawn — the frontend reconnect loop does not call it.
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
// daemon.json missing/broken/dead PID/version mismatch → Ok(None) (conservative). A live compatible daemon → Ok(Some(DTO)).
#[tauri::command]
pub fn read_daemon_info() -> Result<Option<DaemonInfoDto>, String> {
    let data_dir = discovery::default_data_dir();
    Ok(discovery::read_live_daemon(&data_dir).map(DaemonInfoDto::from))
}

// ★T7c: the TauriTransport.start() entry point (spawn allowed)★.
// The frontend TauriTransport.start() calls it with invoke (corresponds to WsTransport.openSocket(true)).
#[tauri::command]
pub async fn daemon_connect(
    client: tauri::State<'_, std::sync::Arc<crate::daemon_client::DaemonClient>>,
) -> Result<(), String> {
    client.connect().await.map_err(|e| e.to_string())
}

// ★T7c: the TauriTransport.ensureReady() entry point (attach-only, no-spawn)★.
// The frontend TauriTransport.ensureReady() calls it with invoke (corresponds to WsTransport.ensureReady()).
#[tauri::command]
pub async fn daemon_ensure(
    client: tauri::State<'_, std::sync::Arc<crate::daemon_client::DaemonClient>>,
) -> Result<(), String> {
    client.ensure().await.map_err(|e| e.to_string())
}

// ★T7c: the TauriTransport.close() entry point★.
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

// Daemon termination fallback (§5). Forcibly turns off the pid in daemon.json along with its child tree (`/F /T` — platform `process::kill_tree`).
//
// ★graceful first★: a frontend holding a connection must first send the StopDaemon AgentCommand (graceful, exits voluntarily after
// cleaning up children). This command is the fallback for when there is no connection or graceful does not work.
// Returns: the pid it tried to kill (None if none).
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

    // ★Multi-spawn serialization★: lines up simultaneous multi-WebView ensure with a process-global lock (see the ensure_lock
    // comment). The lock covers only the ensure (spawn included) section — the first call spawns + publishes daemon.json, and the calls that follow
    // only attach (no spawn). Unrelated to ADR-0024 C1 (no direct daemon spawn) — the WMI path is unchanged.
    let _guard = ensure_lock().lock().await;

    // blocking (including polling · sleep) — to protect the async executor use the tauri runtime's spawn_blocking
    // (via tauri::async_runtime, without a direct tokio dependency).
    tauri::async_runtime::spawn_blocking(move || {
        discovery::ensure_daemon(&data_dir, &exe, timeout, console)
    })
    .await
    .map_err(|e| format!("discover_daemon join 실패: {e}"))?
    .map(DaemonInfoDto::from)
    // Security: the error message has no token (DiscoveryError does not include token).
    //
    // ★Do not stop at passing only a string to the frontend★: then in release the cause exists only as a banner,
    //   and disappears the moment the banner is closed. Leave it at the default level so the log file explains that banner instead.
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
        // start_time is internal-IPC only so it is not carried in the DTO (the frontend does not use it for liveness decisions).
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
