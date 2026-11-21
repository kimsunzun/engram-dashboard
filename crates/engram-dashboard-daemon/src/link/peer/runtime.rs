//! The running peer link — [`PeerLink`]: the trust operations over the peer state ([`PeerStore`]), the link endpoint's lifecycle, the
//!   accept gate, one transport registry of PeerWire links (dialing handles and accepted links) and the state `peer.list` and the app read.
//!
//! Lifecycle (ADR-0389, TRD D7 · R13): the endpoint runs only while the peer status is ready (a loaded key, the identity not held by another copy, a
//!   verified list) and at least one PC is listed. Every trust operation re-publishes the state and the runtime brings the endpoint up,
//!   tears it down, or restarts it when the identity or the network settings changed. Building it runs the link adapter's check of the
//!   stored helper URLs first — ★a refused URL is an endpoint error, never a fallback to n0's servers★ — and a bind failure (e.g. the port
//!   in use) is retried every [`BIND_RETRY`] (right after a restart, first quickly for about 3 s). The link book's duplicate evidence
//!   survives a restart. While the identity is only transiently unreadable, held by another copy, or the list file
//!   could not be read, the store is retried with a backoff.
//!
//! Who does what:
//! - ★Trust operations are blocking★ (disk, DPAPI) — call them on the blocking pool; they finish without waiting for any link (endpoint and
//!   link results show in [`PeerLink::snapshot`]). The store's lock is held only by them and by the start-up load; nothing async takes it.
//! - ★Removal bites before the save★ (ADR-0390 decision 4): the PC leaves the published trust first, so the accept gate and the
//!   per-frame check refuse it at once, its links are cut as refused (no flush, no wait for acknowledgements), then the list is saved.
//! - One actor task owns the endpoint, the registry and the handles, and carries out the link book's decisions ([`super::book`]). One
//!   drain task per handle reads its inbox to the end (never awaiting a request): it checks every frame against the current trust,
//!   answers `Probe`, sends the acceptor's `Hello` when an accepted link comes up, records a `Duplicate` warning in the book, and hands
//!   events and a `ProbeAck` for an open probe to the actor ([`route`]) — so a broken peer repeating frames cannot fill the actor's
//!   queue. Other PeerWire messages are not handled yet and are dropped with a log line.
//! - Lock order: the store lock before the published state; the link book, the refused-knock list and the endpoint view are each taken
//!   alone and briefly (the handshake and the accept gate take theirs on transport tasks).
//!
//! Not here: who may call which operation and asking a human (approvals), `peer.*` commands, mail and the directory.

// ADR-0388, ADR-0389, ADR-0390, ADR-0401

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant, SystemTime};

use engram_dashboard_base::sync::{lock, read, write};
use engram_dashboard_transport::iroh::{
    self as adapter, IrohAcceptPolicy, IrohConfig, IrohDialer, IrohEndpoint, IrohListener,
};
use engram_dashboard_transport::{
    with_deadline, Accepted, Address, Clock, CloseCode, DialFailure, Dialer, GaveUpReason, Inbox,
    Incoming, LinkError, LinkRx, LinkTx, Listener, Peer, PeerId as Label, PeerState, Policy,
    ReconnectPolicy, Registry, SystemClock, TransportEvent,
};
use futures_util::future::BoxFuture;
use tokio::sync::{mpsc, oneshot, watch};
use tokio::task::JoinHandle;

use super::book::{Action, CloseAs, Dir, LinkBook, LinkKey, Online, Warning};
use super::gate::{Knock, Knocks, LogLine};
use super::handshake::{HandshakeNotes, PeerHandshake, Role};
use super::id::PeerId;
use super::identity::Secret;
use super::store::{PeerEdit, PeerError, PeerStatus, PeerStore, Regenerated, Removal};
use super::trusted::{Helper, ListRefusal, NetworkSettings, Permission, TrustedPeer};
use super::wire::{Instance, PeerMsg, PeerWire, ALPN, MAX_FRAME_BYTES, PEER_WIRE_VERSION};

/// A bind that failed is tried again after this.
pub(crate) const BIND_RETRY: Duration = Duration::from_secs(60);
/// Right after an endpoint restart a failed bind is first retried this often, this many times (about 3 s) — the closed endpoint may
///   still hold the fixed port for a moment — before falling back to [`BIND_RETRY`].
const QUICK_BIND_RETRY: Duration = Duration::from_millis(250);
const QUICK_BIND_TRIES: u32 = 12;
/// [`PeerLink::stop`] waits at most this for the links and the endpoint to close; what is left is cut.
const STOP_DEADLINE: Duration = Duration::from_secs(3);
/// The store retry's backoff while the identity cannot be used for now.
const IDENTITY_RETRY_FIRST: Duration = Duration::from_secs(5);
const IDENTITY_RETRY_MAX: Duration = Duration::from_secs(5 * 60);
/// How long a stop or an endpoint restart lets the links send what they hold before they close.
const FLUSH: Duration = Duration::from_secs(2);
/// The accepting side: QUIC + TLS handshake, then the stream and preamble, then the `Hello` (the transport's auth deadline).
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
const STREAM_ACCEPT_TIMEOUT: Duration = Duration::from_secs(10);
const ACCEPT_PERMITS: usize = 16;
/// Dialing (Reason Network `net.rs`: 20 s connect, backoff 2 → 60 s), without end (the shell link's rule, ADR-0346).
const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);
const BACKOFF_BASE: Duration = Duration::from_secs(2);
const BACKOFF_CAP: Duration = Duration::from_secs(60);
/// The `Mail` request's own deadline — the registry's one request timeout (ADR-0388 decision 8).
const MAIL_DEADLINE: Duration = Duration::from_secs(120);

/// The logical dial address of a listed PC — resolved to its key and current direct address at each dial ([`PeerDialer`]).
const DIAL_PREFIX: &str = "peer:";

/// How the endpoint binds.
#[derive(Debug, Clone)]
pub(crate) struct LinkSettings {
    /// The dev build's default port (data root `.engram-dev`) instead of release's.
    pub(crate) dev: bool,
    /// Bind 127.0.0.1 only — in-process tests (nothing leaves the machine, no firewall prompt).
    pub(crate) loopback_only: bool,
}

impl LinkSettings {
    /// A daemon's settings — the dev port in debug builds, which are the ones that run on `.engram-dev`.
    pub(crate) fn daemon() -> Self {
        Self {
            dev: cfg!(debug_assertions),
            loopback_only: false,
        }
    }
}

/// The endpoint as `peer.list` shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum EndpointView {
    /// Not running — no PC is listed, or the peer status keeps the feature off (the status says which).
    Off,
    /// Running on these sockets.
    Up {
        addrs: Vec<SocketAddr>,
    },
    Error(EndpointError),
}

/// Why the endpoint is not running although it should.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum EndpointError {
    /// A stored relay or address-lookup URL the link adapter refuses — the settings must change; n0's servers are never used instead.
    HelperRefused(String),
    /// Binding failed (e.g. the port is in use) — retried every [`BIND_RETRY`]. Not shown during the quick retries after a restart.
    Bind(String),
}

/// One listed PC as `peer.list` shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PeerRow {
    pub(crate) peer: TrustedPeer,
    pub(crate) online: Online,
    pub(crate) warning: Option<Warning>,
}

/// Everything `peer.list` and the app read about the peer link.
#[derive(Debug, Clone)]
pub(crate) struct PeerSnapshot {
    pub(crate) status: PeerStatus,
    /// This PC's ID, once there is a key.
    pub(crate) id: Option<PeerId>,
    /// Why the local identity lock is not held although the key is loaded (an OS failure).
    pub(crate) lock_note: Option<String>,
    pub(crate) network: Option<NetworkSettings>,
    pub(crate) endpoint: EndpointView,
    pub(crate) peers: Vec<PeerRow>,
    /// PCs refused at the accept gate this run, most recent first.
    pub(crate) knocks: Vec<Knock>,
}

/// The state the gate, the dialer and the runtime read — re-published after every trust operation.
#[derive(Clone)]
struct Published {
    status: PeerStatus,
    id: Option<PeerId>,
    peers: Vec<TrustedPeer>,
    network: Option<NetworkSettings>,
    /// A copy of the identity secret for the endpoint — zeroed when the last copy goes.
    key: Option<Arc<Secret>>,
    lock_note: Option<String>,
}

impl Published {
    fn of(store: &PeerStore) -> Self {
        Self {
            status: store.status(),
            id: store.id(),
            peers: store.peers().to_vec(),
            network: store.network().cloned(),
            key: store
                .secret()
                .map(|secret| Arc::new(Secret::from_bytes(*secret.expose()))),
            lock_note: store.lock_note().map(str::to_string),
        }
    }

    /// A listed PC that may link now — only while the feature runs, and only with the `mail` permission.
    fn trusted(&self, id: &PeerId) -> Option<&TrustedPeer> {
        if self.status != PeerStatus::Ready {
            return None;
        }
        self.peers
            .iter()
            .find(|p| p.id == *id && p.permission == Permission::Mail)
    }

    /// What the endpoint should be built from — `None` = no endpoint.
    fn want(&self) -> Option<Want> {
        if self.status != PeerStatus::Ready || self.peers.is_empty() || self.key.is_none() {
            return None;
        }
        Some(Want {
            id: self.id?,
            network: self.network.clone()?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Want {
    id: PeerId,
    network: NetworkSettings,
}

enum Msg {
    /// The published state changed.
    Changed,
    /// A PC was removed — close its links now (the save follows).
    Revoke(PeerId),
    /// A PC was added — re-arm the refused dialing handles.
    Rearm,
    Accepted {
        epoch: u64,
        accepted: Accepted,
    },
    Link {
        epoch: u64,
        key: LinkKey,
        event: LinkEvent,
    },
    /// The handle's inbox ended.
    Ended {
        epoch: u64,
        key: LinkKey,
    },
    Stop(oneshot::Sender<()>),
    /// The refusal-log window started folding — wake to schedule its trailing change tick.
    Wake,
    /// Tests: the actor waits on this before anything else (a stuck runtime).
    #[cfg(test)]
    Stall(oneshot::Receiver<()>),
    /// Tests: the state of a PC's dialing handle.
    #[cfg(test)]
    DialState(PeerId, oneshot::Sender<Option<PeerState>>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LinkEvent {
    Up,
    Down,
    Failed,
    GaveUp {
        rejected: bool,
    },
    ProbeAck(u64),
    /// A frame came from a PC no longer trusted.
    NotTrusted,
}

/// What a drain task does with one PeerWire message ([`route`]).
#[derive(Debug, Clone, PartialEq, Eq)]
enum Route {
    /// To the actor.
    Forward(LinkEvent),
    /// Answered on the same connection.
    Reply(PeerMsg),
    /// The sending PC is no longer trusted — dropped, its links closed.
    NotTrusted,
    Drop,
}

/// What the actor, the trust operations, the gate, the dialer and the drain tasks share.
struct Core {
    store: Mutex<PeerStore>,
    published: RwLock<Arc<Published>>,
    book: Arc<Mutex<LinkBook>>,
    knocks: Mutex<Knocks>,
    endpoint: Mutex<EndpointView>,
    /// This daemon run — the `Hello` instance.
    instance: Instance,
    clock: Arc<dyn Clock>,
    settings: LinkSettings,
    tx: mpsc::UnboundedSender<Msg>,
    changes: watch::Sender<u64>,
    /// The actor task — aborted when a stop runs past [`STOP_DEADLINE`].
    actor: Mutex<Option<JoinHandle<()>>>,
    /// Tests: endpoints brought up so far.
    #[cfg(test)]
    bring_ups: std::sync::atomic::AtomicU64,
}

impl Core {
    fn published(&self) -> Arc<Published> {
        read(&self.published).clone()
    }

    fn is_trusted(&self, id: &PeerId) -> bool {
        read(&self.published).trusted(id).is_some()
    }

    /// Re-publishes the store's state (the store lock is held by the caller) and tells the actor.
    fn publish(&self, store: &PeerStore) {
        *write(&self.published) = Arc::new(Published::of(store));
        let _ = self.tx.send(Msg::Changed);
        self.bump();
    }

    /// Takes `id` out of the published trust at once and has its links closed.
    // ADR-0390 d4: revoked in memory before the list is saved.
    fn revoke(&self, id: &PeerId) {
        {
            let mut published = write(&self.published);
            let mut next = Published::clone(&published);
            next.peers.retain(|p| p.id != *id);
            *published = Arc::new(next);
        }
        let _ = self.tx.send(Msg::Revoke(*id));
        self.bump();
    }

    fn set_endpoint(&self, view: EndpointView) {
        *lock(&self.endpoint) = view;
        self.bump();
    }

    fn bump(&self) {
        self.changes.send_modify(|n| *n = n.wrapping_add(1));
    }

    /// The accept gate (`accept_if`): a listed PC with the `mail` permission passes; anyone else is refused before any stream, recorded
    ///   and logged (throttled).
    // ADR-0390 d3
    fn gate(&self, key: &[u8; 32]) -> bool {
        let id = PeerId::from_bytes(*key);
        if self.is_trusted(&id) {
            return true;
        }
        // One budget for every refusal (gate.rs head): the log line and the change tick both follow it.
        let line = lock(&self.knocks).knock(id, self.clock.now(), SystemTime::now());
        match line {
            LogLine::Now { folded, pcs } => {
                tracing::warn!(pc = %id, folded, folded_pcs = pcs, "a PC that is not on the trusted list tried to link — refused (not paired)");
                self.bump();
            }
            // Once per window: the actor schedules the trailing tick that shows the folded knocks.
            LogLine::Fold { first: true } => {
                let _ = self.tx.send(Msg::Wake);
            }
            LogLine::Fold { first: false } => {}
        }
        false
    }

    /// Reloads what may clear by itself (`PeerStore::retry`) — blocking.
    fn retry_store(&self) {
        let mut store = lock(&self.store);
        if store.retry() {
            self.publish(&store);
        }
    }
}

/// The peer link (module head). Cheap to clone. ★Call [`PeerLink::stop`]★ — dropping the last handle does not stop the actor (it holds
/// its own sender), so the endpoint would keep running.
#[derive(Clone)]
pub(crate) struct PeerLink {
    core: Arc<Core>,
}

impl PeerLink {
    /// Daemon start: loads this account's peer state from the daemon state folder, clears temp files a crash left, and starts the
    ///   runtime, which brings the endpoint up if the state allows. Loading runs on the blocking pool.
    pub(crate) async fn start(state_dir: PathBuf, settings: LinkSettings) -> Result<Self, String> {
        Self::start_loading(
            move || PeerStore::open(state_dir),
            settings,
            Arc::new(SystemClock),
        )
        .await
    }

    /// [`PeerLink::start`] with the store's loader and the clock given (tests).
    pub(crate) async fn start_loading(
        load: impl FnOnce() -> PeerStore + Send + 'static,
        settings: LinkSettings,
        clock: Arc<dyn Clock>,
    ) -> Result<Self, String> {
        let store = tokio::task::spawn_blocking(move || {
            let store = load();
            sweep(&store);
            store
        })
        .await
        .map_err(|e| format!("the peer state could not be loaded: {e}"))?;
        let instance =
            Instance::generate().map_err(|e| format!("no run instance could be drawn: {e}"))?;
        let (tx, rx) = mpsc::unbounded_channel();
        let (changes, _) = watch::channel(0);
        let own = store.id().unwrap_or(PeerId::from_bytes([0; 32]));
        let core = Arc::new(Core {
            published: RwLock::new(Arc::new(Published::of(&store))),
            store: Mutex::new(store),
            book: Arc::new(Mutex::new(LinkBook::new(own, nonce_seed()))),
            knocks: Mutex::new(Knocks::default()),
            endpoint: Mutex::new(EndpointView::Off),
            instance,
            clock,
            settings,
            tx,
            changes,
            actor: Mutex::new(None),
            #[cfg(test)]
            bring_ups: std::sync::atomic::AtomicU64::new(0),
        });
        let actor = Actor {
            core: core.clone(),
            rx,
            running: None,
            epoch: 0,
            failed: None,
            quick_binds: 0,
            retry: None,
        };
        *lock(&core.actor) = Some(tokio::spawn(actor.run()));
        Ok(Self { core })
    }

    /// Closes every link (letting each send what it holds, within a short deadline) and the endpoint, and ends the runtime — within
    ///   [`STOP_DEADLINE`]: past it the runtime is cut off (the endpoint then closes without telling the other PCs; they notice at their
    ///   idle-out). Later trust operations still change the files but bring nothing up.
    pub(crate) async fn stop(&self) {
        let (done, wait) = oneshot::channel();
        if self.core.tx.send(Msg::Stop(done)).is_err() {
            return;
        }
        if with_deadline(&*self.core.clock, STOP_DEADLINE, wait)
            .await
            .is_err()
        {
            tracing::warn!(
                deadline_ms = STOP_DEADLINE.as_millis() as u64,
                "the peer link did not close within its deadline — cut"
            );
            if let Some(actor) = lock(&self.core.actor).take() {
                // Dropping its state cuts every link and closes the endpoint (`Running`'s `Drop`).
                actor.abort();
                let _ = actor.await;
            }
            self.core.set_endpoint(EndpointView::Off);
        }
    }

    /// What `peer.list` shows.
    pub(crate) fn snapshot(&self) -> PeerSnapshot {
        let published = self.core.published();
        let endpoint = lock(&self.core.endpoint).clone();
        let rows = {
            let book = lock(&self.core.book);
            published
                .peers
                .iter()
                .map(|peer| PeerRow {
                    peer: peer.clone(),
                    online: match &endpoint {
                        EndpointView::Error(_) => Online::EndpointError,
                        EndpointView::Off => Online::Offline,
                        EndpointView::Up { .. } => book.online(&peer.id),
                    },
                    warning: book.warning(&peer.id),
                })
                .collect()
        };
        PeerSnapshot {
            status: published.status.clone(),
            id: published.id,
            lock_note: published.lock_note.clone(),
            network: published.network.clone(),
            endpoint,
            peers: rows,
            knocks: lock(&self.core.knocks).list(),
        }
    }

    /// Ticks on every change of what [`PeerLink::snapshot`] shows — refused knocks fold into one tick per refusal-log window, so a new knock
    /// shows at most one window late.
    pub(crate) fn changes(&self) -> watch::Receiver<u64> {
        self.core.changes.subscribe()
    }

    // ── trust operations (blocking — call on the blocking pool) ──

    /// This PC's ID, making the key on first need.
    pub(crate) fn ensure_identity(&self) -> Result<PeerId, PeerError> {
        self.change(PeerStore::ensure_identity)
    }

    /// Adds a PC; an applied add also re-arms every dialing handle the other side refused.
    pub(crate) fn add(&self, peer: TrustedPeer) -> Result<(), PeerError> {
        let id = peer.id;
        let added = self.change(|store| store.add(peer));
        if added.is_ok() {
            lock(&self.core.knocks).forget(&id);
            let _ = self.core.tx.send(Msg::Rearm);
        }
        added
    }

    /// Changes a PC's name and / or direct address; its links stay (a new address is used from the next dial).
    pub(crate) fn edit(&self, id: &PeerId, edit: PeerEdit) -> Result<(), PeerError> {
        self.change(|store| store.edit(id, edit))
    }

    /// Removes a PC: revoked and its links closed first, then the list is saved — a refused save is in [`Removal::saved`] and the
    ///   revocation stays for this run.
    pub(crate) fn remove(&self, id: &PeerId) -> Result<Removal, PeerError> {
        let mut store = lock(&self.core.store);
        self.core.revoke(id);
        let removed = store.remove(id);
        self.core.publish(&store);
        removed
    }

    /// Replaces the network settings; a running endpoint restarts with them.
    pub(crate) fn set_network(&self, network: NetworkSettings) -> Result<(), PeerError> {
        self.change(|store| store.set_network(network))
    }

    pub(crate) fn regenerate(&self) -> Result<Regenerated, PeerError> {
        self.change(PeerStore::regenerate)
    }

    pub(crate) fn accept_list(&self) -> Result<(), PeerError> {
        self.change(PeerStore::accept_list)
    }

    pub(crate) fn adopt(&self, machine: &str) -> Result<(), PeerError> {
        self.change(|store| store.adopt(machine))
    }

    pub(crate) fn start_fresh(&self) -> Result<PeerId, PeerError> {
        self.change(PeerStore::start_fresh)
    }

    fn change<T>(&self, op: impl FnOnce(&mut PeerStore) -> T) -> T {
        let mut store = lock(&self.core.store);
        let out = op(&mut store);
        self.core.publish(&store);
        out
    }
}

/// Clears the key and list temp files a crashed write left and logs what happened.
// ADR-0291 R12
fn sweep(store: &PeerStore) {
    match store.sweep_temps() {
        Ok(swept) => {
            for (path, removed) in swept {
                match removed {
                    Ok(()) => {
                        tracing::debug!(path = %path.display(), "removed a temp file a crashed peer-state write left")
                    }
                    Err(e) => {
                        tracing::warn!(path = %path.display(), "a temp file a crashed peer-state write left could not be removed: {e}")
                    }
                }
            }
        }
        Err(e) => tracing::warn!(
            "the daemon state folder could not be listed for peer-state temp files: {e}"
        ),
    }
}

/// The registry's policy — no supervisor ping or silence cut (QUIC's own liveness ends a dead link, ADR-0388 decision 6), redial without
///   end on a capped backoff, `Mail`'s own deadline.
fn policy() -> Policy {
    Policy {
        request_timeout: MAIL_DEADLINE,
        connect_timeout: CONNECT_TIMEOUT,
        supervisor_keepalive: false,
        reconnect: ReconnectPolicy {
            max_attempts: u32::MAX,
            base: BACKOFF_BASE,
            cap: BACKOFF_CAP,
            ..ReconnectPolicy::default()
        },
        ..Policy::default()
    }
}

fn adapter_helper(helper: &Helper) -> adapter::Helper {
    match helper {
        Helper::Default => adapter::Helper::Default,
        Helper::None => adapter::Helper::Disabled,
        Helper::Url(url) => adapter::Helper::Url(url.clone()),
    }
}

fn close_code(how: CloseAs) -> CloseCode {
    match how {
        CloseAs::GoingAway => CloseCode::GOING_AWAY,
        CloseAs::Refused => CloseCode::HANDSHAKE_REJECTED,
    }
}

/// Dials a listed PC by its logical address: its key and the direct address listed now, so an edited address is used from the next dial.
struct PeerDialer {
    iroh: IrohDialer,
    core: Arc<Core>,
}

impl Dialer for PeerDialer {
    fn dial(
        &self,
        addr: &Address,
    ) -> BoxFuture<'_, Result<(Box<dyn LinkTx>, Box<dyn LinkRx>), DialFailure>> {
        let id = addr
            .as_str()
            .strip_prefix(DIAL_PREFIX)
            .and_then(|text| PeerId::parse(text).ok());
        Box::pin(async move {
            let Some(id) = id else {
                return Err(DialFailure::Unreachable(LinkError::new(
                    "not a peer address",
                )));
            };
            let direct = match self.core.published().trusted(&id) {
                Some(peer) => peer.address,
                None => {
                    return Err(DialFailure::Refused {
                        code: CloseCode::HANDSHAKE_REJECTED,
                        reason: "that PC is no longer trusted".to_string(),
                    })
                }
            };
            let target = adapter::dial_address(id.as_bytes(), direct.as_slice());
            self.iroh.dial(&target).await
        })
    }
}

/// One live transport handle.
struct Handle {
    node: PeerId,
    dir: Dir,
    peer: Peer<PeerWire>,
    notes: Arc<HandshakeNotes>,
    /// The task reading this handle's inbox.
    drain: JoinHandle<()>,
}

/// A running endpoint and everything on it.
struct Running {
    epoch: u64,
    want: Want,
    endpoint: IrohEndpoint,
    registry: Registry<PeerWire>,
    accept_task: JoinHandle<()>,
    handles: HashMap<LinkKey, Handle>,
    /// The direct address each PC was last dialed with in mind — an edit nudges a waiting handle.
    addresses: HashMap<PeerId, Option<SocketAddr>>,
    /// [`Actor::teardown`] closed the endpoint — the drop then holds no endpoint clone past it (a same-port restart binds at once).
    closed: bool,
}

/// ★Nothing of an endpoint outlives its `Running`★ — also when the actor is aborted mid-way (a stop past [`STOP_DEADLINE`]) and never
///   runs [`Actor::teardown`]: the accept task and the drain tasks are aborted, every handle is cut at once (a drain task's `Peer` clone
///   would otherwise keep its supervisor alive), and the endpoint is closed. After a normal teardown all of this is already done.
impl Drop for Running {
    fn drop(&mut self) {
        self.accept_task.abort();
        for handle in self.handles.values() {
            handle.peer.abort(CloseCode::GOING_AWAY, "going away");
            handle.drain.abort();
        }
        if self.closed {
            return;
        }
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            let endpoint = self.endpoint.clone();
            runtime.spawn(async move { endpoint.close().await });
        }
    }
}

/// The store retry's schedule.
struct Retry {
    at: Instant,
    backoff: Duration,
}

struct Actor {
    core: Arc<Core>,
    rx: mpsc::UnboundedReceiver<Msg>,
    running: Option<Running>,
    epoch: u64,
    /// The settings the endpoint last failed with, and how — not retried while they stay (a bind failure: until its retry time).
    failed: Option<(Want, EndpointError, Instant)>,
    /// Quick bind retries left after an endpoint restart.
    quick_binds: u32,
    retry: Option<Retry>,
}

impl Actor {
    async fn run(mut self) {
        self.reconcile().await;
        loop {
            let deadline = self.next_deadline();
            let msg = match deadline {
                Some(at) => {
                    let wait = self
                        .core
                        .clock
                        .sleep(at.saturating_duration_since(self.core.clock.now()));
                    tokio::select! {
                        msg = self.rx.recv() => msg,
                        () = wait => {
                            self.on_timer().await;
                            continue;
                        }
                    }
                }
                None => self.rx.recv().await,
            };
            let Some(msg) = msg else {
                self.teardown().await;
                return;
            };
            match msg {
                Msg::Changed => self.reconcile().await,
                Msg::Revoke(node) => self.revoke(node),
                Msg::Rearm => {
                    let actions = lock(&self.core.book).rearm_all();
                    self.execute(actions);
                    self.core.bump();
                }
                Msg::Accepted { epoch, accepted } => self.on_accepted(epoch, accepted),
                Msg::Link { epoch, key, event } => self.on_link(epoch, key, event),
                Msg::Ended { epoch, key } => self.on_ended(epoch, key),
                Msg::Stop(done) => {
                    self.teardown().await;
                    let _ = done.send(());
                    return;
                }
                Msg::Wake => {}
                #[cfg(test)]
                Msg::Stall(release) => {
                    let _ = release.await;
                }
                #[cfg(test)]
                Msg::DialState(node, answer) => {
                    let dial = lock(&self.core.book).dial_of(&node);
                    let state = dial
                        .and_then(|key| self.running.as_ref()?.handles.get(&key))
                        .map(|handle| handle.peer.peer_state());
                    let _ = answer.send(state);
                }
            }
        }
    }

    fn next_deadline(&self) -> Option<Instant> {
        let book = self
            .running
            .as_ref()
            .and_then(|_| lock(&self.core.book).next_deadline());
        let bind = match &self.failed {
            Some((_, EndpointError::Bind(_), at)) => Some(*at),
            _ => None,
        };
        let retry = self.retry.as_ref().map(|r| r.at);
        let knocks = lock(&self.core.knocks).trailing_due();
        [book, bind, retry, knocks].into_iter().flatten().min()
    }

    async fn on_timer(&mut self) {
        let now = self.core.clock.now();
        if lock(&self.core.knocks).take_trailing(now) {
            self.core.bump();
        }
        if self.retry.as_ref().is_some_and(|r| r.at <= now) {
            if let Some(retry) = self.retry.as_mut() {
                retry.backoff = (retry.backoff * 2).min(IDENTITY_RETRY_MAX);
                retry.at = now + retry.backoff;
            }
            // Disk and DPAPI — off the async workers; a change comes back as `Changed`.
            let core = self.core.clone();
            tokio::task::spawn_blocking(move || core.retry_store());
        }
        if matches!(&self.failed, Some((_, EndpointError::Bind(_), at)) if *at <= now) {
            self.failed = None;
            self.reconcile().await;
        }
        if self.running.is_some() {
            let actions = lock(&self.core.book).tick(now);
            self.execute(actions);
            self.core.bump();
        }
    }

    /// Brings the endpoint to what the published state wants.
    async fn reconcile(&mut self) {
        let published = self.core.published();
        let retryable = matches!(
            published.status,
            PeerStatus::KeyTransient(_)
                | PeerStatus::IdentityInUse
                | PeerStatus::ListRefused(ListRefusal::ReadFailed(_))
        );
        match (retryable, &self.retry) {
            (true, None) => {
                self.retry = Some(Retry {
                    at: self.core.clock.now() + IDENTITY_RETRY_FIRST,
                    backoff: IDENTITY_RETRY_FIRST,
                })
            }
            (false, Some(_)) => self.retry = None,
            _ => {}
        }
        let Some(want) = published.want() else {
            self.failed = None;
            self.quick_binds = 0;
            if self.running.is_some() {
                self.teardown().await;
            }
            self.core.set_endpoint(EndpointView::Off);
            return;
        };
        if self.running.as_ref().is_some_and(|r| r.want == want) {
            self.sync_nodes(&published);
            return;
        }
        if self.running.is_some() {
            // The identity or the network settings changed.
            self.teardown().await;
            self.quick_binds = QUICK_BIND_TRIES;
        }
        if self
            .failed
            .as_ref()
            .is_some_and(|(failed, _, _)| *failed == want)
        {
            return;
        }
        self.failed = None;
        self.bring_up(want, &published).await;
    }

    async fn bring_up(&mut self, want: Want, published: &Published) {
        let Some(secret) = published.key.clone() else {
            return;
        };
        // ADR-0389: a stored helper URL the adapter refuses is an endpoint error — never replaced by n0's defaults.
        if let Err(why) = want.network.check_for_endpoint() {
            tracing::error!("the peer link endpoint cannot start — a stored helper server URL is refused: {why}");
            let error = EndpointError::HelperRefused(why);
            self.failed = Some((want, error.clone(), self.core.clock.now()));
            self.core.set_endpoint(EndpointView::Error(error));
            return;
        }
        let settings = &self.core.settings;
        let config = IrohConfig {
            secret_key: *secret.expose(),
            alpn: ALPN.to_vec(),
            listen_port: want.network.listen_port(settings.dev),
            loopback_only: settings.loopback_only,
            relay: adapter_helper(&want.network.relay),
            address_lookup: adapter_helper(&want.network.address_lookup),
            mdns: want.network.mdns,
            max_payload: MAX_FRAME_BYTES,
        };
        let gate = self.core.clone();
        let accept = IrohAcceptPolicy {
            accept_if: Arc::new(move |key: &[u8; 32]| gate.gate(key)),
            handshake_timeout: HANDSHAKE_TIMEOUT,
            stream_accept_timeout: STREAM_ACCEPT_TIMEOUT,
            max_pending: ACCEPT_PERMITS,
        };
        let (endpoint, listener) =
            match IrohEndpoint::bind(config, accept, self.core.clock.clone()).await {
                Ok(bound) => bound,
                Err(e) => {
                    let error = EndpointError::Bind(e.to_string());
                    if self.quick_binds > 0 {
                        // Just restarted — the old endpoint may hold the port a moment longer; the view stays as it is.
                        self.quick_binds -= 1;
                        tracing::debug!(
                            "the peer link endpoint could not bind yet, retrying shortly: {e}"
                        );
                        self.failed = Some((want, error, self.core.clock.now() + QUICK_BIND_RETRY));
                        return;
                    }
                    tracing::warn!(
                        retry_in_secs = BIND_RETRY.as_secs(),
                        "the peer link endpoint could not bind: {e}"
                    );
                    self.failed = Some((want, error.clone(), self.core.clock.now() + BIND_RETRY));
                    self.core.set_endpoint(EndpointView::Error(error));
                    return;
                }
            };
        self.quick_binds = 0;
        self.epoch += 1;
        // ADR-0389 d8: the duplicate evidence lives for the daemon run — only links and dialing start over.
        let restarted = lock(&self.core.book).restart(want.id);
        let registry = Registry::new(
            Arc::new(PeerWire),
            Arc::new(PeerDialer {
                iroh: endpoint.dialer(),
                core: self.core.clone(),
            }),
            self.core.clock.clone(),
            policy(),
        );
        let accept_task = tokio::spawn(accept_loop(listener, self.core.tx.clone(), self.epoch));
        let addrs = endpoint.bound_addrs();
        tracing::info!(addrs = ?addrs, pc = %want.id, "peer link endpoint up");
        self.running = Some(Running {
            epoch: self.epoch,
            want,
            endpoint,
            registry,
            accept_task,
            handles: HashMap::new(),
            addresses: HashMap::new(),
            closed: false,
        });
        self.execute(restarted);
        self.sync_nodes(published);
        // Up only once the listed PCs are tracked — a link that comes up is then never taken for an unlisted PC's.
        self.core.set_endpoint(EndpointView::Up { addrs });
        #[cfg(test)]
        self.core
            .bring_ups
            .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
    }

    /// Closes every link and the endpoint.
    async fn teardown(&mut self) {
        let Some(mut running) = self.running.take() else {
            return;
        };
        running.accept_task.abort();
        let _ = (&mut running.accept_task).await;
        // The drain tasks keep reading these inboxes to the end; this task drains none of them.
        if running.registry.close_all(FLUSH).await.is_err() {
            tracing::debug!("not every peer link finished closing within the flush deadline");
        }
        running.endpoint.close().await;
        running.closed = true;
        // Its drop cuts whatever did not finish within the flush (`Running`'s `Drop`).
        drop(running);
        tracing::info!("peer link endpoint down");
        self.core.set_endpoint(EndpointView::Off);
    }

    /// Lines the book and the handles up with the listed PCs.
    fn sync_nodes(&mut self, published: &Published) {
        let Some(running) = self.running.as_mut() else {
            return;
        };
        let mut actions = Vec::new();
        let gone: Vec<PeerId> = {
            let mut book = lock(&self.core.book);
            for peer in &published.peers {
                actions.extend(book.ensure(peer.id));
            }
            let gone: Vec<PeerId> = book
                .nodes()
                .filter(|id| !published.peers.iter().any(|p| p.id == **id))
                .copied()
                .collect();
            for id in &gone {
                book.forget(id);
            }
            gone
        };
        for id in gone {
            close_node(running, &id);
            running.addresses.remove(&id);
        }
        for peer in &published.peers {
            let before = running.addresses.insert(peer.id, peer.address);
            if before.is_some_and(|before| before != peer.address) {
                // A handle waiting out its backoff tries the new address now; one mid-attempt or live keeps going.
                for handle in running.handles.values() {
                    if handle.node == peer.id
                        && handle.dir == Dir::Dialed
                        && handle.peer.peer_state() == PeerState::Backoff
                    {
                        handle.peer.reconnect_now();
                    }
                }
            }
        }
        self.execute(actions);
    }

    fn revoke(&mut self, node: PeerId) {
        lock(&self.core.book).forget(&node);
        if let Some(running) = self.running.as_mut() {
            close_node(running, &node);
            running.addresses.remove(&node);
        }
    }

    fn on_accepted(&mut self, epoch: u64, accepted: Accepted) {
        let Some(running) = self.running.as_mut().filter(|r| r.epoch == epoch) else {
            return;
        };
        let node = accepted
            .info()
            .peer_key
            .as_deref()
            .and_then(|key| <[u8; 32]>::try_from(key).ok())
            .map(PeerId::from_bytes);
        // The gate let it in; it may have been removed since — dropping it closes the connection.
        let Some(node) = node.filter(|node| self.core.is_trusted(node)) else {
            tracing::debug!("an accepted link from a PC no longer trusted — dropped");
            return;
        };
        let key = lock(&self.core.book).accepted_key();
        let notes = Arc::new(HandshakeNotes::default());
        let handshake = PeerHandshake::new(
            Role::Accept,
            node,
            self.core.instance,
            self.core.book.clone(),
            self.core.clock.clone(),
            notes.clone(),
        );
        let (peer, inbox) = running.registry.attach(
            Label::new(format!("acc-{key}")),
            accepted,
            Arc::new(handshake),
        );
        tracing::debug!(pc = %node, key, "peer link accepted");
        let drain = tokio::spawn(drain(
            self.core.clone(),
            epoch,
            key,
            node,
            Dir::Accepted,
            peer.clone(),
            inbox,
        ));
        running.handles.insert(
            key,
            Handle {
                node,
                dir: Dir::Accepted,
                peer,
                notes,
                drain,
            },
        );
    }

    fn start_dial(&mut self, node: PeerId, key: LinkKey) {
        let Some(running) = self.running.as_mut() else {
            return;
        };
        let notes = Arc::new(HandshakeNotes::default());
        let handshake = PeerHandshake::new(
            Role::Dial,
            node,
            self.core.instance,
            self.core.book.clone(),
            self.core.clock.clone(),
            notes.clone(),
        );
        // One label per PC: a new dialing handle replaces whatever the old one left.
        let (peer, inbox) = running.registry.add(
            Label::new(format!("dial-{node}")),
            Address::new(format!("{DIAL_PREFIX}{node}")),
            Arc::new(handshake),
        );
        let drain = tokio::spawn(drain(
            self.core.clone(),
            running.epoch,
            key,
            node,
            Dir::Dialed,
            peer.clone(),
            inbox,
        ));
        running.handles.insert(
            key,
            Handle {
                node,
                dir: Dir::Dialed,
                peer,
                notes,
                drain,
            },
        );
    }

    fn on_link(&mut self, epoch: u64, key: LinkKey, event: LinkEvent) {
        let Some(running) = self.running.as_mut().filter(|r| r.epoch == epoch) else {
            return;
        };
        let Some(handle) = running.handles.get(&key) else {
            return;
        };
        let (node, dir) = (handle.node, handle.dir);
        let now = self.core.clock.now();
        let actions = match event {
            LinkEvent::Up => match handle.notes.take_run() {
                Some(run) => {
                    tracing::info!(pc = %node, ?dir, %run, "peer link up");
                    let mut book = lock(&self.core.book);
                    // ADR-0388 d7: a dialing handle between attempts is closed at once; only one mid-attempt is left to the tie-break.
                    let dial_idle = book
                        .dial_of(&node)
                        .filter(|dial| *dial != key)
                        .and_then(|dial| running.handles.get(&dial))
                        .is_some_and(|dial| {
                            matches!(
                                dial.peer.peer_state(),
                                PeerState::Idle | PeerState::Backoff | PeerState::GaveUp(_)
                            )
                        });
                    book.link_up(node, key, dir, run, dial_idle, now)
                }
                None => {
                    tracing::warn!(pc = %node, "a peer link came up without a recorded Hello — closed");
                    vec![Action::Close {
                        key,
                        how: CloseAs::GoingAway,
                        reason: "no hello",
                    }]
                }
            },
            LinkEvent::Down => {
                let actions = lock(&self.core.book).link_down(key, now);
                tracing::info!(pc = %node, ?dir, "peer link down");
                actions
            }
            LinkEvent::Failed => lock(&self.core.book).dial_failed(key),
            LinkEvent::GaveUp { rejected: true } => {
                let why = handle.notes.refusal();
                tracing::warn!(pc = %node, ?why, "the other PC refused this one — dialing waits for a re-arm");
                lock(&self.core.book).dial_gave_up(key, why, now)
            }
            LinkEvent::GaveUp { rejected: false } => {
                tracing::warn!(pc = %node, "dialing gave up");
                Vec::new()
            }
            LinkEvent::ProbeAck(nonce) => lock(&self.core.book).probe_ack(key, nonce, now),
            LinkEvent::NotTrusted => {
                close_node(running, &node);
                Vec::new()
            }
        };
        self.execute(actions);
        self.core.bump();
    }

    fn on_ended(&mut self, epoch: u64, key: LinkKey) {
        let Some(running) = self.running.as_mut().filter(|r| r.epoch == epoch) else {
            return;
        };
        running.handles.remove(&key);
        let actions = lock(&self.core.book).handle_ended(key, self.core.clock.now());
        self.execute(actions);
        self.core.bump();
    }

    fn execute(&mut self, actions: Vec<Action>) {
        for action in actions {
            let Some(running) = self.running.as_mut() else {
                return;
            };
            match action {
                Action::Close { key, how, reason } => {
                    if let Some(handle) = running.handles.get(&key) {
                        // The close proceeds without awaiting the returned future.
                        drop(handle.peer.close_after_flush(close_code(how), reason));
                    }
                }
                Action::Send { key, msg } => {
                    if let Some(handle) = running.handles.get(&key) {
                        let kind = msg.kind();
                        if let Err(e) = handle.peer.notify(msg) {
                            tracing::debug!(pc = %handle.node, kind, "a peer link message was not sent: {e}");
                        }
                    }
                }
                Action::AddDial { node, key } => {
                    if self.core.is_trusted(&node) {
                        self.start_dial(node, key);
                    }
                }
                Action::Redial { key } => {
                    if let Some(handle) = running.handles.get(&key) {
                        handle.notes.rearm();
                        handle.peer.reconnect_now();
                    }
                }
            }
        }
    }
}

/// Cuts every handle of `node` as refused, at once — trust was withdrawn, nothing more is owed to that PC: no flush, and no wait for
///   the PC to acknowledge what was written (transport `Peer::abort`), so a PC that stops reading or answering cannot hold the link.
// ADR-0390 d4
fn close_node(running: &Running, node: &PeerId) {
    for handle in running.handles.values().filter(|h| h.node == *node) {
        handle
            .peer
            .abort(CloseCode::HANDSHAKE_REJECTED, "not paired");
    }
}

fn nonce_seed() -> u64 {
    let mut bytes = [0u8; 8];
    if let Err(e) = getrandom::getrandom(&mut bytes) {
        // A predictable seed only lets a PC already trusted on that very link guess a probe nonce.
        tracing::debug!("no random probe nonce seed, starting from zero: {e}");
    }
    u64::from_le_bytes(bytes)
}

/// Hands every accepted link to the actor until the endpoint closes.
async fn accept_loop(mut listener: IrohListener, tx: mpsc::UnboundedSender<Msg>, epoch: u64) {
    loop {
        match listener.accept().await {
            Ok(accepted) => {
                if tx.send(Msg::Accepted { epoch, accepted }).is_err() {
                    return;
                }
            }
            Err(e) => {
                tracing::debug!("the peer link listener ended: {e}");
                return;
            }
        }
    }
}

/// What to do with one message from `node` on the handle `key`. ★Every frame is checked against the list as it is now★ (ADR-0390 d4).
///   Only the first `ProbeAck` with the nonce of the probe open on that link reaches the actor, and a `Duplicate` warning is logged only when it changes the PC's
///   warning — a broken peer repeating either cannot fill the actor's queue or the log.
fn route(core: &Core, node: &PeerId, key: LinkKey, msg: PeerMsg) -> Route {
    if !core.is_trusted(node) {
        return Route::NotTrusted;
    }
    match msg {
        PeerMsg::Probe { nonce } => Route::Reply(PeerMsg::ProbeAck { nonce }),
        PeerMsg::ProbeAck { nonce } => {
            if lock(&core.book).claim_probe_answer(key, nonce) {
                Route::Forward(LinkEvent::ProbeAck(nonce))
            } else {
                tracing::debug!(pc = %node, "a ProbeAck that answers no open probe on this link (or not first) — dropped");
                Route::Drop
            }
        }
        PeerMsg::Duplicate { kind } => {
            if lock(&core.book).duplicate_reported(node, kind) {
                tracing::warn!(pc = %node, ?kind, "that PC reports this PC's identity in use by another running copy — look for a second running copy; a new key re-pairs every PC");
                core.bump();
            }
            Route::Drop
        }
        PeerMsg::Hello { .. } => {
            tracing::debug!(pc = %node, "a Hello after the handshake — ignored");
            Route::Drop
        }
        other => {
            tracing::debug!(pc = %node, kind = other.kind(), "a PeerWire message this build does not handle yet — dropped");
            Route::Drop
        }
    }
}

/// Reads one handle's inbox to the end (module head).
async fn drain(
    core: Arc<Core>,
    epoch: u64,
    key: LinkKey,
    node: PeerId,
    dir: Dir,
    peer: Peer<PeerWire>,
    mut inbox: Inbox<PeerWire>,
) {
    let mut cut = false;
    while let Some(item) = inbox.recv().await {
        let event = match item {
            Incoming::Message(arrival) => match route(&core, &node, key, arrival.msg) {
                Route::Forward(event) => event,
                Route::Reply(msg) => {
                    if let Err(e) = peer.notify_in(arrival.generation, msg) {
                        tracing::debug!(pc = %node, "a probe answer was not sent: {e}");
                    }
                    continue;
                }
                Route::NotTrusted => {
                    if !cut {
                        cut = true;
                        tracing::info!(pc = %node, "a frame from a PC no longer trusted — dropped, its links closed");
                        let _ = core.tx.send(Msg::Link {
                            epoch,
                            key,
                            event: LinkEvent::NotTrusted,
                        });
                    }
                    continue;
                }
                Route::Drop => continue,
            },
            Incoming::Event(event) => match event {
                TransportEvent::Connected { generation, .. } => {
                    if dir == Dir::Accepted {
                        // The acceptor's half of the Hello exchange (the dialer's handshake waits for it).
                        let hello = PeerMsg::Hello {
                            protocol_version: PEER_WIRE_VERSION,
                            instance: core.instance,
                        };
                        if let Err(e) = peer.notify_in(generation, hello) {
                            tracing::debug!(pc = %node, "the Hello was not sent: {e}");
                        }
                    }
                    LinkEvent::Up
                }
                TransportEvent::Disconnected { .. } | TransportEvent::Closed { .. } => {
                    LinkEvent::Down
                }
                TransportEvent::ConnectFailed { .. } => LinkEvent::Failed,
                TransportEvent::GaveUp { reason, .. } => LinkEvent::GaveUp {
                    rejected: reason == GaveUpReason::Rejected,
                },
                other => {
                    tracing::debug!(pc = %node, kind = other.kind(), "peer link event");
                    continue;
                }
            },
        };
        let _ = core.tx.send(Msg::Link { epoch, key, event });
    }
    let _ = core.tx.send(Msg::Ended { epoch, key });
}

#[cfg(test)]
mod tests;
