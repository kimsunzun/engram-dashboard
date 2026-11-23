//! DaemonClient — the shell's single authority over the daemon WS connection (S14 module ①, ADR-0036): one connection however many windows (the frontend reaches the daemon through the shell, not a WS per window — `src/api/wsTransport.ts`); owns its setup, handshake and life.
//! Protocol semantics are `protocol_state.rs`; output routing is `output_router` / `output_channel`.
//! - The connection runs on the driver in `connection.rs` over the transport crate (`link/`); its module doc holds the concurrency model (one actor owns the routing; callers' round trips run on their own tasks).
//! - `connect` (explicit, may spawn the daemon) and `ensure` (attach-only, never spawns) stay apart — ADR-0021.
//! - States: connected / down / reconnecting — no connecting state.

// ADR-0310: the driver on the transport crate (TRD S21 trd-3 §6-2 U3) — every client runs on it.
mod connection;
// ADR-0012: the seam for the connection's emits to the frontend (real AppHandle / recording fake / no-op); the reasons and contract live in that file's header.
// `pub(crate)` on purpose: its consumers are the connection drivers inside this crate; opening it would leak the `AppHandle` carrier onto the lib surface.
pub(crate) mod events;
// ADR-0231: relay decision for one output binary frame (decode · incarnation marker filter · viewing window) — seq is never skipped.
mod frame_relay;
// ADR-0155 decision 4: the entry point that receives commands the daemon sends. Applying them runs outside the connection task.
pub mod inbound;
// ADR-0310: the daemon link on the transport crate (TRD S21 trd-3 §6-2 U2).
pub mod link;
pub mod protocol_state;
pub(crate) mod refusal;
// ADR-0046 M1: the daemon's replay events applied to the single-flight state — no socket, no Tauri.
pub mod replay_events;
// ADR-0046 M1: single-flight replay numbering/fence state machine + replay boundary marker encoding (pure — zero socket/Tauri dependencies).
pub mod replay_flight;
// ADR-0343: the one stop path (single flight · identity-checked kill) — TRD S21 trd-3-2-u5 §3-3.
pub mod shutdown;
// ADR-0394, ADR-0410: what the daemon is told about the PC list slot — whether one is open (pure).
pub mod peer_watch;
// Usage interest (layout → daemon subscription set) + subscription reconciliation · cache state machine (pure — TRD S21 usage-limit-slot §1-7).
pub mod usage_interest;

use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use engram_dashboard_base::sync;
use engram_dashboard_protocol::{AgentCommand, AgentEvent, AgentId, AgentInfo};
use engram_dashboard_transport::ws::WsDialer;
use engram_dashboard_transport::SystemClock;
use tokio::runtime::Handle;
use tokio::sync::watch;

pub use connection::{HandshakeError, StartAttempt};
use events::{DaemonEvents, TauriEmitter};
// The assembly that discards notifications is harness-only (`events::NoDaemonEvents` doc) — it does not exist in production builds at all.
#[cfg(test)]
use events::NoDaemonEvents;
use inbound::InboundSlot;
use link::endpoint::LocalEndpointSource;
use link::DaemonKey;
use peer_watch::PeerWatch;
use refusal::{RefusalAlerts, TauriRefusalBox};
use shutdown::{StopPorts, Stopper};
// The assembly that discards the box is also harness-only (`refusal::NoRefusalNotice` doc).
#[cfg(test)]
use refusal::{NoRefusalNotice, RefusalNotice};
use usage_interest::{InterestAction, UsageInterest};

use crate::layout::ViewManager;
use crate::output_channel::WindowChannelRegistry;
use crate::output_router::OutputRouter;

/// The connection's lifetime state. On the link driver, routing (`link/route.rs`) is its only emitter. ★Its serialized
/// words (`connected` · `reconnecting` · `down`) are the frontend contract★ (`src/api/tauriTransport.ts`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, ts_rs::TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, rename = "ConnectionStateWord")]
pub enum ConnectionState {
    /// Not attached, explicitly closed, refused, or a never-connected handle's first failure (the link may still retry
    /// in the background — ADR-0346, TRD rev 9 ①).
    Down,
    Connected,
    // Retrying after an unintended loss (backoff … next attempt); the link driver never gives up on network failures. Distinct from an explicit close (Down), matching wsTransport's `reconnecting`.
    Reconnecting,
}

/// What the frontend gets about the daemon link — the payload of the `daemon-connection-state` event and the reply of
/// the `daemon_connection_state` pull (TRD S21 trd-3-2-u5 §4-2). Routing (`link/route.rs`) is its only producer and
/// emits it when `(state, band)` changes — a band can change while `state` keeps its word.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, ts_rs::TS)]
#[ts(export)]
pub struct ConnectionStatus {
    pub state: ConnectionState,
    /// The connection band the windows show; `null` = none.
    pub band: Option<Band>,
}

impl ConnectionStatus {
    pub const DOWN: Self = Self {
        state: ConnectionState::Down,
        band: None,
    };
}

/// The connection bands (TRD S21 trd-3-2-u5 §4-1 · ADR-0395 · ADR-0399). A reader that meets a `kind` it does not know treats it as
/// `not_connected` without a cause.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, ts_rs::TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export)]
pub enum Band {
    /// ① Not connected. `refused` = the daemon refused this app's handshake for a reason other than the version
    /// check (the token text — TRD U2).
    NotConnected { refused: bool },
    /// ② The running daemon speaks another protocol version. `can_stop` = a stop applies: the daemon is older than
    /// this app and did not start after this app's process. No frontend reads it since ADR-0356.
    Version { can_stop: bool },
    /// The daemon's listener runs under another OS account, so this app sent nothing (ADR-0395); `unverified` = its account could not be
    /// read at all (the app and Engram not run the same way — elevated vs not).
    Account { unverified: bool },
    /// The last daemon start was refused before a spawn: the data folder's drive does not store file ACLs (FAT / exFAT), or
    /// `unreadable` = whether it does could not be read (ADR-0399).
    Volume { unreadable: bool },
}

/// The shared handle to one shell's usage interest ([`UsageInterest`]) — held by the connection driver, the side that changes the interest (layout recompute · hide · shrink timer) and the webview pull (TRD S21 usage-limit-slot §1-7).
/// Lock order = `ViewManager → this lock`. A nudge ([`DaemonClient::nudge_usage_interest`]) issued inside this lock only sends to the actor (non-blocking); the driver takes the ViewManager lock ([`AgentNamesSink`]) holding no lock, so there is no reverse order. Inside this lock run only [`UsageInterest`] methods, that nudge and the driver's usage-subscribe queueing (queued here on purpose so read order = queue order; `Peer::notify_in` is a non-blocking leaf and the peer is looked up before the lock); emits happen after release (ADR-0006).
/// Poison is recovered from: a panic leaving the state half-fixed costs a subscription out of step or a missed delivery target that the next recompute · snapshot reconciliation · new socket repairs, whereas propagating the poison would kill the connection task and stop output and commands.
#[derive(Clone, Default)]
pub struct SharedUsageInterest(Arc<Mutex<UsageInterest>>);

impl SharedUsageInterest {
    pub fn lock(&self) -> MutexGuard<'_, UsageInterest> {
        sync::lock(&self.0)
    }
}

/// Port receiving the name part of the daemon's `AgentListUpdated` — fills the agent-name ledger tab auto-names read (TRD S26-window-chrome §4-3). Real = the shell assembly (`lib.rs`) calling `layout::apply::agent_names_updated`; harness = discard (`NoAgentNames`) · recording type.
/// - Separate from `events::DaemonEvents` (its doc forbids widening): that port only notifies the frontend while this one edits shell state (the ledger inside the layout lock).
/// - The driver calls it holding no lock: the real one takes the layout lock, inside which the usage interest lock is taken ([`SharedUsageInterest`]) — calling it under that lock is the reverse order.
/// - Never blocks beyond the layout lock's short critical section and swallows failures: a failed ledger edit is no reason to roll the connection back, and the next list refills it.
pub trait AgentNamesSink: Send + Sync {
    fn agents(&self, agents: &[AgentInfo]);
}

/// Discards the name-ledger port — harness-only, `#[cfg(test)]` for the same reason as `events::NoDaemonEvents`: in production it would let a client be assembled whose tab names stay at 「agent」 though the daemon list arrives.
#[cfg(test)]
pub(crate) struct NoAgentNames;

#[cfg(test)]
impl AgentNamesSink for NoAgentNames {
    fn agents(&self, _agents: &[AgentInfo]) {}
}

#[cfg(test)]
struct NoDaemonAnywhere;

#[cfg(test)]
impl link::endpoint::EndpointSource for NoDaemonAnywhere {
    fn ensure(
        &self,
        _timeout: Duration,
    ) -> Result<link::endpoint::Endpoint, crate::discovery::StartFailure> {
        Err(crate::discovery::StartFailure::other(
            "this test client never finds a daemon",
        ))
    }

    fn current(&self) -> link::endpoint::Current {
        link::endpoint::Current::Absent
    }

    fn verify_server(
        &self,
        _ends: Option<engram_dashboard_transport::SocketEnds>,
    ) -> Result<(), link::account::CheckFailure> {
        Err(link::account::CheckFailure::Refused(
            link::account::AccountRefusal::Unverified,
        ))
    }
}

const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// Thread name of the client's own runtime — blocking waits on that runtime must not run on its threads (`shutdown::DaemonClient::stop_daemon_blocking`).
pub(crate) const CLIENT_THREAD_NAME: &str = "engram-daemon-client";

// TRD S21 trd-3 §5 15 (C10): one ws message or frame on the daemon link is at most 16 MiB (the shell's side; the daemon's accept side takes the same value in stage 3-3). The largest legitimate message is not measured.
const DAEMON_MESSAGE_MAX_BYTES: usize = 16 << 20;

/// The text a caller gets when no connection takes commands right now (ADR-0195) — "never connected", "lost and reconnecting" and "explicitly closed" all fall into this one, since the caller's next step is the same.
/// It does not say "connect first": while reconnecting, a caller following that would re-add the link and cut the recovery in progress. This text reaches LLM callers through the control surface (§5), so a wrong next step here is one they would take; ADR-0195 leaves the retry timing to the caller.
pub(crate) const NOT_CONNECTED: &str =
    "not connected to the daemon — no commands are taken now (the caller decides when to retry)";

/// Something that should take the command is gone — the driver's actor stopped while a caller waited for the connection to open (`connection.rs`), or a write found the peer's queue closed (`link::errors::send_error_text` for `SendError::Closed`). A different fact from [`NOT_CONNECTED`].
pub(crate) const CHANNEL_GONE: &str = "the connection task did not take the command (disconnected)";

pub(crate) const REPLAY_UNSENT: &str =
    "replay request not sent (disconnected · no window shows that agent) — safe for the frontend to request again";

/// A round trip asked for a command that takes no reply (no `request_id`) — read by `link::errors` texts.
pub(crate) const NO_REQUEST_ID: &str =
    "send_command: a command without a request_id cannot expect a reply";

// The single handle to the daemon connection, shared by invoke handlers, the tray and state subscribers (`Arc<DaemonClient>`). The link driver (`connection`) owns the connection (its actor the routing, the transport peer the socket); this struct holds the driver's caller side (`link`) and the status watch (`status_rx`) — never the stream.
pub struct DaemonClient {
    rt: Handle,
    // Owns a dedicated runtime (production — T6a): `setup` may run outside a tokio context where `Handle::current()` panics, so `new_real_with_owned_runtime` builds a dedicated multi-thread runtime (spike §2) and keeps it alive here — dropping it invalidates the Handle and kills the connection task. Tied to the `DaemonClient` lifetime (`Arc<DaemonClient>` = app lifetime). `None` = external handle injected (tests = current runtime).
    _owned_rt: Option<Arc<tokio::runtime::Runtime>>,
    /// Filter for the command-registration refusal box (ADR-0281). Production = `refusal::TauriRefusalBox`, harness = `refusal::NoRefusalNotice` or a recording fake ([`Self::new_on_link`]). One per client, shared with the driver — never one per connection: repeat suppression lives as long as the shell process (`RefusalAlerts`), so per-connection filters would re-show the box on every reconnect with the same refusal.
    refusals: Arc<RefusalAlerts>,
    /// Connection status read without a lock — the driver's watch, which routing publishes; only borrowed here.
    status_rx: watch::Receiver<ConnectionStatus>,
    #[cfg(test)]
    router: Arc<OutputRouter>,
    #[cfg(test)]
    registry: WindowChannelRegistry,
    /// Entry for commands the daemon delivers (ADR-0155 decision 4). Filled late: the table's spawn port holds this client, so the assembly has a cycle (`inbound::InboundSlot`). Shared with the driver.
    inbound: Arc<InboundSlot>,
    /// The usage interest — outlives every connection (it comes from the layout, not the socket). Shared with the driver.
    usage: SharedUsageInterest,
    /// The peer watch — what the daemon is told about the PC list slot; the layout changes it. Lock order `ViewManager → this`; the
    ///   want is queued to the driver inside this lock, so the queue order is the order of the wants.
    peer: Arc<Mutex<PeerWatch>>,
    /// The driver on the transport crate (`connection`): connect, ensure, close, command round trips, fire-and-forget sends, unsubscribe, replay requests and usage nudges go there; `status_rx` is its watch.
    link: connection::LinkClient,
    stopper: Arc<Stopper>,
}

impl DaemonClient {
    /// A client on the link driver over a test network (TRD S21 trd-3 §6-2 U3): `inner` dials the address `source` resolves (a memory network), `clock` drives the transport. The assembly is production's ([`Self::on_link`]) with its own empty `router` and `registry`; the ports go in here — the driver starts with this call and keeps what it got.
    #[cfg(test)]
    pub(crate) fn new_on_link(
        rt: Handle,
        inner: Arc<dyn engram_dashboard_transport::Dialer>,
        source: Arc<dyn link::endpoint::EndpointSource>,
        clock: Arc<dyn engram_dashboard_transport::Clock>,
        events: Arc<dyn DaemonEvents>,
        names: Arc<dyn AgentNamesSink>,
        refusal_notice: Arc<dyn RefusalNotice>,
    ) -> Self {
        let stop = shutdown::inert_ports(inner.clone());
        Self::new_on_link_with_stop(
            rt,
            inner,
            source,
            clock,
            events,
            names,
            refusal_notice,
            stop,
        )
    }

    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new_on_link_with_stop(
        rt: Handle,
        inner: Arc<dyn engram_dashboard_transport::Dialer>,
        source: Arc<dyn link::endpoint::EndpointSource>,
        clock: Arc<dyn engram_dashboard_transport::Clock>,
        events: Arc<dyn DaemonEvents>,
        names: Arc<dyn AgentNamesSink>,
        refusal_notice: Arc<dyn RefusalNotice>,
        stop: StopPorts,
    ) -> Self {
        Self::on_link(
            rt,
            None,
            inner,
            source,
            clock,
            events,
            names,
            Arc::new(RefusalAlerts::new(refusal_notice)),
            Arc::new(OutputRouter::new()),
            WindowChannelRegistry::default(),
            stop,
        )
    }

    /// A client that is never connected — its endpoint source finds no daemon, so `connect` / `ensure` fail and every command fails as not connected.
    #[cfg(test)]
    pub(crate) fn unconnected(rt: Handle) -> Self {
        Self::new_on_link(
            rt,
            engram_dashboard_transport::testing::MemoryNetwork::new().dialer(),
            Arc::new(NoDaemonAnywhere),
            engram_dashboard_transport::testing::ManualClock::new().handle(),
            Arc::new(NoDaemonEvents),
            Arc::new(NoAgentNames),
            Arc::new(NoRefusalNotice),
        )
    }

    /// Starts the link driver on `rt` and builds the client around it; the driver's ports are this client's own objects (`router`, `registry`, command table slot, refusal filter, usage interest), so [`Self::install_command_table`] · [`Self::refusal_alerts`] · [`Self::usage_interest`] see what the driver sees.
    #[allow(clippy::too_many_arguments)]
    fn on_link(
        rt: Handle,
        owned_rt: Option<Arc<tokio::runtime::Runtime>>,
        inner: Arc<dyn engram_dashboard_transport::Dialer>,
        source: Arc<dyn link::endpoint::EndpointSource>,
        clock: Arc<dyn engram_dashboard_transport::Clock>,
        events: Arc<dyn DaemonEvents>,
        names: Arc<dyn AgentNamesSink>,
        refusals: Arc<RefusalAlerts>,
        router: Arc<OutputRouter>,
        registry: WindowChannelRegistry,
        stop: StopPorts,
    ) -> Self {
        Self::around_link(
            rt,
            owned_rt,
            events,
            names,
            refusals,
            router,
            registry,
            stop,
            |rt, ports| connection::LinkClient::start(rt, inner, source, clock, ports),
        )
    }

    /// The client around the link `start` makes from this client's ports — [`Self::on_link`] starts the driver; a test may build one it never starts.
    #[allow(clippy::too_many_arguments)]
    fn around_link(
        rt: Handle,
        owned_rt: Option<Arc<tokio::runtime::Runtime>>,
        events: Arc<dyn DaemonEvents>,
        names: Arc<dyn AgentNamesSink>,
        refusals: Arc<RefusalAlerts>,
        router: Arc<OutputRouter>,
        registry: WindowChannelRegistry,
        stop: StopPorts,
        start: impl FnOnce(&Handle, connection::Ports) -> connection::LinkClient,
    ) -> Self {
        let inbound = Arc::new(InboundSlot::new());
        let usage = SharedUsageInterest::default();
        let link = start(
            &rt,
            connection::Ports {
                events,
                names,
                router: router.clone(),
                windows: registry.clone(),
                inbound: inbound.clone(),
                refusals: refusals.clone(),
                usage: usage.clone(),
            },
        );
        let stopper = Stopper::new(rt.clone(), stop, link.control(), link.clock());
        Self {
            rt,
            _owned_rt: owned_rt,
            status_rx: link.status_rx(),
            #[cfg(test)]
            router,
            #[cfg(test)]
            registry,
            inbound,
            refusals,
            usage,
            peer: Arc::new(Mutex::new(PeerWatch::new())),
            link,
            stopper,
        }
    }

    /// The production constructor — `lib.rs` `setup` calls it. The client owns a multi-thread runtime built here because `setup` may run outside a tokio context (`Handle::current()` panics — spike §2); the runtime lives as long as the client (`_owned_rt`). Runs on the link driver: ws dialer, the local daemon's endpoint record (`daemon.json` in the default data folder), system clock. `Err` = the runtime could not be built; the caller reports it and goes on without daemon commands.
    /// `router` and `registry` are the shared instances `setup` also hands the layout commands and `subscribe_output`.
    pub fn new_real_with_owned_runtime(
        router: Arc<OutputRouter>,
        registry: WindowChannelRegistry,
        app: tauri::AppHandle,
        names: Arc<dyn AgentNamesSink>,
    ) -> std::io::Result<Self> {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .thread_name(CLIENT_THREAD_NAME)
            .build()?;
        let handle = rt.handle().clone();
        let client = Self::on_local_daemon(
            handle,
            Some(Arc::new(rt)),
            crate::discovery::default_data_dir(),
            Arc::new(TauriEmitter(app.clone())),
            names,
            Arc::new(RefusalAlerts::new(Arc::new(TauriRefusalBox(app)))),
            router,
            registry,
        );
        // TRD S21 trd-3-2-u5 R20: before any window exists, so no window's attach precedes it.
        client.link.hold_bands_for_boot_start();
        Ok(client)
    }

    /// [`Self::on_link`] on the local daemon of `data_dir`: ws dialer, that folder's endpoint record (`daemon.json`), system clock — production's transport, driven in tests on a temporary folder. The stop path reads the same folder (`StopPorts::local`).
    #[allow(clippy::too_many_arguments)]
    fn on_local_daemon(
        rt: Handle,
        owned_rt: Option<Arc<tokio::runtime::Runtime>>,
        data_dir: PathBuf,
        events: Arc<dyn DaemonEvents>,
        names: Arc<dyn AgentNamesSink>,
        refusals: Arc<RefusalAlerts>,
        router: Arc<OutputRouter>,
        registry: WindowChannelRegistry,
    ) -> Self {
        let stop = StopPorts::local(&data_dir);
        Self::on_link(
            rt,
            owned_rt,
            Arc::new(WsDialer::new(DAEMON_MESSAGE_MAX_BYTES)),
            Arc::new(LocalEndpointSource::new(
                DaemonKey::new(connection::DAEMON),
                data_dir,
            )),
            Arc::new(SystemClock),
            events,
            names,
            refusals,
            router,
            registry,
            stop,
        )
    }

    /// Plug in the command table this client can execute — call it before connecting (ADR-0155 decisions 4·5). The client picks the runtime with its own `rt`: apply tasks must run on the same runtime as the connection driver (while `agent.spawnInto` waits on a daemon round trip the driver's tasks must keep running), and letting each assembly pick the `Handle` would make each re-keep that invariant.
    /// Late calls are allowed (first winner wins), but after a connection opened, envelopes that arrived meanwhile get a "no table" error reply and that connection sends no registration — the next does (the driver registers on every `LinkUp`).
    /// `view` = where names absent from the shell table go (the webview's part — TRD §3-8 hop ③). It is plugged in at the same spot as the table because the registration packet carries both layers at once (TRD §3-7 clause 2) — separately, some assembly could send a registration without the webview's part and every view command would be `UNKNOWN_COMMAND` on that connection.
    pub fn install_command_table(
        &self,
        table: engram_dashboard_command::CommandTable,
        catalog_version: u32,
        view: Arc<dyn inbound::ViewCommandPort>,
    ) {
        self.inbound
            .set(Arc::new(inbound::InboundReceiver::with_view(
                table,
                Arc::new(inbound::RuntimeSpawner(self.rt.clone())),
                catalog_version,
                view,
            )));
    }

    pub fn state(&self) -> ConnectionState {
        self.status_rx.borrow().state
    }

    pub fn status(&self) -> ConnectionStatus {
        *self.status_rx.borrow()
    }

    pub fn subscribe_status(&self) -> watch::Receiver<ConnectionStatus> {
        self.status_rx.clone()
    }

    /// The main window's boot daemon search began with `timeout`: no never-connected band is held past that search plus a margin, and the 60 s cap no longer applies (TRD S21 trd-3-2-u5 R20 · R24). Nothing once the hold is over. Call once the search really runs.
    pub fn boot_start_began(&self, timeout: Duration) {
        self.link.bound_boot_start(timeout);
    }

    /// The main window's boot start answered (`daemon_start` failed or `daemon_connect` returned): the cold-boot band hold ends (TRD S21 trd-3-2-u5 R20). Call before the answer reaches the frontend, so an attach the frontend then makes routes after the hold ended.
    pub fn boot_start_answered(&self, why: &'static str) {
        self.link.end_boot_start_hold(why);
    }

    /// An ensure attempt made outside the link — `daemon_start` (every window's, boot and popout runs included) or the tray's Start —
    ///   begins: its place in the start-attempt order. Take it before the search; hand it back with the answer
    ///   ([`Self::set_start_refusal`]). ADR-0399: the latest-started attempt's answer wins.
    pub fn begin_start_attempt(&self) -> StartAttempt {
        self.link.begin_start_attempt()
    }

    /// The answer of ensure attempt `attempt` about the data folder's drive: `Some` = refused before a spawn, so the band shows that
    ///   cause until a later-started attempt answers, a connect or a daemon's refusal replaces it, or a close; `None` = clears it
    ///   (ADR-0399). Ignored when a later-started attempt or a close was applied already. The link's own search sets it itself
    ///   (`LinkClient::connect`).
    pub fn set_start_refusal(
        &self,
        attempt: StartAttempt,
        refusal: Option<crate::discovery::VolumeRefusal>,
    ) {
        self.link.set_start_refusal(attempt, refusal);
    }

    /// Usage-interest state — the webview pull reads the cache. The side that changes the interest calls the recompute set ([`Self::usage_layout_changed`] · [`Self::usage_window_destroyed`] · [`Self::usage_window_visibility`]) instead, where result handling (nudge · shrink timer) lives. Lock order: [`SharedUsageInterest`].
    pub fn usage_interest(&self) -> &SharedUsageInterest {
        &self.usage
    }

    pub(crate) fn refusal_alerts(&self) -> &RefusalAlerts {
        &self.refusals
    }

    /// Explicit connect (ADR-0021 §1 — may start a daemon; wsTransport `start()`). Contract: `LinkClient::connect`.
    pub async fn connect(&self) -> Result<(), HandshakeError> {
        self.link.connect().await
    }

    /// Attach-only connect (ADR-0021 B-1 — never starts a daemon; wsTransport `ensureReady()`). Contract: `LinkClient::ensure`.
    pub async fn ensure(&self) -> Result<(), HandshakeError> {
        self.link.ensure().await
    }

    /// Explicit close (wsTransport `close()`) — no reconnect. Returns at once; every window learns `down` through routing. Contract: `LinkClient::close`.
    pub fn close(&self) {
        self.link.close();
    }

    // ── T6a: invoke command request/reply plane (spawn/kill/interrupt/write/resize/…) ─────────
    /// Sends a side-effect command through the driver and awaits the daemon's reply (matched by request_id); failure as text only — contract · flow in [`Self::send_command_with_origin`].
    pub async fn send_command(&self, cmd: AgentCommand) -> Result<AgentEvent, String> {
        self.send_command_with_origin(cmd)
            .await
            .map_err(|failure| failure.to_string())
    }

    /// Same round trip as [`Self::send_command`] but keeps the failure's origin as a type (`CommandFailure::Daemon` = the daemon replied `Error`, `CommandFailure::Local` = ended shell-side) so callers need not read the wording (ADR-0281); text identical to the sibling's. Every failure inside this function (no request_id · not connected · channel closed · reply lost) is `Local`.
    /// Precondition: the caller already stamped a request_id into `cmd` (commands/agent.rs fills `RequestId::new()`), so the reply-matching key is known to the caller too and fits idempotency (same key on retry).
    pub async fn send_command_with_origin(
        &self,
        cmd: AgentCommand,
    ) -> Result<AgentEvent, protocol_state::CommandFailure> {
        self.link.send_command(cmd).await
    }

    // ── T6b: fire-and-forget plane (Subscribe/Unsubscribe/Resize — no reply) ─────────────
    /// Writes `Unsubscribe` on the open connection behind earlier unsubscribes and replay requests; with none open it does nothing (the daemon drops a connection's subscriptions with it). Never blocks (the layout calls it under its lock). Contract: `LinkClient::unsubscribe`.
    pub fn unsubscribe(&self, agent_id: AgentId) {
        self.link.unsubscribe(agent_id);
    }

    /// The user's `ui.language` changed (or the app started) — the daemon's own dialogs follow it (ADR-0372). Never blocks; the
    ///   value is kept and sent on every connection the client opens.
    pub fn set_ui_language(&self, code: &str) {
        self.link.set_ui_language(code);
    }

    /// A command without a reply (Resize …); `agent_resize` uses it. Never blocks; with no connection it is dropped (ADR-0195) and the frontend re-issues a lost Resize. Contract: `LinkClient::send_fire_and_forget`.
    pub fn send_fire_and_forget(&self, cmd: AgentCommand) {
        self.link.send_fire_and_forget(cmd);
    }

    /// Usage interest may have changed — one set-less nudge to the connection task (TRD S21 usage-limit-slot §1-7). The task reads the current interest via [`UsageInterest::sync`] when it takes the nudge out, so the last nudge on the socket carries the latest interest in whatever order they are handled. Do not revive reading the set here and carrying it: interleaved changes would deliver the old set last (TRD §3 #74).
    /// Non-blocking, so callable inside the [`SharedUsageInterest`] lock. With no connection open it does nothing — the next one gets the whole interest on open (`LinkClient::nudge_usage`).
    pub fn nudge_usage_interest(&self) {
        self.link.nudge_usage();
    }

    /// The layout changed — recompute the usage interest and nudge or start the shrink timer per the result (TRD S21 usage-limit-slot §1-7). Called by the layout write's subscription resync (`layout::SubscriptionSync`). Called inside the ViewManager lock (order: [`SharedUsageInterest`]); it only computes in memory, nudges without blocking and starts a timer task, keeping that port's inside-the-lock · synchronous · non-blocking contract — as do [`Self::usage_window_destroyed`] · [`Self::usage_window_visibility`].
    pub fn usage_layout_changed(&self, mgr: &ViewManager) {
        let action = self.usage.lock().recompute(mgr, Instant::now());
        self.run_usage_action(action);
    }

    /// Popup window `label` is gone — called inside the same ViewManager lock that removed it from the model; it forgets that label's hidden flag whether or not the model held the window ([`UsageInterest::forget_window`]).
    pub fn usage_window_destroyed(&self, label: &str, mgr: &ViewManager) {
        let action = self.usage.lock().forget_window(label, mgr, Instant::now());
        self.run_usage_action(action);
    }

    /// The shell hid (`false`) or re-showed (`true`) window `label` — tray show/hide takes the ViewManager lock briefly per window and calls this (the OS window call is outside that lock).
    pub fn usage_window_visibility(&self, label: &str, visible: bool, mgr: &ViewManager) {
        let action = self
            .usage
            .lock()
            .set_visible(label, visible, mgr, Instant::now());
        self.run_usage_action(action);
    }

    /// The layout changed — what the daemon is told about the PC list slot may change too (ADR-0394, ADR-0410). Called inside the
    ///   ViewManager lock next to [`Self::usage_layout_changed`]; never blocks. The want is queued under the peer-watch lock, so two wants
    ///   reach the driver in the order they were decided.
    pub fn peer_layout_changed(&self, mgr: &ViewManager) {
        let mut watch = sync::lock(&self.peer);
        if let Some(want) = watch.layout_changed(mgr) {
            self.link.set_peer_want(want);
        }
    }

    // The single place that handles the recompute result — the usage-interest lock is already released (a nudge carries no set, so order does not interleave outside the lock — [`Self::nudge_usage_interest`]).
    fn run_usage_action(&self, action: InterestAction) {
        match action {
            InterestAction::Unchanged => {}
            InterestAction::Nudge => self.nudge_usage_interest(),
            InterestAction::Defer { gen, deadline } => {
                // Build the sleep to the deadline at the call site with this runtime's clock — inside a task it would shift the deadline by its first poll, and in a fake-clock test by the whole advance.
                let delay = deadline.saturating_duration_since(Instant::now());
                let sleep = {
                    let _rt = self.rt.enter();
                    tokio::time::sleep(delay)
                };
                tracing::debug!(
                    ?gen,
                    delay_ms = delay.as_millis() as u64,
                    "usage interest: shrink pending — nudge at the deadline"
                );
                // Holds the shared interest and a nudger that does not keep the actor alive (a nudge after the client is gone does nothing).
                let usage = self.usage.clone();
                let nudger = self.link.usage_nudger();
                self.rt.spawn(async move {
                    sleep.await;
                    if !usage.lock().deferral_elapsed(gen) {
                        tracing::debug!(
                            ?gen,
                            "usage interest: shrink deadline — reverted or a new generation started in the meantime, no nudge"
                        );
                        return;
                    }
                    tracing::debug!(?gen, "usage interest: shrink deadline — nudge");
                    nudger.nudge();
                });
            }
        }
    }

    /// View-driven replay (ADR-0046 M1, single flight): the generation the view fences its flush with. `Err` when no connection is open, no window shows the agent or its `Subscribe` could not be written. Contract: `LinkClient::request_replay`.
    pub async fn request_replay(&self, agent_id: AgentId) -> Result<u64, String> {
        self.link.request_replay(agent_id).await
    }
}

impl Drop for DaemonClient {
    fn drop(&mut self) {
        // The link driver ends with its `LinkClient` (dropped with the fields below).
        // Shut the dedicated runtime down here without blocking: the fields drop next, and a tokio `Runtime`'s default drop blocks until the workers stop and panics inside an async context — a panic inside `Drop` during unwinding aborts the process with no diagnostics. `shutdown_background` detaches the workers and returns at once in any context.
        // Do not rest on "unreachable today": every test constructor has `_owned_rt: None` and the production client is never dropped (an `Arc` cycle: `lib.rs` bakes the client's own `Arc` into the command table installed by `install_command_table`) — that harmlessness is an accident, not structure (one more runtime-owning constructor, or breaking the cycle, ends it), so it is blocked by structure.
        // Shut down only when this is the last reference — a remaining `Arc` means another owner still uses that runtime.
        if let Some(rt) = self._owned_rt.take() {
            if let Ok(rt) = Arc::try_unwrap(rt) {
                rt.shutdown_background();
            }
        }
    }
}

#[cfg(test)]
mod test_support;
