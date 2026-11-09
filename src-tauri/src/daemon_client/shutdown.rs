//! The one stop path — [`DaemonClient::stop_daemon`] (TRD S21 trd-3-2-u5 §3-3). Tray 「데몬 끄기」 and app exit run it
//! today; the band and the LLM command join in U5b. The wire half is `link::stop`.
//!
//! - ★One flight per client★ (R2): a stop while one runs joins it and gets its outcome; the first caller's plan rules.
//!   An [`StopPlan::Exit`] joiner waits only its own bound.
//! - ★One absolute clock★ (R13): every deadline is measured from flight start — the `daemon.json` read and its retries
//!   included — on the client's one clock. Flight start = the instant the starting caller asked (the blocking wrapper
//!   takes it before its spawn). [`StopPlan::User`] = 10 s graceful + 3 s margin, [`StopPlan::Exit`] = 3 s + 1 s.
//! - Order (R1): stopping mode on → the shell's link closed (no redials during the flight) → the target resolved once
//!   (the kill uses this identity, never a re-read) → the stop client → "gone" polled → the identity-checked kill
//!   (`User` only: at once when nothing was sent, at the graceful deadline when it was; never without a start time) →
//!   stopping mode off → the attach-only re-add fired, not awaited (not for `Exit`). With no daemon that re-add fails
//!   and the link keeps retrying in the background (U4 — no "intended stop" latch).
//! - ★Stopping mode★ ([`StoppingMode`]) lasts until the flight ends or its flight deadline, whichever comes first — past
//!   the deadline it reads as off even if nobody released it. While on, `ensure` answers `HandshakeError::Stopping` and
//!   adds nothing, and `connect` waits it out (`connection.rs`).
//! - ★Expiry (R19)★: a timer outside the flight task aborts it at the flight deadline. Every side effect (dial ·
//!   `StopDaemon` · kill) checks that deadline before it runs, a `daemon.json` read still running is discarded, and the
//!   supervisor releases the mode and fires the re-add. Joiners and the blocking wrapper get `Timeout`, never a dropped
//!   sender.
//! - Outcome (R1 step 6): gone without a kill → `Closed`; gone after a kill (whatever the kill call returned) →
//!   `Killed`; alive or unknown at the end → `Timeout`; nothing to stop → `NoTarget`. An unreadable process ("unknown")
//!   is never reported as gone.
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

    /// From flight start to the flight deadline — the longest a flight (and stopping mode) lasts.
    pub fn flight_bound(self) -> Duration {
        self.grace() + self.margin()
    }
}

/// Who asked — logged only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopOrigin {
    Tray,
    AppExit,
}

/// How a stop ended — see the module doc.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopOutcome {
    Closed,
    Killed,
    Timeout,
    NoTarget,
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

/// Everything a stop flight touches outside memory. ★No clock★ — a client's flight runs on the client's clock, the one
/// its stopping mode is judged on (`Stopper`), so the two can never differ.
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
/// `started` (the flight's start on `clock`). The caller owns stopping mode, the link and expiry
/// ([`DaemonClient::stop_daemon`]); a real-daemon test drives this directly (`tests/stop_smoke.rs`).
pub async fn stop_flight(
    ports: &StopPorts,
    clock: &Arc<dyn Clock>,
    plan: StopPlan,
    started: Instant,
) -> StopOutcome {
    let graceful = started + plan.grace();
    let deadline = graceful + plan.margin();
    let Some(target) = resolve(ports, clock.as_ref(), deadline).await else {
        return StopOutcome::NoTarget;
    };
    let presence = match dial(ports, clock, &target, graceful, deadline).await {
        // Finish as soon as it is gone; the kill waits for the graceful deadline.
        StopSend::Sent { .. } => wait_gone(ports, clock.as_ref(), &target, graceful).await,
        // Nothing graceful is in progress — no reason to wait.
        StopSend::NothingSent => ports.process.presence(target.pid, target.start_time),
    };
    if presence == Presence::Gone {
        tracing::info!(pid = target.pid, "the daemon stopped");
        return StopOutcome::Closed;
    }
    match plan {
        StopPlan::Exit => {
            tracing::info!(
                pid = target.pid,
                ?presence,
                "app exit: the daemon is still up — not killing"
            );
            StopOutcome::Timeout
        }
        StopPlan::User => kill(ports, clock.as_ref(), &target, deadline).await,
    }
}

/// The target from `daemon.json`, a busy record read again within the flight deadline. `None` = nothing to stop.
async fn resolve(ports: &StopPorts, clock: &dyn Clock, deadline: Instant) -> Option<StopTarget> {
    for read in 1..=BUSY_READS {
        let (reader, liveness) = (ports.reader.clone(), ports.liveness.clone());
        let resolved = tokio::task::spawn_blocking(move || {
            resolve_stop_target(reader.as_ref(), liveness.as_ref())
        })
        .await;
        match resolved {
            Ok(Resolved::Target(target)) => return Some(target),
            Ok(Resolved::NoTarget(why)) => {
                tracing::info!(%why, "no daemon to stop");
                return None;
            }
            Ok(Resolved::Busy) => {}
            Err(e) => {
                tracing::warn!(error = %e, "reading the daemon record failed — no daemon to stop");
                return None;
            }
        }
        if read == BUSY_READS || clock.now() + BUSY_GAP >= deadline {
            break;
        }
        clock.sleep(BUSY_GAP).await;
    }
    tracing::warn!("the daemon record stayed held by another process — no daemon to stop");
    None
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
) -> StopOutcome {
    if target.start_time == 0 {
        tracing::warn!(
            pid = target.pid,
            reason = "no_identity",
            "the daemon did not stop and its record has no start time — not killing"
        );
        return StopOutcome::Timeout;
    }
    if clock.now() >= deadline {
        tracing::warn!(pid = target.pid, "the flight deadline passed — not killing");
        return StopOutcome::Timeout;
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
        Presence::Gone => StopOutcome::Killed,
        presence => {
            tracing::warn!(
                pid = target.pid,
                ?presence,
                "the daemon is still up after the kill"
            );
            StopOutcome::Timeout
        }
    }
}

/// See the module doc — shared by the stop path and the link (`LinkClient`), which reads it on `ensure` / `connect`.
/// ★Its clock is the client's one clock★: the link's, and the stop flights run on it too (`Stopper`).
pub(crate) struct StoppingMode {
    clock: Arc<dyn Clock>,
    /// The flight that entered the mode and its flight deadline.
    state: Mutex<Option<(u64, Instant)>>,
    /// Signalled when a flight leaves the mode.
    left: watch::Sender<()>,
    #[cfg(test)]
    pub(crate) race: RaceHook,
}

impl StoppingMode {
    pub(crate) fn new(clock: Arc<dyn Clock>) -> Self {
        Self {
            clock,
            state: Mutex::new(None),
            left: watch::channel(()).0,
            #[cfg(test)]
            race: RaceHook::default(),
        }
    }

    fn enter(&self, flight: u64, until: Instant) {
        *sync::lock(&self.state) = Some((flight, until));
    }

    /// Leaves the mode if `flight` is the one in it — a late leave never ends a newer flight's mode.
    fn leave(&self, flight: u64) {
        {
            let mut state = sync::lock(&self.state);
            if state.is_some_and(|(id, _)| id == flight) {
                *state = None;
            }
        }
        self.left.send_replace(());
    }

    /// A flight is in the mode and its flight deadline has not passed.
    pub(crate) fn active(&self) -> bool {
        self.until().is_some()
    }

    fn until(&self) -> Option<Instant> {
        let until = sync::lock(&self.state).map(|(_, until)| until)?;
        (self.clock.now() < until).then_some(until)
    }

    /// Returns once the mode is off — at most until the flight deadline of the flight in it.
    pub(crate) async fn wait_over(&self) {
        loop {
            // Subscribed before the check, so a leave after it wakes the wait.
            let mut left = self.left.subscribe();
            let Some(until) = self.until() else {
                return;
            };
            let sleep = self
                .clock
                .sleep(until.saturating_duration_since(self.clock.now()));
            tokio::select! {
                _ = left.changed() => {}
                () = sleep => {}
            }
        }
    }
}

/// A point between the two steps of an ordering the stop path relies on (the ★ comments where
/// [`RaceHook::fire`] is called).
#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RacePoint {
    /// `LinkClient::ensure`: the close epoch is read, the mode not yet.
    EnsureEpochRead,
    /// `LinkClient::connect`: the close epoch is read, the mode not yet.
    ConnectEpochRead,
    /// `Stopper::join_or_start`: the mode is on, the link not yet closed. Its step runs holding the `flight` slot lock,
    /// so it must not start or join a stop.
    StopModeEntered,
    /// `DaemonClient::stop_daemon_blocking`: the flight start is taken, the stop not yet spawned.
    BlockingStartTaken,
}

/// Test-only: a step run once when its [`RacePoint`] is reached — so a test lands the racing side exactly between the
/// two steps an ordering keeps apart.
#[cfg(test)]
#[derive(Default)]
pub(crate) struct RaceHook(Mutex<Option<(RacePoint, Box<dyn FnOnce() + Send>)>>);

#[cfg(test)]
impl RaceHook {
    pub(crate) fn arm(&self, at: RacePoint, step: impl FnOnce() + Send + 'static) {
        *sync::lock(&self.0) = Some((at, Box::new(step)));
    }

    pub(crate) fn fire(&self, at: RacePoint) {
        let step = {
            let mut armed = sync::lock(&self.0);
            if armed.as_ref().is_some_and(|(point, _)| *point == at) {
                armed.take().map(|(_, step)| step)
            } else {
                None
            }
        };
        // Run outside the lock — the step may reach another point.
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
    mode: Arc<StoppingMode>,
    flight: Mutex<Option<Flight>>,
    next_flight: AtomicU64,
}

struct Flight {
    id: u64,
    /// `None` until the supervisor publishes the outcome.
    outcome: watch::Receiver<Option<StopOutcome>>,
}

impl Stopper {
    pub(crate) fn new(
        rt: Handle,
        ports: StopPorts,
        link: LinkControl,
        mode: Arc<StoppingMode>,
    ) -> Arc<Self> {
        Arc::new(Self {
            rt,
            ports,
            link,
            mode,
            flight: Mutex::new(None),
            next_flight: AtomicU64::new(0),
        })
    }

    pub(crate) fn mode(&self) -> &Arc<StoppingMode> {
        &self.mode
    }

    /// The client's clock — flights, their deadlines and the mode all run on it.
    pub(crate) fn clock(&self) -> &Arc<dyn Clock> {
        &self.mode.clock
    }

    /// Joins the flight in progress or starts one, then waits for its outcome — an `Exit` joiner at most its own bound.
    /// `started` = when the caller asked, on [`Self::clock`]: a flight this call starts measures its deadlines from it,
    /// and an `Exit` joiner its own bound. With no flight to join and its bound already over, it starts nothing and
    /// returns `Timeout`.
    pub(crate) async fn stop(
        self: &Arc<Self>,
        plan: StopPlan,
        origin: StopOrigin,
        started: Instant,
    ) -> StopOutcome {
        let (mut outcome, joined) = self.join_or_start(plan, origin, started);
        let wait = async move {
            loop {
                if let Some(done) = *outcome.borrow_and_update() {
                    return done;
                }
                if outcome.changed().await.is_err() {
                    return StopOutcome::Timeout;
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
                    StopOutcome::Timeout
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
    ) -> (watch::Receiver<Option<StopOutcome>>, bool) {
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
            // The caller's task ran only after its whole bound (a stalled runtime) — its wait is over, so nothing starts:
            //   no mode, no close, no re-add.
            tracing::warn!(
                ?origin,
                ?plan,
                "the daemon stop request is past its flight deadline — not stopping"
            );
            return (watch::channel(Some(StopOutcome::Timeout)).1, false);
        }
        let id = self.next_flight.fetch_add(1, Ordering::Relaxed);
        // ★The mode before the close★ — an `ensure` that read the close epoch after the close sees the mode on.
        self.mode.enter(id, deadline);
        #[cfg(test)]
        self.mode.race.fire(RacePoint::StopModeEntered);
        self.link.close();
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

    /// Runs the flight under its deadline, then ends the flight: mode off, re-add fired, outcome published.
    async fn supervise(
        self: Arc<Self>,
        id: u64,
        plan: StopPlan,
        started: Instant,
        deadline: Instant,
        outcome: watch::Sender<Option<StopOutcome>>,
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
                StopOutcome::Timeout
            }),
            () = expiry => {
                flight.abort();
                tracing::warn!(flight = id, "the daemon stop passed its flight deadline — abandoned");
                StopOutcome::Timeout
            }
        };
        self.mode.leave(id);
        if plan != StopPlan::Exit {
            self.link.reattach();
        }
        {
            let mut slot = sync::lock(&self.flight);
            if slot.as_ref().is_some_and(|flight| flight.id == id) {
                *slot = None;
            }
        }
        tracing::info!(flight = id, ?plan, outcome = ?done, "daemon stop finished");
        let _ = outcome.send(Some(done));
    }
}

impl DaemonClient {
    /// The one stop path — see the module doc. Returns when the flight this call started or joined ends.
    pub async fn stop_daemon(&self, plan: StopPlan, origin: StopOrigin) -> StopOutcome {
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
        stopper.mode().race.fire(RacePoint::BlockingStartTaken);
        self.rt.spawn(async move {
            let _ = tx.send(stopper.stop(plan, origin, started).await);
        });
        let left = deadline.saturating_duration_since(self.stopper.clock().now());
        rx.recv_timeout(left).unwrap_or_else(|e| {
            tracing::warn!(?plan, error = %e, "stopped waiting for the daemon stop");
            StopOutcome::Timeout
        })
    }

    /// Blocks until no stop is in flight — at most a `User` flight bound (tray 「데몬 켜기」, R10). Same thread rule as
    /// [`Self::stop_daemon_blocking`].
    pub fn wait_stop_over_blocking(&self) {
        debug_assert_ne!(
            std::thread::current().name(),
            Some(CLIENT_THREAD_NAME),
            "wait_stop_over_blocking on the daemon client's own runtime"
        );
        let (tx, rx) = std::sync::mpsc::channel();
        let mode = self.stopper.mode().clone();
        self.rt.spawn(async move {
            mode.wait_over().await;
            let _ = tx.send(());
        });
        if rx.recv_timeout(StopPlan::User.flight_bound()).is_err() {
            tracing::warn!("stopped waiting for the daemon stop in flight");
        }
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
