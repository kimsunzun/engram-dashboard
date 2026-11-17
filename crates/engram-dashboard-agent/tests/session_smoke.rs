//! ② Isolated integration test — directly assert-verifies the AgentSession (OutputCore + Box<dyn AgentTransport>) composition.
//!
//! Spawns a real PTY (default shell). Light and with no global contention, so default (runs automatically).

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use uuid::Uuid;

use engram_dashboard_agent::backend::{AgentBackend, ShellBackend};
use engram_dashboard_agent::output_core::{OutputCore, TurnWiring};
use engram_dashboard_agent::session::AgentSession;
use engram_dashboard_agent::transport::pty::PtyTransport;
use engram_dashboard_agent::transport::AgentTransport;
use engram_dashboard_agent::types::{
    AgentId, AgentInfo, AgentStatus, CommandSpec, OutputFrame, OutputPayload, OutputSink,
    SinkError, SinkId, StatusSink,
};
use engram_dashboard_base::testing::wait_until;
use engram_dashboard_platform::shell::default_shell;

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

    // intent 0 = TerminationIntent::None — this smoke does not use set_intent.
    let intent = std::sync::Arc::new(std::sync::atomic::AtomicU8::new(0));
    // This smoke launches default_shell, so the backend caps are also by shell (resume=false).
    let shell_cmd = engram_dashboard_agent::profile::AgentCommand::Shell {
        program: "cmd.exe".into(),
        args: vec![],
    };
    let backend_caps = ShellBackend.capabilities(&shell_cmd);
    let encoder = engram_dashboard_agent::backend::InputEncoder::Raw;
    // ★Do not write it as a literal★: if the harness holds the opposite value from the real one, the premise of the mail
    //   eligibility defense, "live session value == backend-derived value", breaks in-tree first. Derive it from the same command.
    let reads_messages = engram_dashboard_agent::backend::reads_messages(&shell_cmd);
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
        "did not receive the initial PTY output within 2s"
    );

    session
        .write_input(b"echo session-test\r\n")
        .expect("write_input failed");
    assert!(
        wait_until(Duration::from_secs(3), || out_sink
            .output_contains("session-test")),
        "the echo input was not reflected in the PTY output (session-test not received)"
    );

    session.resize(100, 30).expect("resize failed");
    assert_eq!(
        (session.cols(), session.rows()),
        (100, 30),
        "cols/rows not reflected after resize"
    );

    let entered = session.enter_exiting();
    assert!(
        entered,
        "enter_exiting returned false (it should have been Running)"
    );
    assert!(
        matches!(session.status(), AgentStatus::Exiting),
        "after enter_exiting the status must be Exiting: {:?}",
        session.status()
    );

    let kill_started = Instant::now();
    session.kill(Duration::from_secs(5));
    let kill_elapsed = kill_started.elapsed();
    assert!(
        kill_elapsed < Duration::from_secs(5),
        "kill (shutdown+join) did not finish within 5s (suspected hang): {kill_elapsed:?}"
    );

    assert!(
        matches!(session.status(), AgentStatus::Killed),
        "after kill the status is not Killed: {:?}",
        session.status()
    );

    let seq = status_sink.statuses();
    assert!(
        seq.iter().any(|s| matches!(s, AgentStatus::Exiting)),
        "no transitional Exiting in the transitions: {seq:?}"
    );
    assert!(
        matches!(seq.last(), Some(AgentStatus::Killed)),
        "the transition end is not Killed: {seq:?}"
    );

    assert!(
        started.elapsed() < Duration::from_secs(20),
        "the whole flow took abnormally long: {:?}",
        started.elapsed()
    );
}
