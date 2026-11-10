//! The session id **first-submission latch** — one per incarnation.
//!
//! Calls the commit port once at the first moment **both** of two inputs have happened. The order does not matter.
//! - [`SessionIdLatch::offer`] — this incarnation's session id became known (the backend · path receives it through [`SessionIdLatch::offer_sink`]).
//! - [`SessionIdLatch::note_submission`] — this incarnation's first user turn is **right before going out** to the peer.
//!   ★The mode where the path carries the turn (codex JSON) is an exception★: the session does not count it, and the moment the peer **first
//!   echoes** the user message the path calls it through [`SessionIdLatch::first_turn_sink`] — that mode's commit happens **after**
//!   the first turn (if it dies after sending and before the echo there is no persistence and the next activation is a new conversation — an accepted outcome).
//!
//! If the incarnation ends without a submission, the port is never called — 「저장된 id 가 있다 ⟺ 이어받을 대화가 있다」 (a stored id exists ⟺ there is a conversation to resume)
//! stands here.
//!
//! ## Invariants
//! - ★The port is called at most once per incarnation. Once the call has started (whether it ends in success · refusal · panic), it is not
//!   called again★. That is why `committed` is set **before** the port call. The port has no return so the result does not
//!   come back to the latch, and the same input gives the same answer so there is nothing to retry.
//! - offer is at most once per incarnation (the contract of the record port [`SessionIdSink`]). A second offer outside the contract is dropped whenever it
//!   comes — even before commit the first value wins.
//! - ★The fast-path marker (`settled`) is set **only after** the first commit has **returned** — do not set it as 「제출이 있었다」 (a submission happened)★:
//!   when two submitters are on different threads (mail flush · a connection's user input), the second would slip onto the fast
//!   path before the first's commit finishes and the first turn would go out before persistence. Every submission before settled takes the mutex and queues
//!   behind the commit in progress.
//! - The only external call made while holding the latch mutex is the commit port (logs are also written after releasing). Lock order =
//!   `래치 → 포트의 expected 칸 → profiles → 저장소 상태 칸` one-way — a caller that takes the latch calls it holding
//!   no other lock.
//! - ★There is no `catch_unwind`★: every place that calls the latch is outside the containment marker (the codex reader's first-turn port
//!   call turns the marker off), so in the production process a port panic is ended by the hook — it would be code that never gets reached even if we caught it (ADR-0292). If the port panics
//!   in a test build without the hook, the mutex is poisoned, and the following call takes back the poisoned guard as is
//!   (`sync::lock`), sees `committed`, and passes — it does not call the port again.
//!
//! It knows nothing of backend · transport · profile — uuid resolution and tracking 「이 화신이 시작 때 본 값」 (the value this incarnation saw at start) belong inside the commit port
//! (the assembly point).
// ADR-0226

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use engram_dashboard_base::sync;

use crate::backend::{FirstTurnSink, SessionIdSink};
use crate::types::AgentId;

pub(crate) struct SessionIdLatch {
    /// For log fields only.
    agent: AgentId,
    /// For log fields only — the incarnation guard is carried by the commit port.
    epoch: u32,
    commit: SessionIdSink,
    settled: AtomicBool,
    state: Mutex<LatchState>,
    /// The argument of the first [`Self::first_turn_sink`] call — whether that echo delivered the instructions. Later
    /// calls do not change it. `None` = the port has not been called.
    // ADR-0337
    first_turn_delivered: OnceLock<bool>,
}

#[derive(Default)]
struct LatchState {
    /// An id that was received but whose commit is held because there is no submission yet.
    pending: Option<String>,
    submitted: bool,
    committed: bool,
}

enum OfferOutcome {
    Held,
    Committed,
    Dropped,
}

impl SessionIdLatch {
    pub(crate) fn new(agent: AgentId, epoch: u32, commit: SessionIdSink) -> Arc<Self> {
        Arc::new(Self {
            agent,
            epoch,
            commit,
            settled: AtomicBool::new(false),
            state: Mutex::new(LatchState::default()),
            first_turn_delivered: OnceLock::new(),
        })
    }

    /// The record port handed to the backend · path — calling it is [`SessionIdLatch::offer`].
    pub(crate) fn offer_sink(self: &Arc<Self>) -> SessionIdSink {
        let latch = Arc::clone(self);
        Arc::new(move |raw: &str| latch.offer(raw))
    }

    /// The first-turn port handed to the path in the mode where the path carries the turn — calling it is [`SessionIdLatch::note_submission`].
    ///
    /// In that mode the session does not count submissions. Counting at the time of sending a first input that can be withdrawn with ✕ would persist an id with no conversation.
    ///   The path calls this port at the echo of the peer's first user message.
    /// The argument (whether the instructions were delivered — [`FirstTurnSink`]) does not change the commit; the first
    ///   call's value is kept ([`Self::first_turn_delivered`]).
    // ADR-0226 · ADR-0231 · ADR-0233: user decision — the id is persisted when it becomes real (the conversation has a turn = the first echo).
    pub(crate) fn first_turn_sink(self: &Arc<Self>) -> FirstTurnSink {
        let latch = Arc::clone(self);
        Arc::new(move |delivered: bool| {
            // `Err` = an earlier call already set it — the first value is the one kept.
            let _ = latch.first_turn_delivered.set(delivered);
            latch.note_submission();
        })
    }

    /// The first first-turn report — `None` = not reported yet.
    // TODO(TRD S25 §5-2 4 (b)): step 8 reads this to record the applied instructions hash only when it is `Some(true)`.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn first_turn_delivered(&self) -> Option<bool> {
        self.first_turn_delivered.get().copied()
    }

    pub(crate) fn offer(&self, raw: &str) {
        let mut state = self.lock_state();
        let outcome = if state.committed || state.pending.is_some() {
            OfferOutcome::Dropped
        } else if state.submitted {
            self.commit_locked(&mut state, raw);
            OfferOutcome::Committed
        } else {
            state.pending = Some(raw.to_owned());
            OfferOutcome::Held
        };
        drop(state);
        match outcome {
            // ★Do not delete this line★: after persistence the commit port's reflection log remains, but for a 0-turn incarnation nothing other than this line
            //   announces 「id 가 왔다」 (the id has arrived) — it is the event that field diagnosis and GUI measurement wait for.
            //   The id value is not put in.
            OfferOutcome::Held => tracing::info!(
                agent = %self.agent,
                epoch = self.epoch,
                "세션 id 를 받았다 — 첫 제출 전이라 영속을 보류한다"
            ),
            OfferOutcome::Dropped => tracing::debug!(
                agent = %self.agent,
                epoch = self.epoch,
                "이 화신의 둘째 세션 id 를 버린다 — 기록 포트 계약(화신당 한 번) 밖의 호출"
            ),
            OfferOutcome::Committed => {}
        }
    }

    /// Call it **right before** the first turn goes out. If the id has already been received, it commits on the spot, and returns after commit has returned —
    /// the caller must send the turn **after** this call returns for 「첫 턴 전에 영속」 (persist before the first turn) to hold.
    /// ★In the mode where the path carries the turn it is called **after** the first echo ([`Self::first_turn_sink`])★ — that mode has
    ///   no 「첫 턴 전에 영속」 (persist before the first turn).
    pub(crate) fn note_submission(&self) {
        if self.settled.load(Ordering::Acquire) {
            return;
        }
        let mut state = self.lock_state();
        if state.committed {
            // The earlier commit returned or ended in a panic — either way it is not called again.
            self.settled.store(true, Ordering::Release);
            return;
        }
        state.submitted = true;
        if let Some(raw) = state.pending.take() {
            self.commit_locked(&mut state, &raw);
        }
    }

    fn commit_locked(&self, state: &mut LatchState, raw: &str) {
        state.committed = true;
        (self.commit)(raw);
        self.settled.store(true, Ordering::Release);
    }

    fn lock_state(&self) -> MutexGuard<'_, LatchState> {
        sync::lock(&self.state)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use std::sync::{mpsc, Barrier};
    use std::thread;
    use std::time::Duration;

    /// The cap put on every wait of an item that uses threads — ★it prevents hanging, it is not a correctness threshold★. A correct
    /// latch finishes within milliseconds, so reaching this value happens only in the regression case where it never returns, and then it makes the test
    /// fail instead of hanging.
    const HANG_GUARD: Duration = Duration::from_secs(10);

    fn recording_port() -> (SessionIdSink, Arc<Mutex<Vec<String>>>) {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let seen = calls.clone();
        let port: SessionIdSink =
            Arc::new(move |raw: &str| seen.lock().unwrap().push(raw.to_owned()));
        (port, calls)
    }

    fn latch_with(port: SessionIdSink) -> Arc<SessionIdLatch> {
        SessionIdLatch::new(AgentId::new_v4(), 7, port)
    }

    // ── Single-thread order ──

    /// ① — the same rule holds going through the receiving port (`offer_sink`).
    #[test]
    fn an_offer_then_a_submission_commits_the_offered_id_once() {
        let (port, calls) = recording_port();
        let latch = latch_with(port);

        (latch.offer_sink())("sid-1");
        assert!(
            calls.lock().unwrap().is_empty(),
            "제출 전에는 부르지 않는다"
        );
        latch.note_submission();

        assert_eq!(calls.lock().unwrap().as_slice(), ["sid-1"]);
    }

    /// The first-turn port (`first_turn_sink`) is also the same submission input — the mode where the path carries the turn counts only with this (ADR-0226
    ///   amendment). Even if called many times, commit is once — and the first call's delivery report is the one kept
    ///   (ADR-0337), whatever later calls say. The commit does not depend on the report: an undelivered first echo commits too.
    #[test]
    fn the_first_turn_port_is_the_submission_input_and_commits_once() {
        for first in [false, true] {
            let (port, calls) = recording_port();
            let latch = latch_with(port);
            let first_turn = latch.first_turn_sink();

            (latch.offer_sink())("sid-1");
            assert!(
                calls.lock().unwrap().is_empty(),
                "첫 턴 전에는 부르지 않는다"
            );
            assert_eq!(latch.first_turn_delivered(), None);
            first_turn(first);
            first_turn(!first);

            assert_eq!(calls.lock().unwrap().as_slice(), ["sid-1"], "first={first}");
            assert_eq!(latch.first_turn_delivered(), Some(first));
        }
    }

    /// ② — if the submission comes first, the following offer commits on the spot.
    #[test]
    fn a_submission_then_an_offer_commits_at_the_offer() {
        let (port, calls) = recording_port();
        let latch = latch_with(port);

        latch.note_submission();
        assert!(calls.lock().unwrap().is_empty());
        latch.offer("sid-1");

        assert_eq!(calls.lock().unwrap().as_slice(), ["sid-1"]);
    }

    /// ③ — an incarnation that ended without a submission persists nothing.
    #[test]
    fn an_offer_without_a_submission_commits_nothing() {
        let (port, calls) = recording_port();
        let latch = latch_with(port);

        latch.offer("sid-1");

        assert!(calls.lock().unwrap().is_empty());
        assert!(!latch.settled.load(Ordering::Acquire));
    }

    /// ④
    #[test]
    fn repeated_submissions_commit_once() {
        let (port, calls) = recording_port();
        let latch = latch_with(port);

        latch.note_submission();
        latch.note_submission();
        latch.offer("sid-1");
        for _ in 0..3 {
            latch.note_submission();
        }

        assert_eq!(calls.lock().unwrap().as_slice(), ["sid-1"]);
    }

    /// ⑤ — the success branch of 「포트는 화신당 많아야 한 번」 (the port at most once per incarnation). A second offer after commit is outside the contract and is dropped.
    #[test]
    fn an_offer_after_a_commit_does_not_call_the_port() {
        let (port, calls) = recording_port();
        let latch = latch_with(port);

        latch.offer("sid-1");
        latch.note_submission();
        latch.offer("sid-2");
        latch.note_submission();

        assert_eq!(calls.lock().unwrap().as_slice(), ["sid-1"]);
    }

    /// ⑪ — a second offer that comes before commit is dropped too. The first value wins.
    #[test]
    fn a_second_offer_before_the_commit_is_dropped_and_the_first_wins() {
        let (port, calls) = recording_port();
        let latch = latch_with(port);

        latch.offer("sid-1");
        latch.offer("sid-2");
        latch.note_submission();

        assert_eq!(calls.lock().unwrap().as_slice(), ["sid-1"]);
    }

    /// ⑧ — while there is no offer (during handshake · before lock recovery) there is nothing to commit so the fast path does not stand.
    #[test]
    fn submissions_without_an_offer_never_settle() {
        let (port, calls) = recording_port();
        let latch = latch_with(port);

        for _ in 0..5 {
            latch.note_submission();
        }

        assert!(calls.lock().unwrap().is_empty());
        assert!(!latch.settled.load(Ordering::Acquire));
    }

    /// ⑩ — the refusal branch of 「포트는 화신당 많아야 한 번」 (the port at most once per incarnation). Even if the port returns writing nothing, to the latch it is indistinguishable from success
    ///   (there is no return), and the fast path stands and it is not called again.
    #[test]
    fn a_commit_that_records_nothing_still_settles_and_is_not_retried() {
        let calls = Arc::new(AtomicUsize::new(0));
        let counted = calls.clone();
        let refusing: SessionIdSink = Arc::new(move |_raw: &str| {
            counted.fetch_add(1, Ordering::SeqCst);
        });
        let latch = latch_with(refusing);

        latch.offer("sid-1");
        latch.note_submission();
        assert!(latch.settled.load(Ordering::Acquire));
        latch.note_submission();
        latch.offer("sid-2");
        latch.note_submission();

        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    /// ⑨ — the panic branch of 「포트는 화신당 많아야 한 번」 (the port at most once per incarnation). ★This is a branch of builds without the hook★ — in the production process the hook ends this
    ///   panic so this branch itself does not exist (ADR-0292). Even if the mutex is poisoned, the following call does not stall and
    ///   does not call the port again.
    #[cfg(panic = "unwind")]
    #[test]
    fn a_commit_that_panics_is_never_called_again() {
        let calls = Arc::new(AtomicUsize::new(0));
        let counted = calls.clone();
        let panicking: SessionIdSink = Arc::new(move |_raw: &str| {
            counted.fetch_add(1, Ordering::SeqCst);
            panic!("시험대: commit 포트가 패닉한다");
        });
        let latch = latch_with(panicking);

        latch.offer("sid-1");
        let unwound =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| latch.note_submission()));
        assert!(unwound.is_err(), "패닉은 그 호출을 타고 올라간다");
        assert!(latch.state.is_poisoned(), "전제: 뮤텍스에 독이 들었다");

        latch.note_submission();
        latch.offer("sid-2");
        latch.note_submission();

        assert_eq!(calls.load(Ordering::SeqCst), 1, "포트를 다시 불렀다");
        assert!(
            latch.settled.load(Ordering::Acquire),
            "독을 걷어 낸 뒤 빠른 길이 서야 한다"
        );
    }

    // ── Between threads ──

    /// ⑥ — even with one offer and two submitters running at the same time, exactly once.
    #[test]
    fn a_concurrent_offer_and_submissions_commit_exactly_once() {
        for round in 0..200 {
            let (port, calls) = recording_port();
            let latch = latch_with(port);
            let start = Arc::new(Barrier::new(3));
            let (done_tx, done_rx) = mpsc::channel::<()>();
            let spawn = |act: fn(&SessionIdLatch)| {
                let latch = latch.clone();
                let start = start.clone();
                let done_tx = done_tx.clone();
                thread::spawn(move || {
                    start.wait();
                    act(&latch);
                    done_tx.send(()).expect("시험 본체");
                })
            };
            // Rotate the launch order per run — the thread that hit the barrier last tends to run first, so with a fixed
            //   order the offer always stands at the same spot and rarely takes the 「offer 가 먼저」 (offer first) branch (measured).
            let mut acts: [fn(&SessionIdLatch); 3] = [
                |l| l.offer("sid-1"),
                |l| l.note_submission(),
                |l| l.note_submission(),
            ];
            acts.rotate_left(round % 3);
            let handles = acts.map(spawn);
            for _ in &handles {
                done_rx.recv_timeout(HANG_GUARD).unwrap_or_else(|e| {
                    panic!("{round}번째 판에서 시험 스레드가 돌아오지 않는다: {e}")
                });
            }
            for h in handles {
                h.join().expect("시험 스레드");
            }
            assert_eq!(
                calls.lock().unwrap().as_slice(),
                ["sid-1"],
                "{round}번째 판에서 commit 횟수가 1 이 아니다"
            );
        }
    }

    /// ⑦ ★Regression net for the fast-path race★ — while a commit is in progress, another thread's submission must not return **before
    ///   that commit returns**. If it returns, that thread's turn goes out before persistence.
    ///
    /// Hold the commit port with a channel. We watch for one span whether the second submitter does not return while held —
    ///   ★that span is only a window to catch the regression★: a correct latch passes regardless of the span length, and a regression that sets the
    ///   fast path by the submission marker returns within microseconds and gets caught.
    #[test]
    fn a_submission_racing_an_in_flight_commit_waits_for_it_to_return() {
        let (entered_tx, entered_rx) = mpsc::channel::<()>();
        let (release_tx, release_rx) = mpsc::channel::<()>();
        let release_rx = Mutex::new(release_rx);
        let events = Arc::new(Mutex::new(Vec::<&'static str>::new()));
        let port_events = events.clone();
        let held: SessionIdSink = Arc::new(move |_raw: &str| {
            entered_tx.send(()).expect("시험 본체가 기다린다");
            release_rx
                .lock()
                .unwrap()
                .recv_timeout(HANG_GUARD)
                .expect("시험 본체가 포트를 풀어 주지 않는다");
            port_events.lock().unwrap().push("commit 반환");
        });
        let latch = latch_with(held);
        latch.offer("sid-1");

        let (first_done_tx, first_done_rx) = mpsc::channel::<()>();
        let first = {
            let latch = latch.clone();
            thread::spawn(move || {
                latch.note_submission();
                first_done_tx.send(()).expect("시험 본체");
            })
        };
        entered_rx.recv_timeout(HANG_GUARD).expect(
            "첫 제출이 commit 포트에 들어가지 않는다 — offer 뒤의 제출이 commit 을 안 불렀다",
        );

        let (returned_tx, returned_rx) = mpsc::channel::<()>();
        let second = {
            let latch = latch.clone();
            let events = events.clone();
            thread::spawn(move || {
                latch.note_submission();
                events.lock().unwrap().push("둘째 제출 반환");
                returned_tx.send(()).expect("시험 본체");
            })
        };
        assert!(
            matches!(
                returned_rx.recv_timeout(Duration::from_millis(200)),
                Err(mpsc::RecvTimeoutError::Timeout)
            ),
            "commit 이 붙잡힌 동안 둘째 제출이 돌아왔다 — 첫 턴이 영속 전에 나간다"
        );

        release_tx.send(()).expect("포트");
        first_done_rx
            .recv_timeout(HANG_GUARD)
            .expect("포트를 풀었는데 첫 제출자가 돌아오지 않는다");
        returned_rx
            .recv_timeout(HANG_GUARD)
            .expect("commit 이 돌아왔는데 둘째 제출자가 돌아오지 않는다");
        first.join().expect("첫 제출자");
        second.join().expect("둘째 제출자");
        assert_eq!(
            events.lock().unwrap().as_slice(),
            ["commit 반환", "둘째 제출 반환"]
        );
    }
}
