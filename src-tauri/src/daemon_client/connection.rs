//! The daemon connection driver on the transport crate (TRD S21 trd-3 §6-2 U3).
//!
//! Every `DaemonClient` runs on it: production (`DaemonClient::new_real_with_owned_runtime`) on the ws dialer and the
//! system clock; tests (`DaemonClient::new_on_link`) on a memory network and a manual clock.
//!
//! One actor task per client owns the routing: the registry's dialer registrations, [`RouteState`] and the inbox pumps
//! all meet on it, and it is the only place that applies what [`route`] returns (connection state · frontend
//! broadcasts · replay events · daemon command requests · usage snapshots · output frames). Callers' round trips run on
//! the callers' tasks through a registry clone, never on the actor.
//!
//! - ★The actor never awaits anything that waits on an inbox★ (transport invariant 1, transitively — see `link::pump`):
//!   no `Peer::request`, no `Peer::close_after_flush` future, no `Registry::close_all`. Its awaits are its two
//!   channels and the replay deadline timer on the injected clock; it writes with `notify_in`, and the one request it
//!   makes (the registration) it only queues — the reply is awaited on another task.
//! - ★Every add is one actor step★ — `register` (a fresh dial address) + `adopt` + `Registry::add` + `spawn_pump`,
//!   with `adopt`'s output applied; a re-add of the key takes the same step. Every `Ended` gives its dial registration
//!   back, current or replaced handle.
//! - ★Routing is the only source of connection state★ — `close` removes the peer and shows nothing itself; the
//!   `Closed` that follows shows `Down`.
//! - ★Retries never run out★ — network failures retry without limit on the capped backoff ([`link_policy`] —
//!   ADR-0346). ★No refusal is retried★: terminal ones stop in the transport (the dialer's version check ·
//!   `ShellHandshake`), and a refusal the transport would retry (a refusal close code other than 4001 — `RejectKind::of`;
//!   1000 · 1001 · 4000 are not refusals and retry as network failures) stops here: the actor removes the peer when it
//!   routes that `ConnectFailed`. ★Best effort★ — the transport has already scheduled its next attempt by then, so an
//!   actor delayed past the first backoff lets one more dial out before the stop lands. A first connect shows its first
//!   failure as `Down` at once and keeps retrying in the background (`link::route`).
//! - ★Every decision that a close must order runs on the actor★ — "already connected" and "was this connect overtaken
//!   by a close" are answered there, in channel order after any earlier `Close`. A close bumps the close epoch at once;
//!   a connect / ensure carries the epoch it began under, and an add older than the last close builds nothing (a close
//!   during `connect`'s daemon search wins).
//! - `connect` / `ensure` resolve on their handle's first answer — `Connected` (once it is shown: status watch and
//!   frontend event first — the frontend relies on that order), or its first failure; no live daemon (no or an unusable record)
//!   answers `NoLiveDaemon`. A connect that a re-add overtakes gets the new handle's answer; a close before the answer
//!   ends it with `ClosedBeforeHello`.
//! - ★While a daemon stop is in flight★ (its stopping mode — `shutdown`), `ensure` answers `HandshakeError::Stopping`
//!   and builds no handle, and `connect` first waits for the stop to end and only then reads its close epoch — the
//!   stop's own close never overtakes it (TRD S21 trd-3-2-u5 R2 · R16).
//! - ★The boot hold★ (TRD S21 trd-3-2-u5 R20 · R24): the production client holds every band of a never-connected
//!   handle from its construction until the main window's boot start answers — its failed `daemon_start`, or its
//!   `daemon_connect` returning (`commands::discovery` gates both on the window) — or any handle connects, or a bound
//!   passes: 60 s from construction until main's daemon search begins, then that search's timeout +
//!   [`BOOT_START_MARGIN`] from its start instead. Its end is an actor step that shows what each current handle's last
//!   answer calls for (`RouteState::end_boot_start_hold`).
//! - ★Subscribe · Unsubscribe · RequestReplay go through this one actor, in order, onto one peer queue★ (TRD §6-2 U3 —
//!   master re-check A1). A replay request checks the window routing when the actor takes it, so an `Unsubscribe`
//!   queued before it is written first and the check sees the routing that `Unsubscribe` came from. The `Subscribe`
//!   of a merged request is checked again when the one before it resolves.
//! - ★Replay flights belong to the routed connection★ (`LinkUp` … `LinkDown`): every write goes out with `notify_in`
//!   on that connection only, and `LinkDown` clears the flights. A replay `Subscribe` that cannot be written, or an
//!   output frame too short to read, drops that connection (`Peer::drop_link_in` — bound to it, back to the backoff);
//!   the actor clears its flights at once and handles none of its later output frames or driver events (its
//!   broadcasts and lifecycle events still pass).
//!
//! - ★A connection is opened by the actor before any caller writes on it★ — on its `LinkUp` the actor queues the
//!   usage subscribe and the command registration (`RegisterCommands`, every connect — TRD §3-7 clause 1), then
//!   publishes the generation as opened, with the close epoch of its handle. Callers bind their writes to the
//!   connection live when they were called and write only once it is published, never on "the peer is live" alone: a
//!   round trip that comes while that connection is live but not yet opened waits for the publish (bounded by the
//!   request timeout) and fails if the connection ends meanwhile — it never moves on to a later connection (ADR-0195);
//!   a fire-and-forget then waits in the gate's pending list, which the actor writes out when that connection opens
//!   (older ones dropped). So a command sent while a connection opens goes out after that connection's on-open
//!   writes. A publish older than a close the caller saw counts as closed, so `close()` followed by a write fails at
//!   once.
//! - ★The gate★ (`Gate` — one leaf lock shared by the callers and the actor) holds the opened connection and the
//!   pending fire-and-forgets. A caller writes a fire-and-forget on an opened connection while holding it; the actor
//!   writes the pending ones and sets the opened connection while holding it; a round trip writes only after it saw
//!   the opened connection there. So ★one caller's fire-and-forget then round trip keep their order, at all times★.
//!   ★There is no order across callers★, and none between fire-and-forgets and Subscribe · Unsubscribe ·
//!   RequestReplay (those go through the actor — A1 above; fire-and-forgets go to the peer directly, as in 3-2 U3b).
//! - ★The registration's reply is never awaited on the actor★ — its request is queued in the `LinkUp` step and the
//!   reply is read on a spawned task, which logs it and shows a daemon refusal in the refusal box.
//! - ★A daemon command request is applied off the actor★ (`InboundReceiver::accept`) and its outcome goes back only on
//!   the connection it came on (`send_command_outcome` with the request's generation).
//! - ★Usage subscribes are read and queued under the usage lock★ — the actor's (open · nudge · snapshot resend) and a
//!   caller's ⟳ pre-subscribe alike, so the order the interest is read in is the order the subscribes reach the peer
//!   queue and the last one on the wire is the latest interest (TRD S21 usage-limit-slot §1-7). The usage socket
//!   epoch is the routed generation; it does not rewind across re-adds (`Registry::add`).
// ADR-0046
// ADR-0310
// ADR-0346

use std::collections::{BTreeSet, HashMap, VecDeque};
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::task::{Context, Poll, Waker};
use std::time::Duration;

use engram_dashboard_base::sync;
use engram_dashboard_command::{CommandEnvelope, CommandError, CommandReply as BusReply};
use engram_dashboard_protocol::{
    AgentBackendKind, AgentCommand, AgentEvent, AgentId, UsageLimitSnapshot,
};
use engram_dashboard_transport::{
    mask_reason, Address, Clock, ConnectFailure, Dialer, DisconnectCause, Generation, Incoming,
    Peer, PeerState, Policy, Registry, RejectKind, RequestError, TransportEvent,
};
use futures_util::future::BoxFuture;
use tokio::runtime::Handle;
use tokio::sync::{mpsc, oneshot, watch};

use super::events::DaemonEvents;
use super::frame_relay::{relay_binary_frame, FrameRelay, UnknownTagLog};
use super::inbound::{registration_command, InboundSlot};
use super::link::dialer::{no_live_daemon_cause, LocalDaemonDialer};
use super::link::endpoint::{Endpoint, EndpointSource};
use super::link::errors::{
    request_error_text, require_live, send_error_text, SEND_FAILED_PREFIX, UNSENT_ON_DISCONNECT,
};
use super::link::pump::{spawn_pump, Routable};
use super::link::route::{
    emit_broadcast, refusal_message, route, route_end, send_command_outcome, LinkNotice,
    RouteState, Routed,
};
use super::link::wire::{ShellIn, ShellWire};
use super::link::{DaemonKey, HandleToken};
use super::protocol_state::{self, CommandFailure, SubState};
use super::refusal::{RefusalAlerts, RefusalSite};
use super::replay_events::{
    apply_replay_event, known_epoch, ReplayFollowUp, REPLAY_DEADLINE, REPLAY_DEADLINE_TICK,
};
use super::replay_flight::{encode_marker_frame, ReplayFlightSet};
#[cfg(test)]
use super::shutdown::RacePoint;
use super::shutdown::StoppingMode;
use super::{
    AgentNamesSink, ConnectionStatus, SharedUsageInterest, CHANNEL_GONE, DEFAULT_CONNECT_TIMEOUT,
    NOT_CONNECTED, NO_REQUEST_ID, REPLAY_UNSENT,
};
use crate::output_channel::{send_to_windows, WindowChannelRegistry};
use crate::output_router::{OutputRouter, WindowLabel};

/// Why a `connect` / `ensure` failed — its `Display` is the text callers get.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HandshakeError {
    /// `connect`'s daemon search (which may start a daemon) failed — carries its error (no token).
    Discovery(String),
    /// No live daemon to attach to — no endpoint record, or one that cannot be used. Carries the cause (a local
    /// diagnostic, no file content). The text names the cause and asks whether the daemon runs — it does not tell the
    /// caller to start one (ADR-0021: attaching never spawns; the caller decides).
    NoLiveDaemon(String),
    /// The daemon could not be reached (a network-class failure).
    Connect(String),
    /// The daemon refused the handshake (token · version · identity).
    AuthRejected(String),
    /// A close came before `Hello`.
    ClosedBeforeHello,
    /// The driver ended or a close overtook the call before its first answer.
    TaskGone,
    /// An attach-only connect while a daemon stop is in flight — nothing was added (TRD S21 trd-3-2-u5 R2).
    Stopping,
}

impl std::fmt::Display for HandshakeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HandshakeError::Discovery(m) => write!(f, "daemon discovery 실패: {m}"),
            HandshakeError::NoLiveDaemon(cause) => {
                write!(f, "no live daemon ({cause}) — is the daemon running?")
            }
            HandshakeError::Connect(m) => write!(f, "daemon websocket 접속 실패: {m}"),
            HandshakeError::AuthRejected(m) => write!(f, "daemon auth failed: {m}"),
            HandshakeError::ClosedBeforeHello => {
                write!(f, "daemon websocket closed before handshake")
            }
            HandshakeError::TaskGone => write!(f, "연결 task 가 핸드셰이크 전 종료됨"),
            HandshakeError::Stopping => write!(f, "the daemon is being stopped — not attaching"),
        }
    }
}

impl std::error::Error for HandshakeError {}

/// The one daemon of stage 3 (D4 ① — the registry is built for N).
pub(super) const DAEMON: &str = "local";

/// Room in the pumps → actor channel. A full channel is backpressure, not loss (`link::pump`).
const ROUTABLE_QUEUE: usize = 256;

/// The boot hold's bound from the client's construction — "the main window never booted" (TRD S21 trd-3-2-u5 R20).
const BOOT_START_CAP: Duration = Duration::from_secs(60);
/// Added to the main window's `daemon_start` search timeout for the boot hold's short bound — covers the `connect` that
/// follows the search. The bound is armed once the search holds the ensure lock, so another window's search is not in
/// it. Main's own value — the TRD sets none.
const BOOT_START_MARGIN: Duration = Duration::from_secs(5);

/// Fire-and-forget commands pending in the gate for a connection not opened yet. Past it the caller drops the command
/// (logged).
const QUEUED_FIRES_MAX: usize = 512;

/// A connection the actor opened, and the close epoch its handle was added under — a caller that read a later epoch
/// (a close came since) does not write on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Opened {
    generation: Generation,
    closes: u64,
}

/// The callers' and the actor's shared view of the opened connection (module doc). ★A leaf lock★ — no other lock is
/// taken while it is held (the registry and the usage lock are read before it); under it there are only these fields
/// and non-blocking `Peer::notify_in` writes. Logging happens after it is released.
#[derive(Default)]
struct Gate {
    /// The connection the actor opened — its on-open writes and its pending fire-and-forgets are queued on the peer.
    opened: Option<Opened>,
    /// Fire-and-forget commands for a live connection the actor has not opened yet, in the order sent, each with that
    /// connection's generation.
    pending: VecDeque<(Generation, AgentCommand)>,
    /// The close epoch the current handle was added under — set before that handle's peer exists. A caller that read a
    /// newer epoch saw a close after it: its fire-and-forget is dropped, opened or not.
    handle_closes: u64,
}

fn lock_gate(gate: &Mutex<Gate>) -> MutexGuard<'_, Gate> {
    sync::lock(gate)
}

/// The transport policy of the shell's daemon link: the crate defaults (TRD §7 — every other value there is the
/// default), except two. Network-class failures retry without limit (ADR-0346 decision 1 — production never reaches
/// `GaveUp(BudgetExhausted)`), and a reply is awaited [`REQUEST_TIMEOUT`] — one value for every round trip, the
/// registration and the wait for a connection to open included.
pub(crate) fn link_policy() -> Policy {
    let mut policy = Policy::default();
    policy.reconnect.max_attempts = u32::MAX;
    policy.request_timeout = REQUEST_TIMEOUT;
    policy
}

// ADR-0181 · ADR-0310 (decision 21, D15 ①): one shared request timeout. 30 s, not the crate's 8 s — D15 ① keeps the
//   frontend command path's
//   former 30 s until agent spawn time is measured (ADR-0181 "검증 안 된 것" 1).
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Where the actor hands what it routes.
pub(crate) struct Ports {
    pub events: Arc<dyn DaemonEvents>,
    pub names: Arc<dyn AgentNamesSink>,
    /// Which windows show which agent — the replay request check and every output frame read it.
    pub router: Arc<OutputRouter>,
    /// The windows' output channels. Output frames and replay markers go through the same channel so the frontend
    /// sees them in arrival order (ADR-0046).
    pub windows: WindowChannelRegistry,
    /// The command table slot — the `DaemonClient`'s own, so a table installed after the start is seen (daemon command
    /// requests · the registration on every connect).
    pub inbound: Arc<InboundSlot>,
    /// The refusal box filter — the `DaemonClient`'s own, so its repeat suppression spans connections.
    pub refusals: Arc<RefusalAlerts>,
    /// The usage interest — the `DaemonClient`'s own, which the layout recomputes and the webview pull reads.
    pub usage: SharedUsageInterest,
}

/// The callers' side of the driver — `DaemonClient` holds one.
pub(crate) struct LinkClient {
    key: DaemonKey,
    rt: Handle,
    source: Arc<dyn EndpointSource>,
    registry: Registry<ShellWire>,
    /// The actor's dialer — held here for tests of its registrations.
    #[cfg(test)]
    dialer: Arc<LocalDaemonDialer>,
    /// The pumps' channel into the actor — tests feed a replaced handle's tail through it.
    #[cfg(test)]
    routable_tx: mpsc::Sender<Routable>,
    /// Bumped by every `close` before it queues `Close` (module doc).
    closes: Arc<AtomicU64>,
    cmd_tx: mpsc::UnboundedSender<DriverCmd>,
    status_rx: watch::Receiver<ConnectionStatus>,
    /// Shared with the actor (module doc).
    gate: Arc<Mutex<Gate>>,
    /// Changes whenever the actor changes the gate's opened connection — wakes round trips waiting for it.
    opened_rx: watch::Receiver<()>,
    /// The ⟳ pre-subscribe reads the interest here.
    usage: SharedUsageInterest,
    /// Times the wait for an opened connection.
    clock: Arc<dyn Clock>,
    /// On while a daemon stop is in flight (`shutdown`) — `ensure` adds nothing, `connect` waits.
    stopping: Arc<StoppingMode>,
}

/// Closes and re-adds the link from outside the client — the stop path (`shutdown::Stopper`) holds one. It does not keep
/// the actor alive: after the client is gone both do nothing.
pub(crate) struct LinkControl {
    key: DaemonKey,
    cmd_tx: mpsc::WeakUnboundedSender<DriverCmd>,
    closes: Arc<AtomicU64>,
    stopping: Arc<StoppingMode>,
}

impl LinkControl {
    /// [`LinkClient::close`] for the stop path — not an explicit close: the shown band stays ([`CloseBy::Stop`]).
    pub(crate) fn close(&self) {
        if let Some(cmd_tx) = self.cmd_tx.upgrade() {
            close_link(&self.key, &self.closes, &cmd_tx, CloseBy::Stop);
        }
    }

    /// [`LinkClient::ensure`] without waiting for its answer — nothing while a stop is in flight. That check is
    /// defensive: today's one caller, the stop's supervisor, calls this after its own flight left the mode and while
    /// that flight still holds the single-flight slot, so no newer flight can be in the mode then.
    pub(crate) fn reattach(&self) {
        let closes = self.closes.load(Ordering::SeqCst);
        if self.stopping.active() {
            tracing::debug!(peer = %self.key.peer_id(), "a daemon stop is in flight — not re-attaching");
            return;
        }
        let Some(cmd_tx) = self.cmd_tx.upgrade() else {
            return;
        };
        tracing::info!(peer = %self.key.peer_id(), "re-attaching to the daemon after a stop");
        // Nobody waits for the first answer — the actor ignores a gone caller.
        let (first, _answer) = oneshot::channel();
        let _ = cmd_tx.send(DriverCmd::Add {
            closes,
            found: None,
            first,
        });
    }
}

/// Bump the epoch before queueing `Close` — [`LinkClient::close`] holds why.
fn close_link(
    key: &DaemonKey,
    closes: &AtomicU64,
    cmd_tx: &mpsc::UnboundedSender<DriverCmd>,
    by: CloseBy,
) {
    tracing::info!(peer = %key.peer_id(), ?by, "closing the daemon link");
    closes.fetch_add(1, Ordering::SeqCst);
    let _ = cmd_tx.send(DriverCmd::Close { by });
}

/// Who closes the link — the two differ only in the band (TRD S21 trd-3-2-u5 R7 · R8).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CloseBy {
    /// The frontend's explicit close (`daemon_close`): no band for the rest of the handle.
    Frontend,
    /// The stop path: what is shown stays — a stop sets no band; the re-add's first answer does.
    Stop,
}

/// Nudges the actor's usage interest without keeping the actor alive — the usage shrink timer holds one
/// (`DaemonClient::run_usage_action`). A nudge after the client is gone does nothing.
pub(crate) struct UsageNudger(mpsc::WeakUnboundedSender<DriverCmd>);

impl UsageNudger {
    pub(crate) fn nudge(&self) {
        if let Some(cmd_tx) = self.0.upgrade() {
            let _ = cmd_tx.send(DriverCmd::UsageNudge);
        }
    }
}

enum DriverCmd {
    /// Is the key's current peer live?
    IsLive {
        reply: oneshot::Sender<bool>,
    },
    /// Start a handle for the key — the first one or a re-add — unless the current peer is live (`first` gets `Ok`)
    /// or a close came after `closes` was read (`first` gets an error). Otherwise `first` gets the new handle's first
    /// answer.
    Add {
        closes: u64,
        /// The endpoint `connect`'s daemon search found — the new registration's last-known endpoint.
        found: Option<Endpoint>,
        first: oneshot::Sender<Result<(), HandshakeError>>,
    },
    Close {
        by: CloseBy,
    },
    /// Write `Unsubscribe` on the open connection. With none open there is nothing to undo — the daemon drops a
    /// connection's subscriptions with it.
    Unsubscribe {
        agent_id: AgentId,
    },
    /// Start the agent's replay or join the one in flight (single flight — ADR-0046 M1); `reply` gets the generation
    /// assigned (a joined request writes nothing and still gets one), or [`REPLAY_UNSENT`] / [`NOT_CONNECTED`] when no
    /// generation can be served: no open connection, no window shows the agent, or its `Subscribe` could not be written.
    RequestReplay {
        agent_id: AgentId,
        reply: oneshot::Sender<Result<u64, String>>,
    },
    /// The usage interest may have changed — the set is read when the actor takes this (TRD S21 usage-limit-slot §1-7
    /// "one serial path"), so it carries none.
    UsageNudge,
    /// Start the boot hold (module doc) — bounded by [`BOOT_START_CAP`] from now.
    HoldForBootStart,
    /// The main window's daemon search began with timeout `search`: the hold now ends `search` + [`BOOT_START_MARGIN`]
    /// from now — this bound replaces the 60 s cap. Nothing when no hold is on.
    BoundBootStart {
        search: Duration,
    },
    /// The main window's boot start answered — the hold ends now. Nothing when no hold is on.
    EndBootStartHold {
        why: &'static str,
    },
    /// The key's current handle.
    #[cfg(test)]
    CurrentHandle {
        reply: oneshot::Sender<Option<HandleToken>>,
    },
    /// Whether the boot hold is on.
    #[cfg(test)]
    BootStartHeld {
        reply: oneshot::Sender<bool>,
    },
}

impl LinkClient {
    /// Starts the actor on `rt`. `inner` dials the resolved daemon address (the transport's ws dialer in production,
    /// a memory network in tests); `clock` drives the registry, the failure log and the replay deadlines.
    pub(crate) fn start(
        rt: &Handle,
        inner: Arc<dyn Dialer>,
        source: Arc<dyn EndpointSource>,
        clock: Arc<dyn Clock>,
        ports: Ports,
    ) -> Self {
        let (client, driver, cmd_rx, routable_rx) = Self::build(rt, inner, source, clock, ports);
        rt.spawn(driver.run(cmd_rx, routable_rx));
        client
    }

    /// The client and its actor, not yet running.
    fn build(
        rt: &Handle,
        inner: Arc<dyn Dialer>,
        source: Arc<dyn EndpointSource>,
        clock: Arc<dyn Clock>,
        ports: Ports,
    ) -> (
        Self,
        Driver,
        mpsc::UnboundedReceiver<DriverCmd>,
        mpsc::Receiver<Routable>,
    ) {
        let key = DaemonKey::new(DAEMON);
        let dialer = Arc::new(LocalDaemonDialer::new(inner));
        let registry = Registry::new(
            Arc::new(ShellWire),
            dialer.clone(),
            clock.clone(),
            link_policy(),
        );
        let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
        let (status_tx, status_rx) = watch::channel(ConnectionStatus::DOWN);
        let (routable_tx, routable_rx) = mpsc::channel(ROUTABLE_QUEUE);
        let closes = Arc::new(AtomicU64::new(0));
        let (opened_tx, opened_rx) = watch::channel(());
        let gate = Arc::new(Mutex::new(Gate::default()));
        let stopping = Arc::new(StoppingMode::new(clock.clone()));
        let Ports {
            events,
            names,
            router,
            windows,
            inbound,
            refusals,
            usage,
        } = ports;
        let client = Self {
            key: key.clone(),
            rt: rt.clone(),
            source: source.clone(),
            registry: registry.clone(),
            #[cfg(test)]
            dialer: dialer.clone(),
            #[cfg(test)]
            routable_tx: routable_tx.clone(),
            closes: closes.clone(),
            cmd_tx,
            status_rx,
            gate: gate.clone(),
            opened_rx,
            usage: usage.clone(),
            clock: clock.clone(),
            stopping: stopping.clone(),
        };
        let driver = Driver {
            key,
            source,
            registry,
            dialer,
            route: RouteState::new(clock.clone(), stopping),
            boot_deadline: None,
            clock,
            routable_tx,
            registrations: HashMap::new(),
            current: None,
            first: None,
            closes,
            status_tx,
            events,
            names,
            router,
            windows,
            open: None,
            subs: HashMap::new(),
            flight: ReplayFlightSet::new(REPLAY_DEADLINE),
            unknown_tags: UnknownTagLog::default(),
            dropping: None,
            inbound,
            refusals,
            usage,
            gate,
            opened_tx,
        };
        (client, driver, cmd_rx, routable_rx)
    }

    /// The connection status routing last showed.
    pub(crate) fn status_rx(&self) -> watch::Receiver<ConnectionStatus> {
        self.status_rx.clone()
    }

    /// Starts the boot hold (module doc) — call it at construction, before any window can attach.
    pub(crate) fn hold_bands_for_boot_start(&self) {
        let _ = self.cmd_tx.send(DriverCmd::HoldForBootStart);
    }

    /// The main window's daemon search began with timeout `search` (module doc).
    pub(crate) fn bound_boot_start(&self, search: Duration) {
        let _ = self.cmd_tx.send(DriverCmd::BoundBootStart { search });
    }

    /// The main window's boot start answered (module doc) — `why` is logged.
    pub(crate) fn end_boot_start_hold(&self, why: &'static str) {
        let _ = self.cmd_tx.send(DriverCmd::EndBootStartHold { why });
    }

    /// Whether the boot hold is on, as the actor sees it — every command sent before is applied by then.
    #[cfg(test)]
    pub(crate) async fn boot_start_held(&self) -> bool {
        let (reply, answer) = oneshot::channel();
        self.cmd_tx
            .send(DriverCmd::BootStartHeld { reply })
            .expect("the actor runs");
        answer.await.expect("the actor answers")
    }

    /// The key's current handle, as the actor sees it.
    #[cfg(test)]
    pub(crate) async fn current_handle(&self) -> Option<HandleToken> {
        let (reply, answer) = oneshot::channel();
        self.cmd_tx
            .send(DriverCmd::CurrentHandle { reply })
            .expect("the actor runs");
        answer.await.expect("the actor answers")
    }

    /// Hands `item` of handle `token` to the actor as if its pump had.
    #[cfg(test)]
    pub(crate) async fn feed(&self, token: HandleToken, item: Incoming<ShellWire>) {
        self.routable_tx
            .send(Routable::Item {
                key: self.key.clone(),
                token,
                item,
            })
            .await
            .expect("the actor runs");
    }

    /// The stopping mode this link reads — the stop path enters and leaves it.
    pub(crate) fn stopping(&self) -> Arc<StoppingMode> {
        self.stopping.clone()
    }

    /// A close / re-add handle for the stop path.
    pub(crate) fn control(&self) -> LinkControl {
        LinkControl {
            key: self.key.clone(),
            cmd_tx: self.cmd_tx.downgrade(),
            closes: self.closes.clone(),
            stopping: self.stopping.clone(),
        }
    }

    /// Explicit connect (ADR-0021 — may start a daemon). A failed search still starts the handle — unless a close
    /// overtook this connect, then nothing is built: it reads `daemon.json` on every attempt and connects once a
    /// daemon is there, and the caller gets the search's error.
    ///
    /// ★A daemon stop in flight is waited out first★ (at most its flight deadline), and the close epoch is read only
    /// after that wait (TRD S21 trd-3-2-u5 R16) — the stop's own close is not a close that overtook this connect.
    pub(crate) async fn connect(&self) -> Result<(), HandshakeError> {
        let closes = loop {
            let closes = self.closes.load(Ordering::SeqCst);
            #[cfg(test)]
            self.stopping.race.fire(RacePoint::ConnectEpochRead);
            if !self.stopping.active() {
                break closes;
            }
            tracing::info!(peer = %self.key.peer_id(), "a daemon stop is in flight — connecting after it");
            self.stopping.wait_over().await;
        };
        if self.is_live().await {
            return Ok(());
        }
        tracing::info!(peer = %self.key.peer_id(), "connecting to the daemon (may start it)");
        let source = self.source.clone();
        let searched = match self
            .rt
            .spawn_blocking(move || source.ensure(DEFAULT_CONNECT_TIMEOUT))
            .await
        {
            Ok(found) => found,
            Err(e) => Err(format!("the daemon search task failed: {e}")),
        };
        let first = self.add(closes, searched.as_ref().ok().cloned());
        match searched {
            Ok(_) => first.await,
            Err(e) => {
                tracing::warn!(
                    peer = %self.key.peer_id(),
                    error = %e,
                    "finding or starting the daemon failed — retrying daemon.json in the background"
                );
                Err(HandshakeError::Discovery(e))
            }
        }
    }

    /// Attach-only connect (ADR-0021 — never starts a daemon). While a daemon stop is in flight it answers
    /// [`HandshakeError::Stopping`] and adds nothing (TRD S21 trd-3-2-u5 R2).
    ///
    /// ★The epoch before the mode★ — the stop enters its mode before it closes, so an epoch read after that close
    /// always finds the mode on.
    pub(crate) async fn ensure(&self) -> Result<(), HandshakeError> {
        let closes = self.closes.load(Ordering::SeqCst);
        #[cfg(test)]
        self.stopping.race.fire(RacePoint::EnsureEpochRead);
        if self.stopping.active() {
            tracing::info!(peer = %self.key.peer_id(), "a daemon stop is in flight — not attaching");
            return Err(HandshakeError::Stopping);
        }
        tracing::info!(peer = %self.key.peer_id(), "attaching to a running daemon");
        self.add(closes, None).await
    }

    /// The actor's answer — ordered after every `close` called before.
    async fn is_live(&self) -> bool {
        let (reply, answer) = oneshot::channel();
        if self.cmd_tx.send(DriverCmd::IsLive { reply }).is_err() {
            return false;
        }
        answer.await.unwrap_or(false)
    }

    /// Sends the add now and returns the wait for its first answer.
    fn add(
        &self,
        closes: u64,
        found: Option<Endpoint>,
    ) -> impl std::future::Future<Output = Result<(), HandshakeError>> {
        let (first, answer) = oneshot::channel();
        let sent = self.cmd_tx.send(DriverCmd::Add {
            closes,
            found,
            first,
        });
        async move {
            if sent.is_err() {
                return Err(HandshakeError::TaskGone);
            }
            answer.await.unwrap_or(Err(HandshakeError::TaskGone))
        }
    }

    /// Explicit close — the link does not reconnect. Returns at once; the state follows through routing.
    ///
    /// ★Bump the epoch before queueing `Close`★ — the actor reads the epoch after it takes `Close`, so an `Add` that
    /// captured the old epoch is seen as stale. The other order lets an `Add` queued behind `Close` still read the old
    /// value and revive the link.
    pub(crate) fn close(&self) {
        close_link(&self.key, &self.closes, &self.cmd_tx, CloseBy::Frontend);
    }

    /// One command round trip on the caller's task, on the opened connection ([`Self::opened_generation`]); failure
    /// texts are `link::errors`', plus one for a connection that did not open in time.
    ///
    /// ★⟳ (`RefreshUsageLimits`) is one act with the usage subscribe★ (TRD S21 usage-limit-slot §1-7 "⟳"): the interest
    /// goes out first on the same connection, even when it equals what was sent; when it cannot be queued the refresh
    /// is not sent either.
    pub(crate) async fn send_command(
        &self,
        cmd: AgentCommand,
    ) -> Result<AgentEvent, CommandFailure> {
        use CommandFailure::Local;

        if protocol_state::command_request_id(&cmd).is_none() {
            return Err(Local(NO_REQUEST_ID.to_owned()));
        }
        // Read first — a close called before this call must be seen by every check below.
        let closes = self.closes.load(Ordering::SeqCst);
        let Some(peer) = self.registry.get(self.key.peer_id()) else {
            return Err(Local(NOT_CONNECTED.to_owned()));
        };
        require_live(peer.peer_state()).map_err(Local)?;
        let generation = self.opened_generation(&peer, closes).await.map_err(Local)?;
        if matches!(cmd, AgentCommand::RefreshUsageLimits { .. }) {
            self.write_refresh_subscribe(&peer, generation)
                .map_err(Local)?;
        }
        match peer.request_in(generation, cmd).await {
            Ok(ShellIn::Event(event)) => protocol_state::classify_reply(Ok(event)),
            // `ShellWire::reply_tag` never tags an output frame, so none answers a request.
            Ok(ShellIn::Output(bytes)) => Err(Local(format!(
                "{SEND_FAILED_PREFIX}: an output frame answered the request ({} bytes)",
                bytes.len()
            ))),
            Err(e) => Err(Local(request_error_text(&e))),
        }
    }

    /// A command without a reply, for the connection live now. Never blocks. Not live, or a close came → dropped
    /// (ADR-0195).
    ///
    /// On the opened connection it is written here, under the gate; on a live connection the actor has not opened yet
    /// it waits in the gate's pending list (at most [`QUEUED_FIRES_MAX`], then dropped) and the actor writes it when
    /// that connection opens — in either case ahead of any later round trip of this caller (module doc).
    pub(crate) fn send_fire_and_forget(&self, cmd: AgentCommand) {
        let closes = self.closes.load(Ordering::SeqCst);
        let peer = self
            .registry
            .get(self.key.peer_id())
            .filter(|peer| require_live(peer.peer_state()).is_ok());
        let Some(peer) = peer else {
            tracing::debug!(
                peer = %self.key.peer_id(),
                why = "not live",
                "fire-and-forget: dropped"
            );
            return;
        };
        let generation = peer.generation();
        let outcome = {
            let mut gate = lock_gate(&self.gate);
            match gate.opened {
                _ if gate.handle_closes < closes => FireOutcome::Closed,
                Some(opened) if opened.generation == generation => {
                    FireOutcome::Written(peer.notify_in(generation, cmd))
                }
                _ if gate.pending.len() < QUEUED_FIRES_MAX => {
                    gate.pending.push_back((generation, cmd));
                    FireOutcome::Pending
                }
                _ => FireOutcome::Full,
            }
        };
        match outcome {
            FireOutcome::Written(Ok(())) | FireOutcome::Pending => {}
            FireOutcome::Written(Err(e)) => tracing::debug!(
                peer = %self.key.peer_id(),
                generation = generation.0,
                error = %send_error_text(&e),
                "fire-and-forget: not sent"
            ),
            FireOutcome::Closed => tracing::debug!(
                peer = %self.key.peer_id(),
                why = "closed",
                "fire-and-forget: dropped"
            ),
            FireOutcome::Full => tracing::debug!(
                peer = %self.key.peer_id(),
                generation = generation.0,
                why = "pending full",
                "fire-and-forget: too many waiting for a connection that has not opened yet — dropped"
            ),
        }
    }

    /// The usage interest may have changed — the actor writes the interest of the moment on the open connection, if it
    /// differs from what that connection got. Never blocks; with no connection open it does nothing — the next one
    /// gets the whole interest when it opens.
    pub(crate) fn nudge_usage(&self) {
        let _ = self.cmd_tx.send(DriverCmd::UsageNudge);
    }

    pub(crate) fn usage_nudger(&self) -> UsageNudger {
        UsageNudger(self.cmd_tx.downgrade())
    }

    /// The generation of `peer`'s live connection once the actor opened it — the generation a caller writes on.
    ///
    /// ★Bound to the connection live at the call★ (ADR-0195): waits while the actor has not opened it yet (routing
    /// trails the peer), bounded by the request timeout, and never moves on to a later connection — the next one may
    /// be another daemon. `closes` = the close epoch the caller read on entry. `Err` = the text for a command that
    /// never went out: a close came ([`NOT_CONNECTED`]), the connection ended while waiting
    /// ([`UNSENT_ON_DISCONNECT`]), the actor is gone ([`CHANNEL_GONE`]), or it did not open in time (a text with
    /// [`SEND_FAILED_PREFIX`]).
    async fn opened_generation(
        &self,
        peer: &Peer<ShellWire>,
        closes: u64,
    ) -> Result<Generation, String> {
        let bound = peer.generation();
        // Both receivers before the first check, so a change after it wakes the wait.
        let mut state = peer.state();
        let mut opened = self.opened_rx.clone();
        opened.borrow_and_update();
        if opened_now(peer, bound, closes, &self.gate)? {
            return Ok(bound);
        }
        let wait = async {
            loop {
                tokio::select! {
                    changed = opened.changed() => if changed.is_err() {
                        return Err(CHANNEL_GONE.to_owned());
                    },
                    changed = state.changed() => if changed.is_err() {
                        return Err(UNSENT_ON_DISCONNECT.to_owned());
                    },
                }
                if opened_now(peer, bound, closes, &self.gate)? {
                    return Ok(bound);
                }
            }
        };
        let after = link_policy().request_timeout;
        tokio::select! {
            // An outcome of the connection itself wins over a timeout that is due at the same poll.
            biased;
            opened = wait => opened,
            () = self.clock.sleep(after) => {
                tracing::warn!(
                    peer = %self.key.peer_id(),
                    generation = bound.0,
                    after_secs = after.as_secs(),
                    "a command waited for its daemon connection to open and gave up — not sent"
                );
                Err(format!(
                    "{SEND_FAILED_PREFIX}: the daemon connection did not finish opening within {}s",
                    after.as_secs()
                ))
            }
        }
    }

    /// Queues the ⟳ pre-subscribe on `generation`, read and queued under the usage lock (module doc). Nothing to queue
    /// when the usage state does not see `generation` as its socket (it ended meanwhile).
    // Known residue: the `Err` branch (the subscribe could not be queued, so the ⟳ is not sent and fails with the send
    //   text) has no test — it needs `notify_in` to refuse on a connection that is opened (full queue · lost link).
    fn write_refresh_subscribe(
        &self,
        peer: &Peer<ShellWire>,
        generation: Generation,
    ) -> Result<(), String> {
        let mut usage = self.usage.lock();
        let Some(vendors) = usage.sync(generation.0, true) else {
            return Ok(());
        };
        peer.notify_in(generation, usage_subscribe(vendors))
            .map_err(|e| {
                tracing::debug!(
                    peer = %self.key.peer_id(),
                    generation = generation.0,
                    error = %e,
                    "the usage subscribe ahead of a refresh was not queued — the refresh is not sent"
                );
                send_error_text(&e)
            })
    }

    /// Queues the unsubscribe on the actor, behind every earlier unsubscribe and replay request. Never blocks — the
    /// layout calls it under its lock.
    pub(crate) fn unsubscribe(&self, agent_id: AgentId) {
        let _ = self.cmd_tx.send(DriverCmd::Unsubscribe { agent_id });
    }

    /// The generation the frontend fences this replay with (ADR-0046 M1). `Err` = no open connection
    /// ([`NOT_CONNECTED`]), or no window shows the agent / its `Subscribe` could not be written ([`REPLAY_UNSENT`]).
    pub(crate) async fn request_replay(&self, agent_id: AgentId) -> Result<u64, String> {
        let (reply, answer) = oneshot::channel();
        if self
            .cmd_tx
            .send(DriverCmd::RequestReplay { agent_id, reply })
            .is_err()
        {
            return Err(NOT_CONNECTED.to_owned());
        }
        answer
            .await
            .unwrap_or_else(|_| Err(REPLAY_UNSENT.to_owned()))
    }
}

/// The actor — see the module doc.
struct Driver {
    key: DaemonKey,
    source: Arc<dyn EndpointSource>,
    registry: Registry<ShellWire>,
    dialer: Arc<LocalDaemonDialer>,
    route: RouteState,
    clock: Arc<dyn Clock>,
    /// Cloned into every pump.
    routable_tx: mpsc::Sender<Routable>,
    /// The dial address of every handle whose inbox has not ended.
    registrations: HashMap<HandleToken, Address>,
    /// The newest handle — the key's current one. `None` after a user close or a refusal stop, so a late event of
    /// that handle cannot act on the key again.
    current: Option<HandleToken>,
    /// When the boot hold ends by itself — `None` while no hold is on.
    boot_deadline: Option<std::time::Instant>,
    /// The current handle and the callers waiting for its first answer — callers of a handle it replaced included.
    first: Option<(
        HandleToken,
        Vec<oneshot::Sender<Result<(), HandshakeError>>>,
    )>,
    closes: Arc<AtomicU64>,
    status_tx: watch::Sender<ConnectionStatus>,
    events: Arc<dyn DaemonEvents>,
    names: Arc<dyn AgentNamesSink>,
    router: Arc<OutputRouter>,
    windows: WindowChannelRegistry,
    /// The connection routed as `LinkUp` and not yet as `LinkDown` — the one replay flights and callers' writes
    /// belong to.
    open: Option<Generation>,
    /// The last known epoch of each agent (ADR-0046 T5). Kept across connections.
    subs: HashMap<AgentId, SubState>,
    /// Single-flight replay bookkeeping. Its generation counters live as long as the actor; its flights end with
    /// their connection.
    flight: ReplayFlightSet,
    /// Unknown output tags already warned about on the open connection.
    unknown_tags: UnknownTagLog,
    /// A connection this actor decided to drop and whose `LinkDown` has not come yet — its later output frames and
    /// driver events are not handled (`drop_connection`); its broadcasts and lifecycle events still pass.
    dropping: Option<Generation>,
    inbound: Arc<InboundSlot>,
    refusals: Arc<RefusalAlerts>,
    usage: SharedUsageInterest,
    /// Shared with the callers (module doc): its opened connection is set at the end of a `LinkUp` step and cleared by
    /// that connection's `LinkDown`, its drop and a close.
    gate: Arc<Mutex<Gate>>,
    /// Signalled after every change of the gate's opened connection.
    opened_tx: watch::Sender<()>,
}

impl Driver {
    async fn run(
        mut self,
        mut cmd_rx: mpsc::UnboundedReceiver<DriverCmd>,
        mut routable_rx: mpsc::Receiver<Routable>,
    ) {
        // Armed only while a replay waits for its deadline — an idle actor (no daemon, no replay) does not wake.
        let mut sweep: Option<BoxFuture<'static, ()>> = None;
        // Armed for the boot hold's deadline, re-armed when it moves.
        let mut boot_timer: Option<(std::time::Instant, BoxFuture<'static, ()>)> = None;
        loop {
            if !self.flight.awaits_deadline() {
                sweep = None;
            } else if sweep.is_none() {
                sweep = Some(self.clock.sleep(REPLAY_DEADLINE_TICK));
            }
            match self.boot_deadline {
                None => boot_timer = None,
                Some(at) if boot_timer.as_ref().is_some_and(|(armed, _)| *armed == at) => {}
                Some(at) => {
                    let left = at.saturating_duration_since(self.clock.now());
                    boot_timer = Some((at, self.clock.sleep(left)));
                }
            }
            tokio::select! {
                cmd = cmd_rx.recv() => match cmd {
                    Some(DriverCmd::IsLive { reply }) => {
                        let _ = reply.send(self.is_live());
                    }
                    Some(DriverCmd::Add {
                        closes,
                        found,
                        first,
                    }) => self.add(closes, found, first),
                    Some(DriverCmd::Close { by }) => self.close_by(by),
                    Some(DriverCmd::Unsubscribe { agent_id }) => self.unsubscribe(agent_id),
                    Some(DriverCmd::RequestReplay { agent_id, reply }) => {
                        let _ = reply.send(self.request_replay(agent_id));
                    }
                    Some(DriverCmd::UsageNudge) => self.nudge_usage(),
                    Some(DriverCmd::HoldForBootStart) => self.hold_for_boot_start(),
                    Some(DriverCmd::BoundBootStart { search }) => self.bound_boot_start(search),
                    Some(DriverCmd::EndBootStartHold { why }) => self.end_boot_start_hold(why),
                    #[cfg(test)]
                    Some(DriverCmd::CurrentHandle { reply }) => {
                        let _ = reply.send(self.current);
                    }
                    #[cfg(test)]
                    Some(DriverCmd::BootStartHeld { reply }) => {
                        let _ = reply.send(self.route.boot_start_held());
                    }
                    None => break,
                },
                // Never `None` — `self.routable_tx` is a sender.
                Some(routable) = routable_rx.recv() => self.on_routable(routable),
                () = async {
                    match sweep.as_mut() {
                        Some(tick) => tick.await,
                        None => std::future::pending().await,
                    }
                } => {
                    self.sweep_replay_deadlines();
                    sweep = None;
                }
                () = async {
                    match boot_timer.as_mut() {
                        Some((_, timer)) => timer.await,
                        None => std::future::pending().await,
                    }
                } => {
                    boot_timer = None;
                    self.end_boot_start_hold("its bound passed");
                }
            }
        }
        tracing::debug!(peer = %self.key.peer_id(), "the daemon client is gone — closing its link");
        // Dropping `routable_rx` with the actor makes every pump close its inbox and drain it (`link::pump`).
        self.registry.remove(self.key.peer_id());
        for address in self.registrations.values() {
            self.dialer.unregister(address);
        }
    }

    /// The current handle's peer is live, or its connection is still open as routed. Routing trails the peer (the
    /// supervisor publishes `Live` before it queues `Connected`), so a connection that ended but whose `Disconnected`
    /// is not routed yet still counts — a connect then answers `Ok` instead of re-adding behind its loss.
    fn is_live(&self) -> bool {
        self.current
            .is_some_and(|token| self.route.is_open(&self.key, token))
            || self
                .registry
                .get(self.key.peer_id())
                .is_some_and(|peer| peer.peer_state().is_live())
    }

    fn hold_for_boot_start(&mut self) {
        tracing::info!(peer = %self.key.peer_id(), "holding connection bands until the boot start answers");
        self.route.hold_for_boot_start();
        self.boot_deadline = Some(self.clock.now() + BOOT_START_CAP);
    }

    // TRD R24: the search's own bound replaces the cap — a main start that begins late keeps its whole search.
    fn bound_boot_start(&mut self, search: Duration) {
        if !self.route.boot_start_held() {
            return;
        }
        self.boot_deadline = Some(self.clock.now() + search + BOOT_START_MARGIN);
    }

    fn end_boot_start_hold(&mut self, why: &'static str) {
        self.boot_deadline = None;
        if !self.route.boot_start_held() {
            return;
        }
        tracing::info!(peer = %self.key.peer_id(), why, "the boot hold of connection bands ended");
        let out = self.route.end_boot_start_hold();
        self.apply(out);
    }

    /// A queued close. ★Routing hears an explicit frontend close before the peer goes★ — what the removal routes
    /// later shows no band. The stop path's close leaves routing alone — a stop sets no band.
    fn close_by(&mut self, by: CloseBy) {
        let out = match by {
            CloseBy::Frontend => self.route.close(&self.key),
            CloseBy::Stop => Vec::new(),
        };
        self.close();
        self.apply(out);
    }

    /// The link does not reconnect; a late item of the handle no longer counts as the current one's (no refusal
    /// stops it, no notice follows the close). Callers still waiting for the first answer get `ClosedBeforeHello` now —
    /// a later handle's answer is not theirs.
    fn close(&mut self) {
        self.current = None;
        self.registry.remove(self.key.peer_id());
        {
            let mut gate = lock_gate(&self.gate);
            gate.opened = None;
            gate.pending.clear();
        }
        self.opened_tx.send_replace(());
        if let Some((_, waiting)) = self.first.take() {
            for caller in waiting {
                let _ = caller.send(Err(HandshakeError::ClosedBeforeHello));
            }
        }
    }

    fn add(
        &mut self,
        closes: u64,
        found: Option<Endpoint>,
        first: oneshot::Sender<Result<(), HandshakeError>>,
    ) {
        if closes != self.closes.load(Ordering::SeqCst) {
            tracing::info!(
                peer = %self.key.peer_id(),
                "a close came after this connect began — not connecting"
            );
            let _ = first.send(Err(HandshakeError::TaskGone));
            return;
        }
        if self.is_live() {
            let _ = first.send(Ok(()));
            return;
        }
        // Before the peer exists — a caller that sees it live must not read the previous handle's epoch.
        lock_gate(&self.gate).handle_closes = closes;
        let token = HandleToken::mint();
        let (address, handshake, refusal) =
            self.dialer.register(&self.key, self.source.clone(), found);
        let ended = self.route.adopt(self.key.clone(), token, refusal);
        let (_peer, inbox) =
            self.registry
                .add(self.key.peer_id().clone(), address.clone(), handshake);
        spawn_pump(inbox, self.key.clone(), token, self.routable_tx.clone());
        self.registrations.insert(token, address);
        self.current = Some(token);
        // Callers of the replaced handle had no answer yet (no close came — it would have answered them): this handle
        // carries on their connect.
        let mut waiting = self
            .first
            .take()
            .map(|(_, waiting)| waiting)
            .unwrap_or_default();
        waiting.push(first);
        self.first = Some((token, waiting));
        self.apply(ended);
    }

    fn on_routable(&mut self, routable: Routable) {
        match routable {
            Routable::Item { key, token, item } => {
                let connected = matches!(item, Incoming::Event(TransportEvent::Connected { .. }));
                let out = self.route_item(&key, token, item);
                let linked = out.iter().any(|r| matches!(r, Routed::LinkUp { .. }));
                self.apply(out);
                // A link that connected ends a cold boot — the boot hold has nothing left to wait for (R-h).
                if linked {
                    self.end_boot_start_hold("a daemon link connected");
                }
                // After the status watch and the frontend's `connected` — the frontend registers its listeners before
                //   it calls connect and relies on that order (`src/api/tauriTransport.ts` `doConnect`).
                if connected {
                    self.answer_first_with(token, Ok(()));
                }
            }
            Routable::Ended { key, token } => {
                if let Some(address) = self.registrations.remove(&token) {
                    self.dialer.unregister(&address);
                }
                self.answer_first_with(token, Err(HandshakeError::TaskGone));
                let out = route_end(&mut self.route, &key, token);
                self.apply(out);
            }
        }
    }

    /// What one inbox item becomes. A refusal the transport would retry stops the peer here, its notice ahead of what
    /// routing returns — the order of a terminal refusal (`Notice`, then `Down`).
    fn route_item(
        &mut self,
        key: &DaemonKey,
        token: HandleToken,
        item: Incoming<ShellWire>,
    ) -> Vec<Routed> {
        self.answer_first(token, &item);
        let mut out = Vec::new();
        if let Some(code) = self.retryable_refusal(token, &item) {
            self.stop_refused(code);
            out.push(Routed::Notice(LinkNotice::Refused { code }));
        }
        out.extend(route(&mut self.route, key, token, item));
        out
    }

    /// The close code of a refusal the transport would retry, on the current handle (ADR-0346 — none is retried here).
    fn retryable_refusal(
        &self,
        token: HandleToken,
        item: &Incoming<ShellWire>,
    ) -> Option<engram_dashboard_transport::CloseCode> {
        match item {
            Incoming::Event(TransportEvent::ConnectFailed {
                cause:
                    ConnectFailure::Rejected {
                        kind: RejectKind::Retryable,
                        code,
                        ..
                    },
                ..
            }) if self.current == Some(token) => Some(*code),
            _ => None,
        }
    }

    /// Ends the current peer after a refusal, as the transport ends a terminal one; the `Closed` the removal produces
    /// shows `Down` (the caller sends the notice).
    fn stop_refused(&mut self, code: engram_dashboard_transport::CloseCode) {
        tracing::warn!(
            peer = %self.key.peer_id(),
            code = code.0,
            "the daemon refused the connection — not retrying"
        );
        self.close();
    }

    /// The first answer of the newest handle, when it is a failure. Only that handle is the key's current one (`adopt`
    /// in the same step as the add), so the handle check is the routing's rule ①; items before it carry no older
    /// generation of it. A `Connected` is answered by [`Self::on_routable`] after it is applied.
    fn answer_first(&mut self, token: HandleToken, item: &Incoming<ShellWire>) {
        let Incoming::Event(event) = item else {
            return;
        };
        let answer = match event {
            TransportEvent::ConnectFailed {
                cause: ConnectFailure::Unreachable(e),
                ..
            } => Err(match no_live_daemon_cause(&e.message) {
                Some(cause) => HandshakeError::NoLiveDaemon(cause),
                None => HandshakeError::Connect(e.to_string()),
            }),
            // The refusal's own text can be the daemon's — the fixed local text instead (ADR-0342), from the same
            //   decision routing makes for the band (TRD S21 trd-3-2-u5 §4-3).
            TransportEvent::ConnectFailed {
                cause: ConnectFailure::Rejected { code, .. },
                ..
            } => Err(HandshakeError::AuthRejected(
                refusal_message(*code, false).to_owned(),
            )),
            TransportEvent::Rejected { code, .. } => {
                let version = self.route.version_refusal(&self.key, token).is_some();
                Err(HandshakeError::AuthRejected(
                    refusal_message(*code, version).to_owned(),
                ))
            }
            TransportEvent::GaveUp { .. } | TransportEvent::Closed { .. } => {
                Err(HandshakeError::ClosedBeforeHello)
            }
            _ => return,
        };
        self.answer_first_with(token, answer);
    }

    fn answer_first_with(&mut self, token: HandleToken, answer: Result<(), HandshakeError>) {
        if self
            .first
            .as_ref()
            .is_some_and(|(newest, _)| *newest == token)
        {
            if let Some((_, waiting)) = self.first.take() {
                for caller in waiting {
                    // The caller may have stopped waiting.
                    let _ = caller.send(answer.clone());
                }
            }
        }
    }

    /// The open connection and the peer to write on — `None` before `LinkUp`, after `LinkDown`, and once a close
    /// removed the peer.
    fn open_peer(&self) -> Option<(Peer<ShellWire>, Generation)> {
        let generation = self.open?;
        let peer = self.registry.get(self.key.peer_id())?;
        Some((peer, generation))
    }

    fn unsubscribe(&self, agent_id: AgentId) {
        let Some((peer, generation)) = self.open_peer() else {
            tracing::debug!(
                peer = %self.key.peer_id(),
                %agent_id,
                "unsubscribe: no open daemon connection — nothing to undo"
            );
            return;
        };
        if let Err(e) = peer.notify_in(generation, AgentCommand::Unsubscribe { agent_id }) {
            tracing::debug!(
                peer = %self.key.peer_id(),
                %agent_id,
                generation = generation.0,
                error = %e,
                "unsubscribe: not written"
            );
        }
    }

    fn request_replay(&mut self, agent_id: AgentId) -> Result<u64, String> {
        let Some((peer, generation)) = self.open_peer() else {
            return Err(NOT_CONNECTED.to_owned());
        };
        // The window check: a `Subscribe` for an agent no window shows would leave a subscription that no
        //   `Unsubscribe` ever undoes. Running it here, when the actor takes the request, is what orders it after an
        //   earlier `Unsubscribe` (module doc).
        if self.router.targets(agent_id).is_empty() {
            tracing::debug!(
                peer = %self.key.peer_id(),
                %agent_id,
                "replay request: no window shows the agent — no Subscribe"
            );
            return Err(REPLAY_UNSENT.to_owned());
        }
        let epoch = known_epoch(&self.subs, agent_id);
        let outcome = self.flight.request_replay(agent_id, self.clock.now());
        tracing::debug!(
            peer = %self.key.peer_id(),
            %agent_id,
            gen = outcome.generation,
            send_now = outcome.send_now,
            "replay request"
        );
        if outcome.send_now && !self.write_subscribe(&peer, generation, agent_id, epoch) {
            return Err(REPLAY_UNSENT.to_owned());
        }
        Ok(outcome.generation)
    }

    /// Writes a replay `Subscribe` on connection `generation`; returns whether it went into the peer queue. Its
    /// generation is already in flight, and a `Subscribe` that never went out gets no answer — the slot would stay
    /// taken and every later request of the agent would merge into it. So a failed write drops the connection, which
    /// clears every flight (`drop_connection`).
    fn write_subscribe(
        &mut self,
        peer: &Peer<ShellWire>,
        generation: Generation,
        agent_id: AgentId,
        epoch: Option<u32>,
    ) -> bool {
        let subscribe = AgentCommand::Subscribe {
            agent_id,
            epoch,
            after_seq: None,
        };
        match peer.notify_in(generation, subscribe) {
            Ok(()) => true,
            Err(e) => {
                tracing::warn!(
                    peer = %self.key.peer_id(),
                    %agent_id,
                    generation = generation.0,
                    error = %e,
                    "a replay Subscribe was not written"
                );
                self.drop_connection(generation, DisconnectCause::WriteFailed);
                false
            }
        }
    }

    /// Ends connection `generation` as a lost link (`Peer::drop_link_in`): the handle stays, `Reconnecting` follows
    /// from the `Disconnected`, and the next dial waits the usual backoff — a daemon that keeps sending what made us
    /// drop cannot drive a reconnect loop. ★Bound to that connection★: when it already ended, the transport does nothing
    /// (its `LinkDown` is on the way) and never drops a newer connection.
    ///
    /// ★The connection is over for this actor at once★: its flights are cleared
    /// now and `open` goes, and its output frames and replay events still in the inbox (or salvaged before its
    /// `Disconnected`) are not relayed until its `LinkDown` — a later `ReplayComplete` would mark a replay complete
    /// that is missing the frame we could not read.
    ///
    /// The actor-side part runs with or without a peer — after a close the registry no longer holds one, but the tail of
    /// the connection is still routed until its `LinkDown`.
    // A drop lost in the transport would leave a live connection blacked out here; `link_policy` keeps
    //   `QueueFullPolicy::Disconnect`, under which a drop that meets a full queue drops the link anyway.
    fn drop_connection(&mut self, generation: Generation, cause: DisconnectCause) {
        tracing::info!(
            peer = %self.key.peer_id(),
            generation = generation.0,
            ?cause,
            "dropping the daemon connection if it is still the live one — the link reconnects after its backoff"
        );
        if let Some(peer) = self.registry.get(self.key.peer_id()) {
            peer.drop_link_in(generation, cause);
        }
        self.dropping = Some(generation);
        if self.open == Some(generation) {
            self.open = None;
            self.flight.on_disconnect();
            self.close_gate();
        }
    }

    /// An item of a connection being dropped — not relayed.
    fn is_dropping(&self, generation: Generation, what: &'static str) -> bool {
        let dropping = self.dropping == Some(generation);
        if dropping {
            tracing::debug!(
                peer = %self.key.peer_id(),
                generation = generation.0,
                what,
                "an item of a daemon connection being dropped — not relayed"
            );
        }
        dropping
    }

    /// The `Subscribe` of a merged replay request, released by the resolution of the one before it. ★The window check
    /// again★ — the agent's last window may have closed since the request merged, and its `Unsubscribe` is already
    /// written: a `Subscribe` now would leak the subscription (the A1 shape). Then nothing is written and the slot is
    /// released as a refusal releases it — no answer can come for a `Subscribe` that never went out.
    fn write_follow_up(&mut self, generation: Generation, agent_id: AgentId, epoch: Option<u32>) {
        if self.router.targets(agent_id).is_empty() {
            tracing::debug!(
                peer = %self.key.peer_id(),
                %agent_id,
                "no window shows the agent any more — its merged replay request is dropped"
            );
            // Nothing is merged behind it — the resolution just promoted it; its failure marker has no window to go to.
            let _ = self.flight.on_refused(agent_id, self.clock.now());
            return;
        }
        match self.registry.get(self.key.peer_id()) {
            Some(peer) => {
                self.write_subscribe(&peer, generation, agent_id, epoch);
            }
            // Closed — the `LinkDown` before its `Closed` clears the flight.
            None => tracing::debug!(
                peer = %self.key.peer_id(),
                generation = generation.0,
                "the next replay Subscribe has no peer to go to — the link was closed"
            ),
        }
    }

    /// An event of connection `generation` the driver handles itself: a replay event (`SubscribeAck` ·
    /// `ReplayComplete` · `SubscribeFailed` — the follow-up `Subscribe` goes out on that connection), a daemon command
    /// request, a command reply too broken to match a request, a usage snapshot.
    fn on_driver_event(&mut self, generation: Generation, event: AgentEvent) {
        let windows = &self.windows;
        let mut deliver =
            |labels: &[WindowLabel], bytes: &[u8]| send_to_windows(windows, labels, bytes);
        let follow_up = apply_replay_event(
            &event,
            &mut self.flight,
            &mut self.subs,
            &self.router,
            self.clock.now(),
            &mut deliver,
        );
        match follow_up {
            ReplayFollowUp::Handled(None) => {}
            ReplayFollowUp::Handled(Some(AgentCommand::Subscribe {
                agent_id, epoch, ..
            })) => self.write_follow_up(generation, agent_id, epoch),
            ReplayFollowUp::Handled(Some(other)) => tracing::warn!(
                peer = %self.key.peer_id(),
                ?other,
                "a replay follow-up that is not a Subscribe — not written"
            ),
            ReplayFollowUp::NotHandled => match event {
                // ADR-0272 decision 6: an envelope that does not parse has no request id to answer with — it is
                //   neither run nor answered; the daemon ends that round trip by its own deadline.
                AgentEvent::CommandRequest { envelope } => {
                    match protocol_state::envelope_from_wire(&envelope) {
                        Ok(envelope) => self.accept_inbound(generation, envelope),
                        Err(e) => tracing::warn!(
                            peer = %self.key.peer_id(),
                            generation = generation.0,
                            event = "CommandRequest",
                            class = ?e.classify(),
                            line = e.line(),
                            column = e.column(),
                            "a daemon command envelope does not parse — no request id to answer with; dropped"
                        ),
                    }
                }
                // ADR-0272 decision 6: a reply reaches the driver only when its request id cannot be read (a readable
                //   one is matched in the transport, or reported there as unmatched) — no waiter to wake, log only.
                AgentEvent::CommandReply { reply } => match protocol_state::reply_from_wire(&reply)
                {
                    Err(e) => tracing::warn!(
                        peer = %self.key.peer_id(),
                        generation = generation.0,
                        event = "CommandReply",
                        class = ?e.classify(),
                        line = e.line(),
                        column = e.column(),
                        "a daemon command reply does not parse — no request id to wake; dropped"
                    ),
                    Ok(reply) => tracing::debug!(
                        peer = %self.key.peer_id(),
                        generation = generation.0,
                        request_id = %reply.request_id,
                        "a daemon command reply reached the driver — dropped"
                    ),
                },
                AgentEvent::UsageLimitsUpdated {
                    snapshot,
                    subscribed,
                } => self.relay_usage_snapshot(generation, &snapshot, &subscribed),
                other => tracing::debug!(
                    peer = %self.key.peer_id(),
                    event = event_kind(&other),
                    "a daemon event this driver does not handle — dropped"
                ),
            },
        }
    }

    /// Hands a daemon command request to the command table, off the actor (ADR-0155 decision 4); its outcome goes back
    /// only on connection `generation`, the one it came on.
    fn accept_inbound(&self, generation: Generation, envelope: CommandEnvelope) {
        let request_id = envelope.request_id;
        let deliver = self.outcome_sink(generation);
        match self.inbound.get() {
            Some(receiver) => receiver.accept(envelope, deliver),
            // Not dropped silently — the daemon would hold the round trip until its deadline. A missing table is an
            //   assembly miss (`install_command_table` not called), so a retry would not help.
            None => {
                tracing::error!(
                    peer = %self.key.peer_id(),
                    %request_id,
                    "a daemon command request came before a command table was installed — assembly miss \
                     (install_command_table)"
                );
                deliver(BusReply::err(
                    request_id,
                    CommandError::internal("this client has no command table installed"),
                ));
            }
        }
    }

    /// Where a command outcome goes — connection `generation` of the key's peer at the time the outcome is ready, never
    /// a later connection (`send_command_outcome`).
    fn outcome_sink(&self, generation: Generation) -> impl FnOnce(BusReply) + Send + 'static {
        let registry = self.registry.clone();
        let peer_id = self.key.peer_id().clone();
        move |reply: BusReply| match registry.get(&peer_id) {
            Some(peer) => send_command_outcome(&peer, generation, &reply),
            None => tracing::warn!(
                peer = %peer_id,
                request_id = %reply.request_id,
                generation = generation.0,
                "a command outcome has no peer to go to — the link was closed; dropped"
            ),
        }
    }

    /// A usage snapshot of connection `generation`: matched against the interest, a resend queued on that connection
    /// when the daemon's set differs, then handed to the windows with a usage slot (`UsageInterest::on_snapshot`).
    /// ★The resend is queued under the usage lock★ (module doc) — so ahead of the frontend notice, which goes out with
    /// no lock held (ADR-0006).
    fn relay_usage_snapshot(
        &self,
        generation: Generation,
        snapshot: &UsageLimitSnapshot,
        subscribed: &[AgentBackendKind],
    ) {
        // Before the usage lock — the registry takes its own lock.
        let peer = self.registry.get(self.key.peer_id());
        let labels = {
            let mut usage = self.usage.lock();
            // The real clock, not the injected one — the webview pull ages the cache against `Instant::now()`.
            let outcome = usage.on_snapshot(
                generation.0,
                snapshot,
                subscribed,
                std::time::Instant::now(),
            );
            if let Some(vendors) = outcome.resend {
                match &peer {
                    Some(peer) => write_usage_subscribe(peer, generation, vendors, "resend"),
                    None => tracing::debug!(
                        peer = %self.key.peer_id(),
                        generation = generation.0,
                        "a usage resend has no peer to go to — the link was closed"
                    ),
                }
            }
            outcome.labels
        };
        // No window to receive it: a snapshot of a socket that is no longer current, or no window has a usage slot.
        if !labels.is_empty() {
            self.events
                .usage_limits_updated(snapshot, &labels, generation.0);
        }
    }

    /// Writes the interest of the moment on the open connection when it differs from what that connection got.
    fn nudge_usage(&self) {
        let Some((peer, generation)) = self.open_peer() else {
            tracing::debug!(
                peer = %self.key.peer_id(),
                "usage nudge: no open daemon connection — the next one gets the interest when it opens"
            );
            return;
        };
        let mut usage = self.usage.lock();
        if let Some(vendors) = usage.sync(generation.0, false) {
            write_usage_subscribe(&peer, generation, vendors, "nudge");
        }
    }

    /// No connection is opened any more — callers wait or fail; pending fire-and-forgets stay for a later connection.
    fn close_gate(&self) {
        lock_gate(&self.gate).opened = None;
        self.opened_tx.send_replace(());
    }

    /// Opens connection `generation` (module doc): the usage subscribe and the command registration are queued, then —
    /// under the gate — the pending fire-and-forgets of this connection are written (older ones dropped, newer ones
    /// kept) and the connection is set as opened. With no peer (the user closed, the connection's tail is still routed)
    /// nothing is written or opened; the usage state still pairs this open with the `LinkDown` to come.
    fn on_link_up(&mut self, generation: Generation) {
        self.open = Some(generation);
        let peer = self.registry.get(self.key.peer_id());
        // The interest first, then the registration.
        {
            let mut usage = self.usage.lock();
            let opened = usage.on_socket_open(generation.0);
            if let (Some(vendors), Some(peer)) = (opened, &peer) {
                write_usage_subscribe(peer, generation, vendors, "open");
            }
        }
        let Some(peer) = peer else {
            return;
        };
        self.register_commands(&peer, generation);
        let (mut failed, mut dropped) = (Vec::new(), 0usize);
        // ★The drain and the publish stay one critical section, before the signal★ — a caller's round trip writes
        // only after it saw `opened` under the gate, so its own pending fire-and-forgets are already on the peer
        // queue. Draining after the publish (or after the signal) lets that round trip overtake them.
        {
            let mut gate = lock_gate(&self.gate);
            for (pending, cmd) in std::mem::take(&mut gate.pending) {
                if pending == generation {
                    if let Err(e) = peer.notify_in(generation, cmd) {
                        failed.push(e);
                    }
                } else if pending > generation {
                    gate.pending.push_back((pending, cmd));
                } else {
                    dropped += 1;
                }
            }
            gate.opened = Some(Opened {
                generation,
                closes: gate.handle_closes,
            });
        }
        self.opened_tx.send_replace(());
        if dropped > 0 {
            tracing::debug!(
                peer = %self.key.peer_id(),
                generation = generation.0,
                dropped,
                "fire-and-forget: pending for a connection that never opened — dropped"
            );
        }
        for e in failed {
            tracing::debug!(
                peer = %self.key.peer_id(),
                generation = generation.0,
                error = %send_error_text(&e),
                "fire-and-forget: not sent"
            );
        }
    }

    /// Sends this shell's command names to the daemon roster on connection `generation` (TRD §3-7 clause 1 — every
    /// connect; a resend replaces the names, as the daemon roster is last-wins per name — ADR-0150 decision 3). The
    /// request is queued in this step; the reply is read on a spawned task (module doc).
    ///
    /// ★A refusal is not retried on this connection★ — the names stay off the roster until the next one. Known limit:
    /// the reply has the transport's request timeout ([`link_policy`]) — a daemon slower than that leaves a warn and no
    /// refusal box.
    fn register_commands(&self, peer: &Peer<ShellWire>, generation: Generation) {
        let Some(receiver) = self.inbound.get() else {
            tracing::debug!(
                peer = %self.key.peer_id(),
                generation = generation.0,
                "no command table installed — nothing to register"
            );
            return;
        };
        let Some(cmd) = registration_command(receiver) else {
            return;
        };
        let names = match &cmd {
            AgentCommand::RegisterCommands { decls, .. } => {
                // Just encoded by `registration_command` — failing to decode means that function changed.
                match protocol_state::decls_from_wire(decls) {
                    Ok(decls) => decls.len(),
                    Err(e) => {
                        tracing::error!(
                            error = %e,
                            "the registration packet carries a declaration list that does not decode"
                        );
                        return;
                    }
                }
            }
            other => {
                tracing::error!(?other, "the registration packet is not RegisterCommands");
                return;
            }
        };
        let refusals = self.refusals.clone();
        let peer = peer.clone();
        match start_now(async move { peer.request_in(generation, cmd).await }) {
            Ok(answer) => report_registration(answer, names, generation, &refusals),
            Err(reply) => {
                tokio::spawn(async move {
                    report_registration(reply.await, names, generation, &refusals);
                });
            }
        }
    }

    /// One output frame of connection `generation` to the windows that show its agent (`frame_relay`).
    fn relay_output(&mut self, generation: Generation, bytes: &[u8]) {
        let windows = &self.windows;
        let mut deliver =
            |labels: &[WindowLabel], bytes: &[u8]| send_to_windows(windows, labels, bytes);
        let relayed = relay_binary_frame(
            bytes,
            &mut self.flight,
            &mut self.subs,
            &self.router,
            &mut self.unknown_tags,
            self.clock.now(),
            &mut deliver,
        );
        if relayed == FrameRelay::Disconnect {
            self.drop_connection(generation, DisconnectCause::ReadFailed);
        }
    }

    /// Fails every replay that made no progress within [`REPLAY_DEADLINE`]: one failure marker each, the slot kept (why = `ReplayFlightSet::check_deadlines`).
    fn sweep_replay_deadlines(&mut self) {
        for (agent_id, marker) in self.flight.check_deadlines(self.clock.now()) {
            tracing::warn!(
                peer = %self.key.peer_id(),
                %agent_id,
                gen = marker.generation,
                "a replay made no progress before its deadline — failure marker sent; its Subscribe slot stays taken \
                 until a late answer or the connection ends"
            );
            // Best effort — a failure has no Ack to take the epoch from; the view does not fence failure markers.
            let epoch = known_epoch(&self.subs, agent_id).unwrap_or(0);
            let frame = encode_marker_frame(agent_id, epoch, marker);
            send_to_windows(&self.windows, &self.router.targets(agent_id), &frame);
        }
    }

    fn apply(&mut self, out: Vec<Routed>) {
        for routed in out {
            match routed {
                Routed::Status(status) => {
                    self.status_tx.send_replace(status);
                    self.events.connection_status(&status);
                }
                Routed::Broadcast(event) => {
                    emit_broadcast(self.events.as_ref(), self.names.as_ref(), &event)
                }
                // Notices stay log-only — bands come from `Routed::Status`; routing already logged the refusal.
                Routed::Notice(notice) => {
                    tracing::debug!(peer = %self.key.peer_id(), ?notice, "daemon link notice")
                }
                Routed::LinkUp { generation } => self.on_link_up(generation),
                // The per-connection reset: flights cleared without markers (the frontend re-requests on its
                //   `connected` edge), unknown tags warned again on the next connection, the usage state forgets the
                //   socket. Publishing `None` always wakes callers waiting for an opened connection, so they see the
                //   loss now.
                Routed::LinkDown { generation } => {
                    self.open = None;
                    self.flight.on_disconnect();
                    self.unknown_tags = UnknownTagLog::default();
                    if self.dropping == Some(generation) {
                        self.dropping = None;
                    }
                    self.usage.lock().on_socket_lost(generation.0);
                    self.close_gate();
                }
                // ★A daemon command request on a connection being dropped is skipped too — not run, not answered★. The
                //   daemon does not fail it when the link closes: its delivery table reaps only
                //   the round trips the closing connection started (`command_delivery.rs` `drop_origin` doc), so the
                //   round trip ends at its delivery deadline as `TIMEOUT` (`CommandDeliveries::expire`). Running it
                //   here would apply its side effects with an outcome that can never reach the daemon — the drop is
                //   queued ahead of it and the connection is gone after.
                Routed::ToDriver { generation, .. } if self.is_dropping(generation, "event") => {}
                // ★Likewise a daemon command request in a closed link's tail★ (`close` leaves the old handle routed
                //   until its `LinkDown`): its outcome can never reach the daemon — the peer is removed — so running it
                //   would apply side effects for nothing; the daemon ends the round trip at its delivery deadline as
                //   `TIMEOUT`, as above.
                Routed::ToDriver {
                    generation,
                    event: AgentEvent::CommandRequest { .. },
                } if self.current.is_none() => tracing::debug!(
                    peer = %self.key.peer_id(),
                    generation = generation.0,
                    "a daemon command request of a closed link — not run"
                ),
                Routed::ToDriver { generation, event } => self.on_driver_event(generation, event),
                Routed::Output { generation, .. } if self.is_dropping(generation, "output") => {}
                Routed::Output { generation, bytes } => self.relay_output(generation, &bytes),
            }
        }
    }
}

/// Whether connection `bound` of `peer` is opened for a caller that read close epoch `closes`; `false` while the actor
/// has not opened it yet. `Err`: the peer was closed, or a close came after the connection was opened
/// ([`NOT_CONNECTED`]), or connection `bound` is no longer the live one
/// ([`UNSENT_ON_DISCONNECT`]).
fn opened_now(
    peer: &Peer<ShellWire>,
    bound: Generation,
    closes: u64,
    gate: &Mutex<Gate>,
) -> Result<bool, String> {
    match peer.peer_state() {
        PeerState::Closed => return Err(NOT_CONNECTED.to_owned()),
        state if !state.is_live() => return Err(UNSENT_ON_DISCONNECT.to_owned()),
        _ if peer.generation() != bound => return Err(UNSENT_ON_DISCONNECT.to_owned()),
        _ => {}
    }
    let opened = lock_gate(gate).opened;
    match opened {
        Some(opened) if opened.generation == bound && opened.closes < closes => {
            Err(NOT_CONNECTED.to_owned())
        }
        Some(opened) => Ok(opened.generation == bound),
        None => Ok(false),
    }
}

/// What became of a fire-and-forget at the caller.
enum FireOutcome {
    Written(Result<(), engram_dashboard_transport::SendError>),
    Pending,
    Closed,
    Full,
}

fn usage_subscribe(vendors: BTreeSet<AgentBackendKind>) -> AgentCommand {
    AgentCommand::UsageSubscribe {
        vendors: vendors.into_iter().collect(),
    }
}

/// Queues the usage subscribe on connection `generation`. ★A failure does not drop the connection★ — unlike a replay
/// `Subscribe` no slot waits on it, and a connection that ended sends the whole interest again on its successor.
fn write_usage_subscribe(
    peer: &Peer<ShellWire>,
    generation: Generation,
    vendors: BTreeSet<AgentBackendKind>,
    why: &'static str,
) {
    if let Err(e) = peer.notify_in(generation, usage_subscribe(vendors)) {
        tracing::debug!(
            peer = %peer.id(),
            generation = generation.0,
            why,
            error = %e,
            "a usage subscribe was not queued"
        );
    }
}

/// Polls `future` once on this task, so what it does before its first wait happens in this step (a request is queued
/// on the peer); the rest is handed back to run elsewhere. Polled with a no-op waker: the next poll, on the task that
/// takes it, registers that task's waker.
fn start_now<F: Future>(future: F) -> Result<F::Output, Pin<Box<F>>> {
    let mut future = Box::pin(future);
    match future
        .as_mut()
        .poll(&mut Context::from_waker(Waker::noop()))
    {
        Poll::Ready(output) => Ok(output),
        Poll::Pending => Err(future),
    }
}

/// Logs the registration's answer; a daemon refusal also goes to the refusal box (ADR-0281 — a local failure does not).
fn report_registration(
    answer: Result<ShellIn, RequestError>,
    names: usize,
    generation: Generation,
    refusals: &RefusalAlerts,
) {
    let generation = generation.0;
    match answer {
        // A daemon `Error` reply is the daemon's refusal (ADR-0281 — `protocol_state::classify_reply`).
        Ok(ShellIn::Event(AgentEvent::Error { message, .. })) => {
            tracing::warn!(
                names,
                generation,
                reason = %mask_reason(&message),
                "the daemon refused the shell's command registration — its commands stay off the daemon roster \
                 until the next connection"
            );
            refusals.surface(RefusalSite::Register, &message);
        }
        Ok(ShellIn::Event(_)) => {
            tracing::info!(names, generation, "shell commands registered with the daemon")
        }
        // `ShellWire::reply_tag` never tags an output frame.
        Ok(ShellIn::Output(bytes)) => tracing::warn!(
            names,
            generation,
            bytes = bytes.len(),
            "an output frame answered the shell's command registration"
        ),
        Err(RequestError::TimedOut { after }) => tracing::warn!(
            names,
            generation,
            after_secs = after.as_secs(),
            "the daemon did not answer the shell's command registration within the request timeout — its commands \
             may be off the daemon roster until the next connection"
        ),
        Err(RequestError::Disconnected { sent: false, .. }) => tracing::debug!(
            names,
            generation,
            "the shell's command registration was not sent — its connection already ended; a next connection, if \
             any, registers again"
        ),
        Err(e) => tracing::warn!(
            names,
            generation,
            error = %request_error_text(&e),
            "the shell's command registration failed — its commands stay off the daemon roster until the next \
             connection"
        ),
    }
}

fn event_kind(event: &AgentEvent) -> &'static str {
    match event {
        AgentEvent::SubscribeAck { .. } => "SubscribeAck",
        AgentEvent::ReplayComplete { .. } => "ReplayComplete",
        AgentEvent::SubscribeFailed { .. } => "SubscribeFailed",
        AgentEvent::CommandRequest { .. } => "CommandRequest",
        AgentEvent::CommandReply { .. } => "CommandReply",
        AgentEvent::UsageLimitsUpdated { .. } => "UsageLimitsUpdated",
        _ => "other",
    }
}

#[cfg(test)]
mod tests;
