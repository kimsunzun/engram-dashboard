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

use engram_dashboard_protocol::{AgentEvent, AgentId, AgentStatus, Secret, PROTOCOL_VERSION};
use engram_dashboard_transport::testing::{ClientMsg, ManualClock, MemoryEndpoint, MemoryNetwork};
use engram_dashboard_transport::{Address, Frame};
use tokio::task::JoinHandle;

use crate::daemon_client::link::endpoint::{Current, Endpoint, EndpointSource};
use crate::daemon_client::link::stop::RecordRead;
use crate::daemon_client::link::DaemonKey;
use crate::daemon_client::refusal::NoRefusalNotice;
use crate::daemon_client::test_support::{status_lines, RecordingEvents};
use crate::daemon_client::{Band, ConnectionState, ConnectionStatus};
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

/// The shell link's endpoint source — counts the dials' record reads (a re-add shows here).
struct LinkSource {
    found: Mutex<Option<Endpoint>>,
    reads: AtomicUsize,
    /// Set: what a dial's record read answers instead of `found`.
    current: Mutex<Option<Current>>,
}

impl LinkSource {
    fn reads(&self) -> usize {
        self.reads.load(Ordering::SeqCst)
    }
}

impl EndpointSource for LinkSource {
    fn ensure(&self, _timeout: Duration) -> Result<Endpoint, String> {
        self.found
            .lock()
            .unwrap()
            .clone()
            .ok_or_else(|| "no daemon in this test".to_owned())
    }

    fn current(&self) -> Current {
        self.reads.fetch_add(1, Ordering::SeqCst);
        if let Some(current) = self.current.lock().unwrap().clone() {
            return current;
        }
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
    events: Arc<RecordingEvents>,
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
            current: Mutex::new(None),
        });
        let events = Arc::new(RecordingEvents::default());
        let client = Arc::new(DaemonClient::new_on_link_with_stop(
            Handle::current(),
            link_net.dialer(),
            source.clone(),
            clock.handle(),
            events.clone(),
            events.clone(),
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
            events,
            client,
        }
    }

    /// Every connection status the frontend got, in order, one line per emit (`status:<word>:<band>`).
    fn statuses(&self) -> Vec<String> {
        status_lines(&self.events.seen())
    }

    fn stop(&self, plan: StopPlan) -> JoinHandle<StopOutcome> {
        let client = self.client.clone();
        tokio::spawn(async move { client.stop_daemon(plan, StopOrigin::Tray).await })
    }

    /// [`Self::stop`] with why the flight ended so.
    fn stop_with_reason(&self, plan: StopPlan) -> JoinHandle<StopResult> {
        let client = self.client.clone();
        tokio::spawn(async move {
            client
                .stop_daemon_with_reason(plan, StopOrigin::Command)
                .await
        })
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

/// `connect`s the shell's link to a daemon on the link network that answers `Hello` — returns the daemon's side.
async fn connected_link(s: &Stage) -> Arc<MemoryEndpoint> {
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
        serde_json::to_string(&AgentEvent::Hello {
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
    link
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
    let end = joined("stop", s.stop_with_reason(StopPlan::User)).await;
    assert_eq!(
        (end.outcome, end.reason),
        (StopOutcome::Timeout, Some(StopReason::NoIdentity))
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
    let task = s.stop_with_reason(StopPlan::User);
    s.advance_until("the stop ends", Duration::from_millis(500), || {
        task.is_finished()
    })
    .await;
    let end = joined("stop", task).await;
    assert_eq!(
        (end.outcome, end.reason),
        (StopOutcome::Timeout, Some(StopReason::StillRunning))
    );
    assert_eq!(
        s.target.kills().len(),
        1,
        "unknown counts as alive for the fallback"
    );
}

#[tokio::test]
async fn no_daemon_or_a_newer_daemon_is_no_target_with_no_dial() {
    for (read, reason) in [
        (RecordRead::Absent, StopReason::NoRecord),
        (
            record(PID, START, PROTOCOL_VERSION + 1),
            StopReason::NewerDaemon,
        ),
    ] {
        let s = Stage::new(Record::new([read]));
        let end = joined("stop", s.stop_with_reason(StopPlan::User)).await;
        assert_eq!(
            (end.outcome, end.reason),
            (StopOutcome::NoTarget, Some(reason))
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

    s.clock.advance(Duration::from_secs(13));
    assert_eq!(joined("stop", task).await, StopOutcome::Timeout);
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
        tokio::spawn(async move {
            stopper
                .stop(StopPlan::User, StopOrigin::Tray, asked)
                .await
                .outcome
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

// ── single flight ───────────────────────────────────────────────────────

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
    s.advance_until("the user's stop ends", Duration::from_millis(500), || {
        user.is_finished()
    })
    .await;
    assert_eq!(joined("user", user).await, StopOutcome::Killed);
}

// ── the shell's link through a stop (user decision 2026-11-10) ──────────────────────────────────────

#[tokio::test]
async fn during_a_user_stop_the_link_stays_open_and_the_daemons_shutdown_broadcasts_reach_the_frontend(
) {
    let s = Stage::new(Record::new([our_daemon()]));
    let main = connected_link(&s).await;
    let task = s.stop(StopPlan::User);
    let daemon = s.daemon().await;
    s.hello_then_stop_daemon(&daemon).await;
    // The daemon's `shutdown_all` after `StopDaemon`: its agent exits, then the emptied list — on the main link.
    let agent = AgentId::new_v4();
    for ev in [
        AgentEvent::StatusChanged {
            agent_id: agent,
            status: AgentStatus::Exited { code: Some(0) },
            epoch: 1,
        },
        AgentEvent::AgentListUpdated { agents: Vec::new() },
    ] {
        main.push(Frame::Text(serde_json::to_string(&ev).unwrap()));
    }
    let broadcasts = [
        format!("agent_status:{agent}:Exited {{ code: Some(0) }}:1"),
        "agent_list:0".to_owned(),
    ];
    wait_for("the shutdown broadcasts reach the frontend", || {
        let seen = s.events.seen();
        broadcasts.iter().all(|line| seen.contains(line))
    })
    .await;
    assert_eq!(s.client.state(), ConnectionState::Connected);

    // The daemon exits: both of its links end, and the main link's redials find nothing.
    s.link_net.fail_forever();
    s.target.set(Presence::Gone);
    daemon.close();
    main.close();
    assert_eq!(joined("stop", task).await, StopOutcome::Closed);
    s.advance_until(
        "band ① after the daemon exited",
        Duration::from_millis(100),
        || s.client.status().band == BAND_ONE,
    )
    .await;
}

// ── blocking wrapper ────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn a_request_past_its_flight_deadline_starts_nothing() {
    let s = Stage::new(Record::new([our_daemon()]));
    let _link = connected_link(&s).await;

    // The wrapper took this instant, and its task ran only after the whole `User` bound.
    let asked = s.clock.now();
    s.clock.advance(StopPlan::User.flight_bound());
    let outcome = s
        .client
        .stopper
        .stop(StopPlan::User, StopOrigin::Tray, asked)
        .await;
    assert_eq!(outcome.outcome, StopOutcome::Timeout);
    let_it_run().await;
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
        .blocking_start_taken
        .arm(move || clock.advance(Duration::from_secs(5)));
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

// ── bands around a stop (TRD S21 trd-3-2-u5 U4) ─────────────────────────────────────────

const BAND_ONE: Option<Band> = Some(Band::NotConnected { refused: false });

// A stop with nothing to stop still fires the re-add — a link parked on ② has no other way to leave it.
#[tokio::test]
async fn a_stop_with_no_target_still_re_adds_and_its_first_failure_shows_band_one() {
    let s = Stage::new(Record::new([RecordRead::Absent]));
    // The link's daemon speaks an older version: the attach is refused and shows ②.
    *s.source.current.lock().unwrap() = Some(Current::VersionMismatch {
        daemon: PROTOCOL_VERSION - 1,
        pid: PID,
        start_time: 0,
    });
    let attach = {
        let client = s.client.clone();
        tokio::spawn(async move { client.ensure().await })
    };
    assert!(joined("attach", attach).await.is_err());
    let version = |status: ConnectionStatus| matches!(status.band, Some(Band::Version { .. }));
    wait_for("band ②", || version(s.client.status())).await;

    // No daemon by the time of the stop: the re-add finds nothing.
    *s.source.current.lock().unwrap() = Some(Current::Absent);
    let end = joined("stop", s.stop_with_reason(StopPlan::User)).await;
    assert_eq!(
        (end.outcome, end.reason),
        (StopOutcome::NoTarget, Some(StopReason::NoRecord))
    );
    // U4: ① can only come from the re-add's failure — the ② handle is parked and never redials.
    wait_for("band ① after the re-add", || {
        s.client.status().band == BAND_ONE
    })
    .await;
}

/// Scripts the link's daemon as an older version — an attach shows ② — then makes what the re-add after the stop
/// finds a live daemon on the link network, which answers only when the test lets it.
async fn version_band_then_a_live_daemon_for_the_re_add(s: &Stage) {
    *s.source.current.lock().unwrap() = Some(Current::VersionMismatch {
        daemon: PROTOCOL_VERSION - 1,
        pid: PID,
        start_time: 0,
    });
    let attach = {
        let client = s.client.clone();
        tokio::spawn(async move { client.ensure().await })
    };
    assert!(joined("attach", attach).await.is_err());
    wait_for("band ②", || {
        matches!(s.client.status().band, Some(Band::Version { .. }))
    })
    .await;
    *s.source.current.lock().unwrap() = None;
    *s.source.found.lock().unwrap() = Some(Endpoint {
        key: DaemonKey::new("local"),
        address: Address::new("ws://127.0.0.1:5123"),
        credential: Secret::new("t".repeat(64)),
    });
}

// A stop that did not end the daemon (`timeout`) keeps the shown band until the re-add's first answer.
#[tokio::test]
async fn a_stop_that_times_out_keeps_the_shown_band_until_the_re_add_answers() {
    let s = Stage::new(Record::new([record(PID, 0, PROTOCOL_VERSION)]));
    version_band_then_a_live_daemon_for_the_re_add(&s).await;
    s.daemons.fail_next(1);
    let end = joined("stop", s.stop_with_reason(StopPlan::User)).await;
    assert_eq!(
        (end.outcome, end.reason),
        (StopOutcome::Timeout, Some(StopReason::NoIdentity))
    );
    let version = |status: ConnectionStatus| matches!(status.band, Some(Band::Version { .. }));
    // The re-add has dialed and not answered — ② still shows.
    wait_for("the re-add dials", || s.link_net.open_endpoints() > 0).await;
    let_it_run().await;
    assert!(version(s.client.status()), "{:?}", s.client.status());
}

/// Lets a running `User` stop end the daemon: `Hello`, `StopDaemon`, the daemon gone.
async fn the_daemon_stops(s: &Stage) {
    let daemon = s.daemon().await;
    s.hello_then_stop_daemon(&daemon).await;
    s.target.set(Presence::Gone);
    daemon.close();
}

// A stop sets no band (user decision 2026-11-09): a stop that ends the daemon emits nothing itself — ② stays after the
//   flight — and the re-add's first failure shows ①.
#[tokio::test]
async fn a_stop_that_ends_the_daemon_sets_no_band_and_the_re_adds_first_failure_shows_band_one() {
    let s = Stage::new(Record::new([our_daemon()]));
    version_band_then_a_live_daemon_for_the_re_add(&s).await;
    let before = s.statuses();
    let task = s.stop_with_reason(StopPlan::User);
    the_daemon_stops(&s).await;
    let end = joined("stop", task).await;
    assert_eq!((end.outcome, end.reason), (StopOutcome::Closed, None));

    let mut link = None;
    wait_for("the re-add dials", || {
        link = link.take().or_else(|| s.link_net.accept());
        link.is_some()
    })
    .await;
    let_it_run().await;
    assert_eq!(s.statuses(), before, "the stop emitted nothing");
    assert!(
        matches!(s.client.status().band, Some(Band::Version { .. })),
        "{:?}",
        s.client.status()
    );

    // The re-add's attempt fails before `Hello`.
    link.unwrap().close();
    wait_for("band ① after the re-add", || {
        s.client.status().band == BAND_ONE
    })
    .await;
    assert_eq!(s.statuses()[before.len()..], ["status:down:not_connected"]);
}
