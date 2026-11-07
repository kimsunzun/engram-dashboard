//! ② Isolated integration test — directly asserts the StdioTransport (pipe child) + OutputCore new path without a manager.
//!
//! The transport that json mode (ADR-0044) uses. Verified with a cmd.exe fake child, without a real claude (isolation — ADR-0012).
//! It spawns a real process but is light and has no global contention.
//!
//! ★Windows only★: the fixture is cmd.exe-based, so this file compiles and runs only on Windows
//!   (the project assumes Windows/WebView2).
#![cfg(windows)]

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use uuid::Uuid;

use engram_dashboard_agent::output_core::{OutputCore, TurnWiring};
use engram_dashboard_agent::transport::stdio::StdioTransport;
use engram_dashboard_agent::transport::{AgentTransport, OutputDecoder};
use engram_dashboard_agent::types::{
    AgentId, AgentInfo, AgentStatus, CommandSpec, InputEvent, OutputEvent, OutputFrame,
    OutputPayload, OutputSink, SinkError, SinkId, StatusSink,
};
use engram_dashboard_agent::usage::{UsageObservation, UsageSource, UsageVendorKey, WindowObs};
use engram_dashboard_base::testing::wait_until;

// ── RecordingSink: accumulates (seq, bytes) bytes + structured event tags ─────────────────
#[derive(Clone)]
struct RecordingSink {
    id: SinkId,
    events: Arc<Mutex<Vec<(u64, Vec<u8>)>>>,
    event_tags: Arc<Mutex<Vec<String>>>,
}
impl RecordingSink {
    fn new() -> Self {
        Self {
            id: Uuid::new_v4(),
            events: Arc::new(Mutex::new(Vec::new())),
            event_tags: Arc::new(Mutex::new(Vec::new())),
        }
    }
    fn concat(&self) -> Vec<u8> {
        let mut out = Vec::new();
        for (_, b) in self.events.lock().unwrap().iter() {
            out.extend_from_slice(b);
        }
        out
    }
    fn contains(&self, needle: &str) -> bool {
        String::from_utf8_lossy(&self.concat()).contains(needle)
    }
    fn seqs(&self) -> Vec<u64> {
        self.events.lock().unwrap().iter().map(|e| e.0).collect()
    }
    fn event_tags(&self) -> Vec<String> {
        self.event_tags.lock().unwrap().clone()
    }
    fn has_bytes(&self) -> bool {
        !self.events.lock().unwrap().is_empty()
    }
}
impl OutputSink for RecordingSink {
    fn send(&self, frame: OutputFrame<'_>) -> Result<(), SinkError> {
        match frame.payload {
            OutputPayload::Bytes(b) => {
                self.events.lock().unwrap().push((frame.seq, b.to_vec()));
            }
            OutputPayload::Event(e) => {
                self.event_tags.lock().unwrap().push(event_tag(e));
            }
        }
        Ok(())
    }
    fn sink_id(&self) -> SinkId {
        self.id
    }
}

fn event_tag(e: &OutputEvent) -> String {
    match e {
        OutputEvent::TerminalBytes(_) => "terminal".to_string(),
        OutputEvent::TextDelta { .. } => "text".to_string(),
        OutputEvent::ToolCall { name, .. } => format!("tool:{name}"),
        OutputEvent::ToolResult { outcome, .. } => format!("tool-result:{outcome:?}"),
        OutputEvent::Usage { .. } => "usage".to_string(),
        OutputEvent::MessageDone { .. } => "done".to_string(),
        OutputEvent::TurnEnd { .. } => "turn-end".to_string(),
        OutputEvent::Error(_) => "error".to_string(),
        OutputEvent::Structured { kind, .. } => format!("structured:{kind}"),
        OutputEvent::QueuedInput(_) => "queued-input".to_string(),
    }
}

// ── RecordingStatusSink: count of terminal transitions (verifies finalize once) + usage observations ─────────────
#[derive(Clone)]
struct RecordingStatusSink {
    statuses: Arc<Mutex<Vec<AgentStatus>>>,
    usage: Arc<Mutex<Vec<UsageObservation>>>,
}
impl RecordingStatusSink {
    fn new() -> Self {
        Self {
            statuses: Arc::new(Mutex::new(Vec::new())),
            usage: Arc::new(Mutex::new(Vec::new())),
        }
    }
    fn usage(&self) -> Vec<UsageObservation> {
        self.usage.lock().unwrap().clone()
    }
    fn terminal_count(&self) -> usize {
        self.statuses
            .lock()
            .unwrap()
            .iter()
            .filter(|s| {
                matches!(
                    s,
                    AgentStatus::Exited { .. } | AgentStatus::Killed | AgentStatus::Failed { .. }
                )
            })
            .count()
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
    fn usage_observed(&self, obs: UsageObservation) {
        self.usage.lock().unwrap().push(obs);
    }
}

fn spec(program: &str, args: &[&str]) -> CommandSpec {
    CommandSpec {
        program: program.to_string(),
        args: args.iter().map(|s| s.to_string()).collect(),
        env: vec![],
        cwd: PathBuf::from("."),
    }
}

#[test]
fn stdio_stdout_pump_and_finalize_once() {
    let id = Uuid::new_v4();
    // The structured argument is irrelevant to this pump test — open with the json path (true) (affects only caps).
    let (transport, _pid) =
        StdioTransport::open(&spec("cmd.exe", &["/c", "echo OUT-MARKER"]), true, None)
            .expect("open");

    let status_sink = RecordingStatusSink::new();
    let core = Arc::new(OutputCore::new(
        id,
        0,
        Arc::new(status_sink.clone()),
        TurnWiring::detached(),
    ));
    let transport: Box<dyn AgentTransport> = Box::new(transport);
    transport.start(core.clone());

    let out = RecordingSink::new();
    core.subscribe(Arc::new(out.clone()));

    assert!(
        wait_until(Duration::from_secs(5), || out.contains("OUT-MARKER")),
        "5s 내 stdout 마커 미수신"
    );

    assert!(
        wait_until(Duration::from_secs(5), || matches!(
            core.status(),
            AgentStatus::Exited { .. }
        )),
        "자연 종료가 Exited 로 finish 되지 않음: {:?}",
        core.status()
    );
    core.join_pump(Duration::from_secs(5));

    let seqs = out.seqs();
    assert!(!seqs.is_empty(), "출력 청크가 없음");
    assert_eq!(seqs[0], 0, "첫 seq 는 0");
    assert!(
        seqs.windows(2).all(|w| w[1] > w[0]),
        "seq 가 단조 증가여야 함: {seqs:?}"
    );

    assert_eq!(
        status_sink.terminal_count(),
        1,
        "finalize 1회 위반 — terminal 전이 {}건: {:?}",
        status_sink.terminal_count(),
        status_sink.statuses()
    );
}

#[test]
fn stdio_stderr_not_in_output_stream() {
    let id = Uuid::new_v4();
    // A single-argument compound command: cmd strips the outer quotes and runs it as `echo OUT& echo ERR 1>&2`
    // (redirecting echo's stdout to stderr). The markers are ASCII, so locale-independent.
    let (transport, _pid) = StdioTransport::open(
        &spec("cmd.exe", &["/c", "echo OUT-MARKER& echo ERR-MARKER 1>&2"]),
        true,
        None,
    )
    .expect("open");

    let status_sink = RecordingStatusSink::new();
    let core = Arc::new(OutputCore::new(
        id,
        0,
        Arc::new(status_sink),
        TurnWiring::detached(),
    ));
    let transport: Box<dyn AgentTransport> = Box::new(transport);
    transport.start(core.clone());
    let out = RecordingSink::new();
    core.subscribe(Arc::new(out.clone()));

    assert!(
        wait_until(Duration::from_secs(5), || out.contains("OUT-MARKER")),
        "5s 내 stdout 마커 미수신"
    );
    // Wait for exit to secure a point at which stderr has also been fully drained.
    assert!(
        wait_until(Duration::from_secs(5), || matches!(
            core.status(),
            AgentStatus::Exited { .. }
        )),
        "종료 미도달"
    );
    core.join_pump(Duration::from_secs(5));
    // Spare wait (time for the stderr drain thread to finish).
    std::thread::sleep(Duration::from_millis(200));

    assert!(out.contains("OUT-MARKER"), "stdout 마커는 출력에 있어야 함");
    assert!(
        !out.contains("ERR-MARKER"),
        "stderr 가 출력 스트림에 새어들어옴(NDJSON 오염): {}",
        String::from_utf8_lossy(&out.concat())
    );
}

/// cmd.exe (no arguments, pipe stdin) stays alive waiting for commands unless stdin is closed → a long-lived child.
#[test]
fn stdio_stdin_reaches_child_then_shutdown_kills() {
    let id = Uuid::new_v4();
    let (transport, _pid) = StdioTransport::open(&spec("cmd.exe", &[]), true, None).expect("open");

    let status_sink = RecordingStatusSink::new();
    let core = Arc::new(OutputCore::new(
        id,
        0,
        Arc::new(status_sink),
        TurnWiring::detached(),
    ));
    let transport: Box<dyn AgentTransport> = Box::new(transport);
    transport.start(core.clone());
    let out = RecordingSink::new();
    core.subscribe(Arc::new(out.clone()));

    transport
        .send_input(InputEvent::Raw(b"echo engram-echo-back\r\n".to_vec()))
        .expect("send_input");
    assert!(
        wait_until(Duration::from_secs(5), || out.contains("engram-echo-back")),
        "5s 내 echo-back 미수신(stdin 이 자식에 도달 못 함): {}",
        String::from_utf8_lossy(&out.concat())
    );

    assert!(
        !matches!(
            core.status(),
            AgentStatus::Exited { .. } | AgentStatus::Killed
        ),
        "echo-back 후에도 살아 있어야 함(장수 자식)"
    );

    let kill_started = Instant::now();
    transport.shutdown();
    core.join_pump(Duration::from_secs(5));
    let elapsed = kill_started.elapsed();
    assert!(
        elapsed < Duration::from_secs(5),
        "shutdown+join 이 5s 내 안 끝남(hang 의심): {elapsed:?}"
    );
    assert!(
        matches!(core.status(), AgentStatus::Killed),
        "shutdown 후 status 가 Killed 가 아님: {:?}",
        core.status()
    );
}

// ── S15 B3: decoder wiring (pump→core) ───────────────────────────────────────────────

/// A fake decoder that verifies only **whether the pump actually calls decoder.decode/flush**, without real parsing.
struct FakeDecoder {
    total_bytes: Arc<Mutex<usize>>,
}
impl OutputDecoder for FakeDecoder {
    fn decode(&mut self, chunk: &[u8]) -> Vec<OutputEvent> {
        *self.total_bytes.lock().unwrap() += chunk.len();
        vec![OutputEvent::Structured {
            kind: "fake-chunk".to_string(),
            json: "{}".to_string(),
        }]
    }
    fn flush(&mut self) -> Vec<OutputEvent> {
        vec![OutputEvent::MessageDone {
            turn_id: None,
            message_id: None,
        }]
    }
}

#[test]
fn stdio_decoder_routes_bytes_and_flushes_on_eof() {
    let id = Uuid::new_v4();
    let total_bytes = Arc::new(Mutex::new(0usize));
    let decoder: Box<dyn OutputDecoder> = Box::new(FakeDecoder {
        total_bytes: total_bytes.clone(),
    });
    let (transport, _pid) = StdioTransport::open(
        &spec("cmd.exe", &["/c", "echo ROUTE-ME"]),
        true,
        Some(decoder),
    )
    .expect("open");

    let status_sink = RecordingStatusSink::new();
    let core = Arc::new(OutputCore::new(
        id,
        0,
        Arc::new(status_sink),
        TurnWiring::detached(),
    ));
    let transport: Box<dyn AgentTransport> = Box::new(transport);
    transport.start(core.clone());
    let out = RecordingSink::new();
    core.subscribe(Arc::new(out.clone()));

    assert!(
        wait_until(Duration::from_secs(5), || matches!(
            core.status(),
            AgentStatus::Exited { .. }
        )),
        "종료 미도달: {:?}",
        core.status()
    );
    core.join_pump(Duration::from_secs(5));

    assert!(
        *total_bytes.lock().unwrap() > 0,
        "pump 가 자식 바이트를 decoder.decode 로 라우팅하지 않음"
    );
    let tags = out.event_tags();
    assert!(
        tags.iter().any(|t| t == "structured:fake-chunk"),
        "decode 산출 구조화 이벤트 미수신: {tags:?}"
    );
    assert_eq!(
        tags.last().map(String::as_str),
        Some("done"),
        "EOF 후 flush(MessageDone)가 마지막에 와야 함: {tags:?}"
    );
    assert!(
        !out.has_bytes(),
        "decoder 주입 경로에서 콘솔 바이트 프레임이 새면 안 됨"
    );
}

#[test]
fn stdio_decoder_flush_skipped_on_shutdown_kill() {
    let id = Uuid::new_v4();
    let total_bytes = Arc::new(Mutex::new(0usize));
    let decoder: Box<dyn OutputDecoder> = Box::new(FakeDecoder {
        total_bytes: total_bytes.clone(),
    });
    let (transport, _pid) =
        StdioTransport::open(&spec("cmd.exe", &[]), true, Some(decoder)).expect("open");

    let status_sink = RecordingStatusSink::new();
    let core = Arc::new(OutputCore::new(
        id,
        0,
        Arc::new(status_sink),
        TurnWiring::detached(),
    ));
    let transport: Box<dyn AgentTransport> = Box::new(transport);
    transport.start(core.clone());
    let out = RecordingSink::new();
    core.subscribe(Arc::new(out.clone()));

    // A cmd with no arguments may have no banner, so stdin echo is used to surely induce stdout.
    transport
        .send_input(InputEvent::Raw(b"echo kill-route-probe\r\n".to_vec()))
        .expect("send_input");
    assert!(
        wait_until(Duration::from_secs(5), || *total_bytes.lock().unwrap() > 0),
        "stdin echo 주입 후에도 decode 라우팅을 확인 못함"
    );

    transport.shutdown();
    core.join_pump(Duration::from_secs(5));
    assert!(
        matches!(core.status(), AgentStatus::Killed),
        "shutdown 후 status 가 Killed 가 아님: {:?}",
        core.status()
    );

    let tags = out.event_tags();
    assert!(
        !tags.iter().any(|t| t == "done"),
        "kill 경로에서 flush(done)가 방출됨 — shutdown 시 flush 스킵 회귀(FIX-2): {tags:?}"
    );
}

/// Proves the ClaudeStreamDecoder wiring through the real-process pump path — the trait delegation itself is
///   covered separately by the `backend::output_decoder` unit tests, so it is not looked at again here.
///
/// ※ NDJSON lines have many `"` so `cmd /c echo <json>` gets tangled with Rust arg quoting (`"` escape) and the bytes
///   break → instead write the NDJSON to a temp file and stream it with `cmd /c type <path>` (the path has no special characters).
#[test]
fn stdio_real_claude_decoder_emits_structured_event() {
    use engram_dashboard_agent::backend::claude::ClaudeStreamDecoder;
    use std::io::Write;

    let ndjson = concat!(
        r#"{"type":"assistant","message":{"id":"m1","content":[{"type":"text","text":"hi"}]}}"#,
        "\n",
        r#"{"type":"result","subtype":"success"}"#,
        "\n",
    );
    let mut path = std::env::temp_dir();
    path.push(format!("engram-b3-ndjson-{}.jsonl", Uuid::new_v4()));
    {
        let mut f = std::fs::File::create(&path).expect("temp create");
        f.write_all(ndjson.as_bytes()).expect("temp write");
    }
    let path_str = path.to_string_lossy().to_string();

    let id = Uuid::new_v4();
    let decoder: Box<dyn OutputDecoder> = Box::new(ClaudeStreamDecoder::new());
    let (transport, _pid) = StdioTransport::open(
        &spec("cmd.exe", &["/c", "type", &path_str]),
        true,
        Some(decoder),
    )
    .expect("open");

    let status_sink = RecordingStatusSink::new();
    let core = Arc::new(OutputCore::new(
        id,
        0,
        Arc::new(status_sink),
        TurnWiring::detached(),
    ));
    let transport: Box<dyn AgentTransport> = Box::new(transport);
    transport.start(core.clone());
    let out = RecordingSink::new();
    core.subscribe(Arc::new(out.clone()));

    assert!(
        wait_until(Duration::from_secs(5), || matches!(
            core.status(),
            AgentStatus::Exited { .. }
        )),
        "종료 미도달"
    );
    core.join_pump(Duration::from_secs(5));
    let _ = std::fs::remove_file(&path);

    let tags = out.event_tags();
    assert!(
        tags.iter().any(|t| t == "text"),
        "실 ClaudeStreamDecoder 가 assistant text 라인을 정제 못함: {tags:?}"
    );
    assert!(
        tags.iter().any(|t| t == "done"),
        "실 ClaudeStreamDecoder 가 result 라인을 MessageDone 으로 정제 못함: {tags:?}"
    );
    assert!(!out.has_bytes(), "구조화 경로에 콘솔 바이트 누출");
}

// ── Usage pickup: the pump passes the decoder's observations to the status sink ──────────────

fn observation(pct: f64) -> UsageObservation {
    UsageObservation {
        vendor: UsageVendorKey::new("test-vendor"),
        five_hour: Some(WindowObs {
            used_pct: Some(pct),
            resets_at: Some(1_900_000_000),
        }),
        weekly: None,
        model_scoped: None,
        plan: None,
        source: UsageSource::Passive,
        limits_unavailable: None,
    }
}

/// A decoder that accumulates a 10 % observation per `decode` and a 20 % observation at `flush` — measures whether the pump drains at both places.
struct UsageEmittingDecoder {
    pending: Vec<UsageObservation>,
}
impl OutputDecoder for UsageEmittingDecoder {
    fn decode(&mut self, _chunk: &[u8]) -> Vec<OutputEvent> {
        self.pending.push(observation(10.0));
        Vec::new()
    }
    fn flush(&mut self) -> Vec<OutputEvent> {
        self.pending.push(observation(20.0));
        Vec::new()
    }
    fn take_usage(&mut self) -> Vec<UsageObservation> {
        std::mem::take(&mut self.pending)
    }
}

#[test]
fn stdio_pump_forwards_usage_after_decode_and_flush() {
    let decoder: Box<dyn OutputDecoder> = Box::new(UsageEmittingDecoder {
        pending: Vec::new(),
    });
    let (transport, _pid) = StdioTransport::open(
        &spec("cmd.exe", &["/c", "echo USAGE-ME"]),
        true,
        Some(decoder),
    )
    .expect("open");

    let status_sink = RecordingStatusSink::new();
    let core = Arc::new(OutputCore::new(
        Uuid::new_v4(),
        0,
        Arc::new(status_sink.clone()),
        TurnWiring::detached(),
    ));
    let transport: Box<dyn AgentTransport> = Box::new(transport);
    transport.start(core.clone());

    assert!(
        wait_until(Duration::from_secs(5), || matches!(
            core.status(),
            AgentStatus::Exited { .. }
        )),
        "종료 미도달: {:?}",
        core.status()
    );
    core.join_pump(Duration::from_secs(5));

    let pcts: Vec<Option<f64>> = status_sink
        .usage()
        .iter()
        .map(|o| o.five_hour.and_then(|w| w.used_pct))
        .collect();
    assert!(
        pcts.iter().any(|p| *p == Some(10.0)),
        "decode 뒤 관측이 sink 에 안 닿음: {pcts:?}"
    );
    assert_eq!(
        pcts.last(),
        Some(&Some(20.0)),
        "flush 뒤 관측이 마지막에 와야 함: {pcts:?}"
    );
}

/// One round with the real `ClaudeStreamDecoder` — a `rate_limit_event` line becomes an observation at the sink with no output event.
/// (The reason for streaming NDJSON through a temp file = `stdio_real_claude_decoder_emits_structured_event`)
#[test]
fn stdio_real_claude_decoder_reports_rate_limit_usage() {
    use engram_dashboard_agent::backend::claude::ClaudeStreamDecoder;
    use std::io::Write;

    let ndjson = concat!(
        r#"{"type":"rate_limit_event","rate_limit_info":{"status":"allowed","rateLimitType":"five_hour","utilization":0.55,"resetsAt":1790425800}}"#,
        "\n",
        r#"{"type":"result","subtype":"success"}"#,
        "\n",
    );
    let mut path = std::env::temp_dir();
    path.push(format!("engram-usage-ndjson-{}.jsonl", Uuid::new_v4()));
    {
        let mut f = std::fs::File::create(&path).expect("temp create");
        f.write_all(ndjson.as_bytes()).expect("temp write");
    }
    let path_str = path.to_string_lossy().to_string();

    let decoder: Box<dyn OutputDecoder> = Box::new(ClaudeStreamDecoder::new());
    let (transport, _pid) = StdioTransport::open(
        &spec("cmd.exe", &["/c", "type", &path_str]),
        true,
        Some(decoder),
    )
    .expect("open");

    let status_sink = RecordingStatusSink::new();
    let core = Arc::new(OutputCore::new(
        Uuid::new_v4(),
        0,
        Arc::new(status_sink.clone()),
        TurnWiring::detached(),
    ));
    let transport: Box<dyn AgentTransport> = Box::new(transport);
    transport.start(core.clone());
    let out = RecordingSink::new();
    core.subscribe(Arc::new(out.clone()));

    assert!(
        wait_until(Duration::from_secs(5), || matches!(
            core.status(),
            AgentStatus::Exited { .. }
        )),
        "종료 미도달"
    );
    core.join_pump(Duration::from_secs(5));
    let _ = std::fs::remove_file(&path);

    let usage = status_sink.usage();
    assert_eq!(usage.len(), 1, "{usage:?}");
    assert_eq!(
        usage[0].five_hour,
        Some(WindowObs {
            used_pct: Some(55.0),
            resets_at: Some(1_790_425_800),
        })
    );
    let tags = out.event_tags();
    assert_eq!(
        tags,
        vec!["done".to_string()],
        "rate_limit_event 는 출력 이벤트가 되지 않는다"
    );
}

#[test]
fn stdio_no_decoder_passes_terminal_bytes_through() {
    let id = Uuid::new_v4();
    let (transport, _pid) =
        StdioTransport::open(&spec("cmd.exe", &["/c", "echo PLAIN-BYTES"]), false, None)
            .expect("open");

    let status_sink = RecordingStatusSink::new();
    let core = Arc::new(OutputCore::new(
        id,
        0,
        Arc::new(status_sink),
        TurnWiring::detached(),
    ));
    let transport: Box<dyn AgentTransport> = Box::new(transport);
    transport.start(core.clone());
    let out = RecordingSink::new();
    core.subscribe(Arc::new(out.clone()));

    assert!(
        wait_until(Duration::from_secs(5), || out.contains("PLAIN-BYTES")),
        "5s 내 바이트 직통 미수신"
    );
    assert!(
        wait_until(Duration::from_secs(5), || matches!(
            core.status(),
            AgentStatus::Exited { .. }
        )),
        "종료 미도달"
    );
    core.join_pump(Duration::from_secs(5));

    assert!(
        out.has_bytes(),
        "None 경로는 콘솔 바이트를 그대로 흘려야 함"
    );
    assert!(
        out.event_tags().is_empty(),
        "None 경로에서 구조화 이벤트가 나오면 안 됨: {:?}",
        out.event_tags()
    );
}
