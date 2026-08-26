//! ② 격리 통합테스트 — StdioTransport(파이프 자식) + OutputCore 신경로를 manager 없이 직접 단언.
//!
//! json 모드(ADR-0044)가 쓰는 transport. 실 claude 없이(격리 — ADR-0012) cmd.exe 가짜 자식으로
//! 검증한다. 실 프로세스를 spawn 하지만 가볍고 전역 경합이 없다.
//!
//! ★Windows 전용★: 픽스처가 cmd.exe 기반이라 이 파일은 Windows 에서만 컴파일·실행한다
//!   (프로젝트는 Windows/WebView2 전제).
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

// ── RecordingSink: (seq, bytes) 바이트 + 구조화 이벤트 태그 누적 ─────────────────────
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
        OutputEvent::Usage { .. } => "usage".to_string(),
        OutputEvent::MessageDone { .. } => "done".to_string(),
        OutputEvent::Error(_) => "error".to_string(),
        OutputEvent::Structured { kind, .. } => format!("structured:{kind}"),
    }
}

// ── RecordingStatusSink: terminal 전이 횟수(finalize 1회 검증) ─────────────────────
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
    // structured 인자는 이 pump 테스트와 무관 — json 경로(true)로 open(caps 만 영향).
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
    // 단일 인자 복합 명령: cmd 가 바깥 따옴표를 벗기고 `echo OUT& echo ERR 1>&2` 로 실행한다
    // (echo 의 stdout 을 stderr 로 리다이렉트). 마커는 ASCII 라 로케일 무관.
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
    // 종료까지 대기해 stderr 도 다 흘렀을 시점 확보.
    assert!(
        wait_until(Duration::from_secs(5), || matches!(
            core.status(),
            AgentStatus::Exited { .. }
        )),
        "종료 미도달"
    );
    core.join_pump(Duration::from_secs(5));
    // 여유 대기(stderr drain 스레드가 끝날 시간).
    std::thread::sleep(Duration::from_millis(200));

    assert!(out.contains("OUT-MARKER"), "stdout 마커는 출력에 있어야 함");
    assert!(
        !out.contains("ERR-MARKER"),
        "stderr 가 출력 스트림에 새어들어옴(NDJSON 오염): {}",
        String::from_utf8_lossy(&out.concat())
    );
}

/// cmd.exe(무인자, 파이프 stdin)는 stdin 을 안 닫으면 명령 대기로 계속 살아 있다 → 장수 자식.
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

// ── S15 B3: decoder 배선(pump→core) ─────────────────────────────────────────────

/// 실 파싱 없이 **pump 가 decoder.decode/flush 를 실제로 호출하는지**만 검증하는 fake decoder.
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

    // 무인자 cmd 는 배너가 없을 수 있어, stdin echo 로 stdout 을 확실히 유도한다.
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

/// 실 프로세스 pump 경로로 ClaudeStreamDecoder 배선을 증명한다 — 트레이트 위임 자체는
///   `backend::output_decoder` 단위테스트가 별도 커버하므로 여기서 다시 보지 않는다.
///
/// ※ NDJSON 라인엔 `"` 가 많아 `cmd /c echo <json>` 은 Rust arg 인용(`"` escape)이 얽혀 바이트가
///   깨진다 → 대신 임시 파일에 NDJSON 을 쓰고 `cmd /c type <path>` 로 흘린다(경로엔 특수문자 없음).
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
