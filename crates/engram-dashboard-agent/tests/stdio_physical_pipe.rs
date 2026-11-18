//! ③ Physical OS-pipe layer integration test — verifies delivery accuracy with a real StdioTransport + a real OS pipe.
//!
//! The ADR-0088 Stage 1 daemon seam test (control_send.rs) has `SeamTransport` capture an already complete Vec with an atomic `push`, so
//!   it **bypasses the physical pipe layer**; this file covers the production StdioTransport's physical stdin write path.
//!
//! ★The path's runner changed — the items here stand on that★: `send_input` only enqueues into a bounded queue and the **dedicated writer
//!   thread** writes (`transport::input_queue`). So this file always calls `transport.start()` (the writer spawns there) and reads
//!   `send_input`'s `Ok`
//!   as **"accepted"**, not "delivered"; delivery is always measured by the child's echo.
//!
//! ★Windows only★: the child is powershell (a project premise).
// ADR-0088
#![cfg(windows)]

use std::path::PathBuf;
use std::sync::{Arc, Barrier, Mutex};
use std::time::{Duration, Instant};

use uuid::Uuid;

use engram_dashboard_agent::output_core::{OutputCore, TurnWiring};
use engram_dashboard_agent::transport::input_queue::INPUT_QUEUE_MAX_BYTES;
use engram_dashboard_agent::transport::stdio::StdioTransport;
use engram_dashboard_agent::transport::AgentTransport;
use engram_dashboard_agent::types::OutputSink;
use engram_dashboard_agent::types::{
    AgentId, AgentInfo, AgentStatus, CommandSpec, InputEvent, OutputFrame, OutputPayload, PtyError,
    SinkError, SinkId, StatusSink,
};

// ── Collecting sink ───────────────────────────────────────────────────────────────────────
#[derive(Clone)]
struct CollectingSink {
    id: SinkId,
    chunks: Arc<Mutex<Vec<(u64, Vec<u8>)>>>,
}
impl CollectingSink {
    fn new() -> Self {
        Self {
            id: Uuid::new_v4(),
            chunks: Arc::new(Mutex::new(Vec::new())),
        }
    }
    /// Stream order = the child's stdout order.
    fn concat_ordered(&self) -> Vec<u8> {
        let mut v = self.chunks.lock().unwrap().clone();
        v.sort_by_key(|(seq, _)| *seq);
        let mut out = Vec::new();
        for (_, b) in v {
            out.extend_from_slice(&b);
        }
        out
    }
    fn total_len(&self) -> usize {
        self.chunks
            .lock()
            .unwrap()
            .iter()
            .map(|(_, b)| b.len())
            .sum()
    }
}
impl OutputSink for CollectingSink {
    fn send(&self, frame: OutputFrame<'_>) -> Result<(), SinkError> {
        // It is the decoder=None path, so the payload is always Bytes (TerminalBytes direct). No Event comes.
        if let OutputPayload::Bytes(b) = frame.payload {
            self.chunks.lock().unwrap().push((frame.seq, b.to_vec()));
        }
        Ok(())
    }
    fn sink_id(&self) -> SinkId {
        self.id
    }
}

// ── status sink: this file does not assert state transitions, so no-op ──────────────────────────
struct NoopStatusSink;
impl StatusSink for NoopStatusSink {
    fn status_changed(&self, _id: AgentId, _status: AgentStatus, _epoch: u32) {}
    fn agent_list_updated(&self, _agents: Vec<AgentInfo>) {}
}

fn spec(program: &str, args: &[&str]) -> CommandSpec {
    CommandSpec {
        program: program.to_string(),
        args: args.iter().map(|s| s.to_string()).collect(),
        env: vec![],
        cwd: PathBuf::from("."),
    }
}

// Test 1 — physical OS-pipe concurrent write with no interleaving (stdin.lock() regression net)

/// ★Proves★: **application-layer serialization** of the physical pipe — even if one logical message is split across several OS writes,
///   another writer cannot slip in. ★The bearer changed★: formerly `send_input` held `stdin.lock()` across write_all+flush; now **one
///   chunk is one queue entry and
///   only one thread takes it out and writes** (`transport::input_queue`). The measured property and oracle are unchanged: the
///   concatenated stream is **exactly N contiguous runs** (1 per fill byte, each exactly L) = no interleaving (else > N runs) + no loss
///   (total and each run exact) + no duplication/substitution.
///   ▷ Verified regression shape = **"one logical message written with several OS writes without an exclusive lock"** (mutating to
///   chunked-writes-lock-dropped grew the runs to 6 > N=4 and failed).
/// ★Does not prove★:
///   (1) That interleaving **actually occurs without the lock** cannot be forced — with the lock present it proves only the **absence**;
///   slow reader · backpressure · L ≥ pipe buffer maximize the window.
///   (2) It does **not claim** to catch a **lock-free single-WriteFile implementation** (each 256 KiB message as one blocking WriteFile) —
///   NPFS (anonymous byte-mode pipe) serializes the whole request in the kernel, so runs could still be N; that atomicity is
///   **undocumented kernel behavior**, asserted neither way. Production
///       cannot be touched to force a multi-syscall write, so the claim is limited to application-layer serialization.
#[test]
fn physical_pipe_concurrent_sends_no_interleave() {
    // Slow reader echo child (1 ms per 8 KiB chunk) → the pipe fills with backpressure, which **widens the interleaving window** for a
    //   write split across several OS writes. A byte loop over the Console standard streams — cmd.exe cannot (no byte-level control), so
    //   powershell.
    let child_script = "\
$stdin=[Console]::OpenStandardInput();\
$stdout=[Console]::OpenStandardOutput();\
$buf=New-Object byte[] 8192;\
while(($n=$stdin.Read($buf,0,$buf.Length)) -gt 0){\
$stdout.Write($buf,0,$n);$stdout.Flush();Start-Sleep -Milliseconds 1}";

    let (transport, _pid) = StdioTransport::open(
        &spec("powershell.exe", &["-NoProfile", "-Command", child_script]),
        false, // Plain-text stdio — TerminalBytes direct (decoder=None)
        None,
    )
    .expect("open");
    let transport = Arc::new(transport);

    let sink = CollectingSink::new();
    let core = Arc::new(OutputCore::new(
        Uuid::new_v4(),
        0,
        Arc::new(NoopStatusSink),
        TurnWiring::detached(),
    ));
    transport.start(core.clone());
    core.subscribe(Arc::new(sink.clone()));

    // L comfortably exceeds the OS pipe buffer (maximizes the interleaving window without the lock); 256 KiB × 4 = 1 MiB, within seconds
    //   even at 8 KiB/1 ms ≈ 8 MiB/s.
    const N: usize = 4;
    const L: usize = 256 * 1024;

    // ★FIX 3★: every failure/timeout/panic-join/spawn-failure path cleans up the blocked writer · child · pump with transport.shutdown()
    //   before panicking (no leaks while other tests run concurrently).
    macro_rules! fail {
        ($($arg:tt)*) => {{
            transport.shutdown();
            panic!($($arg)*);
        }};
    }

    let barrier = Arc::new(Barrier::new(N));
    let mut handles = Vec::with_capacity(N);
    for i in 0..N {
        let t = transport.clone();
        let b = barrier.clone();
        // ★Thread-creation failure (round-3 MEDIUM)★: a raw spawn would panic at once and break the all-panic-paths-cleanup contract
        //   without shutdown → take the Err from Builder::spawn and route it to fail! (panic after shutdown). Writers already spawned are
        //   parked at Barrier::wait (an N-party barrier
        //   opens only when N arrive), so even shutdown's pipe kill cannot wake them — they stay detached until process exit (a spawn
        //   failure is already a big failure).
        let h = match std::thread::Builder::new().spawn(move || {
            let fill = b'A' + i as u8;
            let payload = vec![fill; L];
            b.wait(); // Entry alignment — maximize the contention window with simultaneous entry.
            t.send_input(InputEvent::Raw(payload))
        }) {
            Ok(h) => h,
            Err(e) => fail!("failed to create the OS thread of writer {i}: {e} — writers spawned earlier are parked at the Barrier (detached, collected when the process exits)"),
        };
        handles.push(h);
    }
    // ★watchdog (FIX 2)★: a child that stops draining stdin blocks write_all holding the lock by backpressure → a naive `join()` never
    //   returns; poll `is_finished()` until the deadline to turn the hang into a failure.

    let join_deadline = Instant::now() + Duration::from_secs(120);
    for (i, h) in handles.into_iter().enumerate() {
        while !h.is_finished() {
            if Instant::now() >= join_deadline {
                fail!("writer {i} did not return within 120s — write_all hung on pipe backpressure (suspected stall of the child's echo drain)");
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let r = match h.join() {
            Ok(r) => r,
            Err(_) => fail!("writer {i} thread panicked — internal failure of send_input"),
        };
        if !r.is_ok() {
            fail!("writer {i} send_input failed: {r:?}");
        }
    }

    let expected_total = N * L;
    let deadline = Instant::now() + Duration::from_secs(120);
    while sink.total_len() < expected_total {
        if Instant::now() >= deadline {
            let got = sink.total_len();
            transport.shutdown();
            panic!("the echo total did not reach the expected value ({expected_total}) within 120s (now {got}). Suspected child echo hang.");
        }
        std::thread::sleep(Duration::from_millis(20));
    }

    let stream = sink.concat_ordered();
    if stream.len() != expected_total {
        fail!(
            "the echo total bytes do not match N*L (loss/excess): got {}, want {expected_total}",
            stream.len()
        );
    }
    let runs = run_length_encode(&stream);
    if runs.len() != N {
        fail!(
            "the run count is not N ({N}) — interleaving happened (stdin.lock() regression). Run summary: {:?}",
            summarize_runs(&runs)
        );
    }
    // The order of runs is irrelevant (contention).
    let mut seen_bytes = std::collections::HashSet::new();
    for (byte, len) in &runs {
        if *len != L {
            fail!(
                "the run length of byte {byte:#x} is not L ({L}) ({len}) — loss or fragmentation"
            );
        }
        if !seen_bytes.insert(*byte) {
            fail!("byte {byte:#x} appears as a run twice or more — duplication/fragmentation (interleaving)");
        }
    }
    for i in 0..N {
        let fill = b'A' + i as u8;
        if !seen_bytes.contains(&fill) {
            fail!("fill byte {fill:#x} is not in the stream — writer {i} lost");
        }
    }

    transport.shutdown();
    core.join_pump(Duration::from_secs(10));
}

fn run_length_encode(bytes: &[u8]) -> Vec<(u8, usize)> {
    let mut runs: Vec<(u8, usize)> = Vec::new();
    for &b in bytes {
        match runs.last_mut() {
            Some((val, len)) if *val == b => *len += 1,
            _ => runs.push((b, 1)),
        }
    }
    runs
}

fn summarize_runs(runs: &[(u8, usize)]) -> Vec<(char, usize)> {
    runs.iter()
        .take(20)
        .map(|(b, l)| (*b as char, *l))
        .collect()
}

// ═══════════════════════════════════════════════════════════════════════════════════════════
// Test 2 — exceeding the bound is a **refusal** (it is not silently dropped)
// ═══════════════════════════════════════════════════════════════════════════════════════════

/// ★The item once here (`physical_pipe_partial_write_then_err_surfaces_as_err`) **no longer holds — the contract changed, it was not
///   weakened**★: it relied on 「send_input runs write_all on the calling thread」 and measured 「that call returns Err even after the prefix
///   has gone out」. Now `send_input` only enqueues
///   (`transport::input_queue`), so **the only failure that call can see is 「not accepted」**; the other half (「how a failure after
///   accepting is reported」) moved to Test 3.
///
/// ★Proves★: a payload over the queue bound is **refused whole** (`Err`), and ★as direct evidence the refusal is not a silent loss, **the
///   child receives not a single byte**★; a regression that partially accepts an overflow or writes only the front and returns `Ok` is
///   caught by one of the two assertions.
/// (The physical-layer edition of ADR-0190: "if exceeded, that call returns `Err` — it is not discarded".)
#[test]
fn physical_pipe_oversized_payload_is_refused_whole() {
    // The child keeps reading stdin and echoing — this measures that **it could have been delivered but was not** (not that it could not
    //   because the child did not read).
    let child_script = "\
$stdin=[Console]::OpenStandardInput();\
$stdout=[Console]::OpenStandardOutput();\
$buf=New-Object byte[] 8192;\
while(($n=$stdin.Read($buf,0,$buf.Length)) -gt 0){\
$stdout.Write($buf,0,$n);$stdout.Flush()}";

    let (transport, _pid) = StdioTransport::open(
        &spec("powershell.exe", &["-NoProfile", "-Command", child_script]),
        false,
        None,
    )
    .expect("open");

    let sink = CollectingSink::new();
    let core = Arc::new(OutputCore::new(
        Uuid::new_v4(),
        0,
        Arc::new(NoopStatusSink),
        TurnWiring::detached(),
    ));
    transport.start(core.clone());
    core.subscribe(Arc::new(sink.clone()));

    let over = vec![b'z'; INPUT_QUEUE_MAX_BYTES + 1];
    let result = transport.send_input(InputEvent::Raw(over));
    if !matches!(result, Err(PtyError::WriteFailed(_))) {
        transport.shutdown();
        panic!(
            "exceeding the cap must be refused with WriteFailed (no disguising as Ok): {result:?}"
        );
    }

    // ★Direct evidence that it is not a silent loss★: for 1 s after the refusal the child echoes nothing back; a partial-acceptance
    //   regression would echo that front part.
    std::thread::sleep(Duration::from_secs(1));
    let leaked = sink.total_len();

    // Control group — below the bound it is delivered normally (rules out that the 0 above means "the path was dead from the start").
    const PROBE: &[u8] = b"under-the-bound\n";
    transport
        .send_input(InputEvent::Raw(PROBE.to_vec()))
        .expect("a payload below the cap must be accepted");
    let deadline = Instant::now() + Duration::from_secs(30);
    while sink.total_len() < PROBE.len() {
        if Instant::now() >= deadline {
            let got = sink.total_len();
            transport.shutdown();
            panic!(
                "the control group was not echoed within 30s ({got}) — the link was dead to begin with, so the 0 above is vacuous"
            );
        }
        std::thread::sleep(Duration::from_millis(20));
    }

    if leaked != 0 {
        transport.shutdown();
        panic!("{leaked} bytes of the refused payload went to the child — regression of partial acceptance (silent loss)");
    }

    transport.shutdown();
    core.join_pump(Duration::from_secs(10));
}

// ═══════════════════════════════════════════════════════════════════════════════════════════
// Test 3 — a write that failed after being accepted is refused by the **next call** carrying the reason
// ═══════════════════════════════════════════════════════════════════════════════════════════

/// ★Proves★:
///   (a) If accepted bytes **cannot go out because the child died**, the path closes and ★the **next** `send_input` returns `Err` carrying
///   the reason★ — the failure is not swallowed. (No way to return it to the accepting call is the design's known cost; this pins the
///   cost's **boundary**: from the next
///       call on it is honest.) ★Which thread notices first is a race, so it is not measured★ (see the (a) assertion below).
///   (b) The prefix that went out before the failure is **exactly the first K bytes of the payload** — a child dying before its first read
///   or a writer failing before writing breaks this equality (blocks a vacuous pass); the old Test 2's non-repeating payload oracle, inherited.
/// ★Differs from the old Test 2 only in (a)★ — there **that call** was Err, here the **next call**.
#[test]
fn physical_pipe_write_failure_after_acceptance_surfaces_on_the_next_call() {
    // K = 64 KiB: reads and echoes each chunk then exits; afterwards the pipe breaks and the remaining write_all fails. The last read is
    //   capped at the remaining (K - total) so consumption · echo is exactly K.
    const K: usize = 64 * 1024;
    let child_script = format!(
        "\
$stdin=[Console]::OpenStandardInput();\
$stdout=[Console]::OpenStandardOutput();\
$buf=New-Object byte[] 4096;\
$total=0;\
while($total -lt {K}){{\
$cap=[Math]::Min($buf.Length,{K}-$total);\
$n=$stdin.Read($buf,0,$cap);\
if($n -le 0){{break}};\
$stdout.Write($buf,0,$n);$stdout.Flush();\
$total+=$n}};\
exit 0"
    );

    let (transport, _pid) = StdioTransport::open(
        &spec("powershell.exe", &["-NoProfile", "-Command", &child_script]),
        false, // Plain-text stdio — TerminalBytes direct (decoder=None)
        None,
    )
    .expect("open");

    let sink = CollectingSink::new();
    let core = Arc::new(OutputCore::new(
        Uuid::new_v4(),
        0,
        Arc::new(NoopStatusSink),
        TurnWiring::detached(),
    ));
    transport.start(core.clone());
    core.subscribe(Arc::new(sink.clone()));

    // 1 MiB — overwhelms K + the pipe buffer but is under the queue bound (2 MiB). ★Lowered from the old 8 MiB because of the bound; the
    //   measured property (the child eats only K and dies, the rest fails) is unchanged★ — over 10× the default pipe buffer, so unsent
    //   remainder surely remains.
    //   ★Non-repeating stream★: fixed-seed xorshift64 (seed=0x9E37_79B9_7F4A_7C15) — deterministic, and no K-window at any offset overlaps
    //   payload[..K], so (b) catches a prefix written at a wrong offset.
    const PAYLOAD_LEN: usize = 1024 * 1024;
    let payload: Vec<u8> = {
        let mut v = Vec::with_capacity(PAYLOAD_LEN);
        let mut state: u64 = 0x9E37_79B9_7F4A_7C15;
        while v.len() < PAYLOAD_LEN {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            v.extend_from_slice(&state.to_le_bytes());
        }
        v.truncate(PAYLOAD_LEN);
        v
    };
    let expected_prefix = payload[..K].to_vec();

    macro_rules! fail {
        ($($arg:tt)*) => {{
            transport.shutdown();
            panic!($($arg)*);
        }};
    }

    // ★self-check — no aliasing★: at every shift s, payload[s..s+K] != payload[..K]; the same window would blind the (b) oracle at that
    //   offset. Fast thanks to early-exit.
    let p0 = payload[0];
    for s in 1..=(payload.len() - K) {
        if payload[s] == p0 && payload[s..s + K] == expected_prefix[..] {
            fail!("no-aliasing self-check failed: shift s={s} — the seed/generator must be changed and regenerated");
        }
    }

    // ★This call is immediately `Ok` — that is the point of this round★ (the caller does not wait for the OS write).
    let accept_start = Instant::now();
    if let Err(e) = transport.send_input(InputEvent::Raw(payload)) {
        fail!("a payload below the cap must be accepted: {e:?}");
    }
    let accept_elapsed = accept_start.elapsed();
    if accept_elapsed >= Duration::from_secs(2) {
        fail!(
            "send_input took {accept_elapsed:?} — regression of the caller hanging on the OS write"
        );
    }

    // (b) The prefix was actually delivered physically — blocks a vacuous pass.
    let prefix_deadline = Instant::now() + Duration::from_secs(30);
    while sink.total_len() < K {
        if Instant::now() >= prefix_deadline {
            fail!(
                "the echo prefix did not reach K ({K}) within 30s (now {}) — prefix not delivered (suspected vacuous pass)",
                sink.total_len()
            );
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let echoed = sink.concat_ordered();
    if echoed.len() != K {
        fail!(
            "the echo total is not K ({K}) ({}) — the child did not consume/echo exactly K",
            echoed.len()
        );
    }
    if echoed != expected_prefix {
        fail!("the echo prefix does not match payload[..K] — what was physically delivered is not the front prefix (integrity violation)");
    }

    // (a) The child dies and the path closes → the **next** call is refused carrying the reason.
    //     ★Polling★: the close is another thread's work; wait until observed, and the deadline turns a swallowed-failure regression (never
    //   closes) into a failure.
    // ★★Which reason wins is a **race, and both are true** — so the reason string is not asserted★★: when the child dies, ⓐ the writer's
    //   blocked write freed with an error and ⓑ the pump seeing stdout EOF and closing with `WriterStop` start **together**; ⓑ won first
    //   in measurement (2026-09-17). `InputQueue::close`
    //   keeps the first reason, so the phrase is then "the stream ended". The property is **「a failure after accepting is not silently
    //   swallowed」**, so the assertion is 「refused」 + 「the reason is not the bound」 (the bound would pass for a different reason than measured).
    let close_deadline = Instant::now() + Duration::from_secs(30);
    loop {
        match transport.send_input(InputEvent::Raw(b"after\n".to_vec())) {
            Err(PtyError::WriteFailed(msg)) => {
                if msg.contains("cap") {
                    fail!("the refusal reason is the cap — it was not refused because the link closed: {msg}");
                }
                break;
            }
            Ok(()) => {
                if Instant::now() >= close_deadline {
                    fail!("the link did not close within 30s — the child's death was silently swallowed");
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(other) => fail!("must be WriteFailed: {other:?}"),
        }
    }

    transport.shutdown();
    core.join_pump(Duration::from_secs(10));
}
