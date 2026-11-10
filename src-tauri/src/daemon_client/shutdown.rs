//! The one stop path — [`DaemonClient::stop_daemon`] (TRD S21 trd-3-2-u5 §3-3). Tray 「데몬 끄기」 and app exit run it
//! today; the band and the LLM command join in U5b. The wire half is `link::stop`.
//!
//! - ★One flight per client★ (R2): a stop while one runs joins it and gets its outcome; the first caller's plan rules.
//!   An [`StopPlan::Exit`] joiner waits only its own bound.
//! - ★One absolute clock★ (R13): every deadline is measured from flight start — the `daemon.json` read and its retries
//!   included — on the client's one clock. Flight start = the instant the starting caller asked (the blocking wrapper
//!   takes it before its spawn). [`StopPlan::User`] = 10 s graceful + 3 s margin, [`StopPlan::Exit`] = 3 s + 1 s.
//! - Order (R1): the target resolved once (the kill uses this identity, never a re-read) → the stop client → "gone"
//!   polled → the identity-checked kill (`User` only: at once when nothing was sent, at the graceful deadline when it
//!   was; never without a start time) → the attach-only re-add fired, not awaited (not for `Exit`).
//! - ★The shell's link is left alone★ (user decision 2026-11-10): it stays open through the flight, so the daemon's own
//!   shutdown broadcasts (its agents exiting, the emptied agent list) reach the frontend. When the daemon exits the link
//!   sees a normal disconnect and reconnects as always; its first failed reconnect shows band ①. ★Do not close it at
//!   flight start★ — those broadcasts are then lost and the agent tree keeps showing running agents. Accepted instead: a
//!   redial during the flight may reach the exiting daemon, and an attach or a tray 「데몬 켜기」 during the flight is
//!   not held back — a failed redial just shows ①.
//! - The re-add is for a link parked by a terminal refusal (band ② — a version refusal never redials): without it ②
//!   would stay after the old daemon is gone. With a live link it does nothing; with no daemon it fails and the link
//!   keeps retrying in the background (U4 — no "intended stop" latch).
//! - ★Expiry (R19)★: a timer outside the flight task aborts it at the flight deadline. Every side effect (dial ·
//!   `StopDaemon` · kill) checks that deadline before it runs, a `daemon.json` read still running is discarded, and the
//!   supervisor fires the re-add. Joiners and the blocking wrapper get `Timeout`, never a dropped sender.
//! - Outcome (R1 step 6): gone without a kill → `Closed`; gone after a kill (whatever the kill call returned) →
//!   `Killed`; alive or unknown at the end → `Timeout`; nothing to stop → `NoTarget`. An unreadable process ("unknown")
//!   is never reported as gone.
//! - ★A stop sets no band★ (user decision 2026-11-09): only routing does, from the link's own answers.
// ADR-0021
// ADR-0026
// ADR-0343
// ADR-0344

use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use engram_dashboard_base::sync;
use engram_dashboard_platform::process::Presence;
use engram_dashboard_transport::ws::WsDialer;
use engram_dashboard_transport::{with_deadline, Clock, Dialer};
use tokio::runtime::Handle;
use tokio::sync::watch;

use super::connection::LinkControl;
use super::link::stop::{
    resolve_stop_target, send_stop, DaemonFileReader, RecordReader, Resolved, StopDial, StopSend,
    StopTarget,
};
use super::{DaemonClient, CLIENT_THREAD_NAME, DAEMON_MESSAGE_MAX_BYTES};
use crate::discovery::{PidLiveness, RealLiveness};

/// How a stop may end the daemon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopPlan {
    /// A user asked (tray · band · LLM): 10 s graceful, then the identity-checked kill.
    User,
    /// The app exits (ADR-0026): 3 s, never kills — the app exits whatever the outcome.
    Exit,
}

impl StopPlan {
    // ADR-0343 d3 · TRD S21 trd-3-2-u5 D7: 10 s graceful like `docker stop` / supervisord `stopwaitsecs`.
    fn grace(self) -> Duration {
        match self {
            StopPlan::User => Duration::from_secs(10),
            StopPlan::Exit => Duration::from_secs(3),
        }
    }

    /// Room after the graceful deadline for busy-read retries, the kill and the "gone" poll — not a graceful budget.
    fn margin(self) -> Duration {
        match self {
            StopPlan::User => Duration::from_secs(3),
            StopPlan::Exit => Duration::from_secs(1),
        }
    }

    /// From flight start to the flight deadline — the longest a flight lasts.
    pub fn flight_bound(self) -> Duration {
        self.grace() + self.margin()
    }
}

/// Who asked — logged only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopOrigin {
    Tray,
    AppExit,
    /// The `daemon_shutdown` command — the LLM `daemon.stop`.
    Command,
}

/// How a stop ended (the `daemon_shutdown` reply): `closed` — the daemon exited after the stop request · `killed` — it
/// was killed after an identity check (the request could not be sent, or it did not exit in time) · `timeout` — it may still run (no identity, never
/// killed, or the flight deadline passed; see the reason) · `no_target` — there was no daemon this app may stop.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum StopOutcome {
    Closed,
    Killed,
    Timeout,
    NoTarget,
}

/// Why a stop ended `no_target` or `timeout` — the `daemon_shutdown` reply's `reason`. Serialized snake_case.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum StopReason {
    /// `no_target`: no `daemon.json`.
    NoRecord,
    /// `no_target`: `daemon.json` could not be read.
    RecordUnreadable,
    /// `no_target`: `daemon.json` does not parse.
    RecordBroken,
    /// `no_target`: `daemon.json` stayed held by another process for every read.
    RecordBusy,
    /// `no_target`: the record names a pid that is not alive.
    DeadPid,
    /// `no_target`: the daemon is newer than this app — this app does not stop it.
    NewerDaemon,
    /// `timeout`: the daemon did not stop and its record has no start time, so it is not killed (TRD R1).
    NoIdentity,
    /// `timeout`: the daemon was not seen gone — still running or unreadable — and was not (or could not be) ended:
    /// app exit never kills, or it outlived the kill.
    StillRunning,
    /// `timeout`: the flight deadline passed before the stop finished (or before the kill could run).
    FlightExpired,
    /// `timeout`: the stop task failed.
    StopFailed,
}

/// How one flight ended — `reason` is `None` where the outcome says it all (`closed` · `killed`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StopResult {
    pub outcome: StopOutcome,
    pub reason: Option<StopReason>,
}

impl StopResult {
    fn closed() -> Self {
        Self {
            outcome: StopOutcome::Closed,
            reason: None,
        }
    }

    fn timeout(reason: StopReason) -> Self {
        Self {
            outcome: StopOutcome::Timeout,
            reason: Some(reason),
        }
    }
}

/// Whether the target process still runs, and ending it — the OS behind a seam (ADR-0012).
pub trait TargetProcess: Send + Sync {
    /// One handle, three answers (R4) — `platform::process::presence`.
    fn presence(&self, pid: u32, start_time: u64) -> Presence;
    /// `platform::process::terminate_if_same`.
    fn terminate_if_same(&self, pid: u32, start_time: u64) -> std::io::Result<bool>;
}

/// The real one.
pub struct OsProcess;

impl TargetProcess for OsProcess {
    fn presence(&self, pid: u32, start_time: u64) -> Presence {
        engram_dashboard_platform::process::presence(pid, start_time)
    }

    fn terminate_if_same(&self, pid: u32, start_time: u64) -> std::io::Result<bool> {
        engram_dashboard_platform::process::terminate_if_same(pid, start_time)
    }
}

/// Everything a stop flight touches outside memory. ★No clock★ — a client's flight runs on the client's one clock, its
/// link's (`Stopper`).
#[derive(Clone)]
pub struct StopPorts {
    pub reader: Arc<dyn RecordReader>,
    pub liveness: Arc<dyn PidLiveness + Send + Sync>,
    pub process: Arc<dyn TargetProcess>,
    pub dialer: Arc<dyn Dialer>,
}

impl StopPorts {
    /// The local daemon of `data_dir` — production's ports: `daemon.json` there, the OS, the ws dialer.
    pub fn local(data_dir: &Path) -> Self {
        Self {
            reader: Arc::new(DaemonFileReader::new(data_dir)),
            liveness: Arc::new(RealLiveness),
            process: Arc::new(OsProcess),
            dialer: Arc::new(WsDialer::new(DAEMON_MESSAGE_MAX_BYTES)),
        }
    }
}

/// Reads of a busy `daemon.json` per flight, and the gap between them (TRD R1 제안 — 5 × 100 ms).
const BUSY_READS: u32 = 5;
const BUSY_GAP: Duration = Duration::from_millis(100);
/// Kept off the dial bound so the stop still has time to finish after a `Hello` at the last moment.
const DIAL_RESERVE: Duration = Duration::from_millis(500);
/// How often "gone" is asked while waiting.
const GONE_POLL: Duration = Duration::from_millis(100);

/// One stop flight without its single-flight bookkeeping: steps 2–6 of R1 against `ports`, every deadline measured from
/// `started` (the flight's start on `clock`). The caller owns the re-add and expiry
/// ([`DaemonClient::stop_daemon`]); a real-daemon test drives this directly (`tests/stop_smoke.rs`).
pub async fn stop_flight(
    ports: &StopPorts,
    clock: &Arc<dyn Clock>,
    plan: StopPlan,
    started: Instant,
) -> StopResult {
    let graceful = started + plan.grace();
    let deadline = graceful + plan.margin();
    let target = match resolve(ports, clock.as_ref(), deadline).await {
        Ok(target) => target,
        Err(why) => {
            return StopResult {
                outcome: StopOutcome::NoTarget,
                reason: Some(why),
            }
        }
    };
    let presence = match dial(ports, clock, &target, graceful, deadline).await {
        // Finish as soon as it is gone; the kill waits for the graceful deadline.
        StopSend::Sent { .. } => wait_gone(ports, clock.as_ref(), &target, graceful).await,
        // Nothing graceful is in progress — no reason to wait.
        StopSend::NothingSent => ports.process.presence(target.pid, target.start_time),
    };
    if presence == Presence::Gone {
        tracing::info!(pid = target.pid, "the daemon stopped");
        return StopResult::closed();
    }
    match plan {
        StopPlan::Exit => {
            tracing::info!(
                pid = target.pid,
                ?presence,
                "app exit: the daemon is still up — not killing"
            );
            StopResult::timeout(StopReason::StillRunning)
        }
        StopPlan::User => kill(ports, clock.as_ref(), &target, deadline).await,
    }
}

/// The target from `daemon.json`, a busy record read again within the flight deadline. `Err` = nothing to stop, and why.
async fn resolve(
    ports: &StopPorts,
    clock: &dyn Clock,
    deadline: Instant,
) -> Result<StopTarget, StopReason> {
    for read in 1..=BUSY_READS {
        let (reader, liveness) = (ports.reader.clone(), ports.liveness.clone());
        let resolved = tokio::task::spawn_blocking(move || {
            resolve_stop_target(reader.as_ref(), liveness.as_ref())
        })
        .await;
        match resolved {
            Ok(Resolved::Target(target)) => return Ok(target),
            Ok(Resolved::NoTarget(reason, why)) => {
                tracing::info!(%why, ?reason, "no daemon to stop");
                return Err(reason);
            }
            Ok(Resolved::Busy) => {}
            Err(e) => {
                tracing::warn!(error = %e, "reading the daemon record failed — no daemon to stop");
                return Err(StopReason::RecordUnreadable);
            }
        }
        if read == BUSY_READS || clock.now() + BUSY_GAP >= deadline {
            break;
        }
        clock.sleep(BUSY_GAP).await;
    }
    tracing::warn!("the daemon record stayed held by another process — no daemon to stop");
    Err(StopReason::RecordBusy)
}

/// The stop client's one dial, bounded by the graceful time left minus [`DIAL_RESERVE`] (R13). No time left = nothing
/// sent — the graceful deadline comes before the flight deadline, so this is also the dial's flight-deadline gate.
async fn dial(
    ports: &StopPorts,
    clock: &Arc<dyn Clock>,
    target: &StopTarget,
    graceful: Instant,
    deadline: Instant,
) -> StopSend {
    let dial_bound = graceful
        .saturating_duration_since(clock.now())
        .saturating_sub(DIAL_RESERVE);
    if dial_bound.is_zero() {
        tracing::warn!(
            pid = target.pid,
            "no time left to dial the daemon — nothing sent"
        );
        return StopSend::NothingSent;
    }
    send_stop(
        ports.dialer.clone(),
        clock.clone(),
        target,
        StopDial {
            dial_bound,
            read_until: graceful,
            notify_before: deadline,
        },
    )
    .await
}

/// Asks "gone" until it is or `until` passes — the last answer.
async fn wait_gone(
    ports: &StopPorts,
    clock: &dyn Clock,
    target: &StopTarget,
    until: Instant,
) -> Presence {
    loop {
        let presence = ports.process.presence(target.pid, target.start_time);
        let left = until.saturating_duration_since(clock.now());
        if presence == Presence::Gone || left.is_zero() {
            return presence;
        }
        clock.sleep(left.min(GONE_POLL)).await;
    }
}

/// The identity-checked kill and the wait for its effect (R1 step 5).
async fn kill(
    ports: &StopPorts,
    clock: &dyn Clock,
    target: &StopTarget,
    deadline: Instant,
) -> StopResult {
    if target.start_time == 0 {
        tracing::warn!(
            pid = target.pid,
            reason = "no_identity",
            "the daemon did not stop and its record has no start time — not killing"
        );
        return StopResult::timeout(StopReason::NoIdentity);
    }
    if clock.now() >= deadline {
        tracing::warn!(pid = target.pid, "the flight deadline passed — not killing");
        return StopResult::timeout(StopReason::FlightExpired);
    }
    match ports
        .process
        .terminate_if_same(target.pid, target.start_time)
    {
        Ok(true) => tracing::warn!(pid = target.pid, "the daemon did not stop — terminated it"),
        Ok(false) => tracing::info!(
            pid = target.pid,
            "the daemon was no longer that process at the kill"
        ),
        Err(e) => tracing::warn!(pid = target.pid, error = %e, "terminating the daemon failed"),
    }
    match wait_gone(ports, clock, target, deadline).await {
        Presence::Gone => StopResult {
            outcome: StopOutcome::Killed,
            reason: None,
        },
        presence => {
            tracing::warn!(
                pid = target.pid,
                ?presence,
                "the daemon is still up after the kill"
            );
            StopResult::timeout(StopReason::StillRunning)
        }
    }
}

/// Test-only: a step run once where [`Self::fire`] is called — so a test lands its step exactly there.
#[cfg(test)]
#[derive(Default)]
pub(crate) struct TestHook(Mutex<Option<Box<dyn FnOnce() + Send>>>);

#[cfg(test)]
impl TestHook {
    pub(crate) fn arm(&self, step: impl FnOnce() + Send + 'static) {
        *sync::lock(&self.0) = Some(Box::new(step));
    }

    fn fire(&self) {
        // Run outside the lock.
        let step = sync::lock(&self.0).take();
        if let Some(step) = step {
            step();
        }
    }
}

/// The single flight of one client — see the module doc.
pub(crate) struct Stopper {
    /// The client's runtime — flights and their supervisors run here whoever calls.
    rt: Handle,
    ports: StopPorts,
    link: LinkControl,
    /// The client's one clock — its link's.
    clock: Arc<dyn Clock>,
    flight: Mutex<Option<Flight>>,
    next_flight: AtomicU64,
    /// [`DaemonClient::stop_daemon_blocking`]: the flight start is taken, the stop not yet spawned.
    #[cfg(test)]
    pub(crate) blocking_start_taken: TestHook,
}

struct Flight {
    id: u64,
    /// `None` until the supervisor publishes the end.
    outcome: watch::Receiver<Option<StopResult>>,
}

impl Stopper {
    pub(crate) fn new(
        rt: Handle,
        ports: StopPorts,
        link: LinkControl,
        clock: Arc<dyn Clock>,
    ) -> Arc<Self> {
        Arc::new(Self {
            rt,
            ports,
            link,
            clock,
            flight: Mutex::new(None),
            next_flight: AtomicU64::new(0),
            #[cfg(test)]
            blocking_start_taken: TestHook::default(),
        })
    }

    /// The client's clock — flights and their deadlines run on it.
    pub(crate) fn clock(&self) -> &Arc<dyn Clock> {
        &self.clock
    }

    /// Joins the flight in progress or starts one, then waits for its end — an `Exit` joiner at most its own bound.
    /// `started` = when the caller asked, on [`Self::clock`]: a flight this call starts measures its deadlines from it,
    /// and an `Exit` joiner its own bound. With no flight to join and its bound already over, it starts nothing and
    /// returns `Timeout`.
    pub(crate) async fn stop(
        self: &Arc<Self>,
        plan: StopPlan,
        origin: StopOrigin,
        started: Instant,
    ) -> StopResult {
        let (mut outcome, joined) = self.join_or_start(plan, origin, started);
        let gave_up = || StopResult::timeout(StopReason::FlightExpired);
        let wait = async move {
            loop {
                if let Some(done) = *outcome.borrow_and_update() {
                    return done;
                }
                if outcome.changed().await.is_err() {
                    return gave_up();
                }
            }
        };
        if joined && plan == StopPlan::Exit {
            let left =
                (started + plan.flight_bound()).saturating_duration_since(self.clock().now());
            with_deadline(self.clock().as_ref(), left, wait)
                .await
                .unwrap_or_else(|_| {
                    tracing::info!("app exit: stopped waiting for the daemon stop in flight");
                    gave_up()
                })
        } else {
            wait.await
        }
    }

    fn join_or_start(
        self: &Arc<Self>,
        plan: StopPlan,
        origin: StopOrigin,
        started: Instant,
    ) -> (watch::Receiver<Option<StopResult>>, bool) {
        let mut slot = sync::lock(&self.flight);
        if let Some(flight) = slot.as_ref() {
            tracing::info!(
                ?origin,
                ?plan,
                flight = flight.id,
                "a daemon stop is in flight — joining it"
            );
            return (flight.outcome.clone(), true);
        }
        let deadline = started + plan.flight_bound();
        if self.clock().now() >= deadline {
            // The caller's task ran only after its whole bound (a stalled runtime) — its wait is over, so nothing starts,
            //   not even the re-add.
            tracing::warn!(
                ?origin,
                ?plan,
                "the daemon stop request is past its flight deadline — not stopping"
            );
            let end = StopResult::timeout(StopReason::FlightExpired);
            return (watch::channel(Some(end)).1, false);
        }
        let id = self.next_flight.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = watch::channel(None);
        *slot = Some(Flight {
            id,
            outcome: rx.clone(),
        });
        drop(slot);
        tracing::info!(?origin, ?plan, flight = id, "stopping the daemon");
        let me = self.clone();
        self.rt
            .spawn(async move { me.supervise(id, plan, started, deadline, tx).await });
        (rx, false)
    }

    /// Runs the flight under its deadline, then ends the flight: re-add fired, outcome published.
    async fn supervise(
        self: Arc<Self>,
        id: u64,
        plan: StopPlan,
        started: Instant,
        deadline: Instant,
        outcome: watch::Sender<Option<StopResult>>,
    ) {
        let (ports, clock) = (self.ports.clone(), self.clock().clone());
        let mut flight = self
            .rt
            .spawn(async move { stop_flight(&ports, &clock, plan, started).await });
        let expiry = self
            .clock()
            .sleep(deadline.saturating_duration_since(self.clock().now()));
        let done = tokio::select! {
            biased;
            joined = &mut flight => joined.unwrap_or_else(|e| {
                tracing::warn!(flight = id, error = %e, "the daemon stop task failed");
                StopResult::timeout(StopReason::StopFailed)
            }),
            () = expiry => {
                flight.abort();
                tracing::warn!(flight = id, "the daemon stop passed its flight deadline — abandoned");
                StopResult::timeout(StopReason::FlightExpired)
            }
        };
        if plan != StopPlan::Exit {
            self.link.reattach();
        }
        {
            let mut slot = sync::lock(&self.flight);
            if slot.as_ref().is_some_and(|flight| flight.id == id) {
                *slot = None;
            }
        }
        tracing::info!(flight = id, ?plan, outcome = ?done.outcome, reason = ?done.reason, "daemon stop finished");
        let _ = outcome.send(Some(done));
    }
}

impl DaemonClient {
    /// The one stop path — see the module doc. Returns when the flight this call started or joined ends.
    pub async fn stop_daemon(&self, plan: StopPlan, origin: StopOrigin) -> StopOutcome {
        self.stop_daemon_with_reason(plan, origin).await.outcome
    }

    /// [`Self::stop_daemon`] with why it ended so.
    pub async fn stop_daemon_with_reason(&self, plan: StopPlan, origin: StopOrigin) -> StopResult {
        let started = self.stopper.clock().now();
        self.stopper.stop(plan, origin, started).await
    }

    /// [`Self::stop_daemon`] for a caller off the async runtimes (tray worker · `quit_app` on the main thread): runs it on
    /// the client's runtime and waits until the plan's flight deadline (R5). The re-add is fired, never awaited.
    ///
    /// The flight start is taken here, before the spawn, and the wait ends at the deadline measured from it — a slow
    /// spawn eats into the flight's time, never past the wait.
    ///
    /// ★Never call it on a client runtime thread★ — the flight runs on that runtime, and a blocked worker there can stall
    /// the flight it waits for.
    pub fn stop_daemon_blocking(&self, plan: StopPlan, origin: StopOrigin) -> StopOutcome {
        debug_assert_ne!(
            std::thread::current().name(),
            Some(CLIENT_THREAD_NAME),
            "stop_daemon_blocking on the daemon client's own runtime"
        );
        let (tx, rx) = std::sync::mpsc::channel();
        let stopper = self.stopper.clone();
        let started = stopper.clock().now();
        let deadline = started + plan.flight_bound();
        #[cfg(test)]
        stopper.blocking_start_taken.fire();
        self.rt.spawn(async move {
            let _ = tx.send(stopper.stop(plan, origin, started).await.outcome);
        });
        let left = deadline.saturating_duration_since(self.stopper.clock().now());
        rx.recv_timeout(left).unwrap_or_else(|e| {
            tracing::warn!(?plan, error = %e, "stopped waiting for the daemon stop");
            StopOutcome::Timeout
        })
    }

    /// Fires [`Self::ensure`] on the client's runtime and logs a failure — for a caller that does not wait (tray).
    pub fn ensure_in_background(self: &Arc<Self>) {
        let client = self.clone();
        self.rt.spawn(async move {
            if let Err(e) = client.ensure().await {
                tracing::info!(error = %e, "attaching to the daemon did not succeed — the link retries in the background");
            }
        });
    }
}

/// Ports that find nothing to stop — for client tests that never stop a daemon.
#[cfg(test)]
pub(crate) fn inert_ports(dialer: Arc<dyn Dialer>) -> StopPorts {
    use super::link::stop::RecordRead;

    struct NoRecord;
    impl RecordReader for NoRecord {
        fn read(&self) -> RecordRead {
            RecordRead::Absent
        }
    }
    struct NeverDead;
    impl PidLiveness for NeverDead {
        fn is_dead(&self, _pid: u32, _start_time: u64) -> bool {
            false
        }
    }
    struct NoProcess;
    impl TargetProcess for NoProcess {
        fn presence(&self, _pid: u32, _start_time: u64) -> Presence {
            Presence::Unknown
        }
        fn terminate_if_same(&self, _pid: u32, _start_time: u64) -> std::io::Result<bool> {
            Err(std::io::ErrorKind::Unsupported.into())
        }
    }
    StopPorts {
        reader: Arc::new(NoRecord),
        liveness: Arc::new(NeverDead),
        process: Arc::new(NoProcess),
        dialer,
    }
}

#[cfg(test)]
mod tests;
