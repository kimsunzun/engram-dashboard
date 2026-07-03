//! ② 격리 통합테스트 — PtyTransport + OutputCore 신경로를 manager 없이 직접 단언 검증.
//!
//! (구 examples/transport_smoke.rs 이관 — assert 보존·강화, 출력 단언 추가.)
//!
//! 검증 기준:
//!   open → start(pump 기동) → subscribe → echo 입력(출력에 smoke-test) → resize 성공 →
//!   shutdown → join_pump → status 가 Killed 로 finish.
//!   인과: master drop → reader EOF → pump break → core.finish(Killed). hang 없이 즉시 반환.
//!
//! 실 PTY(default shell)를 spawn 한다. 가볍고 전역 경합 없어 default(자동 실행).

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use uuid::Uuid;

use engram_dashboard_core::agent::manager::default_shell;
use engram_dashboard_core::agent::output_core::OutputCore;
use engram_dashboard_core::agent::transport::pty::PtyTransport;
use engram_dashboard_core::agent::transport::AgentTransport;
use engram_dashboard_core::agent::types::{
    AgentId, AgentInfo, AgentStatus, CommandSpec, InputEvent, OutputFrame, OutputPayload,
    OutputSink, SinkError, SinkId, StatusSink,
};

// ── RecordingSink ────────────────────────────────────────────────────────────
// 받은 출력 바이트를 누적해 단언에 쓰는 기록형 OutputSink. StatusSink 는 no-op(core.status() 직접 조회).

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
        // S15 B5 payload-generic: 콘솔 바이트만 수집(smoke 테스트는 구조화 이벤트를 안 다룸).
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

    // 1) CommandSpec 으로 default shell spawn(manager 없이 PtyTransport 직접).
    let spec = CommandSpec {
        program: default_shell().to_string(),
        args: vec![],
        env: vec![],
        cwd: PathBuf::from("."),
    };
    let (transport, _child_pid) = PtyTransport::open(&spec, 80, 24).expect("open failed");

    // 2) OutputCore + status sink. 생성 직후 Running.
    let status_sink: Arc<dyn StatusSink> = Arc::new(NoopStatusSink);
    let core = Arc::new(OutputCore::new(id, 0, status_sink));
    assert!(
        matches!(core.status(), AgentStatus::Running),
        "open 직후 status 가 Running 이어야 함"
    );

    // 3) start(pump 기동) + subscribe(출력 수신).
    let transport: Box<dyn AgentTransport> = Box::new(transport);
    transport.start(core.clone());
    let out_sink = RecordingSink::new();
    let _sid = core.subscribe(Arc::new(out_sink.clone()));

    // 초기 프롬프트 출력 1개 이상 수신.
    assert!(
        wait_until(Duration::from_secs(2), || out_sink.output_len() > 0),
        "2s 내 PTY 초기 출력 미수신"
    );

    // 4) echo 입력 → 출력에 smoke-test 가 보여야 함.
    transport
        .send_input(InputEvent::Raw(b"echo smoke-test\r\n".to_vec()))
        .expect("send_input failed");
    assert!(
        wait_until(Duration::from_secs(3), || out_sink
            .output_contains("smoke-test")),
        "echo 입력이 PTY 출력에 반영되지 않음(smoke-test 미수신)"
    );

    // 5) resize 성공.
    transport.resize(100, 30).expect("resize failed");

    // 6) shutdown → core.join_pump. 인과: master drop → reader EOF → pump break →
    //    core.finish(Killed). join_pump 가 5s 안에 즉시 반환해야 함(hang 이면 recv_timeout 소진).
    transport.shutdown();
    let join_started = Instant::now();
    core.join_pump(Duration::from_secs(5));
    let join_elapsed = join_started.elapsed();
    assert!(
        join_elapsed < Duration::from_secs(5),
        "join_pump 가 5s 안에 끝나지 않음(hang 의심): {join_elapsed:?}"
    );

    // 7) status 가 Killed 로 finish 되어야 함.
    assert!(
        matches!(core.status(), AgentStatus::Killed),
        "shutdown 후 status 가 Killed 가 아님: {:?}",
        core.status()
    );

    // 전체가 5s 안에 끝났는지(여유 — hang 회귀 방지).
    assert!(
        started.elapsed() < Duration::from_secs(20),
        "전체 흐름이 비정상적으로 오래 걸림: {:?}",
        started.elapsed()
    );
}
