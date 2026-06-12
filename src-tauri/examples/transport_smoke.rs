//! transport_smoke — PtyTransport + OutputCore 신경로 실측(manager 없이).
//!
//! 검증 기준: open → start → subscribe → echo 입력 → 출력에 smoke-test → resize →
//!           shutdown → join_pump → status가 Killed로 finish(인과: master drop→reader EOF→
//!           pump break→core.finish(Killed)). hang 없이 즉시 반환.
//!
//! 실행: src-tauri 디렉토리에서 `cargo run --example transport_smoke`

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use base64::Engine as _;
use uuid::Uuid;

use engram_dashboard_lib::logging::{init_logging, mask_secrets, set_log_level};
use engram_dashboard_lib::pty::manager::default_shell;
use engram_dashboard_lib::pty::output_core::OutputCore;
use engram_dashboard_lib::pty::transport::pty::PtyTransport;
use engram_dashboard_lib::pty::transport::AgentTransport;
use engram_dashboard_lib::pty::types::{
    AgentId, AgentInfo, AgentStatus, CommandSpec, InputEvent, OutputSink, PtyEvent, SinkError,
    SinkId, StatusSink,
};

// ── LogSink ──────────────────────────────────────────────────────────────────
// OutputSink + StatusSink 양쪽을 구현하는 테스트용 sink(headless.rs와 동일 패턴).

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

    // 1) CommandSpec으로 default shell spawn(manager 없이 PtyTransport 직접).
    let spec = CommandSpec {
        program: default_shell().to_string(),
        args: vec![],
        env: vec![],
        cwd: PathBuf::from("."),
    };
    let (transport, child_pid) = PtyTransport::open(&spec, 80, 24).expect("open failed");
    tracing::info!(?child_pid, "PtyTransport opened");

    // 2) OutputCore + status sink.
    let status_sink: Arc<dyn StatusSink> = Arc::new(LogSink::new());
    let core = Arc::new(OutputCore::new(id, 0, status_sink));

    // 3) start(pump 기동) + subscribe(출력 수신).
    let transport: Box<dyn AgentTransport> = Box::new(transport);
    transport.start(core.clone());
    let out_sink: Arc<dyn OutputSink> = Arc::new(LogSink::new());
    let _sink_id = core.subscribe(out_sink);

    // 초기 프롬프트 수신 대기.
    std::thread::sleep(Duration::from_secs(2));

    // 4) echo 입력 → 출력 로그에 "smoke-test"가 보여야 함.
    transport
        .send_input(InputEvent::Raw(b"echo smoke-test\r\n".to_vec()))
        .expect("send_input failed");
    std::thread::sleep(Duration::from_secs(1));

    // 5) resize.
    transport.resize(100, 30).expect("resize failed");

    // 6) shutdown → core.join_pump. 인과: master drop → reader EOF → pump break →
    //    core.finish(Killed). join_pump가 5s 안에 즉시 반환해야 PASS(hang이면 recv_timeout 소진).
    transport.shutdown();
    core.join_pump(Duration::from_secs(5));

    // 7) status가 Killed면 PASS.
    let status = core.status();
    let elapsed = started.elapsed();
    tracing::info!(?status, ?elapsed, "after shutdown — Killed여야 PASS");

    assert!(
        matches!(status, AgentStatus::Killed),
        "shutdown 후 status가 Killed가 아님: {status:?}"
    );
    tracing::info!("transport_smoke PASS");
}
