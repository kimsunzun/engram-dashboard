//! Connection port — the contract between the daemon's network row (`link`) and its upper layers (ADR-0129).
//!
//! Upper layers name the contract only through this module, so swapping the network row does not touch them. The only frame vocabulary this contract knows is `Frame` below; protocol decode/encode and its types stay in the upper layer (`agent_conn`). The one frame the network row reads itself is the first auth frame (`link::auth`).
//! Both sides plug in the concrete parts — `FrameSink` (per-connection exit) and `FrameFanout` (all-connection exit) by the network row (`link::server::ConnFrameSink` · `ConnRegistry`), `ConnectionHandler`/`ConnectionHandlerFactory` by the upper layer (port-ownership idiom of ADR-0110).

use std::sync::Arc;

use futures_util::future::BoxFuture;

/// Connection identifier (monotonically increasing). Both port traits take it, so it lives on the port side.
pub type ConnId = u64;

/// The output unit put on one connection's outbound queue — these three are the whole frame vocabulary.
#[derive(Debug)]
pub enum Frame {
    Text(String),
    Binary(Vec<u8>),
    /// Connection end — it enters the queue, so earlier frames go out first (FIFO) and then the connection closes (`link::server` maps it to `Peer::close_after_flush`). `reason` is for logs only, not delivered to the client.
    Close(String),
}

/// Frame queuing failed — the connection send queue is saturated or already closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameError;

impl std::fmt::Display for FrameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "frame sink enqueue failed")
    }
}

impl std::error::Error for FrameError {}

/// The only exit for frames leaving on one connection; the network row does not know the vocabulary of what is carried.
///
/// ★The two methods behave differently on saturation (load-bearing asymmetry)★:
/// - `try_send` = non-blocking only, for paths where blocking is forbidden (output pump thread, status callbacks). On saturation it is an immediate `FrameError`, and the implementation must close the connection at that moment without going through the queue — a saturated queue cannot even take `Frame::Close`, so the connection would stay open with nothing to end it (`link::server::ConnFrameSink` closes the peer at once).
/// - `send` = async context only. ★The daemon's implementation does not wait for room★ — on saturation it closes the connection like `try_send`. Do not push a burst through it expecting backpressure; it carries only `on_connect`'s two frames and the `Close`.
pub trait FrameSink: Send + Sync {
    fn try_send(&self, frame: Frame) -> Result<(), FrameError>;

    fn send(&self, frame: Frame) -> BoxFuture<'_, Result<(), FrameError>>;
}

/// An all-connection fanout surface pushing the same text to all registered connections (delivery is best-effort per duties 2 and 3 below). The second shape of the opaque frame hole — per connection is `FrameSink`, all-connection is this (ADR-0129 decision 1).
///
/// ★Why `String` and not `Frame`★: what comes through is one already-encoded text from the upper layer. Widening to `Frame` would make `Frame::Close` to all connections — an all-connection lifetime operation, owned by the network row — expressible in the upper layer. `Frame` is also not `Clone`, so a fanout would have to unwrap and rewrap the payload per connection.
///
/// ★Three implementation duties — upper-layer behavior hangs on them★:
/// 1. Non-blocking. The core's sync threads (pump/manager status callback) call it; blocking here stops the agent output pump entirely.
/// 2. One connection's failure does not block the rest. A saturated connection is skipped and delivery continues — else one slow client stalls updates for all.
/// 3. Failure is not reported to the caller (no return value). Who failed to receive is not observable, so the caller must not presume delivery.
///
/// ★Asymmetry with `FrameSink::try_send` (load-bearing)★: per-connection `try_send` closes that connection on saturation, but fanout saturation does not close a connection. The call targets no particular connection, so no slow-consumer decision is made here; changing that needs a separate decision.
pub trait FrameFanout: Send + Sync {
    fn broadcast_text(&self, text: String);
}

/// What the receive loop does after handling one frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnFlow {
    Continue,
    /// Leave the receive loop and close this connection (e.g. StopDaemon · protocol violation).
    Close,
}

/// Disposition of one frame that arrived while the arrival-order queue is full — the upper layer decides.
///
/// ★Why the upper layer★: 「must this not starve behind the queue」 needs the meaning of a frame, and the network row does not know the agent vocabulary even as a type (ADR-0129). So it asks back only at the moment of saturation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Saturated {
    /// A frame that must not starve — it skips the arrival-order queue and is handled separately, possibly before frames that entered the queue earlier.
    Bypass,
    /// The upper layer has already replied with a refusal — the network row drops this frame and keeps reading.
    Refused,
    /// The upper layer could not reply with a refusal (that connection's send queue is also saturated or closed) — the network row drops this frame and ends the connection.
    ///
    /// ★Kept apart from [`Refused`](Self::Refused)★: that one's 「drop and keep reading」 holds only because the sender gets a reply (duty 2 below). Returning `Refused` after a failed reply would have the sender wait until its own deadline receiving neither the command nor its refusal — silent loss returns through this branch.
    /// ★Cutting is right★: this arises when both queues are saturated at once; the connection can neither receive nor reply, which converges with the choice send-queue saturation already made (the asymmetry of [`FrameSink::try_send`]) — 「if no reply can be made, cut」.
    Unanswered,
}

/// The upper-layer handler receiving one connection's lifetime events. The network row handles only sockets and frames and hands "what that frame means" entirely to this trait.
///
/// ★Call order★: `on_connect` → (`on_text` | `on_binary`)\* → `on_disconnect`; `on_connect` and `on_disconnect` once each per connection.
///
/// ★★Middle calls — in arrival order, never overlapping★★: one single-consumer task per connection calls them; the next call never starts before the previous returns, in socket arrival order. The implementation's ordering invariants (consecutive inputs to one target · acquiring a permission and its later use · a subscription and its release · a registration and its diff) all stand on this.
/// ★The only exception = what the implementation chooses itself★: a frame for which it chose [`Saturated::Bypass`] under receive-queue saturation is handled in a separate task and can run before and overlapping queued frames. The implementation makes the exception and knows its cost; the network row does not judge which frame may do so ([`ConnectionHandler::on_inbound_saturated`]).
/// ★Cost = a long call delays that connection's next frames★: reading from the socket no longer blocks (separate read loop) but the handling queue is one. Detaching long-running work from this queue is the implementation's judgment — only work with no meaningful order may be `spawn`ed out, with the grounds written on the spot. The network row cannot classify it (it does not know the vocabulary).
///
/// ★`on_disconnect` is not a quiet moment — a residual race★: just before it the network row calls `abort()` on the surviving tasks without waiting for completion. `abort()` is only a cancel request, so on a multi-worker runtime the cancelled dispatch task may still be inside the `.await` of `on_text`/`on_binary` — that call and `on_disconnect` can overlap (e.g. a subscription registered after `on_disconnect` snapshots the subscription list is missed by cleanup).
///   ★Work the implementation `spawn`ed out is not reached by this `abort()`★ — it may keep running after cleanup; making sure it cannot revive this connection's state is the implementation's responsibility.
///   The implementation must synchronize its own state and not assume "nobody touches it now". ★Known and left open★ — closing it is a separate decision.
///
/// ★Why `&Arc<dyn FrameSink>`★: a sink that outlives this connection (an output sink registered in the core subscribers) needs a `'static` shared handle.
/// ★Async representation★: `async fn` in a trait is not dyn compatible, so a manual `BoxFuture` is used (no new dependency).
pub trait ConnectionHandler: Send + Sync {
    /// Once, right after connect. A frame put in here is the first output this handler emits.
    ///
    /// ★Not the first frame of the whole connection★: registration precedes this call, so the all-connection fanout (`FrameFanout`) may already have queued frames. Do not assume FIFO head.
    ///
    /// ★Put in only a few frames and do not block for long★: the read task is spawned only after this returns, so a long wait delays everything the connection receives — and if it never returns, the connection is never cleaned up (no `on_disconnect`). (The writer already runs during this call and a saturated `send` closes the connection instead of waiting.)
    ///
    /// ★No safe upper bound as a number★: do not read the queue capacity (`CONN_TX_CAP` in `link::server`) as that bound. The connection is registered in the fanout registry before this call, so the fanout concurrently takes slots from the same queue; real headroom is smaller than the capacity and variable.
    fn on_connect<'a>(
        &'a self,
        conn_id: ConnId,
        frames: &'a Arc<dyn FrameSink>,
    ) -> BoxFuture<'a, ()>;

    fn on_text<'a>(
        &'a self,
        conn_id: ConnId,
        text: &'a str,
        frames: &'a Arc<dyn FrameSink>,
    ) -> BoxFuture<'a, ConnFlow>;

    /// Client → daemon binary frame. Which frame kinds are valid is the upper-layer protocol's judgment, so the network row passes it up as is.
    fn on_binary<'a>(
        &'a self,
        conn_id: ConnId,
        payload: &'a [u8],
        frames: &'a Arc<dyn FrameSink>,
    ) -> BoxFuture<'a, ConnFlow>;

    /// Called only when a text frame arrives while the arrival-order queue is full — normal-path cost is 0.
    ///
    /// ★What breaks without this branch★: if the read loop just waits on a full queue, later frames are not even taken from the socket (TCP is a stream), so even a cancel command that could unclog the queue starves behind it. So instead of waiting it asks the upper layer for this frame's disposition and keeps reading.
    ///
    /// ★Duty 1 — non-blocking · short★: it is sync and runs on top of the read loop; I/O here stalls the socket. Only classify, do not handle.
    /// ★Duty 2 — choosing [`Saturated::Refused`] means the reply must already have been sent★: the network row drops the frame and keeps reading, so without a reply the sender gets neither answer nor error until its deadline. ★If the reply attempt failed, return [`Saturated::Unanswered`]★ — returning `Refused` pretends the duty was kept and the network row cannot trace it back.
    /// ★Duty 3 — [`Saturated::Bypass`] only for what may give up ordering★: the frame is handled before and overlapping queued frames (the exception to the contract above). Choosing it for something causally linked to an earlier frame is a silent ordering defect.
    fn on_inbound_saturated(
        &self,
        conn_id: ConnId,
        text: &str,
        frames: &Arc<dyn FrameSink>,
    ) -> Saturated;

    /// Connection cleanup (release subscriptions · return resources). Sync — the cleanup path has no await point.
    ///
    /// ★Duty — by the time it returns, all copies of the `FrameSink` it made must be released★: sinks made through this port (especially those registered in a long-lived place) that are not reclaimed here keep the connection's transport handle (`Peer`) and queues alive past the connection. How the current implementation keeps this, and the known race that misses the reclaim, are in the comment of `agent_conn::AgentConnection::on_disconnect` — the same root as the abort-overlap race above.
    ///
    /// ★This connection is still in the fanout registry here★ (the network row unregisters after this call), but a broadcast from here cannot be assumed to reach itself — in every branch `link::server` has already closed the peer or seen its link end, so the transport refuses it in practice. That is an observation of the current implementation, not a promise: do not write cleanup logic that presumes self-delivery.
    ///
    /// ★Delivery consistency does not hang on this order★: the broadcast walks a snapshot of the registry map, so moving unregistration earlier or later changes nothing the other connections receive.
    fn on_disconnect(&self, conn_id: ConnId);
}

/// The upper-layer factory making a handler per connection. The network row (`link::server::Server`) holds only this, so it does not know the agent vocabulary even as a type.
pub trait ConnectionHandlerFactory: Send + Sync {
    fn handler_for(&self, conn_id: ConnId) -> Arc<dyn ConnectionHandler>;

    /// A text frame telling the client the handshake failed. No connection queue or handler exists yet and the network row does not know the error vocabulary, so it takes only the encoding from the upper layer and writes it straight to the socket. `None` = encoding failed → close with no body.
    fn handshake_error_frame(&self, message: &str) -> Option<String>;
}
