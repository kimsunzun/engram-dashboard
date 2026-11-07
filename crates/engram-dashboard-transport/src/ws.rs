//! Real-socket WS adapter — lives behind feature `ws`. ★The only real transport implementation in this crate★.
//!
//! ## What this file owns
//!
//! The dialing side ([`WsDialer`] = a [`Dialer`] implementation), the receiving side ([`WsListener`] = a [`Listener`] implementation and its accept policy
//! [`AcceptPolicy`] · refusal report [`AcceptRefusals`]), and the work of splitting one open WS stream into the [`LinkTx`]/[`LinkRx`] two ends.
//! ★The point is that both directions live together★ — so this crate **attaches to itself** with no other crate and measures
//! properties that cannot be measured in memory (ADR-0177 decision 8).
//!
//! ★This adapter's stream wrapping is crate-internal only★ — no function that splits a stream
//! into two ends exists outside the crate, so the only path that sends a real WS stream
//! out as an accepted link ([`Accepted`]) is [`WsListener`] (TRD 3 §3-5 · §5 17). ★This is not a guarantee that consumers cannot skip the check★ — [`Accepted::new`] is
//! public for other transport implementations to use, so a link built from a
//! hand-written [`LinkTx`]/[`LinkRx`] also lands on the registry. Code review, not the compiler, catches that bypass.
//!
//! ## What the receiving side guards (TRD 3 §5 1 · 2 · 12 · 14 · 15 · 16 · 18)
//!
//! The consumer gives every policy value through [`AcceptPolicy`]. The values this file chose are only the two that run the loop — the kernel accept queue
//! length (`LISTEN_BACKLOG`) and the pause after an accept error ([`ACCEPT_ERROR_PAUSE`]).
//! - Opens on loopback only — if the bound address is not loopback it is refused before listen.
//! - TCP accept runs on one task and the WS upgrade runs on a separate task spawned per connection — one peer
//!   that never finishes its upgrade does not block other accepts. The upgrade deadline is measured from the TCP accept time — time the spawned task
//!   spent waiting to run counts against the deadline, and an upgrade that finishes past the deadline is not accepted either.
//! - The upgrade permit ([`crate::PermitPool`]) is taken at TCP accept and sent along in [`Accepted`] — links mid-upgrade, in the ready queue, and
//!   taken away but not yet put on the registry are counted as one total. If all are used, the new TCP is closed right after accept.
//! - The Origin and Host headers of the upgrade request are checked against the policy and cut with 403 if outside it.
//! - The size cap of one message and one frame is applied with the same value on the receiving side and the outgoing side.
//! - What was cut above is reported to [`AcceptRefusals`] with its branch and a masked remote address — the `Err` of [`Listener::accept`] means
//!   only that the listener has ended.
//!
//! ## What it guarantees / what it leaves to the consumer
//!
//! Guarantee = the [`Frame`]↔`Message` round trip · carrying the close code and message **in both directions** (so that 「붙었는데
//! 거절」 (connected but rejected) crosses over as [`LinkRead::Closed`] and ADR-0180 decision 5 holds in code) · raising transport-layer Ping/Pong as
//! [`Frame::Keepalive`].
//!
//! ★Of these three, the **translation table** is asserted by the unit tests below this file★ — [`to_message`]/[`from_message`] are
//! pure functions, so they are measured without a socket. **What is not the table** (the real-socket round trip · whether the close frame really goes out)
//! belongs to the real-socket suite in `tests/`, and of those nobody yet measures the keepalive round trip (see 「알려진 한계」 below).
//!
//! What it leaves = the address syntax (this adapter hands [`Address`] to `tokio-tungstenite` as is) · reconnect, deadline and
//! keepalive interval (all up to [`crate::Policy`] and the supervisor — ★the dialing-side adapter does not apply its own deadline again★,
//! [`Dialer`] rustdoc. ★Only the receiving side's upgrade deadline is an exception★ — it is a phase before the supervisor exists, so it lives in [`AcceptPolicy`]) ·
//! auth and version negotiation (the handshake frames are owned by the [`crate::Handshake`] implementation) · the receiving side's accept loop and
//! peer registry (the consumer calls [`Listener::accept`] and puts it on with [`crate::Registry::attach`]).
//!
//! ## Known limits (do not read as guarantees)
//!
//! - ★The refusal when upgrade permits are all used has no HTTP response★ — TCP is simply closed right after accept, so to the dialing side it looks like
//!   a cut during upgrade (could not connect). Anyone on the same PC can fill the permits — loopback is shared by all OS users
//!   (TRD 3 §5 14 · R11 — a known limit).
//! - ★Refusal reports can be lost★ — when the queue of [`AcceptRefusals`] ([`AcceptPolicy::refusal_queue`]) is full, new reports are dropped and only
//!   [`AcceptRefusals::dropped`] goes up. Because if the accept loop waited on the consumer while refusals pour in, that itself would be a denial of
//!   service.
//! - ★A TCP accept error is not terminal★ — it is reported and accepting resumes. Errors tied to a single connection (`ConnectionRefused` ·
//!   `ConnectionAborted` · `ConnectionReset` — the peer cut before we accepted) take the next one at once without pausing, and only other
//!   errors (such as `EMFILE` when file handles run out) pause for [`ACCEPT_ERROR_PAUSE`] — because that error leaves the connection in the kernel accept queue
//!   and would immediately occur again. The branching is the same as the accept loop of axum 0.8.9 (read from the source, not measured —
//!   `handle_accept_error` in `serve/listener.rs` skips the same three with `is_connection_error` and sleeps 1 second
//!   only on the rest, and its comment cites hyper 0.14 as the source). So [`Listener::accept`] returns `Err` only when the accept task
//!   has ended (a panic).
//! - ★TLS is not turned on★ — only the default features of `tokio-tungstenite` are used, so `wss://` fails at the
//!   dial step.
//! - ★An outgoing close **message** is silently truncated when it overflows★ — the cap is the smaller of 123 bytes (the control frame
//!   payload of 125 bytes minus 2 bytes for the code, RFC 6455 §5.5) and that link's size cap − 2 bytes, and the place that truncates is
//!   the [`LinkTx::close`] implementation. ★No ellipsis and no event★ — the consumer has no way to know it was cut, and finding
//!   out by experience is the only discovery path. The code goes out unchanged, so branching does not collapse (do not put decision material in the message) — but
//!   if the size cap is under 2 bytes the code cannot be carried either and an empty close goes out. The canonical source of the cap value is `close_reason_room` below,
//!   and the reason is in that function and the `MAX_CLOSE_REASON_BYTES` rustdoc.
//!
//! ## Link properties (facts of the outside world)
//!
//! - ★A write really stalls when the kernel send buffer fills★ — `SinkExt::send` calls tungstenite's `flush`
//!   and that turns `WouldBlock` into `Pending` (`Sink::poll_flush` → `cvt(s.flush())` in tokio-tungstenite 0.26.2). So if the peer
//!   does not read, the supervisor's `write_deadline` cuts that
//!   spot — which is what `tests/ws_write_deadline.rs` measures.
//! - ★But **one large write** does not stall★ — **measured** (2026-09-07, Windows 11 loopback): to a peer
//!   that does not read, one 16 MiB frame went straight in in 28 ms (4 and 8 MiB the same), and ★the result was the same even with `SO_SNDBUF`/`SO_RCVBUF`
//!   explicitly set to 4 KiB★. Another axis of the same measurement: repeating 1 MiB frames swallows 2 MiB and
//!   then stalls on the third. **Interpretation** (not an observation) — we read the two numbers diverging in the same setup as Windows'
//!   dynamic send buffering swallowing one large send whole. Even if that interpretation is wrong, the two numbers
//!   stand. ★Do not read backpressure as 「한 프레임이 크면 걸린다」 (a big single frame triggers it)★ — the axis that triggers it is not size but **accumulation**.
//! - ★The frame of a truncated write can go out late★ — confirmed by reading the source (not measured by running).
//!   `SinkExt::send` hands over the item and **then** waits in `flush` (futures-util 0.3.32 `sink/feed.rs:38-40`
//!   → `sink/send.rs:37`), and the handed-over item stays in one slot of `SplitSink` (`stream/stream/split.rs:114`) or
//!   in the tungstenite write buffer (tokio-tungstenite 0.26.2 `lib.rs:340-347`). Even if cancelled there, whatever is next written to the same
//!   end pushes it out first (`split.rs:104-109`). The supervisor cuts a write by control or deadline
//!   and then drops the link, sending a close with [`LinkTx::close`], so **the truncated frame can
//!   go out ahead of that close**, and so the supervisor's count of it as a loss can be inflated ([`crate::Direction::Outbound`]).
//!   ★The in-memory harness does not imitate this shape★ — a blocked write stalls **before** handing over, so when cut
//!   nothing is left (`testing::MemTx`).
//! - ★tungstenite measures only the size of what comes in and not what goes out★ — confirmed by reading the source (0.26.2: only the reading side looks at
//!   `max_frame_size` and `max_message_size`, and `WebSocketContext::write` has no size check). Exceeding it is a **receiving-side
//!   read error**, so the link is cut (measured — sending one 32 MiB frame under the defaults makes the receiver cut with `Space limit exceeded:
//!   Message too long: 33554432 > 16777216`, and **the transport layer does not stop it**). This adapter applies one consumer-supplied value to both
//!   directions — the dialing side via [`WsDialer::new`], the receiving side via [`AcceptPolicy::max_message_bytes`]. The receiving
//!   side has `max_message_size` = `max_frame_size` = that value.
//!   ★So the [`LinkTx::send`] implementation **returns an error** for a frame over that value★ — if only the receiving side enforces the cap,
//!   **an innocent peer burns its own reconnect budget**. ★This refusal rests on 「상대도 같은 값을 쓴다」 (the peer uses the same value)★
//!   — a peer using a smaller cap is still cut by the receiving side. ★The point is that it is a value, not a panic★
//!   — a panic raised inside the supervisor task ends the whole process if it is a production process with a panic hook
//!   (ADR-0292), and in a process without a hook nobody holds its `JoinHandle`, so unobserved the emitted state
//!   freezes at `Live` (the comment at that spot names the symptom). The supervisor turns the error into `LinkLost`,
//!   making it a disconnect event plus reconnect. Release and debug behave the same.
//! - ★**But no diagnostic is left on this branch**★ — the error message above is dropped when the supervisor turns `Ok(Err(_))` into `LinkLost(WriteFailed)`
//!   (`Supervisor::write` in `peer.rs`), and [`crate::TransportEvent::Disconnected`] has
//!   no message field at all (`lib.rs` 「운영 중 끊김은 사유 문구를 안 나른다」 (a disconnect during operation does not carry a reason message)). The frame over the cap is
//!   counted as one outgoing-side loss if it is a notification or fan-out, and `FailPending` wakes it if it is a request, but neither
//!   says 「너무 커서」 (because it is too big). What the consumer sees is a **`WriteFailed` disconnect with no reason**. To make this branch
//!   diagnosable, the disconnect event first needs a message field.
//! - ★**When** the automatic Pong for an incoming Ping goes out★ — confirmed by reading the tungstenite 0.26.2 source
//!   (not measured by running): when the receiver meets a Ping it puts the Pong into `additional_send`
//!   (`protocol/mod.rs:651`), and **the read loop flushes that pending write at the top of every iteration**
//!   (`protocol/mod.rs:452`). That is, the place it goes out is **not one but two — reading and writing**, not just the writing side's poll.
//!   ★Reading does not stop even if the send buffer is full★ — it swallows the `WouldBlock` of that flush, only sets
//!   `unflushed_additional`, and keeps reading (the comment there: "If blocked continue reading, but try
//!   again later"), and tokio-tungstenite's waker proxy also hooks the reading side's waker onto the write axis
//!   (`WakerProxy` in `compat.rs`), so when the socket becomes writable the reading task wakes and flushes again. So
//!   the symptom of congestion is 「우리 Pong 이 늦는다」 (our Pong is late), not 「우리가 못 읽는다」 (we cannot read) — the last
//!   receive time the supervisor measures does not stop because of this. If a late Pong exceeds the peer's patience, the symptom is
//!   「상대가 우리를 침묵으로 끊는다」 (the peer cuts us with silence). ★The basis for being bounded is `write_deadline` alone★ — the supervisor applies
//!   that deadline to **every write** (that invariant in `peer.rs`), so a stuck write is cut at that deadline and the link is dropped.
//!   ★Do not cite `Policy::check` as the basis★ — the
//!   `write_deadline < ping_interval < idle_timeout` it demands is checked **only in debug builds** (the production callers are
//!   just the two `debug_assert_eq!` in `registry.rs` and the other callers are tests). In release, if the consumer flips that
//!   order, nobody stops it. ★And that inequality cannot line up the **peer's** patience in the first place★
//!   — the peer's `idle_timeout` is set on its side regardless of our policy, so comparing our own values
//!   cannot establish 「우리의 늦음 < 상대의 인내」 (our lateness < the peer's patience). What can be established is only that 「우리 쓰기가 무한히 걸려 있지는 않다」 (our writes do not hang forever).
//! - ★The keepalive **round trip** is unmeasured on a real socket★ — the unit tests of this file assert the translation table in both directions
//!   (「보증」 (guarantee) above), but no test measures a real Ping going out and a real Pong coming back as [`Frame::Keepalive`].
//!   The crate's `ping_interval` is far longer than the lifetime of any real-socket test, so that round trip does not happen
//!   inside a test. If it goes wrong, the symptom is the one in the item right above.
//! - ★`Message::Frame` does not come up from real-socket receiving★ (tungstenite docs) — so that branch
//!   is skipped rather than translated into a [`Frame`].
//! - ★The cancel safety of [`LinkRx::recv`] **is not load-bearing anywhere** in this crate★ — ★but the reason is
//!   **not** 「취소되지 않아서」 (it is never cancelled)★: the operating-phase read really is
//!   dropped mid-poll every time a link is dropped (`drop_link` in `peer.rs` calls `abort()` on the reading task and waits for its end — the
//!   `recv()` future in progress at that moment is simply dropped). The real reason it is not load-bearing is that **every place that cancels
//!   drops both ends whole right after**: after that `abort()` `frames_rx` and `rx_raw` are released together and
//!   nothing reads the same link again. The handshake side is the same — the deadline and peer-closed branches return `Err` and
//!   drop the link (`handshake_once`), and the branch where control wins drops the handshake future together with both ends
//!   (`shake_hands`). So there is no place where something is left to lose in a cancelled poll. ★That is why no 「미검」 (unmeasured) label is
//!   put here★ — it is not a debt to repay but a debt never taken on, and if the label stayed the next session would try to measure a risk
//!   that does not exist. **If a new call that reads the same link again after cancelling is written, this sentence becomes void.**

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
/// ★External fact★ — RFC 6455 §5.5 fixes the control frame payload at 125 bytes, and a close frame spends the first
/// 2 bytes on the code. tungstenite 0.26.2 **does not check the outgoing side** (`Frame::close` just
/// concatenates the code and message) but **the receiving side cuts anything over 125 with `ControlFrameTooBig`**
/// (`protocol/mod.rs:640`). So with a long message 「붙었는데 거절」 (connected but rejected) reaches the peer as a **read error**,
/// the rejection decision collapses, and the reconnect budget gets burned too — [`WsTx::close`] truncates before that.
const MAX_CLOSE_REASON_BYTES: usize = 123;

/// Kernel accept queue length — the same value tokio `TcpListener::bind` uses. ★Unrelated to the upgrade permits★ — a connection
/// sitting in this queue has not been accepted by us yet.
const LISTEN_BACKLOG: u32 = 1024;

/// The pause before accepting again after a TCP accept error — the reason and the branch that pauses are in the file header 「알려진 한계」. ★It is not a security value but one that keeps the loop
/// from spinning, so it was not raised to an [`AcceptPolicy`] field★ — it merely delays one connection per error.
///
/// ★It pauses with tokio's real clock, not the [`Clock`] given to the listener★ — the pause applies only to real-socket accept errors, so there is nothing to measure with a hand-pushed
/// clock, and if it followed that clock the accept loop would stop until the test pushes the clock.
pub const ACCEPT_ERROR_PAUSE: Duration = Duration::from_millis(100);

/// Whether to pause for [`ACCEPT_ERROR_PAUSE`] after a TCP accept error. ★Errors tied to one connection do not pause★ — the peer cut
/// before we accepted, so the next accept is a different connection, and pausing there would let a peer that cuts right after opening delay accepting (file header 「알려진
/// 한계」).
fn pauses_after_accept_error(kind: io::ErrorKind) -> bool {
    !matches!(
        kind,
        io::ErrorKind::ConnectionRefused
            | io::ErrorKind::ConnectionAborted
            | io::ErrorKind::ConnectionReset
    )
}

/// The bytes usable for a close message on a link whose size cap is `max_payload` — the smaller of [`MAX_CLOSE_REASON_BYTES`] and
/// `max_payload` − 2 (the code's share). `None` = `max_payload` is under 2 so even the code cannot be carried.
///
/// ★Why it also truncates by `max_payload`★ (external fact) — tungstenite 0.26.2 applies the receiving side's `max_frame_size` as soon as it reads the frame header,
/// with no branching (`protocol/frame/mod.rs:181` — control frames included). If truncated only to 125 bytes, then on a link whose cap is set below that
/// a single close frame cuts the peer's read with `MessageTooLong`.
fn close_reason_room(max_payload: usize) -> Option<usize> {
    max_payload
        .checked_sub(2)
        .map(|room| room.min(MAX_CLOSE_REASON_BYTES))
}

/// Truncates the close message to within `room` bytes ([`close_reason_room`]).
///
/// ★Cut at a character boundary★ — cutting by bytes gives broken UTF-8, and that fails in a way **different** from exceeding the length
/// (tungstenite's `Utf8Bytes` guarantees the message is UTF-8).
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

/// The base property needed at one end of a link. Both `MaybeTlsStream<TcpStream>` (dialing side) and `TcpStream` (receiving side)
/// satisfy it, so both directions use the same [`LinkTx`]/[`LinkRx`] implementation.
pub(crate) trait WsStream: AsyncRead + AsyncWrite + Unpin + Send + 'static {}

impl<S> WsStream for S where S: AsyncRead + AsyncWrite + Unpin + Send + 'static {}

// ── Dialing side ──────────────────────────────────────────────────────────────

/// A [`Dialer`] that opens a real WS link.
///
/// [`Address`] is passed to `tokio-tungstenite` as is — that is, the `ws://host:port/path` syntax, and a violation
/// of that syntax also arrives as [`LinkError`] (just as the contract says the crate does not interpret the address).
///
/// ★Every failure is [`DialFailure::Unreachable`] — it never emits [`DialFailure::Refused`]★. An HTTP
/// rejection at the upgrade step (a status other than 101 — e.g. the 403 of the Origin check) is the same: that response has no close code, and mapping a status number
/// to terminal or redial is a policy this adapter does not know (if invented, a transient fault becomes a give-up with no budget). A
/// consumer who knows the terminal outcome before dialing returns `Refused` from its own [`Dialer`] wrapping this one (TRD 3 §3-0 ⑤).
pub struct WsDialer {
    max_message_bytes: usize,
}

impl WsDialer {
    /// `max_message_bytes` = the byte cap of one message and one frame — both the receiving side's size cap and the boundary at which an outgoing frame is
    /// returned as an error (file header 「통로 성질」). Give the same value as the peer listener's [`AcceptPolicy::max_message_bytes`].
    pub fn new(max_message_bytes: usize) -> Self {
        Self { max_message_bytes }
    }
}

impl Dialer for WsDialer {
    fn dial(
        &self,
        addr: &Address,
    ) -> BoxFuture<'_, Result<(Box<dyn LinkTx>, Box<dyn LinkRx>), DialFailure>> {
        // ★The link is created inside the future★ — the [`Dialer`] contract requires it (if cancelled, nothing
        //   must be left behind).
        // ★Moving the address into an owned String is **enforced by the signature**★ — the `'_` in the return type
        //   is tied to `&self` and not to `addr` (lifetime elision rules), so the future cannot borrow `addr`.
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
/// ★There is no `Default`★ — if the crate chose the values, they would become a security policy decided without the consumer's knowledge. ★There is also no
/// value meaning allow-everything★ — there is no way to turn the checks off, so tests also run with real values in the same fields as production. ★Nor is it `#[non_exhaustive]`★
/// — when a field is added, the consumer's construction site becomes a compile error that forces it to choose that value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcceptPolicy {
    /// The values accepted during an upgrade that carries an Origin header — compared byte for byte (TRD 3 §5 2).
    /// ★If empty, every upgrade carrying an Origin is cut with 403★ (the release value of D21 ①). ★An upgrade without an Origin is always
    /// accepted★ — clients outside a browser do not send that header, and what this check blocks is requests launched by a browser page (authentication
    /// belongs to the handshake). If there are two or more headers or it is not text, it is cut.
    pub allowed_origins: Vec<String>,
    /// The values accepted for the name part of the Host header (before the port — case-insensitive). E.g. `127.0.0.1` · `localhost` (TRD 3 §5 16 — DNS
    /// rebinding). The port must be the one this listener bound (no port = 80). ★An upgrade without a Host is accepted★ (D9) ·
    /// if there are two or more headers or it is not text, it is cut. If empty, every upgrade carrying a Host is cut.
    /// ★An IPv6 name is written with its brackets★ (`[::1]`) — the Host header carries IPv6 that way and the name is compared as is. A name with a colon
    /// outside the brackets (`::1` · `localhost:8080`) matches no header, so [`WsListener::bind`] returns `Err`.
    pub allowed_host_names: Vec<String>,
    /// The deadline from TCP accept to the end of the WS upgrade (TRD 3 §5 12) — past it that TCP is closed. ★The accepted peer's auth deadline is measured from where this
    /// deadline ends ([`Accepted::upgraded_at`])★ — the two do not overlap but add up.
    pub upgrade_timeout: Duration,
    /// The number of upgrade permits (TRD 3 §5 14) — taken at TCP accept and released when the peer put on the registry enters the operating phase (or when [`Accepted`] is dropped).
    /// ★The ready queue is also counted by these permits★ — so the cap of the ready queue is this value and it is not counted separately. If all are used, the new TCP is
    /// closed right after accept. 0 = accept nothing.
    pub max_pending_upgrades: usize,
    /// The byte cap of one message and one frame (TRD 3 §5 15) — both the receiving-side cap of the accepted link and the boundary at which an outgoing frame is
    /// returned as an error (file header 「통로 성질」). Give the same value to the dialing side's [`WsDialer::new`].
    pub max_message_bytes: usize,
    /// The size of the refusal report queue (TRD 3 §5 18) — when full, new reports are dropped and [`AcceptRefusals::dropped`] is raised. 0 counts as 1.
    pub refusal_queue: usize,
}

/// Why the listener cut one peer — [`AcceptRefusal::kind`].
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

/// The report of one peer that was cut (TRD 3 §5 18).
///
/// ★The text fields are already masked with [`crate::mask_reason`]★ — they can be printed as is. ★The cut happens before the auth frame is read, so
/// there is no place for a token to ride★.
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

/// The refusal report receiver returned together with the listener by [`WsListener::bind`] — a place to report from outside the [`Listener`] trait (that trait's
/// `Err` means only that it has ended).
///
/// ★You may drop it★ — later reports are discarded and the listener keeps running. ★If you do not read it the queue fills and new reports are lost first★ —
/// the lost count is [`AcceptRefusals::dropped`]. Dropping the listener hands out the remaining reports and then ends with `None`.
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
        // ★It does not wait★ — if the accept loop or an upgrade task waited on the consumer, accepting would stall when refusals pour in.
        if self.tx.try_send(refusal).is_err() {
            self.dropped.fetch_add(1, Ordering::AcqRel);
        }
    }
}

/// A [`Listener`] that accepts real WS links.
///
/// TCP accept runs on one task inside the listener, and the upgrade runs on a separate task spawned per connection. A link that finished its upgrade
/// sits in the ready queue waiting for [`Listener::accept`] (that stretch also holds an upgrade permit). ★Dropping it reaps those tasks and
/// closes the links in the ready queue★ — the dialing side sees that as a link loss.
pub struct WsListener {
    local: SocketAddr,
    ready: mpsc::Receiver<Accepted>,
    permits: PermitPool,
    driver: JoinHandle<()>,
}

impl WsListener {
    /// Opens on `addr` and starts accepting — returns the listener and its refusal report receiver. Giving port 0 opens
    /// on a port the kernel picks (the actual number is [`WsListener::local_addr`]).
    ///
    /// ★Opens on loopback only★ (TRD 3 §5 1 · C8) — if the bound address is not loopback it is `Err` before listen. Plain
    /// `ws://` is not opened to the outside (remote goes with TLS — TRD 3 §4). It is also `Err` if [`AcceptPolicy::allowed_host_names`] has a name with a colon
    /// outside the brackets — that name would quietly set up a policy that accepts nothing.
    ///
    /// ★`clock` must be the same clock as the registry the accepted link will be put on★ — it stamps [`Accepted::upgraded_at`] with this, and the registry measures
    /// the auth deadline from that time ([`crate::Registry::attach_with_policy`] asserts that match in debug builds).
    /// The upgrade deadline is measured with it too.
    ///
    /// Call it inside a tokio runtime — outside one it panics (it spawns the accept task).
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
        // ★Check before listen★ — if refused after listening, connections from outside in the meantime have already entered the kernel accept queue.
        ensure_loopback(local)?;
        let tcp = socket
            .listen(LISTEN_BACKLOG)
            .map_err(|e| LinkError::new(format!("ws listen {local}: {e}")))?;

        let permits = PermitPool::new(policy.max_pending_upgrades);
        // ★The ready queue never fills★ — each link sitting in it holds a permit, so queue length ≤ permits held ≤ this capacity.
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

    /// The [`Address`] that reaches this listener — put it straight into [`WsDialer`].
    pub fn dial_address(&self) -> Address {
        Address::new(format!("ws://{}/", self.local))
    }

    /// The number of upgrade permits currently held — counts together links mid-upgrade, in the ready queue, and taken away but not yet released.
    ///
    /// ★The permit pool itself is not handed out★ — handing it out would let it be used to hold permits and block this listener's accepting.
    pub fn permits_in_use(&self) -> usize {
        self.permits.in_use()
    }

    /// The number of upgrade permits — [`AcceptPolicy::max_pending_upgrades`].
    pub fn permit_limit(&self) -> usize {
        self.permits.limit()
    }

    /// The number sitting in the ready queue not yet taken.
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
            "ws bind {local}: loopback 이 아니다 — 평문 ws:// 청취기는 loopback 에만 연다"
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
            "ws accept policy: Host 이름 {name:?} 에 대괄호 밖 콜론이 있다 — IPv6 는 `[::1]` 꼴로 · 포트는 빼고 적는다"
        ))),
        None => Ok(()),
    }
}

/// What the accept task and the upgrade tasks read together.
struct Shared {
    local: SocketAddr,
    policy: AcceptPolicy,
    permits: PermitPool,
    clock: Arc<dyn Clock>,
    reporter: Reporter,
}

/// The TCP accept loop. ★It does not wait for the upgrade here★ — it spawns a task per connection so one slow peer does not block the next
/// accept (TRD 3 §5 12). The spawned tasks are held by `upgrades`, so when this task is reaped (listener dropped) they are reaped together.
async fn drive(tcp: TcpListener, shared: Arc<Shared>, ready: mpsc::Sender<Accepted>) {
    let mut upgrades = JoinSet::new();
    loop {
        tokio::select! {
            accepted = tcp.accept() => match accepted {
                Ok((stream, remote)) => match shared.permits.try_take() {
                    Some(permit) => {
                        // ★Stamp the deadline's reference here, before spawning★ — if stamped inside the upgrade task, the delay until it first
                        //   runs after spawning stays outside the deadline (TRD 3 §5 12 — from TCP accept).
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
                        // The overflow closes **the new one** (TRD 3 §5 14) — a peer already holding a permit is not touched.
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

/// The upgrade of one link — if it finishes before the deadline time (`accepted_at` + [`AcceptPolicy::upgrade_timeout`]) it is
/// put in the ready queue, otherwise it is closed and reported. `accepted_at` = the TCP accept time (`shared.clock`). `permit` goes out with the accepted link or
/// is dropped here.
///
/// ★The deadline is checked in three places★ — before starting (the spawned task ran late and already passed it), while waiting, and after finishing. The after-finish check is separate
/// because when finishing and the deadline stand in one poll, [`with_deadline`] polls the upgrade side first and that side wins.
/// It counts as past the deadline from **that very moment** of the deadline time.
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
    // The time left until the deadline — `None` = the deadline time was reached or passed. ★Subtract the elapsed time rather than adding the deadline to the time★ —
    //   `Instant + Duration` panics on overflow and the deadline field has no cap.
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
            // ★If it was cut by the header decision and was in the middle of writing the 403, report that branch★ — if reported as the deadline, the reason for the cut and the header seen
            //   disappear (the case where the peer did not read the 403 and the write stalled).
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
    // ★Stamp the very moment the upgrade finished with the same clock as the registry★ — the auth deadline starts here (TRD 3 §5 3).
    let upgraded_at = shared.clock.now();
    if remaining_at(upgraded_at).is_none() {
        // It finished past the deadline — even a finished upgrade is not accepted (this function's rustdoc). The link and the permit are dropped here.
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
        // An unreachable branch (the capacity comment in `WsListener::bind`) — if reached, treat it the same as the permits being full.
        Err(mpsc::error::TrySendError::Full(_)) => report(RefusalKind::SlotsFull, None, None),
        // The listener was dropped — the link is closed here.
        Err(mpsc::error::TrySendError::Closed(_)) => {}
    }
}

/// The tungstenite header callback — checks the upgrade request headers against the policy and leaves that decision in `verdict` (the Origin to accept if it
/// passes, otherwise the branch that cut). If it cuts, it returns 403.
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

/// The 403 response. ★It does not say in the body which check caught it★ — there is no reason to tell the peer the policy, and the branch is carried by the refusal
/// report.
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

/// The shape of reading one named header.
enum OneHeader {
    Absent,
    One(String),
    /// Two or more, or bytes that are not text — cut without deciding. The real value is for reporting (lossy conversion).
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

/// Whether the Host header value `이름[:포트]` matches the accepted names · this listener's port. In the `[::1]:포트` form only the colon after the bracket is the port.
/// The port accepts digits only (signs · blanks rejected) · 80 if absent.
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

/// Splits an open stream into the two ends of a link. `max_payload` = the outgoing frame cap (file header 「통로 성질」).
///
/// ★Crate-internal only★ (TRD 3 §3-5 · §5 17) — if opened to the outside, anyone could build a link from their own accept and
/// skip every check of [`WsListener`].
///
/// ★The two ends share the same stream through a `BiLock`★ — so different tasks may hold them (the supervisor hands the reading
/// side to a dedicated task), and when one side returns `Pending` it releases the lock so the other is not blocked.
/// ★The socket closes only when **both** ends are dropped★ — here that `BiLock` releases
/// the inner stream only when the last end is dropped. The duty to guarantee that rests with the caller and the seam carries that duty
/// ([`LinkTx`] rustdoc).
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
    /// The cap of the outgoing frame payload — the same value as the size cap received by the side that opened this link ([`split_link`]).
    max_payload: usize,
}

impl<S: WsStream> LinkTx for WsTx<S> {
    fn send(&mut self, frame: Frame) -> BoxFuture<'_, Result<(), LinkError>> {
        Box::pin(async move {
            // ★A frame over the cap fails **as a value**★ — otherwise only the receiving side enforces the cap
            //   (file header 「tungstenite 는 들어오는 것의 크기만 재고 나가는 것은 안 잰다」), so the link is cut and
            //   **an innocent peer burns its own reconnect budget**. ★Do not go back to panicking★ —
            //   the version that was a `debug_assert` fires inside the supervisor task, and nobody holds its `JoinHandle`
            //   (`Unstarted::start`), so there was **no place to observe the panic**: the sending side of the state watch dropped,
            //   the emitted state froze at `Live` forever, no disconnect event and no give-up event came, and the reading task
            //   fell away still holding half the link — in a production process with a panic hook the whole process ends instead
            //   (ADR-0292). If it fails as a value, the supervisor turns it into `LinkLost`
            //   (`Supervisor::write`) and a disconnect event and reconnect remain. ★But **the message does not remain**★ — that conversion
            //   drops the `LinkError` message and `Disconnected` has no message field (that item in this file's header).
            let len = payload_len(&frame);
            if len > self.max_payload {
                return Err(LinkError::new(format!(
                    "ws write: 나가는 프레임이 {len} 바이트다 — 상한 {} 을 넘으면 받는 쪽이 통로를 끊는다",
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
            // Truncates the message with [`close_reason_room`] — that function gives the reason.
            let message = match close_reason_room(self.max_payload) {
                Some(room) => Message::Close(Some(CloseFrame {
                    code: WsCloseCode::from(close.code.0),
                    reason: clamp_close_reason(&close.reason, room).into(),
                })),
                // ★If the cap cannot even carry the code, send an empty close★ — the peer reads it as a close with no code. If the code were carried it would exceed the cap
                //   and cut the peer's read.
                None => Message::Close(None),
            };
            // Even on failure there is nowhere to report it ([`LinkTx::close`] contract). If the close frame does not go out the peer cannot see the code and
            //   reads it as cut without a word — that limit is exactly the one in `frame.rs`.
            let _ = self.sink.send(message).await;
            let _ = self.sink.close().await;
        })
    }
}

struct WsRx<S: WsStream> {
    stream: SplitStream<WebSocketStream<S>>,
    /// Reading has already failed on this link.
    ///
    /// ★It exists to keep [`LinkRead::Closed`] as 「상대가 닫았다고 말했다」 (the peer said it closed)★ — the reason is in [`WsRx::recv`].
    failed: bool,
}

impl<S: WsStream> LinkRx for WsRx<S> {
    fn recv(&mut self) -> BoxFuture<'_, Result<LinkRead, LinkError>> {
        Box::pin(async move {
            // ★A read that failed once keeps failing★ — only then does the [`LinkRead::Closed`] contract (「상대가 닫았다고 **말했다**」 (the
            //   peer said it closed)) hold unconditionally. Without it there is a leak like this: the first read of a socket dropped
            //   without a word is a `Protocol(ResetWithoutClosingHandshake)` error, but after that tungstenite
            //   sets its own `ended` marker and **the next read gives `Ready(None)`** (`lib.rs:303`
            //   in 0.26.2). If that `None` is passed along as is, a close nobody spoke is reported, and a consumer that
            //   branches on that value swaps 「거절당했다/곱게 갔다」 (rejected / left politely) for 「끊겼다」 (cut off).
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
                    // ★A dried-up stream is **not** 「말 없이 끊겼다」 (cut without a word)★ — it comes here
                    //   only after a close frame has already been read (external fact: tokio-tungstenite 0.26.2 turns only `ConnectionClosed`
                    //   /`AlreadyClosed` into `Ready(None)` at `lib.rs:303`, and that error arises only when the state is
                    //   `ClosedByPeer`/`CloseAcknowledged` at `protocol/mod.rs:706`).
                    //   A peer that vanished without a word is `Protocol(ResetWithoutClosingHandshake)`, so it comes through the `Err`
                    //   branch above — the [`LinkRead::Closed`] rustdoc carries that contract.
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
        // ★It goes out as the same thing as [`LinkTx::ping`]★ — keepalive is transport-layer vocabulary, so the payload is not
        //   redesigned (the reason given in the [`LinkTx::ping`] rustdoc).
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
        // One character is 3 bytes, so 41 characters = 123 bytes, and the 42nd character exceeds the cap.
        let long = "가".repeat(60);
        let cut = clamp_close_reason(&long, MAX_CLOSE_REASON_BYTES);
        assert!(cut.len() <= MAX_CLOSE_REASON_BYTES);
        assert_eq!(cut.chars().count(), MAX_CLOSE_REASON_BYTES / 3);
        // The truncated result is still a prefix of the original and holds as UTF-8 — if the bytes were broken, this is where it breaks.
        assert!(long.starts_with(cut));
    }

    #[test]
    fn a_four_byte_character_is_cut_below_the_limit_not_at_it() {
        // ★An input with 4-byte characters is why this test exists★ — with 3-byte characters alone 123 is a multiple of 3,
        //   so the mutant 「경계 맞추기를 지우고 123 에서 그냥 자른다」 (delete the boundary alignment and just cut at 123) passes (measured 2026-09-07:
        //   under that mutant all four old inputs were green). With 4-byte characters 123 is not a boundary
        //   and that mutant dies of a slice panic.
        let emoji = "\u{1F600}".repeat(40);
        assert_eq!(emoji.len(), 160);
        let cut = clamp_close_reason(&emoji, MAX_CLOSE_REASON_BYTES);
        assert_eq!(cut.len(), 120, "4 바이트 글자는 120 에서 잘려야 한다");
        assert_eq!(cut.chars().count(), 30);
        assert!(emoji.starts_with(cut));
    }

    // ── The bytes the close path actually emits ──
    //
    // ★This is the pair of the tests above that call [`clamp_close_reason`] directly★ — those alone never measure **whether [`WsTx::close`]
    //   calls that function**, so a mutant that deletes the truncating line passes entirely green
    //   (measured 2026-09-07).
    /// A minimal stream that collects the bytes written. Lets us see the outgoing frame as is, without a real socket.
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
        let (mut tx, _rx) = split_link(ws, usize::MAX);
        // ★Bounded wait★ — if it hangs it fails cleanly.
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            tx.close(Close::new(CloseCode(4001), "긴".repeat(200))),
        )
        .await
        .expect("닫기가 안 끝났다");

        let bytes = written.lock().unwrap().clone();
        assert_eq!(bytes[0], 0x88, "닫기 프레임이 아니다: {:02x?}", &bytes[..2]);
        // ★The second byte is the payload length★ — 126 means it moved to the extended length, and that is itself over 125
        //   (external fact: RFC 6455 §5.5 forbids extended length on control frames and tungstenite cuts
        //   it on the receiving side with `ControlFrameTooBig`).
        assert!(
            bytes[1] <= 125,
            "제어 프레임 payload 가 {} 바이트다 — 받는 쪽이 통로를 끊는다",
            bytes[1]
        );
        // The code goes out as is even when truncated — which is why the branching material is the code, not the message.
        assert_eq!(u16::from_be_bytes([bytes[2], bytes[3]]), 4001);
    }

    #[test]
    fn a_cut_reason_fits_a_control_frame() {
        // Measures on a real frame that code 2 bytes + message does not exceed 125 — if it does, the receiving side
        //   cuts with `ControlFrameTooBig` ([`MAX_CLOSE_REASON_BYTES`]).
        for reason in ["", "짧다", &"긴".repeat(200), &"a".repeat(200)] {
            let frame = WsFrame::close(Some(CloseFrame {
                code: WsCloseCode::from(4001),
                reason: clamp_close_reason(reason, close_reason_room(usize::MAX).unwrap()).into(),
            }));
            assert!(frame.payload().len() <= 125, "{}", frame.payload().len());
        }
    }

    // ★A size cap below 125 also applies to the close frame★ — the receiving side's `max_frame_size` applies to control frames too
    //   ([`close_reason_room`]), so truncating only to 123 would cut the peer's read at these caps.
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
            let (mut tx, _rx) = split_link(ws, limit);
            tokio::time::timeout(
                std::time::Duration::from_secs(5),
                tx.close(Close::new(CloseCode(4001), "a".repeat(200))),
            )
            .await
            .expect("닫기가 안 끝났다");

            let bytes = written.lock().unwrap().clone();
            assert_eq!(bytes[0], 0x88, "상한 {limit}: 닫기 프레임이 아니다");
            assert_eq!(usize::from(bytes[1]), payload, "상한 {limit}: payload 길이");
            if payload >= 2 {
                assert_eq!(
                    u16::from_be_bytes([bytes[2], bytes[3]]),
                    4001,
                    "상한 {limit}: 코드"
                );
            }
        }
    }

    // ── Outgoing size cap ──
    //
    // ★Measures that the cap is the value given to [`split_link`]★ — if turned back into a fixed constant, the consumer-supplied value applies only to the receiving side
    //   and the outgoing side stays at the old value, sending frames that make the peer cut.
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
        assert!(over.is_err(), "상한 + 1 이 나갔다");
        assert!(
            written.lock().unwrap().is_empty(),
            "거절한 프레임이 소켓에 닿았다"
        );
        tx.send(Frame::Binary(vec![0; 8]))
            .await
            .expect("상한 그대로는 나가야 한다");
        assert!(!written.lock().unwrap().is_empty());
    }

    #[test]
    fn the_receive_limit_covers_a_message_and_a_frame_alike() {
        // ★Both must be covered★ — if only the frame is covered, a fragmented message grows up to tungstenite's default 64 MiB.
        let config = ws_config(1234);
        assert_eq!(config.max_message_size, Some(1234));
        assert_eq!(config.max_frame_size, Some(1234));
    }

    // ── loopback 단언 ──
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
            assert!(!pauses_after_accept_error(kind), "{kind:?} 에 쉬었다");
        }
        // Running out of file handles (`EMFILE`) has no named branch and arrives as a different branch per platform — measure with the named rest.
        for kind in [
            io::ErrorKind::Other,
            io::ErrorKind::OutOfMemory,
            io::ErrorKind::PermissionDenied,
            io::ErrorKind::InvalidInput,
        ] {
            assert!(pauses_after_accept_error(kind), "{kind:?} 에 안 쉬었다");
        }
    }

    // ── Upgrade deadline (§5 12) ──
    //
    // ★The reference for the deadline is the TCP accept time★ — even if the spawned task runs late, even if finishing and the deadline stand in one poll, nothing is
    //   accepted from the deadline time on. Measured with a hand-pushed clock and a memory stream, without a socket or real time, with the test choosing the poll moment.
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
                self.clock.now(),
                self.permits.try_take().expect("칸"),
                self.shared.clone(),
                self.ready_tx.clone(),
            )
        }

        fn assert_timed_out(&mut self, what: &str) {
            assert!(
                self.ready.try_recv().is_err(),
                "{what}: 시한을 넘긴 업그레이드가 준비 큐에 앉았다"
            );
            let report = self
                .refusals
                .try_recv()
                .unwrap_or_else(|| panic!("{what}: 보고가 없다"));
            assert_eq!(report.kind, RefusalKind::UpgradeTimeout, "{what}");
            assert_eq!(self.permits.in_use(), 0, "{what}: 칸이 안 돌아갔다");
        }
    }

    /// Polls only once — `true` if finished.
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
            rig.assert_timed_out(&format!("수락 뒤 {late:?} 에 처음 돎"));
            let mut answer = Vec::new();
            tokio::io::AsyncReadExt::read_to_end(&mut client, &mut answer)
                .await
                .unwrap();
            assert!(answer.is_empty(), "시한을 넘긴 뒤 응답을 썼다");
        }
    }

    #[tokio::test]
    async fn an_upgrade_that_finishes_at_or_after_the_cutoff_is_refused() {
        for late in [UPGRADE_TIMEOUT, UPGRADE_TIMEOUT + Duration::from_millis(1)] {
            let mut rig = rig();
            let (mut client, server) = tokio::io::duplex(64 * 1024);
            let mut task = Box::pin(rig.upgrade(server));
            assert!(!poll_once(&mut task).await, "요청 없이 끝났다");
            // The deadline and the request stand together — on the next poll the upgrade is polled before the deadline and finishes.
            rig.clock.advance(late);
            tokio::io::AsyncWriteExt::write_all(&mut client, UPGRADE_REQUEST)
                .await
                .unwrap();
            task.await;
            rig.assert_timed_out(&format!("수락 뒤 {late:?} 에 끝남"));
        }
    }

    // The pair of the test above — in the same order, finishing just before the deadline is accepted, and the reference for the auth deadline is the finish time, not the accept time.
    #[tokio::test]
    async fn an_upgrade_that_finishes_just_before_the_cutoff_is_accepted_stamped_at_completion() {
        let mut rig = rig();
        let (mut client, server) = tokio::io::duplex(64 * 1024);
        let accepted_at = rig.clock.now();
        let mut task = Box::pin(rig.upgrade(server));
        assert!(!poll_once(&mut task).await, "요청 없이 끝났다");
        let early = UPGRADE_TIMEOUT - Duration::from_millis(1);
        rig.clock.advance(early);
        tokio::io::AsyncWriteExt::write_all(&mut client, UPGRADE_REQUEST)
            .await
            .unwrap();
        task.await;
        let accepted = rig.ready.try_recv().expect("시한 전에 끝난 업그레이드");
        assert_eq!(accepted.upgraded_at(), accepted_at + early);
        assert!(rig.refusals.try_recv().is_none());
        assert_eq!(rig.permits.in_use(), 1, "받은 통로가 칸을 쥔다");
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
        assert!(!poll_once(&mut task).await, "403 을 다 썼다");
        rig.clock.advance(UPGRADE_TIMEOUT);
        task.await;
        let report = rig.refusals.try_recv().expect("보고");
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
            "Host 없음은 받는다(D9)"
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
        .expect_err("Host 가 밖이다");
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
        assert_eq!(refusals.dropped(), 2, "찬 줄에서 버린 수");
        let first = refusals.try_recv().expect("첫 보고");
        assert_eq!(
            first.kind,
            RefusalKind::OriginRejected,
            "먼저 온 것이 남는다"
        );
        assert_eq!(first.remote.as_deref(), Some("127.0.0.1:5555"));
        assert_eq!(first.origin.as_deref(), Some("http://evilx"));
        assert!(refusals.try_recv().is_none());
    }
}
