//! Opaque frame port — the contract between the network row and the upper layer (ADR-0129).
//!
//! The frame vocabulary the contract defined here knows is only `Frame` below. Protocol decode/encode and its types do not
//! enter this contract — the consumer of that vocabulary is the upper layer (`agent_conn` in the daemon crate), and the
//! exception left inside this crate is recorded by the lib.rs header.
//! The contract (trait) is owned by the lower layer, the network row, and the concrete parts are plugged in by both sides — `FrameSink` (the
//! per-connection exit) and `FrameFanout` (the all-connection exit) are implemented by the network row (`ws::ConnFrameSink`·`ws::ConnRegistry`),
//! `ConnectionHandler`/`ConnectionHandlerFactory` by the upper layer (the same vein as the port-ownership idiom
//! of ADR-0110).

use std::sync::Arc;

use futures_util::future::BoxFuture;

/// Connection identifier (monotonically increasing). Every method of the two port traits takes it — so it lives on the
/// port side, not in a carrier implementation.
pub type ConnId = u64;

/// The output unit flowing into the single per-connection writer queue — these three are the whole frame vocabulary.
#[derive(Debug)]
pub enum Frame {
    Text(String),
    Binary(Vec<u8>),
    /// Connection end — when the writer receives this it closes and leaves the loop. It enters **into** the queue, so the
    /// frames put in earlier all go out first (FIFO). `reason` is for logs/debugging and is not delivered to the client.
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

/// The only exit for frames leaving on one connection. The upper layer sends only through this, so the network row does not know the
/// vocabulary of what is carried.
///
/// ★The two methods **behave differently** on saturation (load-bearing asymmetry)★:
/// - `try_send` = **non-blocking only**. Called from paths where blocking is forbidden, such as the output pump thread and status callbacks.
///   On saturation it is an immediate `FrameError`, and the implementation must ring the close signal **outside** the queue at that moment — because in the saturated
///   state even `Frame::Close` cannot enter the queue and the connection becomes a zombie (the `ws` module header).
/// - `send` = async context only, backpressure allowed (waits until a slot opens). It does **not ring** the close signal here
///   — the caller can wait, so it is not a candidate for a slow-consumer decision.
pub trait FrameSink: Send + Sync {
    fn try_send(&self, frame: Frame) -> Result<(), FrameError>;

    fn send(&self, frame: Frame) -> BoxFuture<'_, Result<(), FrameError>>;
}

/// An all-connection fanout surface that pushes the same text to **all** registered connections (actual delivery is best-effort per
/// duties 2 and 3 below). The second shape of the opaque frame hole — per connection is `FrameSink` above, all-connection is
/// this (the 2026-08-04 note of ADR-0129 decision 1).
///
/// ★Why `String` and not `Frame`★: what comes through this port is one **already-encoded text** from the upper layer
///   (what was encoded is the upper layer's business and not this contract's concern). Widening to `Frame` would make
///   pushing `Frame::Close` to all connections — that is, **an all-connection lifetime operation** — expressible in the upper layer,
///   but connection lifetime is a concern owned by the network row. The narrow type makes that operation inexpressible in the first place.
///   In addition, `Frame` is not `Clone`, so for a fanout implementation to copy it per connection it would have to take the payload back out and
///   wrap it again — the narrow type keeps that copy loop honest too.
///
/// ★Three implementation duties — upper-layer behavior hangs on them★:
/// 1. **Non-blocking**. It is also called from the core's sync threads (the status callback of the pump/manager). If it blocks here,
///    the agent output pump stops entirely.
/// 2. **One connection's failure does not block the rest**. A connection whose queue is saturated must be skipped and delivery must
///    continue to the remaining connections — violate this and one slow client stalls updates for all clients.
/// 3. **Failure is not reported to the caller** (no return value). Who failed to receive is not observable through this port, so the
///    caller must not presume delivery.
///
/// ★Asymmetry with `FrameSink::try_send` (load-bearing)★: per-connection `try_send` rings that connection's
///   close signal on saturation, but **fanout saturation does not lead to connection close**. Since the call targets no particular connection,
///   it means no slow-consumer decision is put on it here, and changing that needs a separate decision.
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

/// The disposition of one frame that arrived **while the arrival-order queue is full** — the upper layer decides.
///
/// ★Why the upper layer★: 「이것은 줄 뒤에서 굶으면 안 되는 것인가」 (must this not starve behind the queue) is a decision that needs to know the **meaning** of a frame, and
///   the network row does not know the agent vocabulary even as a type (ADR-0129). So it asks back only at the moment of saturation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Saturated {
    /// A frame that must not starve — it **skips** the arrival-order queue and is handled separately. That is, this frame may be handled
    /// before those that entered the queue earlier.
    Bypass,
    /// The upper layer **has already replied with a refusal** — the network row drops this frame and keeps reading.
    Refused,
    /// The upper layer **could not reply with a refusal** (that connection's send queue is also saturated or closed) — the network row drops this
    /// frame and **ends the connection**.
    ///
    /// ★Why it is kept apart from [`Refused`](Self::Refused)★: the only basis on which that one's 「버리고 계속 읽는다」 (drop and keep reading) holds is
    ///   that **the sender gets a reply** (duty 2 below). If the reply failed and we returned the same value, the
    ///   network row would trust a reply that does not exist and drop the frame, and the sender would wait until its own deadline
    ///   receiving **neither the command nor its refusal** — silent loss returns through exactly this branch.
    /// ★Why cutting is right★: this value arises when **both queues are saturated at once**. That connection can neither receive
    ///   more nor reply more, so it converges to the same place as the disposition send-queue saturation already chose (the asymmetry of [`FrameSink::try_send`]) —
    ///   「답장을 만들 수 없으면 끊는다」 (if no reply can be made, cut).
    Unanswered,
}

/// The upper-layer handler that receives one connection's lifetime events. The network row handles only sockets and frames and hands off
/// "what that frame means" entirely behind this trait.
///
/// ★Call order★: `on_connect` → (`on_text` | `on_binary`)\* → `on_disconnect`. Per connection `on_connect` and
///   `on_disconnect` are each once, and the middle is zero or more.
///
/// ★★Contract of the middle calls — **in arrival order, never overlapping**★★: the network row calls these two from one
///   single-consumer task per connection. The next call never starts before the previous one returns, and
///   the order is the order of arrival on the socket. The implementation's ordering invariants (consecutive inputs to the same target · acquiring a permission and
///   its later use · a subscription and its release · a registration and its diff) **all stand on this one property.**
/// ★The contract's **only exception = what the implementation chooses itself**★: a frame for which the implementation chose
///   [`Saturated::Bypass`] when the receive queue is saturated is handled in a separate task, so it can run **before and
///   overlapping** those in the queue. The implementation both makes the exception and is the one that knows its cost — the network row does not judge which
///   frame may do so ([`ConnectionHandler::on_inbound_saturated`]).
/// ★The cost = if this call takes long, **that connection's next frames are delayed with it**★. Taking from the socket
///   no longer blocks (the read loop runs separately) but the handling queue is one. So **the judgment to detach long-running work from this
///   queue is the implementation's** — only work it has itself judged to have no meaningful order may be
///   `spawn`ed out, and the grounds for that decision must be written on the spot. The network row cannot know which frame takes
///   long (it does not know the vocabulary), so it cannot do that classification on its behalf.
///
/// ★`on_disconnect` is **not** a quiet moment — a residual race the implementation must know★:
///   Just before this call the network row calls `abort()` on the surviving tasks but does not wait for their completion.
///   `abort()` is a cancel *request*, so on a multi-worker runtime the cancelled **dispatch task** may still be
///   inside the `.await` of `on_text`/`on_binary` — that is, that call and `on_disconnect` **can
///   overlap**. Example: a subscription registered **after** `on_disconnect` snapshots the subscription list is missed by the cleanup.
///   ★Work the implementation `spawn`ed out is **not even reached** by this `abort()`★ — that task is unknown to the network row,
///   so it may keep running after the connection is cleaned up. Making sure the sent-out work cannot
///   revive this connection's state is the responsibility of the implementation that sent it out.
///   The implementation must synchronize its own state and must not assume "nobody touches it at this point".
///   ★This race was not made by this port; HEAD has it too★ (even when the cleanup block was inline in `handle_connection`, it ran
///   right after abort as is) — whether to close it is a separate decision outside this slice.
///
/// ★Why `&Arc<dyn FrameSink>`★: for the implementation to build a sink that outlives this connection (an output sink
///   registered in the core subscribers) it needs a `'static` shared handle — a borrowed reference cannot do it.
/// ★async representation★: `async fn` in a trait is not dyn compatible, so it is written as a manual `BoxFuture` (with no new dependency).
pub trait ConnectionHandler: Send + Sync {
    /// Once, right after connect. A frame put in here is the first output **this handler emits**.
    ///
    /// ★It does not mean the first frame of the whole connection★: registration precedes this call, so in between the all-connection
    ///   fanout (`FrameFanout`) may have put frames into the same queue first. Do not assume it holds the FIFO head.
    ///
    /// ★No queue consumer yet — this is not a caller-side condition but an **implementation-side duty**★: this call
    ///   is awaited **before the writer task starts**, so frames put in here are taken by nobody.
    ///   So ① put in only a few frames and ② do not block for long for any other reason either. Violate this and
    ///   `FrameSink::send` never returns, and that connection remains **authenticated and registered but with no reader, no writer, no
    ///   timeout and no `on_disconnect`**.
    ///
    /// ★A safe upper bound cannot be nailed down as a number★: do not read the queue capacity (`ws::CONN_TX_CAP`) as that bound.
    ///   The connection is registered in the fanout registry already **before** this call, so from that moment the all-connection fanout
    ///   (`FrameFanout`) puts slots into the same queue concurrently. So the real headroom is **smaller than the capacity and variable**.
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

    /// Client → daemon binary frame. **Which frame kinds are valid is the upper-layer protocol's judgment**, so
    /// the network row does not reject and passes it up as is.
    fn on_binary<'a>(
        &'a self,
        conn_id: ConnId,
        payload: &'a [u8],
        frames: &'a Arc<dyn FrameSink>,
    ) -> BoxFuture<'a, ConnFlow>;

    /// Called only when a text frame arrives **while the arrival-order queue is full** — the normal-path cost is 0.
    ///
    /// ★What breaks without this branch★: if the read loop just waits when the queue is full, **the frames after it
    ///   are not even taken from the socket** — TCP is a stream, so there is no way to see later ones first. Then
    ///   even a cancel command that could unclog the blocked queue starves behind that queue. So instead of 「기다린다」 (waiting), it asks the
    ///   upper layer for the disposition of this one frame and **does not stop reading.**
    ///
    /// ★Implementation duty 1 — non-blocking · short★: it is sync and runs on top of the read loop. Doing I/O here stalls the socket by
    ///   that much. Do not do the actual handling; **only classify**.
    /// ★Implementation duty 2 — if you choose [`Saturated::Refused`], the reply **must already have been sent**★: the network row
    ///   drops that frame and keeps reading, so without a reply the sender gets neither an answer nor an error until its own
    ///   deadline. Silent loss is not the purpose of this branch. ★If that reply attempt **failed**,
    ///   return [`Saturated::Unanswered`]★ — discarding the result and returning `Refused` is pretending to have
    ///   kept that duty, and the network row has no means to trace it back.
    /// ★Implementation duty 3 — [`Saturated::Bypass`] only for **what may give up ordering**★: that frame is handled before and
    ///   overlapping those already in the queue (the exception to the contract above). Choosing this for something causally linked to an earlier frame becomes
    ///   a silent ordering defect.
    fn on_inbound_saturated(
        &self,
        conn_id: ConnId,
        text: &str,
        frames: &Arc<dyn FrameSink>,
    ) -> Saturated;

    /// Connection cleanup (release subscriptions · return resources). Sync — the cleanup path has no await point.
    ///
    /// ★Implementation duty — by the time it returns, **all** copies of the `FrameSink` it made must be released★: if the sinks made
    ///   through this port (especially those registered in a long-lived place) are not reclaimed here, the network row's send
    ///   queue copy survives and the writer task cannot end by itself
    ///   (a task that is not reclaimed even when the connection ends).
    ///   How the current implementation keeps this duty, and the known race that misses that reclaim, are in the
    ///   comment of `agent_conn::AgentConnection::on_disconnect` in the daemon crate — the same root as the abort-overlap race above.
    ///
    /// ★At this point this connection is still in the fanout registry★ (the network row unregisters **after** this
    /// call). Still, a broadcast going out from here **cannot be assumed to be delivered to itself**
    /// — in practice it is usually dropped but that is not guaranteed:
    /// - In the branch where the writer **ended first**, the queue's receiving end is already gone, so it is surely dropped.
    /// - In the branch where the reader ended first, the writer is only **marked** for `abort()`, so before the cancel
    ///   actually takes effect it may pick a frame already in the queue and send it out to the socket.
    /// So only one safe direction needs keeping — **do not write cleanup logic that presumes self-delivery.**
    ///
    /// ★Delivery consistency does **not** hang on this order★: the broadcast takes a snapshot of the registry map and
    /// walks it, so whether unregistration is moved earlier or later, **what the other connections receive is exactly the same**. The reason
    /// to keep this order is only to keep the ADR-0129 slice a pure refactor (the same order as HEAD).
    fn on_disconnect(&self, conn_id: ConnId);
}

/// The upper-layer factory that makes a handler per connection. Because `handle_connection` holds only this, it does not know the agent
/// vocabulary even as a type.
pub trait ConnectionHandlerFactory: Send + Sync {
    fn handler_for(&self, conn_id: ConnId) -> Arc<dyn ConnectionHandler>;

    /// A text frame that tells the client the handshake failed. At this point there is neither a connection queue nor a handler yet, and the network
    /// row does not know the error-expression vocabulary, so it takes only the encoding from the upper layer and passes it straight to the socket.
    /// `None` = encoding failed → only close, with no body.
    fn handshake_error_frame(&self, message: &str) -> Option<String>;
}
