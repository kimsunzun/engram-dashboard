//! ② 격리 통합테스트 — AgentSession(OutputCore + Box<dyn AgentTransport>) 합성을 직접 단언 검증.
//!
//! 실 PTY(default shell)를 spawn 한다. 가볍고 전역 경합 없어 default(자동 실행).

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use uuid::Uuid;

use engram_dashboard_core::agent::backend::{AgentBackend, ShellBackend};
use engram_dashboard_core::agent::manager::default_shell;
use engram_dashboard_core::agent::output_core::{OutputCore, TurnWiring};
use engram_dashboard_core::agent::session::AgentSession;
use engram_dashboard_core::agent::transport::pty::PtyTransport;
use engram_dashboard_core::agent::transport::AgentTransport;
use engram_dashboard_core::agent::types::{
    AgentId, AgentInfo, AgentStatus, CommandSpec, OutputFrame, OutputPayload, OutputSink,
    SinkError, SinkId, StatusSink,
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

#[derive(Clone)]
struct RecordingStatusSink {
    statuses: Arc<Mutex<Vec<AgentStatus>>>,
}

impl RecordingStatusSink {
    fn new() -> Self {
        Self {
            statuses: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn statuses(&self) -> Vec<AgentStatus> {
        self.statuses.lock().unwrap().clone()
    }
}

impl StatusSink for RecordingStatusSink {
    fn status_changed(&self, _id: AgentId, status: AgentStatus, _epoch: u32) {
        self.statuses.lock().unwrap().push(status);
    }
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
fn session_compose_resize_exiting_kill() {
    let started = Instant::now();
    let id = Uuid::new_v4();
    let cwd = PathBuf::from(".");

    let spec = CommandSpec {
        program: default_shell().to_string(),
        args: vec![],
        env: vec![],
        cwd: cwd.clone(),
    };
    let (transport, _child_pid) = PtyTransport::open(&spec, 80, 24).expect("open failed");

    let status_sink = RecordingStatusSink::new();
    let status_dyn: Arc<dyn StatusSink> = Arc::new(status_sink.clone());
    let core = Arc::new(OutputCore::new(id, 0, status_dyn, TurnWiring::detached()));

    let transport: Box<dyn AgentTransport> = Box::new(transport);
    transport.start(core.clone());

    // intent 0 = TerminationIntent::None — 이 smoke 는 set_intent 를 쓰지 않는다.
    let intent = std::sync::Arc::new(std::sync::atomic::AtomicU8::new(0));
    // 이 smoke 는 default_shell 을 띄우므로 backend caps 도 셸 기준(resume=false).
    let shell_cmd = engram_dashboard_core::agent::profile::AgentCommand::Shell {
        program: "cmd.exe".into(),
        args: vec![],
    };
    let backend_caps = ShellBackend.capabilities(&shell_cmd);
    let encoder = engram_dashboard_core::agent::backend::InputEncoder::Raw;
    // ★리터럴로 쓰지 마라★: 하네스가 실물과 반대값을 들면 "산 세션 값 == 백엔드 파생값" 이라는 우편
    //   자격 방어의 전제가 인트리에서 먼저 깨진다. 같은 명령에서 파생시킨다.
    let reads_messages = engram_dashboard_core::agent::backend::reads_messages(&shell_cmd);
    let session = AgentSession::new(
        id,
        cwd,
        0,
        80,
        24,
        intent,
        backend_caps,
        encoder,
        reads_messages,
        core,
        transport,
    );

    let out_sink = RecordingSink::new();
    let _sid = session.subscribe(Arc::new(out_sink.clone()));
    assert!(
        wait_until(Duration::from_secs(2), || out_sink.output_len() > 0),
        "2s 내 PTY 초기 출력 미수신"
    );

    session
        .write_input(b"echo session-test\r\n")
        .expect("write_input failed");
    assert!(
        wait_until(Duration::from_secs(3), || out_sink
            .output_contains("session-test")),
        "echo 입력이 PTY 출력에 반영되지 않음(session-test 미수신)"
    );

    session.resize(100, 30).expect("resize failed");
    assert_eq!(
        (session.cols(), session.rows()),
        (100, 30),
        "resize 후 cols/rows 미반영"
    );

    let entered = session.enter_exiting();
    assert!(entered, "enter_exiting 가 false(Running 이었어야)");
    assert!(
        matches!(session.status(), AgentStatus::Exiting),
        "enter_exiting 후 status 가 Exiting 이어야 함: {:?}",
        session.status()
    );

    let kill_started = Instant::now();
    session.kill(Duration::from_secs(5));
    let kill_elapsed = kill_started.elapsed();
    assert!(
        kill_elapsed < Duration::from_secs(5),
        "kill(shutdown+join) 이 5s 안에 끝나지 않음(hang 의심): {kill_elapsed:?}"
    );

    assert!(
        matches!(session.status(), AgentStatus::Killed),
        "kill 후 status 가 Killed 가 아님: {:?}",
        session.status()
    );

    let seq = status_sink.statuses();
    assert!(
        seq.iter().any(|s| matches!(s, AgentStatus::Exiting)),
        "전이에 Exiting 과도기 없음: {seq:?}"
    );
    assert!(
        matches!(seq.last(), Some(AgentStatus::Killed)),
        "전이 종점이 Killed 가 아님: {seq:?}"
    );

    assert!(
        started.elapsed() < Duration::from_secs(20),
        "전체 흐름이 비정상적으로 오래 걸림: {:?}",
        started.elapsed()
    );
}
