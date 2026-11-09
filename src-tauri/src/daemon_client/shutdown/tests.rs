//! The stop path on a client over memory networks and one manual clock (TRD S21 trd-3-2-u5 §5): the test side plays the
//! daemon on the stop network, scripts `daemon.json` and the target process, and watches the shell's link through its
//! endpoint source.
//!
//! Waits poll with real sleeps up to a bound: `daemon.json` is read on the blocking pool, which the transport's `settle`
//! does not wait for.

use super::*;

use std::collections::VecDeque;
use std::sync::atomic::AtomicUsize;
use std::sync::mpsc;

use engram_dashboard_protocol::{Secret, PROTOCOL_VERSION};
use engram_dashboard_transport::testing::{ClientMsg, ManualClock, MemoryEndpoint, MemoryNetwork};
use engram_dashboard_transport::{Address, Frame};
use futures_util::FutureExt;
use tokio::task::JoinHandle;

use crate::daemon_client::events::NoDaemonEvents;
use crate::daemon_client::link::endpoint::{Current, Endpoint, EndpointSource};
use crate::daemon_client::link::stop::RecordRead;
use crate::daemon_client::link::DaemonKey;
use crate::daemon_client::refusal::NoRefusalNotice;
use crate::daemon_client::{ConnectionState, HandshakeError, NoAgentNames};
use crate::discovery::tests::{info, FakeLiveness};

const PID: u32 = 1001;
const START: u64 = 77;

/// `daemon.json` as the test scripts it — the last answer repeats; the next read can be held.
struct Record {
    answers: Mutex<VecDeque<RecordRead>>,
    hold: Mutex<Option<(mpsc::Sender<()>, mpsc::Receiver<()>)>>,
    reads: AtomicUsize,
}

impl Record {
    fn new(answers: impl IntoIterator<Item = RecordRead>) -> Arc<Self> {
        Arc::new(Self {
            answers: Mutex::new(answers.into_iter().collect()),
            hold: Mutex::new(None),
            reads: AtomicUsize::new(0),
        })
    }

    /// Holds the next read: returns "it started" and the release.
    fn hold_next(&self) -> (mpsc::Receiver<()>, mpsc::Sender<()>) {
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        *self.hold.lock().unwrap() = Some((entered_tx, release_rx));
        (entered_rx, release_tx)
    }

    fn reads(&self) -> usize {
        self.reads.load(Ordering::SeqCst)
    }
}

impl RecordReader for Record {
    fn read(&self) -> RecordRead {
        self.reads.fetch_add(1, Ordering::SeqCst);
        let held = self.hold.lock().unwrap().take();
        if let Some((entered, release)) = held {
            let _ = entered.send(());
            // Bounded, so a failing test does not leave a blocked pool thread behind.
            let _ = release.recv_timeout(Duration::from_secs(10));
        }
        let mut answers = self.answers.lock().unwrap();
        if answers.len() > 1 {
            answers.pop_front().unwrap()
        } else {
            answers.front().cloned().expect("a scripted answer")
        }
    }
}

fn record(pid: u32, start_time: u64, version: u32) -> RecordRead {
    let mut daemon = info(pid, version);
    daemon.start_time = start_time;
    RecordRead::Bytes(daemon.to_json_pretty().unwrap())
}

fn our_daemon() -> RecordRead {
    record(PID, START, PROTOCOL_VERSION)
}

/// The target process as the test scripts it: what "gone" answers, and what it answers once killed.
struct Target {
    clock: ManualClock,
    started: Instant,
    state: Mutex<TargetState>,
}

struct TargetState {
    presence: Presence,
    after_kill: Presence,
    probes: Vec<(u32, u64)>,
    /// Each kill's identity and when it came, from the test's start.
    kills: Vec<(u32, u64, Duration)>,
}

impl Target {
    fn set(&self, presence: Presence) {
        self.state.lock().unwrap().presence = presence;
    }

    fn kills(&self) -> Vec<(u32, u64, Duration)> {
        self.state.lock().unwrap().kills.clone()
    }

    fn probes(&self) -> Vec<(u32, u64)> {
        self.state.lock().unwrap().probes.clone()
    }
}

impl TargetProcess for Target {
    fn presence(&self, pid: u32, start_time: u64) -> Presence {
        let mut state = self.state.lock().unwrap();
        state.probes.push((pid, start_time));
        state.presence
    }

    fn terminate_if_same(&self, pid: u32, start_time: u64) -> std::io::Result<bool> {
        let at = self.clock.now().duration_since(self.started);
        let mut state = self.state.lock().unwrap();
        state.kills.push((pid, start_time, at));
        state.presence = state.after_kill;
        Ok(true)
    }
}

/// The shell link's endpoint source — counts the dials' record reads (a re-add shows here) and the daemon searches.
struct LinkSource {
    found: Mutex<Option<Endpoint>>,
    reads: AtomicUsize,
    searches: AtomicUsize,
    /// The next search waits for this release.
    search_hold: Mutex<Option<mpsc::Receiver<()>>>,
}

impl LinkSource {
    fn reads(&self) -> usize {
        self.reads.load(Ordering::SeqCst)
    }

    /// Holds the next daemon search (`connect`'s): returns the release.
    fn hold_search(&self) -> mpsc::Sender<()> {
        let (release_tx, release_rx) = mpsc::channel();
        *self.search_hold.lock().unwrap() = Some(release_rx);
        release_tx
    }
}

impl EndpointSource for LinkSource {
    fn ensure(&self, _timeout: Duration) -> Result<Endpoint, String> {
        self.searches.fetch_add(1, Ordering::SeqCst);
        let held = self.search_hold.lock().unwrap().take();
        if let Some(release) = held {
            // Bounded, so a failing test does not leave a blocked pool thread behind.
            let _ = release.recv_timeout(Duration::from_secs(10));
        }
        self.found
            .lock()
            .unwrap()
            .clone()
            .ok_or_else(|| "no daemon in this test".to_owned())
    }

    fn current(&self) -> Current {
        self.reads.fetch_add(1, Ordering::SeqCst);
        match self.found.lock().unwrap().clone() {
            Some(endpoint) => Current::Live(endpoint),
            None => Current::Absent,
        }
    }
}

struct Stage {
    clock: ManualClock,
    started: Instant,
    daemons: MemoryNetwork,
    link_net: MemoryNetwork,
    record: Arc<Record>,
    target: Arc<Target>,
    source: Arc<LinkSource>,
    client: Arc<DaemonClient>,
}

impl Stage {
    /// A daemon that is alive until killed, and gone once killed.
    fn new(record: Arc<Record>) -> Self {
        Self::with_target(record, Presence::Alive, Presence::Gone)
    }

    fn with_target(record: Arc<Record>, presence: Presence, after_kill: Presence) -> Self {
        let clock = ManualClock::new();
        let started = clock.now();
        let daemons = MemoryNetwork::new();
        let link_net = MemoryNetwork::new();
        let target = Arc::new(Target {
            clock: clock.clone(),
            started,
            state: Mutex::new(TargetState {
                presence,
                after_kill,
                probes: Vec::new(),
                kills: Vec::new(),
            }),
        });
        let source = Arc::new(LinkSource {
            found: Mutex::new(None),
            reads: AtomicUsize::new(0),
            searches: AtomicUsize::new(0),
            search_hold: Mutex::new(None),
        });
        let client = Arc::new(DaemonClient::new_on_link_with_stop(
            Handle::current(),
            link_net.dialer(),
            source.clone(),
            clock.handle(),
            Arc::new(NoDaemonEvents),
            Arc::new(NoAgentNames),
            Arc::new(NoRefusalNotice),
            StopPorts {
                reader: record.clone(),
                liveness: Arc::new(FakeLiveness { dead: vec![] }),
                process: target.clone(),
                dialer: daemons.dialer(),
            },
        ));
        Self {
            clock,
            started,
            daemons,
            link_net,
            record,
            target,
            source,
            client,
        }
    }

    fn stop(&self, plan: StopPlan) -> JoinHandle<StopOutcome> {
        let client = self.client.clone();
        tokio::spawn(async move { client.stop_daemon(plan, StopOrigin::Tray).await })
    }

    /// Starts a `User` flight at `at`, the way `stop_daemon` starts one — the returned channel gets its outcome watch.
    fn start_a_flight_at(
        &self,
        at: RacePoint,
    ) -> mpsc::Receiver<watch::Receiver<Option<StopOutcome>>> {
        let (flight_tx, flight) = mpsc::channel();
        let stopper = self.client.stopper.clone();
        self.client.stopper.mode().race.arm(at, move || {
            let started = stopper.clock().now();
            let (outcome, _) = stopper.join_or_start(StopPlan::User, StopOrigin::Tray, started);
            let _ = flight_tx.send(outcome);
        });
        flight
    }

    fn elapsed(&self) -> Duration {
        self.clock.now().duration_since(self.started)
    }

    /// The daemon's side of the stop link.
    async fn daemon(&self) -> Arc<MemoryEndpoint> {
        wait_for("the stop link is opened", || {
            self.daemons.open_endpoints() > 0
        })
        .await;
        self.daemons.accept().unwrap()
    }

    /// Lets the stop in: `Hello`, then waits for `StopDaemon`.
    async fn hello_then_stop_daemon(&self, daemon: &MemoryEndpoint) {
        daemon.push(Frame::Text(r#"{"Hello":{}}"#.into()));
        let mut seen = Vec::new();
        wait_for("StopDaemon is written", || {
            seen.extend(daemon.frames());
            seen.iter()
                .any(|f| matches!(f, Frame::Text(t) if t.starts_with(r#"{"StopDaemon""#)))
        })
        .await;
    }

    /// Moves the clock in steps, yielding between them, until `done`.
    async fn advance_until(&self, what: &str, step: Duration, mut done: impl FnMut() -> bool) {
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while !done() {
            assert!(
                std::time::Instant::now() < deadline,
                "not reached within 10 s: {what}"
            );
            self.clock.advance(step);
            for _ in 0..5 {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        }
    }
}

async fn wait_for(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while !done() {
        assert!(
            std::time::Instant::now() < deadline,
            "not reached within 10 s: {what}"
        );
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
}

async fn joined<T>(what: &str, task: JoinHandle<T>) -> T {
    wait_for(what, || task.is_finished()).await;
    task.await.expect("the task ran to its end")
}

/// The outcome of the flight [`Stage::start_a_flight_at`] started.
async fn flight_outcome(
    flight: &mpsc::Receiver<watch::Receiver<Option<StopOutcome>>>,
) -> StopOutcome {
    let outcome = flight.try_recv().expect("the race step started a flight");
    wait_for("the flight ends", || outcome.borrow().is_some()).await;
    let done = outcome.borrow().expect("published");
    done
}

/// A few real milliseconds for the runtime — for "this did not happen" checks.
async fn let_it_run() {
    for _ in 0..50 {
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
}

// ── fallback and outcomes ───────────────────────────────────────────────────────────────

#[tokio::test]
async fn nothing_sent_to_a_live_daemon_kills_it_at_once_with_the_flight_start_identity() {
    // Later reads would name another daemon — the kill must not re-read.
    let s = Stage::new(Record::new([
        our_daemon(),
        record(2002, 99, PROTOCOL_VERSION),
    ]));
    s.daemons.fail_next(1);
    let outcome = joined("stop", s.stop(StopPlan::User)).await;
    assert_eq!(outcome, StopOutcome::Killed);
    assert_eq!(
        s.target.kills(),
        [(PID, START, Duration::ZERO)],
        "at once, flight-start identity"
    );
    assert_eq!(s.record.reads(), 1, "one read per flight");
    assert_eq!(s.daemons.dials(), 1, "one dial");
}

#[tokio::test]
async fn a_slow_hello_inside_the_window_is_a_graceful_stop_without_a_kill() {
    let s = Stage::new(Record::new([our_daemon()]));
    let task = s.stop(StopPlan::User);
    let daemon = s.daemon().await;
    s.clock.advance(Duration::from_secs(9));
    let_it_run().await;
    s.hello_then_stop_daemon(&daemon).await;
    s.target.set(Presence::Gone);
    daemon.close();
    assert_eq!(joined("stop", task).await, StopOutcome::Closed);
    assert!(s.target.kills().is_empty());
}

#[tokio::test]
async fn no_hello_by_the_dial_bound_kills() {
    let s = Stage::new(Record::new([our_daemon()]));
    let task = s.stop(StopPlan::User);
    let _daemon = s.daemon().await;
    s.advance_until("the kill", Duration::from_millis(500), || {
        !s.target.kills().is_empty()
    })
    .await;
    let kill_at = s.target.kills()[0].2;
    assert!(
        kill_at >= Duration::from_millis(9_500) && kill_at < Duration::from_secs(10),
        "the dial is bounded by 10 s − the reserve, then the kill comes at once: {kill_at:?}"
    );
    assert_eq!(joined("stop", task).await, StopOutcome::Killed);
}

#[tokio::test]
async fn a_sent_stop_whose_daemon_stays_up_is_killed_at_the_graceful_deadline() {
    let s = Stage::new(Record::new([our_daemon()]));
    let task = s.stop(StopPlan::User);
    let daemon = s.daemon().await;
    s.hello_then_stop_daemon(&daemon).await;
    daemon.close();
    s.clock.advance(Duration::from_secs(9));
    let_it_run().await;
    assert!(
        s.target.kills().is_empty(),
        "no kill before the graceful deadline"
    );
    s.advance_until("the kill", Duration::from_millis(500), || {
        !s.target.kills().is_empty()
    })
    .await;
    assert_eq!(s.target.kills()[0].2, Duration::from_secs(10));
    assert_eq!(joined("stop", task).await, StopOutcome::Killed);
}

#[tokio::test]
async fn a_sent_stop_finishes_as_soon_as_the_daemon_is_gone() {
    let s = Stage::new(Record::new([our_daemon()]));
    let task = s.stop(StopPlan::User);
    let daemon = s.daemon().await;
    s.hello_then_stop_daemon(&daemon).await;
    daemon.close();
    s.clock.advance(Duration::from_millis(300));
    let_it_run().await;
    s.target.set(Presence::Gone);
    s.clock.advance(Duration::from_millis(100));
    assert_eq!(joined("stop", task).await, StopOutcome::Closed);
    assert!(
        s.elapsed() < Duration::from_secs(1),
        "no sleeping to the deadline"
    );
    assert!(s.target.kills().is_empty());
}

#[tokio::test]
async fn a_record_without_a_start_time_is_never_killed() {
    let s = Stage::new(Record::new([record(PID, 0, PROTOCOL_VERSION)]));
    s.daemons.fail_next(1);
    assert_eq!(
        joined("stop", s.stop(StopPlan::User)).await,
        StopOutcome::Timeout
    );
    assert!(s.target.kills().is_empty(), "no identity → no kill");
    assert!(
        s.target.probes().iter().all(|&probe| probe == (PID, 0)),
        "\"gone\" is asked without a start time, so only 87 or the exit code can answer it"
    );
}

#[tokio::test]
async fn an_unknown_process_is_not_gone_it_is_killed_and_still_reported_as_a_timeout() {
    let s = Stage::with_target(
        Record::new([our_daemon()]),
        Presence::Unknown,
        Presence::Unknown,
    );
    s.daemons.fail_next(1);
    let task = s.stop(StopPlan::User);
    s.advance_until("the stop ends", Duration::from_millis(500), || {
        task.is_finished()
    })
    .await;
    assert_eq!(joined("stop", task).await, StopOutcome::Timeout);
    assert_eq!(
        s.target.kills().len(),
        1,
        "unknown counts as alive for the fallback"
    );
}

#[tokio::test]
async fn no_daemon_or_a_newer_daemon_is_no_target_with_no_dial() {
    for read in [RecordRead::Absent, record(PID, START, PROTOCOL_VERSION + 1)] {
        let s = Stage::new(Record::new([read]));
        assert_eq!(
            joined("stop", s.stop(StopPlan::User)).await,
            StopOutcome::NoTarget
        );
        assert_eq!(s.daemons.dials(), 0);
        assert!(s.target.kills().is_empty());
    }
}

// ── deadlines ───────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn a_busy_record_is_read_again_within_the_flight() {
    let s = Stage::new(Record::new([
        RecordRead::Busy,
        RecordRead::Busy,
        our_daemon(),
    ]));
    s.daemons.fail_next(1);
    let task = s.stop(StopPlan::User);
    s.advance_until("the stop ends", Duration::from_millis(100), || {
        task.is_finished()
    })
    .await;
    assert_eq!(joined("stop", task).await, StopOutcome::Killed);
    assert_eq!(s.record.reads(), 3);
}

#[tokio::test]
async fn a_record_that_stays_busy_is_no_target() {
    let s = Stage::new(Record::new([RecordRead::Busy]));
    let task = s.stop(StopPlan::User);
    s.advance_until("the stop ends", Duration::from_millis(100), || {
        task.is_finished()
    })
    .await;
    assert_eq!(joined("stop", task).await, StopOutcome::NoTarget);
    assert_eq!(s.record.reads(), 5, "5 reads, 100 ms apart");
    assert_eq!(s.daemons.dials(), 0);
}

#[tokio::test]
async fn a_slow_record_read_eats_into_the_dial_bound() {
    let record = Record::new([our_daemon()]);
    let (entered, release) = record.hold_next();
    let s = Stage::new(record);
    let task = s.stop(StopPlan::User);
    wait_for("the read started", || entered.try_recv().is_ok()).await;
    s.clock.advance(Duration::from_secs(5));
    release.send(()).unwrap();
    let _daemon = s.daemon().await;
    s.clock.advance(Duration::from_millis(4_400));
    let_it_run().await;
    assert!(
        s.target.kills().is_empty(),
        "still inside 10 s − 5 s − the reserve"
    );
    s.clock.advance(Duration::from_millis(200));
    assert_eq!(joined("stop", task).await, StopOutcome::Killed);
    assert_eq!(
        s.target.kills()[0].2,
        Duration::from_millis(9_600),
        "the dial gave up at 5 s + 4.5 s — measured from flight start"
    );
}

#[tokio::test]
async fn an_expired_flight_drops_its_late_read_dials_and_kills_nothing_and_fires_the_re_add() {
    let record = Record::new([our_daemon()]);
    let (entered, release) = record.hold_next();
    let s = Stage::new(record);
    let task = s.stop(StopPlan::User);
    wait_for("the read started", || entered.try_recv().is_ok()).await;
    assert!(s.client.stopper.mode().active());

    s.clock.advance(Duration::from_secs(13));
    assert_eq!(joined("stop", task).await, StopOutcome::Timeout);
    assert!(
        !s.client.stopper.mode().active(),
        "expiry releases the mode"
    );
    wait_for("the re-add dials", || s.source.reads() > 0).await;

    release.send(()).unwrap();
    let_it_run().await;
    assert_eq!(s.daemons.dials(), 0, "the late read leads nowhere");
    assert!(s.target.kills().is_empty());
}

#[tokio::test]
async fn a_flight_measures_its_deadlines_from_the_instant_its_caller_asked() {
    let s = Stage::new(Record::new([our_daemon()]));
    // The blocking wrapper takes that instant before it spawns the stop — here the spawn ran 5 s late.
    let asked = s.clock.now();
    s.clock.advance(Duration::from_secs(5));
    let task = {
        let stopper = s.client.stopper.clone();
        tokio::spawn(async move { stopper.stop(StopPlan::User, StopOrigin::Tray, asked).await })
    };
    let _daemon = s.daemon().await;
    s.advance_until("the kill", Duration::from_millis(500), || {
        !s.target.kills().is_empty()
    })
    .await;
    let kill_at = s.target.kills()[0].2;
    assert!(
        kill_at >= Duration::from_millis(9_500) && kill_at < Duration::from_secs(10),
        "the dial gave up at 10 s − the reserve from the ask, not from the late start: {kill_at:?}"
    );
    assert_eq!(joined("stop", task).await, StopOutcome::Killed);
}

// ── exit ────────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn exit_reads_until_its_bound_never_closes_early_never_kills_and_does_not_re_add() {
    let s = Stage::new(Record::new([our_daemon()]));
    let task = s.stop(StopPlan::Exit);
    let daemon = s.daemon().await;
    s.hello_then_stop_daemon(&daemon).await;
    s.clock.advance(Duration::from_millis(2_900));
    let_it_run().await;
    assert!(!task.is_finished(), "still reading inside the 3 s bound");
    assert!(
        !daemon
            .drain()
            .iter()
            .any(|m| matches!(m, ClientMsg::Close(_))),
        "no close right behind StopDaemon"
    );
    s.clock.advance(Duration::from_millis(100));
    assert_eq!(joined("stop", task).await, StopOutcome::Timeout);
    assert!(s.target.kills().is_empty(), "exit never kills");
    // Known limit: a bounded real wait — a re-add slower than this would pass unseen.
    let_it_run().await;
    assert_eq!(s.source.reads(), 0, "no re-add on exit");
}

#[tokio::test]
async fn exit_ends_as_soon_as_the_daemon_is_gone() {
    let s = Stage::new(Record::new([our_daemon()]));
    let task = s.stop(StopPlan::Exit);
    let daemon = s.daemon().await;
    s.hello_then_stop_daemon(&daemon).await;
    s.target.set(Presence::Gone);
    daemon.close();
    assert_eq!(joined("stop", task).await, StopOutcome::Closed);
    assert_eq!(s.elapsed(), Duration::ZERO);
}

// ── single flight · stopping mode ───────────────────────────────────────────────────────

#[tokio::test]
async fn joiners_share_one_flight_and_its_outcome() {
    let s = Stage::new(Record::new([our_daemon()]));
    let first = s.stop(StopPlan::User);
    let daemon = s.daemon().await;
    let second = s.stop(StopPlan::User);
    let_it_run().await;
    s.hello_then_stop_daemon(&daemon).await;
    s.target.set(Presence::Gone);
    daemon.close();
    assert_eq!(joined("first", first).await, StopOutcome::Closed);
    assert_eq!(joined("second", second).await, StopOutcome::Closed);
    assert_eq!(s.daemons.dials(), 1, "one flight");
    assert_eq!(s.record.reads(), 1);
}

#[tokio::test]
async fn an_exit_joiner_waits_only_its_own_bound() {
    let s = Stage::new(Record::new([our_daemon()]));
    let user = s.stop(StopPlan::User);
    let _daemon = s.daemon().await;
    let exit = s.stop(StopPlan::Exit);
    let_it_run().await;
    s.clock.advance(Duration::from_secs(4));
    assert_eq!(joined("exit joiner", exit).await, StopOutcome::Timeout);
    assert!(!user.is_finished(), "the user's flight goes on");
    assert!(s.client.stopper.mode().active());
    s.advance_until("the user's stop ends", Duration::from_millis(500), || {
        user.is_finished()
    })
    .await;
    assert_eq!(joined("user", user).await, StopOutcome::Killed);
}

#[tokio::test]
async fn during_a_stop_ensure_answers_not_connected_and_adds_nothing() {
    let record = Record::new([our_daemon()]);
    let (entered, release) = record.hold_next();
    let s = Stage::new(record);
    let task = s.stop(StopPlan::User);
    wait_for("the read started", || entered.try_recv().is_ok()).await;

    assert_eq!(s.client.ensure().await, Err(HandshakeError::Stopping));
    let_it_run().await;
    assert_eq!(s.source.reads(), 0, "nothing was added");
    assert_eq!(s.link_net.dials(), 0);

    s.daemons.fail_next(1);
    release.send(()).unwrap();
    assert_eq!(joined("stop", task).await, StopOutcome::Killed);
    wait_for("the re-add after a stop dials", || s.source.reads() > 0).await;
}

// ★The two orderings below are what keeps a link from being added during a stop★ — each test lands the racing side
//   between the two steps (`RacePoint`), so swapping those steps fails it.

#[tokio::test]
async fn an_ensure_that_read_the_epoch_before_the_stop_began_answers_stopping_and_adds_nothing() {
    let record = Record::new([RecordRead::Absent]);
    let (entered, release) = record.hold_next();
    let s = Stage::new(record);
    // The stop's whole start (mode on, link closed) lands after this ensure read the close epoch, before its mode check.
    let flight = s.start_a_flight_at(RacePoint::EnsureEpochRead);
    let answer = tokio::time::timeout(Duration::from_secs(10), s.client.ensure())
        .await
        .expect("ensure answers");
    assert_eq!(answer, Err(HandshakeError::Stopping));
    wait_for("the stop's read started", || entered.try_recv().is_ok()).await;
    let_it_run().await;
    assert_eq!(s.source.reads(), 0, "nothing was added");
    assert_eq!(s.link_net.dials(), 0);

    release.send(()).unwrap();
    assert_eq!(flight_outcome(&flight).await, StopOutcome::NoTarget);
}

#[tokio::test]
async fn an_ensure_between_the_stops_mode_and_its_close_answers_stopping_and_adds_nothing() {
    let record = Record::new([RecordRead::Absent]);
    let (entered, release) = record.hold_next();
    let s = Stage::new(record);
    // One poll of `ensure` there: its epoch read and mode check run before its first await.
    let (seen_tx, seen) = mpsc::channel();
    let client = s.client.clone();
    s.client
        .stopper
        .mode()
        .race
        .arm(RacePoint::StopModeEntered, move || {
            let _ = seen_tx.send(client.ensure().now_or_never());
        });
    let task = s.stop(StopPlan::User);
    wait_for("the stop's read started", || entered.try_recv().is_ok()).await;
    assert_eq!(
        seen.try_recv().expect("the step ran"),
        Some(Err(HandshakeError::Stopping))
    );
    let_it_run().await;
    assert_eq!(s.source.reads(), 0, "nothing was added");
    assert_eq!(s.link_net.dials(), 0);

    release.send(()).unwrap();
    assert_eq!(joined("stop", task).await, StopOutcome::NoTarget);
}

#[tokio::test]
async fn a_connect_that_read_the_epoch_before_the_stops_close_waits_it_out_and_connects_on_a_fresh_epoch(
) {
    let record = Record::new([RecordRead::Absent]);
    let (entered, release) = record.hold_next();
    let s = Stage::new(record);
    *s.source.found.lock().unwrap() = Some(Endpoint {
        key: DaemonKey::new("local"),
        address: Address::new("ws://127.0.0.1:5123"),
        credential: Secret::new("t".repeat(64)),
    });
    // The stop starts after this connect read the close epoch, before its mode check — so the stop's close bumps the
    // epoch this connect already holds (R16).
    let flight = s.start_a_flight_at(RacePoint::ConnectEpochRead);
    let release_search = s.source.hold_search();
    let connect = {
        let client = s.client.clone();
        tokio::spawn(async move { client.connect().await })
    };
    wait_for("the stop's read started", || entered.try_recv().is_ok()).await;
    let_it_run().await;
    assert!(!connect.is_finished(), "connect waits for the stop");
    assert_eq!(s.source.searches.load(Ordering::SeqCst), 0);

    release.send(()).unwrap();
    assert_eq!(flight_outcome(&flight).await, StopOutcome::NoTarget);
    // The stop's re-add dials before connect may add: a handle replaced before its dial closes without one, and the
    // count below would never reach two.
    wait_for("the stop's re-add dialed", || s.link_net.dials() >= 1).await;
    release_search.send(()).unwrap();
    // No `Hello` yet, so the stop's re-add is not live and connect adds its own handle (a second dial) — or, on the
    // epoch it read before the stop, the add is refused and connect fails.
    wait_for("connect added or failed", || {
        connect.is_finished() || s.link_net.dials() >= 2
    })
    .await;
    // Answer every link with `Hello`.
    let hello = Frame::Text(
        serde_json::to_string(&engram_dashboard_protocol::AgentEvent::Hello {
            protocol_version: PROTOCOL_VERSION,
            daemon_version: "0.0.0".into(),
            capabilities: None,
        })
        .unwrap(),
    );
    // Held: a dropped endpoint is a failed link.
    let mut links = Vec::new();
    wait_for("connect resolves", || {
        while let Some(link) = s.link_net.accept() {
            link.push(hello.clone());
            links.push(link);
        }
        connect.is_finished()
    })
    .await;
    assert_eq!(
        joined("connect", connect).await,
        Ok(()),
        "the stop's own close is not an overtaking close (R16)"
    );
}

// ── blocking wrapper ────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn a_request_past_its_flight_deadline_starts_nothing() {
    let s = Stage::new(Record::new([our_daemon()]));
    *s.source.found.lock().unwrap() = Some(Endpoint {
        key: DaemonKey::new("local"),
        address: Address::new("ws://127.0.0.1:5123"),
        credential: Secret::new("t".repeat(64)),
    });
    let connect = {
        let client = s.client.clone();
        tokio::spawn(async move { client.connect().await })
    };
    let hello = Frame::Text(
        serde_json::to_string(&engram_dashboard_protocol::AgentEvent::Hello {
            protocol_version: PROTOCOL_VERSION,
            daemon_version: "0.0.0".into(),
            capabilities: None,
        })
        .unwrap(),
    );
    let link = {
        let mut link = None;
        wait_for("the link is dialed", || {
            link = link.take().or_else(|| s.link_net.accept());
            link.is_some()
        })
        .await;
        link.unwrap()
    };
    link.push(hello);
    assert_eq!(joined("connect", connect).await, Ok(()));

    // The wrapper took this instant, and its task ran only after the whole `User` bound.
    let asked = s.clock.now();
    s.clock.advance(StopPlan::User.flight_bound());
    let outcome = s
        .client
        .stopper
        .stop(StopPlan::User, StopOrigin::Tray, asked)
        .await;
    assert_eq!(outcome, StopOutcome::Timeout);
    let_it_run().await;
    assert!(!s.client.stopper.mode().active(), "no mode");
    assert_eq!(
        s.client.state(),
        ConnectionState::Connected,
        "the link is not closed"
    );
    assert_eq!(s.link_net.dials(), 1, "no re-add");
    assert_eq!(s.record.reads(), 0, "no flight");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_blocking_wrapper_measures_the_flight_from_its_call() {
    let s = Stage::new(Record::new([our_daemon()]));
    // The stop task starts 5 s after the call.
    let clock = s.clock.clone();
    s.client
        .stopper
        .mode()
        .race
        .arm(RacePoint::BlockingStartTaken, move || {
            clock.advance(Duration::from_secs(5))
        });
    let task = {
        let client = s.client.clone();
        tokio::task::spawn_blocking(move || {
            client.stop_daemon_blocking(StopPlan::User, StopOrigin::Tray)
        })
    };
    let _daemon = s.daemon().await;
    s.advance_until("the kill", Duration::from_millis(500), || {
        !s.target.kills().is_empty()
    })
    .await;
    let kill_at = s.target.kills()[0].2;
    assert!(
        kill_at >= Duration::from_millis(9_500) && kill_at < Duration::from_secs(10),
        "the dial gave up at 10 s − the reserve from the call, not from the late task: {kill_at:?}"
    );
    assert_eq!(joined("stop", task).await, StopOutcome::Killed);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_blocking_wrapper_gives_up_at_the_flight_bound() {
    let record = Record::new([our_daemon()]);
    let (_entered, release) = record.hold_next();
    // The manual clock never moves here, so only the wrapper's own bound can end the wait.
    let s = Stage::new(record);
    let client = s.client.clone();
    let started = std::time::Instant::now();
    let outcome = tokio::task::spawn_blocking(move || {
        client.stop_daemon_blocking(StopPlan::Exit, StopOrigin::AppExit)
    })
    .await
    .unwrap();
    let waited = started.elapsed();
    let _ = release.send(());
    assert_eq!(outcome, StopOutcome::Timeout);
    assert!(
        waited >= Duration::from_secs(4) && waited < Duration::from_secs(9),
        "bounded by Exit's 3 s + 1 s: {waited:?}"
    );
}

// The refusal is a `debug_assert` — a release build has none to see.
#[cfg(debug_assertions)]
#[tokio::test]
async fn the_blocking_wrapper_refuses_a_client_runtime_thread() {
    let s = Stage::new(Record::new([RecordRead::Absent]));
    let client = s.client.clone();
    let on_client_thread = std::thread::Builder::new()
        .name(CLIENT_THREAD_NAME.to_owned())
        .spawn(move || client.stop_daemon_blocking(StopPlan::Exit, StopOrigin::AppExit))
        .unwrap()
        .join();
    assert!(on_client_thread.is_err(), "the debug assertion stops it");
}
