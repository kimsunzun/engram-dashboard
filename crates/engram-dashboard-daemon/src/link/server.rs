//! The daemon's accepting side on the transport — the `conn_port` contract (`FrameSink` · `FrameFanout` · `ConnectionHandler` calls) implemented over accepted transport peers (TRD S21 trd-3 §6-3 U4 · D11 2').
//!
//! The transport owns the socket: WS upgrade + Origin · Host check ([`accept_policy`]), the auth deadline, the single writer, keepalive (active ping · idle decision) and the close frame. This file owns what sits above: per connection one read task (reads that peer's lossless inbox),
//! the arrival-order lane + byte budget + saturation disposition, the single `dispatch_task`, the overlapping `bypass_task`, and the teardown order.
//!
//! Connection lifetime — three ends:
//! - dispatch finished (`ConnFlow::Close` · the lane closed after the read side stopped) → stop feeding · drain the priority lane · close the peer after flushing what is queued (`Peer::close_after_flush`).
//! - the link ended on the write side (write failure · write deadline · idle · our own close) or a [`FrameSink`] send failed → stop feeding · drain the priority lane · abort dispatch.
//! - the link ended on the read side (peer closed · read error) → the lanes close and dispatch drains what it holds; aborted at the first failed send or after one ping interval, so the last command of a client that sends and closes at once still runs, but a dispatch stuck in a handler does not hold the connection forever.
//!
//! In every branch `on_disconnect` runs before the fan-out list drops the connection, and the read task keeps draining the inbox to its end (`Closed` · `None`) after it stops feeding the lanes (transport receiver invariant 5).
// ADR-0206
// ADR-0310

use std::collections::BTreeMap;
use std::future::Future;
use std::net::{Ipv4Addr, SocketAddr};
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use engram_dashboard_transport::ws::{AcceptPolicy, AcceptRefusal, AcceptRefusals, WsListener};
use engram_dashboard_transport::{
    Accepted, Address, Clock, CloseCode, DialFailure, Dialer, DisconnectCause, Generation,
    Handshake, Inbox, Incoming, LinkError, LinkRx, LinkTx, Listener, Peer, PeerId, Policy,
    QueueFullPolicy, Registry, SendError, SystemClock, TransportEvent,
};
use futures_util::future::BoxFuture;
use futures_util::{Stream, StreamExt};
use tokio::sync::{mpsc, oneshot, Notify};

use super::auth::AuthCheck;
use super::handshake::{DaemonHandshake, RefusedHandshake};
use super::wire::{DaemonWire, RawIn, RawOut};
use crate::account_gate::AccountGate;
use crate::conn_port::{
    ConnFlow, ConnId, ConnectionHandler, ConnectionHandlerFactory, Frame, FrameError, FrameFanout,
    FrameSink, Saturated,
};

/// Per-connection outbound queue capacity: ReplayBuffer.max_events(4096) + control_slack(512) = 4608 — control headroom even when the whole replay fits (consistent with the output_core.rs invariant).
const CONN_TX_CAP: usize = 4608;

/// Number of entries in the per-connection inbound arrival-order lane — headroom so the read loop reads the next frame without waiting for the handler. The memory axis is [`CONN_RX_MAX_BYTES`] (accounting source of truth).
///
/// A burst absorber, not a backlog store: the consumer (`dispatch_task`) holds only one order-bearing frame at a time and long work is peeled off this lane by the upper layer (the "arrival order" contract of `conn_port::ConnectionHandler`). Unlike [`CONN_TX_CAP`], which must hold the entire replay, no large value is needed — do not compare them on one axis.
/// Even when full, the read loop does not stop — it asks the upper layer how to dispose of that frame and keeps reading ([`ConnectionHandler::on_inbound_saturated`]). The earlier "when full, the read loop waits" was a wrong design: the inbox is a stream, so the frames behind are not even taken out, and a frame chosen as "must not starve"
/// ([`Saturated::Bypass`]) would starve behind the blocked lane — the very symptom this arrangement exists to eliminate.
///  · It does not drop silently: [`Saturated::Refused`] means the upper layer already sent a reply (port contract obligation 2); the disposition when even that reply could not be sent is [`Saturated::Unanswered`] — only there is the connection ended.
///  · Otherwise it does not disconnect: the outbound queue ([`ConnFrameSink`]) disconnects on saturation because its caller is a synchronous path that cannot produce a reply (asymmetry source: `conn_port::FrameSink`); here the upper layer can state the refusal in words.
/// This value measures how many frames to hold without loss while the consumer is blocked; whichever of entries or byte budget fills first is saturation. With a healthy consumer it is never seen (only short frames with no await point remain), so being full signals a blocked consumer, where the answer is not to wait but to let through what should be skipped.
// ADR-0206
const CONN_RX_CAP: usize = 64;

/// Budget of payload bytes the arrival-order lane may hold — decides saturation together with [`CONN_RX_CAP`].
///
/// The entry count alone is not enough: each entry holds one whole frame of varying size, making the real ceiling `CONN_RX_CAP × (cap of one frame)` = 64 × [`MAX_MESSAGE_BYTES`].
/// The decision uses the fill level before adding (bytes held ≥ this value = saturated), so a frame arriving at an empty lane is always accepted whatever its size — a legitimate large command is never refused for size. The ceiling is "budget + one frame" (16 MiB + [`MAX_MESSAGE_BYTES`]). The transport's own byte budget below this lane (read-ahead · inbox) is counted by the transport.
/// Basis: this lane holds short control frames and a large one is an occasional single frame; 16 MiB leaves room even if several overlap, and a normal burst never sees it.
const CONN_RX_MAX_BYTES: usize = 16 * 1024 * 1024;

/// Capacity of the lane where a frame that skipped the arrival-order lane ([`Saturated::Bypass`]) stands.
///
/// Small: only things that try to unblock the blocked lane enter it, so in normal operation it uses no entry; 8 full means eight were sent and none finished — that path itself is blocked, and accepting more only lines up the same thing.
/// Here it waits (unlike the lane above): reading stopped by a full lane starves only the frames bound for the arrival-order lane, which were about to be refused anyway; nothing to protect starves. The wait stops only this connection's inbox, which the transport turns into backpressure on this peer's socket alone.
/// The byte budget is not applied: refusing by budget a frame the upper layer chose as "must not starve" removes this lane's reason. The ceiling is the entry count, 8 × [`MAX_MESSAGE_BYTES`]; that this lane is empty in normal operation (effective ceiling 0) is an observation, not a guarantee.
const CONN_BYPASS_CAP: usize = 8;

/// Grace period in connection cleanup waiting until the priority lane empties what it accepted ([`finish_bypass_lane`]).
///
/// It does not bear correctness — the waiting itself does; this is only the upper bound keeping the wait finite. Past it, it falls back to abort, so a wrong constant cannot be worse than aborting at once.
/// Size basis: an item here is an upper-layer action that "finishes something blocked", which has a section synchronously waiting for the target to die (today bounded by the upper layer's 5-second join); 10 s lets the one in progress + the next finish, and more backed up means that path itself is blocked (the same judgment as in [`CONN_BYPASS_CAP`]).
/// Held during the grace period: only that one connection's cleanup — not the accept loop or other connections.
const BYPASS_DRAIN_GRACE: Duration = Duration::from_secs(10);

/// How long a new connection may stay open without its first (auth) frame before it is closed — the transport's auth deadline for accepted peers (`Policy::connect_timeout`, from the end of the WS upgrade). 15 s: the open-source defaults read are 3–60 s, mostly 5–15 s (SignalR `HandshakeTimeout` 15 s, EMQX `idle_timeout` 15 s —
/// `docs/research/first-auth-frame-timeout-2026-11-06.md`); a short window turns a briefly stalled shell into a refusal.
// ADR-0347
const AUTH_TIMEOUT: Duration = Duration::from_secs(15);

const DEFAULT_PING_INTERVAL: Duration = Duration::from_secs(20);
/// 2.5x ping_interval (the connection drops only after several missed Pings — avoids false positives from a transient delay).
const DEFAULT_IDLE_TIMEOUT: Duration = Duration::from_secs(50);

/// The transport requires a write deadline below the ping interval (`Policy::check`); it is set as late as allowed — one millisecond under — so this cut fires as rarely as possible.
const WRITE_DEADLINE_MARGIN: Duration = Duration::from_millis(1);

/// The byte cap of one WS message and one frame, both directions — the shell dialer's value (`DAEMON_MESSAGE_MAX_BYTES`). Tighter than tungstenite's defaults (64 MiB message · 16 MiB frame) only for fragmented messages; a non-fragmenting client like the shell is bounded by the 16 MiB frame cap either way.
const MAX_MESSAGE_BYTES: usize = 16 << 20;

/// Upgrade permits — links from TCP accept until live (upgrade · ready queue · auth wait). Set far above any legitimate client count so it never refuses one.
const MAX_PENDING_UPGRADES: usize = 1024;

/// Refusal reports the listener may hold before the accept loop logs them — past it they are counted, not kept.
const REFUSAL_QUEUE: usize = 64;

/// Inbox length per accepted peer (TRD trd-3 §7 "inbound queue" — 16 entries). The read task drains it at once except while waiting on a full priority lane; then the transport stops reading that peer's socket.
const ACCEPTED_INBOX: usize = 16;

/// Allowed Origin allowlist. No Origin (native/harness) is allowed — the token is the main defense.
/// Unmeasured — known unverified: the four strings below are design values; the Origins Tauri WebView2 · mobile actually send were never measured. A mismatch = 403. When growing the client surface (packaging variants · mobile tunnels), do not treat the list as verified — a 403 may be this list's fault, not a client bug.
const ALLOWED_ORIGINS: &[&str] = &[
    "http://localhost:1420",
    "http://127.0.0.1:1420",
    "tauri://localhost",
    "https://tauri.localhost",
];

/// Host names a WS upgrade may carry (the port must be the bound one). The transport accepts only listed names (an upgrade without Host passes), so the list is the names a loopback client dials by.
const ALLOWED_HOST_NAMES: &[&str] = &["127.0.0.1", "localhost"];

/// WS application-level keepalive settings — the transport's ping interval and idle timeout for accepted peers.
///
/// Test injection: hard-coded constants would make tests take tens of seconds, so short values (e.g. 200ms/600ms) are injectable; the production path stays `default()` (20s/50s).
#[derive(Clone, Copy, Debug)]
pub struct KeepaliveConfig {
    pub ping_interval: Duration,
    pub idle_timeout: Duration,
}

impl Default for KeepaliveConfig {
    fn default() -> Self {
        Self {
            ping_interval: DEFAULT_PING_INTERVAL,
            idle_timeout: DEFAULT_IDLE_TIMEOUT,
        }
    }
}

/// The accept policy of the daemon's WS listener.
fn accept_policy() -> AcceptPolicy {
    AcceptPolicy {
        allowed_origins: ALLOWED_ORIGINS.iter().map(|s| s.to_string()).collect(),
        allowed_host_names: ALLOWED_HOST_NAMES.iter().map(|s| s.to_string()).collect(),
        // The transport requires an upgrade deadline — the auth window's length.
        upgrade_timeout: AUTH_TIMEOUT,
        max_pending_upgrades: MAX_PENDING_UPGRADES,
        max_message_bytes: MAX_MESSAGE_BYTES,
        refusal_queue: REFUSAL_QUEUE,
    }
}

/// The transport policy of one accepted peer: the daemon sets the auth window · keepalive · outbound queue; the rest are transport defaults. Outbound saturation is not a transport disconnect — the per-connection sink closes on it ([`ConnFrameSink`]) and the fan-out skips it ([`ConnRegistry`]), the asymmetry of `conn_port::FrameFanout`.
fn accepted_policy(keepalive: KeepaliveConfig) -> Policy {
    Policy {
        connect_timeout: AUTH_TIMEOUT,
        write_deadline: keepalive
            .ping_interval
            .saturating_sub(WRITE_DEADLINE_MARGIN),
        ping_interval: keepalive.ping_interval,
        idle_timeout: keepalive.idle_timeout,
        outbound_queue: CONN_TX_CAP,
        inbound_queue: ACCEPTED_INBOX,
        full_queue: QueueFullPolicy::DropAndReport,
        ..Policy::default()
    }
}

// ── listener ──────────────────────────────────────────────────────────────────────────

/// The bound loopback WS listener with its refusal reports and the clock its accepted links are stamped with.
pub(crate) struct Listening {
    listener: WsListener,
    refusals: AcceptRefusals,
    clock: Arc<dyn Clock>,
}

impl Listening {
    /// Binds `127.0.0.1` on a port the kernel picks. Call it inside the tokio runtime.
    pub(crate) fn bind_loopback() -> Result<Self, LinkError> {
        let clock: Arc<dyn Clock> = Arc::new(SystemClock);
        let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, 0));
        let (listener, refusals) = WsListener::bind(addr, accept_policy(), clock.clone())?;
        Ok(Self {
            listener,
            refusals,
            clock,
        })
    }

    pub(crate) fn port(&self) -> u16 {
        self.listener.local_addr().port()
    }

    /// The clock the registry must share with this listener (`WsListener::bind` contract).
    pub(crate) fn clock(&self) -> Arc<dyn Clock> {
        self.clock.clone()
    }
}

/// Values are already masked by the transport.
fn log_refusal(refusal: &AcceptRefusal) {
    tracing::warn!(
        kind = ?refusal.kind,
        remote = ?refusal.remote,
        origin = ?refusal.origin,
        host = ?refusal.host,
        "WS upgrade failed (or Origin refused)"
    );
}

/// The registry only accepts — `Registry::new` takes a dialer, which only `Registry::add` would use.
struct NoDialer;

impl Dialer for NoDialer {
    fn dial(
        &self,
        _addr: &Address,
    ) -> BoxFuture<'_, Result<(Box<dyn LinkTx>, Box<dyn LinkRx>), DialFailure>> {
        Box::pin(async {
            Err(DialFailure::Unreachable(LinkError::new(
                "the daemon link registry does not dial",
            )))
        })
    }
}

// ── fan-out registry ─────────────────────────────────────────────────────────────────

/// One live connection's way out — the peer and the connection generation it went live on.
#[derive(Clone)]
struct ConnOutlet {
    peer: Peer<DaemonWire>,
    generation: Generation,
}

/// Every authenticated connection, for the all-connection fan-out ([`FrameFanout`]), and the connection id issuer.
///
/// Connection ids start at 1 and only go up — unregistering never returns an id; the command bus relies on that (`command_delivery::Pending::origin` · `CommandBus::open_caller`).
#[derive(Clone)]
pub(crate) struct ConnRegistry {
    inner: Arc<Mutex<BTreeMap<ConnId, ConnOutlet>>>,
    next_id: Arc<AtomicU64>,
}

impl ConnRegistry {
    pub(crate) fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(BTreeMap::new())),
            next_id: Arc::new(AtomicU64::new(1)),
        }
    }

    fn alloc_id(&self) -> ConnId {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    fn register(&self, id: ConnId, outlet: ConnOutlet) {
        self.inner
            .lock()
            .expect("conn registry poisoned")
            .insert(id, outlet);
    }

    fn unregister(&self, id: ConnId) {
        self.inner
            .lock()
            .expect("conn registry poisoned")
            .remove(&id);
    }

    /// Whether this conn is still a fan-out target — the test observes the cleanup order (release registration after cleanup).
    #[cfg(test)]
    fn contains(&self, id: ConnId) -> bool {
        self.inner
            .lock()
            .expect("conn registry poisoned")
            .contains_key(&id)
    }

    #[cfg(test)]
    fn outlet(&self, id: ConnId) -> Option<ConnOutlet> {
        self.inner
            .lock()
            .expect("conn registry poisoned")
            .get(&id)
            .cloned()
    }
}

impl FrameFanout for ConnRegistry {
    /// Do not send while holding the map locked (ADR-0006 lock order): take a snapshot, release the lock, send with the copy.
    /// One saturated connection is skipped, not closed (`conn_port::FrameFanout` duties 2 · 3) — the transport also counts it on that peer's inbox (`TransportEvent::Dropped`).
    fn broadcast_text(&self, text: String) {
        let conns: Vec<(ConnId, ConnOutlet)> = {
            let guard = self.inner.lock().expect("conn registry poisoned");
            guard.iter().map(|(id, o)| (*id, o.clone())).collect()
        };
        for (id, outlet) in conns {
            if let Err(e) = outlet
                .peer
                .notify_in(outlet.generation, RawOut::Text(text.clone()))
            {
                tracing::warn!(
                    conn = id,
                    "all-connection fanout try_send failed (slow consumer): {e}"
                );
            }
        }
    }
}

impl Default for ConnRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ── ConnFrameSink (per-connection frame outlet) ──────────────────────────────────────

/// The per-connection [`FrameSink`] — puts frames on the peer's outbound queue without waiting.
///
/// Saturation closes the connection (`conn_port::FrameSink::try_send`): the peer is closed at once and the connection task woken through `write_end`. `send` does not wait for room — the transport has no waiting send (a waiting sender on the read side would form a cycle with the supervisor — TRD trd-3 §3-0 1), so it behaves as `try_send`.
/// A send on a link no longer live also wakes `write_end`; a failed send ends the connection.
/// `Frame::Close` = drain then close: earlier frames go out first (`Peer::close_after_flush`); the reason is logged, not sent.
pub(crate) struct ConnFrameSink {
    outlet: ConnOutlet,
    write_end: Arc<Notify>,
}

impl ConnFrameSink {
    fn new(outlet: ConnOutlet, write_end: Arc<Notify>) -> Self {
        Self { outlet, write_end }
    }

    fn put(&self, frame: Frame) -> Result<(), FrameError> {
        let out = match frame {
            Frame::Text(text) => RawOut::Text(text),
            Frame::Binary(bytes) => RawOut::Binary(bytes),
            Frame::Close(reason) => {
                tracing::info!(%reason, "close signal — closing after flushing what was sent");
                let _closing = self
                    .outlet
                    .peer
                    .close_after_flush(CloseCode::GOING_AWAY, "");
                return Ok(());
            }
        };
        match self.outlet.peer.notify_in(self.outlet.generation, out) {
            Ok(()) => Ok(()),
            Err(SendError::QueueFull) => {
                self.outlet.peer.close();
                self.write_end.notify_one();
                Err(FrameError)
            }
            Err(SendError::Disconnected) => {
                self.write_end.notify_one();
                Err(FrameError)
            }
            // Closing after a `Frame::Close` — the link ends by itself once the earlier frames are out.
            Err(SendError::Closed) => Err(FrameError),
        }
    }
}

impl FrameSink for ConnFrameSink {
    fn try_send(&self, frame: Frame) -> Result<(), FrameError> {
        self.put(frame)
    }

    fn send(&self, frame: Frame) -> BoxFuture<'_, Result<(), FrameError>> {
        Box::pin(async move { self.put(frame) })
    }
}

// ── server ────────────────────────────────────────────────────────────────────────────

/// One daemon's accepting side — the transport registry of accepted peers plus what every connection shares.
pub(crate) struct Server {
    registry: Registry<DaemonWire>,
    conns: ConnRegistry,
    handlers: Arc<dyn ConnectionHandlerFactory>,
    auth: Arc<AuthCheck>,
    /// The accept-time account check — a client of another OS account is refused before its first frame is read (ADR-0395).
    gate: AccountGate,
    policy: Policy,
    /// The read-end wait ([`Server::run_connection`]) — after the read side ends, dispatch gets at most one ping interval.
    ping_interval: Duration,
    /// Registry labels of accepted links — handed out at accept, before auth; connection ids are issued after auth.
    labels: AtomicU64,
}

type InboxStream = Pin<Box<dyn Stream<Item = Incoming<DaemonWire>> + Send>>;

fn inbox_stream(inbox: Inbox<DaemonWire>) -> InboxStream {
    Box::pin(futures_util::stream::unfold(
        inbox,
        |mut inbox| async move { inbox.recv().await.map(|item| (item, inbox)) },
    ))
}

impl Server {
    /// `expected_token` is this run's token (the one written to the owner-only token file) and `expected_protocol_version` the
    /// version to accept — both injected, so this layer reads neither. `conns` must be the registry the upper layer's fan-out sends
    /// through (`DaemonWiring`). `clock` = [`Listening::clock`]. `gate` = the daemon's own account and the client-account lookup.
    pub(crate) fn new(
        clock: Arc<dyn Clock>,
        conns: ConnRegistry,
        handlers: Arc<dyn ConnectionHandlerFactory>,
        expected_token: &str,
        expected_protocol_version: u32,
        keepalive: KeepaliveConfig,
        gate: AccountGate,
    ) -> Arc<Self> {
        let policy = accepted_policy(keepalive);
        Arc::new(Self {
            registry: Registry::new(
                Arc::new(DaemonWire),
                Arc::new(NoDialer),
                clock,
                policy.clone(),
            ),
            conns,
            handlers,
            auth: Arc::new(AuthCheck::new(
                expected_token.to_string(),
                expected_protocol_version,
            )),
            gate,
            policy,
            ping_interval: keepalive.ping_interval,
            labels: AtomicU64::new(1),
        })
    }

    /// Accepts until `stop` resolves, then closes every connection after flushing what each has queued (the stop `Ack` · a
    /// last `Error`) and waits for them up to the flush deadline.
    pub(crate) async fn run(self: Arc<Self>, listening: Listening, stop: impl Future<Output = ()>) {
        let Listening {
            mut listener,
            mut refusals,
            clock: _,
        } = listening;
        tokio::pin!(stop);
        let mut accepting = true;
        let mut reporting = true;
        loop {
            tokio::select! {
                () = &mut stop => break,
                accepted = listener.accept(), if accepting => match accepted {
                    Ok(accepted) => {
                        tracing::debug!(remote = ?accepted.info().remote, "connection accepted — handing to the WS handler");
                        tokio::spawn(self.serve(accepted));
                    }
                    // Terminal — the listener's accept task is gone (`Listener::accept` contract).
                    Err(e) => {
                        tracing::error!("WS listener ended — no further connections are accepted: {e}");
                        accepting = false;
                    }
                },
                refusal = refusals.recv(), if reporting => match refusal {
                    Some(refusal) => log_refusal(&refusal),
                    None => reporting = false,
                },
            }
        }
        drop(listener);
        if self
            .registry
            .close_all(self.policy.flush_timeout)
            .await
            .is_err()
        {
            tracing::warn!(
                deadline_ms = self.policy.flush_timeout.as_millis() as u64,
                "connections did not all finish closing before the deadline"
            );
        }
    }

    /// Checks the client's OS account, puts the accepted link on the registry and returns the connection's whole life — handshake,
    ///   operation, cleanup. A refused client still goes through the registry, with a handshake that only sends the refusal and closes, so
    ///   the auth deadline, the permit and the close follow the one path every link takes.
    // ADR-0395
    fn serve(self: &Arc<Self>, accepted: Accepted) -> impl Future<Output = ()> + Send + 'static {
        let remote = accepted.info().remote.clone().unwrap_or_else(|| "?".into());
        let label = PeerId::new(format!(
            "accepted-{}",
            self.labels.fetch_add(1, Ordering::Relaxed)
        ));
        let ends = accepted.info().socket_ends;
        let this = Arc::clone(self);
        async move {
            // No ends (not a TCP link) cannot be verified — the gate refuses and logs it, throttled like every refusal.
            let admitted = this
                .gate
                .admit(ends.map(|e| e.peer), ends.map(|e| e.local))
                .await;
            let handshake: Arc<dyn Handshake> = match admitted {
                Ok(()) => Arc::new(DaemonHandshake::new(
                    this.auth.clone(),
                    this.handlers.clone(),
                    remote.clone(),
                )),
                Err(refusal) => Arc::new(RefusedHandshake::new(
                    this.handlers.as_ref(),
                    refusal.message(),
                )),
            };
            let (peer, inbox) = this.registry.attach(label, accepted, handshake);
            this.run_connection(peer, inbox, remote).await
        }
    }

    async fn run_connection(
        self: Arc<Self>,
        peer: Peer<DaemonWire>,
        inbox: Inbox<DaemonWire>,
        remote: String,
    ) {
        let mut incoming = inbox_stream(inbox);
        let Some(generation) = wait_live(&mut incoming, &remote).await else {
            return;
        };

        let conn_id = self.conns.alloc_id();
        let outlet = ConnOutlet {
            peer: peer.clone(),
            generation,
        };
        self.conns.register(conn_id, outlet.clone());
        tracing::info!(peer = %remote, conn = conn_id, "connection authenticated — registering");

        let write_end = Arc::new(Notify::new());
        let frames: Arc<dyn FrameSink> = Arc::new(ConnFrameSink::new(outlet, write_end.clone()));
        let handler = self.handlers.handler_for(conn_id);

        // This must come before spawning the tasks so command dispatch cannot get ahead of the greeting.
        handler.on_connect(conn_id, &frames).await;

        // Inbound queue = the boundary between the read layer and the processing layer: reading only takes from the inbox and puts here; handler calls are borne by [`dispatch_task`] in arrival order. Capacity and saturation basis: [`CONN_RX_CAP`].
        let (inbound_tx, inbound_rx) = mpsc::channel::<Inbound>(CONN_RX_CAP);
        // The byte budget fill level, deciding saturation together with the entry count — basis: [`CONN_RX_MAX_BYTES`].
        let rx_bytes: RxBytes = Arc::new(AtomicUsize::new(0));
        // The second lane for frames that must not starve behind a blocked lane — why separate capacity and consumer: [`CONN_BYPASS_CAP`] · [`bypass_task`].
        let (bypass_tx, bypass_rx) = mpsc::channel::<Inbound>(CONN_BYPASS_CAP);
        let (stop_feeding, feeding_stopped) = oneshot::channel::<()>();

        let mut read_handle = tokio::spawn(read_task(
            incoming,
            inbound_tx,
            bypass_tx,
            rx_bytes.clone(),
            frames.clone(),
            handler.clone(),
            conn_id,
            async move {
                let _ = feeding_stopped.await;
            },
        ));
        let bypass_handle = tokio::spawn(bypass_task(
            bypass_rx,
            frames.clone(),
            handler.clone(),
            conn_id,
        ));
        let mut dispatch_handle = tokio::spawn(dispatch_task(
            inbound_rx,
            rx_bytes,
            frames,
            handler.clone(),
            conn_id,
        ));

        // Explicitly abort the survivor: a dropped JoinHandle detaches the task, so select on `&mut` (handle not consumed) and abort the losing side.
        // The priority lane (bypass_task) is not a select target: its ending only means feeding stopped (its sender is in read), not a reason to cut the connection. It is not aborted but drained: its frames were accepted as "must not starve", so stop feeding first to release that lane's only sender, and [`finish_bypass_lane`] waits for the drain.
        // A read end does not abort at once: the last command of a client that sends one command and closes right away still runs — dispatch drains the lane, and only a failed send or one ping interval ends the wait.
        let mut link_ended = false;
        tokio::select! {
            _ = &mut dispatch_handle => {
                tracing::debug!(conn = conn_id, "dispatch_task ended → stop read · drain bypass · close + cleanup after draining");
                let _ = stop_feeding.send(());
                finish_bypass_lane(bypass_handle, conn_id).await;
                let _closing = peer.close_after_flush(CloseCode::GOING_AWAY, "");
            }
            () = write_end.notified() => {
                tracing::debug!(conn = conn_id, "send failed → stop read · drain bypass · dispatch abort + cleanup");
                let _ = stop_feeding.send(());
                finish_bypass_lane(bypass_handle, conn_id).await;
                dispatch_handle.abort();
                peer.close();
            }
            end = &mut read_handle => {
                link_ended = true;
                // A panicked read task leaves no cause — treated as a write end (abort).
                let end = end.unwrap_or_default();
                if end.read_side() {
                    tracing::debug!(conn = conn_id, cause = ?end.cause, "read ended → dispatch drains what is left");
                    tokio::select! {
                        _ = &mut dispatch_handle => {
                            finish_bypass_lane(bypass_handle, conn_id).await;
                        }
                        () = write_end.notified() => {
                            finish_bypass_lane(bypass_handle, conn_id).await;
                            dispatch_handle.abort();
                        }
                        () = tokio::time::sleep(self.ping_interval) => {
                            tracing::debug!(conn = conn_id, "one ping period after the read end — dispatch abort");
                            finish_bypass_lane(bypass_handle, conn_id).await;
                            dispatch_handle.abort();
                        }
                    }
                } else {
                    tracing::debug!(conn = conn_id, cause = ?end.cause, "write ended → drain bypass · dispatch abort + cleanup");
                    finish_bypass_lane(bypass_handle, conn_id).await;
                    dispatch_handle.abort();
                }
            }
        }

        // ── cleanup (leak prevention — review required) ──────────────────────────────────────────
        handler.on_disconnect(conn_id);
        self.conns.unregister(conn_id);
        tracing::info!(peer = %remote, conn = conn_id, "connection ended — cleanup done");
        // The read task keeps draining the inbox to its end — every branch above has closed the peer or seen it end.
        if !link_ended {
            let _ = read_handle.await;
        }
    }
}

/// Drains the inbox until the handshake ends: `Some` = the link went live on that generation; `None` = it ended before auth (refused · deadline · peer left), the inbox drained to its end.
async fn wait_live<S>(incoming: &mut S, peer: &str) -> Option<Generation>
where
    S: Stream<Item = Incoming<DaemonWire>> + Unpin,
{
    while let Some(item) = incoming.next().await {
        match item {
            Incoming::Event(TransportEvent::Connected { generation, .. }) => {
                return Some(generation)
            }
            Incoming::Event(TransportEvent::Closed { cause, .. }) => {
                log_handshake_end(peer, cause);
                break;
            }
            // Nothing else comes before the link goes live.
            _ => {}
        }
    }
    while incoming.next().await.is_some() {}
    None
}

fn log_handshake_end(peer: &str, cause: DisconnectCause) {
    match cause {
        DisconnectCause::Idle => tracing::warn!(
            %peer,
            timeout_ms = AUTH_TIMEOUT.as_millis() as u64,
            "no auth frame in time — closing"
        ),
        DisconnectCause::PeerClosed => tracing::warn!(%peer, "connection ended before auth"),
        DisconnectCause::ReadFailed => tracing::warn!(%peer, "first frame receive error — close"),
        // Our refusal — `AuthCheck` logged why — or the daemon shutting down.
        DisconnectCause::Explicit => tracing::debug!(%peer, "handshake ended by the daemon"),
        other => tracing::warn!(%peer, cause = ?other, "handshake ended — close"),
    }
}

// ── read_task + dispatch_task (two layers split by the inbound queue) ─────────────────────────────

/// The unit the read loop hands to the inbound queue.
///
/// Unlike the outgoing [`Frame`], there is no Close: an outgoing Close is a marker the upper layer puts in the queue, but incoming the close decision is made by the handler's answer (`ConnFlow::Close`), not frame content.
enum Inbound {
    Text(String),
    Binary(Vec<u8>),
}

impl From<RawIn> for Inbound {
    fn from(raw: RawIn) -> Self {
        match raw {
            RawIn::Text(text) => Inbound::Text(text),
            RawIn::Binary(bytes) => Inbound::Binary(bytes),
        }
    }
}

impl Inbound {
    /// The payload bytes this frame holds in the lane — the unit of [`CONN_RX_MAX_BYTES`] accounting.
    fn payload_len(&self) -> usize {
        match self {
            Inbound::Text(t) => t.len(),
            Inbound::Binary(b) => b.len(),
        }
    }
}

/// The payload bytes the arrival-order lane currently holds — the producer (`read_task`) adds, the consumer (`dispatch_task`) subtracts. One per connection.
///
/// Subtracting after the handler call is intended: subtracting on take-out would count a taken-but-still-processing frame as 0 and double the budget (the same-axis mistake: the in-flight accounting of `input_queue`).
type RxBytes = Arc<AtomicUsize>;

/// How the link ended, as the read task saw it — the cause of the transport's `Disconnected` · `Closed`.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct LinkEnd {
    /// `None` = the inbox ended without a `Closed` (the supervisor did not finish a normal cleanup).
    cause: Option<DisconnectCause>,
    over: bool,
}

impl LinkEnd {
    /// Takes one transport event. `true` = it was the last item of the inbox.
    fn observe(&mut self, event: TransportEvent<DaemonWire>, conn_id: ConnId) -> bool {
        match event {
            TransportEvent::Disconnected { cause, .. } => {
                self.cause = Some(cause);
                false
            }
            TransportEvent::Closed { cause, .. } => {
                self.cause = Some(cause);
                self.over = true;
                true
            }
            TransportEvent::Dropped {
                direction, count, ..
            } => {
                tracing::debug!(
                    conn = conn_id,
                    ?direction,
                    count,
                    "transport dropped frames"
                );
                false
            }
            other => {
                tracing::debug!(conn = conn_id, kind = other.kind(), "transport event");
                false
            }
        }
    }

    /// The input ended (peer closed · read error). Every other end is the write side's (write failure · write deadline · idle · our own close).
    fn read_side(&self) -> bool {
        matches!(
            self.cause,
            Some(DisconnectCause::PeerClosed | DisconnectCause::ReadFailed)
        )
    }
}

/// The source is generic so an isolation harness can run this loop on a synthetic item sequence without a socket (ADR-0129); production uses the peer's inbox.
///
/// It does not wait for the handler's work: it hands each frame to the inbound queue and takes the next; processing is [`dispatch_task`]'s, in arrival order.
/// Two phases: it feeds the lanes until the link ends, the disposition gives up (`Unanswered` · a lane consumer gone) or `stop` resolves, which drops both lane senders; then it keeps taking items and discards them until the inbox ends, so the transport's cleanup is never held by an undrained inbox. Returns how the link ended.
// ADR-0206
#[allow(clippy::too_many_arguments)]
async fn read_task<S, F>(
    mut incoming: S,
    inbound_tx: mpsc::Sender<Inbound>,
    bypass_tx: mpsc::Sender<Inbound>,
    rx_bytes: RxBytes,
    frames: Arc<dyn FrameSink>,
    handler: Arc<dyn ConnectionHandler>,
    conn_id: ConnId,
    stop: F,
) -> LinkEnd
where
    S: Stream<Item = Incoming<DaemonWire>> + Unpin + Send,
    F: Future<Output = ()> + Send,
{
    let mut end = LinkEnd::default();
    {
        let feed = feed_lanes(
            &mut incoming,
            inbound_tx,
            bypass_tx,
            rx_bytes,
            frames,
            handler,
            conn_id,
            &mut end,
        );
        tokio::select! {
            () = feed => {}
            () = stop => tracing::debug!(conn = conn_id, "read_task: connection cleanup — no more pushes to the queue"),
        }
    }
    while !end.over {
        match incoming.next().await {
            Some(Incoming::Message(_)) => {}
            Some(Incoming::Event(event)) => {
                end.observe(event, conn_id);
            }
            None => end.over = true,
        }
    }
    tracing::debug!(conn = conn_id, "read_task loop ended");
    end
}

/// The feeding phase of [`read_task`] — returns where feeding stops; both lane senders drop with it.
#[allow(clippy::too_many_arguments)]
async fn feed_lanes<S>(
    incoming: &mut S,
    inbound_tx: mpsc::Sender<Inbound>,
    bypass_tx: mpsc::Sender<Inbound>,
    rx_bytes: RxBytes,
    frames: Arc<dyn FrameSink>,
    handler: Arc<dyn ConnectionHandler>,
    conn_id: ConnId,
    end: &mut LinkEnd,
) where
    S: Stream<Item = Incoming<DaemonWire>> + Unpin + Send,
{
    while let Some(item) = incoming.next().await {
        let queued = match item {
            Incoming::Message(arrival) => Inbound::from(arrival.msg),
            Incoming::Event(event) => {
                if end.observe(event, conn_id) {
                    tracing::debug!(conn = conn_id, cause = ?end.cause, "connection ended — read_task closes the queue");
                    return;
                }
                continue;
            }
        };
        // The byte budget can be hit before the entries — the decision uses the fill level before adding (an empty lane accepts one frame regardless of size); basis: [`CONN_RX_MAX_BYTES`].
        let over_budget = rx_bytes.load(Ordering::Acquire) >= CONN_RX_MAX_BYTES;
        // `try_send`: waiting here would leave the frames behind it in the inbox (basis: [`CONN_RX_CAP`]). With room this is all (normal path cost 0); only without room does it go below.
        let full = if over_budget {
            queued
        } else {
            let len = queued.payload_len();
            match inbound_tx.try_send(queued) {
                Ok(()) => {
                    rx_bytes.fetch_add(len, Ordering::AcqRel);
                    continue;
                }
                Err(mpsc::error::TrySendError::Closed(_)) => {
                    tracing::debug!(
                        conn = conn_id,
                        "inbound queue has no consumer — read_task ended"
                    );
                    return;
                }
                // Gets back the frame that could not be put in — only then can a disposition be chosen without silent loss.
                Err(mpsc::error::TrySendError::Full(item)) => item,
            }
        };

        // The lane is full — the disposition of this frame is the upper layer's (only it knows the vocabulary). Binary is not asked: a client→daemon binary is itself a violation the upper layer closes anyway, and lane order is not at stake.
        let disposition = match &full {
            Inbound::Text(text) => handler.on_inbound_saturated(conn_id, text.as_str(), &frames),
            Inbound::Binary(_) => Saturated::Bypass,
        };
        match disposition {
            Saturated::Refused => {
                // The upper layer sent a reply — discard and keep reading ("keep reading" is this branch's reason).
                tracing::debug!(
                    conn = conn_id,
                    "receiving queue saturated — the upper layer refused (keep reading)"
                );
            }
            // It could not even send a refusal — the basis of "discard and keep reading" is gone, so end (basis: [`Saturated::Unanswered`]). If only feeding ends, the arrival-order lane finishes what it already accepted and ends by itself, and the connection task catches that end.
            Saturated::Unanswered => {
                tracing::warn!(
                    conn = conn_id,
                    "receiving queue saturated — could not even queue the refusal reply (the send queue is saturated too) — ending the read"
                );
                return;
            }
            // Waiting is fine here — basis [`CONN_BYPASS_CAP`].
            Saturated::Bypass => {
                if bypass_tx.send(full).await.is_err() {
                    tracing::debug!(
                        conn = conn_id,
                        "priority lane has no consumer — read_task ended"
                    );
                    return;
                }
            }
        }
    }
    end.over = true;
}

/// The single consumer of the inbound queue — raises one connection's frames to the handler in arrival order, without overlapping.
///
/// This serialization is the real thing behind the upper layer's order invariants: inputs to the same agent · input lease acquisition and the input after it · subscribe/unsubscribe on one connection · command registry registration and its diff all hang on arrival order, kept only by this task being one and awaiting one at a time
/// (contract: `conn_port::ConnectionHandler`). A second consumer or scattering with spawn breaks all four at once — peeling a long-running command off the lane is the upper layer's job.
async fn dispatch_task(
    mut inbound_rx: mpsc::Receiver<Inbound>,
    rx_bytes: RxBytes,
    frames: Arc<dyn FrameSink>,
    handler: Arc<dyn ConnectionHandler>,
    conn_id: ConnId,
) {
    while let Some(item) = inbound_rx.recv().await {
        let len = item.payload_len();
        let flow = match item {
            Inbound::Text(text) => handler.on_text(conn_id, text.as_str(), &frames).await,
            // The payload is lent — the rejection path is the only consumer, so no copy is made.
            Inbound::Binary(payload) => handler.on_binary(conn_id, &payload, &frames).await,
        };
        // Subtract after the handler returns — counting the one being processed as 0 doubles the budget ([`RxBytes`]). A handler panic never reaches this line, but the production daemon ends the process in the hook (ADR-0292), and in a hookless test build this task dies and the connection is cleaned up wholesale, so no leaked budget remains.
        rx_bytes.fetch_sub(len, Ordering::AcqRel);
        if flow == ConnFlow::Close {
            tracing::debug!(
                conn = conn_id,
                "handler returned Close — dispatch_task ended"
            );
            break;
        }
    }
    tracing::debug!(conn = conn_id, "dispatch_task loop ended");
}

/// The consumer of frames that skipped the arrival-order lane ([`Saturated::Bypass`]).
///
/// It must be a separate task from [`dispatch_task`], not merged by `select!`: this lane exists precisely when that side's consumer is blocked, and a blocked task cannot poll anything, so a priority arrangement inside one task would never run this frame.
/// So the two lanes run overlapped — the only exception to the port contract's "they do not overlap", chosen by the upper layer itself (`on_inbound_saturated`); what runs here and what is blocked there may touch the same target, so choosing only frames allowed to do so is also the upper layer's responsibility.
/// `Close` is routed as an in-queue marker: this task is not a select target of the connection task (else once feeding stops and this lane closes, teardown would start and the arrival-order lane would lose the chance to drain). The intent to end goes on the write lane as `Frame::Close`, closing after earlier frames go out.
async fn bypass_task(
    mut bypass_rx: mpsc::Receiver<Inbound>,
    frames: Arc<dyn FrameSink>,
    handler: Arc<dyn ConnectionHandler>,
    conn_id: ConnId,
) {
    while let Some(item) = bypass_rx.recv().await {
        let flow = match item {
            Inbound::Text(text) => handler.on_text(conn_id, text.as_str(), &frames).await,
            Inbound::Binary(payload) => handler.on_binary(conn_id, &payload, &frames).await,
        };
        if flow == ConnFlow::Close {
            tracing::debug!(
                conn = conn_id,
                "Close on the priority lane — handing the end to the write lane"
            );
            let _ = frames.try_send(Frame::Close("bypass lane requested close".into()));
            break;
        }
    }
    tracing::debug!(conn = conn_id, "bypass_task loop ended");
}

/// Reaps the priority lane — not with `abort()`, but by draining what is left and waiting for it to end by itself.
///
/// Abort was a defect: a frame in the priority lane was accepted as "must not starve" (`on_inbound_saturated` → [`Saturated::Bypass`]), but when the arrival-order lane ended first (a peer sends one command and closes right away) the task was aborted and an accepted, never-polled frame vanished — collapsing this lane's reason
/// (the arrival-order lane has no such hole: when its sender drops it drains what is left).
/// Caller's obligation: stop the read task's feeding first — this lane's only sender is there, and undropped `recv()` never gets `None` and the wait uses the whole grace period.
/// Past the grace period it falls back to abort, so there is no downside. Lending the handle to `timeout` instead of passing it is intended: passing would drop the handle on timeout and detach rather than abort the task.
async fn finish_bypass_lane(mut handle: tokio::task::JoinHandle<()>, conn_id: ConnId) {
    match tokio::time::timeout(BYPASS_DRAIN_GRACE, &mut handle).await {
        Ok(_) => {}
        Err(_) => {
            tracing::warn!(
                conn = conn_id,
                grace_ms = BYPASS_DRAIN_GRACE.as_millis() as u64,
                "the priority lane was not drained within the grace period — abort (anything held is lost)"
            );
            handle.abort();
        }
    }
}

#[cfg(test)]
mod tests;
