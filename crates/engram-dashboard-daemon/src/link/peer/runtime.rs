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
//!   queue. Mail and directory frames are handed on from there ([`super::mail`], [`super::directory`]): `Mail` to that PC's receive
//!   queue (run on the blocking pool, answered on the frame's own connection), `ReplyOverdue` to the host (id checked, at most
//!   [`MAX_OVERDUE_RUNNING`] per PC at once), directory frames from the PC's chosen link to its copy (a gap answered with `DirResync`),
//!   a `DirResync` with this PC's snapshot off the drain task, at most once per [`RESYNC_INTERVAL`] per link (a request from a link of a
//!   retired endpoint is not answered). ★Nothing is sent to a
//!   PC no longer trusted★ — a removal is published before the actor carries it out, so every sender checks trust itself.
//! - ★Mail and directory frames go on each PC's chosen link★ ([`Route`]): the actor re-reads the chosen links after every event; a
//!   newly chosen link first gets this PC's directory snapshot. The publisher task recomputes the directory when the host rings its
//!   bell (coalesced over [`COALESCE`]) and sends a delta — or a snapshot, one frame (ADR-0411) — on every chosen link; directory frames
//!   are only ever sent under the publisher's lock, so they go in the publisher's order. The actor also records each PC's chosen link
//!   in the copies, where a directory frame's link is checked — applying a frame never holds the link book.
//! - A PC's directory copy follows the PC, not a link: it is kept while the PC is offline (shown `disconnected`), dropped when the PC
//!   leaves the trusted list.
//! - Lock order: the store lock before the verdict gate's write side, then the published state and the copies (a removal; a verdict holds
//!   the gate's read side over the trust check, the link book and the mail kernel's lock); the publisher before the routes; the link book
//!   before the copies (the rows); the refused-knock list, the endpoint view, the in-flight counts, the receive and overdue queues and
//!   the resync state are each taken alone and briefly (the handshake and the accept gate take theirs on transport tasks). The host's
//!   checks of directory rows run before the copies' lock is taken — again under it when this PC's agent ids changed in between.
//!
//! Not here: who may call which operation and asking a human (approvals), `peer.*` commands, and what mail means to the agents (the
//!   daemon's host behind [`MailInbox`]).

// ADR-0388, ADR-0389, ADR-0390, ADR-0391, ADR-0401, ADR-0411

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
    with_deadline, Accepted, Address, Clock, CloseCode, DialFailure, Dialer, GaveUpReason,
    Generation, Inbox, Incoming, LinkError, LinkRx, LinkTx, Listener, Peer, PeerId as Label,
    PeerState, Policy, ReconnectPolicy, Registry, SystemClock, TransportEvent,
};
use futures_util::future::BoxFuture;
use tokio::sync::{mpsc, oneshot, watch};
use tokio::task::JoinHandle;

use super::book::{Action, CloseAs, Dir, LinkBook, LinkKey, Online, Warning};
use super::directory::{sort, Applied, Copies, DirectoryBell, Publish, Publisher, Report};
use super::gate::{Knock, Knocks, LogLine};
use super::handshake::{HandshakeNotes, PeerHandshake, Role};
use super::id::PeerId;
use super::identity::Secret;
use super::mail::{
    mail_bytes, refuse_all, Inbound, Job, MailEnd, MailInbox, Refusal, IN_FLIGHT_CAP,
    MAIL_DEADLINE, MAX_MAIL_BYTES, MAX_MAIL_ROWS, MAX_OVERDUE_RUNNING, MAX_OVERDUE_WAITING,
};
use super::store::{PeerEdit, PeerError, PeerStatus, PeerStore, Regenerated, Removal};
use super::trusted::{Helper, ListRefusal, NetworkPatch, NetworkSettings, Permission, TrustedPeer};
use super::wire::{
    encoded_len, DirAgent, DirDelta, DirSnapshot, Instance, Mail, MailVerdict, PeerMsg, PeerWire,
    ALPN, MAX_FRAME_BYTES, PEER_WIRE_VERSION,
};

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
/// The publisher waits this long after a ring before it recomputes, so a burst of changes is one delta.
const COALESCE: Duration = Duration::from_millis(250);
/// A link's `DirResync` requests are answered at most this often (a snapshot can be large): requests arriving while an answer is due fold
///   into it; one arriving while an answer is being sent gets one more answer, an interval after it.
const RESYNC_INTERVAL: Duration = Duration::from_secs(1);
/// A PC's directory warnings (dropped rows, ids another PC also lists) are logged at most once per this window; the rest are counted
///   into the next line.
const DIR_WARN_WINDOW: Duration = Duration::from_secs(60);

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
    /// The copy of its agent directory, sorted by name — `None` = unknown this run (no copy since this daemon started).
    pub(crate) agents: Option<Vec<DirAgent>>,
}

/// The daemon's side of mail and the directory, handed to [`PeerLink::start`].
pub(crate) struct Host {
    pub(crate) mail: Arc<dyn MailInbox>,
    /// Rung by the daemon when this PC's directory may have changed.
    pub(crate) bell: DirectoryBell,
}

/// A PC's chosen link as mail and directory frames use it.
#[derive(Clone)]
struct Route {
    key: LinkKey,
    /// The endpoint the handle belongs to.
    epoch: u64,
    peer: Peer<PeerWire>,
    /// The other PC's run on that link.
    run: Instance,
}

/// How an inbound mail is answered: on the connection it came on.
struct Reply {
    peer: Peer<PeerWire>,
    generation: Generation,
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
enum Routed {
    /// To the actor.
    Forward(LinkEvent),
    /// Answered on the same connection.
    Reply(PeerMsg),
    /// To that PC's receive queue.
    Mail(Mail),
    /// To the host.
    Overdue(String),
    /// To that PC's directory copy.
    Snapshot(DirSnapshot),
    Delta(DirDelta),
    /// Answered with this PC's directory on the same connection.
    Resync,
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
    /// [`PeerLink::stop`] began — trust operations are refused from then on, so nothing (a late dialog answer, a command still on the
    ///   blocking pool) changes the files of a daemon that is shutting down.
    stopped: std::sync::atomic::AtomicBool,
    /// The daemon's side of mail and the directory — `None` = links only (mail is refused, the directory is empty).
    host: Option<Arc<dyn MailInbox>>,
    /// This PC's directory as published, and the order of directory frames (module head).
    publisher: Mutex<Publisher>,
    /// The trusted PCs' directories.
    copies: Mutex<Copies>,
    /// Each linked PC's chosen link (module head).
    routes: Mutex<HashMap<PeerId, Route>>,
    /// `Mail` requests waiting for a verdict, per PC.
    in_flight: Mutex<HashMap<PeerId, usize>>,
    /// [`IN_FLIGHT_CAP`] (tests lower it).
    in_flight_cap: std::sync::atomic::AtomicUsize,
    /// Inbound mail per PC (module head).
    inbound: Mutex<HashMap<PeerId, Inbound<Reply>>>,
    /// Draws each receive queue's and overdue queue's generation.
    inbound_gen: std::sync::atomic::AtomicU64,
    /// Overdue reports waiting for or being handed to the host, per PC ([`on_overdue`]).
    overdue: Mutex<HashMap<PeerId, OverdueQueue>>,
    /// ★The verdict gate★ — a verdict holds the read side from its trust check until the mail kernel has applied it; a revocation takes
    ///   the write side, so once it returns no verdict of the removed PC is applied any more (ADR-0390 d4).
    verdict_gate: RwLock<()>,
    /// The directory warning windows, per PC ([`DIR_WARN_WINDOW`]).
    dir_warns: Mutex<HashMap<PeerId, WarnWindow>>,
    /// Per link of the running endpoint: its `DirResync` answers.
    resyncs: Mutex<Resyncs>,
    /// Where the mail tasks and blocking calls run — `send_mail` is called from blocking threads.
    runtime: tokio::runtime::Handle,
    /// The publisher task — aborted by [`PeerLink::stop`].
    publisher_task: Mutex<Option<JoinHandle<()>>>,
    /// Tests: endpoints brought up so far.
    #[cfg(test)]
    bring_ups: std::sync::atomic::AtomicU64,
    /// Tests: a mail request that got its answer says so on the first and waits on the second before the answer is judged.
    #[cfg(test)]
    verdict_hold: Mutex<Option<(oneshot::Sender<()>, oneshot::Receiver<()>)>>,
    /// Tests: the same, after the trust check and inside the verdict gate.
    #[cfg(test)]
    commit_hold: Mutex<Option<(oneshot::Sender<()>, oneshot::Receiver<()>)>>,
    /// Tests: directory frames this PC sent, and the publisher's recomputes.
    #[cfg(test)]
    dir_sent: std::sync::atomic::AtomicU64,
    #[cfg(test)]
    publishes: std::sync::atomic::AtomicU64,
    /// Tests: the copy re-checks completed.
    #[cfg(test)]
    rechecks: std::sync::atomic::AtomicU64,
    /// Tests: a directory frame listing this id, its rows checked, tells `reached` and waits for `release` before the copies' lock.
    #[cfg(test)]
    dir_hold: Mutex<
        Option<(
            String,
            std::sync::mpsc::Sender<()>,
            std::sync::mpsc::Receiver<()>,
        )>,
    >,
    /// Tests: the next resync answer, once sent, tells `reached` and waits for `release` before it ends.
    #[cfg(test)]
    resync_hold: Mutex<Option<(oneshot::Sender<()>, oneshot::Receiver<()>)>>,
}

/// The running endpoint's `DirResync` state ([`on_resync`]).
#[derive(Default)]
struct Resyncs {
    /// The endpoint whose links may ask — 0 = none; a teardown retires its epoch with its links' state.
    epoch: u64,
    links: HashMap<LinkKey, Resync>,
}

/// One link's `DirResync` answers ([`RESYNC_INTERVAL`]).
#[derive(Default)]
struct Resync {
    /// A worker is scheduled or sending for this link — at most one at a time.
    running: bool,
    /// Where the next answer goes — the latest request's connection (a dialing handle's newer connection replaces an older one).
    target: Option<Reply>,
    last: Option<Instant>,
}

/// One PC's overdue reports: request ids waiting (each once), and how many are being handed to the host.
struct OverdueQueue {
    /// Drawn per queue — a report of a removed PC's old queue never counts against a new one.
    gen: u64,
    waiting: std::collections::VecDeque<String>,
    running: usize,
}

/// One PC's directory warning window.
struct WarnWindow {
    start: Instant,
    /// Frames with something to warn about since the window's line.
    folded: usize,
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
            // ADR-0390 d4: waits for a verdict being applied, so none of this PC's is applied after the revocation.
            let _verdicts = write(&self.verdict_gate);
            let mut published = write(&self.published);
            let mut next = Published::clone(&published);
            next.peers.retain(|p| p.id != *id);
            *published = Arc::new(next);
        }
        // ADR-0391: its directory copy goes with it; its waiting mail and overdue reports are not run.
        lock(&self.copies).drop_node(id);
        lock(&self.inbound).remove(id);
        lock(&self.overdue).remove(id);
        lock(&self.routes).remove(id);
        let _ = self.tx.send(Msg::Revoke(*id));
        self.bump();
    }

    /// Logs what applying directory frames of `node` found, at most once per [`DIR_WARN_WINDOW`]; the frames in between are counted into the
    ///   next line.
    fn warn_directory(&self, node: &PeerId, report: Report) {
        if report.dropped == 0 && report.shared == 0 {
            return;
        }
        let now = self.clock.now();
        let mut warns = lock(&self.dir_warns);
        match warns.get_mut(node) {
            Some(window) if now.saturating_duration_since(window.start) < DIR_WARN_WINDOW => {
                window.folded += 1;
            }
            other => {
                let folded = other.map_or(0, |w| w.folded);
                tracing::warn!(pc = %node, dropped = report.dropped, first = report.first, shared = report.shared, folded, "directory rows from that PC were dropped (a local agent's id, a bad name, not an agent id), or its agent ids are also listed by another PC (copied data folder? — a send to such an id is refused as ambiguous); `folded` frames had the same since the last line");
                warns.insert(
                    *node,
                    WarnWindow {
                        start: now,
                        folded: 0,
                    },
                );
            }
        }
    }

    /// Drops copy rows that name one of this PC's own agents now — run on the blocking pool after a directory read that changed the host's
    ///   local ids (rows may have come in before an agent of this PC took their id). ★Scans and drops under the copies' lock★: a frame
    ///   whose rows were checked before the change either commits before this scan or sees the change and checks again ([`on_directory`]).
    // ADR-0391
    fn recheck_copies(&self) {
        let Some(host) = &self.host else {
            return;
        };
        let dropped = lock(&self.copies).drop_refused(&|rows| host.row_problems(rows));
        // Counted once the scan is done, so a test that waits on it knows the scan ran before what it does next.
        #[cfg(test)]
        self.rechecks
            .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        for (node, dropped) in &dropped {
            tracing::warn!(pc = %node, dropped, "rows of that PC's directory copy now name an agent of this PC (or fail the row checks) — dropped");
        }
        if !dropped.is_empty() {
            self.bump();
        }
    }

    /// The host's local-id version (0 without a host) — see [`MailInbox::local_ids_version`].
    fn local_ids_version(&self) -> u64 {
        self.host
            .as_ref()
            .map_or(0, |host| host.local_ids_version())
    }

    /// The other PC is still trusted and still the run a mail went to — read when its verdict comes back.
    // ADR-0390 d4
    fn still_trusted(&self, node: &PeerId, run: Instance) -> bool {
        self.is_trusted(node) && lock(&self.book).last_run(node) == Some(run)
    }

    /// The rows of the listed PCs: link state, warning, directory copy.
    fn rows(&self, published: &Published, endpoint: &EndpointView) -> Vec<PeerRow> {
        let book = lock(&self.book);
        let copies = lock(&self.copies);
        published
            .peers
            .iter()
            .map(|peer| PeerRow {
                peer: peer.clone(),
                online: match endpoint {
                    EndpointView::Error(_) => Online::EndpointError,
                    EndpointView::Off => Online::Offline,
                    EndpointView::Up { .. } => book.online(&peer.id),
                },
                warning: book.warning(&peer.id),
                agents: copies.get(&peer.id).map(|copy| copy.agents.clone()),
            })
            .collect()
    }

    /// Sends this PC's directory snapshot on `peer`'s connection `generation` (`None` = its current one) — under the publisher's lock,
    ///   held by the caller (which took `snapshot` from it).
    fn send_snapshot(
        &self,
        snapshot: &DirSnapshot,
        node: &PeerId,
        peer: &Peer<PeerWire>,
        generation: Option<Generation>,
    ) {
        let msg = PeerMsg::DirSnapshot(snapshot.clone());
        #[cfg(test)]
        self.dir_sent
            .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        let sent = match generation {
            Some(generation) => peer.notify_in(generation, msg),
            None => peer.notify(msg),
        };
        if let Err(e) = sent {
            tracing::debug!(pc = %node, "the directory snapshot was not sent: {e}");
        }
    }

    /// The host's check of the rows of one arriving directory frame.
    fn row_problems(&self, rows: &[&DirAgent]) -> Vec<Option<&'static str>> {
        match &self.host {
            Some(host) => host.row_problems(rows),
            None => vec![Some("this PC takes no directories (no mail host)"); rows.len()],
        }
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
    ///   With a `host`, the link carries mail and the agent directory.
    pub(crate) async fn start(
        state_dir: PathBuf,
        settings: LinkSettings,
        host: Option<Host>,
    ) -> Result<Self, String> {
        Self::start_loading(
            move || PeerStore::open(state_dir),
            settings,
            Arc::new(SystemClock),
            host,
        )
        .await
    }

    /// [`PeerLink::start`] with the store's loader and the clock given (tests).
    pub(crate) async fn start_loading(
        load: impl FnOnce() -> PeerStore + Send + 'static,
        settings: LinkSettings,
        clock: Arc<dyn Clock>,
        host: Option<Host>,
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
            stopped: std::sync::atomic::AtomicBool::new(false),
            host: host.as_ref().map(|host| host.mail.clone()),
            publisher: Mutex::new(Publisher::new(instance, MAX_FRAME_BYTES)),
            copies: Mutex::new(Copies::default()),
            routes: Mutex::new(HashMap::new()),
            in_flight: Mutex::new(HashMap::new()),
            in_flight_cap: std::sync::atomic::AtomicUsize::new(IN_FLIGHT_CAP),
            inbound: Mutex::new(HashMap::new()),
            inbound_gen: std::sync::atomic::AtomicU64::new(0),
            overdue: Mutex::new(HashMap::new()),
            verdict_gate: RwLock::new(()),
            dir_warns: Mutex::new(HashMap::new()),
            resyncs: Mutex::new(Resyncs::default()),
            runtime: tokio::runtime::Handle::current(),
            publisher_task: Mutex::new(None),
            #[cfg(test)]
            bring_ups: std::sync::atomic::AtomicU64::new(0),
            #[cfg(test)]
            verdict_hold: Mutex::new(None),
            #[cfg(test)]
            commit_hold: Mutex::new(None),
            #[cfg(test)]
            dir_sent: std::sync::atomic::AtomicU64::new(0),
            #[cfg(test)]
            publishes: std::sync::atomic::AtomicU64::new(0),
            #[cfg(test)]
            rechecks: std::sync::atomic::AtomicU64::new(0),
            #[cfg(test)]
            dir_hold: Mutex::new(None),
            #[cfg(test)]
            resync_hold: Mutex::new(None),
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
        log_start(&core.published());
        *lock(&core.actor) = Some(tokio::spawn(actor.run()));
        if let Some(host) = host {
            // The first directory is computed at once.
            host.bell.ring();
            *lock(&core.publisher_task) = Some(tokio::spawn(publish_loop(core.clone(), host)));
        }
        Ok(Self { core })
    }

    /// Closes every link (letting each send what it holds, within a short deadline) and the endpoint, and ends the runtime — within
    ///   [`STOP_DEADLINE`]: past it the runtime is cut off (the endpoint then closes without telling the other PCs; they notice at their
    ///   idle-out). Trust operations are refused from the moment it is called (a store operation already running finishes).
    pub(crate) async fn stop(&self) {
        self.core
            .stopped
            .store(true, std::sync::atomic::Ordering::Release);
        // Overdue reports still waiting are not handed on; one being handed on finishes (each job checks `stopped` as it starts).
        lock(&self.core.overdue).clear();
        if let Some(publisher) = lock(&self.core.publisher_task).take() {
            publisher.abort();
        }
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
        tracing::info!("peer link stopped");
    }

    /// What `peer.list` shows.
    pub(crate) fn snapshot(&self) -> PeerSnapshot {
        let published = self.core.published();
        let endpoint = lock(&self.core.endpoint).clone();
        let rows = self.core.rows(&published, &endpoint);
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

    /// The listed PCs with their link state and directory copy — what mail addressing reads (lighter than [`PeerLink::snapshot`]).
    pub(crate) fn rows(&self) -> Vec<PeerRow> {
        let endpoint = lock(&self.core.endpoint).clone();
        self.core.rows(&self.core.published(), &endpoint)
    }

    // ── mail (non-blocking — callable from any thread) ──

    /// Sends one mail to `node` on its chosen link (module head of [`super::mail`]). ★`done` is called exactly once★, on a blocking-pool
    ///   thread, with how the mail ended.
    pub(crate) fn send_mail(
        &self,
        node: PeerId,
        mail: Mail,
        done: Box<dyn FnOnce(MailEnd) + Send>,
    ) {
        let runtime = self.core.runtime.clone();
        let finish = move |done: Box<dyn FnOnce(MailEnd) + Send>, end: MailEnd| {
            runtime.spawn_blocking(move || done(end));
        };
        let msg = PeerMsg::Mail(mail);
        // ADR-0390: the sender checks the encoded size, so its own oversized frame never closes the link.
        if !encoded_len(&msg).is_ok_and(|n| n <= MAX_FRAME_BYTES) {
            return finish(done, MailEnd::TooLarge);
        }
        let route = lock(&self.core.routes).get(&node).cloned();
        let Some(route) = route.filter(|_| self.core.is_trusted(&node)) else {
            return finish(done, MailEnd::Offline);
        };
        {
            let cap = self
                .core
                .in_flight_cap
                .load(std::sync::atomic::Ordering::Acquire);
            let mut in_flight = lock(&self.core.in_flight);
            let count = in_flight.entry(node).or_insert(0);
            if *count >= cap {
                tracing::warn!(pc = %node, cap, "too many mails waiting for a verdict from that PC — not sent");
                return finish(done, MailEnd::Limit);
            }
            *count += 1;
        }
        let core = self.core.clone();
        self.core.runtime.spawn(async move {
            let result = route.peer.request(msg).await;
            {
                let mut in_flight = lock(&core.in_flight);
                if let Some(count) = in_flight.get_mut(&node) {
                    *count = count.saturating_sub(1);
                    if *count == 0 {
                        in_flight.remove(&node);
                    }
                }
            }
            #[cfg(test)]
            let hold = lock(&core.verdict_hold).take();
            // ADR-0390 d4: trust is judged on the thread that hands the end on, right before it — a removal while the answer waited for
            //   that thread still stops it.
            let runtime = core.runtime.clone();
            runtime.spawn_blocking(move || {
                #[cfg(test)]
                if let Some((reached, release)) = hold {
                    let _ = reached.send(());
                    let _ = release.blocking_recv();
                }
                // ADR-0390 d4: from the trust check until the kernel has applied the end — a revocation waits for it (the kernel only
                //   parks notices and rings the flush doorbell here, so the gate is never held across an injection).
                let _verdict = read(&core.verdict_gate);
                let end = MailEnd::of(result, || core.still_trusted(&node, route.run));
                #[cfg(test)]
                {
                    let hold = lock(&core.commit_hold).take();
                    if let Some((reached, release)) = hold {
                        let _ = reached.send(());
                        let _ = release.blocking_recv();
                    }
                }
                done(end);
            });
        });
    }

    /// Tells `node` that its request `request_id` is overdue here — best-effort, lost when that PC has no live link.
    pub(crate) fn send_reply_overdue(&self, node: PeerId, request_id: &str) {
        let route = lock(&self.core.routes).get(&node).cloned();
        // ADR-0390 d4: nothing goes to a PC once it is removed, even before the actor has dropped its route.
        let Some(route) = route.filter(|_| self.core.is_trusted(&node)) else {
            tracing::debug!(pc = %node, "an overdue report was not sent — that PC is not linked");
            return;
        };
        let msg = PeerMsg::ReplyOverdue {
            request_id: request_id.to_string(),
        };
        if let Err(e) = route.peer.notify(msg) {
            tracing::debug!(pc = %node, "an overdue report was not sent: {e}");
        }
    }

    /// Tests: the next mail request that gets its answer tells `reached` and waits for `release` before the answer is judged.
    #[cfg(test)]
    pub(crate) fn hold_next_verdict(
        &self,
        reached: oneshot::Sender<()>,
        release: oneshot::Receiver<()>,
    ) {
        *lock(&self.core.verdict_hold) = Some((reached, release));
    }

    /// Tests: the next mail end, once judged trusted or not, tells `reached` and waits for `release` inside the verdict gate.
    #[cfg(test)]
    pub(crate) fn hold_next_commit(
        &self,
        reached: oneshot::Sender<()>,
        release: oneshot::Receiver<()>,
    ) {
        *lock(&self.core.commit_hold) = Some((reached, release));
    }

    /// Tests: lowers the in-flight cap.
    #[cfg(test)]
    pub(crate) fn set_in_flight_cap(&self, cap: usize) {
        self.core
            .in_flight_cap
            .store(cap, std::sync::atomic::Ordering::Release);
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
        self.refuse_if_stopped()?;
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

    /// Applies a network patch to the settings of this moment under the store lock; a running endpoint restarts with the result.
    ///   Returns the settings it found.
    pub(crate) fn patch_network(&self, patch: &NetworkPatch) -> Result<NetworkSettings, PeerError> {
        self.change(|store| store.patch_network(patch))
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

    fn change<T>(
        &self,
        op: impl FnOnce(&mut PeerStore) -> Result<T, PeerError>,
    ) -> Result<T, PeerError> {
        self.refuse_if_stopped()?;
        let mut store = lock(&self.core.store);
        let out = op(&mut store);
        self.core.publish(&store);
        out
    }

    fn refuse_if_stopped(&self) -> Result<(), PeerError> {
        if self.core.stopped.load(std::sync::atomic::Ordering::Acquire) {
            return Err(PeerError::Blocked(
                "the peer link has stopped (the daemon is shutting down) — nothing was changed"
                    .to_string(),
            ));
        }
        Ok(())
    }
}

/// One line at start: whether the endpoint comes up, and if not, why — an endpoint that stays off logs nothing else.
fn log_start(published: &Published) {
    let pcs = published.peers.len();
    if published.want().is_some() {
        tracing::info!(pcs, "peer link started — endpoint starting");
        return;
    }
    let why = if published.status != PeerStatus::Ready {
        published.status.to_string()
    } else if pcs == 0 {
        "no trusted PCs".to_string()
    } else {
        "no identity key".to_string()
    };
    tracing::info!(pcs, "peer link started — endpoint off: {why}");
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
            self.refresh_routes();
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

    /// Re-reads each PC's chosen link; a newly chosen one first gets this PC's directory snapshot (module head).
    fn refresh_routes(&self) {
        let mut next: HashMap<PeerId, Route> = HashMap::new();
        if let Some(running) = &self.running {
            let book = lock(&self.core.book);
            for node in book.nodes() {
                // ADR-0390 d4: a PC just removed is still in the book until the actor handles its revocation — never routed to.
                if !self.core.is_trusted(node) {
                    continue;
                }
                let (Some(key), Some(run)) = (book.chosen(node), book.chosen_run(node)) else {
                    continue;
                };
                if let Some(handle) = running.handles.get(&key) {
                    next.insert(
                        *node,
                        Route {
                            key,
                            epoch: running.epoch,
                            peer: handle.peer.clone(),
                            run,
                        },
                    );
                }
            }
        }
        // ADR-0391: a directory frame counts only from the PC's chosen link — checked against this record, under the copies' lock alone.
        lock(&self.core.copies).set_chosen(
            next.iter()
                .map(|(node, route)| (*node, route.key))
                .collect(),
        );
        let mut publisher = lock(&self.core.publisher);
        let mut routes = lock(&self.core.routes);
        if self.core.host.is_some() {
            for (node, route) in &next {
                let old = routes.get(node);
                let fresh = old.map_or(true, |old| {
                    (old.key, old.epoch, old.run) != (route.key, route.epoch, route.run)
                });
                if fresh {
                    let snapshot = publisher.snapshot();
                    self.core.send_snapshot(&snapshot, node, &route.peer, None);
                }
                // ADR-0391: directory frames count only from the chosen link, so what the other PC sent on this link before it was
                //   chosen here was dropped — ask for its snapshot (again).
                if fresh {
                    if let Err(e) = route.peer.notify(PeerMsg::DirResync) {
                        tracing::debug!(pc = %node, "a directory resync request was not sent: {e}");
                    }
                }
            }
        }
        *routes = next;
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
        lock(&self.core.resyncs).epoch = self.epoch;
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
        // Its links are gone; their resync state with them (link keys are never reused, so this only frees memory). Its drains may still
        //   read a request until `Running`'s drop cuts them — the retired epoch keeps such a request from adding state back (`on_resync`).
        {
            let mut resyncs = lock(&self.core.resyncs);
            resyncs.epoch = 0;
            resyncs.links.clear();
        }
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
            lock(&self.core.copies).drop_node(&id);
            lock(&self.core.inbound).remove(&id);
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
        lock(&self.core.resyncs).links.remove(&key);
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
fn route(core: &Core, node: &PeerId, key: LinkKey, msg: PeerMsg) -> Routed {
    if !core.is_trusted(node) {
        return Routed::NotTrusted;
    }
    match msg {
        PeerMsg::Probe { nonce } => Routed::Reply(PeerMsg::ProbeAck { nonce }),
        PeerMsg::ProbeAck { nonce } => {
            if lock(&core.book).claim_probe_answer(key, nonce) {
                Routed::Forward(LinkEvent::ProbeAck(nonce))
            } else {
                tracing::debug!(pc = %node, "a ProbeAck that answers no open probe on this link (or not first) — dropped");
                Routed::Drop
            }
        }
        PeerMsg::Duplicate { kind } => {
            if lock(&core.book).duplicate_reported(node, kind) {
                tracing::warn!(pc = %node, ?kind, "that PC reports this PC's identity in use by another running copy — look for a second running copy; a new key re-pairs every PC");
                core.bump();
            }
            Routed::Drop
        }
        PeerMsg::Hello { .. } => {
            tracing::debug!(pc = %node, "a Hello after the handshake — ignored");
            Routed::Drop
        }
        PeerMsg::Mail(mail) => Routed::Mail(mail),
        PeerMsg::ReplyOverdue { request_id } => Routed::Overdue(request_id),
        PeerMsg::DirSnapshot(snapshot) => Routed::Snapshot(snapshot),
        PeerMsg::DirDelta(delta) => Routed::Delta(delta),
        PeerMsg::DirResync => Routed::Resync,
        PeerMsg::MailVerdict(_) => {
            // Verdicts are matched to their request in the transport; one that reaches the inbox answers no waiting request (late).
            tracing::debug!(pc = %node, "a mail verdict no request waits for (past its deadline) — dropped");
            Routed::Drop
        }
    }
}

/// Queues an inbound mail and starts what may run ([`super::mail`] module head).
fn on_mail(core: &Arc<Core>, node: PeerId, mail: Mail, reply: Reply) {
    if core.host.is_none() {
        let refusal = Refusal {
            code: "REMOTE_REFUSED",
            reason: "remote mail is not enabled on this PC".to_string(),
        };
        answer(&node, &reply, &mail.id, refuse_all(&mail, &refusal));
        return;
    }
    if mail.rows.len() > MAX_MAIL_ROWS {
        tracing::warn!(pc = %node, rows = mail.rows.len(), "a mail from that PC names more agents than one mail may — refused");
        let refusal = Refusal {
            code: "REMOTE_REFUSED",
            reason: format!("more than {MAX_MAIL_ROWS} recipients in one mail"),
        };
        answer(&node, &reply, &mail.id, refuse_all(&mail, &refusal));
        return;
    }
    // ADR-0411: the frame cap is sized for directories — a mail is bounded on its own before it is held in the queue.
    let bytes = mail_bytes(&mail);
    if bytes > MAX_MAIL_BYTES {
        tracing::warn!(pc = %node, bytes, "a mail from that PC is larger than one mail may be — refused");
        let refusal = Refusal {
            code: "REMOTE_REFUSED",
            reason: format!("the mail is over {} KiB", MAX_MAIL_BYTES >> 10),
        };
        answer(&node, &reply, &mail.id, refuse_all(&mail, &refusal));
        return;
    }
    let arrived = core.clock.now();
    let pushed = lock(&core.inbound)
        .entry(node)
        .or_insert_with(|| {
            Inbound::new(
                core.inbound_gen
                    .fetch_add(1, std::sync::atomic::Ordering::AcqRel),
            )
        })
        .push(mail, arrived, reply);
    if let Err(job) = pushed {
        tracing::warn!(pc = %node, "a mail from that PC was refused — its receive queue is full");
        let refusal = Refusal {
            code: "REMOTE_LIMIT",
            reason: "receiver busy — too many mails from this PC are waiting".to_string(),
        };
        answer(
            &node,
            &job.reply,
            &job.mail.id,
            refuse_all(&job.mail, &refusal),
        );
        return;
    }
    run_inbound(core, node);
}

/// Starts the frames of `node`'s receive queue that may run now, each on the blocking pool; drops the expired ones.
fn run_inbound(core: &Arc<Core>, node: PeerId) {
    let now = core.clock.now();
    let started = match lock(&core.inbound).get_mut(&node) {
        Some(queue) => queue.start(now),
        None => return,
    };
    for job in started.expired {
        // ADR-0404 d5: the sender has reported it unconfirmed by now; running it late would break the overdue grace.
        tracing::warn!(pc = %node, waited_s = now.saturating_duration_since(job.arrived).as_secs(), "a mail from that PC waited past its transport deadline — dropped");
    }
    for job in started.run {
        let core_for_job = core.clone();
        core.runtime
            .spawn_blocking(move || run_job(core_for_job, node, job));
    }
}

/// Runs one inbound frame on the blocking pool. Its queue slot is given back on every exit, a panic included ([`Finished`]).
fn run_job(core: Arc<Core>, node: PeerId, job: Job<Reply>) {
    let _finished = Finished {
        core: core.clone(),
        node,
        gen: job.gen,
        tokens: job.tokens().to_vec(),
    };
    // ADR-0404 d5: the job may have waited for a blocking thread — past the deadline since arrival it is dropped, not run late.
    let waited = core.clock.now().saturating_duration_since(job.arrived);
    if waited > MAIL_DEADLINE {
        tracing::warn!(pc = %node, waited_s = waited.as_secs(), "a mail from that PC waited past its transport deadline — dropped");
        return;
    }
    let pc = core.published().trusted(&node).map(|p| p.name.clone());
    match (pc, &core.host) {
        (Some(pc), Some(host)) => {
            let rows = host
                .mail(node, &pc, &job.mail, job.arrived)
                .unwrap_or_else(|refusal| refuse_all(&job.mail, &refusal));
            answer(&node, &job.reply, &job.mail.id, rows);
        }
        _ => {
            tracing::debug!(pc = %node, "a mail from a PC no longer trusted — dropped");
        }
    }
}

/// Gives a running frame's slot and tokens back to the queue it started from (not to a newer queue of the same PC) and starts what may
///   run next — on drop, so a panic in the host does not leak the slot.
struct Finished {
    core: Arc<Core>,
    node: PeerId,
    gen: u64,
    tokens: Vec<String>,
}

impl Drop for Finished {
    fn drop(&mut self) {
        {
            let mut inbound = lock(&self.core.inbound);
            if let Some(queue) = inbound
                .get_mut(&self.node)
                .filter(|queue| queue.gen() == self.gen)
            {
                queue.finish(&self.tokens);
                if queue.is_idle() {
                    inbound.remove(&self.node);
                }
            }
        }
        run_inbound(&self.core, self.node);
    }
}

/// Sends a verdict on the connection the mail came on — after a reconnect it is not sent (the sender reports that mail unconfirmed).
fn answer(node: &PeerId, reply: &Reply, id: &str, mut rows: Vec<super::wire::VerdictRow>) {
    // ADR-0390: an answer over the frame cap would end this PC's own link — rows are left out until it fits (the sender settles them
    //   unconfirmed).
    let mut verdict = PeerMsg::MailVerdict(MailVerdict {
        id: id.to_string(),
        rows: Vec::new(),
    });
    loop {
        if let PeerMsg::MailVerdict(v) = &mut verdict {
            v.rows = std::mem::take(&mut rows);
        }
        if encoded_len(&verdict).is_ok_and(|n| n <= MAX_FRAME_BYTES) {
            break;
        }
        if let PeerMsg::MailVerdict(v) = &mut verdict {
            rows = std::mem::take(&mut v.rows);
        }
        if rows.pop().is_none() {
            break;
        }
        tracing::warn!(pc = %node, "a mail verdict over the frame cap — a row left out");
    }
    if let Err(e) = reply.peer.notify_in(reply.generation, verdict) {
        tracing::debug!(pc = %node, "a mail verdict was not sent — the link it came on is gone: {e}");
    }
}

/// Applies a directory frame from `node`'s link `key` to its copy; a frame that does not follow the copy asks for a fresh snapshot on the
///   same connection.
fn on_directory(core: &Core, node: PeerId, key: LinkKey, reply: &Reply, mut frame: Routed) {
    // One spelling per agent id, so the same agent written two ways is one row (dedup, shared-id checks).
    let canonical = |id: &mut String| {
        if let Ok(uuid) = uuid::Uuid::parse_str(id) {
            *id = uuid.to_string();
        }
    };
    match &mut frame {
        Routed::Snapshot(snapshot) => {
            snapshot
                .agents
                .iter_mut()
                .for_each(|a| canonical(&mut a.id));
            // Sorted here, off the copies' lock — the sort under it then finds the rows in order.
            sort(&mut snapshot.agents);
        }
        Routed::Delta(delta) => {
            delta.upsert.iter_mut().for_each(|a| canonical(&mut a.id));
            delta.remove.iter_mut().for_each(canonical);
        }
        _ => return,
    }
    let problems_of = |frame: &Routed| -> HashMap<String, &'static str> {
        let rows: Vec<&DirAgent> = match frame {
            Routed::Snapshot(snapshot) => snapshot.agents.iter().collect(),
            Routed::Delta(delta) => delta.upsert.iter().collect(),
            _ => Vec::new(),
        };
        rows.iter()
            .zip(core.row_problems(&rows))
            .filter_map(|(row, why)| why.map(|why| (row.id.clone(), why)))
            .collect()
    };
    // The host's checks run before the copies' lock is taken; the version is read first, so a change it does not show is one the
    //   re-check of the copies (`Core::recheck_copies`, after the change) will see.
    let version = core.local_ids_version();
    let mut problems = problems_of(&frame);
    #[cfg(test)]
    hold_directory(core, &frame);
    let (applied, report) = {
        let mut copies = lock(&core.copies);
        // ADR-0391: only the PC's chosen link writes its copy. A frame on any other link, or before the actor has chosen one, is
        //   dropped; every newly chosen link asks for the snapshot (`Actor::refresh_routes`).
        if !copies.is_chosen(&node, key) {
            tracing::debug!(pc = %node, "a directory frame on a link that is not that PC's chosen one — dropped");
            return;
        }
        // ADR-0390 d4: a removal between the frame's trust check and here has dropped the copy — it must not come back.
        if !core.is_trusted(&node) {
            return;
        }
        // ADR-0391: this PC's agent ids changed since the rows were checked, and the copies' re-check may already have run — check
        //   again here, so a row naming a new local agent is not committed past it.
        if core.local_ids_version() != version {
            problems = problems_of(&frame);
        }
        let check = |row: &DirAgent| problems.get(&row.id).copied();
        let applied = match frame {
            Routed::Snapshot(snapshot) => copies.snapshot(node, snapshot, &check),
            Routed::Delta(delta) => copies.delta(node, delta, &check),
            _ => return,
        };
        (applied, copies.take_report())
    };
    core.warn_directory(&node, report);
    match applied {
        Applied::Updated => core.bump(),
        Applied::Resync => {
            if let Err(e) = reply.peer.notify_in(reply.generation, PeerMsg::DirResync) {
                tracing::debug!(pc = %node, "a directory resync request was not sent: {e}");
            }
        }
        Applied::Ignored => {}
    }
}

/// Tests: waits at the directory hold when `frame` lists its id.
#[cfg(test)]
fn hold_directory(core: &Core, frame: &Routed) {
    let mut slot = lock(&core.dir_hold);
    let lists = |id: &str| match frame {
        Routed::Snapshot(snapshot) => snapshot.agents.iter().any(|a| a.id == id),
        Routed::Delta(delta) => delta.upsert.iter().any(|a| a.id == id),
        _ => false,
    };
    if !slot.as_ref().is_some_and(|(id, _, _)| lists(id)) {
        return;
    }
    let Some((_, reached, release)) = slot.take() else {
        return;
    };
    drop(slot);
    let _ = reached.send(());
    // On the drain task — a std channel, as `oneshot::Receiver::blocking_recv` refuses to run inside the runtime.
    let _ = release.recv();
}

/// Recomputes this PC's directory each time the host rings, coalesced, and sends the change on every chosen link (module head).
async fn publish_loop(core: Arc<Core>, host: Host) {
    // The host's local-id version the copies were last re-checked against.
    let mut checked: Option<u64> = None;
    loop {
        host.bell.rung().await;
        core.clock.sleep(COALESCE).await;
        let mail = host.mail.clone();
        let rechecking = core.clone();
        let read = tokio::task::spawn_blocking(move || {
            let directory = mail.directory();
            // ADR-0391: the host refreshed its local ids with this read — when they changed, rows admitted before an agent of this PC
            //   took one of them go now.
            let version = mail.local_ids_version();
            if checked != Some(version) {
                rechecking.recheck_copies();
            }
            (directory, version)
        })
        .await;
        let directory = match read {
            Ok((directory, version)) => {
                checked = Some(version);
                directory
            }
            Err(e) => {
                tracing::warn!("this PC's agent directory could not be read: {e}");
                continue;
            }
        };
        let mut publisher = lock(&core.publisher);
        let Some(publish) = publisher.update(directory) else {
            #[cfg(test)]
            core.publishes
                .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
            continue;
        };
        let routes: Vec<(PeerId, Route)> = lock(&core.routes)
            .iter()
            .map(|(node, route)| (*node, route.clone()))
            .collect();
        for (node, route) in routes {
            // ADR-0390 d4: a PC removed since its route was taken gets nothing.
            if !core.is_trusted(&node) {
                continue;
            }
            match &publish {
                Publish::Delta(delta) => {
                    #[cfg(test)]
                    core.dir_sent
                        .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
                    if let Err(e) = route.peer.notify(PeerMsg::DirDelta(delta.clone())) {
                        tracing::debug!(pc = %node, "a directory change was not sent: {e}");
                    }
                }
                Publish::Snapshot(snapshot) => {
                    core.send_snapshot(snapshot, &node, &route.peer, None)
                }
            }
        }
        drop(publisher);
        #[cfg(test)]
        core.publishes
            .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
    }
}

/// Answers a `DirResync` on its connection — off the drain task, at most once per [`RESYNC_INTERVAL`] per link, by one worker at a time:
///   requests arriving before the worker takes its target fold into it (the answer goes to the latest one's connection); one arriving
///   while it sends gets one more answer, scheduled by the worker as it ends. A request read by a drain of a retired endpoint (`epoch`)
///   is dropped.
fn on_resync(core: &Arc<Core>, epoch: u64, node: PeerId, key: LinkKey, reply: Reply) {
    if core.host.is_none() {
        return;
    }
    let now = core.clock.now();
    let wait = {
        let mut resyncs = lock(&core.resyncs);
        // The epoch is checked with the insert, under one lock: a teardown that cleared this state is never undone by its own drains.
        if resyncs.epoch != epoch {
            tracing::debug!(pc = %node, "a directory resync request on a link of a retired endpoint — dropped");
            return;
        }
        let resync = resyncs.links.entry(key).or_default();
        resync.target = Some(reply);
        if resync.running {
            tracing::debug!(pc = %node, "a directory resync request while one is due — folded into it");
            return;
        }
        resync.running = true;
        resync.last.map_or(Duration::ZERO, |last| {
            (last + RESYNC_INTERVAL).saturating_duration_since(now)
        })
    };
    schedule_resync(core.clone(), node, key, wait);
}

/// Runs a link's resync worker after `wait`: takes the latest target and sends this PC's snapshot to it.
fn schedule_resync(core: Arc<Core>, node: PeerId, key: LinkKey, wait: Duration) {
    let sleep = core.clock.sleep(wait);
    core.runtime.clone().spawn(async move {
        sleep.await;
        let runtime = core.runtime.clone();
        let _ = runtime
            .spawn_blocking(move || {
                let _answered = ResyncDone {
                    core: core.clone(),
                    node,
                    key,
                };
                let reply = lock(&core.resyncs)
                    .links
                    .get_mut(&key)
                    .and_then(|resync| resync.target.take());
                if let Some(reply) = reply.filter(|_| core.is_trusted(&node)) {
                    let mut publisher = lock(&core.publisher);
                    let snapshot = publisher.snapshot();
                    core.send_snapshot(&snapshot, &node, &reply.peer, Some(reply.generation));
                }
                #[cfg(test)]
                {
                    let hold = lock(&core.resync_hold).take();
                    if let Some((reached, release)) = hold {
                        let _ = reached.send(());
                        let _ = release.blocking_recv();
                    }
                }
            })
            .await;
    });
}

/// Ends a link's resync worker on every exit, a panic included: a target that arrived meanwhile gets one more answer an interval later;
///   otherwise the link is idle again. Nothing is left for a link that ended or an endpoint that was torn down meanwhile.
struct ResyncDone {
    core: Arc<Core>,
    node: PeerId,
    key: LinkKey,
}

impl Drop for ResyncDone {
    fn drop(&mut self) {
        let again = {
            let mut resyncs = lock(&self.core.resyncs);
            let Some(resync) = resyncs.links.get_mut(&self.key) else {
                return;
            };
            resync.last = Some(self.core.clock.now());
            resync.running = resync.target.is_some();
            resync.running
        };
        if again {
            schedule_resync(self.core.clone(), self.node, self.key, RESYNC_INTERVAL);
        }
    }
}

/// Queues an overdue report for the host: a request id another PC may not use is dropped; the others wait in that PC's queue (each id
///   once, at most [`MAX_OVERDUE_WAITING`] — past it the report is dropped with a warning) and are handed to the host on the blocking
///   pool, at most [`MAX_OVERDUE_RUNNING`] at once. A sweep on the other PC sends one report per overdue contract back to back, never
///   again, so a burst must be kept, not dropped. A removal drops the PC's queue and a stop drops every queue; each report checks trust
///   and the stop again as its job starts.
// ADR-0393
fn on_overdue(core: &Arc<Core>, node: PeerId, request_id: String) {
    let Some(host) = core.host.as_ref() else {
        return;
    };
    if !host.mail_id_ok(&request_id) {
        tracing::debug!(pc = %node, "an overdue report with a malformed request id — dropped");
        return;
    }
    if core.stopped.load(std::sync::atomic::Ordering::Acquire) {
        return;
    }
    {
        let mut overdue = lock(&core.overdue);
        let queue = overdue.entry(node).or_insert_with(|| OverdueQueue {
            gen: core
                .inbound_gen
                .fetch_add(1, std::sync::atomic::Ordering::AcqRel),
            waiting: std::collections::VecDeque::new(),
            running: 0,
        });
        if queue.waiting.contains(&request_id) {
            return;
        }
        if queue.waiting.len() >= MAX_OVERDUE_WAITING {
            tracing::warn!(pc = %node, waiting = queue.waiting.len(), "too many overdue reports from that PC are waiting — this one is dropped");
            return;
        }
        queue.waiting.push_back(request_id);
    }
    run_overdue(core, node);
}

/// Starts waiting overdue reports of `node` while fewer than [`MAX_OVERDUE_RUNNING`] run.
fn run_overdue(core: &Arc<Core>, node: PeerId) {
    let Some(host) = core.host.clone() else {
        return;
    };
    loop {
        let (gen, request_id) = {
            let mut overdue = lock(&core.overdue);
            let Some(queue) = overdue.get_mut(&node) else {
                return;
            };
            if queue.running >= MAX_OVERDUE_RUNNING {
                return;
            }
            let Some(request_id) = queue.waiting.pop_front() else {
                if queue.running == 0 {
                    overdue.remove(&node);
                }
                return;
            };
            queue.running += 1;
            (queue.gen, request_id)
        };
        let core = core.clone();
        let host = host.clone();
        core.runtime.clone().spawn_blocking(move || {
            let _done = OverdueDone {
                core: core.clone(),
                node,
                gen,
            };
            // ADR-0390 d4: a PC removed, or a daemon stopping, since the report was queued — nothing more is handed on.
            if core.stopped.load(std::sync::atomic::Ordering::Acquire) || !core.is_trusted(&node) {
                return;
            }
            host.reply_overdue(node, &request_id);
        });
    }
}

/// Gives an overdue slot back on every exit (a panic included) and starts the next waiting report.
struct OverdueDone {
    core: Arc<Core>,
    node: PeerId,
    /// The queue the report was started from.
    gen: u64,
}

impl Drop for OverdueDone {
    fn drop(&mut self) {
        if let Some(queue) = lock(&self.core.overdue)
            .get_mut(&self.node)
            .filter(|queue| queue.gen == self.gen)
        {
            queue.running = queue.running.saturating_sub(1);
        }
        run_overdue(&self.core, self.node);
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
                Routed::Forward(event) => event,
                Routed::Reply(msg) => {
                    if let Err(e) = peer.notify_in(arrival.generation, msg) {
                        tracing::debug!(pc = %node, "a probe answer was not sent: {e}");
                    }
                    continue;
                }
                Routed::Mail(mail) => {
                    let reply = Reply {
                        peer: peer.clone(),
                        generation: arrival.generation,
                    };
                    on_mail(&core, node, mail, reply);
                    continue;
                }
                Routed::Overdue(request_id) => {
                    on_overdue(&core, node, request_id);
                    continue;
                }
                frame @ (Routed::Snapshot(_) | Routed::Delta(_)) => {
                    let reply = Reply {
                        peer: peer.clone(),
                        generation: arrival.generation,
                    };
                    on_directory(&core, node, key, &reply, frame);
                    continue;
                }
                Routed::Resync => {
                    let reply = Reply {
                        peer: peer.clone(),
                        generation: arrival.generation,
                    };
                    on_resync(&core, epoch, node, key, reply);
                    continue;
                }
                Routed::NotTrusted => {
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
                Routed::Drop => continue,
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
