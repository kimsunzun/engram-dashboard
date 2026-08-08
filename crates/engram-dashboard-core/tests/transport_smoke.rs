//! ② 격리 통합테스트 — PtyTransport + OutputCore 신경로를 manager 없이 직접 단언 검증.
//!
//! 실 PTY(default shell)를 spawn 한다. 가볍고 전역 경합 없어 default(자동 실행).

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use uuid::Uuid;

use engram_dashboard_core::agent::manager::default_shell;
use engram_dashboard_core::agent::output_core::{OutputCore, TurnWiring};
use engram_dashboard_core::agent::transport::pty::PtyTransport;
use engram_dashboard_core::agent::transport::AgentTransport;
use engram_dashboard_core::agent::types::{
    AgentId, AgentInfo, AgentStatus, CommandSpec, InputEvent, OutputFrame, OutputPayload,
    OutputSink, SinkError, SinkId, StatusSink,
};

// ── RecordingSink ────────────────────────────────────────────────────────────

#[derive(Clone)]
struct RecordingSink {
    id: SinkId,
    output: Arc<Mutex<Vec<u8>>>,
}

impl RecordingSink {
    fn new() -> Self {
        Self {
            id: Uuid::new_v4(),
            output: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn output_len(&self) -> usize {
        self.output.lock().unwrap().len()
    }

    fn output_contains(&self, needle: &str) -> bool {
        let buf = self.output.lock().unwrap();
        String::from_utf8_lossy(&buf).contains(needle)
    }
}

impl OutputSink for RecordingSink {
    fn send(&self, frame: OutputFrame<'_>) -> Result<(), SinkError> {
        if let OutputPayload::Bytes(b) = frame.payload {
            self.output.lock().unwrap().extend_from_slice(b);
        }
        Ok(())
    }

    fn sink_id(&self) -> SinkId {
        self.id
    }
}

struct NoopStatusSink;
impl StatusSink for NoopStatusSink {
    fn status_changed(&self, _id: AgentId, _status: AgentStatus, _epoch: u32) {}
    fn agent_list_updated(&self, _agents: Vec<AgentInfo>) {}
}

fn wait_until<F: Fn() -> bool>(timeout: Duration, cond: F) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if cond() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    cond()
}

#[test]
fn transport_open_input_resize_shutdown() {
    let started = Instant::now();
    let id = Uuid::new_v4();

    let spec = CommandSpec {
        program: default_shell().to_string(),
        args: vec![],
        env: vec![],
        cwd: PathBuf::from("."),
    };
    let (transport, _child_pid) = PtyTransport::open(&spec, 80, 24).expect("open failed");

    let status_sink: Arc<dyn StatusSink> = Arc::new(NoopStatusSink);
    let core = Arc::new(OutputCore::new(id, 0, status_sink, TurnWiring::detached()));
    assert!(
        matches!(core.status(), AgentStatus::Running),
        "open 직후 status 가 Running 이어야 함"
    );

    let transport: Box<dyn AgentTransport> = Box::new(transport);
    transport.start(core.clone());
    let out_sink = RecordingSink::new();
    let _sid = core.subscribe(Arc::new(out_sink.clone()));

    assert!(
        wait_until(Duration::from_secs(2), || out_sink.output_len() > 0),
        "2s 내 PTY 초기 출력 미수신"
    );

    transport
        .send_input(InputEvent::Raw(b"echo smoke-test\r\n".to_vec()))
        .expect("send_input failed");
    assert!(
        wait_until(Duration::from_secs(3), || out_sink
            .output_contains("smoke-test")),
        "echo 입력이 PTY 출력에 반영되지 않음(smoke-test 미수신)"
    );

    transport.resize(100, 30).expect("resize failed");

    transport.shutdown();
    let join_started = Instant::now();
    core.join_pump(Duration::from_secs(5));
    let join_elapsed = join_started.elapsed();
    assert!(
        join_elapsed < Duration::from_secs(5),
        "join_pump 가 5s 안에 끝나지 않음(hang 의심): {join_elapsed:?}"
    );

    assert!(
        matches!(core.status(), AgentStatus::Killed),
        "shutdown 후 status 가 Killed 가 아님: {:?}",
        core.status()
    );

    // 20s = 개별 단계(5s)보다 느슨한 여유 상한 — 전체 hang 회귀만 잡는다.
    assert!(
        started.elapsed() < Duration::from_secs(20),
        "전체 흐름이 비정상적으로 오래 걸림: {:?}",
        started.elapsed()
    );
}
