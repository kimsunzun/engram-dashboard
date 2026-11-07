//! The **per-agent input queue** of console-family paths (PTY · stdio) — the seam that makes `send_input` **not do the OS write
//! on the calling thread**.
//!
//! ★Why it exists★: in the old shape `send_input` held the writer mutex throughout `write_all`+`flush` and did that OS write
//!   on the caller's thread. If the child does not read stdin, that write hangs on pipe backpressure and
//!   **the caller hangs with it.** The daemon's per-connection dispatch consumer is that caller, so when one agent bites,
//!   **all other commands** of that connection stall together. So it puts into the queue and returns immediately, and a dedicated writer thread
//!   pulls from it in FIFO and writes.
//!
//! ★The precedent already exists in this repository — it is not a newly invented shape★:
//!   [`crate::backend::codex::transport`] solved the same problem in the same shape (a bounded queue + a dedicated `engram-codex-writer`
//!   thread + exceeding the cap is `Err`). There one entry is a **complete user turn** so it counts by entries, but here one entry
//!   can be a single keystroke or an entire pasted body, so **the entry count cannot measure memory** — so
//!   the axis of this side's cap is bytes ([`INPUT_QUEUE_MAX_BYTES`]).
//!
//! ★★We state precisely the contract this queue changes — this is the most important sentence of this file★★:
//!   The meaning of `Ok` **has moved from 「바이트가 자식에게 갔다」 (the bytes went to the child) to 「전량을 순서까지 확정해 받았다」 (we accepted all of it with its order fixed)**.
//!   A write that fails after being accepted **has no way back to that caller** (the same place as the 「알려진
//!   한계」 (known limitation) of the codex-side `send_input`). What that failure leaves is ⓐ a `warn` log ⓑ an `Err` that returns from the **next** `send_input`
//!   on as the queue closes ⓒ and usually the pump's terminal transition that follows soon — a broken pipe means the reader will
//!   see EOF soon too, so the signal people see is ultimately that termination.
//!
//! Each chunk can carry a 「실제로 나간 뒤 부를 것」 (callback to call after it actually goes out) ([`OnWritten`]) — a door used by a side that, even after `Ok` no longer means 「나갔다」 (it went out),
//! needs to know the moment a chunk goes out.
//!
//! tauri import 0.

use std::collections::VecDeque;
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

use engram_dashboard_base::panic::{contain, payload_text, uncontained};
use engram_dashboard_base::sync;

use crate::types::{AgentId, PtyError};

/// What the writer calls **after one chunk has actually gone out to the OS**.
///
/// - When it is called = once on the writer thread after that chunk's write succeeds and the accounting is released, **holding none of this queue's locks**.
///   So even putting into the same queue again from inside does not hang.
/// - When it is not called = the queue did not accept it (`Err`) · the queue closed before it was taken out · the write failed. In all three it is dropped without being called.
/// - ★It is called even if the queue closes after taking it out, as long as that write succeeds★ — so this call can overlap with the side that closes the queue (the path's finishing
///   `shutdown()` · stream end). If it is something that must not be done after retiring, look directly at the retiring marker inside it
///   (`crate::transport::process_group::RetiringSignal`) — the queue's closing does not block it for you.
/// - ★Write it so it does not panic★ — the writer runs this call outside the containment marker (ADR-0292 decision 3), so in the production process
///   a panic inside it is caught by the base panic hook and ends the process. The inner catch the writer wraps around it (catch, leave it as `error`, and keep writing the next
///   chunk — the queue is not closed) is a safety net that stands only in test builds without that hook.
/// - One writer thread also writes the later chunks, so blocking long inside delays later input by that much. If it waits for the queue to empty inside
///   ([`InputQueue::wait_drained`]), it waits for itself.
pub type OnWritten = Box<dyn FnOnce() + Send>;

/// The cap (bytes) on input that has not yet gone out to the OS. ★If exceeded, that call returns `Err` — it is not discarded★
/// (ADR-0190's handling as is. The branch that silently discards was already rejected by that ADR).
///
/// ★「아직 나가지 못한」 (not yet gone out) **also includes the chunk the writer has taken out and is writing**★ — the source of record of the accounting is
///   [`Inner::bytes`]. If counted without that one chunk, the ceiling this constant declares actually becomes double
///   (it is counted as 0 the moment it is taken out so a producer can put in another cap's worth). The time it is bitten is exactly the span this cap
///   targets, so if that span is not counted, this value measures nothing.
///
/// ★Why the axis is bytes★: one entry here is not a complete unit — it can be a single keystroke or an entire pasted
///   body. Counted by entries, 「32 칸」 (32 entries) can be 32 bytes or 32MiB, so it **measures
///   nothing.** What this cap actually measures is **the memory one agent can hold when the child does not read stdin**,
///   and the honest unit on that axis is only bytes.
/// ★Ground for the value = symmetry with a number this repository already chose★: the ceiling at which one agent's **output** can sit in memory
///   is 2MiB (the replay ring of [`crate::output_core`]). Giving the input side a larger ceiling would become 「출력은
///   2MiB 로 자르면서 입력은 무제한에 가깝게 문다」 (output is cut at 2MiB while input is held nearly unbounded), so we give the same number.
/// ★Which way it errs if it is off★: too small and a legitimately large paste is rejected (a person knows immediately and
///   can retry), too large and every bitten agent silently locks up that much memory. A body sent at once exceeding 2MiB is not normal use of this path
///   (a 2MiB prompt ≈ 500k tokens), so we picked a spot where the cost of the smaller side
///   practically does not occur.
pub const INPUT_QUEUE_MAX_BYTES: usize = 2 * 1024 * 1024;

/// The recheck period that makes it fall to **delay instead of permanent silence** even if the writer misses a notification.
///
/// ★This value is not what carries correctness★ — [`InputQueue::push`] · [`InputQueue::close`] `notify` **while holding the lock**
///   so there is no window for a wake-up to be lost. This timeout is a backstop so that if that discipline is broken later, the agent's input is
///   only delayed by this much instead of dying forever (the same role as the codex writer's `SWEEP_INTERVAL`,
///   but this side has nothing to sweep so it is a pure backstop).
const WAKE_BACKSTOP: Duration = Duration::from_millis(500);

struct Inner {
    pending: VecDeque<(Vec<u8>, Option<OnWritten>)>,
    /// The sum of payload bytes **not yet gone out to the OS** = what `pending` holds **+ what the writer has taken out and is
    /// writing**. [`InputQueue::push`] adds and [`InputQueue::mark_written`] subtracts (the total is carried so
    /// as not to iterate each time).
    ///
    /// ★The point is that the place that subtracts is **not** [`InputQueue::pop`]★: the chunk taken out may from that moment be bitten by a blocking
    ///   OS write, and that time is exactly the span this cap targets. Subtracting at pop would make the bitten chunk
    ///   **count as 0** so a producer can put in another cap's worth — the real ceiling becomes double the declared value. So it counts throughout
    ///   「받았다 → 나갔다」 (received → gone out), and the closing ([`InputQueue::close`]) discards the remaining
    ///   waiting part, so it resets to 0 there.
    bytes: usize,
    /// `None` = open. `Some(사유)` = closed — that reason is the `Err` text of later [`InputQueue::push`].
    closed: Option<String>,
    /// The number of chunks **received** so far. This is where the target that [`InputQueue::wait_drained`] waits for is taken.
    pushed_seq: u64,
    /// The number of chunks that have **actually gone out to the OS** so far. The writer raises it by one per successful write.
    ///
    /// ★The difference of the two is the distance between 「받았다」 (received) and 「나갔다」 (gone out)★ — making that distance visible from outside is
    ///   the reason these two fields exist, and without it the receipt reports acceptance as delivery (the two
    ///   cases ADR-0088 wants to tell apart get flipped). Chunks go out in FIFO so **the k-th received is the k-th to go out** — so
    ///   two monotonic counters are enough and there is no need to attach a ticket to each chunk.
    written_seq: u64,
}

/// One agent's input queue. Producer = any thread that calls `send_input`, consumer = one dedicated writer.
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

    /// ★Accept all or none★ — a partial acceptance is not reported down as `Ok`.
    ///
    /// There are only two places that make it `Err` and **both are decided before this call returns**: ① the queue is closed
    /// ② the cap is exceeded. A failure after acceptance shows up not in this return value but in the `Err` of the next call
    /// (the contract paragraph of the module header).
    pub(crate) fn push(&self, bytes: Vec<u8>) -> Result<(), PtyError> {
        self.push_with(bytes, None)
    }

    /// Along with [`push`](Self::push), it carries what to call after that chunk goes out (contract = [`OnWritten`]). On `Err` it is dropped without being called.
    pub(crate) fn push_with(
        &self,
        bytes: Vec<u8>,
        on_written: Option<OnWritten>,
    ) -> Result<(), PtyError> {
        let mut inner = sync::lock(&self.inner);
        if let Some(reason) = &inner.closed {
            return Err(PtyError::WriteFailed(reason.clone()));
        }
        // ★A case where a single payload alone exceeds the cap is also caught here — intended★: if it were let through, one call would take without bound the very
        //   memory the cap was meant to measure.
        let after = inner.bytes.saturating_add(bytes.len());
        if after > INPUT_QUEUE_MAX_BYTES {
            return Err(PtyError::WriteFailed(format!(
                "입력 대기열 상한({INPUT_QUEUE_MAX_BYTES} bytes)을 넘는다 — 대기 {} + 요청 {} — 이 입력은 받지 않았다(버린 것이 아니다)",
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

    /// Take out the next chunk. `None` if the queue is closed — the writer sees it and ends the loop.
    ///
    /// ★Closing is decided **before** the remaining waiting part — so input that has not gone out yet at the moment of closing is lost★.
    ///   Every place that closes is 「이 자식은 더 이상 읽지 않는다」 (this child no longer reads) (kill · stream end · write failure), so even trying to
    ///   write the rest would all fail. The same as the codex writer's handling.
    /// ★Taking out **does not reduce** [`Inner::bytes`]★ — the chunk taken out has not gone out yet. The only thing that
    ///   releases that accounting is [`mark_written`](Self::mark_written), and the ground is that field's doc.
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

    /// Close the queue (idempotent — **the first reason wins**). Wakes and ends the writer, and later `push` gives `Err` with that reason.
    ///
    /// ★The only thing taken here is this queue's lock★ — a writer hung on a blocking write writes after **releasing** this lock
    ///   ([`drain`] writes after `pop` returns), so this call cannot hang behind that write. That is the ground for it being safe for `shutdown()` to
    ///   call this before kill, and reversing the order is also safe but **the side that closes first**
    ///   feeds a dying child one line less.
    /// ★Why the first reason wins★: the real cause always comes first (write failure → termination after). Overwriting
    ///   with a later reason would smear the diagnosis into 「에이전트를 종료했다」 (the agent was terminated).
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
        // The callback of a discarded chunk is dropped after releasing the lock — so that even if the drop of the value it holds comes back to this queue, it does not hang.
        drop(abandoned);
    }

    /// One chunk **has actually gone out to the OS**. Only the writer calls it ([`drain`]).
    ///
    /// ★`len` is that chunk's byte count★ — this is the **only** place that releases the [`Inner::bytes`] accounting.
    ///   Closing may have reset it to 0 first, so it subtracts with `saturating_sub` (in that case 0 is already right).
    fn mark_written(&self, len: usize) {
        let mut inner = sync::lock(&self.inner);
        inner.bytes = inner.bytes.saturating_sub(len);
        inner.written_seq += 1;
        // Wake the waiter ([`wait_drained`]). `pop` also sleeps on the same condition variable but that side is a loop so it is harmless.
        self.wake.notify_all();
    }

    /// ★Wait until **everything accepted up to the time of this call has gone out**★ — `Ok` = gone out.
    ///
    /// ★Why it exists — this is the place that restores the meaning of the receipt★: with the queue, `send_input`'s `Ok` dropped to
    ///   「받았다」 (received), and putting that as is on the delivery receipt would **record a body that has not gone out yet as 「배달 성공」
    ///   (delivery succeeded)** (the "transmission failure" and "model ignored" that ADR-0088 wants to tell apart get flipped). If only the delivery verb
    ///   passes this check, the meaning becomes 「나갔다」 (gone out) again. ★The keystroke path does not call this★ — waiting there
    ///   would bring back the very head-of-line blocking this queue meant to remove.
    /// ★Take the target inside the lock★: that way 「내가 넣은 것까지」 (up to what I put in) is surely included. If someone else puts in more in between,
    ///   we wait for that too (over-waiting) but it is not short — the side it errs toward is the safe one.
    /// ★Even past the deadline, **what is in the queue is not cancelled**★: so 「확인 실패」 (confirmation failed) and 「안 나갔다」 (did not go out) are not the same —
    ///   a branch that goes out late remains. The handling of that residue (a retry may become a duplicate delivery) is the job of the calling side, not this layer,
    ///   and the call site writes that fact down.
    pub(crate) fn wait_drained(&self, timeout: Duration) -> Result<(), PtyError> {
        let deadline = Instant::now() + timeout;
        let mut inner = sync::lock(&self.inner);
        let target = inner.pushed_seq;
        loop {
            if inner.written_seq >= target {
                return Ok(());
            }
            // ★Closing comes first★ — once closed, the remaining waiting part is discarded ([`pop`]) so the target is never reached.
            //   That reason is exactly 「왜 못 나갔나」 (why it did not go out), so we carry it as is (if it was a write failure, that text arrives here).
            if let Some(reason) = &inner.closed {
                return Err(PtyError::WriteFailed(format!(
                    "쓰기 확인 실패 — 통로가 닫혔다: {reason}"
                )));
            }
            let now = Instant::now();
            if now >= deadline {
                return Err(PtyError::WriteFailed(format!(
                    "쓰기 확인 시한({timeout:?})을 넘겼다 — {} 덩이가 아직 안 나갔다(취소하지는 않았다)",
                    target - inner.written_seq
                )));
            }
            let (guard, _timed_out) = sync::wait_timeout(&self.wake, inner, deadline - now);
            inner = guard;
        }
    }

    /// The waiting payload bytes — the window through which the test bench measures the cap decision.
    #[cfg(test)]
    pub(crate) fn queued_bytes(&self) -> usize {
        self.inner.lock().unwrap_or_else(|p| p.into_inner()).bytes
    }
}

/// The **body** of the dedicated writer thread. Pulls in FIFO until the queue closes and hands to `write`.
///
/// ★Why the write target is abstracted with `impl FnMut`★: for PTY the writer thread must **own** a `Box<dyn Write + Send>`
///   (portable-pty's writer can be taken only once and is not `Sync`), and for stdio it must **share** a `Mutex<Option<ChildStdin>>`
///   because of `shutdown()`'s `try_lock` discipline — the two write targets differ in shape so there is no
///   common type. Taking a closure lets the two paths use the same loop while each keeps its own ownership
///   discipline, and the test bench can drive this loop itself without a real process (the standalone verification of ADR-0012).
/// ★A write failure closes the queue and **ends**★ — retries after a broken pipe all fail, and if it keeps running, the input
///   piling up in the queue never goes out and only holds memory up to the cap. When closed, the next `send_input` returns `Err` carrying the reason,
///   so the caller can decide to retry or give up.
/// ★★Panic gets the same handling — this is what `contain` is for★★: if it panics during a write, the queue lock
///   is **not poisoned** (the write is done after releasing the lock). So if this branch is not caught, only the writer dies and the queue stays
///   **open**, and later `send_input` keeps returning `Ok` and from the moment it reaches the cap it rejects with the **wrong
///   reason** (「상한 초과」 (cap exceeded)) — the real cause is nowhere in that log. Meanwhile not a single byte
///   goes out. It is the same place as the discipline the `spawn` failure branch (`pty.rs` · `stdio.rs`) already chose: **if there is no
///   writer, close the queue and reject honestly from that moment.** A panic in the chunk's callback ([`OnWritten`]) is outside this containment
///   (that type's doc · ADR-0292 decision 3).
/// ★`agent` is for log attribution★ — these warnings are the **only trace** of a 「받아 둔 뒤 실패한 쓰기」 (write that failed after being accepted), so with
///   several agents, without whose it is, it is not a trace (the format convention of `docs/reference/logging-conventions.md`).
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
                    // ★This one line that counts successes is the only progress signal of [`InputQueue::wait_drained`]★.
                    //   The same line also releases the [`Inner::bytes`] accounting — 「나갔다」 (gone out) carries both meanings together.
                    queue.mark_written(chunk.len());
                    // ADR-0292 decision 3: the callback changes shared state (the door of claude residue cleanup) and relies on abort for its protection
                    //   (ADR-0262 K7) — it turns off the marker, and a panic inside is not contained. The inner catch is the place that keeps a panic from reaching the outer guard and
                    //   closing the queue, but in a process with the base panic hook (`install_hook`) installed, the hook ends the process first so
                    //   it is never reached (it catches only in unwind builds without that hook). ★Do not change the inner catch to `contain`★
                    //   — it would set the turned-off marker back.
                    if let Some(on_written) = on_written {
                        uncontained(|| {
                            if let Err(payload) =
                                std::panic::catch_unwind(std::panic::AssertUnwindSafe(on_written))
                            {
                                // Level = error — even when isolated, a panic is an error (the level table of `docs/reference/logging-conventions.md`).
                                tracing::error!(
                                    agent = %agent,
                                    "{label} 입력 덩이가 나간 뒤 부를 것이 panic 했다 — 다음 덩이는 계속 쓴다: {}",
                                    payload_text(&*payload)
                                );
                            }
                        });
                    }
                }
                Err(e) => {
                    // ★This failure has no way back to the caller that sent those bytes★ — that call already left
                    //   having received `Ok`. All that can be done here is to leave the reason and close the path so that **from the next** call on
                    //   it is honest (the contract paragraph of the module header).
                    let reason = format!("{label} 입력 쓰기 실패: {e}");
                    tracing::warn!(agent = %agent, "{reason}");
                    queue.close(&reason);
                    return;
                }
            }
        }
    }));

    if let Err(caught) = outcome {
        let reason = format!("{label} 입력 라이터가 panic 했다: {caught}");
        // Level = error. This agent's input path died **unrecoverably** and a person must look
        //   (the level table of `docs/reference/logging-conventions.md` — a panic is error).
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

    /// The test bench's agent marker — this layer does not use the value and only carries it as a log field.
    fn test_agent() -> AgentId {
        AgentId::nil()
    }

    /// FIFO — it comes out in the order put in, chunk boundaries unchanged.
    #[test]
    fn drain_writes_chunks_in_fifo_order() {
        let queue = Arc::new(InputQueue::new());
        queue.push(b"first".to_vec()).expect("push 1");
        queue.push(b"second".to_vec()).expect("push 2");
        queue.push(b"third".to_vec()).expect("push 3");
        queue.close("시험 종료");

        // Closing is decided before the waiting part (the `pop` contract above), so if you drain after closing nothing comes out —
        // so this item that measures order runs the writer **before** closing. The item below is that shape.
        let written: Arc<std::sync::Mutex<Vec<Vec<u8>>>> = Arc::new(std::sync::Mutex::new(vec![]));
        drain(&queue, "시험", test_agent(), |b| {
            written.lock().unwrap().push(b.to_vec());
            Ok(())
        });
        assert!(
            written.lock().unwrap().is_empty(),
            "닫힌 뒤에는 남은 대기분을 쓰지 않는다(계약)"
        );

        // The real item — start the writer first and close after writing.
        let queue = Arc::new(InputQueue::new());
        let written: Arc<std::sync::Mutex<Vec<Vec<u8>>>> = Arc::new(std::sync::Mutex::new(vec![]));
        let seen = written.clone();
        let writer_queue = queue.clone();
        let handle = std::thread::spawn(move || {
            drain(&writer_queue, "시험", test_agent(), |b| {
                seen.lock().unwrap().push(b.to_vec());
                Ok(())
            });
        });

        queue.push(b"first".to_vec()).expect("push 1");
        queue.push(b"second".to_vec()).expect("push 2");
        queue.push(b"third".to_vec()).expect("push 3");

        let deadline = Instant::now() + Duration::from_secs(5);
        while written.lock().unwrap().len() < 3 {
            assert!(Instant::now() < deadline, "5s 안에 3 덩이가 안 나갔다");
            std::thread::sleep(Duration::from_millis(5));
        }
        queue.close("시험 종료");
        handle.join().expect("라이터 스레드 panic");

        let got = written.lock().unwrap().clone();
        assert_eq!(
            got,
            vec![b"first".to_vec(), b"second".to_vec(), b"third".to_vec()],
            "FIFO 순서·덩이 경계 보존"
        );
    }

    /// Exceeding the cap is a **rejection**, not a loss — the queue contents stay as they are.
    #[test]
    fn push_over_the_bound_is_refused_not_dropped() {
        let queue = InputQueue::new();
        let half = INPUT_QUEUE_MAX_BYTES / 2;
        queue.push(vec![b'a'; half]).expect("절반은 들어간다");
        queue
            .push(vec![b'b'; half])
            .expect("나머지 절반도 들어간다");
        assert_eq!(queue.queued_bytes(), INPUT_QUEUE_MAX_BYTES);

        let err = queue
            .push(vec![b'c'; 1])
            .expect_err("상한을 1 바이트라도 넘으면 거절");
        match err {
            PtyError::WriteFailed(msg) => {
                assert!(msg.contains("상한"), "사유가 상한임을 말해야: {msg}");
                assert!(
                    msg.contains("받지 않았다"),
                    "거절이지 유실이 아님을 말해야: {msg}"
                );
            }
            other => panic!("WriteFailed 여야: {other:?}"),
        }
        assert_eq!(
            queue.queued_bytes(),
            INPUT_QUEUE_MAX_BYTES,
            "거절이 앞서 받아 둔 것을 건드리지 않는다"
        );

        // Once one **actually goes out**, that much is accepted again — the cap is not a permanent death sentence.
        // ★Taking out alone does not free room★ — the source of record for that accounting and its regression net is
        //   `in_flight_bytes_still_count_against_the_bound` below.
        let (popped, _) = queue.pop().expect("하나 꺼내기");
        assert_eq!(popped.len(), half);
        queue.mark_written(popped.len());
        queue.push(vec![b'c'; 1]).expect("자리가 나면 다시 받는다");
    }

    /// ★A chunk that was taken out but has not gone out yet is also **counted** against the cap★ — if not counted, the ceiling becomes double the declared value.
    ///
    /// ★The real accident this measures★: the writer takes out one chunk and gets bitten by a blocking OS write (the situation where the child does not
    ///   read stdin is the reason this queue exists). If that chunk is counted as 0 meanwhile, the producer can put in another cap's worth **again**,
    ///   and the memory one agent holds becomes double `INPUT_QUEUE_MAX_BYTES`.
    /// ★No deadline, no thread★ — that window can be made with `pop` and `push` alone, so it is deterministic.
    #[test]
    fn in_flight_bytes_still_count_against_the_bound() {
        let queue = InputQueue::new();
        queue
            .push(vec![b'a'; INPUT_QUEUE_MAX_BYTES])
            .expect("상한 전량은 들어간다");

        let (in_flight, _) = queue.pop().expect("라이터가 하나 꺼낸다");
        assert_eq!(in_flight.len(), INPUT_QUEUE_MAX_BYTES);
        assert_eq!(
            queue.queued_bytes(),
            INPUT_QUEUE_MAX_BYTES,
            "꺼냈다고 회계가 풀리면 안 된다 — 아직 OS 로 나가지 않았다"
        );

        let err = queue
            .push(vec![b'b'; 1])
            .expect_err("쓰는 중인 덩이를 0 으로 세면 여기서 상한이 두 배가 된다");
        match err {
            PtyError::WriteFailed(msg) => assert!(msg.contains("상한"), "{msg}"),
            other => panic!("WriteFailed 여야: {other:?}"),
        }

        // Room is freed only after it has actually gone out.
        queue.mark_written(in_flight.len());
        assert_eq!(queue.queued_bytes(), 0);
        queue.push(vec![b'b'; 1]).expect("나간 뒤엔 받는다");
    }

    /// ★Even if the writer panics, the queue **closes**★ — if not closed, later `send_input` keeps returning `Ok` and from the moment it
    /// reaches the cap rejects with the **wrong reason** (「상한 초과」 (cap exceeded)). The real cause remains nowhere.
    ///
    /// ★Lock poison does not do it for us★: the write is done **after releasing** the queue lock (after `pop` returns), so even on panic
    ///   the `Mutex` is fine — so without `contain` there is no place for this transition to happen.
    #[test]
    fn a_panicking_writer_closes_the_queue_with_the_panic_as_the_reason() {
        let queue = Arc::new(InputQueue::new());
        queue.push(b"doomed".to_vec()).expect("push");

        let writer_queue = queue.clone();
        let handle = std::thread::spawn(move || {
            drain(&writer_queue, "시험", test_agent(), |_| {
                panic!("라이터 폭발");
            });
        });
        handle
            .join()
            .expect("drain 이 panic 을 흡수해 라이터 스레드가 정상 종료해야");

        match queue.push(b"after".to_vec()) {
            Err(PtyError::WriteFailed(msg)) => {
                assert!(msg.contains("panic"), "사유가 panic 임을 말해야: {msg}");
                assert!(msg.contains("라이터 폭발"), "원 payload 보존: {msg}");
                assert!(
                    !msg.contains("상한"),
                    "상한 초과로 둔갑하면 진단이 반대 방향으로 간다: {msg}"
                );
            }
            other => panic!("panic 뒤 push 는 닫힌 큐의 WriteFailed 여야: {other:?}"),
        }
    }

    /// The same handling even if a single payload alone exceeds the cap (it is not let through even into an empty queue).
    #[test]
    fn single_oversized_payload_is_refused_on_an_empty_queue() {
        let queue = InputQueue::new();
        assert!(queue.push(vec![b'x'; INPUT_QUEUE_MAX_BYTES + 1]).is_err());
        assert_eq!(queue.queued_bytes(), 0, "거절된 것은 쌓이지 않는다");
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
            drain(&writer_queue, "시험", test_agent(), |_| Ok(()));
            writer_finished.store(true, Ordering::Release);
        });

        // Yield briefly so the writer surely enters its wait.
        std::thread::sleep(Duration::from_millis(50));
        assert!(!finished.load(Ordering::Acquire), "닫기 전엔 안 끝난다");

        queue.close("시험 종료");
        let deadline = Instant::now() + Duration::from_secs(5);
        while !handle.is_finished() {
            assert!(
                Instant::now() < deadline,
                "close 후 5s 안에 라이터가 안 끝났다 — shutdown 이 스레드를 못 거둔다"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        handle.join().expect("라이터 스레드 panic");
        assert!(finished.load(Ordering::Acquire));
    }

    /// A `push` after closing returns `Err` **carrying the closing reason**.
    #[test]
    fn push_after_close_reports_the_close_reason() {
        let queue = InputQueue::new();
        queue.close("에이전트를 종료했다");
        match queue.push(b"late".to_vec()) {
            Err(PtyError::WriteFailed(msg)) => {
                assert!(msg.contains("에이전트를 종료했다"), "사유 전달: {msg}")
            }
            other => panic!("닫힌 큐는 WriteFailed 여야: {other:?}"),
        }
    }

    /// A write failure closes the queue and ends the writer — after that `send_input` is rejected with that reason.
    #[test]
    fn write_failure_closes_the_queue_with_its_reason() {
        let queue = InputQueue::new();
        queue.push(b"doomed".to_vec()).expect("push");
        drain(&queue, "시험", test_agent(), |_| {
            Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "파이프가 끊겼다",
            ))
        });
        match queue.push(b"after".to_vec()) {
            Err(PtyError::WriteFailed(msg)) => {
                assert!(msg.contains("입력 쓰기 실패"), "사유 전달: {msg}");
                assert!(msg.contains("파이프가 끊겼다"), "원 오류 보존: {msg}");
            }
            other => panic!("쓰기 실패 뒤 push 는 WriteFailed 여야: {other:?}"),
        }
    }

    /// The first reason wins — a later close does not overwrite the diagnosis.
    #[test]
    fn first_close_reason_wins() {
        let queue = InputQueue::new();
        queue.close("진짜 원인");
        queue.close("나중에 온 종료");
        match queue.push(b"x".to_vec()) {
            Err(PtyError::WriteFailed(msg)) => assert!(msg.contains("진짜 원인"), "{msg}"),
            other => panic!("{other:?}"),
        }
    }

    // ── Callback after going out (`OnWritten`) ──

    type Written = Arc<Mutex<Vec<Vec<u8>>>>;

    /// A writer thread that collects the written chunks into `written` in order.
    fn spawn_recording_writer(
        queue: &Arc<InputQueue>,
        written: &Written,
    ) -> std::thread::JoinHandle<()> {
        let (seen, writer_queue) = (written.clone(), queue.clone());
        std::thread::spawn(move || {
            drain(&writer_queue, "시험", test_agent(), |b| {
                seen.lock().unwrap().push(b.to_vec());
                Ok(())
            });
        })
    }

    fn wait_until(mut done: impl FnMut() -> bool, what: &str) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !done() {
            assert!(Instant::now() < deadline, "5 초 안에 안 됐다: {what}");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// Once after a successful write — by then that chunk has already been written and the accounting released. It tries taking the queue lock inside so calling it holding the lock
    /// hangs and hits the deadline. Finishing the put before the writer makes the accounting at that time (only the later chunk's share remains) deterministic.
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
        wait_until(|| written.lock().unwrap().len() == 3, "세 덩이가 나가기");
        queue.close("시험 종료");
        handle.join().expect("라이터 스레드 panic");

        assert_eq!(
            *calls.lock().unwrap(),
            vec![(vec![b"plain".to_vec(), b"marked".to_vec()], b"after".len())],
            "부를 것은 제 덩이가 적히고 회계가 풀린 뒤 한 번만 불린다"
        );
    }

    /// The callback of a chunk that did not go out is dropped without being called (it does not stay held) — closed before taking out · write failure · not accepted.
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
        queue.close("시험 종료");
        assert_eq!(
            drops.load(Ordering::SeqCst),
            1,
            "닫힘이 부를 것을 쥐고 있다"
        );
        let mut wrote = false;
        drain(&queue, "시험", test_agent(), |_| {
            wrote = true;
            Ok(())
        });
        assert!(!wrote, "닫힌 뒤에 썼다");

        let queue = InputQueue::new();
        queue.push_with(b"b".to_vec(), marked()).expect("push_with");
        drain(&queue, "시험", test_agent(), |_| {
            Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "파이프가 끊겼다",
            ))
        });
        assert_eq!(
            drops.load(Ordering::SeqCst),
            2,
            "쓰기 실패 뒤에도 쥐고 있다"
        );

        assert!(queue.push_with(b"c".to_vec(), marked()).is_err());
        let queue = InputQueue::new();
        assert!(queue
            .push_with(vec![b'x'; INPUT_QUEUE_MAX_BYTES + 1], marked())
            .is_err());
        assert_eq!(
            drops.load(Ordering::SeqCst),
            4,
            "받지 않은 부를 것이 남았다"
        );

        assert_eq!(
            calls.load(Ordering::SeqCst),
            0,
            "나가지 못한 덩이의 부를 것이 불렸다"
        );
    }

    /// Even if the queue closes between taking out and writing, if that write succeeds the callback is called once — closing does not block it. Closing inside the fake
    /// write makes that window deterministic. The later chunk is caught by the closing and does not go out.
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
        drain(&queue, "시험", test_agent(), |b| {
            wrote.push(b.to_vec());
            queue.close("쓰는 사이 닫힘");
            Ok(())
        });

        assert_eq!(wrote, vec![b"popped".to_vec()], "닫힌 뒤 남은 덩이를 썼다");
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "꺼낸 뒤 닫혀도 쓰기가 성공했으면 한 번 불려야 한다"
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
                        .expect("부를 것 안의 push");
                })),
            )
            .expect("push_with");

        let written: Written = Arc::new(Mutex::new(Vec::new()));
        let handle = spawn_recording_writer(&queue, &written);
        wait_until(
            || written.lock().unwrap().len() == 2,
            "부를 것 안에서 넣은 덩이가 나가기 — 큐 락을 쥔 채 불렀으면 여기서 매달린다",
        );
        queue.close("시험 종료");
        handle.join().expect("라이터 스레드 panic");
        assert_eq!(
            *written.lock().unwrap(),
            vec![b"first".to_vec(), b"from-callback".to_vec()]
        );
    }

    /// A panic in the callback stops at that spot — the queue stays open and the later chunk and what was put in after it both go out. ★This is a branch of builds without the hook★
    /// — in the production process this panic is outside the marker and the hook ends it, so this branch itself does not exist (ADR-0292).
    #[cfg(panic = "unwind")]
    #[test]
    fn a_panicking_on_written_does_not_stop_later_chunks() {
        let queue = Arc::new(InputQueue::new());
        queue
            .push_with(b"first".to_vec(), Some(Box::new(|| panic!("부를 것 폭발"))))
            .expect("push_with");
        queue.push(b"second".to_vec()).expect("push");

        let written: Written = Arc::new(Mutex::new(Vec::new()));
        let handle = spawn_recording_writer(&queue, &written);
        wait_until(|| written.lock().unwrap().len() == 2, "뒤 덩이가 나가기");
        queue
            .push(b"third".to_vec())
            .expect("부를 것의 panic 이 큐를 닫았다");
        wait_until(
            || written.lock().unwrap().len() == 3,
            "그 뒤에 넣은 덩이가 나가기",
        );
        queue.close("시험 종료");
        handle.join().expect("라이터 스레드 panic");
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
                        .push(("부를 것", is_contained()));
                })),
            )
            .expect("push_with");

        drain(&queue, "시험", test_agent(), |_| {
            marks.lock().unwrap().push(("쓰기", is_contained()));
            queue.close("한 덩이만");
            Ok(())
        });

        assert_eq!(
            *marks.lock().unwrap(),
            vec![("쓰기", true), ("부를 것", false)]
        );
        assert!(!is_contained(), "drain 이 끝난 뒤 표시가 남았다");
    }
}
