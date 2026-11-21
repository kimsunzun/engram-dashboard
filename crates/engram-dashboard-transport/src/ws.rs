//! Real-socket WS adapter — behind feature `ws`: the loopback shell link (the other real adapter is `iroh.rs`).
//!
//! Owns the dialing side ([`WsDialer`]), the receiving side ([`WsListener`] with [`AcceptPolicy`] · [`AcceptRefusals`]) and the split of one open WS
//! stream into [`LinkTx`]/[`LinkRx`]. Both directions live here so the crate attaches to itself and measures what memory cannot (ADR-0177 decision 8).
//! The stream split is crate-internal, so [`WsListener`] is the only path from a real WS stream to an [`Accepted`] link (TRD 3 §3-5 · §5 17); a link
//! built from hand-written ends via the public [`Accepted::new`] bypasses it — code review, not the compiler, catches that.
//!
//! ## What the receiving side guards (TRD 3 §5 1 · 2 · 12 · 14 · 15 · 16 · 18)
//!
//! The consumer gives every policy value via [`AcceptPolicy`]; this file chose only `LISTEN_BACKLOG` and [`ACCEPT_ERROR_PAUSE`].
//! - Loopback only — a non-loopback bound address is refused before listen.
//! - TCP accept on one task, each upgrade on its own spawned task (a stalled upgrade blocks no accept). The upgrade deadline runs from TCP accept;
//!   an upgrade finishing past it is not accepted.
//! - The upgrade permit ([`crate::PermitPool`]) is taken at TCP accept and travels in [`Accepted`]; mid-upgrade, ready-queue and not-yet-registered links
//!   count together. All used: the new TCP is closed right after accept.
//! - Origin and Host headers are checked against the policy (403 if outside); one message/frame size cap applies to both directions.
//! - Every cut is reported to [`AcceptRefusals`] with its branch and a masked remote address ([`Listener::accept`]'s `Err` means only that the listener ended).
//!
//! ## Guarantees / left to the consumer
//!
//! Guaranteed: the [`Frame`]↔`Message` round trip · close code and message in both directions (so 「connected but rejected」 crosses as [`LinkRead::Closed`],
//! ADR-0180 decision 5) · Ping/Pong raised as [`Frame::Keepalive`] · the TCP ends of every link, typed (accepted: [`LinkInfo::socket_ends`];
//! dialed: [`LinkTx::socket_ends`]) — facts for a consumer's own check, never a decision here (ADR-0395). The translation table is unit-tested (`to_message`/`from_message` are pure); the socket
//! round trip is in `tests/`, and nothing measures the keepalive round trip. Left to the consumer: address syntax · reconnect/deadline/keepalive timing
//! ([`crate::Policy`]; ★the dialer applies no deadline of its own★ — only the upgrade deadline, a pre-supervisor phase, lives in [`AcceptPolicy`]) ·
//! auth and version negotiation ([`crate::Handshake`]) · the accept loop and registry ([`Listener::accept`] → [`crate::Registry::attach`]).
//!
//! ## Known limits (do not read as guarantees)
//!
//! - ★No HTTP response when upgrade permits are exhausted★ — TCP is closed after accept, so the dialer sees a cut during upgrade; anyone on the PC can fill
//!   the permits (loopback is shared by all OS users; TRD 3 §5 14 · R11).
//! - ★Refusal reports can be lost★ — a full [`AcceptRefusals`] queue ([`AcceptPolicy::refusal_queue`]) drops new reports and raises
//!   [`AcceptRefusals::dropped`]; waiting on the consumer would itself be a denial of service.
//! - ★A TCP accept error is not terminal★ — per-connection errors (`ConnectionRefused` · `ConnectionAborted` · `ConnectionReset`) take the next accept at once,
//!   others (e.g. `EMFILE`) pause [`ACCEPT_ERROR_PAUSE`]; same branching as axum 0.8.9's accept loop (from source, not measured). [`Listener::accept`]
//!   returns `Err` only when the accept task ended (a panic).
//! - ★TLS is off★ — default `tokio-tungstenite` features only; `wss://` fails at dial.
//! - ★An outgoing close **message** is silently truncated★ to min(123 bytes, size cap − 2) by [`LinkTx::close`] (RFC 6455 §5.5); no ellipsis, no event,
//!   so never put decision material in it. The code goes out unchanged; a size cap under 2 bytes sends an empty close (`close_reason_room`).
//!
//! ## Link properties (facts of the outside world)
//!
//! - ★A write stalls when the kernel send buffer fills★ — `SinkExt::send` flushes and tungstenite turns `WouldBlock` into `Pending` (tokio-tungstenite 0.26.2),
//!   so `write_deadline` cuts it against a non-reading peer (`tests/ws_write_deadline.rs`). ★One large write does not stall★ — measured on Windows 11
//!   loopback: a 16 MiB frame went in in 28 ms (also with 4 KiB socket buffers), repeated 1 MiB frames stall on the third. Interpretation (not observed):
//!   dynamic send buffering. Backpressure is triggered by accumulation, not frame size.
//! - ★A truncated write's frame can go out late★ (from source: futures-util 0.3.32 `sink/send.rs:37`, `split.rs:104-114`; tokio-tungstenite `lib.rs:340-347`) —
//!   the handed-over item stays in a `SplitSink` slot or the tungstenite write buffer and the next write to that end pushes it out first. After a cut
//!   by control or deadline the close can therefore **go out behind the truncated frame**, and the supervisor's loss count can be inflated
//!   ([`crate::Direction::Outbound`]). The in-memory harness stalls before hand-over and does not imitate this (`testing::MemTx`).
//! - ★tungstenite checks sizes only on receive★ (0.26.2: `max_frame_size` / `max_message_size`; `WebSocketContext::write` has none); an oversize frame is a
//!   receiver read error that cuts the link (measured: a 32 MiB frame under defaults). One consumer value ([`WsDialer::new`] ·
//!   [`AcceptPolicy::max_message_bytes`]) sets both caps, and ★[`LinkTx::send`] **returns an error** for an oversize frame — never a panic★: otherwise an
//!   innocent peer burns its reconnect budget, and a panic in the supervisor task ends a hooked process (ADR-0292) or freezes the state at `Live`. It
//!   assumes the peer uses the same value. ★No diagnostic survives★: the supervisor maps the error to `LinkLost(WriteFailed)` and
//!   [`crate::TransportEvent::Disconnected`] has no message field, so the consumer sees a `WriteFailed` disconnect with no reason.
//! - ★When the automatic Pong goes out★ (from tungstenite 0.26.2 source, not measured): it is flushed by both the reading loop (`protocol/mod.rs:452`) and the
//!   writing side; a full send buffer does not stop reading (`WouldBlock` is swallowed; `WakerProxy` re-wakes the reader), so congestion shows as a late
//!   Pong and the peer may cut us for silence. ★The only bound is `write_deadline`★ on every write (`peer.rs` invariant); `Policy::check`'s ordering is
//!   debug-only and cannot establish the peer's patience.
//! - ★The keepalive round trip is unmeasured on a real socket★ — `ping_interval` far exceeds any real-socket test.
//! - `Message::Frame` does not arise from real-socket receive (tungstenite docs); that branch is skipped.
//! - ★[`LinkRx::recv`] cancel safety is not load-bearing★ — not because it is never cancelled (`drop_link` aborts the read task), but because every canceller
//!   drops both ends right after (`peer.rs` `drop_link` · `handshake_once` · `shake_hands`). Not an `[unverified]` debt; a new caller that re-reads a link after
//!   cancelling voids this.

use std::io;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::future::BoxFuture;
use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::{TcpListener, TcpSocket};
use tokio::sync::mpsc;
use tokio::task::{JoinHandle, JoinSet};
use tokio_tungstenite::tungstenite::handshake::server::{
    Callback, ErrorResponse, Request, Response,
};
use tokio_tungstenite::tungstenite::http::{header, HeaderMap, StatusCode};
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode as WsCloseCode;
use tokio_tungstenite::tungstenite::protocol::{CloseFrame, WebSocketConfig};
use tokio_tungstenite::tungstenite::{Bytes, Message};
use tokio_tungstenite::WebSocketStream;

use crate::clock::{with_deadline, Clock, Elapsed};
use crate::frame::{Close, CloseCode, Frame};
use crate::link::{
    Accepted, Address, ConnPermit, DialFailure, Dialer, LinkError, LinkInfo, LinkRead, LinkRx,
    LinkTx, Listener, PermitPool, SocketEnds,
};
use crate::mask::mask_reason;

/// Maximum bytes a close message can carry — **not a character count**. RFC 6455 §5.5 caps control payloads at 125 bytes and the code takes 2. tungstenite
/// does not check the outgoing side but the receiver cuts anything over 125 (`ControlFrameTooBig`), turning 「connected but rejected」 into a read error and
/// burning the reconnect budget — [`WsTx::close`] truncates first.
const MAX_CLOSE_REASON_BYTES: usize = 123;

/// Kernel accept queue length (tokio's default). Unrelated to the upgrade permits.
const LISTEN_BACKLOG: u32 = 1024;

/// Pause before accepting again after a TCP accept error (branches: file header). ★Not a security value — it only stops the loop from spinning★, so it is not an
/// [`AcceptPolicy`] field. ★Uses tokio's real clock, not the listener's [`Clock`]★ (a hand-advanced clock would stall the accept loop).
pub const ACCEPT_ERROR_PAUSE: Duration = Duration::from_millis(100);

/// Whether to pause after a TCP accept error. ★Per-connection errors do not pause★ — pausing would let a peer that cuts right after opening delay accepting.
fn pauses_after_accept_error(kind: io::ErrorKind) -> bool {
    !matches!(
        kind,
        io::ErrorKind::ConnectionRefused
            | io::ErrorKind::ConnectionAborted
            | io::ErrorKind::ConnectionReset
    )
}

/// Bytes usable for a close message on a link capped at `max_payload`: min([`MAX_CLOSE_REASON_BYTES`], `max_payload` − 2). `None` = under 2, so even the code
/// cannot be carried. ★Also capped by `max_payload`★ — tungstenite applies the receiving `max_frame_size` to control frames too (`protocol/frame/mod.rs:181`).
fn close_reason_room(max_payload: usize) -> Option<usize> {
    max_payload
        .checked_sub(2)
        .map(|room| room.min(MAX_CLOSE_REASON_BYTES))
}

/// Truncates the close message to within `room` bytes ([`close_reason_room`]) ★at a character boundary★ (a byte cut gives invalid UTF-8, a different failure).
fn clamp_close_reason(reason: &str, room: usize) -> &str {
    if reason.len() <= room {
        return reason;
    }
    &reason[..reason.floor_char_boundary(room)]
}

/// The receiving side's size cap — the same value for one message and one frame (TRD 3 §5 15). The other fields are tungstenite defaults.
fn ws_config(max_message_bytes: usize) -> WebSocketConfig {
    WebSocketConfig::default()
        .max_message_size(Some(max_message_bytes))
        .max_frame_size(Some(max_message_bytes))
}

/// The base property of one link end; both the dialing and receiving streams satisfy it, so one [`LinkTx`]/[`LinkRx`] implementation serves both.
pub(crate) trait WsStream: AsyncRead + AsyncWrite + Unpin + Send + 'static {}

impl<S> WsStream for S where S: AsyncRead + AsyncWrite + Unpin + Send + 'static {}

// ── Dialing side ──────────────────────────────────────────────────────────────

/// A [`Dialer`] that opens a real WS link; [`Address`] goes to `tokio-tungstenite` as is (`ws://host:port/path`), a syntax violation arrives as [`LinkError`].
/// ★Every failure is [`DialFailure::Unreachable`] — never [`DialFailure::Refused`]★: an HTTP rejection at upgrade (e.g. the Origin 403) has no close code, and mapping
/// a status to terminal or redial is a policy this adapter does not own (a transient fault would become a give-up). A consumer that knows the terminal outcome
/// returns `Refused` from its own [`Dialer`] wrapping this one (TRD 3 §3-0 ⑤).
pub struct WsDialer {
    max_message_bytes: usize,
}

impl WsDialer {
    /// Byte cap of one message and one frame: the receiving size cap and the boundary where an outgoing frame becomes an error. Use the peer listener's
    /// [`AcceptPolicy::max_message_bytes`].
    pub fn new(max_message_bytes: usize) -> Self {
        Self { max_message_bytes }
    }
}

impl Dialer for WsDialer {
    fn dial(
        &self,
        addr: &Address,
    ) -> BoxFuture<'_, Result<(Box<dyn LinkTx>, Box<dyn LinkRx>), DialFailure>> {
        // ★The link is created inside the future★ ([`Dialer`] contract: cancelled, nothing left behind); the address is an owned String (the signature forbids borrowing `addr`).
        let url = addr.as_str().to_string();
        let max = self.max_message_bytes;
        Box::pin(async move {
            let (ws, _response) = tokio_tungstenite::connect_async_with_config(
                url.as_str(),
                Some(ws_config(max)),
                false,
            )
            .await
            .map_err(|e| LinkError::new(format!("ws connect {url}: {e}")))?;
            let ends = dialed_ends(ws.get_ref());
            Ok(split_link(ws, max, ends))
        })
    }
}

/// The TCP ends of a dialed stream — `None` for a TLS stream (TLS is off in this crate's features) or when the OS will not tell.
fn dialed_ends(
    stream: &tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
) -> Option<SocketEnds> {
    match stream {
        tokio_tungstenite::MaybeTlsStream::Plain(tcp) => Some(SocketEnds {
            local: tcp.local_addr().ok()?,
            peer: tcp.peer_addr().ok()?,
        }),
        _ => None,
    }
}

// ── Receiving side ────────────────────────────────────────────────────────────

/// The accept policy of [`WsListener`] (TRD 3 §5 17 · C1). ★No `Default`, no allow-everything value, not `#[non_exhaustive]`★ — a crate-chosen value would be a
/// security policy set without the consumer, checks cannot be turned off (tests use real values), and a new field must force the consumer to choose.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcceptPolicy {
    /// Origin values accepted during an upgrade carrying an Origin header, compared byte for byte (TRD 3 §5 2). ★Empty = every upgrade with an Origin gets 403★. ★No Origin =
    /// always accepted★ (non-browser clients; this blocks browser pages — auth belongs to the handshake). Two or more headers, or non-text, is cut.
    pub allowed_origins: Vec<String>,
    /// Host-header names accepted (before the port, case-insensitive), e.g. `127.0.0.1` · `localhost` (TRD 3 §5 16, DNS rebinding); the port must be this listener's
    /// (none = 80). ★No Host = accepted★; two or more headers or non-text is cut; empty = every upgrade with a Host is cut. ★IPv6 is written with brackets★ (`[::1]`);
    /// a name with a colon outside brackets matches nothing, so [`WsListener::bind`] returns `Err`.
    pub allowed_host_names: Vec<String>,
    /// Deadline from TCP accept to the end of the WS upgrade (TRD 3 §5 12); past it the TCP is closed. ★The auth deadline starts where this ends
    /// ([`Accepted::upgraded_at`]) — they add up★.
    pub upgrade_timeout: Duration,
    /// Upgrade permits (TRD 3 §5 14): taken at TCP accept, released when the registered peer enters the operating phase (or [`Accepted`] is dropped). ★The ready queue
    /// counts against them too★ (its cap is this value). All used: the new TCP is closed after accept. 0 = accept nothing.
    pub max_pending_upgrades: usize,
    /// Byte cap of one message and one frame (TRD 3 §5 15): the accepted link's receiving cap and the boundary where an outgoing frame becomes an error. Use the dialer's
    /// [`WsDialer::new`] value.
    pub max_message_bytes: usize,
    /// Refusal report queue size (TRD 3 §5 18); full = new reports dropped ([`AcceptRefusals::dropped`]). 0 counts as 1.
    pub refusal_queue: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum RefusalKind {
    /// The upgrade permits are all used — TCP was closed right after accept (no HTTP response).
    SlotsFull,
    /// The upgrade did not finish within [`AcceptPolicy::upgrade_timeout`].
    UpgradeTimeout,
    /// The Origin header is outside [`AcceptPolicy::allowed_origins`] — 403.
    OriginRejected,
    /// The Host header is outside [`AcceptPolicy::allowed_host_names`] · the bound port — 403.
    HostRejected,
    /// The upgrade failed for a reason outside the branches above — not a WS request · the peer cut during the upgrade, etc.
    UpgradeFailed,
    /// The TCP accept itself failed — there may be no peer, so the remote address is empty.
    AcceptError,
}

/// One cut peer's report (TRD 3 §5 18). ★Text fields are already masked ([`crate::mask_reason`])★; the cut precedes the auth frame, so no token can ride.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct AcceptRefusal {
    pub kind: RefusalKind,
    /// The peer end address (`ip:port`). `None` for [`RefusalKind::AcceptError`].
    pub remote: Option<String>,
    /// The Origin header of the upgrade request — filled only for Origin and Host refusals that read up to the headers.
    pub origin: Option<String>,
    /// The Host header of the upgrade request — same as above.
    pub host: Option<String>,
}

/// The refusal-report receiver returned by [`WsListener::bind`] (outside the [`Listener`] trait, whose `Err` means only "ended"). ★You may drop it★ (the listener keeps running);
/// ★unread, the queue fills and new reports are lost first★ ([`AcceptRefusals::dropped`]). Dropping the listener drains the rest, then `None`.
pub struct AcceptRefusals {
    rx: mpsc::Receiver<AcceptRefusal>,
    dropped: Arc<AtomicU64>,
}

impl AcceptRefusals {
    /// Waits for the next report. ★Cancel safe★.
    pub async fn recv(&mut self) -> Option<AcceptRefusal> {
        self.rx.recv().await
    }

    /// One report currently queued — `None` if there is none (it does not wait).
    pub fn try_recv(&mut self) -> Option<AcceptRefusal> {
        self.rx.try_recv().ok()
    }

    /// The cumulative number of reports dropped because the queue was full — it never decreases.
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Acquire)
    }
}

/// Puts a refusal report into the queue — if full, drops it and counts.
struct Reporter {
    tx: mpsc::Sender<AcceptRefusal>,
    dropped: Arc<AtomicU64>,
}

impl Reporter {
    fn report(
        &self,
        kind: RefusalKind,
        remote: Option<SocketAddr>,
        origin: Option<&str>,
        host: Option<&str>,
    ) {
        let refusal = AcceptRefusal {
            kind,
            remote: remote.map(|addr| mask_reason(&addr.to_string())),
            origin: origin.map(mask_reason),
            host: host.map(mask_reason),
        };
        // ★Does not wait★ — waiting on the consumer would stall accepting during a refusal flood.
        if self.tx.try_send(refusal).is_err() {
            self.dropped.fetch_add(1, Ordering::AcqRel);
        }
    }
}

/// A [`Listener`] for real WS links: one TCP accept task, one task per upgrade, finished upgrades wait in a ready queue (holding their permit) for [`Listener::accept`].
/// ★Dropping it reaps those tasks and closes the queued links★ (the dialer sees a link loss).
pub struct WsListener {
    local: SocketAddr,
    ready: mpsc::Receiver<Accepted>,
    permits: PermitPool,
    driver: JoinHandle<()>,
}

impl WsListener {
    /// Opens on `addr` and starts accepting; returns the listener and its refusal receiver. Port 0 = kernel-picked ([`WsListener::local_addr`]).
    /// ★Loopback only★ (TRD 3 §5 1 · C8): a non-loopback bound address is `Err` before listen; plain `ws://` is never opened outside (TRD 3 §4). Also `Err` for an
    /// [`AcceptPolicy::allowed_host_names`] name with a colon outside brackets. ★`clock` must be the registry's clock★ — it stamps [`Accepted::upgraded_at`], where the
    /// auth deadline starts ([`crate::Registry::attach_with_policy`] asserts it in debug) — and drives the upgrade deadline. Call inside a tokio runtime (it panics outside).
    pub fn bind(
        addr: SocketAddr,
        policy: AcceptPolicy,
        clock: Arc<dyn Clock>,
    ) -> Result<(Self, AcceptRefusals), LinkError> {
        ensure_host_names(&policy.allowed_host_names)?;
        let socket = if addr.is_ipv4() {
            TcpSocket::new_v4()
        } else {
            TcpSocket::new_v6()
        }
        .map_err(|e| LinkError::new(format!("ws bind {addr}: {e}")))?;
        socket
            .bind(addr)
            .map_err(|e| LinkError::new(format!("ws bind {addr}: {e}")))?;
        let local = socket
            .local_addr()
            .map_err(|e| LinkError::new(format!("ws local_addr: {e}")))?;
        // ★Check before listen★ — refused after listening, outside connections would already be queued.
        ensure_loopback(local)?;
        let tcp = socket
            .listen(LISTEN_BACKLOG)
            .map_err(|e| LinkError::new(format!("ws listen {local}: {e}")))?;

        let permits = PermitPool::new(policy.max_pending_upgrades);
        // ★The ready queue never fills★ — queued length ≤ permits held ≤ this capacity.
        let (ready_tx, ready) = mpsc::channel(policy.max_pending_upgrades.max(1));
        let (refusal_tx, refusal_rx) = mpsc::channel(policy.refusal_queue.max(1));
        let dropped = Arc::new(AtomicU64::new(0));
        let shared = Arc::new(Shared {
            local,
            permits: permits.clone(),
            clock,
            reporter: Reporter {
                tx: refusal_tx,
                dropped: dropped.clone(),
            },
            policy,
        });
        let driver = tokio::spawn(drive(tcp, shared, ready_tx));
        Ok((
            Self {
                local,
                ready,
                permits,
                driver,
            },
            AcceptRefusals {
                rx: refusal_rx,
                dropped,
            },
        ))
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.local
    }

    pub fn dial_address(&self) -> Address {
        Address::new(format!("ws://{}/", self.local))
    }

    /// Upgrade permits currently held (mid-upgrade, ready queue, taken-not-released). ★The pool itself is not handed out★ — it could be used to block accepting.
    pub fn permits_in_use(&self) -> usize {
        self.permits.in_use()
    }

    /// The number of upgrade permits — [`AcceptPolicy::max_pending_upgrades`].
    pub fn permit_limit(&self) -> usize {
        self.permits.limit()
    }

    pub fn queued(&self) -> usize {
        self.ready.len()
    }
}

impl Listener for WsListener {
    fn accept(&mut self) -> BoxFuture<'_, Result<Accepted, LinkError>> {
        Box::pin(async move {
            self.ready
                .recv()
                .await
                .ok_or_else(|| LinkError::new("ws listener: accept task ended"))
        })
    }
}

impl Drop for WsListener {
    fn drop(&mut self) {
        // The accept task holds the set of upgrade tasks, so they are reaped together (`drive`).
        self.driver.abort();
    }
}

/// Whether the bound address is loopback — if not, refuse (TRD 3 §5 1).
fn ensure_loopback(local: SocketAddr) -> Result<(), LinkError> {
    if local.ip().is_loopback() {
        Ok(())
    } else {
        Err(LinkError::new(format!(
            "ws bind {local}: not loopback — a plaintext ws:// listener is opened only on loopback"
        )))
    }
}

/// Whether any accepted Host name has a colon outside the brackets — if so, refuse ([`AcceptPolicy::allowed_host_names`]).
fn ensure_host_names(names: &[String]) -> Result<(), LinkError> {
    let bracketed = |name: &str| name.starts_with('[') && name.ends_with(']');
    match names
        .iter()
        .find(|name| name.contains(':') && !bracketed(name))
    {
        Some(name) => Err(LinkError::new(format!(
            "ws accept policy: Host name {name:?} has a colon outside brackets — write IPv6 as `[::1]` · leave the port out"
        ))),
        None => Ok(()),
    }
}

struct Shared {
    local: SocketAddr,
    policy: AcceptPolicy,
    permits: PermitPool,
    clock: Arc<dyn Clock>,
    reporter: Reporter,
}

/// The TCP accept loop. ★Never waits for the upgrade★ (one task per connection — TRD 3 §5 12); `upgrades` holds the tasks, so reaping this reaps them.
async fn drive(tcp: TcpListener, shared: Arc<Shared>, ready: mpsc::Sender<Accepted>) {
    let mut upgrades = JoinSet::new();
    loop {
        tokio::select! {
            accepted = tcp.accept() => match accepted {
                Ok((stream, remote)) => match shared.permits.try_take() {
                    Some(permit) => {
                        // ★Stamp the deadline reference before spawning★ — inside the task, spawn delay would escape the deadline (TRD 3 §5 12).
                        let accepted_at = shared.clock.now();
                        let ends = stream.local_addr().ok().map(|local| SocketEnds { local, peer: remote });
                        upgrades.spawn(upgrade(
                            stream,
                            remote,
                            ends,
                            accepted_at,
                            permit,
                            shared.clone(),
                            ready.clone(),
                        ));
                    }
                    None => {
                        // Overflow closes **the new one** (TRD 3 §5 14).
                        drop(stream);
                        shared
                            .reporter
                            .report(RefusalKind::SlotsFull, Some(remote), None, None);
                    }
                },
                Err(error) => {
                    shared.reporter.report(RefusalKind::AcceptError, None, None, None);
                    if pauses_after_accept_error(error.kind()) {
                        tokio::time::sleep(ACCEPT_ERROR_PAUSE).await;
                    }
                }
            },
            // Reaps finished upgrade tasks — if not reaped, the results pile up in the set.
            Some(_) = upgrades.join_next(), if !upgrades.is_empty() => {}
        }
    }
}

/// The decision on the upgrade request headers. If it cuts, it carries that branch and the header value seen (before masking).
#[derive(Debug, PartialEq, Eq)]
struct Screened {
    kind: RefusalKind,
    origin: Option<String>,
    host: Option<String>,
}

/// One link's upgrade: finished before `accepted_at` + [`AcceptPolicy::upgrade_timeout`] it enters the ready queue, otherwise it is closed and reported; `permit`
/// goes out with the link or is dropped here. ★The deadline is checked three times★ — before starting, while waiting, and after finishing (a finish and the deadline in
/// one poll resolve in the upgrade's favour in [`with_deadline`], so that moment counts as past the deadline).
async fn upgrade<S: WsStream>(
    stream: S,
    remote: SocketAddr,
    ends: Option<SocketEnds>,
    accepted_at: Instant,
    permit: ConnPermit,
    shared: Arc<Shared>,
    ready: mpsc::Sender<Accepted>,
) {
    let policy = &shared.policy;
    let report = |kind, origin: Option<&str>, host: Option<&str>| {
        shared.reporter.report(kind, Some(remote), origin, host)
    };
    // Time left until the deadline (`None` = reached). ★Subtract elapsed time, do not add the deadline to now★ — `Instant + Duration` panics on overflow.
    let remaining_at = |now: Instant| {
        policy
            .upgrade_timeout
            .checked_sub(now.saturating_duration_since(accepted_at))
            .filter(|left| !left.is_zero())
    };
    let Some(remaining) = remaining_at(shared.clock.now()) else {
        report(RefusalKind::UpgradeTimeout, None, None);
        return;
    };
    let mut verdict: Option<Result<Option<String>, Screened>> = None;
    let handshake = tokio_tungstenite::accept_hdr_async_with_config(
        stream,
        Gate {
            policy,
            local_port: shared.local.port(),
            verdict: &mut verdict,
        },
        Some(ws_config(policy.max_message_bytes)),
    );
    let upgraded = with_deadline(&*shared.clock, remaining, handshake).await;
    let ws = match upgraded {
        Ok(Ok(ws)) => ws,
        Ok(Err(_)) => {
            match verdict {
                Some(Err(screened)) => report(
                    screened.kind,
                    screened.origin.as_deref(),
                    screened.host.as_deref(),
                ),
                _ => report(RefusalKind::UpgradeFailed, None, None),
            }
            return;
        }
        Err(Elapsed) => {
            // ★A cut by the header decision mid-403 reports that branch★ — as the deadline it would lose the reason and header seen.
            match verdict {
                Some(Err(screened)) => report(
                    screened.kind,
                    screened.origin.as_deref(),
                    screened.host.as_deref(),
                ),
                _ => report(RefusalKind::UpgradeTimeout, None, None),
            }
            return;
        }
    };
    // ★Stamp the finish with the registry's clock★ — the auth deadline starts here (TRD 3 §5 3).
    let upgraded_at = shared.clock.now();
    if remaining_at(upgraded_at).is_none() {
        // Past the deadline — even a finished upgrade is not accepted; link and permit drop here.
        report(RefusalKind::UpgradeTimeout, None, None);
        return;
    }
    let origin = match verdict {
        Some(Ok(origin)) => origin,
        _ => None,
    };
    let (tx, rx) = split_link(ws, policy.max_message_bytes, ends);
    let info = LinkInfo {
        remote: Some(remote.to_string()),
        origin,
        peer_key: None,
        socket_ends: ends,
    };
    let accepted = Accepted::new(tx, rx, info, upgraded_at, permit);
    match ready.try_send(accepted) {
        Ok(()) => {}
        // Unreachable (capacity comment in `WsListener::bind`); treat as permits full.
        Err(mpsc::error::TrySendError::Full(_)) => report(RefusalKind::SlotsFull, None, None),
        // The listener was dropped — the link is closed here.
        Err(mpsc::error::TrySendError::Closed(_)) => {}
    }
}

/// The tungstenite header callback — checks upgrade headers against the policy and leaves the decision in `verdict`; a cut returns 403.
struct Gate<'a> {
    policy: &'a AcceptPolicy,
    local_port: u16,
    verdict: &'a mut Option<Result<Option<String>, Screened>>,
}

impl Callback for Gate<'_> {
    fn on_request(self, request: &Request, response: Response) -> Result<Response, ErrorResponse> {
        let screened = screen(self.policy, request.headers(), self.local_port);
        let passed = screened.is_ok();
        *self.verdict = Some(screened);
        if passed {
            Ok(response)
        } else {
            Err(forbidden())
        }
    }
}

/// The 403 response. ★The body does not name the failing check★ — the refusal report carries the branch.
fn forbidden() -> ErrorResponse {
    let mut response = ErrorResponse::new(None);
    *response.status_mut() = StatusCode::FORBIDDEN;
    response
}

/// Checks the upgrade request headers against the accept policy — if it passes, returns the Origin to accept (`None` if absent). Origin is checked first.
fn screen(
    policy: &AcceptPolicy,
    headers: &HeaderMap,
    local_port: u16,
) -> Result<Option<String>, Screened> {
    let origin = single(headers, header::ORIGIN);
    let host = single(headers, header::HOST);
    let refused = |kind| Screened {
        kind,
        origin: origin.raw(),
        host: host.raw(),
    };
    let origin = match &origin {
        OneHeader::Absent => None,
        OneHeader::One(value) if policy.allowed_origins.iter().any(|a| a == value) => {
            Some(value.clone())
        }
        _ => return Err(refused(RefusalKind::OriginRejected)),
    };
    match &host {
        OneHeader::Absent => {}
        OneHeader::One(value) if host_matches(value, &policy.allowed_host_names, local_port) => {}
        _ => return Err(refused(RefusalKind::HostRejected)),
    }
    Ok(origin)
}

enum OneHeader {
    Absent,
    One(String),
    /// Two or more, or non-text — cut undecided; the lossy value is for reporting.
    Malformed(String),
}

impl OneHeader {
    fn raw(&self) -> Option<String> {
        match self {
            OneHeader::Absent => None,
            OneHeader::One(value) | OneHeader::Malformed(value) => Some(value.clone()),
        }
    }
}

fn single(headers: &HeaderMap, name: header::HeaderName) -> OneHeader {
    let mut values = headers.get_all(name).iter();
    match (values.next(), values.next()) {
        (None, _) => OneHeader::Absent,
        (Some(value), None) => match value.to_str() {
            Ok(text) => OneHeader::One(text.to_owned()),
            Err(_) => OneHeader::Malformed(String::from_utf8_lossy(value.as_bytes()).into_owned()),
        },
        (Some(first), Some(_)) => {
            OneHeader::Malformed(String::from_utf8_lossy(first.as_bytes()).into_owned())
        }
    }
}

/// Whether the Host value `name[:port]` matches the accepted names and this listener's port (digits only; 80 if absent; `[::1]:port` form).
fn host_matches(value: &str, names: &[String], local_port: u16) -> bool {
    let (name, port) = match value.rsplit_once(':') {
        Some((name, port)) if !port.contains(']') => (name, Some(port)),
        _ => (value, None),
    };
    let port_ok = match port {
        Some(text) => {
            !text.is_empty()
                && text.bytes().all(|b| b.is_ascii_digit())
                && text.parse::<u16>() == Ok(local_port)
        }
        None => local_port == 80,
    };
    port_ok
        && names
            .iter()
            .any(|allowed| allowed.eq_ignore_ascii_case(name))
}

/// Splits an open stream into the two ends of a link; `max_payload` = outgoing frame cap. ★Crate-internal only★ (TRD 3 §3-5 · §5 17) — opened, anyone could skip every
/// [`WsListener`] check. The ends share the stream through a `BiLock`; ★the socket closes only when **both** ends are dropped★ — the caller must guarantee that
/// ([`LinkTx`] rustdoc).
pub(crate) fn split_link<S: WsStream>(
    ws: WebSocketStream<S>,
    max_payload: usize,
    ends: Option<SocketEnds>,
) -> (Box<dyn LinkTx>, Box<dyn LinkRx>) {
    let (sink, stream) = ws.split();
    (
        Box::new(WsTx {
            sink,
            max_payload,
            ends,
        }),
        Box::new(WsRx {
            stream,
            failed: false,
        }),
    )
}

// ── The two link ends ─────────────────────────────────────────────────────────

struct WsTx<S: WsStream> {
    sink: SplitSink<WebSocketStream<S>, Message>,
    /// Outgoing frame payload cap — the size cap received by the side that opened this link ([`split_link`]).
    max_payload: usize,
    /// [`LinkTx::socket_ends`].
    ends: Option<SocketEnds>,
}

impl<S: WsStream> LinkTx for WsTx<S> {
    fn send(&mut self, frame: Frame) -> BoxFuture<'_, Result<(), LinkError>> {
        Box::pin(async move {
            // ★A frame over the cap fails **as a value**★ — else only the receiver enforces it and an innocent peer burns its reconnect budget. ★Do not go back to panicking★
            //   (ADR-0292): the old `debug_assert` fired in the supervisor task where nobody holds the `JoinHandle` (`Unstarted::start`), so the state froze at `Live`
            //   with no event. As a value the supervisor turns it into `LinkLost` (`Supervisor::write`); the message is dropped (file header).
            let len = payload_len(&frame);
            if len > self.max_payload {
                return Err(LinkError::new(format!(
                    "ws write: the outgoing frame is {len} bytes — above the cap {} the receiver cuts the link",
                    self.max_payload
                )));
            }
            self.sink
                .send(to_message(frame))
                .await
                .map_err(|e| LinkError::new(format!("ws write: {e}")))
        })
    }

    fn ping(&mut self) -> BoxFuture<'_, Result<(), LinkError>> {
        Box::pin(async move {
            self.sink
                .send(Message::Ping(Bytes::new()))
                .await
                .map_err(|e| LinkError::new(format!("ws ping: {e}")))
        })
    }

    fn close(&mut self, close: Close) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            let message = match close_reason_room(self.max_payload) {
                Some(room) => Message::Close(Some(CloseFrame {
                    code: WsCloseCode::from(close.code.0),
                    reason: clamp_close_reason(&close.reason, room).into(),
                })),
                // ★A cap that cannot carry the code sends an empty close★ — carrying it would exceed the cap and cut the peer's read.
                None => Message::Close(None),
            };
            // No place to report a failure ([`LinkTx::close`] contract); an unsent close reads as cut without a word (`frame.rs`).
            let _ = self.sink.send(message).await;
            let _ = self.sink.close().await;
        })
    }

    fn socket_ends(&self) -> Option<SocketEnds> {
        self.ends
    }
}

struct WsRx<S: WsStream> {
    stream: SplitStream<WebSocketStream<S>>,
    /// Reading already failed. ★Keeps [`LinkRead::Closed`] meaning 「the peer said it closed」★ (see [`WsRx::recv`]).
    failed: bool,
}

impl<S: WsStream> LinkRx for WsRx<S> {
    fn recv(&mut self) -> BoxFuture<'_, Result<LinkRead, LinkError>> {
        Box::pin(async move {
            // ★A read that failed once keeps failing★ — else the [`LinkRead::Closed`] contract breaks: a socket dropped without a word errors once
            //   (`ResetWithoutClosingHandshake`), then tungstenite's `ended` marker makes the next read `Ready(None)` (0.26.2 `lib.rs:303`), a close nobody spoke.
            if self.failed {
                return Err(LinkError::new("ws read: link already failed"));
            }
            loop {
                let msg = match self.stream.next().await {
                    Some(Ok(msg)) => msg,
                    Some(Err(e)) => {
                        self.failed = true;
                        return Err(LinkError::new(format!("ws read: {e}")));
                    }
                    // ★A dried-up stream is **not** 「cut without a word」★ — `Ready(None)` arises only after a close frame was read (tokio-tungstenite 0.26.2
                    //   `lib.rs:303`, `protocol/mod.rs:706`); a vanished peer comes through the `Err` branch.
                    None => return Ok(LinkRead::Closed(None)),
                };
                if let Some(read) = from_message(msg) {
                    return Ok(read);
                }
            }
        })
    }
}

/// Translates an incoming `Message` into the link vocabulary. `None` = this branch is not raised and the next one is read.
fn from_message(msg: Message) -> Option<LinkRead> {
    match msg {
        Message::Text(text) => Some(LinkRead::Frame(Frame::Text(text.as_str().to_owned()))),
        Message::Binary(bytes) => Some(LinkRead::Frame(Frame::Binary(bytes.to_vec()))),
        Message::Ping(_) | Message::Pong(_) => Some(LinkRead::Frame(Frame::Keepalive)),
        Message::Close(Some(frame)) => Some(LinkRead::Closed(Some(Close::new(
            CloseCode(u16::from(frame.code)),
            frame.reason.as_str().to_owned(),
        )))),
        Message::Close(None) => Some(LinkRead::Closed(None)),
        // It does not come up from real-socket receiving (file header).
        Message::Frame(_) => None,
    }
}

fn to_message(frame: Frame) -> Message {
    match frame {
        Frame::Text(text) => Message::Text(text.into()),
        Frame::Binary(bytes) => Message::Binary(bytes.into()),
        // ★Same as [`LinkTx::ping`]★ — keepalive is transport vocabulary, the payload is not redesigned (reason in that rustdoc).
        Frame::Keepalive => Message::Ping(Bytes::new()),
    }
}

fn payload_len(frame: &Frame) -> usize {
    match frame {
        Frame::Text(text) => text.len(),
        Frame::Binary(bytes) => bytes.len(),
        Frame::Keepalive => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio_tungstenite::tungstenite::protocol::frame::Frame as WsFrame;
    use tokio_tungstenite::tungstenite::Utf8Bytes;

    // ── Frame ↔ Message table ──
    #[test]
    fn outgoing_frames_map_to_their_message() {
        assert_eq!(
            to_message(Frame::Text("hi".into())),
            Message::Text(Utf8Bytes::from("hi"))
        );
        assert_eq!(
            to_message(Frame::Binary(vec![1, 2, 3])),
            Message::Binary(Bytes::from_static(&[1, 2, 3]))
        );
        // ★Keepalive goes out as a Ping and the payload is empty★ — the reason is in the [`to_message`] comment.
        assert_eq!(to_message(Frame::Keepalive), Message::Ping(Bytes::new()));
    }

    #[test]
    fn incoming_messages_map_back() {
        let text = from_message(Message::Text(Utf8Bytes::from("hi")));
        assert!(matches!(text, Some(LinkRead::Frame(Frame::Text(t))) if t == "hi"));
        let binary = from_message(Message::Binary(Bytes::from_static(&[7])));
        assert!(matches!(binary, Some(LinkRead::Frame(Frame::Binary(b))) if b == vec![7]));
    }

    #[test]
    fn both_keepalive_directions_arrive_as_one_frame() {
        // ★Pong and Ping both come up as the same thing★ — only the fact of arrival is the signal (that contract in `frame.rs`).
        for msg in [
            Message::Ping(Bytes::from_static(&[9])),
            Message::Pong(Bytes::new()),
        ] {
            assert!(matches!(
                from_message(msg),
                Some(LinkRead::Frame(Frame::Keepalive))
            ));
        }
    }

    #[test]
    fn a_spoken_close_carries_its_code_and_reason() {
        let msg = Message::Close(Some(CloseFrame {
            code: WsCloseCode::from(4001),
            reason: Utf8Bytes::from("version 3 != 4"),
        }));
        match from_message(msg) {
            Some(LinkRead::Closed(Some(close))) => {
                assert_eq!(close.code, CloseCode(4001));
                assert_eq!(close.reason, "version 3 != 4");
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(
            from_message(Message::Close(None)),
            Some(LinkRead::Closed(None))
        ));
    }

    #[test]
    fn a_raw_frame_is_skipped_rather_than_lifted() {
        assert!(from_message(Message::Frame(WsFrame::ping(Bytes::new()))).is_none());
    }

    // ── Close message cap ──
    #[test]
    fn a_short_reason_is_left_alone() {
        assert_eq!(clamp_close_reason("nope", MAX_CLOSE_REASON_BYTES), "nope");
        let exact = "x".repeat(MAX_CLOSE_REASON_BYTES);
        assert_eq!(clamp_close_reason(&exact, MAX_CLOSE_REASON_BYTES), exact);
    }

    #[test]
    fn a_long_reason_is_cut_on_a_character_boundary() {
        let long = "가".repeat(60); // hangul-ok
        let cut = clamp_close_reason(&long, MAX_CLOSE_REASON_BYTES);
        assert!(cut.len() <= MAX_CLOSE_REASON_BYTES);
        assert_eq!(cut.chars().count(), MAX_CLOSE_REASON_BYTES / 3);
        assert!(long.starts_with(cut));
    }

    #[test]
    fn a_four_byte_character_is_cut_below_the_limit_not_at_it() {
        // ★4-byte characters are why this test exists★ — with 3-byte ones 123 is a multiple of 3 and a 「cut at 123」 mutant passes; with 4-byte it dies on a slice panic.
        let emoji = "\u{1F600}".repeat(40);
        assert_eq!(emoji.len(), 160);
        let cut = clamp_close_reason(&emoji, MAX_CLOSE_REASON_BYTES);
        assert_eq!(cut.len(), 120, "4-byte characters must be cut at 120");
        assert_eq!(cut.chars().count(), 30);
        assert!(emoji.starts_with(cut));
    }

    // ── The bytes the close path actually emits ──
    //
    // ★Pairs with the tests above that call [`clamp_close_reason`] directly★ — they never prove [`WsTx::close`] calls it (a mutant deleting the truncation passes).
    struct Recorder(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

    impl tokio::io::AsyncWrite for Recorder {
        fn poll_write(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
            buf: &[u8],
        ) -> std::task::Poll<std::io::Result<usize>> {
            self.0.lock().unwrap().extend_from_slice(buf);
            std::task::Poll::Ready(Ok(buf.len()))
        }

        fn poll_flush(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            std::task::Poll::Ready(Ok(()))
        }

        fn poll_shutdown(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            std::task::Poll::Ready(Ok(()))
        }
    }

    impl tokio::io::AsyncRead for Recorder {
        /// ★It returns EOF★ — if it returned `Pending`, the close cleanup would hang right there, and libtest has no per-test
        /// deadline, so that regression would stop the whole suite.
        fn poll_read(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
            _buf: &mut tokio::io::ReadBuf<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            std::task::Poll::Ready(Ok(()))
        }
    }

    #[tokio::test]
    async fn the_close_path_puts_a_legal_control_frame_on_the_wire() {
        let written = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        // It plays the server role, so outgoing frames are **not masked** — so the bytes are read as is.
        let ws = WebSocketStream::from_raw_socket(
            Recorder(written.clone()),
            tokio_tungstenite::tungstenite::protocol::Role::Server,
            None,
        )
        .await;
        let (mut tx, _rx) = split_link(ws, usize::MAX, None);
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            tx.close(Close::new(CloseCode(4001), "긴".repeat(200))), // hangul-ok
        )
        .await
        .expect("the close did not finish");

        let bytes = written.lock().unwrap().clone();
        assert_eq!(bytes[0], 0x88, "not a close frame: {:02x?}", &bytes[..2]);
        // ★The second byte is the payload length★ — 126 means extended length, itself over 125 (RFC 6455 §5.5 forbids it on control frames).
        assert!(
            bytes[1] <= 125,
            "the control frame payload is {} bytes — the receiver cuts the link",
            bytes[1]
        );
        // The code goes out as is even when truncated — which is why the branching material is the code, not the message.
        assert_eq!(u16::from_be_bytes([bytes[2], bytes[3]]), 4001);
    }

    #[test]
    fn a_cut_reason_fits_a_control_frame() {
        // Measures on a real frame that code 2 bytes + message does not exceed 125 — if it does, the receiving side
        //   cuts with `ControlFrameTooBig` ([`MAX_CLOSE_REASON_BYTES`]).
        // hangul-ok-begin
        for reason in ["", "짧다", &"긴".repeat(200), &"a".repeat(200)] {
            let frame = WsFrame::close(Some(CloseFrame {
                code: WsCloseCode::from(4001),
                reason: clamp_close_reason(reason, close_reason_room(usize::MAX).unwrap()).into(),
            }));
            assert!(frame.payload().len() <= 125, "{}", frame.payload().len());
        }
        // hangul-ok-end
    }

    // ★A size cap below 125 applies to the close frame too★ ([`close_reason_room`]).
    #[tokio::test]
    async fn the_close_frame_stays_within_a_small_configured_limit() {
        // (size cap, payload that must go out) — under 2 the code cannot be carried either, so it is an empty close.
        for (limit, payload) in [(10usize, 10usize), (2, 2), (1, 0), (0, 0)] {
            let written = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
            let ws = WebSocketStream::from_raw_socket(
                Recorder(written.clone()),
                tokio_tungstenite::tungstenite::protocol::Role::Server,
                None,
            )
            .await;
            let (mut tx, _rx) = split_link(ws, limit, None);
            tokio::time::timeout(
                std::time::Duration::from_secs(5),
                tx.close(Close::new(CloseCode(4001), "a".repeat(200))),
            )
            .await
            .expect("the close did not finish");

            let bytes = written.lock().unwrap().clone();
            assert_eq!(bytes[0], 0x88, "cap {limit}: not a close frame");
            assert_eq!(
                usize::from(bytes[1]),
                payload,
                "cap {limit}: payload length"
            );
            if payload >= 2 {
                assert_eq!(
                    u16::from_be_bytes([bytes[2], bytes[3]]),
                    4001,
                    "cap {limit}: code"
                );
            }
        }
    }

    // ── Outgoing size cap ──
    //
    // ★Proves the cap is the value given to [`split_link`]★ — a fixed constant would limit only the receiver.
    #[tokio::test]
    async fn a_frame_over_the_given_limit_fails_as_a_value_and_one_at_it_goes_out() {
        let written = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let ws = WebSocketStream::from_raw_socket(
            Recorder(written.clone()),
            tokio_tungstenite::tungstenite::protocol::Role::Server,
            None,
        )
        .await;
        let (mut tx, _rx) = split_link(ws, 8, None);
        let over = tx.send(Frame::Binary(vec![0; 9])).await;
        assert!(over.is_err(), "cap + 1 went out");
        assert!(
            written.lock().unwrap().is_empty(),
            "a rejected frame reached the socket"
        );
        tx.send(Frame::Binary(vec![0; 8]))
            .await
            .expect("exactly the cap must go out");
        assert!(!written.lock().unwrap().is_empty());
    }

    #[test]
    fn the_receive_limit_covers_a_message_and_a_frame_alike() {
        // ★Both must be covered★ — if only the frame is covered, a fragmented message grows up to tungstenite's default 64 MiB.
        let config = ws_config(1234);
        assert_eq!(config.max_message_size, Some(1234));
        assert_eq!(config.max_frame_size, Some(1234));
    }

    // ── loopback assertions ──
    #[test]
    fn only_a_loopback_address_may_be_listened_on() {
        for ok in ["127.0.0.1:0", "127.9.9.9:4000", "[::1]:0"] {
            assert!(ensure_loopback(ok.parse().unwrap()).is_ok(), "{ok}");
        }
        // ★`::ffff:127.0.0.1` is also refused★ — the standard decision (`Ipv6Addr::is_loopback`) sees only `::1` as loopback.
        for refused in [
            "0.0.0.0:0",
            "192.168.0.10:0",
            "[::]:0",
            "[::ffff:127.0.0.1]:0",
        ] {
            let err = ensure_loopback(refused.parse().unwrap()).expect_err(refused);
            assert!(err.message.contains("loopback"), "{err}");
        }
    }

    // ── Accepted Host names ──
    #[test]
    fn a_host_name_with_a_colon_outside_brackets_is_refused_at_bind() {
        let names = |list: &[&str]| list.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(ensure_host_names(&names(&["127.0.0.1", "localhost", "[::1]"])).is_ok());
        assert!(ensure_host_names(&[]).is_ok());
        for bad in ["::1", "fe80::1", "localhost:8080", "[::1]:8080"] {
            let err = ensure_host_names(&names(&["localhost", bad])).expect_err(bad);
            assert!(err.message.contains(bad), "{err}");
        }
        // ★A name written with its brackets matches the real header★ — the form the check above accepts must be the form the header check accepts.
        assert!(host_matches("[::1]:4000", &names(&["[::1]"]), 4000));
        assert!(!host_matches("[::1]:4000", &names(&["::1"]), 4000));
    }

    // ── Pause after an accept error ──
    #[test]
    fn only_accept_errors_not_tied_to_one_connection_pause_the_loop() {
        for kind in [
            io::ErrorKind::ConnectionRefused,
            io::ErrorKind::ConnectionAborted,
            io::ErrorKind::ConnectionReset,
        ] {
            assert!(!pauses_after_accept_error(kind), "rested on {kind:?}");
        }
        // Running out of file handles (`EMFILE`) has no named branch and arrives as a different branch per platform — measure with the named rest.
        for kind in [
            io::ErrorKind::Other,
            io::ErrorKind::OutOfMemory,
            io::ErrorKind::PermissionDenied,
            io::ErrorKind::InvalidInput,
        ] {
            assert!(pauses_after_accept_error(kind), "did not rest on {kind:?}");
        }
    }

    // ── Upgrade deadline (§5 12) ──
    //
    // ★The reference is the TCP accept time★ — nothing is accepted from the deadline on, even if the task ran late or finish and deadline share a poll.
    const UPGRADE_TIMEOUT: Duration = Duration::from_secs(10);

    /// An upgrade request with no Host or Origin — passes the header check.
    const UPGRADE_REQUEST: &[u8] =
        b"GET / HTTP/1.1\r\nConnection: Upgrade\r\nUpgrade: websocket\r\n\
        Sec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\r\n";

    struct Rig {
        clock: crate::testing::ManualClock,
        permits: PermitPool,
        shared: Arc<Shared>,
        ready_tx: mpsc::Sender<Accepted>,
        ready: mpsc::Receiver<Accepted>,
        refusals: AcceptRefusals,
    }

    fn rig() -> Rig {
        let clock = crate::testing::ManualClock::new();
        let permits = PermitPool::new(1);
        let (ready_tx, ready) = mpsc::channel(1);
        let (tx, rx) = mpsc::channel(4);
        let dropped = Arc::new(AtomicU64::new(0));
        let shared = Arc::new(Shared {
            local: "127.0.0.1:4000".parse().unwrap(),
            policy: AcceptPolicy {
                upgrade_timeout: UPGRADE_TIMEOUT,
                max_message_bytes: 1024,
                ..policy(&[], &[])
            },
            permits: permits.clone(),
            clock: clock.handle(),
            reporter: Reporter {
                tx,
                dropped: dropped.clone(),
            },
        });
        Rig {
            clock,
            permits,
            shared,
            ready_tx,
            ready,
            refusals: AcceptRefusals { rx, dropped },
        }
    }

    impl Rig {
        /// An upgrade that holds one permit and takes now as the TCP accept time — it is not spawned (the test chooses the poll moment).
        fn upgrade<S: WsStream>(&self, stream: S) -> impl std::future::Future<Output = ()> {
            upgrade(
                stream,
                "127.0.0.1:5555".parse().unwrap(),
                None,
                self.clock.now(),
                self.permits.try_take().expect("permit"),
                self.shared.clone(),
                self.ready_tx.clone(),
            )
        }

        fn assert_timed_out(&mut self, what: &str) {
            assert!(
                self.ready.try_recv().is_err(),
                "{what}: an upgrade past its deadline sat in the ready queue"
            );
            let report = self
                .refusals
                .try_recv()
                .unwrap_or_else(|| panic!("{what}: no report"));
            assert_eq!(report.kind, RefusalKind::UpgradeTimeout, "{what}");
            assert_eq!(
                self.permits.in_use(),
                0,
                "{what}: the permit was not returned"
            );
        }
    }

    async fn poll_once<F: std::future::Future<Output = ()>>(
        task: &mut std::pin::Pin<Box<F>>,
    ) -> bool {
        std::future::poll_fn(|cx| {
            std::task::Poll::Ready(std::future::Future::poll(task.as_mut(), cx).is_ready())
        })
        .await
    }

    #[tokio::test]
    async fn an_upgrade_first_polled_at_or_after_the_cutoff_is_refused_before_its_handshake() {
        for late in [UPGRADE_TIMEOUT, UPGRADE_TIMEOUT + Duration::from_millis(1)] {
            let mut rig = rig();
            let (mut client, server) = tokio::io::duplex(64 * 1024);
            // The request has already arrived — apart from the deadline, the upgrade finishes on the first poll.
            tokio::io::AsyncWriteExt::write_all(&mut client, UPGRADE_REQUEST)
                .await
                .unwrap();
            let task = rig.upgrade(server);
            // The spawned task never ran before the deadline.
            rig.clock.advance(late);
            task.await;
            rig.assert_timed_out(&format!("first ran {late:?} after accept"));
            let mut answer = Vec::new();
            tokio::io::AsyncReadExt::read_to_end(&mut client, &mut answer)
                .await
                .unwrap();
            assert!(
                answer.is_empty(),
                "wrote the response after the deadline passed"
            );
        }
    }

    #[tokio::test]
    async fn an_upgrade_that_finishes_at_or_after_the_cutoff_is_refused() {
        for late in [UPGRADE_TIMEOUT, UPGRADE_TIMEOUT + Duration::from_millis(1)] {
            let mut rig = rig();
            let (mut client, server) = tokio::io::duplex(64 * 1024);
            let mut task = Box::pin(rig.upgrade(server));
            assert!(!poll_once(&mut task).await, "ended without a request");
            // The deadline and the request stand together — on the next poll the upgrade is polled before the deadline and finishes.
            rig.clock.advance(late);
            tokio::io::AsyncWriteExt::write_all(&mut client, UPGRADE_REQUEST)
                .await
                .unwrap();
            task.await;
            rig.assert_timed_out(&format!("ended {late:?} after accept"));
        }
    }

    // Pair of the test above — finishing just before the deadline is accepted; the auth deadline starts at the finish.
    #[tokio::test]
    async fn an_upgrade_that_finishes_just_before_the_cutoff_is_accepted_stamped_at_completion() {
        let mut rig = rig();
        let (mut client, server) = tokio::io::duplex(64 * 1024);
        let accepted_at = rig.clock.now();
        let mut task = Box::pin(rig.upgrade(server));
        assert!(!poll_once(&mut task).await, "ended without a request");
        let early = UPGRADE_TIMEOUT - Duration::from_millis(1);
        rig.clock.advance(early);
        tokio::io::AsyncWriteExt::write_all(&mut client, UPGRADE_REQUEST)
            .await
            .unwrap();
        task.await;
        let accepted = rig
            .ready
            .try_recv()
            .expect("an upgrade that finished before the deadline");
        assert_eq!(accepted.upgraded_at(), accepted_at + early);
        assert!(rig.refusals.try_recv().is_none());
        assert_eq!(rig.permits.in_use(), 1, "the accepted link holds a permit");
    }

    /// A stream whose reads give the bytes it was handed and whose writes are never accepted — builds an upgrade that stalled while writing the response.
    struct WriteStalled(std::io::Cursor<Vec<u8>>);

    impl tokio::io::AsyncRead for WriteStalled {
        fn poll_read(
            mut self: std::pin::Pin<&mut Self>,
            cx: &mut std::task::Context<'_>,
            buf: &mut tokio::io::ReadBuf<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            std::pin::Pin::new(&mut self.0).poll_read(cx, buf)
        }
    }

    impl tokio::io::AsyncWrite for WriteStalled {
        fn poll_write(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
            _buf: &[u8],
        ) -> std::task::Poll<std::io::Result<usize>> {
            std::task::Poll::Pending
        }

        fn poll_flush(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            std::task::Poll::Pending
        }

        fn poll_shutdown(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            std::task::Poll::Pending
        }
    }

    #[tokio::test]
    async fn a_403_cut_by_the_deadline_is_reported_as_its_screening_with_the_headers() {
        let mut rig = rig();
        let request = b"GET / HTTP/1.1\r\nOrigin: http://evil\r\nConnection: Upgrade\r\n\
            Upgrade: websocket\r\nSec-WebSocket-Version: 13\r\n\
            Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\r\n";
        let mut task = Box::pin(rig.upgrade(WriteStalled(std::io::Cursor::new(request.to_vec()))));
        assert!(!poll_once(&mut task).await, "wrote all of the 403");
        rig.clock.advance(UPGRADE_TIMEOUT);
        task.await;
        let report = rig.refusals.try_recv().expect("report");
        assert_eq!(report.kind, RefusalKind::OriginRejected);
        assert_eq!(report.origin.as_deref(), Some("http://evil"));
        assert!(rig.ready.try_recv().is_err());
        assert_eq!(rig.permits.in_use(), 0);
    }

    // ── Upgrade header decision ──
    fn policy(origins: &[&str], hosts: &[&str]) -> AcceptPolicy {
        AcceptPolicy {
            allowed_origins: origins.iter().map(|s| s.to_string()).collect(),
            allowed_host_names: hosts.iter().map(|s| s.to_string()).collect(),
            upgrade_timeout: Duration::from_secs(1),
            max_pending_upgrades: 1,
            max_message_bytes: 1,
            refusal_queue: 1,
        }
    }

    fn headers(pairs: &[(&str, &[u8])]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (name, value) in pairs {
            map.append(
                header::HeaderName::from_bytes(name.as_bytes()).unwrap(),
                header::HeaderValue::from_bytes(value).unwrap(),
            );
        }
        map
    }

    fn kind(result: Result<Option<String>, Screened>) -> Option<RefusalKind> {
        result.err().map(|s| s.kind)
    }

    #[test]
    fn origin_absent_passes_listed_passes_and_anything_else_is_refused() {
        let p = policy(&["http://localhost:1420"], &["127.0.0.1"]);
        assert_eq!(screen(&p, &headers(&[]), 9000), Ok(None));
        assert_eq!(
            screen(&p, &headers(&[("origin", b"http://localhost:1420")]), 9000),
            Ok(Some("http://localhost:1420".into()))
        );
        for bad in [
            &b"http://evil.example"[..],
            b"http://LOCALHOST:1420",
            b"null",
            b"",
            b"http://localhost:1420/",
        ] {
            assert_eq!(
                kind(screen(&p, &headers(&[("origin", bad)]), 9000)),
                Some(RefusalKind::OriginRejected),
                "{}",
                String::from_utf8_lossy(bad)
            );
        }
        // Two or more · not text — cut even if the value is in the list.
        let twice = headers(&[
            ("origin", b"http://localhost:1420"),
            ("origin", b"http://localhost:1420"),
        ]);
        assert_eq!(
            kind(screen(&p, &twice, 9000)),
            Some(RefusalKind::OriginRejected)
        );
        let bytes = headers(&[("origin", b"http://localhost:1420\xff")]);
        assert_eq!(
            kind(screen(&p, &bytes, 9000)),
            Some(RefusalKind::OriginRejected)
        );
    }

    #[test]
    fn an_empty_origin_list_refuses_every_upgrade_that_carries_one() {
        // The release value of D21 ① — if there is no Origin, it is accepted as is.
        let p = policy(&[], &["127.0.0.1"]);
        assert_eq!(screen(&p, &headers(&[]), 9000), Ok(None));
        assert_eq!(
            kind(screen(
                &p,
                &headers(&[("origin", b"tauri://localhost")]),
                9000
            )),
            Some(RefusalKind::OriginRejected)
        );
    }

    #[test]
    fn host_must_name_an_allowed_host_on_the_bound_port_or_be_absent() {
        let p = policy(&[], &["127.0.0.1", "localhost", "[::1]"]);
        for ok in [
            &b"127.0.0.1:9000"[..],
            b"localhost:9000",
            b"LocalHost:9000",
            b"[::1]:9000",
        ] {
            assert_eq!(
                screen(&p, &headers(&[("host", ok)]), 9000),
                Ok(None),
                "{}",
                String::from_utf8_lossy(ok)
            );
        }
        assert_eq!(
            screen(&p, &headers(&[]), 9000),
            Ok(None),
            "no Host is accepted (D9)"
        );
        for bad in [
            &b"evil.example:9000"[..],
            b"127.0.0.1:9001",
            b"127.0.0.1",
            b"127.0.0.1:",
            b"127.0.0.1:+9000",
            b"[::1]",
            b"127.0.0.1.evil.example:9000",
        ] {
            assert_eq!(
                kind(screen(&p, &headers(&[("host", bad)]), 9000)),
                Some(RefusalKind::HostRejected),
                "{}",
                String::from_utf8_lossy(bad)
            );
        }
        let twice = headers(&[("host", b"127.0.0.1:9000"), ("host", b"127.0.0.1:9000")]);
        assert_eq!(
            kind(screen(&p, &twice, 9000)),
            Some(RefusalKind::HostRejected)
        );
        // A missing port is 80 — accepted if the listener opened on 80.
        assert_eq!(
            screen(&p, &headers(&[("host", b"localhost")]), 80),
            Ok(None)
        );
    }

    #[test]
    fn a_refusal_carries_both_headers_it_saw() {
        let p = policy(&["http://ok"], &["127.0.0.1"]);
        let seen = screen(
            &p,
            &headers(&[("origin", b"http://ok"), ("host", b"evil:9000")]),
            9000,
        )
        .expect_err("Host is outside");
        assert_eq!(
            seen,
            Screened {
                kind: RefusalKind::HostRejected,
                origin: Some("http://ok".into()),
                host: Some("evil:9000".into()),
            }
        );
    }

    // ── Refusal report ──
    #[tokio::test]
    async fn a_full_refusal_queue_drops_new_reports_and_counts_them() {
        let (tx, rx) = mpsc::channel(1);
        let dropped = Arc::new(AtomicU64::new(0));
        let reporter = Reporter {
            tx,
            dropped: dropped.clone(),
        };
        let mut refusals = AcceptRefusals { rx, dropped };
        let remote: SocketAddr = "127.0.0.1:5555".parse().unwrap();
        // Text built by the peer is carried masked — line breaks · invisible characters are removed.
        reporter.report(
            RefusalKind::OriginRejected,
            Some(remote),
            Some("http://evil\n\u{202E}x"),
            None,
        );
        reporter.report(RefusalKind::SlotsFull, Some(remote), None, None);
        reporter.report(RefusalKind::SlotsFull, Some(remote), None, None);
        assert_eq!(refusals.dropped(), 2, "count dropped from the full queue");
        let first = refusals.try_recv().expect("first report");
        assert_eq!(
            first.kind,
            RefusalKind::OriginRejected,
            "the first to come remains"
        );
        assert_eq!(first.remote.as_deref(), Some("127.0.0.1:5555"));
        assert_eq!(first.origin.as_deref(), Some("http://evilx"));
        assert!(refusals.try_recv().is_none());
    }
}
