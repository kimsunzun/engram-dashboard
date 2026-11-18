//! The session id **first-submission latch** — one per incarnation.
//!
//! Calls the commit port once at the first moment **both** inputs have happened (order irrelevant).
//! - [`SessionIdLatch::offer`] — this incarnation's session id became known (backend · path hand it over via [`SessionIdLatch::offer_sink`]).
//! - [`SessionIdLatch::note_submission`] — this incarnation's first user turn is **right before going out**.
//!   ★Exception: the mode where the path carries the turn (codex JSON)★ — the session does not count it; the path calls
//!   [`SessionIdLatch::first_turn_sink`] when the peer **first echoes** the user message, so that mode commits **after** the first turn (a
//!   death between send and echo persists nothing and the next activation is a new conversation — accepted).
//!
//! If the incarnation ends without a submission the port is never called — 「a stored id exists ⟺ there is a conversation to resume」.
//!
//! ## Invariants
//! - ★The port is called at most once per incarnation; once started (success · refusal · panic) never again★ — so `committed` is set
//!   **before** the port call. The port has no return and the same input gives the same answer, so nothing retries.
//! - offer is at most once per incarnation ([`SessionIdSink`] contract); a second offer is dropped whenever it comes — the first value wins.
//! - ★`settled` (fast-path marker) is set **only after** the first commit has **returned** — not as 「a submission happened」★: with two
//!   submitters on different threads (mail flush · a connection's user input) the second would take the fast path before the first commit
//!   finishes and its turn would go out before persistence. Every submission before settled takes the mutex and queues behind the commit.
//! - The only external call under the latch mutex is the commit port (logs after release). Lock order = `latch → the port's expected field →
//!   profiles → store-state field`; a caller taking the latch holds no other lock.
//! - ★No `catch_unwind`★: every caller is outside the containment marker (the codex reader's first-turn call turns it off), so in
//!   production the hook ends a port panic and a catch would be unreachable (ADR-0292). Without the hook the mutex is poisoned and the
//!   next call takes the poisoned guard (`sync::lock`), sees `committed`, and passes without calling the port.
//!
//! It knows nothing of backend · transport · profile — uuid resolution and tracking 「the value this incarnation saw at start」 belong in
//! the commit port (the assembly point).
// ADR-0226

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use engram_dashboard_base::sync;

use crate::backend::{FirstTurnSink, SessionIdSink};
use crate::types::AgentId;

pub(crate) struct SessionIdLatch {
    agent: AgentId,
    /// For log fields only — the incarnation guard is carried by the commit port.
    epoch: u32,
    commit: SessionIdSink,
    settled: AtomicBool,
    state: Mutex<LatchState>,
}

#[derive(Default)]
struct LatchState {
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
        })
    }

    pub(crate) fn offer_sink(self: &Arc<Self>) -> SessionIdSink {
        let latch = Arc::clone(self);
        Arc::new(move |raw: &str| latch.offer(raw))
    }

    /// The first-turn port for the mode where the path carries the turn — calling it is [`SessionIdLatch::note_submission`].
    ///
    /// The session does not count submissions there: counting a first input that can still be withdrawn with ✕ would persist an id with no
    ///   conversation. The path calls this at the echo of the peer's first user message.
    // ADR-0226 · ADR-0231 · ADR-0233: the id is persisted when it becomes real (the conversation has a turn = the first echo).
    pub(crate) fn first_turn_sink(self: &Arc<Self>) -> FirstTurnSink {
        let latch = Arc::clone(self);
        Arc::new(move || latch.note_submission())
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
            // ★Do not delete this line★: the commit port's reflection log remains after persistence, but for a 0-turn incarnation nothing else
            //   announces 「the id has arrived」 — field diagnosis and GUI measurement wait on it. The id value is not put in.
            OfferOutcome::Held => tracing::info!(
                agent = %self.agent,
                epoch = self.epoch,
                "received the session id — before the first submission, so persistence is held back"
            ),
            OfferOutcome::Dropped => tracing::debug!(
                agent = %self.agent,
                epoch = self.epoch,
                "discarding this incarnation's second session id — a call outside the record port contract (once per incarnation)"
            ),
            OfferOutcome::Committed => {}
        }
    }

    /// Call it **right before** the first turn goes out. If the id is already received it commits on the spot and returns after the commit
    /// returns — the caller must send the turn **after** this returns for 「persist before the first turn」.
    /// ★The path-carries-the-turn mode calls it **after** the first echo ([`Self::first_turn_sink`])★ and has no 「persist before the first
    ///   turn」.
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

    /// The cap on every wait of a threaded item — ★it prevents hanging, it is not a correctness threshold★. A correct latch finishes
    /// within milliseconds, so reaching it means the regression where it never returns, and then the test fails instead of hanging.
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
            "not called before submission"
        );
        latch.note_submission();

        assert_eq!(calls.lock().unwrap().as_slice(), ["sid-1"]);
    }

    /// The first-turn port (`first_turn_sink`) is the same submission input — the mode where the path carries the turn counts only with
    ///   this (ADR-0226 amendment). Called many times, commit is once.
    #[test]
    fn the_first_turn_port_is_the_submission_input_and_commits_once() {
        let (port, calls) = recording_port();
        let latch = latch_with(port);
        let first_turn = latch.first_turn_sink();

        (latch.offer_sink())("sid-1");
        assert!(
            calls.lock().unwrap().is_empty(),
            "not called before the first turn"
        );
        first_turn();
        first_turn();

        assert_eq!(calls.lock().unwrap().as_slice(), ["sid-1"]);
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

    /// ⑤ — the success branch of 「the port at most once per incarnation」. A second offer after commit is outside the contract and is dropped.
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

    /// ⑩ — the refusal branch of 「the port at most once per incarnation」. A port that returns writing nothing is indistinguishable from
    ///   success to the latch (no return): the fast path stands and the port is not called again.
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

    /// ⑨ — the panic branch of 「the port at most once per incarnation」. ★Exists only in builds without the hook★ (production ends this
    ///   panic, ADR-0292). The poisoned mutex does not stall the next call, and the port is not called again.
    #[cfg(panic = "unwind")]
    #[test]
    fn a_commit_that_panics_is_never_called_again() {
        let calls = Arc::new(AtomicUsize::new(0));
        let counted = calls.clone();
        let panicking: SessionIdSink = Arc::new(move |_raw: &str| {
            counted.fetch_add(1, Ordering::SeqCst);
            panic!("test rig: the commit port panics");
        });
        let latch = latch_with(panicking);

        latch.offer("sid-1");
        let unwound =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| latch.note_submission()));
        assert!(
            unwound.is_err(),
            "the panic propagates up through that call"
        );
        assert!(latch.state.is_poisoned(), "premise: the mutex is poisoned");

        latch.note_submission();
        latch.offer("sid-2");
        latch.note_submission();

        assert_eq!(calls.load(Ordering::SeqCst), 1, "called the port again");
        assert!(
            latch.settled.load(Ordering::Acquire),
            "after clearing the poison the fast path must be in place"
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
                    done_tx.send(()).expect("test body");
                })
            };
            // Rotate the launch order per run — the thread that hit the barrier last tends to run first, so a fixed order would rarely
            //   take the 「offer first」 branch (measured).
            let mut acts: [fn(&SessionIdLatch); 3] = [
                |l| l.offer("sid-1"),
                |l| l.note_submission(),
                |l| l.note_submission(),
            ];
            acts.rotate_left(round % 3);
            let handles = acts.map(spawn);
            for _ in &handles {
                done_rx.recv_timeout(HANG_GUARD).unwrap_or_else(|e| {
                    panic!("in round {round} the test thread does not come back: {e}")
                });
            }
            for h in handles {
                h.join().expect("test thread");
            }
            assert_eq!(
                calls.lock().unwrap().as_slice(),
                ["sid-1"],
                "in round {round} the commit count is not 1"
            );
        }
    }

    /// ⑦ ★Regression net for the fast-path race★ — while a commit is in progress, another thread's submission must not return **before it
    ///   returns**, else that turn goes out before persistence.
    ///
    /// Holds the commit port with a channel and watches one span that the second submitter does not return — ★the span is only a window to
    ///   catch the regression★: a correct latch passes at any length, a regression setting the fast path by the submission marker returns
    ///   within microseconds.
    #[test]
    fn a_submission_racing_an_in_flight_commit_waits_for_it_to_return() {
        let (entered_tx, entered_rx) = mpsc::channel::<()>();
        let (release_tx, release_rx) = mpsc::channel::<()>();
        let release_rx = Mutex::new(release_rx);
        let events = Arc::new(Mutex::new(Vec::<&'static str>::new()));
        let port_events = events.clone();
        let held: SessionIdSink = Arc::new(move |_raw: &str| {
            entered_tx.send(()).expect("the test body waits");
            release_rx
                .lock()
                .unwrap()
                .recv_timeout(HANG_GUARD)
                .expect("the test body does not release the port");
            port_events.lock().unwrap().push("commit returned");
        });
        let latch = latch_with(held);
        latch.offer("sid-1");

        let (first_done_tx, first_done_rx) = mpsc::channel::<()>();
        let first = {
            let latch = latch.clone();
            thread::spawn(move || {
                latch.note_submission();
                first_done_tx.send(()).expect("test body");
            })
        };
        entered_rx.recv_timeout(HANG_GUARD).expect(
            "the first submission does not enter the commit port — the submission after offer did not call commit",
        );

        let (returned_tx, returned_rx) = mpsc::channel::<()>();
        let second = {
            let latch = latch.clone();
            let events = events.clone();
            thread::spawn(move || {
                latch.note_submission();
                events.lock().unwrap().push("second submission returned");
                returned_tx.send(()).expect("test body");
            })
        };
        assert!(
            matches!(
                returned_rx.recv_timeout(Duration::from_millis(200)),
                Err(mpsc::RecvTimeoutError::Timeout)
            ),
            "the second submission returned while commit was held — the first turn goes out before persistence"
        );

        release_tx.send(()).expect("port");
        first_done_rx
            .recv_timeout(HANG_GUARD)
            .expect("the port was released but the first submitter does not come back");
        returned_rx
            .recv_timeout(HANG_GUARD)
            .expect("commit came back but the second submitter does not come back");
        first.join().expect("first submitter");
        second.join().expect("second submitter");
        assert_eq!(
            events.lock().unwrap().as_slice(),
            ["commit returned", "second submission returned"]
        );
    }
}
