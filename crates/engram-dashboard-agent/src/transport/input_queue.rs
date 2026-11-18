//! The **per-agent input queue** of console-family paths (PTY · stdio) — the seam that keeps `send_input` from **doing the OS write on the
//!   calling thread**.
//!
//! ★Why★: `send_input` used to hold the writer mutex through `write_all`+`flush` on the caller's thread; a child that does not read stdin
//!   hangs that write on pipe backpressure and **the caller with it** — the daemon's per-connection dispatch consumer, so
//!   **all other commands** of that connection stall. So it enqueues and returns at once, and a dedicated writer thread writes in FIFO.
//!
//! ★Precedent★: [`crate::backend::codex::transport`] solved the same problem the same way (bounded queue + dedicated `engram-codex-writer`
//!   + over-cap is `Err`). There an entry is a **complete user turn** and counts by entries; here an entry can be a keystroke or a whole
//!   pasted body, so entries cannot measure memory and the cap is in bytes ([`INPUT_QUEUE_MAX_BYTES`]).
//!
//! ★★The contract this queue changes — the most important sentence here★★: `Ok` **moved from 「the bytes went to the child」 to 「we accepted
//!   all of it with its order fixed」**. A write that fails after acceptance **cannot return to that caller** (same as the codex-side
//!   `send_input`'s 「known limitation」); it leaves ⓐ a `warn` log ⓑ an `Err` from the **next** `send_input` as the queue closes ⓒ usually
//!   the pump's terminal transition soon after (a broken pipe means EOF is near).
//!
//! A chunk can carry a 「call after it actually goes out」 ([`OnWritten`]) for sides that need the moment a chunk goes out although `Ok` no
//!   longer means that. Zero tauri imports.

use std::collections::VecDeque;
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

use engram_dashboard_base::panic::{contain, payload_text, uncontained};
use engram_dashboard_base::sync;

use crate::types::{AgentId, PtyError};

/// What the writer calls **after one chunk has actually gone out to the OS**.
///
/// - Called once on the writer thread after the write succeeds and the accounting is released, **holding none of this queue's locks** (so
///   putting into the same queue from inside does not hang).
/// - Not called (dropped) when: the queue did not accept it (`Err`) · the queue closed before it was taken out · the write failed.
/// - ★Called even if the queue closes after taking it out, as long as that write succeeds★ — it can overlap with the closer (`shutdown()`
///   · stream end). If it must not run after retiring, check the retiring marker inside
///   (`crate::transport::process_group::RetiringSignal`); the close does not block it.
/// - ★Write it not to panic★ — the writer runs it outside the containment marker (ADR-0292 decision 3), so in production the base panic
///   hook ends the process; the writer's inner catch (log `error`, keep writing, queue stays open) is a safety net only in test builds without
///   the hook.
/// - One writer thread also writes later chunks, so blocking inside delays later input; waiting inside for the queue to empty
///   ([`InputQueue::wait_drained`]) waits for itself.
pub type OnWritten = Box<dyn FnOnce() + Send>;

/// The cap (bytes) on input not yet gone out to the OS. ★Exceeding it returns `Err` — not a silent discard★ (ADR-0190; the discarding
///   branch was rejected there).
///
/// ★「Not yet gone out」 **includes the chunk the writer has taken out and is writing**★ ([`Inner::bytes`] has the accounting): counted as 0
///   at take-out, the declared ceiling would double — and the bitten span is exactly what this cap targets.
/// ★Why bytes★: an entry can be a keystroke or a whole pasted body, so 「32 entries」 can be 32 B or 32 MiB; the real measure is **the
///   memory one agent holds when the child does not read stdin**.
/// ★Value = symmetry with a number the repo already chose★: one agent's **output** sits in at most 2 MiB (replay ring of
///   [`crate::output_core`]); a larger input ceiling would leave input nearly unbounded against output cut at 2 MiB.
/// ★Which way it errs★: too small rejects a legitimate large paste (a person sees it and retries); too large lets every bitten agent
///   silently lock up that memory. A single body over 2 MiB (≈500k tokens) is not normal use.
pub const INPUT_QUEUE_MAX_BYTES: usize = 2 * 1024 * 1024;

/// The recheck period that makes a missed writer wake-up fall to **delay, not permanent silence**.
///
/// ★Not what carries correctness★ — [`InputQueue::push`] · [`InputQueue::close`] `notify` **under the lock**, leaving no lost-wake window;
///   this is a backstop if that discipline breaks (the codex writer's `SWEEP_INTERVAL` role, minus the sweeping).
const WAKE_BACKSTOP: Duration = Duration::from_millis(500);

struct Inner {
    pending: VecDeque<(Vec<u8>, Option<OnWritten>)>,
    /// Payload bytes **not yet gone out to the OS** = what `pending` holds **+ what the writer has taken out and is writing**.
    ///   [`InputQueue::push`] adds, [`InputQueue::mark_written`] subtracts (a running total).
    ///
    /// ★Subtract **not** in [`InputQueue::pop`]★: the taken chunk may be bitten by a blocking OS write — exactly the span this cap
    ///   targets; subtracting at pop counts it as 0 and doubles the real ceiling. So it counts 「received → gone out」, and
    /// [`InputQueue::close`] discards the waiting part and resets it to 0.
    bytes: usize,
    /// `None` = open. `Some(reason)` = closed — that reason is the `Err` text of later [`InputQueue::push`].
    closed: Option<String>,
    pushed_seq: u64,
    /// The number of chunks that have **actually gone out to the OS**; the writer raises it by one per successful write.
    ///
    /// ★The difference of the two counters is the distance between 「received」 and 「gone out」★ — without it the receipt reports acceptance
    ///   as delivery (the two cases ADR-0088 tells apart get flipped). Chunks go out in FIFO (**the k-th received is the k-th out**), so
    ///   two monotonic counters suffice.
    written_seq: u64,
}

pub(crate) struct InputQueue {
    inner: Mutex<Inner>,
    wake: Condvar,
}

impl InputQueue {
    pub(crate) fn new() -> Self {
        InputQueue {
            inner: Mutex::new(Inner {
                pending: VecDeque::new(),
                bytes: 0,
                closed: None,
                pushed_seq: 0,
                written_seq: 0,
            }),
            wake: Condvar::new(),
        }
    }

    /// ★Accept all or none★ — a partial acceptance is never reported as `Ok`.
    ///
    /// Only two `Err` causes, **both decided before return**: ① queue closed ② cap exceeded. A failure after acceptance shows in the next
    ///   call's `Err` (module header contract).
    pub(crate) fn push(&self, bytes: Vec<u8>) -> Result<(), PtyError> {
        self.push_with(bytes, None)
    }

    pub(crate) fn push_with(
        &self,
        bytes: Vec<u8>,
        on_written: Option<OnWritten>,
    ) -> Result<(), PtyError> {
        let mut inner = sync::lock(&self.inner);
        if let Some(reason) = &inner.closed {
            return Err(PtyError::WriteFailed(reason.clone()));
        }
        // ★A single payload alone over the cap is caught here too — intended★: letting it through would let one call take the very memory
        //   the cap measures without bound.
        let after = inner.bytes.saturating_add(bytes.len());
        if after > INPUT_QUEUE_MAX_BYTES {
            return Err(PtyError::WriteFailed(format!(
                "input queue cap ({INPUT_QUEUE_MAX_BYTES} bytes) exceeded — queued {} + requested {} — this input was not accepted (it was not discarded)",
                inner.bytes,
                bytes.len()
            )));
        }
        inner.bytes = after;
        inner.pending.push_back((bytes, on_written));
        inner.pushed_seq += 1;
        // ★Notify while holding the lock★ — this is why [`WAKE_BACKSTOP`] stays only a backstop.
        self.wake.notify_one();
        Ok(())
    }

    /// Take out the next chunk. `None` if the queue is closed — the writer ends its loop.
    ///
    /// ★Closing is decided **before** the waiting part — input not yet out at close is lost★: every close means 「this child no longer
    ///   reads」 (kill · stream end · write failure), so writing the rest would fail anyway (as the codex writer).
    /// ★Taking out **does not reduce** [`Inner::bytes`]★ — only [`mark_written`](Self::mark_written) releases that accounting (ground:
    ///   that field's doc).
    pub(crate) fn pop(&self) -> Option<(Vec<u8>, Option<OnWritten>)> {
        let mut inner = sync::lock(&self.inner);
        loop {
            if inner.closed.is_some() {
                return None;
            }
            if let Some(chunk) = inner.pending.pop_front() {
                return Some(chunk);
            }
            let (guard, _timed_out) = sync::wait_timeout(&self.wake, inner, WAKE_BACKSTOP);
            inner = guard;
        }
    }

    /// Close the queue (idempotent — **the first reason wins**). Wakes and ends the writer; later `push` gives `Err` with that reason.
    ///
    /// ★Only this queue's lock is taken★ — a writer hung on a blocking write writes after **releasing** it ([`drain`] writes after `pop`
    ///   returns), so this cannot hang behind that write; hence `shutdown()` may call it before kill (the reverse order is also safe, but
    ///   **the closer first**
    ///   feeds a dying child one line less).
    /// ★First reason wins★: the real cause comes first (write failure → termination after); overwriting would smear the diagnosis into
    ///   「the agent was terminated」.
    pub(crate) fn close(&self, reason: &str) {
        let abandoned = {
            let mut inner = sync::lock(&self.inner);
            if inner.closed.is_none() {
                inner.closed = Some(reason.to_string());
            }
            inner.bytes = 0;
            self.wake.notify_all();
            std::mem::take(&mut inner.pending)
        };
        // A discarded chunk's callback is dropped after releasing the lock, so a held value whose drop comes back to this queue does not hang.
        drop(abandoned);
    }

    /// One chunk **has actually gone out to the OS**. Only the writer calls it ([`drain`]).
    ///
    /// ★`len` = that chunk's bytes — the **only** place that releases the [`Inner::bytes`] accounting★; close may have reset it to 0
    ///   first, hence `saturating_sub`.
    fn mark_written(&self, len: usize) {
        let mut inner = sync::lock(&self.inner);
        inner.bytes = inner.bytes.saturating_sub(len);
        inner.written_seq += 1;
        self.wake.notify_all();
    }

    /// ★Wait until **everything accepted up to this call has gone out**★ — `Ok` = gone out.
    ///
    /// ★Why it exists — restores the receipt's meaning★: with the queue `send_input`'s `Ok` dropped to 「received」, and putting that on the
    ///   delivery receipt would **record an unsent body as 「delivery succeeded」** (ADR-0088's "transmission failure" vs "model ignored"
    ///   flipped).
    ///   Only the delivery verb passes this check. ★The keystroke path does not call it★ — waiting there would bring back the head-of-line
    ///   blocking this queue removes.
    /// ★Take the target inside the lock★ so 「up to what I put in」 is included; others' later puts are waited for too (over-waiting, never
    ///   short).
    /// ★Past the deadline, **the queue is not cancelled**★: 「confirmation failed」 ≠ 「did not go out」 — a late-out branch remains. Handling
    ///   that residue (a retry may duplicate) belongs to the caller; its call site writes it down.
    pub(crate) fn wait_drained(&self, timeout: Duration) -> Result<(), PtyError> {
        let deadline = Instant::now() + timeout;
        let mut inner = sync::lock(&self.inner);
        let target = inner.pushed_seq;
        loop {
            if inner.written_seq >= target {
                return Ok(());
            }
            // ★Close first★ — once closed the waiting part is discarded ([`pop`]) so the target is never reached; that reason is 「why it
            //   did not go out」, carried as is (a write failure's text arrives here).
            if let Some(reason) = &inner.closed {
                return Err(PtyError::WriteFailed(format!(
                    "write confirmation failed — the link closed: {reason}"
                )));
            }
            let now = Instant::now();
            if now >= deadline {
                return Err(PtyError::WriteFailed(format!(
                    "passed the write confirmation time limit ({timeout:?}) — {} chunks have not gone out yet (not cancelled)",
                    target - inner.written_seq
                )));
            }
            let (guard, _timed_out) = sync::wait_timeout(&self.wake, inner, deadline - now);
            inner = guard;
        }
    }

    #[cfg(test)]
    pub(crate) fn queued_bytes(&self) -> usize {
        self.inner.lock().unwrap_or_else(|p| p.into_inner()).bytes
    }
}

/// The **body** of the dedicated writer thread. Pulls in FIFO until the queue closes and hands to `write`.
///
/// ★`impl FnMut` abstracts the write target★: PTY's writer thread must **own** a `Box<dyn Write + Send>` (portable-pty's writer is
///   take-once, not `Sync`); stdio must **share** a `Mutex<Option<ChildStdin>>` for `shutdown()`'s `try_lock` discipline — no common type.
///   A closure lets
///   both paths share the loop with their own ownership, and the test bench drives it without a real process (ADR-0012).
/// ★A write failure closes the queue and **ends**★ — retries after a broken pipe all fail, and a running writer would only let input pile
///   up to the cap; the next `send_input` returns `Err` with the reason so the caller decides.
/// ★★A panic gets the same handling — what `contain` is for★★: the write runs after releasing the lock, so the queue lock is **not
///   poisoned**; uncaught, only the writer dies, the queue stays **open**, `send_input` keeps returning `Ok` and at the cap rejects with
///   the **wrong reason**
///   (「cap exceeded」) while nothing goes out. Same discipline as the `spawn` failure branch (`pty.rs` · `stdio.rs`): **no writer → close
///   the queue and reject honestly from then.** A panic in the chunk's callback ([`OnWritten`]) is outside this containment
///   (ADR-0292 decision 3).
/// ★`agent` is for log attribution★ — these warnings are the **only trace** of a 「write that failed after acceptance」
///   (`docs/reference/logging-conventions.md`).
pub(crate) fn drain(
    queue: &InputQueue,
    label: &str,
    agent: AgentId,
    mut write: impl FnMut(&[u8]) -> std::io::Result<()>,
) {
    // ADR-0292
    let outcome = contain(std::panic::AssertUnwindSafe(|| {
        while let Some((chunk, on_written)) = queue.pop() {
            match write(&chunk) {
                Ok(()) => {
                    // ★This success count is the only progress signal of [`InputQueue::wait_drained`]★ and the same line releases the
                    //   [`Inner::bytes`] accounting — 「gone out」 carries both.
                    queue.mark_written(chunk.len());
                    // ADR-0292 decision 3: the callback changes shared state (claude residue cleanup's door) and relies on abort for
                    //   protection (ADR-0262 K7) — it turns off the marker, so a panic inside is not contained. The inner catch keeps a
                    //   panic from reaching the outer guard and
                    //   closing the queue, but where the base panic hook (`install_hook`) is installed the hook ends the process first; it
                    //   catches only in unwind builds without the hook. ★Do not change the inner catch to `contain`★ — it would set the
                    //   turned-off marker back.
                    if let Some(on_written) = on_written {
                        uncontained(|| {
                            if let Err(payload) =
                                std::panic::catch_unwind(std::panic::AssertUnwindSafe(on_written))
                            {
                                // Level = error — even when isolated, a panic is an error (the level table of
                                //   `docs/reference/logging-conventions.md`).
                                tracing::error!(
                                    agent = %agent,
                                    "{label}: the callback after an input chunk went out panicked — the next chunks keep being written: {}",
                                    payload_text(&*payload)
                                );
                            }
                        });
                    }
                }
                Err(e) => {
                    // ★No way back to the caller that sent those bytes★ (it already left with `Ok`) — leave the reason and close the path
                    //   so **the next** call is honest (module header contract).
                    let reason = format!("{label} input write failed: {e}");
                    tracing::warn!(agent = %agent, "{reason}");
                    queue.close(&reason);
                    return;
                }
            }
        }
    }));

    if let Err(caught) = outcome {
        let reason = format!("{label} input writer panicked: {caught}");
        // Level = error: this agent's input path died **unrecoverably** and a person must look (`docs/reference/logging-conventions.md`
        //   level table).
        tracing::error!(agent = %agent, "{reason}");
        queue.close(&reason);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Instant;

    fn test_agent() -> AgentId {
        AgentId::nil()
    }

    #[test]
    fn drain_writes_chunks_in_fifo_order() {
        let queue = Arc::new(InputQueue::new());
        queue.push(b"first".to_vec()).expect("push 1");
        queue.push(b"second".to_vec()).expect("push 2");
        queue.push(b"third".to_vec()).expect("push 3");
        queue.close("test end");

        // Closing is decided before the waiting part (the `pop` contract), so draining after close yields nothing — this order test runs
        //   the writer **before** closing; the item below is that shape.
        let written: Arc<std::sync::Mutex<Vec<Vec<u8>>>> = Arc::new(std::sync::Mutex::new(vec![]));
        drain(&queue, "test", test_agent(), |b| {
            written.lock().unwrap().push(b.to_vec());
            Ok(())
        });
        assert!(
            written.lock().unwrap().is_empty(),
            "after closing, the remaining queued items are not written (contract)"
        );

        let queue = Arc::new(InputQueue::new());
        let written: Arc<std::sync::Mutex<Vec<Vec<u8>>>> = Arc::new(std::sync::Mutex::new(vec![]));
        let seen = written.clone();
        let writer_queue = queue.clone();
        let handle = std::thread::spawn(move || {
            drain(&writer_queue, "test", test_agent(), |b| {
                seen.lock().unwrap().push(b.to_vec());
                Ok(())
            });
        });

        queue.push(b"first".to_vec()).expect("push 1");
        queue.push(b"second".to_vec()).expect("push 2");
        queue.push(b"third".to_vec()).expect("push 3");

        let deadline = Instant::now() + Duration::from_secs(5);
        while written.lock().unwrap().len() < 3 {
            assert!(
                Instant::now() < deadline,
                "3 chunks did not go out within 5s"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        queue.close("test end");
        handle.join().expect("writer thread panic");

        let got = written.lock().unwrap().clone();
        assert_eq!(
            got,
            vec![b"first".to_vec(), b"second".to_vec(), b"third".to_vec()],
            "FIFO order and chunk boundaries preserved"
        );
    }

    /// Exceeding the cap is a **rejection**, not a loss — the queue contents stay as they are.
    #[test]
    fn push_over_the_bound_is_refused_not_dropped() {
        let queue = InputQueue::new();
        let half = INPUT_QUEUE_MAX_BYTES / 2;
        queue.push(vec![b'a'; half]).expect("half goes in");
        queue
            .push(vec![b'b'; half])
            .expect("the other half goes in too");
        assert_eq!(queue.queued_bytes(), INPUT_QUEUE_MAX_BYTES);

        let err = queue
            .push(vec![b'c'; 1])
            .expect_err("exceeding the cap by even 1 byte is refused");
        match err {
            PtyError::WriteFailed(msg) => {
                assert!(
                    msg.contains("cap"),
                    "the reason must say it is the cap: {msg}"
                );
                assert!(
                    msg.contains("not accepted"),
                    "must say it is a refusal, not a loss: {msg}"
                );
            }
            other => panic!("must be WriteFailed: {other:?}"),
        }
        assert_eq!(
            queue.queued_bytes(),
            INPUT_QUEUE_MAX_BYTES,
            "the refusal does not touch what was accepted before"
        );

        // Once one **actually goes out**, that much is accepted again — the cap is not a permanent death sentence. ★Taking out alone does
        //   not free room★ (accounting + regression net: `in_flight_bytes_still_count_against_the_bound` below).
        let (popped, _) = queue.pop().expect("pop one");
        assert_eq!(popped.len(), half);
        queue.mark_written(popped.len());
        queue
            .push(vec![b'c'; 1])
            .expect("accepts again once there is room");
    }

    /// ★A chunk taken out but not yet gone out is also **counted** against the cap★ — otherwise the ceiling doubles.
    ///
    /// ★The accident measured★: the writer takes one chunk and is bitten by a blocking OS write (the child not reading stdin — why this
    ///   queue exists); counted as 0 meanwhile, the producer can put in another cap's worth **again**. ★No deadline, no thread★ — `pop`
    ///   and `push` alone make that window, so it is deterministic.
    #[test]
    fn in_flight_bytes_still_count_against_the_bound() {
        let queue = InputQueue::new();
        queue
            .push(vec![b'a'; INPUT_QUEUE_MAX_BYTES])
            .expect("the full cap goes in");

        let (in_flight, _) = queue.pop().expect("the writer pops one");
        assert_eq!(in_flight.len(), INPUT_QUEUE_MAX_BYTES);
        assert_eq!(
            queue.queued_bytes(),
            INPUT_QUEUE_MAX_BYTES,
            "the accounting must not be released just because it was popped — it has not gone out to the OS yet"
        );

        let err = queue
            .push(vec![b'b'; 1])
            .expect_err("if the chunk being written were counted as 0, the cap would double here");
        match err {
            PtyError::WriteFailed(msg) => assert!(msg.contains("cap"), "{msg}"),
            other => panic!("must be WriteFailed: {other:?}"),
        }

        queue.mark_written(in_flight.len());
        assert_eq!(queue.queued_bytes(), 0);
        queue
            .push(vec![b'b'; 1])
            .expect("accepts after it went out");
    }

    /// ★Even if the writer panics, the queue **closes**★ — else `send_input` keeps returning `Ok` and at the cap rejects with the **wrong
    ///   reason** (「cap exceeded」), the real cause nowhere.
    ///
    /// ★Lock poison does not do it for us★: the write runs **after releasing** the lock, so the `Mutex` is fine even on panic; without
    ///   `contain` nothing triggers this transition.
    #[test]
    fn a_panicking_writer_closes_the_queue_with_the_panic_as_the_reason() {
        let queue = Arc::new(InputQueue::new());
        queue.push(b"doomed".to_vec()).expect("push");

        let writer_queue = queue.clone();
        let handle = std::thread::spawn(move || {
            drain(&writer_queue, "test", test_agent(), |_| {
                panic!("writer blew up");
            });
        });
        handle
            .join()
            .expect("drain must absorb the panic so the writer thread exits normally");

        match queue.push(b"after".to_vec()) {
            Err(PtyError::WriteFailed(msg)) => {
                assert!(
                    msg.contains("panic"),
                    "the reason must say it is a panic: {msg}"
                );
                assert!(
                    msg.contains("writer blew up"),
                    "original payload preserved: {msg}"
                );
                assert!(
                    !msg.contains("cap"),
                    "if it masquerades as a cap overflow, diagnosis goes in the opposite direction: {msg}"
                );
            }
            other => {
                panic!("a push after the panic must be the closed queue's WriteFailed: {other:?}")
            }
        }
    }

    /// The same handling even if a single payload alone exceeds the cap (it is not let through even into an empty queue).
    #[test]
    fn single_oversized_payload_is_refused_on_an_empty_queue() {
        let queue = InputQueue::new();
        assert!(queue.push(vec![b'x'; INPUT_QUEUE_MAX_BYTES + 1]).is_err());
        assert_eq!(queue.queued_bytes(), 0, "refused items do not pile up");
    }

    /// ★Writer termination contract★: `close` ends `drain` — whether waiting or writing.
    /// (That is the very verb that `shutdown()` · pump termination call.)
    #[test]
    fn close_terminates_the_drain_loop() {
        let queue = Arc::new(InputQueue::new());
        let finished = Arc::new(AtomicBool::new(false));

        let writer_queue = queue.clone();
        let writer_finished = finished.clone();
        let handle = std::thread::spawn(move || {
            drain(&writer_queue, "test", test_agent(), |_| Ok(()));
            writer_finished.store(true, Ordering::Release);
        });

        std::thread::sleep(Duration::from_millis(50));
        assert!(
            !finished.load(Ordering::Acquire),
            "does not finish before closing"
        );

        queue.close("test end");
        let deadline = Instant::now() + Duration::from_secs(5);
        while !handle.is_finished() {
            assert!(
                Instant::now() < deadline,
                "the writer did not finish within 5s after close — shutdown cannot collect the thread"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        handle.join().expect("writer thread panic");
        assert!(finished.load(Ordering::Acquire));
    }

    /// A `push` after closing returns `Err` **carrying the closing reason**.
    #[test]
    fn push_after_close_reports_the_close_reason() {
        let queue = InputQueue::new();
        queue.close("the agent was terminated");
        match queue.push(b"late".to_vec()) {
            Err(PtyError::WriteFailed(msg)) => {
                assert!(
                    msg.contains("the agent was terminated"),
                    "reason passed on: {msg}"
                )
            }
            other => panic!("a closed queue must be WriteFailed: {other:?}"),
        }
    }

    /// A write failure closes the queue and ends the writer — after that `send_input` is rejected with that reason.
    #[test]
    fn write_failure_closes_the_queue_with_its_reason() {
        let queue = InputQueue::new();
        queue.push(b"doomed".to_vec()).expect("push");
        drain(&queue, "test", test_agent(), |_| {
            Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "the pipe broke",
            ))
        });
        match queue.push(b"after".to_vec()) {
            Err(PtyError::WriteFailed(msg)) => {
                assert!(
                    msg.contains("input write failed"),
                    "reason passed on: {msg}"
                );
                assert!(
                    msg.contains("the pipe broke"),
                    "original error preserved: {msg}"
                );
            }
            other => panic!("a push after a write failure must be WriteFailed: {other:?}"),
        }
    }

    /// The first reason wins — a later close does not overwrite the diagnosis.
    #[test]
    fn first_close_reason_wins() {
        let queue = InputQueue::new();
        queue.close("the real cause");
        queue.close("an exit that came later");
        match queue.push(b"x".to_vec()) {
            Err(PtyError::WriteFailed(msg)) => assert!(msg.contains("the real cause"), "{msg}"),
            other => panic!("{other:?}"),
        }
    }

    // ── Callback after going out (`OnWritten`) ──

    type Written = Arc<Mutex<Vec<Vec<u8>>>>;

    fn spawn_recording_writer(
        queue: &Arc<InputQueue>,
        written: &Written,
    ) -> std::thread::JoinHandle<()> {
        let (seen, writer_queue) = (written.clone(), queue.clone());
        std::thread::spawn(move || {
            drain(&writer_queue, "test", test_agent(), |b| {
                seen.lock().unwrap().push(b.to_vec());
                Ok(())
            });
        })
    }

    fn wait_until(mut done: impl FnMut() -> bool, what: &str) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !done() {
            assert!(
                Instant::now() < deadline,
                "did not happen within 5 seconds: {what}"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// Once after a successful write — that chunk is already written and the accounting released. It tries the queue lock inside, so
    ///   calling it under the lock hangs and hits the deadline. Finishing the put before the writer makes the accounting (only the later
    ///   chunk's share) deterministic.
    #[test]
    fn on_written_runs_once_after_its_chunk_is_written_without_the_queue_lock() {
        let queue = Arc::new(InputQueue::new());
        let written: Written = Arc::new(Mutex::new(Vec::new()));
        let calls: Arc<Mutex<Vec<(Vec<Vec<u8>>, usize)>>> = Arc::new(Mutex::new(Vec::new()));
        let (log, seen, probe) = (calls.clone(), written.clone(), queue.clone());
        queue.push(b"plain".to_vec()).expect("push");
        queue
            .push_with(
                b"marked".to_vec(),
                Some(Box::new(move || {
                    let so_far = seen.lock().unwrap().clone();
                    log.lock().unwrap().push((so_far, probe.queued_bytes()));
                })),
            )
            .expect("push_with");
        queue.push(b"after".to_vec()).expect("push");

        let handle = spawn_recording_writer(&queue, &written);
        wait_until(|| written.lock().unwrap().len() == 3, "three chunks go out");
        queue.close("test end");
        handle.join().expect("writer thread panic");

        assert_eq!(
            *calls.lock().unwrap(),
            vec![(vec![b"plain".to_vec(), b"marked".to_vec()], b"after".len())],
            "the callback is called only once, after its chunk is written and the accounting is released"
        );
    }

    /// The callback of a chunk that did not go out is dropped without being called (it does not stay held) — closed before taking out ·
    ///   write failure · not accepted.
    #[test]
    fn on_written_is_dropped_uncalled_when_its_chunk_never_goes_out() {
        struct CountDrop(Arc<AtomicUsize>);
        impl Drop for CountDrop {
            fn drop(&mut self) {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }
        let calls = Arc::new(AtomicUsize::new(0));
        let drops = Arc::new(AtomicUsize::new(0));
        let marked = || -> Option<OnWritten> {
            let token = CountDrop(drops.clone());
            let calls = calls.clone();
            Some(Box::new(move || {
                let _held = &token;
                calls.fetch_add(1, Ordering::SeqCst);
            }))
        };

        let queue = InputQueue::new();
        queue.push_with(b"a".to_vec(), marked()).expect("push_with");
        queue.close("test end");
        assert_eq!(
            drops.load(Ordering::SeqCst),
            1,
            "the close is holding the callback"
        );
        let mut wrote = false;
        drain(&queue, "test", test_agent(), |_| {
            wrote = true;
            Ok(())
        });
        assert!(!wrote, "wrote after closing");

        let queue = InputQueue::new();
        queue.push_with(b"b".to_vec(), marked()).expect("push_with");
        drain(&queue, "test", test_agent(), |_| {
            Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "the pipe broke",
            ))
        });
        assert_eq!(
            drops.load(Ordering::SeqCst),
            2,
            "still holding after a write failure"
        );

        assert!(queue.push_with(b"c".to_vec(), marked()).is_err());
        let queue = InputQueue::new();
        assert!(queue
            .push_with(vec![b'x'; INPUT_QUEUE_MAX_BYTES + 1], marked())
            .is_err());
        assert_eq!(
            drops.load(Ordering::SeqCst),
            4,
            "an unaccepted callback remained"
        );

        assert_eq!(
            calls.load(Ordering::SeqCst),
            0,
            "the callback of a chunk that did not go out was called"
        );
    }

    /// Even if the queue closes between take-out and write, a successful write still calls the callback once — close does not block it
    ///   (closing inside the fake write makes the window deterministic); the later chunk is caught by the close and does not go out.
    #[test]
    fn on_written_still_runs_when_the_queue_closes_after_its_chunk_was_popped() {
        let calls = Arc::new(AtomicUsize::new(0));
        let counted = calls.clone();
        let queue = InputQueue::new();
        queue
            .push_with(
                b"popped".to_vec(),
                Some(Box::new(move || {
                    counted.fetch_add(1, Ordering::SeqCst);
                })),
            )
            .expect("push_with");
        queue.push(b"left".to_vec()).expect("push");

        let mut wrote = Vec::new();
        drain(&queue, "test", test_agent(), |b| {
            wrote.push(b.to_vec());
            queue.close("closed while writing");
            Ok(())
        });

        assert_eq!(
            wrote,
            vec![b"popped".to_vec()],
            "wrote a remaining chunk after closing"
        );
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "if the write succeeded, it must be called once even when closed after popping"
        );
    }

    /// Even putting into the same queue from inside the callback does not hang, and what was put in goes out afterwards.
    #[test]
    fn a_push_from_inside_on_written_does_not_deadlock() {
        let queue = Arc::new(InputQueue::new());
        let again = queue.clone();
        queue
            .push_with(
                b"first".to_vec(),
                Some(Box::new(move || {
                    again
                        .push(b"from-callback".to_vec())
                        .expect("push inside the callback");
                })),
            )
            .expect("push_with");

        let written: Written = Arc::new(Mutex::new(Vec::new()));
        let handle = spawn_recording_writer(&queue, &written);
        wait_until(
            || written.lock().unwrap().len() == 2,
            "a chunk pushed inside the callback goes out — if it were called holding the queue lock, it would hang here",
        );
        queue.close("test end");
        handle.join().expect("writer thread panic");
        assert_eq!(
            *written.lock().unwrap(),
            vec![b"first".to_vec(), b"from-callback".to_vec()]
        );
    }

    /// A panic in the callback stops there — the queue stays open and the later chunk and what was put in after both go out. ★A branch of
    ///   builds without the hook★: in production this panic is outside the marker and the hook ends it (ADR-0292).
    #[cfg(panic = "unwind")]
    #[test]
    fn a_panicking_on_written_does_not_stop_later_chunks() {
        let queue = Arc::new(InputQueue::new());
        queue
            .push_with(
                b"first".to_vec(),
                Some(Box::new(|| panic!("callback blew up"))),
            )
            .expect("push_with");
        queue.push(b"second".to_vec()).expect("push");

        let written: Written = Arc::new(Mutex::new(Vec::new()));
        let handle = spawn_recording_writer(&queue, &written);
        wait_until(
            || written.lock().unwrap().len() == 2,
            "the later chunk goes out",
        );
        queue
            .push(b"third".to_vec())
            .expect("the callback's panic closed the queue");
        wait_until(
            || written.lock().unwrap().len() == 3,
            "a chunk pushed after that goes out",
        );
        queue.close("test end");
        handle.join().expect("writer thread panic");
        assert_eq!(
            *written.lock().unwrap(),
            vec![b"first".to_vec(), b"second".to_vec(), b"third".to_vec()]
        );
    }

    /// The write runs inside the containment marker and the callback outside the marker — the callback changes shared state.
    // ADR-0292
    #[test]
    fn on_written_runs_outside_the_contain_mark_and_the_write_inside_it() {
        use engram_dashboard_base::panic::is_contained;

        let marks = Arc::new(Mutex::new(Vec::new()));
        let at_callback = marks.clone();
        let queue = InputQueue::new();
        queue
            .push_with(
                b"marked".to_vec(),
                Some(Box::new(move || {
                    at_callback
                        .lock()
                        .unwrap()
                        .push(("callback", is_contained()));
                })),
            )
            .expect("push_with");

        drain(&queue, "test", test_agent(), |_| {
            marks.lock().unwrap().push(("write", is_contained()));
            queue.close("only one chunk");
            Ok(())
        });

        assert_eq!(
            *marks.lock().unwrap(),
            vec![("write", true), ("callback", false)]
        );
        assert!(!is_contained(), "the marker remained after drain finished");
    }
}
