//! Headless 백엔드 검증 — Tauri/React 없이 PTY 전체 흐름을 로그로 실측.
//! 검증 기준: spawn → PTY out 수신 → echo → resize → kill,
//!           STATUS: Running(spawn 시) → Exiting → Killed 순서,
//!           kill 후 list 비고(count=0), recv_timeout 없이 즉시 종료.
//!
//! 실행: src-tauri 디렉토리에서 `cargo run --example headless`

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use uuid::Uuid;

use engram_dashboard_lib::logging::{init_logging, set_log_level};
use engram_dashboard_lib::pty::manager::PtyManager;
use engram_dashboard_lib::pty::types::{
    AgentId, AgentInfo, AgentStatus, OutputSink, PtyEvent, SinkError, SinkId, StatusSink,
};

// ── LogSink ──────────────────────────────────────────────────────────────────
// OutputSink + StatusSink 양쪽을 구현하는 테스트용 sink.
// PTY 출력을 그대로 tracing::info!로 찍는다.
//
// ⚠️  실제 Claude 에이전트 연결 시 PTY 출력에 API 키가 포함될 수 있다.
//     반드시 masking layer(logging/masking.rs)를 적용한 뒤 로그로 출력해야 한다
//     (tracking T-1). 현재는 cmd.exe/pwsh 대상 테스트라 안전하지만 이 주석은 유지.

struct LogSink {
    id: SinkId,
}

impl LogSink {
    fn new() -> Self {
        Self { id: Uuid::new_v4() }
    }
}

impl OutputSink for LogSink {
    fn send(&self, event: PtyEvent) -> Result<(), SinkError> {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(&event.data_b64)
            .unwrap_or_default();
        tracing::info!(
            agent = %event.agent_id,
            seq = event.seq,
            "PTY out: {:?}",
            String::from_utf8_lossy(&bytes)
        );
        Ok(())
    }

    fn sink_id(&self) -> SinkId {
        self.id
    }
}

impl StatusSink for LogSink {
    fn status_changed(&self, id: AgentId, status: AgentStatus) {
        // Running→Exiting→Killed 전이가 이 로그에 순서대로 찍혀야 한다.
        tracing::info!(agent = %id, ?status, "STATUS changed");
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
    let manager = PtyManager::new(status_sink);

    // 1) spawn — cmd.exe (AgentStatus::Running 상태로 시작됨).
    let info = manager.spawn_agent(Path::new(".")).expect("spawn failed");
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
