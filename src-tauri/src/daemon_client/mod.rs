//! DaemonClient — the shell's single authority over the daemon WS connection (S14 module ①, ADR-0036).
//!
//! **One connection to the daemon however many windows** — the frontend reaches the daemon through the shell, not
//! with a WS per window (`src/api/wsTransport.ts`). This module owns that connection's setup, handshake and life.
//! Protocol semantics (epoch guard · request id extraction · reply classification) are `protocol_state.rs`, output
//! routing is `output_router` / `output_channel`.
//!
//! - The connection runs on the driver in `connection.rs` over the transport crate (`link/`) — its module doc holds
//!   the concurrency model (one actor owns the routing; callers' round trips run on their own tasks).
//! - `connect` (explicit, may spawn the daemon) and `ensure` (attach-only, never spawns) stay apart — ADR-0021.
//! - States: connected / down / reconnecting — there is no connecting state.

// ADR-0310: the driver on the transport crate (TRD S21 trd-3 §6-2 U3) — every client runs on it.
mod connection;
// ADR-0012: the seam for the connection's emits to the frontend (real AppHandle / recording fake / no-op). Without it
// this module's unit harness cannot stand — the reasons and contract live in that file's header.
// ★Being `pub(crate)` is the point★: its consumers are the connection drivers inside this crate; opening it would
//   leak the `AppHandle` carrier onto the lib surface with nowhere outside the crate to plug it in.
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
// ADR-0343: the one stop path (single flight · stopping mode · identity-checked kill) — TRD S21 trd-3-2-u5 §3-3.
pub mod shutdown;
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

pub use connection::HandshakeError;
use events::{DaemonEvents, TauriEmitter};
// The assembly that discards notifications is harness-only (`events::NoDaemonEvents` doc) — it does not exist in production builds at all.
#[cfg(test)]
use events::NoDaemonEvents;
use inbound::InboundSlot;
use link::endpoint::LocalEndpointSource;
use link::DaemonKey;
use refusal::{RefusalAlerts, TauriRefusalBox};
use shutdown::{StopPorts, Stopper};
// The assembly that discards the box is also harness-only (`refusal::NoRefusalNotice` doc).
#[cfg(test)]
use refusal::{NoRefusalNotice, RefusalNotice};
use usage_interest::{InterestAction, UsageInterest};

use crate::layout::ViewManager;
use crate::output_channel::WindowChannelRegistry;
use crate::output_router::OutputRouter;

/// The connection's lifetime state. On the link driver, routing (`link/route.rs`) is its only emitter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionState {
    /// Not attached, explicitly closed, refused, or a never-connected handle's first failure (the link may still retry
    /// in the background — ADR-0346, TRD rev 9 ①).
    Down,
    // `Hello` received = authenticated. Commands and subscriptions work.
    Connected,
    // Retrying after an unintended loss (backoff … next attempt); the link driver never gives up on network failures.
    // Matches wsTransport's `reconnecting` — distinct from an explicit close (Down).
    Reconnecting,
}

/// The shared handle to one shell's usage interest ([`UsageInterest`]) — the connection driver, the side that changes
/// the interest (layout recompute · hide · shrink timer) and the webview pull hold the same one (TRD S21
/// usage-limit-slot §1-7).
///
/// ★Lock order = `ViewManager → this lock`★. On the link driver a nudge ([`DaemonClient::nudge_usage_interest`])
/// issued inside this lock only sends to the driver's actor (non-blocking); the driver takes the ViewManager lock
/// ([`AgentNamesSink`]) holding no lock, so there is no reverse order. Inside this lock only [`UsageInterest`] methods,
/// that nudge and the link driver's usage-subscribe queueing run; ★emits happen after the lock is released★ (ADR-0006). The link
/// driver queues its usage subscribe inside this lock on purpose, so the order sets are read is the order they are
/// queued — that queueing (`Peer::notify_in`) is a non-blocking leaf, and the peer is looked up before the lock is
/// taken.
///
/// ★Poison is recovered from★: if a panic inside that method leaves the state half-fixed, the outcome is a subscription out of
/// step with the daemon or a missed delivery target, and the next recompute · snapshot reconciliation · new socket sets it right
/// again. Propagating the poison would kill the connection task on the spot and stop output and commands altogether.
#[derive(Clone, Default)]
pub struct SharedUsageInterest(Arc<Mutex<UsageInterest>>);

impl SharedUsageInterest {
    pub fn lock(&self) -> MutexGuard<'_, UsageInterest> {
        sync::lock(&self.0)
    }
}

/// Port that receives the name part of the daemon's `AgentListUpdated` — it fills the agent-name ledger that tab auto-names read (TRD
/// S26-window-chrome §4-3). The real one = the shell assembly (`lib.rs`) calling the layout apply service `layout::apply::agent_names_updated`;
/// harness = discard (`NoAgentNames`) · recording type.
///
/// - A sibling cut separately without widening the notification port `events::DaemonEvents` (its doc forbids widening) — that port only
///   notifies the frontend, while the real one of this port edits shell state (the ledger inside the layout lock).
/// - ★The connection driver calls it holding no lock★ — the real one takes the layout lock, and inside that lock the
///   usage interest lock is taken ([`SharedUsageInterest`] doc); calling it while holding that lock is the reverse
///   order.
/// - The implementation never blocks except for the short critical section of the layout lock, and it swallows failures — failing to edit the ledger is not
///   a reason to roll the connection back, and the next list refills it.
pub trait AgentNamesSink: Send + Sync {
    /// Called with each `AgentListUpdated`, passing that list as is — after the frontend notification (`agent_list_updated`) has been emitted.
    fn agents(&self, agents: &[AgentInfo]);
}

/// The assembly that discards the name-ledger port — ★harness-only★. The reason it is `#[cfg(test)]` is the same as `events::NoDaemonEvents` —
/// in a production build it would open a way to quietly assemble a client whose tab names stay at 「에이전트」 (agent) even when the daemon list arrives.
#[cfg(test)]
pub(crate) struct NoAgentNames;

#[cfg(test)]
impl AgentNamesSink for NoAgentNames {
    fn agents(&self, _agents: &[AgentInfo]) {}
}

/// The endpoint source of [`DaemonClient::unconnected`] — no daemon, and none is ever started.
#[cfg(test)]
struct NoDaemonAnywhere;

#[cfg(test)]
impl link::endpoint::EndpointSource for NoDaemonAnywhere {
    fn ensure(&self, _timeout: Duration) -> Result<link::endpoint::Endpoint, String> {
        Err("this test client never finds a daemon".to_owned())
    }

    fn current(&self) -> link::endpoint::Current {
        link::endpoint::Current::Absent
    }
}

// Default discover (spawnable) timeout (aligned with wsTransport discover_daemon 5s).
const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// The thread name of the client's own runtime — blocking waits on that runtime must not run on its threads
/// (`shutdown::DaemonClient::stop_daemon_blocking`).
pub(crate) const CLIENT_THREAD_NAME: &str = "engram-daemon-client";

// TRD S21 trd-3 §5 15 (C10): one ws message or frame on the daemon link is at most 16 MiB — the shell's side; the
//   daemon's accept side takes the same value in stage 3-3. The largest legitimate message is not measured.
const DAEMON_MESSAGE_MAX_BYTES: usize = 16 << 20;

/// The text a caller gets when no connection takes commands right now (ADR-0195) — "never connected",
/// "lost and reconnecting" and "explicitly closed" **all** fall into this one. The text does not tell them apart
/// because the caller's next step is the same for each.
///
/// ★It does not say "connect first"★ — while the link is reconnecting, a caller following that would re-add the
/// link and cut the recovery in progress. This text reaches LLM callers through the control surface (§5), so a
/// wrong next step written here is one they would actually take. ADR-0195 decided that the caller picks when to
/// retry, and the text says only that.
pub(crate) const NOT_CONNECTED: &str =
    "데몬에 연결되어 있지 않음 — 지금은 명령을 받지 않는다(재시도 시점은 호출자가 정한다)";

/// Something that should take the command is gone — the driver's actor stopped while a caller waited for the
/// connection to open (`connection.rs`), or a write found the peer's queue closed (`link::errors::send_error_text`
/// for `SendError::Closed`). A **different fact** from [`NOT_CONNECTED`] (no connection right now).
pub(crate) const CHANNEL_GONE: &str = "연결 task 가 명령을 받지 못함(끊김)";

/// A replay request that wrote no `Subscribe` and got no generation — the link dropped, or no window shows the agent.
pub(crate) const REPLAY_UNSENT: &str =
    "replay 요청 미전송(연결 끊김 · 그 에이전트를 보이는 창 없음) — 프론트 재요청 안전";

/// A round trip asked for a command that takes no reply (no `request_id`) — read by the link's error texts
/// (`link::errors`).
pub(crate) const NO_REQUEST_ID: &str =
    "send_command: request_id 없는 명령은 reply 를 기대할 수 없다";

// The single handle to the daemon connection, shared by invoke handlers, the tray and state subscribers
// (`Arc<DaemonClient>`).
//
// The link driver (`connection`) owns the connection: its actor owns the routing and the transport peer owns the
// socket; this struct holds the driver's caller side (`link`) and the state watch (`state_rx`) — never the stream.
pub struct DaemonClient {
    // Runtime handle on which to spawn the connection task. Production = dedicated multi-thread (`_owned_rt`), tests = current runtime.
    rt: Handle,
    // ★Owns a dedicated runtime (production — T6a)★. The `setup` callback may not be in a tokio runtime context, so
    // `Handle::current()` panics. So the production constructor (`new_real_with_owned_runtime`) follows spike §2
    // "tokio multi-thread (like the daemon)" and *builds* a dedicated multi-thread runtime *itself*, using its Handle.
    // This field keeps the runtime alive — if it is dropped the Handle becomes invalid and the connection task dies, so it is tied to the DaemonClient
    // lifetime (`Arc<DaemonClient>` is the app lifetime). `None` = external handle injected (tests = current runtime).
    _owned_rt: Option<Arc<tokio::runtime::Runtime>>,
    /// The filter for the command-registration refusal box (ADR-0281). Production = `refusal::TauriRefusalBox`,
    /// harness = `refusal::NoRefusalNotice` or a recording fake ([`Self::new_on_link`]).
    /// ★The client holds one and shares it with the driver — never make one per connection★: repeat suppression lives
    /// as long as the shell process (`RefusalAlerts` doc), so one per connection re-shows the box on every reconnect
    /// that gets the same refusal.
    refusals: Arc<RefusalAlerts>,
    /// The connection state, read without a lock — the driver's watch, which routing publishes. Only borrowed here.
    state_rx: watch::Receiver<ConnectionState>,
    /// The driver's output routing — tests set which windows show which agent here.
    #[cfg(test)]
    router: Arc<OutputRouter>,
    /// The driver's window output channels — tests register a window here.
    #[cfg(test)]
    registry: WindowChannelRegistry,
    /// ★The entry for commands the daemon delivers★ (ADR-0155 decision 4). Filled late — the table's spawn port holds
    ///   this client, so the assembly has a cycle (`inbound::InboundSlot` doc). Shared with the driver.
    inbound: Arc<InboundSlot>,
    /// The usage interest — outlives every connection (the interest comes from the layout, not the socket). Shared
    ///   with the driver.
    usage: SharedUsageInterest,
    /// The driver on the transport crate (`connection`) — connect, ensure, close, the command round trips,
    /// fire-and-forget sends, unsubscribe, replay requests and usage nudges go there; `state_rx` is its watch.
    link: connection::LinkClient,
    /// The one stop path's single flight (`shutdown`) — it closes and re-adds `link` and owns its stopping mode.
    stopper: Arc<Stopper>,
}

impl DaemonClient {
    /// A client on the link driver over a test network (TRD S21 trd-3 §6-2 U3): `inner` dials the address the endpoint
    /// `source` resolves (a memory network), `clock` drives the transport. The assembly is production's
    /// ([`Self::on_link`]); this client's `router` and `registry` are its own, empty ones. ★The ports go in here★ — the
    /// driver starts with this call and keeps what it got.
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

    /// [`Self::new_on_link`] with the stop path's ports — a test that stops a daemon plugs its fakes in here.
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

    /// A client that is never connected — its endpoint source finds no daemon, so `connect` / `ensure` fail and every
    /// command fails as not connected. For tests of callers that do not connect.
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

    /// Starts the link driver on `rt` and builds the client around it. Every connection path of the client runs on
    /// the driver, and the driver's ports are this client's own objects (`router`, `registry`, command table slot,
    /// refusal filter, usage interest), so [`Self::install_command_table`] · [`Self::refusal_alerts`] ·
    /// [`Self::usage_interest`] see what the driver sees.
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

    /// The client around the link `start` makes from this client's ports — [`Self::on_link`] starts the driver; a test
    /// may build one it never starts.
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
        let stopper = Stopper::new(rt.clone(), stop, link.control(), link.stopping());
        Self {
            rt,
            _owned_rt: owned_rt,
            state_rx: link.state_rx(),
            #[cfg(test)]
            router,
            #[cfg(test)]
            registry,
            inbound,
            refusals,
            usage,
            link,
            stopper,
        }
    }

    /// ★The production constructor★ — `lib.rs` `setup` calls it. The client owns a multi-thread runtime it builds here,
    /// because `setup` may run outside a tokio context, where `Handle::current()` panics (spike §2); the runtime lives
    /// as long as the client (`_owned_rt`). It runs on the link driver: the ws dialer, the local daemon's endpoint
    /// record (`daemon.json` in the default data folder) and the system clock. `Err` = the runtime could not be built;
    /// the caller reports it and goes on without daemon commands.
    ///
    /// `router` and `registry` are the shared instances `setup` also hands the layout commands and `subscribe_output`,
    /// so the output fan-out and those commands see the same ones.
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
        Ok(Self::on_local_daemon(
            handle,
            Some(Arc::new(rt)),
            crate::discovery::default_data_dir(),
            Arc::new(TauriEmitter(app.clone())),
            names,
            Arc::new(RefusalAlerts::new(Arc::new(TauriRefusalBox(app)))),
            router,
            registry,
        ))
    }

    /// [`Self::on_link`] on the local daemon of `data_dir`: the ws dialer, that folder's endpoint record
    /// (`daemon.json`) and the system clock — production's transport, which a test drives on a temporary folder. The
    /// stop path reads the same folder (`StopPorts::local`).
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

    /// Plug in the command table this client can execute — ★call it before connecting★ (ADR-0155 decisions 4·5).
    ///
    /// ★Choosing the runtime here is the point★: the apply tasks must run on the **same runtime** as the connection
    /// driver — while `agent.spawnInto` waits on a daemon round trip, the driver's tasks must keep running for the
    /// answer to arrive (on another runtime that interleaving is left to chance). Letting each assembly pick the
    /// `Handle` would make every assembly re-keep that invariant, so the client decides with its own `rt`.
    /// Calling it late is allowed (the first winner wins), but **after a connection opened**, envelopes that arrived
    /// meanwhile get a "no table" error reply, and that connection sends no registration — the next connection does
    /// (the driver registers on every `LinkUp`).
    /// `view` = where names absent from the shell table go (the webview's share — TRD §3-8 hop ③). It is plugged in
    /// **at the same spot** as the table because the registration packet carries both layers in one go (TRD §3-7
    /// clause 2) — plugging them separately lets some assembly send a registration without the webview's share, and on
    /// that connection every view command is `UNKNOWN_COMMAND`.
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

    // Snapshot of the current connection state (lock-free).
    pub fn state(&self) -> ConnectionState {
        *self.state_rx.borrow()
    }

    // Subscribe to state changes (watch). The caller awaits the next transition or looks at the current value.
    pub fn subscribe_state(&self) -> watch::Receiver<ConnectionState> {
        self.state_rx.clone()
    }

    /// Usage-interest state — the webview pull reads the cache. The side that changes the interest does not use this handle but calls the recompute set
    /// ([`Self::usage_layout_changed`] · [`Self::usage_window_destroyed`] · [`Self::usage_window_visibility`]) —
    /// result handling (nudge · shrink timer) lives there in one place. For lock order see the [`SharedUsageInterest`] doc.
    pub fn usage_interest(&self) -> &SharedUsageInterest {
        &self.usage
    }

    /// The **same** filter as the registration spot of the connection task — used by the diff spot (`commands::view_bus`).
    pub(crate) fn refusal_alerts(&self) -> &RefusalAlerts {
        &self.refusals
    }

    /// Explicit connect (ADR-0021 §1 — may start a daemon; wsTransport `start()`). Contract: `LinkClient::connect`.
    pub async fn connect(&self) -> Result<(), HandshakeError> {
        self.link.connect().await
    }

    /// Attach-only connect (ADR-0021 B-1 — never starts a daemon; wsTransport `ensureReady()`). Contract:
    /// `LinkClient::ensure`.
    pub async fn ensure(&self) -> Result<(), HandshakeError> {
        self.link.ensure().await
    }

    /// Explicit close (wsTransport `close()`) — the link does not reconnect. Returns at once; every window learns `down`
    /// through routing. Contract: `LinkClient::close`.
    pub fn close(&self) {
        self.link.close();
    }

    // ── T6a: invoke command request/reply plane (spawn/kill/interrupt/write/resize/…) ─────────
    // Sends a side-effect command through the connection driver and awaits the daemon's reply (matched by request_id).
    /// Sibling of [`Self::send_command_with_origin`] that returns a failure only as text — contract · flow · remainder are written there.
    pub async fn send_command(&self, cmd: AgentCommand) -> Result<AgentEvent, String> {
        self.send_command_with_origin(cmd)
            .await
            .map_err(|failure| failure.to_string())
    }

    /// Same round trip as [`Self::send_command`], but it keeps the **origin** of the failure as a type — used by callers that must tell whether the daemon replied with `Error`
    /// (`CommandFailure::Daemon`) or the failure ended on the shell side (`CommandFailure::Local`) without reading the wording
    /// (ADR-0281). The text is byte-for-byte the same as the sibling's (`Display` of `CommandFailure`).
    /// Every failure inside this function (no request_id · not connected · channel closed · reply lost) is `Local`.
    ///
    /// ★Call precondition — the caller already stamps a request_id into `cmd`★ (the builder in commands/agent.rs fills it with
    /// `RequestId::new()`). That way the reply-matching key is known to the caller too and fits idempotency (same
    /// key on retry) — if this round trip filled it arbitrarily the caller would not know the key.
    pub async fn send_command_with_origin(
        &self,
        cmd: AgentCommand,
    ) -> Result<AgentEvent, protocol_state::CommandFailure> {
        self.link.send_command(cmd).await
    }

    // ── T6b: fire-and-forget plane (Subscribe/Unsubscribe/Resize — no reply) ─────────────
    /// Writes `Unsubscribe` on the open connection, behind earlier unsubscribes and replay requests; with none open it
    /// does nothing — the daemon drops a connection's subscriptions with it. Never blocks (the layout calls it under its
    /// lock). Contract: `LinkClient::unsubscribe`.
    pub fn unsubscribe(&self, agent_id: AgentId) {
        self.link.unsubscribe(agent_id);
    }

    /// A command without a reply (Resize …) — `agent_resize` uses it. Never blocks; with no connection to take it the
    /// command is dropped (ADR-0195) and the frontend re-issues a lost Resize. Contract:
    /// `LinkClient::send_fire_and_forget`.
    pub fn send_fire_and_forget(&self, cmd: AgentCommand) {
        self.link.send_fire_and_forget(cmd);
    }

    /// Usage interest may have changed — one **set-less** nudge to the connection task (TRD S21 usage-limit-slot §1-7
    /// 「한 직렬 경로」 (one serial path)). The set to send is read by the connection task as the **current** interest via [`UsageInterest::sync`] when it takes the nudge out —
    /// so however many nudges there are and in whatever order they are handled, the last one on the socket carries the latest interest. If the set were read here
    /// and carried along, when two changes interleave between the read and the send the old set would arrive last — do not revive that
    /// (TRD §3 #74).
    ///
    /// It is non-blocking, so it may be called inside the [`SharedUsageInterest`] lock (lock order in that doc).
    /// With no connection open it does nothing — the next one gets the whole interest when it opens
    /// (`LinkClient::nudge_usage`).
    pub fn nudge_usage_interest(&self) {
        self.link.nudge_usage();
    }

    /// The layout changed — recompute the usage interest and, per the result, nudge or start the shrink timer (TRD S21
    /// usage-limit-slot §1-7 「재계산 자리」 (the recompute spot)). Called by the layout write's subscription resync (`layout::SubscriptionSync`).
    ///
    /// ★Called inside the ViewManager lock★ — lock order is as in the [`SharedUsageInterest`] doc, and all it does is in-memory
    /// computation · a non-blocking nudge · starting a timer task, so it keeps that port's 「락 안 · 동기 · 비블로킹」 (inside the lock · synchronous · non-blocking) contract. The other two
    /// siblings ([`Self::usage_window_destroyed`] · [`Self::usage_window_visibility`]) are the same.
    pub fn usage_layout_changed(&self, mgr: &ViewManager) {
        let action = self.usage.lock().recompute(mgr, Instant::now());
        self.run_usage_action(action);
    }

    /// Popup window `label` is gone — called inside the same ViewManager lock that removed the window from the model. Whether or not the model
    /// held that window, it forgets that label's hidden flag ([`UsageInterest::forget_window`]).
    pub fn usage_window_destroyed(&self, label: &str, mgr: &ViewManager) {
        let action = self.usage.lock().forget_window(label, mgr, Instant::now());
        self.run_usage_action(action);
    }

    /// The shell hid (`false`) or re-showed (`true`) window `label` — tray show/hide takes the ViewManager lock briefly per window
    /// and calls this (the OS window call is outside that lock).
    pub fn usage_window_visibility(&self, label: &str, visible: bool, mgr: &ViewManager) {
        let action = self
            .usage
            .lock()
            .set_visible(label, visible, mgr, Instant::now());
        self.run_usage_action(action);
    }

    // The single place that handles the recompute result — the usage-interest lock is already released (a nudge carries no set, so even outside the lock the order
    // does not interleave — [`Self::nudge_usage_interest`]).
    fn run_usage_action(&self, action: InterestAction) {
        match action {
            InterestAction::Unchanged => {}
            InterestAction::Nudge => self.nudge_usage_interest(),
            InterestAction::Defer { gen, deadline } => {
                // Build the sleep until the deadline **at the call site**, with this runtime's clock — building it inside a task shifts the deadline by
                //   its first poll, and in a fake-clock test it is built after the clock was advanced, so the deadline is shifted wholesale.
                let delay = deadline.saturating_duration_since(Instant::now());
                let sleep = {
                    let _rt = self.rt.enter();
                    tokio::time::sleep(delay)
                };
                tracing::debug!(
                    ?gen,
                    delay_ms = delay.as_millis() as u64,
                    "사용량 관심: 줄임 대기 — 기한에 넛지"
                );
                // Holds the shared interest and a nudger that does not keep the actor alive — not the client (a nudge
                //   after the client is gone does nothing).
                let usage = self.usage.clone();
                let nudger = self.link.usage_nudger();
                self.rt.spawn(async move {
                    sleep.await;
                    if !usage.lock().deferral_elapsed(gen) {
                        tracing::debug!(
                            ?gen,
                            "사용량 관심: 줄임 기한 — 그 사이 무르거나 새 세대가 섰다, 넛지 없음"
                        );
                        return;
                    }
                    tracing::debug!(?gen, "usage interest: shrink deadline — nudge");
                    nudger.nudge();
                });
            }
        }
    }

    /// View-driven replay (ADR-0046 M1 — single flight): the generation the view fences its flush with. `Err` when no
    /// connection is open, no window shows the agent or its `Subscribe` could not be written. Contract:
    /// `LinkClient::request_replay`.
    pub async fn request_replay(&self, agent_id: AgentId) -> Result<u64, String> {
        self.link.request_replay(agent_id).await
    }
}

impl Drop for DaemonClient {
    fn drop(&mut self) {
        // The link driver ends with its `LinkClient` (dropped with the fields below).
        // ★The dedicated runtime is shut down here **without blocking**★ — when this function body ends the fields are dropped
        //   next, and among them [`Self::_owned_rt`] is a tokio `Runtime`. A `Runtime`'s default drop blocks until the workers
        //   stop, and **if that drop happens inside an async context tokio panics**. A panic inside `Drop`
        //   aborts the process if it happens during unwinding — an end that dies with no diagnostics.
        //   `shutdown_background` detaches the workers and returns at once, so it gives the same result in any context.
        //   That is exactly what a destructor wants (no waiting + no panic).
        // ★Do not hang this on "it cannot be reached today"★: there is no path by which this spot reaches that combination today — every test
        //   constructor has `_owned_rt: None`, and the production client is never dropped in the first place, because of an `Arc` cycle (`lib.rs` bakes the client's own `Arc`
        //   into the command table and plugs that table into this client with `install_command_table`). But that
        //   harmlessness is **an accident, not structure** (it disappears the day one more constructor that owns a runtime is
        //   added, or the cycle is broken). So it is blocked by structure.
        // ★Shut down only when this is the last reference★ — if an `Arc` remains, another owner is still using that runtime.
        if let Some(rt) = self._owned_rt.take() {
            if let Ok(rt) = Arc::try_unwrap(rt) {
                rt.shutdown_background();
            }
        }
    }
}

#[cfg(test)]
mod test_support;
