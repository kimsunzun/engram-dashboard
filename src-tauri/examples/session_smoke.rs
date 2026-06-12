//! session_smoke — AgentSession(OutputCore + Box<dyn AgentTransport>) 합성 실측(manager 없이).
//!
//! 검증 기준(transport_smoke를 본떠 합성 표면 전체):
//!   open → (manager 흉내) start → AgentSession::new → subscribe → echo 입력(session-test 출력) →
//!   resize(cols/rows 반영) → enter_exiting(Exiting 알림·true) → kill(shutdown THEN join_pump) →
//!   status가 Killed로 finish. hang 없이 즉시 반환하면 PASS.
//!
//! start는 AgentSession 밖(여기서 manager처럼)에서 호출한다 — impl-spec: new는 start를 부르지 않는다.
//!
//! 실행: src-tauri 디렉토리에서 `cargo run --example session_smoke`

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use base64::Engine as _;
use uuid::Uuid;

use engram_dashboard_lib::logging::{init_logging, mask_secrets, set_log_level};
use engram_dashboard_lib::pty::manager::default_shell;
use engram_dashboard_lib::pty::output_core::OutputCore;
use engram_dashboard_lib::pty::session::AgentSession;
use engram_dashboard_lib::pty::transport::pty::PtyTransport;
use engram_dashboard_lib::pty::transport::AgentTransport;
use engram_dashboard_lib::pty::types::{
    AgentId, AgentInfo, AgentStatus, CommandSpec, OutputSink, PtyEvent, SinkError, SinkId,
    StatusSink,
};

// ── LogSink ──────────────────────────────────────────────────────────────────
// OutputSink + StatusSink 양쪽을 구현하는 테스트용 sink(transport_smoke와 동일 패턴).

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
        let text = String::from_utf8_lossy(&bytes);
        tracing::info!(
            agent = %event.agent_id,
            seq = event.seq,
            "PTY out: {:?}",
            mask_secrets(&text)
        );
        Ok(())
    }

    fn sink_id(&self) -> SinkId {
        self.id
    }
}

impl StatusSink for LogSink {
    fn status_changed(&self, id: AgentId, status: AgentStatus, epoch: u32) {
        tracing::info!(agent = %id, ?status, epoch, "STATUS changed");
    }

    fn agent_list_updated(&self, _agents: Vec<AgentInfo>) {}
}

// ── main ─────────────────────────────────────────────────────────────────────

fn main() {
    init_logging();
    set_log_level("debug").expect("set_log_level failed");

    let started = Instant::now();
    let id = Uuid::new_v4();
    let cwd = PathBuf::from(".");

    // 1) CommandSpec으로 default shell open(manager 없이 PtyTransport 직접).
    let spec = CommandSpec {
        program: default_shell().to_string(),
        args: vec![],
        env: vec![],
        cwd: cwd.clone(),
    };
    let (transport, child_pid) = PtyTransport::open(&spec, 80, 24).expect("open failed");
    tracing::info!(?child_pid, "PtyTransport opened");

    // 2) OutputCore + status sink.
    let status_sink: Arc<dyn StatusSink> = Arc::new(LogSink::new());
    let core = Arc::new(OutputCore::new(id, 0, status_sink));

    // 3) start(pump 기동) — AgentSession 밖에서 manager처럼 호출. new는 start를 부르지 않는다(impl-spec).
    let transport: Box<dyn AgentTransport> = Box::new(transport);
    transport.start(core.clone());

    // 4) AgentSession 합성. 이미 start된 core/transport를 묶는다.
    let session = AgentSession::new(id, cwd, 0, 80, 24, core, transport);

    // 5) subscribe → 초기 프롬프트 대기 → echo 입력 → session-test 출력이 보여야 함.
    let out_sink: Arc<dyn OutputSink> = Arc::new(LogSink::new());
    let _sink_id = session.subscribe(out_sink);
    std::thread::sleep(Duration::from_secs(2));

    session
        .write_input(b"echo session-test\r\n")
        .expect("write_input failed");
    std::thread::sleep(Duration::from_secs(1));

    // 6) resize → session.cols/rows가 100/30으로 반영돼야 함.
    session.resize(100, 30).expect("resize failed");
    let (c, r) = (session.cols(), session.rows());
    tracing::info!(cols = c, rows = r, "after resize — 100/30이어야 함");
    assert_eq!((c, r), (100, 30), "resize 후 cols/rows 미반영: {c}/{r}");

    // 7) enter_exiting → Exiting 알림 + true(아직 Running이었으므로).
    let entered = session.enter_exiting();
    tracing::info!(entered, "enter_exiting — true여야 함");
    assert!(entered, "enter_exiting가 false(Running이었어야)");

    // 8) kill = shutdown() THEN join_pump(). 인과: master drop → reader EOF → pump break →
    //    core.finish(Killed). join_pump가 5s 안에 즉시 반환해야 PASS(hang이면 recv_timeout 소진).
    session.kill(Duration::from_secs(5));

    // 9) status가 Killed면 PASS.
    let status = session.status();
    let elapsed = started.elapsed();
    tracing::info!(?status, ?elapsed, "after kill — Killed여야 PASS");

    assert!(
        matches!(status, AgentStatus::Killed),
        "kill 후 status가 Killed가 아님: {status:?}"
    );
    tracing::info!("session_smoke PASS");
}
