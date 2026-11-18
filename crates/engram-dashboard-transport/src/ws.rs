//! Real-socket WS adapter — behind feature `ws`. ★The only real transport implementation in this crate★.
//!
//! ## What this file owns
//!
//! The dialing side ([`WsDialer`] = a [`Dialer`]), the receiving side ([`WsListener`] = a [`Listener`], its accept policy [`AcceptPolicy`] ·
//! refusal report [`AcceptRefusals`]), and splitting one open WS stream into the [`LinkTx`]/[`LinkRx`] ends. ★Both directions live together★,
//! so this crate **attaches to itself** and measures what memory cannot (ADR-0177 decision 8).
//!
//! ★The stream wrapping is crate-internal★ — no function that splits a stream into two ends exists outside the crate, so the only path that
//! sends a real WS stream out as an accepted link ([`Accepted`]) is [`WsListener`] (TRD 3 §3-5 · §5 17). ★Not a guarantee that consumers cannot
//! skip the check★ — [`Accepted::new`] is public for other transports, so a link built from hand-written [`LinkTx`]/[`LinkRx`] also lands on the
//! registry; code review, not the compiler, catches that bypass.
//!
//! ## What the receiving side guards (TRD 3 §5 1 · 2 · 12 · 14 · 15 · 16 · 18)
//!
//! The consumer gives every policy value through [`AcceptPolicy`]; this file chose only the two that run the loop — the kernel accept queue
//! length (`LISTEN_BACKLOG`) and the pause after an accept error ([`ACCEPT_ERROR_PAUSE`]).
//! - Opens on loopback only — a non-loopback bound address is refused before listen.
//! - TCP accept runs on one task and each WS upgrade on a separate spawned task, so a peer that never finishes its upgrade does not block other
//!   accepts. The upgrade deadline is measured from the TCP accept time (task wait counts; an upgrade finishing past it is not accepted).
//! - The upgrade permit ([`crate::PermitPool`]) is taken at TCP accept and travels in [`Accepted`] — links mid-upgrade, in the ready queue, and
//!   taken but not yet on the registry count as one total. If all are used the new TCP is closed right after accept.
//! - Origin and Host headers are checked against the policy and cut with 403 if outside it.
//! - The size cap of one message and one frame is applied with the same value on the receiving and outgoing side.
//! - What was cut is reported to [`AcceptRefusals`] with its branch and a masked remote address — the `Err` of [`Listener::accept`] means only
//!   that the listener has ended.
//!
//! ## What it guarantees / what it leaves to the consumer
//!
//! Guarantees: the [`Frame`]↔`Message` round trip · the close code and message **in both directions** (so 「connected but rejected」 crosses as
//! [`LinkRead::Closed`] and ADR-0180 decision 5 holds in code) · transport-layer Ping/Pong raised as [`Frame::Keepalive`]. ★The **translation
//! table** is asserted by this file's unit tests★ ([`to_message`]/[`from_message`] are pure); the real-socket round trip and whether the close
//! frame really goes out belong to the suites in `tests/`, and nobody yet measures the keepalive round trip (「Known limits」).
//!
//! Left to the consumer: address syntax (handed to `tokio-tungstenite` as is) · reconnect, deadline, keepalive interval (up to [`crate::Policy`]
//! and the supervisor — ★the dialing adapter applies no deadline of its own★ ([`Dialer`] rustdoc); ★only the receiving side's upgrade deadline is
//! an exception★, a phase before the supervisor exists, in [`AcceptPolicy`]) · auth and version negotiation ([`crate::Handshake`]) · the accept
//! loop and peer registry (the consumer calls [`Listener::accept`] and [`crate::Registry::attach`]).
//!
//! ## Known limits (do not read as guarantees)
//!
//! - ★The refusal when upgrade permits are all used has no HTTP response★ — TCP is closed right after accept, so the dialer sees a cut during
//!   upgrade. Anyone on the same PC can fill the permits — loopback is shared by all OS users (TRD 3 §5 14 · R11).
//! - ★Refusal reports can be lost★ — when the [`AcceptRefusals`] queue ([`AcceptPolicy::refusal_queue`]) is full new reports are dropped and only
//!   [`AcceptRefusals::dropped`] rises; an accept loop waiting on the consumer during a refusal flood would itself be a denial of service.
//! - ★A TCP accept error is not terminal★ — it is reported and accepting resumes. Per-connection errors (`ConnectionRefused` · `ConnectionAborted`
//!   · `ConnectionReset`) take the next accept at once; other errors (e.g. `EMFILE`) pause [`ACCEPT_ERROR_PAUSE`] because they leave the connection
//!   in the kernel queue and recur immediately. Same branching as axum 0.8.9's accept loop (read from source, not measured —
//!   `handle_accept_error` in `serve/listener.rs`). So [`Listener::accept`] returns `Err` only when the accept task has ended (a panic).
//! - ★TLS is not turned on★ — only `tokio-tungstenite` default features, so `wss://` fails at the dial step.
//! - ★An outgoing close **message** is silently truncated when it overflows★ — cap = the smaller of 123 bytes (125-byte control payload minus 2
//!   for the code, RFC 6455 §5.5) and that link's size cap − 2; [`LinkTx::close`] truncates. ★No ellipsis and no event★ — the consumer cannot know
//!   (do not put decision material in the message). The code goes out unchanged, but with a size cap under 2 bytes even the code cannot be carried
//!   and an empty close goes out. Canonical: `close_reason_room` and the `MAX_CLOSE_REASON_BYTES` rustdoc.
//!
//! ## Link properties (facts of the outside world)
//!
//! - ★A write really stalls when the kernel send buffer fills★ — `SinkExt::send` calls tungstenite's `flush`, which turns `WouldBlock` into
//!   `Pending` (`Sink::poll_flush` → `cvt(s.flush())` in tokio-tungstenite 0.26.2), so for a peer that does not read the supervisor's
//!   `write_deadline` cuts it — what `tests/ws_write_deadline.rs` measures.
//! - ★But **one large write** does not stall★ — **measured** (2026-09-07, Windows 11 loopback): one 16 MiB frame (4 and 8 MiB the same) went
//!   straight in in 28 ms to a non-reading peer, also with `SO_SNDBUF`/`SO_RCVBUF` set to 4 KiB; repeating 1 MiB frames swallows 2 MiB then
//!   stalls on the third. **Interpretation** (not observed): Windows' dynamic send buffering swallows one large send whole; the numbers stand
//!   even if that is wrong. ★Backpressure is triggered not by frame size but by **accumulation**★.
//! - ★The frame of a truncated write can go out late★ — from reading source (futures-util 0.3.32 `sink/feed.rs:38-40` → `sink/send.rs:37`;
//!   `SplitSink` slot `stream/stream/split.rs:114`; tokio-tungstenite 0.26.2 `lib.rs:340-347`): `send` hands over the item **then** waits in
//!   `flush`, and the handed-over item stays in a `SplitSink` slot or the tungstenite write buffer; whatever is next written to the same end
//!   pushes it out first (`split.rs:104-109`). The supervisor cuts a write by control or deadline and then sends a close with [`LinkTx::close`],
//!   so **the truncated frame can go out ahead of that close** and the supervisor's loss count can be inflated ([`crate::Direction::Outbound`]).
//!   ★The in-memory harness does not imitate this★ — a blocked write stalls **before** handing over, so a cut leaves nothing (`testing::MemTx`).
//! - ★tungstenite measures only the size of what comes in, not what goes out★ — from source (0.26.2: only the reading side checks
//!   `max_frame_size` / `max_message_size`; `WebSocketContext::write` has no size check). Exceeding it is a **receiving-side read error** that
//!   cuts the link (measured — one 32 MiB frame under defaults: `Space limit exceeded: Message too long: 33554432 > 16777216`). This adapter
//!   applies one consumer-supplied value to both directions ([`WsDialer::new`] · [`AcceptPolicy::max_message_bytes`]); the receiving side has
//!   `max_message_size` = `max_frame_size` = that value. ★So [`LinkTx::send`] **returns an error** for a frame over it★ — if only the receiver
//!   enforced the cap, **an innocent peer burns its own reconnect budget**. ★This rests on 「the peer uses the same value」★ — a peer with a
//!   smaller cap is still cut. ★It is a value, not a panic★ — a panic in the supervisor task ends the whole process in a production process
//!   with a panic hook (ADR-0292), and in one without, nobody holds its `JoinHandle`, so the emitted state freezes at `Live`. The supervisor turns
//!   the error into `LinkLost` (disconnect event plus reconnect); release and debug behave the same.
//! - ★**But no diagnostic is left on this branch**★ — the error message is dropped when the supervisor turns `Ok(Err(_))` into
//!   `LinkLost(WriteFailed)` (`Supervisor::write` in `peer.rs`), and [`crate::TransportEvent::Disconnected`] has no message field (`lib.rs`
//!   「a disconnect during operation does not carry a reason message」). The oversize frame is counted as an outgoing loss (notification or
//!   fan-out) or woken by `FailPending` (request), neither saying 「too big」: the consumer sees a **`WriteFailed` disconnect with no reason**.
//!   Diagnosing it needs a message field on the disconnect event first.
//! - ★**When** the automatic Pong for an incoming Ping goes out★ — from tungstenite 0.26.2 source (not measured): the receiver puts the Pong into
//!   `additional_send` (`protocol/mod.rs:651`) and **the read loop flushes that pending write at the top of every iteration**
//!   (`protocol/mod.rs:452`), so it goes out in **both reading and writing**. ★Reading does not stop when the send buffer is full★ — it swallows the
//!   flush's `WouldBlock`, sets `unflushed_additional` and keeps reading ("If blocked continue reading, but try again later"), and
//!   tokio-tungstenite's waker proxy (`WakerProxy` in `compat.rs`) hooks the reading waker onto the write axis, so a writable socket wakes the
//!   reading task to flush again. Congestion therefore shows as 「our Pong is late」, not 「we cannot read」 — the supervisor's last-receive time does
//!   not stop. A late Pong beyond the peer's patience shows as 「the peer cuts us with silence」. ★The only basis for being bounded is
//!   `write_deadline`★ — applied to **every write** (`peer.rs` invariant), a stuck write is cut and the link dropped. ★Do not cite
//!   `Policy::check`★: its `write_deadline < ping_interval < idle_timeout` is checked **only in debug builds** (production callers are two
//!   `debug_assert_eq!` in `registry.rs`), and ★it cannot establish **the peer's** patience★ anyway (the peer's `idle_timeout` is its own); all
//!   that holds is 「our writes do not hang forever」.
//! - ★The keepalive **round trip** is unmeasured on a real socket★ — unit tests assert the translation table in both directions, but none measures
//!   a real Ping out and a real Pong back as [`Frame::Keepalive`]; the crate's `ping_interval` far exceeds any real-socket test. If it goes wrong
//!   the symptom is the one above.
//! - ★`Message::Frame` does not come up from real-socket receiving★ (tungstenite docs), so that branch is skipped, not translated.
//! - ★The cancel safety of [`LinkRx::recv`] is not load-bearing anywhere in this crate★ — ★not because 「it is never cancelled」★: the
//!   operating-phase read is dropped mid-poll on every link drop (`drop_link` in `peer.rs` `abort()`s the reading task). It is not load-bearing
//!   because **every place that cancels drops both ends whole right after** (`frames_rx` and `rx_raw` released together; the handshake's deadline
//!   and peer-closed branches return `Err` and drop the link in `handshake_once`, and control winning drops the handshake future with both ends in
//!   `shake_hands`). ★Hence no 「unverified」 label★ — it is a debt never taken on; the label would send a session measuring a nonexistent risk.
//!   **A new call that reads the same link again after cancelling voids this sentence.**

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
    LinkTx, Listener, PermitPool,
};
use crate::mask::mask_reason;

/// The maximum bytes a close message can carry — **not a character count**.
///
/// ★External fact★ — RFC 6455 §5.5 fixes the control frame payload at 125 bytes and a close frame spends 2 on the code. tungstenite 0.26.2 **does
/// not check the outgoing side** (`Frame::close` concatenates code and message) but **the receiver cuts anything over 125 with
/// `ControlFrameTooBig`** (`protocol/mod.rs:640`), so a long message makes 「connected but rejected」 reach the peer as a **read error**, collapsing the
/// rejection decision and burning the reconnect budget — [`WsTx::close`] truncates first.
const MAX_CLOSE_REASON_BYTES: usize = 123;

/// Kernel accept queue length — the same value tokio `TcpListener::bind` uses. ★Unrelated to the upgrade permits★ (a connection in this queue is not yet accepted by us).
const LISTEN_BACKLOG: u32 = 1024;

/// The pause before accepting again after a TCP accept error (reason and pausing branches: file header 「Known limits」). ★Not a security value —
/// it only keeps the loop from spinning, so it is not an [`AcceptPolicy`] field★.
///
/// ★Uses tokio's real clock, not the listener's [`Clock`]★ — it applies only to real-socket accept errors; following a hand-advanced clock would
/// stop the accept loop until the test advances it.
pub const ACCEPT_ERROR_PAUSE: Duration = Duration::from_millis(100);

/// Whether to pause for [`ACCEPT_ERROR_PAUSE`] after a TCP accept error. ★Per-connection errors do not pause★ — the peer cut before we accepted, so
/// the next accept is a different connection, and pausing would let a peer that cuts right after opening delay accepting.
fn pauses_after_accept_error(kind: io::ErrorKind) -> bool {
    !matches!(
        kind,
        io::ErrorKind::ConnectionRefused
            | io::ErrorKind::ConnectionAborted
            | io::ErrorKind::ConnectionReset
    )
}

/// Bytes usable for a close message on a link whose size cap is `max_payload` — the smaller of [`MAX_CLOSE_REASON_BYTES`] and `max_payload` − 2
/// (the code's share). `None` = `max_payload` is under 2, so even the code cannot be carried.
///
/// ★Why it also truncates by `max_payload`★ (external fact) — tungstenite 0.26.2 applies the receiving `max_frame_size` as soon as it reads the frame
/// header, control frames included (`protocol/frame/mod.rs:181`); truncating only to 125 would let one close frame cut the peer's read with
/// `MessageTooLong` on a link capped below that.
fn close_reason_room(max_payload: usize) -> Option<usize> {
    max_payload
        .checked_sub(2)
        .map(|room| room.min(MAX_CLOSE_REASON_BYTES))
}

/// Truncates the close message to within `room` bytes ([`close_reason_room`]). ★Cut at a character boundary★ — a byte cut gives broken UTF-8, which
/// fails **differently** from exceeding the length (tungstenite's `Utf8Bytes` guarantees UTF-8).
fn clamp_close_reason(reason: &str, room: usize) -> &str {
    if reason.len() <= room {
        return reason;
    }
    &reason[..reason.floor_char_boundary(room)]
}

/// The receiving side's size cap — the same value for one message and one frame (TRD 3 §5 15); other fields are tungstenite defaults.
fn ws_config(max_message_bytes: usize) -> WebSocketConfig {
    WebSocketConfig::default()
        .max_message_size(Some(max_message_bytes))
        .max_frame_size(Some(max_message_bytes))
}

/// The base property needed at one end of a link; both `MaybeTlsStream<TcpStream>` (dialing) and `TcpStream` (receiving) satisfy it, so both
/// directions share one [`LinkTx`]/[`LinkRx`] implementation.
pub(crate) trait WsStream: AsyncRead + AsyncWrite + Unpin + Send + 'static {}

impl<S> WsStream for S where S: AsyncRead + AsyncWrite + Unpin + Send + 'static {}

// ── Dialing side ──────────────────────────────────────────────────────────────

/// A [`Dialer`] that opens a real WS link. [`Address`] is passed to `tokio-tungstenite` as is (`ws://host:port/path`); a syntax violation arrives as
/// [`LinkError`] (the crate does not interpret the address).
///
/// ★Every failure is [`DialFailure::Unreachable`] — it never emits [`DialFailure::Refused`]★. An HTTP rejection at the upgrade (a status other than
/// 101, e.g. the Origin check's 403) is the same: it has no close code, and mapping a status number to terminal or redial is a policy this adapter
/// does not know (invented, a transient fault becomes a give-up with no budget). A consumer that knows the terminal outcome before dialing returns
/// `Refused` from its own [`Dialer`] wrapping this one (TRD 3 §3-0 ⑤).
pub struct WsDialer {
    max_message_bytes: usize,
}

impl WsDialer {
    /// Byte cap of one message and one frame — both the receiving size cap and the boundary at which an outgoing frame is returned as an error (file
    /// header 「Link properties」). Give the same value as the peer listener's [`AcceptPolicy::max_message_bytes`].
    pub fn new(max_message_bytes: usize) -> Self {
        Self { max_message_bytes }
    }
}

impl Dialer for WsDialer {
    fn dial(
        &self,
        addr: &Address,
    ) -> BoxFuture<'_, Result<(Box<dyn LinkTx>, Box<dyn LinkRx>), DialFailure>> {
        // ★The link is created inside the future★ — the [`Dialer`] contract (cancelled, nothing may be left behind).
        // ★The address moves into an owned String — enforced by the signature★: the `'_` in the return type is tied to `&self`, not `addr`, so the
        //   future cannot borrow `addr`.
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
            Ok(split_link(ws, max))
        })
    }
}

// ── Receiving side ────────────────────────────────────────────────────────────

/// The accept policy of [`WsListener`] (TRD 3 §5 17 · C1).
///
/// ★No `Default`★ — crate-chosen values would be a security policy decided without the consumer's knowledge. ★No allow-everything value★ — checks
/// cannot be turned off, so tests run with real values in the same fields as production. ★Not `#[non_exhaustive]`★ — a new field makes the consumer's
/// construction site a compile error that forces a choice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcceptPolicy {
    /// Values accepted during an upgrade that carries an Origin header — compared byte for byte (TRD 3 §5 2). ★If empty, every upgrade carrying an
    /// Origin is cut with 403★ (the release value of D21 ①). ★An upgrade without an Origin is always accepted★ — clients outside a browser send none,
    /// and this check blocks browser-page requests (authentication belongs to the handshake). Two or more headers, or non-text, is cut.
    pub allowed_origins: Vec<String>,
    /// Values accepted for the name part of the Host header (before the port — case-insensitive), e.g. `127.0.0.1` · `localhost` (TRD 3 §5 16 — DNS
    /// rebinding). The port must be the one this listener bound (no port = 80). ★An upgrade without a Host is accepted★ (D9); two or more headers or
    /// non-text is cut; if empty, every upgrade carrying a Host is cut. ★An IPv6 name is written with brackets★ (`[::1]`) — the header carries it that way
    /// and the name is compared as is; a name with a colon outside the brackets (`::1` · `localhost:8080`) matches no header, so [`WsListener::bind`]
    /// returns `Err`.
    pub allowed_host_names: Vec<String>,
    /// Deadline from TCP accept to the end of the WS upgrade (TRD 3 §5 12) — past it that TCP is closed. ★The accepted peer's auth deadline is measured
    /// from where this one ends ([`Accepted::upgraded_at`])★ — the two add up.
    pub upgrade_timeout: Duration,
    /// Number of upgrade permits (TRD 3 §5 14) — taken at TCP accept, released when the registered peer enters the operating phase (or [`Accepted`] is
    /// dropped). ★The ready queue is counted by these permits too★ — its cap is this value, not counted separately. All used: the new TCP is closed
    /// right after accept. 0 = accept nothing.
    pub max_pending_upgrades: usize,
    /// Byte cap of one message and one frame (TRD 3 §5 15) — both the accepted link's receiving cap and the boundary at which an outgoing frame is
    /// returned as an error (file header 「Link properties」). Give the same value to the dialer's [`WsDialer::new`].
    pub max_message_bytes: usize,
    /// Size of the refusal report queue (TRD 3 §5 18) — when full, new reports are dropped and [`AcceptRefusals::dropped`] rises. 0 counts as 1.
    pub refusal_queue: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum RefusalKind {
    /// Upgrade permits all used — TCP closed right after accept (no HTTP response).
    SlotsFull,
    UpgradeTimeout,
    OriginRejected,
    HostRejected,
    /// The upgrade failed outside the branches above — not a WS request · peer cut during the upgrade, etc.
    UpgradeFailed,
    /// The TCP accept itself failed — there may be no peer, so the remote address is empty.
    AcceptError,
}

/// The report of one peer that was cut (TRD 3 §5 18). ★The text fields are already masked with [`crate::mask_reason`]★ — printable as is. ★The cut
/// happens before the auth frame is read, so no token can ride★.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct AcceptRefusal {
    pub kind: RefusalKind,
    /// Peer end address (`ip:port`); `None` for [`RefusalKind::AcceptError`].
    pub remote: Option<String>,
    pub origin: Option<String>,
    pub host: Option<String>,
}

/// The refusal report receiver returned with the listener by [`WsListener::bind`] — a reporting place outside the [`Listener`] trait (whose `Err` means
/// only that it ended). ★You may drop it★ — later reports are discarded and the listener keeps running. ★Unread, the queue fills and new reports are
/// lost first★ ([`AcceptRefusals::dropped`]). Dropping the listener hands out the remaining reports and then ends with `None`.
pub struct AcceptRefusals {
    rx: mpsc::Receiver<AcceptRefusal>,
    dropped: Arc<AtomicU64>,
}

impl AcceptRefusals {
    /// Waits for the next report. ★Cancel safe★.
    pub async fn recv(&mut self) -> Option<AcceptRefusal> {
        self.rx.recv().await
    }

    pub fn try_recv(&mut self) -> Option<AcceptRefusal> {
        self.rx.try_recv().ok()
    }

    /// Cumulative reports dropped because the queue was full — never decreases.
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Acquire)
    }
}

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
        // ★Does not wait★ — an accept loop or upgrade task waiting on the consumer would stall accepting during a refusal flood.
        if self.tx.try_send(refusal).is_err() {
            self.dropped.fetch_add(1, Ordering::AcqRel);
        }
    }
}

/// A [`Listener`] that accepts real WS links. TCP accept runs on one listener task and each upgrade on a spawned task; a finished upgrade waits in the
/// ready queue for [`Listener::accept`] (still holding an upgrade permit). ★Dropping it reaps those tasks and closes the ready-queue links★ — the dialer
/// sees a link loss.
pub struct WsListener {
    local: SocketAddr,
    ready: mpsc::Receiver<Accepted>,
    permits: PermitPool,
    driver: JoinHandle<()>,
}

impl WsListener {
    /// Opens on `addr` and starts accepting — returns the listener and its refusal receiver. Port 0 lets the kernel pick ([`WsListener::local_addr`]).
    ///
    /// ★Opens on loopback only★ (TRD 3 §5 1 · C8) — a non-loopback bound address is `Err` before listen; plain `ws://` is not opened outside (remote
    /// goes with TLS — TRD 3 §4). Also `Err` if [`AcceptPolicy::allowed_host_names`] has a name with a colon outside the brackets (it would quietly
    /// accept nothing).
    ///
    /// ★`clock` must be the same clock as the registry the accepted link is put on★ — it stamps [`Accepted::upgraded_at`], from which the registry
    /// measures the auth deadline ([`crate::Registry::attach_with_policy`] asserts the match in debug builds); the upgrade deadline uses it too.
    ///
    /// Call inside a tokio runtime — outside one it panics (it spawns the accept task).
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
        // ★Check before listen★ — refused after listening, outside connections would already be in the kernel accept queue.
        ensure_loopback(local)?;
        let tcp = socket
            .listen(LISTEN_BACKLOG)
            .map_err(|e| LinkError::new(format!("ws listen {local}: {e}")))?;

        let permits = PermitPool::new(policy.max_pending_upgrades);
        // ★The ready queue never fills★ — each queued link holds a permit, so queue length ≤ permits held ≤ this capacity.
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

    /// Upgrade permits currently held — links mid-upgrade, in the ready queue, and taken but not yet released. ★The permit pool itself is not handed
    /// out★ — it could be used to hold permits and block this listener's accepting.
    pub fn permits_in_use(&self) -> usize {
        self.permits.in_use()
    }

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
        self.driver.abort();
    }
}

fn ensure_loopback(local: SocketAddr) -> Result<(), LinkError> {
    if local.ip().is_loopback() {
        Ok(())
    } else {
        Err(LinkError::new(format!(
            "ws bind {local}: not loopback — a plaintext ws:// listener is opened only on loopback"
        )))
    }
}

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

/// The TCP accept loop. ★It does not wait for the upgrade★ — one task per connection so a slow peer does not block the next accept (TRD 3 §5 12);
/// `upgrades` holds the spawned tasks, so reaping this task reaps them.
async fn drive(tcp: TcpListener, shared: Arc<Shared>, ready: mpsc::Sender<Accepted>) {
    let mut upgrades = JoinSet::new();
    loop {
        tokio::select! {
            accepted = tcp.accept() => match accepted {
                Ok((stream, remote)) => match shared.permits.try_take() {
                    Some(permit) => {
                        // ★Stamp the deadline's reference before spawning★ — stamped inside the task, the delay until it first runs would stay outside the
                        //   deadline (TRD 3 §5 12 — from TCP accept).
                        let accepted_at = shared.clock.now();
                        upgrades.spawn(upgrade(
                            stream,
                            remote,
                            accepted_at,
                            permit,
                            shared.clone(),
                            ready.clone(),
                        ));
                    }
                    None => {
                        // The overflow closes **the new one** (TRD 3 §5 14); a peer already holding a permit is untouched.
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
            Some(_) = upgrades.join_next(), if !upgrades.is_empty() => {}
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
struct Screened {
    kind: RefusalKind,
    origin: Option<String>,
    host: Option<String>,
}

/// The upgrade of one link — finished before the deadline (`accepted_at` + [`AcceptPolicy::upgrade_timeout`]) it goes into the ready queue, otherwise it
/// is closed and reported. `accepted_at` = TCP accept time (`shared.clock`); `permit` goes out with the accepted link or is dropped here.
///
/// ★The deadline is checked in three places★ — before starting (the spawned task ran late), while waiting, and after finishing. The after-finish check
/// is separate because when finishing and the deadline stand in one poll, [`with_deadline`] polls the upgrade side first and it wins; it counts as past
/// the deadline from **that very moment**.
async fn upgrade<S: WsStream>(
    stream: S,
    remote: SocketAddr,
    accepted_at: Instant,
    permit: ConnPermit,
    shared: Arc<Shared>,
    ready: mpsc::Sender<Accepted>,
) {
    let policy = &shared.policy;
    let report = |kind, origin: Option<&str>, host: Option<&str>| {
        shared.reporter.report(kind, Some(remote), origin, host)
    };
    // Time left until the deadline — `None` = reached or passed. ★Subtract elapsed time rather than adding the deadline to the time★ — `Instant + Duration`
    //   panics on overflow and the deadline field has no cap.
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
            // ★If cut by the header decision mid-403, report that branch★ — reported as the deadline, the cut reason and header seen disappear (the peer did
            //   not read the 403 and the write stalled).
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
    // ★Stamp the upgrade-finish moment with the registry's clock★ — the auth deadline starts here (TRD 3 §5 3).
    let upgraded_at = shared.clock.now();
    if remaining_at(upgraded_at).is_none() {
        // Finished past the deadline — even a finished upgrade is not accepted (this function's rustdoc). Link and permit are dropped here.
        report(RefusalKind::UpgradeTimeout, None, None);
        return;
    }
    let origin = match verdict {
        Some(Ok(origin)) => origin,
        _ => None,
    };
    let (tx, rx) = split_link(ws, policy.max_message_bytes);
    let info = LinkInfo {
        remote: Some(remote.to_string()),
        origin,
        peer_key: None,
    };
    let accepted = Accepted::new(tx, rx, info, upgraded_at, permit);
    match ready.try_send(accepted) {
        Ok(()) => {}
        // Unreachable (the capacity comment in `WsListener::bind`) — if reached, treat as permits full.
        Err(mpsc::error::TrySendError::Full(_)) => report(RefusalKind::SlotsFull, None, None),
        Err(mpsc::error::TrySendError::Closed(_)) => {}
    }
}

/// The tungstenite header callback — checks upgrade headers against the policy and leaves the decision in `verdict` (the Origin to accept, or the cutting
/// branch). A cut returns 403.
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

/// The 403 response. ★The body does not say which check caught it★ — no reason to tell the peer the policy; the refusal report carries the branch.
fn forbidden() -> ErrorResponse {
    let mut response = ErrorResponse::new(None);
    *response.status_mut() = StatusCode::FORBIDDEN;
    response
}

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
    /// Two or more, or non-text bytes — cut without deciding; the real value (lossy conversion) is for reporting.
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

/// Whether the Host header `name[:port]` matches the accepted names · this listener's port. In `[::1]:port` form only the colon after the bracket is
/// the port; digits only (signs · blanks rejected); 80 if absent.
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

/// Splits an open stream into the two ends of a link. `max_payload` = outgoing frame cap (file header 「Link properties」).
///
/// ★Crate-internal only★ (TRD 3 §3-5 · §5 17) — opened outside, anyone could build a link from their own accept and skip every [`WsListener`] check.
///
/// ★The two ends share the stream through a `BiLock`★ so different tasks may hold them (the supervisor hands the reading side to a dedicated task);
/// a side returning `Pending` releases the lock. ★The socket closes only when **both** ends are dropped★ (the `BiLock` releases the inner stream with
/// the last end) — the caller must guarantee that, and the seam carries that duty ([`LinkTx`] rustdoc).
pub(crate) fn split_link<S: WsStream>(
    ws: WebSocketStream<S>,
    max_payload: usize,
) -> (Box<dyn LinkTx>, Box<dyn LinkRx>) {
    let (sink, stream) = ws.split();
    (
        Box::new(WsTx { sink, max_payload }),
        Box::new(WsRx {
            stream,
            failed: false,
        }),
    )
}

// ── The two link ends ─────────────────────────────────────────────────────────

struct WsTx<S: WsStream> {
    sink: SplitSink<WebSocketStream<S>, Message>,
    /// Cap of the outgoing frame payload — the same value as the size cap received by the side that opened this link ([`split_link`]).
    max_payload: usize,
}

impl<S: WsStream> LinkTx for WsTx<S> {
    fn send(&mut self, frame: Frame) -> BoxFuture<'_, Result<(), LinkError>> {
        Box::pin(async move {
            // ★A frame over the cap fails **as a value**★ — otherwise only the receiver enforces the cap (file header 「tungstenite measures only the size of
            //   what comes in」), the link is cut and **an innocent peer burns its own reconnect budget**. ★Do not go back to panicking★ — the
            //   `debug_assert` version fired inside the supervisor task, nobody holds its `JoinHandle` (`Unstarted::start`), so **the panic was unobservable**:
            //   the state watch's sender dropped, the emitted state froze at `Live`, no disconnect or give-up event came, and the reading task fell away holding
            //   half the link — in a production process with a panic hook the whole process ends instead (ADR-0292). As a value the supervisor turns it into
            //   `LinkLost` (`Supervisor::write`), leaving a disconnect event and reconnect. ★But **the message does not remain**★ — that conversion drops the
            //   `LinkError` message and `Disconnected` has no message field (file header).
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
                // ★If the cap cannot even carry the code, send an empty close★ — read as a close with no code; carrying the code would exceed the cap and cut the peer's read.
                None => Message::Close(None),
            };
            // Even on failure there is nowhere to report ([`LinkTx::close`] contract); an unsent close frame leaves the peer reading it as cut without a word (the limit in `frame.rs`).
            let _ = self.sink.send(message).await;
            let _ = self.sink.close().await;
        })
    }
}

struct WsRx<S: WsStream> {
    stream: SplitStream<WebSocketStream<S>>,
    /// Reading has already failed on this link. ★Keeps [`LinkRead::Closed`] meaning 「the peer said it closed」★ (see [`WsRx::recv`]).
    failed: bool,
}

impl<S: WsStream> LinkRx for WsRx<S> {
    fn recv(&mut self) -> BoxFuture<'_, Result<LinkRead, LinkError>> {
        Box::pin(async move {
            // ★A read that failed once keeps failing★ — only then does the [`LinkRead::Closed`] contract (「the peer **said** it closed」) hold. Otherwise: the
            //   first read of a socket dropped without a word is `Protocol(ResetWithoutClosingHandshake)`, but tungstenite then sets its own `ended` marker and
            //   **the next read gives `Ready(None)`** (`lib.rs:303` in 0.26.2); passed along, a close nobody spoke is reported and a consumer branching on it
            //   swaps 「rejected / left politely」 for 「cut off」.
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
                    // ★A dried-up stream is **not** 「cut without a word」★ — it arrives only after a close frame was already read (external fact:
                    //   tokio-tungstenite 0.26.2 turns only `ConnectionClosed`/`AlreadyClosed` into `Ready(None)` at `lib.rs:303`, raised only in
                    //   `ClosedByPeer`/`CloseAcknowledged` at `protocol/mod.rs:706`). A peer that vanished without a word is
                    //   `Protocol(ResetWithoutClosingHandshake)` and comes through the `Err` branch — the [`LinkRead::Closed`] rustdoc carries that contract.
                    None => return Ok(LinkRead::Closed(None)),
                };
                if let Some(read) = from_message(msg) {
                    return Ok(read);
                }
            }
        })
    }
}

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
        Message::Frame(_) => None,
    }
}

fn to_message(frame: Frame) -> Message {
    match frame {
        Frame::Text(text) => Message::Text(text.into()),
        Frame::Binary(bytes) => Message::Binary(bytes.into()),
        // ★Same as [`LinkTx::ping`]★ — keepalive is transport-layer vocabulary, so the payload is not redesigned (reason in the [`LinkTx::ping`] rustdoc).
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
        // ★Keepalive goes out as a Ping with an empty payload★ — reason in the [`to_message`] comment.
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
        // ★Pong and Ping both come up as the same thing★ — only arrival is the signal (`frame.rs`).
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
        // ★4-byte characters are why this test exists★ — with 3-byte characters alone 123 is a multiple of 3, so the mutant 「delete the boundary alignment and
        //   cut at 123」 passes (measured 2026-09-07: all four old inputs green). With 4-byte characters 123 is not a boundary and that mutant dies of a slice panic.
        let emoji = "\u{1F600}".repeat(40);
        assert_eq!(emoji.len(), 160);
        let cut = clamp_close_reason(&emoji, MAX_CLOSE_REASON_BYTES);
        assert_eq!(cut.len(), 120, "4-byte characters must be cut at 120");
        assert_eq!(cut.chars().count(), 30);
        assert!(emoji.starts_with(cut));
    }

    // ── The bytes the close path actually emits ──
    //
    // ★The pair of the tests above that call [`clamp_close_reason`] directly★ — those never measure **whether [`WsTx::close`] calls that function**, so a mutant
    //   deleting the truncating line passes green (measured 2026-09-07).
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
        /// ★Returns EOF★ — `Pending` would hang the close cleanup there, and libtest has no per-test deadline.
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
        // Plays the server role, so outgoing frames are **not masked** — the bytes are read as is.
        let ws = WebSocketStream::from_raw_socket(
            Recorder(written.clone()),
            tokio_tungstenite::tungstenite::protocol::Role::Server,
            None,
        )
        .await;
        let (mut tx, _rx) = split_link(ws, usize::MAX);
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            tx.close(Close::new(CloseCode(4001), "긴".repeat(200))), // hangul-ok
        )
        .await
        .expect("the close did not finish");

        let bytes = written.lock().unwrap().clone();
        assert_eq!(bytes[0], 0x88, "not a close frame: {:02x?}", &bytes[..2]);
        // ★The second byte is the payload length★ — 126 means extended length, itself over 125 (external fact: RFC 6455 §5.5 forbids extended length on
        //   control frames and tungstenite cuts it with `ControlFrameTooBig`).
        assert!(
            bytes[1] <= 125,
            "the control frame payload is {} bytes — the receiver cuts the link",
            bytes[1]
        );
        assert_eq!(u16::from_be_bytes([bytes[2], bytes[3]]), 4001);
    }

    #[test]
    fn a_cut_reason_fits_a_control_frame() {
        // Measured on a real frame: code 2 bytes + message must not exceed 125, or the receiver cuts with `ControlFrameTooBig` ([`MAX_CLOSE_REASON_BYTES`]).
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

    // ★A size cap below 125 also applies to the close frame★ — the receiver's `max_frame_size` covers control frames ([`close_reason_room`]); truncating only to 123 would cut the peer's read.
    #[tokio::test]
    async fn the_close_frame_stays_within_a_small_configured_limit() {
        for (limit, payload) in [(10usize, 10usize), (2, 2), (1, 0), (0, 0)] {
            let written = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
            let ws = WebSocketStream::from_raw_socket(
                Recorder(written.clone()),
                tokio_tungstenite::tungstenite::protocol::Role::Server,
                None,
            )
            .await;
            let (mut tx, _rx) = split_link(ws, limit);
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
    // ★Measures that the cap is the value given to [`split_link`]★ — as a fixed constant again, the consumer's value would apply only to the receiver and the
    //   outgoing side would send frames that make the peer cut.
    #[tokio::test]
    async fn a_frame_over_the_given_limit_fails_as_a_value_and_one_at_it_goes_out() {
        let written = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let ws = WebSocketStream::from_raw_socket(
            Recorder(written.clone()),
            tokio_tungstenite::tungstenite::protocol::Role::Server,
            None,
        )
        .await;
        let (mut tx, _rx) = split_link(ws, 8);
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
    // ★The deadline's reference is the TCP accept time★ — even if the spawned task runs late or finishing and the deadline stand in one poll, nothing is
    //   accepted from the deadline on. Measured with a hand-advanced clock and a memory stream, the test choosing the poll moment.
    const UPGRADE_TIMEOUT: Duration = Duration::from_secs(10);

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
        fn upgrade<S: WsStream>(&self, stream: S) -> impl std::future::Future<Output = ()> {
            upgrade(
                stream,
                "127.0.0.1:5555".parse().unwrap(),
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
            tokio::io::AsyncWriteExt::write_all(&mut client, UPGRADE_REQUEST)
                .await
                .unwrap();
            let task = rig.upgrade(server);
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
            rig.clock.advance(late);
            tokio::io::AsyncWriteExt::write_all(&mut client, UPGRADE_REQUEST)
                .await
                .unwrap();
            task.await;
            rig.assert_timed_out(&format!("ended {late:?} after accept"));
        }
    }

    // Pair of the test above — finishing just before the deadline is accepted, and the auth deadline's reference is the finish time, not the accept time.
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
