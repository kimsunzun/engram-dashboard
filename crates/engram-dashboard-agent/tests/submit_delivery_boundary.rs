//! ④ A regression net at the **physical boundary** of the delivery verb (`AgentSession::submit_input_observed`).
//!
//! The two items in this file each measure one of the two contracts that introducing the queue broke. ★Both go through a **real OS pipe**, not a seam
//! ★ — because what we want to measure is not "what was called" but **what the receiver read, and when**.
//!
//! ★Why a seam cannot measure it (the place the existing item missed)★: the existing pacing regression item in `session.rs` records only
//!   **whether** the injected sleeper was called. The defect after the queue appeared is not "it did not wait" but "it measured the wait **from the wrong
//!   moment**", so an oracle that looks only at the call record lets that defect pass as is. Only by looking at the real read boundary
//!   can they be told apart.
//!
//! ★Windows only★: the child is powershell, so it compiles and runs only on Windows (a project premise).
// ADR-0088 · ADR-0038
#![cfg(windows)]

use std::path::PathBuf;
use std::sync::atomic::AtomicU8;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use uuid::Uuid;

use engram_dashboard_agent::backend::{AgentBackend, InputEncoder, ShellBackend};
use engram_dashboard_agent::output_core::{OutputCore, TurnWiring};
use engram_dashboard_agent::session::AgentSession;
use engram_dashboard_agent::transport::stdio::StdioTransport;
use engram_dashboard_agent::transport::AgentTransport;
use engram_dashboard_agent::types::{
    AgentId, AgentInfo, AgentStatus, CommandSpec, OutputFrame, OutputPayload, OutputSink,
    SinkError, SinkId, StatusSink,
};

// ── Collecting sink: gathers in order the text the child echoed back ───────────────────────
#[derive(Clone)]
struct TextSink {
    id: SinkId,
    chunks: Arc<Mutex<Vec<(u64, Vec<u8>)>>>,
}
impl TextSink {
    fn new() -> Self {
        Self {
            id: Uuid::new_v4(),
            chunks: Arc::new(Mutex::new(Vec::new())),
        }
    }
    fn text(&self) -> String {
        let mut v = self.chunks.lock().unwrap().clone();
        v.sort_by_key(|(seq, _)| *seq);
        let mut out = Vec::new();
        for (_, b) in v {
            out.extend_from_slice(&b);
        }
        String::from_utf8_lossy(&out).into_owned()
    }
}
impl OutputSink for TextSink {
    fn send(&self, frame: OutputFrame<'_>) -> Result<(), SinkError> {
        if let OutputPayload::Bytes(b) = frame.payload {
            self.chunks.lock().unwrap().push((frame.seq, b.to_vec()));
        }
        Ok(())
    }
    fn sink_id(&self) -> SinkId {
        self.id
    }
}

struct NoopStatusSink;
impl StatusSink for NoopStatusSink {
    fn status_changed(&self, _id: AgentId, _s: AgentStatus, _e: u32) {}
    fn agent_list_updated(&self, _a: Vec<AgentInfo>) {}
}

/// Stands up `AgentSession` on a real pipe child. ★encoder = `Raw`★ — so that `submit_sequence()` is
/// `Some(b"\r")` and the **body/submit two-write** path this file measures actually runs.
/// ★Leave pacing · sleeper at the production defaults★ (do not shorten with `with_submit_pacing`) — what we measure is
/// the physical interval that wait creates.
fn session_over(script: &str) -> (AgentSession, TextSink) {
    let spec = CommandSpec {
        program: "powershell.exe".into(),
        args: vec!["-NoProfile".into(), "-Command".into(), script.into()],
        env: vec![],
        cwd: PathBuf::from("."),
    };
    let (transport, _pid) = StdioTransport::open(&spec, false, None).expect("open");
    let id = Uuid::new_v4();
    let core = Arc::new(OutputCore::new(
        id,
        0,
        Arc::new(NoopStatusSink) as Arc<dyn StatusSink>,
        TurnWiring::detached(),
    ));
    let transport: Box<dyn AgentTransport> = Box::new(transport);
    transport.start(core.clone());

    let shell_cmd = engram_dashboard_agent::profile::AgentCommand::Shell {
        program: "powershell.exe".into(),
        args: vec![],
    };
    let backend_caps = ShellBackend.capabilities(&shell_cmd);
    let reads_messages = engram_dashboard_agent::backend::reads_messages(&shell_cmd);
    let session = AgentSession::new(
        id,
        PathBuf::from("."),
        0,
        80,
        24,
        Arc::new(AtomicU8::new(0)),
        backend_caps,
        InputEncoder::Raw,
        reads_messages,
        core,
        transport,
    );
    let sink = TextSink::new();
    session.subscribe(Arc::new(sink.clone()));
    (session, sink)
}

// ═══════════════════════════════════════════════════════════════════════════════════════════
// FINDING 2 — the submit interval must be measured from **after the body has actually gone out**
// ═══════════════════════════════════════════════════════════════════════════════════════════

/// ★Proves★: the submit byte (CR) arrives at the receiver with **its own read** — that is, it is not bundled into one chunk with the tail of the body. That is
///   the very property `backend::SUBMIT_PACING` is meant to keep by measurement (without the interval the
///   claude TUI reads CR as part of the pasted text and **does not start the turn** — ADR-0038).
///
/// ★Oracle★: per `Read()` the child echoes one line `READ <바이트수> <마지막바이트>`. If the last line is
///   `READ 1 13`, the CR was read alone. If any line has last byte 13 but length not 1, it was read
///   together with the body, and = this defect.
///
/// ★Why this item **actually fails on the pre-fix code**★: before the fix, the wait was measured from the **time it was put in the queue**.
///   If draining the body takes longer than the wait (the child below is built that way), the body is still
///   going out even after 500ms, so the writer writes the CR right after finishing the body → the CR rides in the same read as the tail of the body.
///   After the fix, it first confirms the body's OS landing and then sleeps 500ms, so the pipe is empty and the CR stands alone.
///
/// ★★This child shows that the reviewer's counterexample was right — the excuse 「자식이 stdin 을 안 읽으니 어차피 안 먹힌다」 (the child does not read stdin, so it would not work anyway)
///   does not hold★★: the child below **reads normally without rest.** Even so, the body is far larger than the pipe
///   buffer, so draining takes several cycles, and the interval collapses to 0 in between.
#[test]
fn submit_byte_lands_in_its_own_read_even_when_the_body_takes_many_write_cycles() {
    // Reads 8KiB at a time, reports each read, then rests briefly — a "normal but slow receiver". Draining the body (512KiB+ below)
    //   takes roughly 64 cycles, so it takes longer than SUBMIT_PACING (500ms).
    let script = "\
$stdin=[Console]::OpenStandardInput();\
$out=New-Object System.IO.StreamWriter([Console]::OpenStandardOutput());\
$buf=New-Object byte[] 8192;\
while(($n=$stdin.Read($buf,0,$buf.Length)) -gt 0){\
$out.WriteLine(\"READ $n $($buf[$n-1])\");$out.Flush();\
Start-Sleep -Milliseconds 10}";

    let (session, sink) = session_over(script);

    // ★Avoid multiples of 8192★: if it divides exactly, the body's last read ends exactly at the boundary and the CR may **by chance**
    //   ride alone — then it passes even with the defect (the oracle goes blind).
    const BODY: usize = 512 * 1024 + 777;
    let body = vec![b'x'; BODY];

    let started = Instant::now();
    session
        .submit_input_observed(&body)
        .expect("정상 수신자 상대 배달은 성공해야 한다");
    let elapsed = started.elapsed();

    // Leeway for the child to report its last read after finishing through submit.
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let text = sink.text();
        if text.lines().filter(|l| l.starts_with("READ ")).count() > 0
            && text.trim_end().ends_with("13")
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "20s 안에 CR(13) 보고가 안 왔다 — 제출 바이트가 도착하지 않았다. 지금까지: {}",
            tail(&text)
        );
        std::thread::sleep(Duration::from_millis(20));
    }

    let text = sink.text();
    let reads: Vec<(usize, u8)> = text
        .lines()
        .filter_map(|l| {
            let mut it = l.strip_prefix("READ ")?.split_whitespace();
            Some((it.next()?.parse().ok()?, it.next()?.parse().ok()?))
        })
        .collect();
    assert!(!reads.is_empty(), "자식이 read 를 하나도 보고하지 않았다");

    let total: usize = reads.iter().map(|(n, _)| *n).sum();
    assert_eq!(
        total,
        BODY + 1,
        "수신 총량이 본문+CR 과 다르다 — 유실/중복(읽은 줄: {})",
        tail(&text)
    );

    let (last_n, last_b) = *reads.last().expect("비지 않음");
    assert_eq!(
        (last_n, last_b),
        (1, 13),
        "★제출 바이트가 자기만의 read 로 도착하지 않았다★ — 본문 꼬리와 한 덩이로 묶였다(마지막 read = {last_n} 바이트, 끝 바이트 {last_b}). \
         본문 드레인 {elapsed:?}. 이것이 SUBMIT_PACING 이 막으려는 바로 그 상태다. 읽은 줄: {}",
        tail(&text)
    );

    session.kill(Duration::from_secs(5));
}

/// Only the last few lines, so a failure message does not cover the screen.
fn tail(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let start = lines.len().saturating_sub(6);
    lines[start..].join(" | ")
}

// ═══════════════════════════════════════════════════════════════════════════════════════════
// FINDING 1 — the delivery receipt must mean **landing**, not **acceptance**
// ═══════════════════════════════════════════════════════════════════════════════════════════

/// ★Proves★: `submit_input_observed` does not emit `Ok` when the body actually could not go out.
///
/// ★Why this was CRITICAL★: this verb's `Ok` becomes the delivery receipt as is (`InjectReceipt` →
///   `DeliveryObservation`), and the ledger records it as "delivery succeeded". Reporting acceptance as landing makes
///   the two cases ADR-0088 exists to tell apart **exactly flip** — the transmission actually failed, but the ledger
///   says "delivered and the model ignored it". "The next call gives `Err`" cannot fix a record already written.
///
/// ★Why this item **actually fails on the pre-fix code**★: before the fix `submit_input_observed` returned `Ok(WriteOutcome)` as soon as the queue
///   accepted. The child below dies after reading a little, so the writer's write fails and
///   most of the body never goes out, yet a receipt was issued.
///
/// ★Put a control group alongside★: for a healthy receiver it must still be `Ok` — without it, an implementation that passes by "failing everything"
///   would satisfy this item.
#[test]
fn receipt_is_refused_when_the_body_never_reaches_the_child() {
    // Control group first — delivery holds for a child that keeps reading.
    {
        let script = "\
$stdin=[Console]::OpenStandardInput();\
$buf=New-Object byte[] 8192;\
while(($n=$stdin.Read($buf,0,$buf.Length)) -gt 0){}";
        let (session, _sink) = session_over(script);
        session
            .submit_input_observed(b"healthy-recipient")
            .expect("★대조군★ 정상 수신자 상대 배달은 영수증이 나와야 한다");
        session.kill(Duration::from_secs(5));
    }

    // The item itself — a child that reads only 64KiB and dies. The body is far larger than that.
    const K: usize = 64 * 1024;
    let script = format!(
        "\
$stdin=[Console]::OpenStandardInput();\
$buf=New-Object byte[] 4096;\
$total=0;\
while($total -lt {K}){{\
$cap=[Math]::Min($buf.Length,{K}-$total);\
$n=$stdin.Read($buf,0,$cap);\
if($n -le 0){{break}};\
$total+=$n}};\
exit 0"
    );
    let (session, _sink) = session_over(&script);

    let result = session.submit_input_observed(&vec![b'y'; 512 * 1024]);
    assert!(
        result.is_err(),
        "★안 나간 본문에 배달 영수증이 나왔다★ — 원장이 '전송 실패' 를 '모델이 무시' 로 뒤집어 적는다: {result:?}"
    );

    session.kill(Duration::from_secs(5));
}
