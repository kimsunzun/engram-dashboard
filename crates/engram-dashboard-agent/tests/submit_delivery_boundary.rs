//! ④ A regression net at the **physical boundary** of the delivery verb (`AgentSession::submit_input_observed`).
//!
//! The two items here each measure one contract the queue broke. ★Both go through a **real OS pipe**, not a seam★ — what we measure is not
//! "what was called" but **what the receiver read, and when**.
//!
//! ★Why a seam cannot measure it★: the `session.rs` pacing item records only **whether** the injected sleeper was called. The post-queue
//!   defect is "the wait was measured **from the wrong moment**", which a call-record oracle lets pass; only the real read boundary tells
//!   them apart.
//!
//! ★Windows only★: the child is powershell (a project premise).
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

/// Stands up `AgentSession` on a real pipe child. ★encoder = `Raw`★ so `submit_sequence()` is `Some(b"\r")` and the **body/submit
///   two-write** path runs.
/// ★Pacing · sleeper stay at production defaults★ (no `with_submit_pacing`) — we measure the physical interval that wait creates.
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

/// ★Proves★: the submit byte (CR) arrives with **its own read** — not bundled with the body's tail. That is the property
///   `backend::SUBMIT_PACING` exists to keep (without the interval the claude TUI reads CR as pasted text and **does not start the turn**
///   — ADR-0038).
///
/// ★Oracle★: per `Read()` the child echoes `READ <byte count> <last byte>`. A last line `READ 1 13` = CR read alone; last byte 13 with
///   length ≠ 1 = read together with the body = this defect.
///
/// ★Fails on the pre-fix code★: the wait was measured from **queue time**; if draining the body outlasts the wait (this child is built
///   so), the writer writes the CR right after the body and it rides in the same read as the tail. After the fix it confirms the body's OS
///   landing first, then sleeps 500ms, so the pipe is empty and the CR stands alone.
///
/// ★「The child does not read stdin anyway」 is no excuse★: this child **reads normally without rest**, yet the body far exceeds the pipe
///   buffer, so draining takes several cycles and the interval collapses to 0 between them.
#[test]
fn submit_byte_lands_in_its_own_read_even_when_the_body_takes_many_write_cycles() {
    // Reads 8KiB at a time, reports each read, then rests briefly — a "normal but slow receiver". Draining the body (512KiB+ below) takes
    //   ~64 cycles, longer than SUBMIT_PACING (500ms).
    let script = "\
$stdin=[Console]::OpenStandardInput();\
$out=New-Object System.IO.StreamWriter([Console]::OpenStandardOutput());\
$buf=New-Object byte[] 8192;\
while(($n=$stdin.Read($buf,0,$buf.Length)) -gt 0){\
$out.WriteLine(\"READ $n $($buf[$n-1])\");$out.Flush();\
Start-Sleep -Milliseconds 10}";

    let (session, sink) = session_over(script);

    // ★Avoid multiples of 8192★: if it divides exactly, the body's last read ends exactly at the boundary and the CR may **by chance**
    //   ride alone — passing even with the defect (the oracle goes blind).
    const BODY: usize = 512 * 1024 + 777;
    let body = vec![b'x'; BODY];

    let started = Instant::now();
    session
        .submit_input_observed(&body)
        .expect("delivery to a normal receiver must succeed");
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
            "no CR (13) report within 20s — the submit byte did not arrive. So far: {}",
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
    assert!(!reads.is_empty(), "the child reported not a single read");

    let total: usize = reads.iter().map(|(n, _)| *n).sum();
    assert_eq!(
        total,
        BODY + 1,
        "the total received differs from body+CR — loss/duplication (lines read: {})",
        tail(&text)
    );

    let (last_n, last_b) = *reads.last().expect("not empty");
    assert_eq!(
        (last_n, last_b),
        (1, 13),
        "★the submit byte did not arrive in its own read★ — it was bundled with the body tail into one chunk (last read = {last_n} bytes, last byte {last_b}). \
         body drain {elapsed:?}. This is exactly the state SUBMIT_PACING is meant to prevent. Lines read: {}",
        tail(&text)
    );

    session.kill(Duration::from_secs(5));
}

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
/// ★Why CRITICAL★: this verb's `Ok` becomes the delivery receipt as is (`InjectReceipt` → `DeliveryObservation`) and the ledger records
///   "delivery succeeded". Reporting acceptance as landing **flips** the two cases ADR-0088 exists to tell apart (transmission failed,
///   ledger says "delivered and the model ignored it"), and a later `Err` cannot fix a written record.
///
/// ★Fails on the pre-fix code★: it returned `Ok(WriteOutcome)` as soon as the queue accepted; this child dies after reading a little, so
///   the write fails and most of the body never goes out, yet a receipt was issued.
///
/// ★Control group alongside★: a healthy receiver must still be `Ok`, else an implementation that "fails everything" would pass.
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
            .expect("★control group★ delivery to a normal receiver must produce a receipt");
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
        "★a delivery receipt came out for a body that did not go out★ — the ledger records 'send failure' flipped as 'ignored by the model': {result:?}"
    );

    session.kill(Duration::from_secs(5));
}
