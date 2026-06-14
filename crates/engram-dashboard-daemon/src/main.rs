//! engram-dashboard-daemon — 데몬 수명주기 토대 (phase 2 step 4a).
//!
//! 책임(이번 단위): 단일 인스턴스로 뜨고 → 랜덤 포트 잡고 → 256-bit 토큰 발급해
//! atomic daemon.json 기록하고 → AgentManager 를 소유(src-tauri 배선 미러)한다.
//!
//! ★범위 밖(다음 단위 — step 4b)★: WebSocket 업그레이드 / auth / 프레임 핸들링.
//! accept loop 는 지금 연결을 수락 후 로그만 남기고 drop 한다(TODO 주석 참조).
//!
//! 동시성 모델: tokio multi-thread 런타임. AgentManager 내부는 자체 스레드(pump 등)를
//! 쓰는 std 동기 코드라 async 와 독립이다. restore_all 은 blocking(3s 조기종료 윈도·stagger)
//! 이므로 async executor 를 막지 않게 spawn_blocking 으로 격리한다.

mod instance;
mod portfile;

use std::path::PathBuf;
use std::sync::Arc;

use engram_dashboard_core::logging;
use engram_dashboard_core::persistence::FileProfileStore;
use engram_dashboard_core::pty::manager::AgentManager;
use engram_dashboard_core::pty::profile::{ProfileRegistry, RestoreReport};
use engram_dashboard_core::pty::session_tracker::{SessionTracker, TrackerConfig};
use engram_dashboard_core::pty::types::{AgentId, AgentInfo, AgentStatus, StatusSink};
use engram_dashboard_protocol::PROTOCOL_VERSION;

use tokio::net::TcpListener;

const DAEMON_FILE: &str = "daemon.json";

// ── LogStatusSink ──────────────────────────────────────────────────────────────

/// StatusSink 의 데몬 stub — 상태 변화를 tracing 으로만 남긴다.
///
/// ★다음 단위 교체 지점★: WS 서버가 생기면 이걸 `WsStatusSink`(연결된 클라이언트에게
/// AgentEvent 를 push)로 교체한다. AgentManager::new 에 주입되는 단 한 곳만 바꾸면 됨.
struct LogStatusSink;

impl StatusSink for LogStatusSink {
    fn status_changed(&self, id: AgentId, status: AgentStatus, epoch: u32) {
        tracing::info!(agent = %id, ?status, epoch, "status_changed");
    }

    fn agent_list_updated(&self, agents: Vec<AgentInfo>) {
        tracing::info!(count = agents.len(), "agent_list_updated");
    }

    fn restore_result(&self, report: RestoreReport) {
        tracing::info!(agent = %report.agent_id, ?report.outcome, "restore_result");
    }
}

// ── data dir / 토큰 ──────────────────────────────────────────────────────────────

/// 데이터 디렉토리 결정: `dirs::data_dir().join("com.engram.dashboard")`.
///
/// ★Embedded 일치★: 이 경로는 src-tauri/tauri.conf.json 의 identifier(com.engram.dashboard)와
/// Tauri app_data_dir 규약에 맞춰야 한다. Tauri 의 app_data_dir() 은 Windows 에서
/// RoamingAppData(`%APPDATA%`)\<identifier> 를 반환하고, `dirs::data_dir()` 도 동일한
/// RoamingAppData 를 반환하므로 둘이 바이트 단위로 일치한다. 바뀌면 Embedded 와 어긋나
/// 같은 agents.json/daemon.json 을 못 보게 된다.
fn resolve_data_dir() -> PathBuf {
    match dirs::data_dir() {
        Some(base) => base.join("com.engram.dashboard"),
        None => {
            tracing::warn!("dirs::data_dir() None — 현재 디렉토리를 data_dir 로 사용");
            PathBuf::from(".")
        }
    }
}

/// 256-bit(32B) 토큰을 OS CSPRNG 로 생성해 hex 64자 문자열로 반환.
/// 보안: 반환값은 로그에 찍지 말 것(daemon.json 에만 기록).
fn generate_token() -> Result<String, getrandom::Error> {
    let mut buf = [0u8; 32];
    getrandom::getrandom(&mut buf)?;
    let mut s = String::with_capacity(64);
    for b in buf {
        // 소문자 hex 2자/바이트.
        use std::fmt::Write as _;
        let _ = write!(s, "{b:02x}");
    }
    Ok(s)
}

// ── AgentManager 배선 (src-tauri lib.rs setup 미러) ───────────────────────────────

/// src-tauri 의 setup 블록과 동일한 방식으로 AgentManager 를 조립한다.
/// 차이: StatusSink 가 TauriStatusSink 대신 LogStatusSink. data_dir 은 Embedded 와 동일
/// (dirs::data_dir()/com.engram.dashboard = Tauri app_data_dir).
fn build_manager(data_dir: &std::path::Path) -> Arc<AgentManager> {
    let status_sink = Arc::new(LogStatusSink);

    // 프로필 저장 = data_dir/agents.json (FileProfileStore 는 디렉토리를 받고 내부에서 파일명 결합).
    let store = Arc::new(FileProfileStore::new(data_dir.to_path_buf()));
    let profiles = Arc::new(ProfileRegistry::new(store));

    // 세션 추적: sid 변경(/clear 등) 관측 시 레지스트리에 반영(즉시 persist).
    let profiles_cb = profiles.clone();
    let tracker = Arc::new(SessionTracker::new(
        TrackerConfig::default(),
        Arc::new(move |agent_id, new_sid| {
            profiles_cb.observe_session_id(agent_id, new_sid);
        }),
    ));
    tracker.start();

    Arc::new(AgentManager::new(status_sink, profiles, tracker))
}

// ── main ─────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() {
    // 기본 warn(OFF) — RUST_LOG 로 재정의. core 의 init_logging 재사용(키 마스킹 포함).
    logging::init_logging();

    if let Err(code) = run().await {
        std::process::exit(code);
    }
}

/// 데몬 본체. 반환 Err(code) 면 main 이 그 코드로 exit. 정상 종료(이미 실행 중 포함)는 Ok.
async fn run() -> Result<(), i32> {
    // 1) 단일 인스턴스 가드. 이미 실행 중이면 로그 남기고 정상 종료(exit 0).
    //    ★_guard 는 프로세스 수명 동안 살아 있어야 한다★(Drop 시 mutex 해제 = 단일성 깨짐).
    let _guard = match instance::acquire() {
        Ok(Some(g)) => g,
        Ok(None) => {
            tracing::info!("데몬이 이미 실행 중 — 종료");
            return Ok(());
        }
        Err(e) => {
            tracing::error!("단일 인스턴스 가드 획득 실패: {e}");
            return Err(1);
        }
    };

    // 2) data_dir 결정 + 생성.
    let data_dir = resolve_data_dir();
    if let Err(e) = std::fs::create_dir_all(&data_dir) {
        tracing::error!("data_dir 생성 실패({:?}): {e}", data_dir);
        return Err(1);
    }
    let daemon_path = data_dir.join(DAEMON_FILE);

    // 2.5) 기존 daemon.json 검사. stale(죽은 PID)이면 무시(로그만)하고 덮어쓴다.
    //      살아있으면 — 단일 인스턴스 가드를 통과했더라도(Global mutex 경계와 별개로 다른
    //      살아있는 데몬이 기록한 토큰/포트일 수 있다) 방어적으로 **덮어쓰지 않고 정상 종료**한다.
    //      덮어쓰면 살아있는 데몬의 토큰/포트를 날려 클라이언트 접속이 끊긴다.
    if let Some(prev) = portfile::read(&daemon_path) {
        if portfile::is_stale(&prev) {
            tracing::info!(pid = prev.pid, "기존 daemon.json 이 stale — 덮어씀");
        } else {
            tracing::warn!(
                pid = prev.pid,
                "기존 daemon.json 의 PID 가 살아있음 — 덮어쓰지 않고 종료(살아있는 데몬 보호)"
            );
            return Ok(());
        }
    }

    // 3) 127.0.0.1:0 바인드 → 실제 포트 취득(로컬 전용).
    let listener = match TcpListener::bind("127.0.0.1:0").await {
        Ok(l) => l,
        Err(e) => {
            tracing::error!("TcpListener bind 실패: {e}");
            return Err(1);
        }
    };
    let port = match listener.local_addr() {
        Ok(addr) => addr.port(),
        Err(e) => {
            tracing::error!("local_addr 조회 실패: {e}");
            return Err(1);
        }
    };

    // 4) 256-bit 토큰 생성. 보안: 토큰 자체는 절대 로그에 찍지 않는다.
    let token = match generate_token() {
        Ok(t) => t,
        Err(e) => {
            tracing::error!("토큰 생성 실패: {e}");
            return Err(1);
        }
    };

    // 5) AgentManager 배선(src-tauri 미러).
    let manager = build_manager(&data_dir);

    // 6) daemon.json atomic 기록. 토큰을 포함하나 파일에만 — 로그엔 port/pid 만.
    let info = portfile::DaemonInfo {
        pid: std::process::id(),
        host: "127.0.0.1".to_string(),
        port,
        token,
        protocol_version: PROTOCOL_VERSION,
    };
    if let Err(e) = portfile::write_atomic(&daemon_path, &info) {
        tracing::error!("daemon.json 기록 실패: {e}");
        return Err(1);
    }
    tracing::info!(
        port,
        pid = info.pid,
        protocol_version = PROTOCOL_VERSION,
        path = %daemon_path.display(),
        "데몬 시작 — daemon.json 기록 완료"
    );

    // 7) 복원은 blocking(3s 조기종료 윈도·stagger). spawn_blocking 으로 async executor 보호.
    //    핸들을 보관한다 — 종료 시 shutdown_all 전에 in-flight restore 와 경합하지 않게 abort.
    //    join 하지 않음 — 복원은 백그라운드로 진행(부팅 블로킹 방지, src-tauri 와 동일 의도).
    let restore_handle = {
        let mgr = manager.clone();
        tokio::task::spawn_blocking(move || {
            mgr.restore_all();
        })
    };

    // 8) accept loop + Ctrl-C graceful 종료.
    //    ★다음 단위(step 4b) TODO★: 아래 accept 한 stream 을
    //      (a) WebSocket 업그레이드 → (b) Hello 에서 토큰 auth 검증 →
    //      (c) AgentCommand/AgentEvent 프레임 핸들링(manager 에 위임).
    //    지금은 수락 후 로그만 남기고 drop 한다.
    tracing::info!("accept loop 시작(현재는 수락 후 drop — WS 핸들링은 다음 단위)");
    loop {
        tokio::select! {
            accepted = listener.accept() => {
                match accepted {
                    Ok((_stream, peer)) => {
                        // TODO(step 4b): _stream 을 WS 업그레이드 + auth + 핸들링으로 넘긴다.
                        tracing::info!(%peer, "연결 수락 — 현재는 drop(WS 미구현)");
                        // _stream 은 여기서 drop → 연결 종료.
                    }
                    Err(e) => {
                        tracing::warn!("accept 실패: {e}");
                    }
                }
            }
            _ = tokio::signal::ctrl_c() => {
                tracing::info!("Ctrl-C 수신 — graceful 종료 시작");
                break;
            }
        }
    }

    // 9) graceful 종료. 먼저 in-flight restore 를 abort 해 shutdown_all 과의 경합을 막는다
    //    (restore 가 spawn 하는 중에 shutdown 이 돌면 좀비/이중정리 위험). spawn_blocking 은
    //    이미 OS 스레드에서 도는 클로저를 즉시 멈추진 못하지만, 핸들을 abort 후 await 하면
    //    스레드 완료를 기다려 shutdown 전에 restore 가 끝남을 보장한다.
    restore_handle.abort();
    let _ = restore_handle.await; // abort/완료 결과 무시(Cancelled 또는 Ok)

    // 모든 에이전트 정리(PTY kill + tracker 정지). blocking 이므로 spawn_blocking 으로 실행하고 대기.
    let mgr = manager.clone();
    if let Err(e) = tokio::task::spawn_blocking(move || mgr.shutdown_all()).await {
        tracing::warn!("shutdown_all join 실패: {e}");
    }

    // daemon.json 은 남겨둔다 — 다음 부팅이 stale 판정으로 무시한다(명시 삭제는 추후 선택).
    tracing::info!("데몬 종료 완료");
    // _guard 가 여기서 drop 되며 mutex 해제.
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_is_64_hex_chars() {
        let t = generate_token().expect("token gen");
        assert_eq!(t.len(), 64, "256-bit = 32B → hex 64자");
        assert!(
            t.chars().all(|c| c.is_ascii_hexdigit()),
            "hex 문자만 포함해야 함"
        );
    }

    #[test]
    fn tokens_are_unique() {
        // CSPRNG 라 연속 호출이 충돌하지 않아야 한다(난수성 기본 확인).
        let a = generate_token().unwrap();
        let b = generate_token().unwrap();
        assert_ne!(a, b);
    }
}
