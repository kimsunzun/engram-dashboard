//! ② Isolated integration test — directly assert-verifies the PtyTransport + OutputCore new path without a manager.
//!
//! Spawns a real PTY (default shell). Light and with no global contention, so default (runs automatically).

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use uuid::Uuid;

use engram_dashboard_agent::output_core::{OutputCore, TurnWiring};
use engram_dashboard_agent::transport::pty::PtyTransport;
use engram_dashboard_agent::transport::AgentTransport;
use engram_dashboard_agent::types::{
    AgentId, AgentInfo, AgentStatus, CommandSpec, InputEvent, OutputFrame, OutputPayload,
    OutputSink, SinkError, SinkId, StatusSink,
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

struct NoopStatusSink;
impl StatusSink for NoopStatusSink {
    fn status_changed(&self, _id: AgentId, _status: AgentStatus, _epoch: u32) {}
    fn agent_list_updated(&self, _agents: Vec<AgentInfo>) {}
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
        "right after open the status must be Running"
    );

    let transport: Box<dyn AgentTransport> = Box::new(transport);
    transport.start(core.clone());
    let out_sink = RecordingSink::new();
    let _sid = core.subscribe(Arc::new(out_sink.clone()));

    assert!(
        wait_until(Duration::from_secs(2), || out_sink.output_len() > 0),
        "did not receive the initial PTY output within 2s"
    );

    transport
        .send_input(InputEvent::Raw(b"echo smoke-test\r\n".to_vec()))
        .expect("send_input failed");
    assert!(
        wait_until(Duration::from_secs(3), || out_sink
            .output_contains("smoke-test")),
        "the echo input was not reflected in the PTY output (smoke-test not received)"
    );

    transport.resize(100, 30).expect("resize failed");

    transport.shutdown();
    let join_started = Instant::now();
    core.join_pump(Duration::from_secs(5));
    let join_elapsed = join_started.elapsed();
    assert!(
        join_elapsed < Duration::from_secs(5),
        "join_pump did not finish within 5s (suspected hang): {join_elapsed:?}"
    );

    assert!(
        matches!(core.status(), AgentStatus::Killed),
        "after shutdown the status is not Killed: {:?}",
        core.status()
    );

    // 20s = a looser leeway bound than the individual stages (5s) — catches only a whole-hang regression.
    assert!(
        started.elapsed() < Duration::from_secs(20),
        "the whole flow took abnormally long: {:?}",
        started.elapsed()
    );
}

// ═══════════════════════════════════════════════════════════════════════════════════════════
// Input queue (no ADR assigned — `transport::input_queue`) — regression net on the PTY side
// ═══════════════════════════════════════════════════════════════════════════════════════════

/// ★Proves★: **FIFO is preserved** even through the queue — three lines put in separately arrive at the child in that order and
///   are echoed in that order. A regression where the writer scrambles the queue or merges chunks is caught by the subsequence assertion below.
/// ★Why the assertion is set as a "subsequence"★: a PTY spits out prompts · ANSI mixed in, so an exact match would depend on the shell.
///   We look only at whether the three markers appear **in this order** — that is all of the property being measured (order).
#[test]
fn pty_input_queue_preserves_fifo_order() {
    let spec = CommandSpec {
        program: default_shell().to_string(),
        args: vec![],
        env: vec![],
        cwd: PathBuf::from("."),
    };
    let (transport, _pid) = PtyTransport::open(&spec, 80, 24).expect("open failed");
    let core = Arc::new(OutputCore::new(
        Uuid::new_v4(),
        0,
        Arc::new(NoopStatusSink) as Arc<dyn StatusSink>,
        TurnWiring::detached(),
    ));
    let transport: Box<dyn AgentTransport> = Box::new(transport);
    transport.start(core.clone());
    let sink = RecordingSink::new();
    let _sid = core.subscribe(Arc::new(sink.clone()));

    assert!(
        wait_until(Duration::from_secs(5), || sink.output_len() > 0),
        "did not receive the initial PTY output within 5s"
    );

    // Put in three separate times — we are measuring whether the queue keeps order, so sending as one chunk would be meaningless.
    for marker in ["fifo-alpha", "fifo-bravo", "fifo-charlie"] {
        transport
            .send_input(InputEvent::Raw(format!("echo {marker}\r\n").into_bytes()))
            .expect("send_input failed");
    }

    assert!(
        wait_until(Duration::from_secs(20), || sink
            .output_contains("fifo-charlie")),
        "the last marker did not arrive within 20s — the queue does not drain"
    );

    let text = {
        let buf = sink.output.lock().unwrap();
        String::from_utf8_lossy(&buf).into_owned()
    };
    let a = text.find("fifo-alpha").expect("alpha did not arrive");
    let b = text.find("fifo-bravo").expect("bravo did not arrive");
    let c = text.find("fifo-charlie").expect("charlie did not arrive");
    assert!(
        a < b && b < c,
        "FIFO violation — arrival positions alpha={a} bravo={b} charlie={c}"
    );

    transport.shutdown();
    core.join_pump(Duration::from_secs(5));
}

/// ★Proves★: exceeding the bound is **refused** with `Err` (not discarded), and input after `shutdown` is refused too —
///   that is, there is no place for bytes to silently pile up after the writer is gone.
/// ★The refusal after `shutdown` is the observation window of the writer's termination★ — no one joins that thread (that is
///   the basis for it not delaying the kill causality), so there is no way to see its death directly. Instead we observe from outside the very
///   signal that ends that thread (the queue closing). That the loop itself ends on that signal is measured by
///   `close_terminates_the_drain_loop` in `transport::input_queue`.
#[test]
fn pty_input_queue_refuses_overflow_and_writes_after_shutdown() {
    use engram_dashboard_agent::transport::input_queue::INPUT_QUEUE_MAX_BYTES;
    use engram_dashboard_agent::types::PtyError;

    let spec = CommandSpec {
        program: default_shell().to_string(),
        args: vec![],
        env: vec![],
        cwd: PathBuf::from("."),
    };
    let (transport, _pid) = PtyTransport::open(&spec, 80, 24).expect("open failed");
    let core = Arc::new(OutputCore::new(
        Uuid::new_v4(),
        0,
        Arc::new(NoopStatusSink) as Arc<dyn StatusSink>,
        TurnWiring::detached(),
    ));
    let transport: Box<dyn AgentTransport> = Box::new(transport);

    // ★Measured before `start()`★ — with no writer the queue is not drained, so the bound decision is deterministic (once the writer
    //   runs, the boundary wobbles by how much was drained). Also measures the fact that the queue accepts even before `start()`.
    let err = transport
        .send_input(InputEvent::Raw(vec![b'z'; INPUT_QUEUE_MAX_BYTES + 1]))
        .expect_err("exceeding the cap must be refused");
    assert!(matches!(err, PtyError::WriteFailed(_)), "{err:?}");

    transport.start(core.clone());
    transport.shutdown();

    let late = transport.send_input(InputEvent::Raw(b"late\r\n".to_vec()));
    assert!(
        matches!(late, Err(PtyError::WriteFailed(_))),
        "input after shutdown must be refused (silently piling up would be a loss): {late:?}"
    );

    core.join_pump(Duration::from_secs(5));
}
