//! Headless 백엔드 검증 — Tauri/React 없이 PTY 전체 흐름을 로그로 실측.
//! 검증 기준: spawn → PTY out 수신 → echo → resize → kill,
//!           STATUS: Running(spawn 시) → Exiting → Killed 순서,
//!           kill 후 list 비고(count=0), recv_timeout 없이 즉시 종료.
//!
//! 실행: src-tauri 디렉토리에서 `cargo run --example headless`

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use uuid::Uuid;

use engram_dashboard_core::logging::{init_logging, mask_secrets, set_log_level};
use engram_dashboard_core::persistence::FileProfileStore;
use engram_dashboard_core::pty::manager::{default_shell, AgentManager};
use engram_dashboard_core::pty::profile::{AgentCommand, AgentProfile, ProfileRegistry, SpawnMode};
use engram_dashboard_core::pty::session_tracker::{SessionTracker, TrackerConfig};
use engram_dashboard_core::pty::types::{
    AgentId, AgentInfo, AgentStatus, OutputFrame, OutputSink, SinkError, SinkId, StatusSink,
};

// ── LogSink ──────────────────────────────────────────────────────────────────
// OutputSink + StatusSink 양쪽을 구현하는 테스트용 sink.
// PTY 출력은 mask_secrets() 적용 후 로그에 찍는다 (T-1 완료).
// 기본 warn 레벨에서는 PTY 출력이 찍히지 않으나, debug 활성화 시 안전망.

struct LogSink {
    id: SinkId,
}

impl LogSink {
    fn new() -> Self {
        Self { id: Uuid::new_v4() }
    }
}

impl OutputSink for LogSink {
    fn send(&self, frame: OutputFrame<'_>) -> Result<(), SinkError> {
        // S12 raw 경계화: frame.data가 이미 raw 바이트(base64 디코드 불필요).
        let text = String::from_utf8_lossy(frame.data);
        tracing::info!(
            agent = %frame.agent_id,
            seq = frame.seq,
            "PTY out: {:?}",
            mask_secrets(&text)  // T-1: API키·토큰 마스킹 후 출력
        );
        Ok(())
    }

    fn sink_id(&self) -> SinkId {
        self.id
    }
}

impl StatusSink for LogSink {
    fn status_changed(&self, id: AgentId, status: AgentStatus, epoch: u32) {
        // Running→Exiting→Killed 전이가 이 로그에 순서대로 찍혀야 한다.
        tracing::info!(agent = %id, ?status, epoch, "STATUS changed");
    }

    fn agent_list_updated(&self, agents: Vec<AgentInfo>) {
        tracing::info!(count = agents.len(), "agent list updated");
    }
}

// ── main ─────────────────────────────────────────────────────────────────────

fn main() {
    // 테스트라 debug 레벨로 켜 전체 흐름 관찰.
    init_logging();
    set_log_level("debug").expect("set_log_level failed");

    let status_sink: Arc<dyn StatusSink> = Arc::new(LogSink::new());

    // 프로필 영속화는 임시 디렉토리, 세션 추적은 비활성(shell이라 세션 파일 없음).
    let store = Arc::new(FileProfileStore::new(
        std::env::temp_dir().join("engram-headless"),
    ));
    let profiles = Arc::new(ProfileRegistry::new(store));
    let tracker = Arc::new(SessionTracker::new(
        TrackerConfig {
            sessions_dir: None,
            enabled: false,
            poll_interval: Duration::from_secs(1),
        },
        Arc::new(|_, _| {}),
    ));
    let manager = AgentManager::new(status_sink, profiles, tracker);

    // 1) spawn — 기본 셸(Shell 프로필, Fresh). AgentStatus::Running 상태로 시작됨.
    let profile = AgentProfile::new(
        "headless".into(),
        AgentCommand::Shell {
            program: default_shell().to_string(),
            args: vec![],
        },
        PathBuf::from("."),
        vec![],
        false,
    );
    let info = manager
        .spawn_agent(&profile, SpawnMode::Fresh)
        .expect("spawn failed");
    tracing::info!(?info, "spawned");

    // 2) subscribe — 이후 PTY 출력이 LogSink.send로 흘러온다.
    let out_sink: Arc<dyn OutputSink> = Arc::new(LogSink::new());
    let _sink_id = manager
        .subscribe(info.id, out_sink)
        .expect("subscribe failed");

    // 3) 초기 프롬프트 수신 대기.
    std::thread::sleep(Duration::from_secs(2));

    // 4) stdin write — PTY out 로그에 "headless-test"가 보여야 함.
    manager
        .write_stdin(info.id, b"echo headless-test\r\n")
        .expect("write_stdin failed");
    std::thread::sleep(Duration::from_secs(1));

    // 5) resize — cols/rows 변경.
    manager.resize(info.id, 100, 30).expect("resize failed");

    // 6) kill — Exiting 설정(kill_agent) → drain EOF → Killed 전이(drain) 순서.
    //    drain_done_rx.recv_timeout(5s) 안에 drain이 완료 신호를 보내야 즉시 반환.
    manager.kill_agent(info.id).expect("kill_agent failed");

    // 7) kill 후 list가 비어있으면 PASS. recv_timeout이 걸렸다면 5s 지연이 생겼을 것.
    let remaining = manager.list_agents().len();
    tracing::info!(remaining, "after kill — 0이어야 PASS");

    assert_eq!(
        remaining, 0,
        "kill 후 세션이 남아있음 — sessions 맵 제거 실패"
    );
    tracing::info!("headless PASS");
}
