//! The daemon's accepting side on the transport — the `conn_port` contract (`FrameSink` · `FrameFanout` ·
//! `ConnectionHandler` calls) implemented over accepted transport peers (TRD S21 trd-3 §6-3 U4 · D11 ②′).
//!
//! The transport owns the socket: WS upgrade + Origin · Host check ([`accept_policy`]), the auth deadline, the single
//! writer, keepalive (active ping · idle decision) and the close frame. This file owns what the net layer's
//! `handle_connection` did above that, carried over as code: per connection one read task (it reads that peer's
//! lossless inbox instead of the socket), the arrival-order lane + byte budget + saturation disposition, the single
//! `dispatch_task`, the overlapping `bypass_task`, and the teardown order. Constants are the net layer's.
//!
//! ★Connection lifetime (net parity)★ — net selected on two ends: the dispatch task finishing (read abort → priority lane
//! drained → write abort) and the write task finishing (read abort → priority lane drained → dispatch abort). Here:
//! - dispatch finished (`ConnFlow::Close` · the lane closed after the read side stopped) → stop feeding · drain the
//!   priority lane · close the peer after flushing what is queued (`Peer::close_after_flush`).
//! - the link ended on the write side (write failure · write deadline · idle · our own close) or a send through the
//!   [`FrameSink`] failed → stop feeding · drain the priority lane · abort dispatch.
//! - the link ended on the read side (the peer closed · a read error) → the lanes close, dispatch drains what it
//!   holds; net's writer then ended at its next write — a failed send, or at the latest its next ping — so dispatch is
//!   aborted at the first failed send or after one ping interval.
//!
//! In every branch `on_disconnect` runs before the fan-out list drops the connection, and the read task keeps draining
//! the inbox to its end (`Closed` · `None`) after it stops feeding the lanes (transport receiver invariant 5).
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
    Accepted, Address, Clock, CloseCode, DialFailure, Dialer, DisconnectCause, Generation, Inbox,
    Incoming, LinkError, LinkRx, LinkTx, Listener, Peer, PeerId, Policy, QueueFullPolicy, Registry,
    SendError, SystemClock, TransportEvent,
};
use futures_util::future::BoxFuture;
use futures_util::{Stream, StreamExt};
use tokio::sync::{mpsc, oneshot, Notify};

use super::auth::AuthCheck;
use super::handshake::DaemonHandshake;
use super::wire::{DaemonWire, RawIn, RawOut};
use crate::conn_port::{
    ConnFlow, ConnId, ConnectionHandler, ConnectionHandlerFactory, Frame, FrameError, FrameFanout,
    FrameSink, Saturated,
};

/// Per-connection outbound queue capacity. ReplayBuffer.max_events(4096) + control_slack(512) = 4608.
/// Leaves control headroom even when the whole replay fits (consistent with the output_core.rs invariant).
const CONN_TX_CAP: usize = 4608;

/// **Number of entries** in the per-connection **inbound** arrival-order lane — headroom that lets the read loop read the next frame without waiting for the handler's work.
/// The memory axis is carried not by this value but by [`CONN_RX_MAX_BYTES`] (that doc is the source of truth for the accounting).
///
/// ★This queue is a **burst absorber**, not a backlog store★: what the consumer (`dispatch_task`) holds at a time
///   is only **one** frame whose arrival order carries meaning, and long work is peeled off this lane by the upper layer
///   (the 「도착 순서」 (arrival order) contract of `frame_port::ConnectionHandler`). So unlike [`CONN_TX_CAP`], which must hold
///   the entire replay, no large value is needed — do not compare the two values on the same axis.
/// ★★Even when full, **the read loop does not stop** — it asks the upper layer how to dispose of that one frame and keeps reading★★
///   ([`ConnectionHandler::on_inbound_saturated`]). ★The 「가득 차면 읽기 루프가
///   기다린다」 (when full, the read loop waits) that was once written here **was a wrong design**★: the inbox is a stream, so while waiting **the frames behind it
///   are not even taken out**. Then a frame the upper layer chose as 「굶으면 안 된다」 (must not starve) ([`Saturated::Bypass`])
///   starves behind the blocked lane, and the very symptom this arrangement was meant to eliminate comes back by another path.
///   ㆍ**Even so it does not drop silently**: asking the upper layer for the disposition and getting [`Saturated::Refused`] means
///     **the upper layer has already sent a reply** (obligation 2 of the port contract). The disposition for when even that reply could not be sent is
///     held separately by [`Saturated::Unanswered`] — only there is the connection ended.
///   ㆍ**Otherwise it does not disconnect either**: the outbound queue ([`ConnFrameSink`]) disconnects on saturation
///     because its caller is a synchronous path that **cannot produce a reply** (the source of truth for the asymmetry =
///     `frame_port::FrameSink`). Here the upper layer can state the refusal in words, so there is no reason to kill the connection.
/// ★So what this value measures★ = 「소비자가 막힌 동안 **손실 없이** 받아 둘 프레임 **개수**」 (the **number** of frames to hold **without loss** while the consumer is blocked). Whichever of the two
///   is hit first is the saturation — the entries all fill, or the byte budget all fills.
/// ★When the consumer is healthy this value is not even seen★: what remains in the lane is only short frames with no await point
///   (long ones are peeled off by the upper layer), so in normal operation the 64 entries never fill. Being full is a signal that **the consumer
///   is blocked**, and the right answer in that state is not to wait but to let through what should be skipped.
// ADR-0206
const CONN_RX_CAP: usize = 64;

/// Budget of **payload bytes** the arrival-order lane may hold — decides saturation **together with** [`CONN_RX_CAP`].
///
/// ★Why the entry count alone is not enough★: each entry holds one whole frame, so **each entry differs in size**. Counting only
///   entries makes this lane's real ceiling `CONN_RX_CAP × (한 프레임의 상한)` = 64 × [`MAX_MESSAGE_BYTES`].
/// ★The decision uses **the fill level before adding**★: if the bytes already held are at or above this value, it counts as saturated. So a frame arriving at an empty lane
///   **is always accepted, whatever its size** — a legitimate command is never refused merely for being large.
///   The price is that the ceiling becomes 「예산 + 프레임 하나」 (budget + one frame) (= 16 MiB + [`MAX_MESSAGE_BYTES`]).
///   The transport's own byte budget below this lane (read-ahead · inbox) is separate and counted by the transport.
/// ★Basis of the value★: what stands in this lane is short control frames, and a large one is an occasional single frame — 16 MiB leaves room even if that
///   「어쩌다 한 장」 (an occasional single frame) overlaps several times, and a normal burst never sees this value.
const CONN_RX_MAX_BYTES: usize = 16 * 1024 * 1024;

/// Capacity of the lane where a frame that skipped the arrival-order lane ([`Saturated::Bypass`]) stands.
///
/// ★Why it is small★: what enters this lane is only 「막힌 줄을 풀려는 것」 (things that try to unblock the blocked lane), so in normal operation it **uses not a single entry**.
///   8 entries being full means eight were sent and not one finished — that path itself is blocked,
///   and accepting more then only lines up the same thing eight more times.
/// ★Here it waits (unlike the lane above)★: if reading stops when this lane is full, what starves is **the frames bound for the arrival-order lane**,
///   and those were about to be refused anyway. That is, waiting here does not starve anything that must be protected. The wait
///   stops only this connection's inbox, and the transport turns that into backpressure on this peer's socket alone.
/// ★The byte budget is **not applied** here★: refusing by budget a frame the upper layer chose as 「굶으면 안 된다」 (must not starve)
///   removes the reason this lane exists. So this lane's ceiling is the entry count as is, and its value is
///   8 × [`MAX_MESSAGE_BYTES`]. The property that this lane is empty in normal operation makes that ceiling effectively 0 — it is an
///   observation, not a guarantee, so it is written as is.
const CONN_BYPASS_CAP: usize = 8;

/// Grace period in connection cleanup, waiting until the priority lane **empties what it has accepted** ([`finish_bypass_lane`]).
///
/// ★This value does **not** bear correctness★ — correctness is borne by 「기다린다」 (waiting) itself, and this value is only the upper bound that keeps that wait
///   from becoming infinite. Past the grace period it falls back to abort, so even if this constant
///   is wrong it **cannot be worse than aborting at once**.
/// ★Basis of the size★: one item in this lane is an action in the upper layer that 「막힌 것을 끝내는」 (finishes something blocked), and such actions have a section
///   that synchronously waits for the target to die (today the upper bound of that section is the upper layer's 5-second join). With 10 seconds the one in progress
///   + the next one finish, and if more is backed up than that, that path itself is blocked, and waiting longer only
///   lines up the same thing (the same judgment as 「8칸이 차 있다는 것은」 (8 entries being full means) in [`CONN_BYPASS_CAP`]).
/// ★What is held during this grace period★ = only **the cleanup of that one connection**. Neither the accept loop nor
///   other connections are caught by this wait.
const BYPASS_DRAIN_GRACE: Duration = Duration::from_secs(10);

/// How long a new connection may stay open without its first (auth) frame before it is closed — the transport's auth
/// deadline for accepted peers (`Policy::connect_timeout`, measured from the end of the WS upgrade). 15 s — the open-source
/// defaults read are 3–60 s, mostly 5–15 s (SignalR `HandshakeTimeout` 15 s, EMQX `idle_timeout` 15 s —
/// `docs/research/first-auth-frame-timeout-2026-11-06.md`). A short window turns a briefly stalled shell into a refusal.
// ADR-0347
const AUTH_TIMEOUT: Duration = Duration::from_secs(15);

const DEFAULT_PING_INTERVAL: Duration = Duration::from_secs(20);
/// 2.5x ping_interval (the connection drops only after several Pings are missed — prevents false positives from a transient delay).
const DEFAULT_IDLE_TIMEOUT: Duration = Duration::from_secs(50);

/// The transport requires a write deadline below the ping interval (`Policy::check`); the net layer had none. The deadline
/// is set as late as the transport allows — one millisecond under the ping interval — so this cut fires as rarely as it can.
const WRITE_DEADLINE_MARGIN: Duration = Duration::from_millis(1);

/// The byte cap of one WS message and one frame, both directions — the shell dialer's value (`DAEMON_MESSAGE_MAX_BYTES`).
/// The net layer kept tungstenite's defaults (64 MiB message · 16 MiB frame); a client that does not fragment, like the
/// shell, was already bounded by the 16 MiB frame cap.
const MAX_MESSAGE_BYTES: usize = 16 << 20;

/// Upgrade permits — links from TCP accept until they go live (upgrade · ready queue · auth wait). The net layer had no cap;
/// this is set far above any legitimate count of clients so it never refuses one.
const MAX_PENDING_UPGRADES: usize = 1024;

/// Refusal reports the listener may hold before the accept loop logs them — past it they are counted, not kept.
const REFUSAL_QUEUE: usize = 64;

/// Inbox length per accepted peer (TRD trd-3 §7 「수신 큐」 — daemon accepted peer 16 entries). The read task drains it at once
/// except while it waits on a full priority lane; then the transport stops reading that peer's socket.
const ACCEPTED_INBOX: usize = 16;

/// Allowed Origin allowlist. No Origin (native/harness) is allowed — the token is the main defense.
/// ★Unmeasured — known unverified★: the four strings below are **design values**. The Origins actually sent by Tauri WebView2 · mobile
///   have never been measured and confirmed. A mismatch = 403. When growing the client surface (packaging variants · mobile tunnels, etc.), do not treat this list
///   as verified — a 403 may be this list's fault, not a client bug.
const ALLOWED_ORIGINS: &[&str] = &[
    "http://localhost:1420",
    "http://127.0.0.1:1420",
    "tauri://localhost",
    "https://tauri.localhost",
];

/// Host names a WS upgrade may carry (the port must be the bound one). The net layer checked no Host; the transport accepts
/// only listed names (an upgrade without Host passes), so the list is the names a loopback client dials by.
const ALLOWED_HOST_NAMES: &[&str] = &["127.0.0.1", "localhost"];

/// WS application-level keepalive settings — the transport's ping interval and idle timeout for accepted peers.
///
/// ★Test injection★: hard-coding constants would make tests take tens of seconds, so short values (e.g. 200ms/600ms)
/// are made configurable for injection. The production path stays `default()` (20s/50s) as is.
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
        // The net layer had no upgrade deadline; the transport requires one — the auth window's length.
        upgrade_timeout: AUTH_TIMEOUT,
        max_pending_upgrades: MAX_PENDING_UPGRADES,
        max_message_bytes: MAX_MESSAGE_BYTES,
        refusal_queue: REFUSAL_QUEUE,
    }
}

/// The transport policy of one accepted peer. The values the net layer had are kept (auth window · keepalive · outbound
/// queue); the rest are the transport's defaults. ★Outbound saturation is not a transport disconnect★ — the per-connection
/// sink closes on it ([`ConnFrameSink`]) and the fan-out skips it ([`ConnRegistry`]), the net layer's asymmetry
/// (`frame_port::FrameFanout`).
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
        "WS 업그레이드 실패(또는 Origin 거부)"
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
/// ★Connection ids start at 1 and only go up★ — unregistering never returns an id. The command bus relies on that
///   (`command_delivery::Pending::origin` · `CommandBus::open_caller`).
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
    /// ★Do not send while holding the map locked (ADR-0006 lock order)★: take a snapshot, release the lock, and send with that copy.
    /// ★One saturated connection is skipped, not closed★ (`frame_port::FrameFanout` duties 2 · 3) — the transport also counts
    ///   it on that peer's inbox (`TransportEvent::Dropped`).
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
                tracing::warn!(conn = id, "전-연결 팬아웃 try_send 실패(느린 소비자): {e}");
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
/// ★Saturation closes the connection★ (`frame_port::FrameSink::try_send`) — the peer is closed at once and the connection
///   task is woken through `write_end` (the net layer's out-of-band close signal). ★`send` does not wait for room★ — the
///   transport has no waiting send (a waiting sender on the read side would form a cycle with the supervisor — TRD trd-3
///   §3-0 ①), so `send` behaves as `try_send`. A send on a link that is no longer live also wakes `write_end` — the net
///   layer's writer ended on a failed write.
/// ★`Frame::Close` = drain then close★ — frames put in earlier go out first (`Peer::close_after_flush`); the reason is
///   logged and not sent.
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
                tracing::info!(%reason, "close 신호 — 보낸 것을 비운 뒤 닫는다");
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
    policy: Policy,
    /// The read-end wait ([`Server::run_connection`]) — the net layer's writer noticed a dead link at its next ping at the latest.
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
    /// `expected_token` is the token from daemon.json and `expected_protocol_version` the version to accept — both injected,
    /// so this layer reads neither. `conns` must be the registry the upper layer's fan-out sends through
    /// (`DaemonWiring`). `clock` = [`Listening::clock`].
    pub(crate) fn new(
        clock: Arc<dyn Clock>,
        conns: ConnRegistry,
        handlers: Arc<dyn ConnectionHandlerFactory>,
        expected_token: &str,
        expected_protocol_version: u32,
        keepalive: KeepaliveConfig,
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
                        tracing::debug!(remote = ?accepted.info().remote, "연결 수락 — WS 핸들러로 넘김");
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

    /// Puts one accepted link on the registry and returns the connection's whole life — handshake, operation, cleanup.
    fn serve(self: &Arc<Self>, accepted: Accepted) -> impl Future<Output = ()> + Send + 'static {
        let remote = accepted.info().remote.clone().unwrap_or_else(|| "?".into());
        let label = PeerId::new(format!(
            "accepted-{}",
            self.labels.fetch_add(1, Ordering::Relaxed)
        ));
        let handshake = Arc::new(DaemonHandshake::new(
            self.auth.clone(),
            self.handlers.clone(),
            remote.clone(),
        ));
        let (peer, inbox) = self.registry.attach(label, accepted, handshake);
        let this = Arc::clone(self);
        async move { this.run_connection(peer, inbox, remote).await }
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
        tracing::info!(peer = %remote, conn = conn_id, "연결 인증 완료 — 등록");

        let write_end = Arc::new(Notify::new());
        let frames: Arc<dyn FrameSink> = Arc::new(ConnFrameSink::new(outlet, write_end.clone()));
        let handler = self.handlers.handler_for(conn_id);

        // In order, this must come before spawning the tasks so that command dispatch cannot get ahead of the greeting.
        handler.on_connect(conn_id, &frames).await;

        // ★Inbound queue = the boundary between the read layer and the processing layer★: reading only takes from the inbox and puts here, and handler calls
        //   are borne by [`dispatch_task`] in arrival order. The basis of capacity and saturation disposition is [`CONN_RX_CAP`].
        let (inbound_tx, inbound_rx) = mpsc::channel::<Inbound>(CONN_RX_CAP);
        // ★The fill level of the byte budget that decides saturation **together with** the entry count★ — source of truth for the basis = [`CONN_RX_MAX_BYTES`].
        let rx_bytes: RxBytes = Arc::new(AtomicUsize::new(0));
        // ★The second lane for frames that must not starve behind a blocked lane★ — why it must have separate capacity · a separate consumer
        //   is held by [`CONN_BYPASS_CAP`] and [`bypass_task`] respectively.
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

        // ★Explicitly abort the survivor★ — if a JoinHandle is simply dropped, the task is detached and keeps running. So select on
        //   &mut so the handle is not consumed, and abort the losing side.
        // ★The priority lane (bypass_task) is not a select target★ — its ending only means feeding stopped (its own sender is in read),
        //   and is not a reason to cut the connection.
        //   ★★The priority lane is **not aborted — it is drained**★★: the frames in that lane are ones the upper layer chose as 「굶으면 안 된다」 (must not starve) and
        //   **accepted**. So stop feeding first to release **that lane's only sender**, and [`finish_bypass_lane`] waits for that draining.
        // ★A read end does not abort at once★: the last command of a client that 「명령 한 장 보내고 곧바로 소켓을 닫는」 (sends one command and closes the socket right away)
        //   still runs — dispatch drains the lane, and only a failed send or one ping interval ends the wait (the net layer's writer).
        let mut link_ended = false;
        tokio::select! {
            _ = &mut dispatch_handle => {
                tracing::debug!(conn = conn_id, "dispatch_task 종료 → read 중단 · bypass 비움 · 비운 뒤 close + cleanup");
                let _ = stop_feeding.send(());
                finish_bypass_lane(bypass_handle, conn_id).await;
                let _closing = peer.close_after_flush(CloseCode::GOING_AWAY, "");
            }
            () = write_end.notified() => {
                tracing::debug!(conn = conn_id, "송신 실패 → read 중단 · bypass 비움 · dispatch abort + cleanup");
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
                    tracing::debug!(conn = conn_id, cause = ?end.cause, "읽기 끝 → dispatch 가 남은 것을 비운다");
                    tokio::select! {
                        _ = &mut dispatch_handle => {
                            finish_bypass_lane(bypass_handle, conn_id).await;
                        }
                        () = write_end.notified() => {
                            finish_bypass_lane(bypass_handle, conn_id).await;
                            dispatch_handle.abort();
                        }
                        () = tokio::time::sleep(self.ping_interval) => {
                            tracing::debug!(conn = conn_id, "읽기 끝 뒤 한 ping 주기 — dispatch abort");
                            finish_bypass_lane(bypass_handle, conn_id).await;
                            dispatch_handle.abort();
                        }
                    }
                } else {
                    tracing::debug!(conn = conn_id, cause = ?end.cause, "쓰기 끝 → bypass 비움 · dispatch abort + cleanup");
                    finish_bypass_lane(bypass_handle, conn_id).await;
                    dispatch_handle.abort();
                }
            }
        }

        // ── cleanup (leak prevention — review required) ──────────────────────────────────────────
        handler.on_disconnect(conn_id);
        self.conns.unregister(conn_id);
        tracing::info!(peer = %remote, conn = conn_id, "연결 종료 — cleanup 완료");
        // The read task keeps draining the inbox to its end — every branch above has closed the peer or seen it end.
        if !link_ended {
            let _ = read_handle.await;
        }
    }
}

/// Drains the inbox until the handshake ends: `Some` = the link went live on that generation. `None` = it ended before
/// auth (refused · deadline · the peer left) — the inbox is drained to its end.
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
        DisconnectCause::PeerClosed => tracing::warn!(%peer, "auth 전에 연결 종료"),
        DisconnectCause::ReadFailed => tracing::warn!(%peer, "첫 frame 수신 오류 — close"),
        // Our refusal — `AuthCheck` logged why — or the daemon shutting down.
        DisconnectCause::Explicit => tracing::debug!(%peer, "handshake ended by the daemon"),
        other => tracing::warn!(%peer, cause = ?other, "handshake ended — close"),
    }
}

// ── read_task + dispatch_task (two layers split by the inbound queue) ─────────────────────────────

/// The unit the read loop hands to the inbound queue.
///
/// ★Unlike the outgoing [`Frame`], there is no Close★: an outgoing Close is a marker the upper layer puts in the queue, but on the incoming
///   side the close decision is made not by frame **content** but by the handler's answer (`ConnFlow::Close`) — so this
///   vocabulary has no such shape.
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

/// The payload bytes the arrival-order lane currently holds — the producer (`read_task`) adds and the consumer
/// (`dispatch_task`) subtracts. One per connection.
///
/// ★That the subtraction happens **after the handler call** is intended★: subtracting right on taking it out would count a frame that is 「꺼냈지만 아직 처리 중인」 (taken out but still being processed) as
///   0, so the budget would actually double (the same-axis mistake = the in-flight accounting of `input_queue`).
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

    /// The input ended (the peer closed · a read error) — the net layer's read task ending. Every other end is the write
    /// side's (write failure · write deadline · idle · our own close).
    fn read_side(&self) -> bool {
        matches!(
            self.cause,
            Some(DisconnectCause::PeerClosed | DisconnectCause::ReadFailed)
        )
    }
}

/// ★Why the source is generic★: to have an isolation harness that runs this loop on a synthetic item sequence without a socket
/// (ADR-0129). The production path is the peer's inbox.
///
/// ★It does **not wait for the handler's work**★: it hands each frame to the inbound queue and immediately takes the next one.
/// Processing is borne by [`dispatch_task`] in arrival order.
///
/// ★Two phases★ — it feeds the lanes until the link ends, the disposition gives up (`Unanswered` · a lane consumer is gone) or `stop`
/// resolves, which drops both lane senders; then it keeps taking items and **discards** them until the inbox ends, so the transport's
/// cleanup is never held by an undrained inbox. Returns how the link ended.
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
            () = stop => tracing::debug!(conn = conn_id, "read_task: 연결 정리 — 줄에 더 넣지 않는다"),
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
    tracing::debug!(conn = conn_id, "read_task 루프 종료");
    end
}

/// The feeding phase of [`read_task`] — returns where the net layer's read loop broke out; both lane senders drop with it.
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
                    tracing::debug!(conn = conn_id, cause = ?end.cause, "연결 끝 — read_task 가 줄을 닫는다");
                    return;
                }
                continue;
            }
        };
        // ★The **byte budget can be hit before the entries**★ — the decision uses the fill level before adding (an empty lane
        //   accepts one frame regardless of size). Source of truth for the basis = [`CONN_RX_MAX_BYTES`].
        let over_budget = rx_bytes.load(Ordering::Acquire) >= CONN_RX_MAX_BYTES;
        // ★It is `try_send` — waiting here would mean **not even taking the frames behind it out of the inbox**★ (source of truth for the basis =
        //   [`CONN_RX_CAP`]). If there is room, this is all (normal path cost 0), and only when there is none does it go below.
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
                    tracing::debug!(conn = conn_id, "수신 큐 소비자 없음 — read_task 종료");
                    return;
                }
                // **Gets back** the frame that could not be put in — only with this can a disposition be chosen without silent loss.
                Err(mpsc::error::TrySendError::Full(item)) => item,
            }
        };

        // ★The lane is full — the disposition of this one frame is decided by the upper layer★ (only the upper layer knows the vocabulary).
        //   Binary is not asked: in this protocol a client→daemon binary is itself a violation and the upper layer closes
        //   the connection anyway, and lane order is not at stake in that decision.
        let disposition = match &full {
            Inbound::Text(text) => handler.on_inbound_saturated(conn_id, text.as_str(), &frames),
            Inbound::Binary(_) => Saturated::Bypass,
        };
        match disposition {
            Saturated::Refused => {
                // The upper layer sent a reply — discard it and keep reading. 「계속 읽는다」 (keep reading) is the reason this branch exists.
                tracing::debug!(
                    conn = conn_id,
                    "수신 줄 포화 — 위층이 거절했다(계속 읽는다)"
                );
            }
            // ★It could not even send a refusal — the basis of 「버리고 계속 읽는다」 (discard and keep reading) is gone, so end★ (source of truth for the basis =
            //   [`Saturated::Unanswered`]). If only feeding ends, the arrival-order lane finishes processing what it already accepted and
            //   ends by itself, and the connection task catches that end and cleans up.
            Saturated::Unanswered => {
                tracing::warn!(
                    conn = conn_id,
                    "수신 줄 포화 — 거절 답장조차 큐에 못 넣었다(송신 큐도 포화) — 읽기를 끝낸다"
                );
                return;
            }
            // ★Waiting is fine here★ — the basis is [`CONN_BYPASS_CAP`].
            Saturated::Bypass => {
                if bypass_tx.send(full).await.is_err() {
                    tracing::debug!(conn = conn_id, "우선 줄 소비자 없음 — read_task 종료");
                    return;
                }
            }
        }
    }
    end.over = true;
}

/// The **single** consumer of the inbound queue — raises one connection's frames to the handler in arrival order, without overlapping.
///
/// ★This serialization is the **real thing** behind the upper layer's order invariants★: 「같은 에이전트로 가는 입력끼리」 (inputs bound for the same agent) · 「입력 lease
/// 획득과 그 뒤 입력」 (input lease acquisition and the input after it) · 「같은 연결의 구독/해지」 (subscribe/unsubscribe on the same connection) · 「명령 명부 등록과 그 차분」 (command registry registration and its diff) all
/// hang on arrival order, and what keeps it is only the property that this task is one and awaits only one at a time
/// (source of truth for the contract = `frame_port::ConnectionHandler`). **Starting a second consumer or scattering here with spawn
/// breaks all four at once** — the decision to peel a long-running command off the lane is the upper layer's job.
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
            // The payload is **lent** — the rejection path is the only consumer, so no copy is made.
            Inbound::Binary(payload) => handler.on_binary(conn_id, &payload, &frames).await,
        };
        // ★Subtract **after** the handler returns★ — counting the one being processed as 0 doubles the budget
        //   ([`RxBytes`]). If the handler panics it never reaches this line, but the production daemon ends the whole process in the hook
        //   (ADR-0292), and in a hookless test build this task itself dies and the connection is cleaned up wholesale, so there is nowhere
        //   for a leaked budget to remain.
        rx_bytes.fetch_sub(len, Ordering::AcqRel);
        if flow == ConnFlow::Close {
            tracing::debug!(conn = conn_id, "핸들러가 Close — dispatch_task 종료");
            break;
        }
    }
    tracing::debug!(conn = conn_id, "dispatch_task 루프 종료");
}

/// The consumer of frames that skipped the arrival-order lane ([`Saturated::Bypass`]).
///
/// ★★It must be a **separate task from** [`dispatch_task`] — it cannot be merged by `select!` in the same task★★: the situation in which this lane
/// exists is precisely one where **that side's consumer is blocked**. A blocked task cannot poll anything, so
/// with a priority arrangement inside one task, this frame would never run.
///
/// ★So the two lanes **run overlapped**★ — the only exception to the port contract's 「겹치지 않는다」 (they do not overlap), and the one who
/// chose that exception is the upper layer itself (`on_inbound_saturated`). It means that what runs here and what is blocked there may
/// touch the same target, so the responsibility of choosing only frames that are allowed to do so also lies with the upper layer.
///
/// ★Route `Close` as an in-queue marker★: this task is **not** a select target of the connection task (if it were,
/// the moment feeding stops and this lane closes, teardown would start, and the arrival-order lane would lose the chance to drain what is left).
/// So the intent to end is sent on the write lane as `Frame::Close` — it closes after the frames put in earlier go out first.
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
                "우선 줄에서 Close — write 줄로 종료를 넘긴다"
            );
            let _ = frames.try_send(Frame::Close("bypass lane requested close".into()));
            break;
        }
    }
    tracing::debug!(conn = conn_id, "bypass_task 루프 종료");
}

/// **Reap** the priority lane — not with `abort()`, but by draining what is left and waiting for it to end by itself.
///
/// ★Why abort was a defect★: a frame that entered the priority lane is one the upper layer decided 「이건 굶으면 안 된다」 (this must not starve) and
///   **accepted** (`on_inbound_saturated` → [`Saturated::Bypass`]). But when the arrival-order lane ended first
///   (the common shape where a peer sends one command and closes right away), this task was aborted on the spot, and
///   **a frame that was accepted and never polled simply vanished.** This is the branch where the reason this lane was made collapses entirely —
///   the arrival-order lane does not have the same hole (when its sender is dropped it drains what is left and ends).
///
/// ★The caller's obligation = **stop the read task's feeding first**★: this lane's only sender is there, and if it is
///   not dropped, `recv()` never gets `None` and this wait uses up the whole grace period.
/// ★Past the grace period it falls back to abort★ — so this function has **no downside**: if it goes well it finishes
///   what was accepted, and if not it aborts. ★Lending the handle to `timeout` **instead of passing it
///   is intended**★: passing it would drop the handle on timeout, so the task would be **detached** rather than aborted.
async fn finish_bypass_lane(mut handle: tokio::task::JoinHandle<()>, conn_id: ConnId) {
    match tokio::time::timeout(BYPASS_DRAIN_GRACE, &mut handle).await {
        Ok(_) => {}
        Err(_) => {
            tracing::warn!(
                conn = conn_id,
                grace_ms = BYPASS_DRAIN_GRACE.as_millis() as u64,
                "우선 줄이 유예 안에 안 비워졌다 — abort(받아 둔 것이 남아 있으면 유실된다)"
            );
            handle.abort();
        }
    }
}

#[cfg(test)]
mod tests;
