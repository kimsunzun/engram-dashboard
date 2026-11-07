//! Supervisor task for one peer — ★the driving layer★.
//!
//! Carries out, on top of [`crate::link`] and [`crate::clock`], what [`crate::machine`] decided. If the pure layer
//! is "what", this file is "how".
//!
//! ## Invariants this file upholds
//!
//! - ★Single writer★ — only the supervisor task holds [`LinkTx`]. Everything from outside goes through the queue.
//! - ★The pending slot is registered **before sending**★ — reverse it and a fast reply cannot find its slot (the shell has this today too).
//! - ★**Every** branch that drops the connection wakes the waiters★ — not only those in the table but **also requests
//!   left in the queue and requests that arrive after leaving the operating
//!   phase**. If even one is missed, that slot waits forever: the given-up state
//!   has no loop to run the deadline, and the waiting caller holds the handle, so the channel never closes either.
//! - ★Every write carries a deadline★ — `write_deadline < ping_interval`, so a write in progress cannot
//!   starve keepalive. The real defect in `net` today is exactly that starvation.
//! - ★The handle is immortal across reconnects★ — a generation boundary does not kill the handle;
//!   it shows up as a marker on [`crate::Arrival::generation`].
//! - ★A command never leaves the generation it is bound to (TRD 3 §3-0 ③)★ — every command the handle queues is stamped with a generation (for `*_in`,
//!   the value the caller gave; for the rest, the current generation at the moment of the call), and the supervisor compares it with the current generation just before writing and does not write if they differ
//!   (`Supervisor::on_cmd`). The queue survives reconnects, but what the machine empties when the link drops is only the queue of that moment, so this comparison closes the gap in which something that
//!   entered afterwards passed through the connection-establishment phase and was written on the next connection. A request that is not written wakes its waiter as a not-sent disconnect, and
//!   a notification, a fanout and a resume are counted — the same disposal as a command that entered while not in the operating phase (`reject_command`). The handle entry point
//!   makes the same comparison first, but that is the fast path; the authority is on the supervisor side.
//! - ★Everything sent upward passes through the ordering area (the supervisor-local queue) and is never dropped★ (TRD 3 §3-0 ②) — the place that
//!   enqueues (event · packet · deadline event · loss report) only enqueues synchronously, and if the line has room it loads right there. If the line
//!   is full it holds the item back, and the 「줄에 자리 → 순서 칸 비우기」 (room in the line → drain the ordering area) branch of **every waiting loop** (connection
//!   establishment · operating · backoff · given-up) does the loading — if even one is missing, what was held back does not go out while staying in that state. While a write is being pushed
//!   that branch is absent, so loading stops — for one write up to `write_deadline`, and at a place that pushes writes
//!   back to back (draining the queue in `Turn::Outbound` · `reissue_resumes`), up to that number of writes × `write_deadline`.
//!   So what one supervisor sends upward is loaded in the order it was produced. Things are dropped only when the receiving side has closed and when the teardown
//!   limit (`CLOSE_DRAIN_LIMIT`) has been exceeded.
//! - ★When the line is full, that peer's reading stops (backpressure — TRD 3 §3-0 ①)★ — while the ordering area holds anything, the operating
//!   loop pulls the frame arm. Then the read-ahead queue ([`Policy::frame_buffer`]) fills and the read task
//!   stops reading the channel. Separately, if the received bytes (minus the resume-window share) reach the read share (all of `BYTE_BUDGET` if streams are not used,
//!   half of it if they are), the read task does not do its next read (`budget.rs` — one count covers the read-ahead queue · the ordering area · the upward-going
//!   queue · the window). So what the ordering area
//!   holds is what one frame (or one timer turn) processed after the line filled produced, plus a few lifecycle events, and what was salvaged from the read-ahead queue
//!   when the channel was dropped (next item) — what teardown must empty from an old supervisor's backlog is that much too, so if the receiving
//!   side is draining, it finishes within `CLOSE_DRAIN_LIMIT` (those bytes are within the count).
//!   ★What was dropped past the limit is neither counted nor reported★ — teardown releases the enqueue end, and the receiver end (`None`) reports that drop
//!   (TRD 3 §3-0 ②).
//! - ★Stream pieces the resume window holds do not stop reading★ — within the same count they are counted separately as the window share and
//!   subtracted from the read gate, and the window share is measured per supervisor against the window cap (half of the stream budget) — a piece that would exceed it if held is not held, and the window is
//!   closed as truncated (`Supervisor::route`). Stopping reading would leave the replay that fills the hole unread, and a hole that could have been filled would
//!   become a permanent loss. One supervisor's byte ceiling is `BYTE_BUDGET` (= read share + window cap) + one message
//!   (`budget.rs` header).
//! - ★While holding, the silence decision and the resume window's clock are stopped★ — what is blocked is our side, so we can read neither the peer's liveness
//!   nor the answer to a re-request. At the moment it clears, `last_rx` is set to that time and the requested time of an open window is
//!   pushed back by the held time (`Supervisor::track_pause`). The held time is counted from when the hold actually started, not from when it was noticed —
//!   even if it started while waiting on a write, and a hold the supervisor never saw because the read gate closed and reopened in between is counted too.
//!   The window may close later than the held time but never earlier (the safe side — see `track_pause`).
//!   Our ping goes out even meanwhile — the peer does not cut us for silence.
//!   The cost is that that peer's replies cannot come in either — if you wait on [`Peer::request`] on the task that drains the receiving side, that
//!   request hangs until its deadline (crate header 「받는 쪽 불변식」 (receiver invariants) 1).
//! - ★If the receiving side drops or closes the receiver end, fold immediately★ (crate header 「받는 쪽 불변식」 (receiver invariants) 5) — every waiting loop (connection
//!   establishment · operating · backoff · given-up) and writes (`write` · `ping`) watch for the receiver being closed (`Sender::closed`), so even with nothing to enqueue
//!   (even if the peer is quiet) they go straight to closing on the spot — they do not reconnect. The channel and handshake being dialed are discarded
//!   as the closed arm wins — if the handshake has just finished and the finished arm wins (`select!` is not biased), `shake_hands` draws a
//!   generation and the first turn of the operating phase sends the close and folds (harmless · no re-check is placed before the draw).
//!   The check also runs before polling the dial phase, so a receiver end dropped before the supervisor starts dialing does not dial a channel. A reply-less frame
//!   that was being written and is dropped is counted like one dropped for any other reason (`write_or_count`). The only thing that does not watch is the single close of `drop_link` —
//!   it pushes the close code through to the end, so it deliberately does not listen to control · receiver closure (it listens only to the start of the pre-close flush and shortens to that deadline), and it takes at most `write_deadline`.
//!   Teardown has nowhere to load, so it does not wait on the ordering area (`Closed` is not loaded either).
//! - ★What was already read is handed over before dropping the channel★ — when leaving the operating phase (the app closed · reconnect · write failure ·
//!   silence …), the read task is folded and the read-ahead queue is drained without waiting and sent the same way as during operation
//!   (`Supervisor::salvage_read_ahead`). This happens before the machine wakes the waiters and emits the disconnect event, so replies go to the waiting
//!   caller and everything else goes ahead of the disconnect event. Even under a policy that disconnects on decode failure, what was already read after that is handed over.
//!   What remains in the channel (not yet read) is dropped with it. If the reader's end (the peer's close · channel error · disconnect on decode failure —
//!   the frontmost item) was already in that line, only our side's disconnect cause (write failure · silence · overflow …) is rewritten to that —
//!   it is an end that was already read before the channel was dropped. If the app closed or asked to reconnect, it stays `Explicit` (`earlier_end`).
//! - ★A held stream piece goes up under the generation of the connection that first carried it★ — the resume window survives re-attaching and may be closed on the next
//!   connection, but [`crate::Arrival::generation`] points to the connection that carried that piece.
//! - ★Teardown order = drop the channel → wake the waiters · close the outbound queue and reject the remaining commands → fold the open resume window (what was held
//!   goes up together with the truncation report) → drain the ordering area → [`TransportEvent::Closed`] last → release the enqueue end★ — what waits on the receiving
//!   side is only the draining of the ordering area (`teardown`). The enqueue end is held by one supervisor alone (`PeerConfig::inbound`), so when the supervisor
//!   ends, the next `recv` on that handle's receiver end is `None`.
//! - ★An accepted peer (`Registry::attach`) does not reconnect (TRD 3 §3-1 ①)★ — it starts from the handshake already holding the channel
//!   (the machine's dial is repaid on the spot as 「열렸다」 (opened) — `Supervisor::run`), and every end where the machine would go to reconnect · give up
//!   (handshake failure · disconnect during operation) is closed immediately with the same fold as the pre-close flush (`Supervisor::fold_into_close`).
//!   ★The machine (pure layer) keeps its dialing-side shape★ — putting an accepted-peer branch in the machine would make every one of those ends (reject · deadline · disconnect ·
//!   reconnect) carry one more branch each, while the supervisor side already has the 「다시 걸지 않고 닫는다」 (close without reconnecting) fold. The handshake deadline
//!   runs from the end of the upgrade (`Supervisor::shake_hands`). The end cause rides on [`TransportEvent::Closed`]
//!   (`Supervisor::end_cause`) and when it ends it removes itself from the registry (`RosterExit`).
//! - ★The pre-close flush is done with one marker in the data queue (TRD 3 §3-0 ④)★ — [`Peer::close_after_flush`] first writes the close to
//!   be loaded (that is the 「비우는 중」 (draining) indication — after that the handle's sends are rejected) and puts the marker at the end of the data queue. While
//!   operating, the supervisor writes the commands before the marker in order and closes at the marker (`Supervisor::on_cmd` — the marker is not bound to a generation). The waiting loops outside
//!   operation (connection establishment · backoff · given-up) see that indication and fold immediately — even if disconnected mid-drain it does not reconnect:
//!   it withdraws the part the machine produced for reconnect · give-up (waiting · `GaveUp` · `ConnectFailed` reports) and folds straight into the close
//!   (`Supervisor::fold_into_close` — the machine is left pure). While draining, 「지금 다시 해라」 (retry now) is dropped too (even
//!   one that entered before it and has not been read yet — `Supervisor::drain_ctl`). One deadline ([`Policy::flush_timeout`]) covers, from when the supervisor
//!   notices, the later writes · keepalive · close frame together (`Supervisor::guard` · `drop_link` — even if the drain starts during that
//!   wait, it shortens on the spot) — that deadline runs even when the receiving line is backed up (`wake_at`).
//!
//! ## Priority of the operating loop (do not revive)
//!
//! The old edition set `select!` to `biased` and put the frame arm ahead of the timer and the outbound queue. `biased` polls in declaration
//! order and **stops at the first ready arm**, so even if frames arrive only as fast as they are processed, the two later arms
//! **are never polled even once.** Then no ping goes out and the peer cuts us for silence, the request deadline does not run so
//! waiters hang past the limit, and the outbound queue fills so the default policy disconnects the connection by itself — the shape is ★it picks the moment of greatest
//! output and disconnects★. The current loop ① collects control first on every turn, ② skips
//! `select` if a deadline has already passed, and ③ yields one turn to the outbound queue after every [`FRAME_BURST`] frames processed.
//!
//! ## Why there are two channels
//!
//! The data queue (bounded) and the control queue (unbounded) are separated. ★Even at the moment the queue is full, 「닫아라」 (close) and 「지금 다시 붙어라」 (reconnect now) must
//! get in★ — with one channel, backpressure blocks control too.

use std::collections::VecDeque;
use std::future::Future;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::sync::{Arc, OnceLock, Weak};
use std::time::{Duration, Instant};

use futures_util::future::OptionFuture;
use tokio::sync::{mpsc, oneshot, watch, Notify};

use crate::budget::{Charge, Charged, Ledger};
use crate::clock::{with_deadline, Clock};
use crate::event::{Arrival, Direction, Incoming, PeerId, TransportEvent};
use crate::frame::{Close, CloseCode, Frame};
use crate::link::{
    Address, ConnPermit, DialFailure, Dialer, Handshake, HandshakeStep, LinkError, LinkRead,
    LinkRx, LinkTx, RejectKind,
};
use crate::machine::{
    Action, ConnectFailure, DisconnectCause, Generation, Input, Machine, MachineEvent, PeerState,
};
use crate::pending::{Insert, Pending, RequestError};
use crate::policy::{DecodeFailurePolicy, Policy, QueueFullPolicy};
use crate::stream::{Streams, Truncation, Verdict};
use crate::wire::{DecodeClass, Wire};

/// After this many frames processed in a row, yield one turn to the outbound queue.
const FRAME_BURST: usize = 32;

/// The upper limit on how long teardown waits for the ordering area to be fully loaded. If exceeded, the rest — up to the last [`TransportEvent::Closed`] —
/// is dropped and it ends (releasing the enqueue end makes the receiver end `None` report that drop). It is a value that keeps the supervisor
/// from hanging on a receiving side that stays alive and does not drain.
///
/// Why 60 s = it covers the worst case where the daemon read task waits on the priority line (8 slots × kill-join 5 s) (TRD 3 §3-0 ②).
pub(crate) const CLOSE_DRAIN_LIMIT: Duration = Duration::from_secs(60);

/// The budget of payload bytes one supervisor may hold (B in `budget.rs`). One count is split between the read share and the window cap —
/// the read gate measures by the read share (if the received bytes minus the resume-window share reach it, the read task stops reading the channel).
/// ★If streams are not used, read share = all of this (window cap 0), and after the first stream piece is seen, it is half and half★
/// (`budget::split` · the reason is in the `budget.rs` header). Reading decides by the 「더하기 전 물높이」 (fill level before adding), so either way one supervisor's
/// ceiling is this + one message (TRD 3 §3-0 ① 「B + M」).
///
/// Why 16 MiB = the same value as net's per-connection receive-line budget (`CONN_RX_MAX_BYTES`) — shell · daemon use the same value
/// (TRD 3 §3-0 ① draft · an internal constant the main session decides and reports). The two consumers that do not use streams (ADR-0310 decision 16) get
/// a read gate of the same 16 MiB as net, which is why the split is deferred. A flood of small frames is handled by the count limits
/// ([`Policy::frame_buffer`] · [`Policy::inbound_queue`]).
pub(crate) const BYTE_BUDGET: usize = 16 << 20;

/// What this turn of the operating loop does first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Turn {
    /// A deadline has already passed — skip `select` and look at the timer first.
    Timer,
    /// Drain this many from the outbound queue.
    Outbound(usize),
    /// Normal — wait fairly on the four arms.
    Wait,
}

/// ★The only place where the anti-starvation rule lives★.
///
/// That it is a pure function is intended: the property this rule protects (the timer and the outbound queue
/// keep running even when frames never stop) is **not reproduced by an in-memory harness** — there the read task is a structural bottleneck so
/// the frame queue is always empty, and then whatever `select` does, the later arms get polled (measured: a mutant that revived `biased`
/// turned not a single flood test red). So **the rule itself is asserted here**.
fn next_turn(
    now: Instant,
    wake: Instant,
    frames_in_row: usize,
    outbound_len: usize,
    outbound_cap: usize,
) -> Turn {
    // TODO(stream-replay-vs-timer): the deadline is ahead of frames, so if the window deadline passes mid-write and replay arrived in the read-ahead
    //   queue in the meantime, the resume window closes before that replay is processed — even without backpressure (crate header 「알려진
    //   한계」 (known limits)).
    if wake <= now {
        return Turn::Timer;
    }
    if outbound_len == 0 {
        return Turn::Wait;
    }
    // ★A fixed ratio alone is not enough★ — if one is taken out every 32 frames, then against a producer above that ratio
    //   the queue still grows and ends in `QueueFull` (the same shape with the threshold only 32x higher). So the rule looks at
    //   **depth**: past half it does not wait for the burst, and when it yields it drains as much as has piled up.
    if outbound_len.saturating_mul(2) >= outbound_cap || frames_in_row >= FRAME_BURST {
        return Turn::Outbound(outbound_len);
    }
    Turn::Wait
}

/// Could not push in something that does not wait for a reply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendError {
    /// The outbound queue is full.
    QueueFull,
    /// Not in the operating phase now (connecting · backing off · given up) — or the bound connection is not the current one ([`Peer::notify_in`]).
    Disconnected,
    /// Nothing more can be sent to this peer — the supervisor task is gone or [`Peer::close_after_flush`] put in the close marker.
    /// ★Waiting for a re-attach will not help★ — this differs from [`SendError::Disconnected`].
    Closed,
}

impl std::fmt::Display for SendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::QueueFull => f.write_str("outbound queue full"),
            Self::Disconnected => f.write_str("peer is not live on the bound generation"),
            Self::Closed => f.write_str("peer supervisor is gone"),
        }
    }
}

impl std::error::Error for SendError {}

/// One data command going to the supervisor — ★carries the generation the caller captured★ (TRD 3 §3-0 ③).
///
/// The outbound queue outlives the connection (the handle is immortal across reconnects). When the machine drops a connection it empties the queue (`fail_pending`),
/// but it cannot see what entered after that, and the connection-establishment phase (dial · handshake) does not read the queue, so that stays until the next operating phase —
/// a command that entered in generation G is written on G+1's channel (the transport version of the defect the shell blocked with ADR-0195). Just before writing, the supervisor compares this
/// generation with the current generation and does not write if they differ (`Supervisor::on_cmd`) — that one comparison is the place that catches the gap.
struct PeerCmd<W: Wire> {
    /// The only connection this command may go out on. Stamped by the handle's entry point — for `*_in`, the value the caller gave; for the rest, the current
    /// generation at the moment of the call ([`Peer::generation`]).
    generation: Generation,
    op: PeerOp<W>,
}

enum PeerOp<W: Wire> {
    Request {
        out: W::Out,
        reply: oneshot::Sender<Result<W::In, RequestError>>,
    },
    Notify {
        out: W::Out,
    },
    /// Fanout — the registry encodes once and hands it out as many times as there are peers.
    Raw(Frame),
    Resume {
        key: W::StreamKey,
        after: Option<u64>,
    },
    /// The marker of the pre-close flush (TRD 3 §3-0 ④ · [`Peer::close_after_flush`]) — an operating supervisor that reaches it closes.
    /// What to load and close with is held not by the marker but by `PeerInner::goodbye` (it loads that close even if the queue is full and the marker cannot be put in).
    ///
    /// ★Not bound to a generation★ — if bound, the marker would be rejected at a moment that crosses a re-attach and that peer would never close. The
    /// commands before the marker each still get the generation comparison as is (the old generation's are not written).
    ///
    /// ★It holds the control channel's sender end — it does not send★: when all handles are gone the control channel closes and the supervisor
    /// reads that as 「닫아라」 (close) and folds immediately. While the marker is in the queue that channel does not close, so the drain of a side that released its handle
    /// to flush and leave is not cut off before the marker.
    Close {
        _keep_ctl: mpsc::UnboundedSender<Ctl>,
    },
    /// [`Peer::drop_link_in`] — the bound connection ends as a lost link with `cause`.
    DropLink {
        cause: DisconnectCause,
    },
}

enum Ctl {
    ReconnectNow,
    Close,
}

/// The minimal representation that lets the handle side read the last known disconnect cause as well.
fn cause_code(cause: DisconnectCause) -> u8 {
    match cause {
        DisconnectCause::PeerClosed => 0,
        DisconnectCause::ReadFailed => 1,
        DisconnectCause::Idle => 2,
        DisconnectCause::WriteDeadline => 3,
        DisconnectCause::QueueFull => 4,
        DisconnectCause::Explicit => 5,
        DisconnectCause::WriteFailed => 6,
    }
}

fn cause_from_code(code: u8) -> DisconnectCause {
    match code {
        0 => DisconnectCause::PeerClosed,
        1 => DisconnectCause::ReadFailed,
        2 => DisconnectCause::Idle,
        3 => DisconnectCause::WriteDeadline,
        4 => DisconnectCause::QueueFull,
        5 => DisconnectCause::Explicit,
        // Only `cause_code` writes this field — the only remaining value is 6.
        _ => DisconnectCause::WriteFailed,
    }
}

struct PeerInner<W: Wire> {
    id: PeerId,
    data_tx: mpsc::Sender<PeerCmd<W>>,
    ctl_tx: mpsc::UnboundedSender<Ctl>,
    state_rx: watch::Receiver<PeerState>,
    generation: Arc<AtomicU64>,
    /// The number of times the handle could not enqueue **because the queue was full**. The supervisor collects it and handles it per [`QueueFullPolicy`].
    overflow: Arc<AtomicU64>,
    /// The newest of the generations stamped on commands that could not be enqueued (`fetch_max` — written before the count). ★The supervisor enforces
    /// [`QueueFullPolicy`] only when this is the current generation★ — so the next generation does not collect an ended generation's overflow and cut that new connection (the count is
    /// reported regardless of generation — `Supervisor::drain_overflow`).
    overflow_generation: Arc<AtomicU64>,
    /// The number of times the handle dropped something **because the generation it bound was not operating** (not in the operating phase, or re-attached under a newer generation).
    /// ★It does not cut the connection★ — the bound connection is already not attached.
    refused: Arc<AtomicU64>,
    /// Wake the supervisor because `overflow` · `refused` was raised — given-up · backoff waits are not woken by that count alone, and without waking, that
    /// report would not come out until the next 「다시 연결」 (reconnect) (if given up) · the end of the backoff (if backing off). The leftover ticket is at most one so it does not pile up,
    /// and a leftover wake-up is harmless (the start of the turn collects again). The connection-establishment phase (dial · handshake) does not listen to this —
    /// what was counted in that phase is reported by the first turn after the phase ends.
    loss_wake: Arc<Notify>,
    /// The last disconnect cause. If it never attached, it is [`DisconnectCause::WriteFailed`] — neither the read nor the write end is accurate,
    /// so we chose the outbound end, which is closer to the receiver of this value (a request that could not go out because there was no channel to load it on).
    last_cause: Arc<AtomicU8>,
    /// The app called [`Peer::close`] · [`Peer::close_after_flush`]. It is set at the moment of the call, regardless of whether the supervisor has
    /// handled it.
    close_requested: AtomicBool,
    /// The close that [`Peer::close_after_flush`] asked to be loaded — only the first call is written. ★What is written is itself the 「비우는 중」
    /// (draining) indication★: when this stands, the handle's sends are rejected and the supervisor does not reconnect. It is written before the marker — so that it is visible
    /// when the supervisor takes the marker out.
    goodbye: Arc<OnceLock<Close>>,
    /// Wake the supervisor because `goodbye` was written — for the places that do not read the data queue (the connection-establishment phase · mid-write).
    /// The leftover ticket points to a fact that is always true afterwards, so it is harmless.
    flush_wake: Arc<Notify>,
    /// It is an accepted peer (`Origin::Accepted`) — it does not reconnect.
    accepted: bool,
    /// The counter from which this supervisor draws generations (`PeerConfig::generations`) — whoever replaces the same name tag inherits it.
    generations: Arc<AtomicU64>,
    #[cfg(test)]
    ledger: Arc<Ledger>,
}

/// A cheap-to-clone handle = the command channel to the supervisor task.
///
/// ★It survives across reconnects★ — this handle does not die because of a disconnect. In exchange for keeping consumer code short,
/// a **generation marker is attached on the data side** so that no silent loss occurs (the idiom of ADR-0163/0164).
pub struct Peer<W: Wire> {
    inner: Arc<PeerInner<W>>,
}

impl<W: Wire> Clone for Peer<W> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<W: Wire> Peer<W> {
    pub fn id(&self) -> &PeerId {
        &self.inner.id
    }

    /// One snapshot of the current state.
    pub fn peer_state(&self) -> PeerState {
        *self.inner.state_rx.borrow()
    }

    /// A handle that watches state changes.
    pub fn state(&self) -> watch::Receiver<PeerState> {
        self.inner.state_rx.clone()
    }

    /// The current connection generation. If it never attached, it is the floor value the registry gave.
    pub fn generation(&self) -> Generation {
        Generation(self.inner.generation.load(Ordering::Acquire))
    }

    /// Waits for the reply — bound to the current generation at the moment of the call (the same as passing
    /// [`Peer::generation`] to [`Peer::request_in`]).
    ///
    /// Error conditions: deadline expiry · disconnect · queue saturation · number clash · reuse of an expired number ·
    /// [`Wire::request_tag`] is `None` ([`RequestError::NotARequest`] — meaning use `notify`).
    ///
    /// ★The only case where the deadline really arrives is 「연결은 멀쩡한데 그 요청만 침묵」 (the connection is fine but only that request is silent)★ — because
    /// any branch that drops the connection wakes the waiters immediately.
    ///
    /// ★A call that overlaps a reconnect does not carry over to the new connection★ — the generation is read first and whether it is operating is checked after, so if it
    /// re-attached in between, the call ends at the entry point, and if it re-attached after that, it ends at the supervisor's just-before-write comparison (both are not-sent
    /// errors). It is not used on a connection newer than the one at the moment of the call.
    ///
    /// ★It is not an `async fn` — do not revive it★: the generation is read at the place of the call and loaded into the returned future. With `async fn`
    /// the body runs on the first poll, so if it re-attaches after the future is made and then you wait, it reads the new generation and is used on the new connection.
    pub fn request(&self, out: W::Out) -> impl Future<Output = Result<W::In, RequestError>> + '_ {
        let generation = self.generation();
        self.request_in(generation, out)
    }

    /// Waits for the reply — ★goes out only on the `generation` connection★ (TRD 3 §3-0 ③).
    ///
    /// If that connection is not operating now (it dropped · it re-attached under another generation), it does not send and ends as a not-sent disconnect
    /// ([`RequestError::Disconnected`] with `sent` false). ★The entry check is the fast path and the authority is the supervisor's just-before-write
    /// comparison★ — if the generation changes after passing the entry and before writing, the supervisor does not write and wakes it with the same error.
    ///
    /// Use = when issuing a request tied to a received message only on the connection that carried that message, pass its [`crate::Arrival::generation`].
    /// It is the place where the shell grabbed the socket before the awaited send (TRD 3 §3-0 ③). The remaining error conditions are the same as
    /// [`Peer::request`] — if first polled after [`Peer::close_after_flush`], it does not send and is a not-sent disconnect (`cause` =
    /// [`DisconnectCause::Explicit`]).
    ///
    /// ★The first poll queues the request (or ends the call) before the first wait★ — a consumer may poll the future
    /// once to place the request in the outbound queue at that point, ahead of what it queues next, and await the reply
    /// elsewhere (the shell driver's registration). Keep every check and the queue push ahead of the reply wait.
    pub async fn request_in(
        &self,
        generation: Generation,
        out: W::Out,
    ) -> Result<W::In, RequestError> {
        if self.flushing() {
            return Err(RequestError::Disconnected {
                generation,
                cause: DisconnectCause::Explicit,
                sent: false,
            });
        }
        if !self.bound_to(generation) {
            return Err(self.disconnected(generation, false));
        }
        let (tx, rx) = oneshot::channel();
        let cmd = PeerCmd {
            generation,
            op: PeerOp::Request { out, reply: tx },
        };
        match self.inner.data_tx.try_send(cmd) {
            Ok(()) => {}
            Err(mpsc::error::TrySendError::Full(_)) => {
                self.count_overflow(generation);
                return Err(RequestError::QueueFull);
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                return Err(self.disconnected(generation, false))
            }
        }
        // Even if the supervisor vanishes and the slot is dropped with it, it wakes here with an error. ★The sent field is true★ — there is no way to know whether it was in the queue or in the wait table,
        //   so it is written toward 「갔을 수 있다」 (it may have gone) (the 「부풀 수 있다」 (may be inflated) of the `sent` field).
        rx.await
            .unwrap_or_else(|_| Err(self.disconnected(generation, true)))
    }

    /// Does not wait for a reply — bound to the current generation at the moment of the call (the same as passing [`Peer::generation`] to
    /// [`Peer::notify_in`]). ★No await★ — if it cannot be queued, `Err` at once.
    ///
    /// A call that overlaps a reconnect does not carry over to the new connection, same as [`Peer::request`] — if it ends at the entry, `Err`; if it ends at the just-before-write
    /// comparison, it is counted as [`TransportEvent::Dropped`] ([`Peer::notify_in`]).
    pub fn notify(&self, out: W::Out) -> Result<(), SendError> {
        self.notify_in(self.generation(), out)
    }

    /// Does not wait for a reply — ★goes out only on the `generation` connection★ (TRD 3 §3-0 ③). ★No await★.
    ///
    /// If that connection is not operating at the entry, `Err` ([`SendError::Disconnected`] — not counted; the caller
    /// got the error). After [`Peer::close_after_flush`] it is [`SendError::Closed`] (also not counted). ★If the generation changes after passing the entry and before
    /// writing, the supervisor does not write and counts it as [`TransportEvent::Dropped`] ([`crate::Direction::Outbound`])★ —
    /// the caller already took an `Ok`, so not counting would be a silent loss (the same rule as a notification disconnected while in the queue).
    ///
    /// Use = when answering a received request only on the connection that request came on, pass its [`crate::Arrival::generation`] — if it
    /// goes out on another connection, that peer gets an answer to a request it did not send (the shell's `CommandOutcome` — TRD 3 §3-0 ③).
    pub fn notify_in(&self, generation: Generation, out: W::Out) -> Result<(), SendError> {
        if self.flushing() {
            return Err(SendError::Closed);
        }
        if !self.bound_to(generation) {
            return Err(SendError::Disconnected);
        }
        self.push(PeerCmd {
            generation,
            op: PeerOp::Notify { out },
        })
    }

    /// Resumes a stream. The return value is the connection generation this request is bound to — the current generation at the moment of the call, and this request goes
    /// out only on that connection (the same comparison as [`Peer::notify`]).
    ///
    /// ★It does not mean accepted★ — if not in the operating phase, the queue is full, or the generation changed before writing, the request is
    /// dropped and shows up only as [`TransportEvent::Dropped`]. The return value only says "bound to this generation".
    /// ★This request, if disconnected mid-write, does not show up even as that★ — it is neither counted nor reissued on the next connection (what
    /// is reissued is only the re-request this crate itself issued after seeing a hole).
    pub fn resume_stream(&self, key: W::StreamKey, after: Option<u64>) -> Generation {
        let generation = self.generation();
        // This is after the app said it would close — neither sent nor counted ([`Peer::close_after_flush`]).
        if self.flushing() {
            return generation;
        }
        if self.bound_to(generation) {
            let _ = self.push(PeerCmd {
                generation,
                op: PeerOp::Resume { key, after },
            });
        } else {
            // An entry point where the caller gets no error (it and the fanout `send_frame` make two) — not counting would be a silent loss.
            self.refuse();
        }
        generation
    }

    /// Is the `generation` connection operating now — the fast path of the data entry. ★Not the authority★ — even if this is true, the generation may change before
    /// writing, and the supervisor's just-before-write comparison catches that (`Supervisor::on_cmd`).
    /// ★Look at the state first★ — the supervisor publishes the generation before the state (`Supervisor::publish`), so a generation read after seeing operating
    /// is that operating phase's or a newer one. If the generation is read first, an operating phase re-attached afterwards would be read as the old generation's,
    /// and something that could have ended here with `Err` is put in the queue (the supervisor drops and counts it).
    fn bound_to(&self, generation: Generation) -> bool {
        self.peer_state().is_live() && self.generation() == generation
    }

    /// [`Peer::close_after_flush`] put in (tried to put in) the marker — after that the data entry rejects everything.
    fn flushing(&self) -> bool {
        self.inner.goodbye.get().is_some()
    }

    /// ★The 「지금 다시 해라」 (retry now) entry★ — resets the budget and tries at once (ADR-0180 decision 7). It listens even in the given-up state.
    /// ★It does not listen after [`Peer::close_after_flush`]★ — that is a peer we decided to drain and close. Even if called before it, if the supervisor
    /// has not read it yet it is dropped (the supervisor drops a reconnect it read after seeing the drain — listening would drop the channel, so what was to be drained is rejected too and
    /// it ends without the close it was given). If the supervisor already read it and went to drop the channel, that drain closes at once with no connection to write on.
    ///
    /// ★An accepted peer ([`crate::Registry::attach`]) does not listen★ — it has no address to dial. If an operating channel were dropped, that
    /// peer would end with no way to re-attach.
    pub fn reconnect_now(&self) {
        if self.flushing() || self.inner.accepted {
            return;
        }
        let _ = self.inner.ctl_tx.send(Ctl::ReconnectNow);
    }

    /// Ends connection `generation` as a lost link with `cause` — the same path as a read failure: `Disconnected`, flap
    /// accounting, and the backoff before the next dial (an accepted peer ends, as on any link loss). ★Unlike
    /// [`Peer::reconnect_now`] it keeps the budget and the backoff★ — a consumer that drops a connection over what the
    /// remote sent must not turn a remote that keeps sending it into a reconnect loop.
    ///
    /// ★Only that connection★ — a no-op when `generation` is not the live one, at the entry and again when the
    /// supervisor takes it (it never drops a newer connection). It goes through the outbound queue, behind what is
    /// already queued; a full queue counts as an overflow of `generation` and [`crate::QueueFullPolicy`] decides — the
    /// default `Disconnect` drops the link anyway, ★under `DropAndReport` the drop itself is lost★ (counted as one dropped
    /// outbound frame). Ignored after [`Peer::close_after_flush`]; ★a drop queued before it ends that flush early★ —
    /// the supervisor reaches the drop ahead of the flush marker, loses the link with the drop's `cause` and closes
    /// without writing what was queued behind it.
    pub fn drop_link_in(&self, generation: Generation, cause: DisconnectCause) {
        if self.flushing() || !self.bound_to(generation) {
            return;
        }
        let _ = self.push(PeerCmd {
            generation,
            op: PeerOp::DropLink { cause },
        });
    }

    /// Explicit close — does not re-attach. It does not wait for what is in the queue (the control channel overtakes the data queue) — to drain and
    /// close, use [`Peer::close_after_flush`]. If that was already called, it loads the close that call wrote without waiting for its marker.
    ///
    /// ★There is no completion signal★ — at the moment of return the supervisor may still be closing the channel, and in that time it may finish the handshake and
    /// **issue one more generation**. The side that raises the same name tag again absorbs that overlap with
    /// the **per-name-tag shared generation counter** (`Registry::add_with_policy`) — it does not remove the overlap
    /// but is a device that keeps numbers from clashing even when they overlap.
    pub fn close(&self) {
        self.inner.close_requested.store(true, Ordering::Release);
        let _ = self.inner.ctl_tx.send(Ctl::Close);
    }

    /// After writing all the outbound work accepted so far, it loads `code` · `reason` and closes (TRD 3 §3-0 ④). The returned future is released when the supervisor
    /// ends — it finished teardown and loaded [`TransportEvent::Closed`], or the receiving side is gone, or it ended abnormally.
    /// If the supervisor is already gone, it is released at once.
    ///
    /// ★The marker enters the end of the data queue at the place of the call★ — the close proceeds even if you do not await the future, and dropping it does not
    /// cancel. An operating supervisor writes the commands before the marker in order (each still gets its generation comparison — [`Peer::notify_in`]) and
    /// closes when it reaches the marker. ★It does not wait for replies to requests★ — a request before the marker, after being written, wakes at the close as a sent disconnect.
    ///
    /// ★One deadline★ — [`Policy::flush_timeout`] covers from when the supervisor notices this close through the close frame (it is not
    /// [`Policy::write_deadline`] per write). If exceeded, it drops the frame being written, rejects the remaining commands, and closes —
    /// the close frame also gets only the remaining time (if past, it is just pushed once), so on a blocked channel it disconnects with no code.
    ///
    /// ★Cases that close at once★ — the queue is full and the marker could not be put in (it loads the same close and acts like [`Peer::close`] — what is in the queue is not written) ·
    /// not in the operating phase (no connection to write on). ★If the connection drops mid-drain, it closes without reconnecting★ — because
    /// what was to be loaded onto that connection. If it was dialing, that attempt is discarded. That disconnect goes up with its cause as is ([`TransportEvent::Disconnected`]'s `cause` ·
    /// the close frame too carries the code of that cause, not the close it was given) and then `Closed` at once — ★the [`TransportEvent::GaveUp`] · [`TransportEvent::ConnectFailed`]
    /// the reconnect machine would emit on that disconnect are not sent up, and [`Peer::state`] does not pass through
    /// [`PeerState::Backoff`] · [`PeerState::GaveUp`]★ (the shell shows a reconnect popup on `GaveUp` —
    /// ADR-0180).
    ///
    /// ★A send after the marker is rejected on every clone★ — `notify` · `notify_in` · fanout = [`SendError::Closed`] ·
    /// `request` · `request_in` = [`RequestError::Disconnected`] (`cause` = [`DisconnectCause::Explicit`] · `sent`
    /// false) · `resume_stream` = not sent. Since this is after the app said it would close, this rejection is not counted (that decision in `lib.rs` 「알려진 한계」 (known limits)). `reconnect_now` is not
    /// listened to — even what was called before this call and the supervisor has not read yet is dropped. Another clone's send that crosses with this call may land after the marker — the supervisor
    /// rejects it while closing and counts it as [`TransportEvent::Dropped`].
    ///
    /// ★Until it ends, hold and drain that handle's receiver end★ (crate header 「받는 쪽 불변식」 (receiver invariants) 5) — if you drop or close it, the supervisor folds
    /// at once and the commands before the marker are dropped too. ★Do not await this future on the task that drains that receiver end★ — the supervisor's end
    /// depends on loading everything teardown holds (on the receiving side draining), so if the side that would drain stands in this wait, it
    /// stalls up to the teardown limit (crate header 「알려진 한계」 (known limits)).
    ///
    /// Calling again leaves the first close as is and waits for the same end. Calling [`Peer::close`] after that closes at once without waiting for the marker,
    /// but loads the first close. Even if all handles are released, the drain continues (the marker holds the control channel).
    ///
    /// ★The output being `()` is intended★ — whether it all went out is told by the receiver end: what could not be written and has no reply is counted by the
    /// [`TransportEvent::Dropped`] ([`Direction::Outbound`]) before `Closed`. We do not create a second source.
    pub fn close_after_flush(
        &self,
        code: CloseCode,
        reason: impl Into<String>,
    ) -> impl Future<Output = ()> + Send + 'static {
        let finished = self.finished();
        if self.inner.goodbye.set(Close::new(code, reason)).is_ok() {
            self.inner.close_requested.store(true, Ordering::Release);
            let marker = PeerCmd {
                generation: self.generation(),
                op: PeerOp::Close {
                    _keep_ctl: self.inner.ctl_tx.clone(),
                },
            };
            match self.inner.data_tx.try_send(marker) {
                Ok(()) => {}
                // Could not put in the marker — close at once without writing what is in the queue. Not counted as loss (it is a close, not a message).
                Err(mpsc::error::TrySendError::Full(_)) => {
                    let _ = self.inner.ctl_tx.send(Ctl::Close);
                }
                // The supervisor is already gone — `finished` is released at once.
                Err(mpsc::error::TrySendError::Closed(_)) => {}
            }
            self.inner.flush_wake.notify_one();
        }
        finished
    }

    /// Released when the supervisor ends — only the supervisor holds the state sender end, so its dropping is the supervisor's end (normal teardown · panic · runtime
    /// shutdown alike). If already ended, it is released at once.
    fn finished(&self) -> impl Future<Output = ()> + Send + 'static {
        let mut state = self.inner.state_rx.clone();
        async move { while state.changed().await.is_ok() {} }
    }

    pub(crate) fn close_requested(&self) -> bool {
        self.inner.close_requested.load(Ordering::Acquire)
    }

    /// The counter from which this supervisor draws generations — whoever replaces it under the same name tag inherits it and does not rewind generations (`registry.rs`).
    pub(crate) fn generations(&self) -> Arc<AtomicU64> {
        self.inner.generations.clone()
    }

    pub(crate) fn same(&self, other: &Peer<W>) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }

    /// Test handle — all payload bytes this supervisor now holds (the count's fill level) — the read-ahead queue · the ordering area · what is in the line,
    /// and the pieces the resume window holds. Of those, only the window share is [`Peer::window_bytes`].
    #[cfg(test)]
    pub(crate) fn held_bytes(&self) -> usize {
        self.inner.ledger.level()
    }

    /// Test handle — of those, the bytes of the pieces the resume window holds (the share that is subtracted from the read gate).
    #[cfg(test)]
    pub(crate) fn window_bytes(&self) -> usize {
        self.inner.ledger.held()
    }

    /// One fanout frame (`Registry::broadcast`). ★What could not be put in is counted by this peer's supervisor and reported to this handle's receiver end as
    /// [`TransportEvent::Dropped`]★ — the registry does not hold the receiver's enqueue end (if it did, 「`Closed` 가 마지막
    /// → 넣는 끝을 놓음 → `None`」 (Closed is last → release the enqueue end → None) would break — TRD 3 §3-0 ②). Queue saturation is already counted by the enqueue (`push`). For a peer whose supervisor
    /// is gone (`SendError::Closed`), the receiver end to report to has already ended.
    ///
    /// ★What could not be put in because it is not the operating phase after the app called [`Peer::close`] is not counted★ — the count of losses after the app said it would close
    /// has no reader and a clean shutdown comes first (user decision 2026-09-26 — `lib.rs` 「알려진 한계」 (known limits)). What
    /// was in the queue before that is counted by the supervisor.
    ///
    /// Bound to the current generation — if the generation changes before writing, the supervisor does not write and counts it (the same comparison as [`Peer::notify_in`]).
    pub(crate) fn send_frame(&self, frame: Frame) -> Result<(), SendError> {
        // This is after the app said it would close — not counted (that decision in the rustdoc below).
        if self.flushing() {
            return Err(SendError::Closed);
        }
        let generation = self.generation();
        if !self.bound_to(generation) {
            if !self.close_requested() {
                self.refuse();
            }
            return Err(SendError::Disconnected);
        }
        self.push(PeerCmd {
            generation,
            op: PeerOp::Raw(frame),
        })
    }

    /// Counts one thing dropped because the bound generation is not operating (not in the operating phase, or re-attached under a newer generation) — the supervisor
    /// collects it and reports it as [`TransportEvent::Dropped`].
    fn refuse(&self) {
        self.inner.refused.fetch_add(1, Ordering::AcqRel);
        self.inner.loss_wake.notify_one();
    }

    /// Counts one thing bound to `generation` that could not get in because the queue was full — the supervisor collects and reports it, and if that generation is the current one,
    /// handles it per [`QueueFullPolicy`] (`PeerInner::overflow_generation`). ★The generation is written before the count★ — so the generation the
    /// supervisor that collected the count reads afterwards is at least this overflow's generation. ★It wakes★ — a count made when enqueueing after seeing a stale `Live` may arrive after the supervisor
    /// has drained the queue and gone to sleep.
    fn count_overflow(&self, generation: Generation) {
        self.inner
            .overflow_generation
            .fetch_max(generation.0, Ordering::AcqRel);
        self.inner.overflow.fetch_add(1, Ordering::AcqRel);
        self.inner.loss_wake.notify_one();
    }

    /// Test handle — the number the handle dropped because its bound generation is not operating and the supervisor has not collected yet.
    #[cfg(test)]
    pub(crate) fn refused_count(&self) -> u64 {
        self.inner.refused.load(Ordering::Acquire)
    }

    /// A request bound to `generation` ended with a disconnect — the disconnect cause is the last one the handle knows. The meaning of `sent` is
    /// that field of [`RequestError::Disconnected`].
    fn disconnected(&self, generation: Generation, sent: bool) -> RequestError {
        RequestError::Disconnected {
            generation,
            cause: cause_from_code(self.inner.last_cause.load(Ordering::Acquire)),
            sent,
        }
    }

    fn push(&self, cmd: PeerCmd<W>) -> Result<(), SendError> {
        let generation = cmd.generation;
        match self.inner.data_tx.try_send(cmd) {
            Ok(()) => Ok(()),
            Err(mpsc::error::TrySendError::Full(_)) => {
                self.count_overflow(generation);
                Err(SendError::QueueFull)
            }
            Err(mpsc::error::TrySendError::Closed(_)) => Err(SendError::Closed),
        }
    }
}

/// How the supervisor gets its channel — that path decides how the supervisor ends.
pub(crate) enum Origin<W: Wire> {
    /// Dials with the registry's [`Dialer`] (`Registry::add`) — if disconnected it dials again after backoff, and the only end is a close.
    Dial {
        addr: Address,
        dialer: Arc<dyn Dialer>,
    },
    /// One already accepted channel (`Registry::attach`) — starts from the handshake and does not reconnect (`Supervisor::run`).
    Accepted(AcceptedLink<W>),
}

/// What an accepted peer brings to the supervisor.
pub(crate) struct AcceptedLink<W: Wire> {
    pub tx: Box<dyn LinkTx>,
    pub rx: Box<dyn LinkRx>,
    /// The base of the auth deadline — deadline = this + [`Policy::connect_timeout`] (TRD 3 §5 3). A value stamped with the registry's clock.
    pub upgraded_at: Instant,
    /// The upgrade field — released on entering the operating phase, and if it cannot enter, released after the supervisor drops the channel (`Supervisor::admission`).
    pub permit: ConnPermit,
    /// The hand by which the supervisor removes itself from the registry when it ends (`RosterExit`).
    pub leave: LeaveRoster<W>,
}

/// Removes this handle from the registry — only when that name tag still points to this handle (a newer one that replaced it is left alone).
pub(crate) type LeaveRoster<W> = Box<dyn FnOnce(&Peer<W>) + Send>;

/// Everything needed to start a supervisor task.
pub(crate) struct PeerConfig<W: Wire> {
    pub id: PeerId,
    pub origin: Origin<W>,
    pub wire: Arc<W>,
    pub clock: Arc<dyn Clock>,
    pub handshake: Arc<dyn Handshake>,
    pub policy: Policy,
    /// ★The enqueue end of this handle's receiver end — only this one supervisor holds it★ (`registry.rs` header). If shared with other supervisors, behind a line filled by a noisy
    /// peer a quiet peer's items would be held in the ordering area, and even if the supervisor releases this when it ends, the receiver end does not
    /// end (「`Closed` 가 마지막 → 넣는 끝을 놓음 → `None`」 (Closed is last → release the enqueue end → None) — TRD 3 §3-0 ②). Each item carries the byte share of the supervisor that
    /// enqueued it and it is released when the receiving side takes it out (`budget.rs`).
    pub inbound: mpsc::Sender<Charged<Incoming<W>>>,
    /// The byte budget to hold — the registry puts in [`BYTE_BUDGET`]. It is a field so that tests can put in a small value.
    pub byte_budget: usize,
    /// ★This name tag's generation counter★ — even if the same name tag is raised again, **the same Arc is handed down**.
    ///
    /// The approach of reading the old supervisor's last generation and passing it as the floor had a race (D1): if the old supervisor
    /// meets the handshake and a queued `Close` **both ready at once** inside `shake_hands`'s `select`,
    /// the handshake can win and issue one more generation, but the registry has already read the value from just before that.
    /// If the counter is shared, the same number is never drawn twice whoever wins.
    pub generations: Arc<AtomicU64>,
}

/// A supervisor not yet started — the registry starts it after raising that handle (`Registry::launch`).
pub(crate) struct Unstarted<W: Wire>(Supervisor<W>);

impl<W: Wire> Unstarted<W> {
    /// ★Drop the `JoinHandle`★ — the supervisor's end is announced by the state watch (`Peer::finished`). If the runtime is folding, the supervisor
    /// does not run here and is dropped, and for an accepted peer that Drop takes the registry lock (`RosterExit`) — do not call
    /// while holding that lock.
    pub(crate) fn start(self, runtime: &tokio::runtime::Handle) {
        runtime.spawn(self.0.run());
    }
}

/// Builds the supervisor and its handle but does not start it — starting is [`Unstarted::start`].
pub(crate) fn prepare_peer<W: Wire>(cfg: PeerConfig<W>) -> (Peer<W>, Unstarted<W>) {
    let (handle, supervisor) = new_peer(cfg);
    (handle, Unstarted(supervisor))
}

/// Test handle — starts the supervisor task at once and returns the handle (without a registry).
#[cfg(test)]
pub(crate) fn spawn_peer<W: Wire>(cfg: PeerConfig<W>) -> Peer<W> {
    let (handle, supervisor) = new_peer(cfg);
    tokio::spawn(supervisor.run());
    handle
}

fn new_peer<W: Wire>(cfg: PeerConfig<W>) -> (Peer<W>, Supervisor<W>) {
    // ★Everything that can panic comes before building `exit`★ — the registry calls this while holding the lock (`Registry::launch`), and
    //   if it panics after `exit` is built, the unwinding drop of it takes the registry lock again on the same thread (deadlock). The injected
    //   clock · deadline addition · channel-capacity assertions are that place. After `exit`, only moves are left.
    let now = cfg.clock.now();
    let (data_tx, data_rx) = mpsc::channel(cfg.policy.outbound_queue.max(1));
    let (ctl_tx, ctl_rx) = mpsc::unbounded_channel();
    let (state_tx, state_rx) = watch::channel(PeerState::Idle);
    let generation_floor = cfg.generations.load(Ordering::Acquire);
    let generation = Arc::new(AtomicU64::new(generation_floor));
    let overflow = Arc::new(AtomicU64::new(0));
    let overflow_generation = Arc::new(AtomicU64::new(0));
    let refused = Arc::new(AtomicU64::new(0));
    let loss_wake = Arc::new(Notify::new());
    let last_cause = Arc::new(AtomicU8::new(cause_code(DisconnectCause::WriteFailed)));
    let goodbye = Arc::new(OnceLock::new());
    let flush_wake = Arc::new(Notify::new());
    let ledger = Ledger::new(cfg.byte_budget, cfg.clock.clone());
    let (mode, link, admission, leave) = match cfg.origin {
        Origin::Dial { addr, dialer } => (Mode::Dial { addr, dialer }, None, None, None),
        Origin::Accepted(AcceptedLink {
            tx,
            rx,
            upgraded_at,
            permit,
            leave,
        }) => (
            Mode::Accepted {
                auth_deadline: upgraded_at + cfg.policy.connect_timeout,
            },
            Some((tx, rx)),
            Some(permit),
            Some(leave),
        ),
    };
    let accepted = matches!(mode, Mode::Accepted { .. });
    let (tx, rx_raw) = link.unzip();
    let machine = Machine::new(
        cfg.policy.clone(),
        cfg.id.jitter_seed(),
        now,
        generation_floor,
    );
    let pending = Pending::new(cfg.policy.retired_tags);
    let streams = Streams::new(
        cfg.policy.max_streams,
        cfg.policy.resume_buffer,
        cfg.policy.resume_timeout,
    );

    let handle = Peer {
        inner: Arc::new(PeerInner {
            id: cfg.id.clone(),
            data_tx,
            ctl_tx,
            state_rx,
            generation: generation.clone(),
            overflow: overflow.clone(),
            overflow_generation: overflow_generation.clone(),
            refused: refused.clone(),
            loss_wake: loss_wake.clone(),
            last_cause: last_cause.clone(),
            close_requested: AtomicBool::new(false),
            goodbye: goodbye.clone(),
            flush_wake: flush_wake.clone(),
            accepted,
            generations: cfg.generations.clone(),
            #[cfg(test)]
            ledger: ledger.clone(),
        }),
    };
    let exit = RosterExit {
        leave,
        me: Arc::downgrade(&handle.inner),
    };
    let supervisor = Supervisor {
        machine,
        pending,
        streams,
        id: cfg.id,
        mode,
        wire: cfg.wire,
        clock: cfg.clock,
        handshake: cfg.handshake,
        policy: cfg.policy,
        inbound: cfg.inbound,
        data_rx,
        ctl_rx,
        state_tx,
        generations: cfg.generations,
        generation,
        overflow,
        overflow_generation,
        refused,
        loss_wake,
        last_cause,
        goodbye,
        flush_wake,
        flush_deadline: None,
        tx,
        rx_raw,
        frames_rx: None,
        reader: None,
        fresh_link: false,
        consumer_gone: AtomicBool::new(false),
        order: VecDeque::new(),
        order_since: now,
        ledger,
        paused: false,
        paused_since: now,
        last_rx: now,
        last_ping: now,
        dropped_out: 0,
        admission,
        exit,
        end_cause: DisconnectCause::Explicit,
    };
    (handle, supervisor)
}

enum LinkMsg {
    /// One received frame and its byte share — the share follows the items this frame produces all the way to the receiving side.
    Frame(Frame, Charge),
    /// The peer closed.
    ///
    /// ★The close code is not carried here★ — this task runs **only after the handshake ends**, and what a disconnect during
    /// operation carries is only [`DisconnectCause`], so there is no field to put a code in (exactly as in TRD §7's event table).
    /// The place where the code decides is the handshake, and it reads [`LinkRx`] directly and sees the code of [`LinkRead::Closed`]
    /// as is.
    Closed,
    /// ★The text the channel gave is dropped here★ — what [`TransportEvent::Disconnected`] carries is only
    /// [`DisconnectCause`] and there is no field to put text in (exactly as in TRD §7's event table).
    Failed,
}

/// `Room` is the answer of [`drain_order`] (the same field of `Down` · `Connecting` too) — `false` = the receiving side has closed. The arm that watches the receiver end
/// closing (`Sender::closed`) also yields `Room(false)`.
enum Wake<W: Wire> {
    Ctl(Option<Ctl>),
    Link(Option<LinkMsg>),
    Cmd(Option<PeerCmd<W>>),
    Room(bool),
    /// The read gate opened (`budget.rs`).
    Budget,
    Timer,
}

enum Down<W: Wire> {
    Ctl(Option<Ctl>),
    Cmd(Option<PeerCmd<W>>),
    Room(bool),
    /// Counted what the handle could not put in — the report at the start of the next turn collects it.
    Lost,
    Elapsed,
}

enum Connecting<T> {
    Ctl(Option<Ctl>),
    Done(T),
    Room(bool),
    /// The app called [`Peer::close_after_flush`] — there is no connection to drain, so discard this attempt and fold.
    Flush,
}

/// The wake of one write (`Supervisor::guard`).
enum Guarded {
    Done(Result<(), Input>),
    /// The pre-close flush started (or a 「지금 다시 해라」 (retry now) that came during the drain was dropped) — put a deadline and keep
    /// waiting on the same write.
    Flush,
}

/// One received, decoded message — it goes up holding its byte share and the generation of the connection that carried it. ★The generation is stamped at
/// receive time★ — if it is held in the resume window and re-attaches, the window closes on the next connection, and stamping the generation of that time would make
/// [`Arrival::generation`] not point to the connection that carried it.
struct Received<M> {
    msg: M,
    charge: Charge,
    generation: Generation,
}

/// What is left of how the supervisor gets its channel after removing the channel · field (`Origin`).
enum Mode {
    Dial {
        addr: Address,
        dialer: Arc<dyn Dialer>,
    },
    /// `auth_deadline` = upgrade end + [`Policy::connect_timeout`] — the handshake's deadline.
    Accepted { auth_deadline: Instant },
}

/// The hand by which an accepted peer removes itself from the registry when it ends (`Registry::attach`). For a dialed peer it is empty — it stays in the registry waiting for `remove` ·
/// replacement.
///
/// ★It removes on drop too★ — so that no ended handle is left in the registry even if the supervisor panics or the runtime folds before teardown finishes.
struct RosterExit<W: Wire> {
    leave: Option<LeaveRoster<W>>,
    /// ★It is a weak handle★ — if strong, the control channel would not close even when the consumer releases all handles, and the supervisor could not hear it as 「닫아라」 (close).
    me: Weak<PeerInner<W>>,
}

impl<W: Wire> RosterExit<W> {
    fn leave(&mut self) {
        let Some(leave) = self.leave.take() else {
            return;
        };
        // If it cannot be raised, the registry does not hold this handle either — there is nothing to remove.
        if let Some(inner) = self.me.upgrade() {
            leave(&Peer { inner });
        }
    }
}

impl<W: Wire> Drop for RosterExit<W> {
    fn drop(&mut self) {
        self.leave();
    }
}

struct Supervisor<W: Wire> {
    id: PeerId,
    mode: Mode,
    wire: Arc<W>,
    clock: Arc<dyn Clock>,
    handshake: Arc<dyn Handshake>,
    policy: Policy,
    machine: Machine,
    pending: Pending<W::Tag, oneshot::Sender<Result<W::In, RequestError>>>,
    /// A held piece is also held carrying its own byte share (counted as the window share) and the generation of the connection that carried it.
    streams: Streams<W::StreamKey, Received<W::In>>,
    inbound: mpsc::Sender<Charged<Incoming<W>>>,
    data_rx: mpsc::Receiver<PeerCmd<W>>,
    ctl_rx: mpsc::UnboundedReceiver<Ctl>,
    state_tx: watch::Sender<PeerState>,
    /// The shared counter of the generations this name tag issued (D1).
    generations: Arc<AtomicU64>,
    generation: Arc<AtomicU64>,
    overflow: Arc<AtomicU64>,
    /// The newest of the generations stamped on commands that could not be put in (the same field of `PeerInner`).
    overflow_generation: Arc<AtomicU64>,
    refused: Arc<AtomicU64>,
    /// The handle raised `overflow` · `refused` — wakes the given-up · backoff wait so it reports (the same field of `PeerInner`).
    loss_wake: Arc<Notify>,
    last_cause: Arc<AtomicU8>,
    /// The close that [`Peer::close_after_flush`] wrote (the same field of `PeerInner`) — if standing, do not reconnect.
    goodbye: Arc<OnceLock<Close>>,
    flush_wake: Arc<Notify>,
    /// The pre-close flush deadline — set when the supervisor first notices `goodbye` ([`Policy::flush_timeout`]).
    flush_deadline: Option<Instant>,
    tx: Option<Box<dyn LinkTx>>,
    /// The reading side, held only during the handshake. On entering the operating phase it moves to the [`read_link`] task.
    rx_raw: Option<Box<dyn LinkRx>>,
    frames_rx: Option<mpsc::Receiver<LinkMsg>>,
    reader: Option<tokio::task::JoinHandle<()>>,
    /// Just attached — pending resumes must be sent out again.
    fresh_link: bool,
    /// The place to send upward has closed altogether. ★This is not a matter of re-attaching but of folding★.
    consumer_gone: AtomicBool,
    /// The ordering area — holds, in the order produced, what could not yet be loaded into the line (this file's header). It is emptied when the receiving side closes.
    order: VecDeque<Charged<Incoming<W>>>,
    /// The time the ordering area last started holding — meaningful only when the ordering area is not empty (`put` writes it).
    order_since: Instant,
    /// The count of bytes this supervisor holds (including the window share) — the same count even when the connection is replaced. A re-attached channel is not read until the share the old channel left
    /// opens the read gate (one supervisor = one ordering area, so that share too waits on the same receiving side).
    ledger: Arc<Ledger>,
    /// Was it held by backpressure at the start of the turn — kept to recognize the moment it clears (`track_pause`). ★The deadline decision ·
    /// wake-up time · window clock all read this field★ — if even one re-looks at the backpressure at its own place, the progress checks
    /// measured against different states break (`Turn::Timer` of `run_live`).
    paused: bool,
    /// The time this hold started — the time it actually started, not when it was noticed (`track_pause`). While held, the window clock is stopped
    /// here (`resume_clock`).
    paused_since: Instant,
    last_rx: Instant,
    last_ping: Instant,
    dropped_out: u64,
    /// The accepted peer's upgrade field (TRD 3 §5 14) — released on entering the operating phase (`shake_hands`),
    /// and if it ends without entering, released at the place
    /// where the channel was dropped (`run` — before teardown): so that whatever teardown waits for on the receiving side, the field is not tied to that wait. For a dialed peer it is `None`.
    ///
    /// ★The auth-waiting field · operating field do not exist yet★ (3-3 U4) — this field covers their role too. When it arrives, `Registry::attach`
    /// holds the auth-waiting field and releases this field, and the supervisor holds that field here and swaps it for the operating field on entering the operating phase.
    admission: Option<ConnPermit>,
    exit: RosterExit<W>,
    /// The `cause` of [`TransportEvent::Closed`]. For a dialed peer it is always `Explicit` (the disconnect cause is carried by the preceding `Disconnected`).
    /// For an accepted peer it is the real cause — for a disconnect during operation that cause (`run`), for a handshake failure the place of that failure (`shake_hands`),
    /// and if the app · receiving side closed, `Explicit` as is.
    end_cause: DisconnectCause,
}

impl<W: Wire> Supervisor<W> {
    async fn run(mut self) {
        let now = self.clock.now();
        let mut acts = self.machine.on(now, Input::Start);
        if self.accepted() {
            // ★An accepted peer already holds the channel★ (`new_peer`) — the machine's dial is repaid on the spot as 「열렸다」 (opened) and it goes to the handshake.
            //   The machine is left in its dialing-side shape (pure layer), and 「걸기 중」 (dialing) passes before being published.
            debug_assert!(matches!(acts.as_slice(), [Action::Dial { .. }]), "{acts:?}");
            acts = self.machine.on(now, Input::Dialed);
        }
        loop {
            self.publish();
            let follow = self.apply(acts).await;
            self.publish();
            if self.machine.state() == PeerState::Closed {
                break;
            }
            let input = match follow {
                Some(input) => input,
                None if self.receiver_gone() => Input::Close,
                None => match self.machine.state() {
                    PeerState::Live => {
                        let input = self.run_live().await;
                        // Hands over what was already read before the machine wakes the waiters and emits the disconnect event.
                        let read_end = self.salvage_read_ahead().await;
                        earlier_end(input, read_end)
                    }
                    PeerState::GaveUp(_) | PeerState::Idle => self.park().await,
                    other => {
                        // For Dialing/Backoff/Handshaking, `apply` always emits a follow-up input.
                        // ★We do not kill the peer here★ — if we got here, that is our mistake, and paying for it by
                        //   「이 연결을 영구히 끝냄」 (ending this connection permanently) is the most destructive default.
                        debug_assert!(false, "후속 입력 없이 {other:?} 로 떨어졌다");
                        Input::ReconnectNow
                    }
                },
            };
            let now = self.clock.now();
            if let (true, Input::LinkLost(cause)) = (self.accepted(), &input) {
                self.end_cause = *cause;
            }
            acts = self.machine.on(now, input);
            // ★This comes before the publish★ — if the `publish` at the start of the turn ran before folding, `Backoff` · `GaveUp` would be seen once.
            // ★An accepted peer always folds★ — it has no address to redial. Every end where the machine goes to reconnect · give up (handshake failure ·
            //   disconnect during operation) becomes a close here.
            if (self.flushing() || self.accepted())
                && matches!(
                    self.machine.state(),
                    PeerState::Backoff | PeerState::GaveUp(_)
                )
            {
                acts = self.fold_into_close(now, acts);
            }
        }
        // The channel is already dropped (the machine's last batch emits `DropLink`) — the field is released before the teardown that waits on the receiving side, and the registry removal
        //   also comes before that.
        self.admission = None;
        self.exit.leave();
        self.teardown().await;
        // Here the supervisor is dropped and releases the enqueue end — the next `recv` on that handle's receiver end is `None`.
    }

    fn accepted(&self) -> bool {
        matches!(self.mode, Mode::Accepted { .. })
    }

    /// The connection was lost during the pre-close flush (a disconnect during operation · the failure of a dial attempt — the machine went to `Backoff` · `GaveUp`) —
    /// does not go to reconnect · give up but folds straight into the close ([`Peer::close_after_flush`]). The machine is left as is (pure layer);
    /// from its actions only the reconnect · give-up part (the wait · [`MachineEvent::GaveUp`] · [`MachineEvent::ConnectFailed`] reports) is
    /// withdrawn and then the close is fed on. Waking the waiters · the disconnect report · dropping the channel keep that disconnect cause — the close code too
    /// (`drop_link`).
    ///
    /// ★Filtering out `GaveUp` is the point★ — the shell sees it and shows the reconnect popup (ADR-0180). That popup must not appear for a peer the app said it would
    /// close, and announcing 「n 초 뒤 다시」 (retry in n seconds) for a peer that will not redial is false too. The machine is already
    /// `Closed`, so the intermediate state (`Backoff` · `GaveUp`) is not published (`run` calls this before the publish). A drain that started during the channel drop of
    /// a batch that decided to give up is not caught here — `apply` collects that report.
    ///
    /// ★An accepted peer always comes here when it loses its connection★ (`run`) — it has no address to dial. For that peer the terminal reject
    /// ([`MachineEvent::Rejected`]) is also filtered: that event promises a `GaveUp` right after it (`TransportEvent::Rejected`),
    /// and an accepted peer's handshake failure ends with the single cause of `Closed` (`Supervisor::end_cause`).
    fn fold_into_close(&mut self, now: Instant, acts: Vec<Action>) -> Vec<Action> {
        let accepted = self.accepted();
        let mut folded: Vec<Action> =
            acts.into_iter()
                .filter(|act| match act {
                    Action::Wait { .. }
                    | Action::Emit(
                        MachineEvent::GaveUp { .. } | MachineEvent::ConnectFailed { .. },
                    ) => false,
                    Action::Emit(MachineEvent::Rejected { .. }) => !accepted,
                    _ => true,
                })
                .collect();
        folded.extend(self.machine.on(now, Input::Close));
        folded
    }

    fn publish(&mut self) {
        self.generation
            .store(self.machine.generation().0, Ordering::Release);
        self.state_tx.send_replace(self.machine.state());
    }

    async fn apply(&mut self, acts: Vec<Action>) -> Option<Input> {
        let mut follow = None;
        let mut failed_pending = false;
        for act in acts {
            match act {
                // ★Look again when emitting★ — a batch that decided to give up on a disconnect (`[.., DropLink, .., Emit(GaveUp)]`) comes out before the drain
                //   and `fold_into_close` cannot collect it. If a drain starts during that `DropLink`'s blocked close, it is collected here —
                //   `park` · `wait` at the start of the turn see that drain and fold into the close.
                Action::Emit(MachineEvent::GaveUp { .. } | MachineEvent::ConnectFailed { .. })
                    if self.flushing() => {}
                Action::Emit(event) => self.emit_machine(event),
                Action::FailPending(cause) => {
                    failed_pending = true;
                    self.fail_pending(cause);
                }
                Action::DropLink { code } => {
                    // `drop_link` splits the close code by the cause (`last_cause`) that the same batch's `FailPending` wrote.
                    debug_assert!(
                        self.tx.is_none() || failed_pending,
                        "통로를 쥔 채 FailPending 없이 DropLink 가 왔다"
                    );
                    self.drop_link(code).await;
                }
                Action::Dial { deadline } => follow = Some(self.dial(deadline).await),
                Action::Handshake { deadline } => follow = Some(self.shake_hands(deadline).await),
                Action::Wait { until } => follow = Some(self.wait(until).await),
            }
        }
        follow
    }

    /// Collects the piled-up control signals at once. `Close` beats `ReconnectNow`.
    ///
    /// ★Folding consecutive `ReconnectNow` into one is the point★ — without folding, when the shell
    /// presses faster than the supervisor, each time the dial is canceled **before it is even polled**, and 「끊고 → 걸다 말고」 (cut → stop mid-dial) repeats forever.
    ///
    /// ★During a pre-close flush, `ReconnectNow` is dropped★ — also one called before the drain and not yet read. The handle
    /// does not accept calls after the drain (`Peer::reconnect_now`), but what entered before it stays in this queue, and listening would drop the channel and
    /// go to redial, so the drain would end without the close it was given. The other places that read control (`guard` · `run_live` · `connecting` ·
    /// `wait` · `park`) follow the same rule.
    fn drain_ctl(&mut self) -> Option<Ctl> {
        let mut seen = None;
        while let Ok(signal) = self.ctl_rx.try_recv() {
            match signal {
                Ctl::Close => return Some(Ctl::Close),
                Ctl::ReconnectNow if self.flushing() => {}
                Ctl::ReconnectNow => seen = Some(Ctl::ReconnectNow),
            }
        }
        seen
    }

    // ── Connection establishment ─────────────────────────────────────────────

    /// ★Listens to control signals throughout both phases (dial · handshake)★ — otherwise `close()` is heard late by
    /// `connect_timeout`. If control wins, the channel is closed along with the canceled future.
    async fn dial(&mut self, deadline: Instant) -> Input {
        let Mode::Dial { addr, dialer } = &self.mode else {
            // The paths that make an accepted peer dial (reconnect · end of backoff) are all blocked by the supervisor — the fold in `run` ·
            //   `Peer::reconnect_now`. If we got here, it is our mistake, and since there is no address to dial, we fold.
            debug_assert!(false, "받은 상대에 걸기가 왔다");
            return Input::Close;
        };
        let (addr, dialer) = (addr.clone(), dialer.clone());
        // A 「지금 다시 붙어라」 (reconnect now) that was already piled up is exactly what we are about to do, so we swallow it.
        if let Some(Ctl::Close) = self.drain_ctl() {
            return Input::Close;
        }
        let dialing = dial_once(dialer, addr, self.clock.clone(), deadline);
        match self.connecting(dialing).await {
            Connecting::Done(Ok((tx, rx))) => {
                self.tx = Some(tx);
                self.rx_raw = Some(rx);
                Input::Dialed
            }
            Connecting::Done(Err(failure)) => Input::ConnectFailed(failure),
            Connecting::Ctl(Some(Ctl::ReconnectNow)) => Input::ReconnectNow,
            Connecting::Ctl(Some(Ctl::Close))
            | Connecting::Ctl(None)
            | Connecting::Room(_)
            | Connecting::Flush => Input::Close,
        }
    }

    async fn shake_hands(&mut self, deadline: Instant) -> Input {
        // ★Pairs with the same line in `dial`★ — it folds the piled-up 「지금 다시 붙어라」 (reconnect now) here too.
        if let Some(Ctl::Close) = self.drain_ctl() {
            return Input::Close;
        }
        let (Some(tx), Some(rx)) = (self.tx.take(), self.rx_raw.take()) else {
            return Input::ConnectFailed(ConnectFailure::Unreachable(LinkError::new(
                "no link to hand shake on",
            )));
        };
        let deadline = match self.mode {
            // ★An accepted peer's deadline runs from the end of the upgrade★ (TRD 3 §5 3) — time sitting in the ready queue counts too. It must always be earlier than or equal to the machine's deadline
            //   (from when the supervisor started), and even if the listener stamped it with a different clock, we use the earlier one so as not to exceed it.
            Mode::Accepted { auth_deadline } => auth_deadline.min(deadline),
            Mode::Dial { .. } => deadline,
        };
        // ★An accepted peer does not run the handshake if the deadline has already passed★ — `with_deadline` polls once even on a past deadline
        //   (its rustdoc), so frames that had already arrived while it sat in the ready queue could get it through. The channel is dropped without a close, like a
        //   handshake that ended on the deadline.
        if self.accepted() && deadline <= self.clock.now() {
            self.end_cause = DisconnectCause::Idle;
            return Input::ConnectFailed(ConnectFailure::Unreachable(LinkError::new(
                "handshake timed out",
            )));
        }
        let shaking = handshake_once(tx, rx, self.handshake.clone(), self.clock.clone(), deadline);
        match self.connecting(shaking).await {
            Connecting::Done(Ok((tx, rx))) => {
                self.tx = Some(tx);
                self.attach_reader(rx);
                // The accepted peer's upgrade field is released now that it has entered the operating phase (`Supervisor::admission`).
                self.admission = None;
                // The number is drawn here — this place is the only point of 「이 통로가 운영 단계에 들어섰다」 (this channel entered the operating phase).
                let minted = Generation(self.generations.fetch_add(1, Ordering::AcqRel) + 1);
                Input::Connected(minted)
            }
            Connecting::Done(Err(failed)) => {
                if self.accepted() {
                    self.end_cause = failed.end;
                }
                Input::ConnectFailed(failed.failure)
            }
            Connecting::Ctl(Some(Ctl::ReconnectNow)) => Input::ReconnectNow,
            Connecting::Ctl(Some(Ctl::Close))
            | Connecting::Ctl(None)
            | Connecting::Room(_)
            | Connecting::Flush => Input::Close,
        }
    }

    /// Runs one phase of connection establishment to the end while listening to control, loading the ordering area, and watching for the receiver end closing. The returned
    /// `Room` is always `false` (the receiving side closed) — a turn that loaded the ordering area just goes on with the phase.
    ///
    /// ★The phase future is pinned outside the turn★ — if it were dropped every time the ordering-area branch wins, the channel being dialed and the
    /// handshake would start over from the beginning.
    async fn connecting<T>(&mut self, step: impl Future<Output = T>) -> Connecting<T> {
        // ★Look before polling the phase★ — if not, even though the receiving side is already gone, the future that dials the channel is polled once
        //   (the `select!` below does not know which of the ready arms it looks at first). A receiver end released before the first event is even put in
        //   means the channel is not dialed at all.
        if self.receiver_gone() {
            return Connecting::Room(false);
        }
        // During a pre-close flush it does not reconnect — that marker was to be loaded onto the dropped connection ([`Peer::close_after_flush`]).
        if self.flushing() {
            return Connecting::Flush;
        }
        tokio::pin!(step);
        loop {
            let backlog = !self.order.is_empty();
            let woken = {
                let ctl = &mut self.ctl_rx;
                let (lane, order) = (&self.inbound, &mut self.order);
                let flush = &self.flush_wake;
                // ★Wakes when the receiver end closes (`closed`)★ — folds at once even with nothing to put in (crate header 「받는 쪽 불변식」
                //   (receiver invariants) 5). It is cancel-safe — calling it again checks whether it is closed first (external fact — tokio `Sender::closed`).
                // ★Also listens for the drain start★ — this phase does not read the data queue, so the marker does not wake it.
                tokio::select! {
                    signal = ctl.recv() => Connecting::Ctl(signal),
                    out = &mut step => Connecting::Done(out),
                    open = drain_order(lane, order), if backlog => Connecting::Room(open),
                    () = lane.closed() => Connecting::Room(false),
                    () = flush.notified() => Connecting::Flush,
                }
            };
            match woken {
                Connecting::Room(true) => {}
                Connecting::Room(false) => {
                    self.consumer_gone.store(true, Ordering::Release);
                    return Connecting::Room(false);
                }
                // A 「지금 다시 해라」 (retry now) read during a drain is dropped (`Self::drain_ctl`) — there is no connection to drain, so it folds.
                Connecting::Ctl(Some(Ctl::ReconnectNow)) if self.flushing() => {
                    return Connecting::Flush;
                }
                other => return other,
            }
        }
    }

    /// Reading is split off into its own task — the reason is in the [`read_link`] header.
    fn attach_reader(&mut self, rx: Box<dyn LinkRx>) {
        let (frames_tx, frames_rx) = mpsc::channel(self.policy.frame_buffer.max(1));
        self.reader = Some(tokio::spawn(read_link(rx, frames_tx, self.ledger.clone())));
        self.frames_rx = Some(frames_rx);
        self.fresh_link = true;
        let now = self.clock.now();
        self.last_rx = now;
        self.last_ping = now;
    }

    /// Backoff wait. ★Even while waiting, requests that enter the queue are failed at once★ — otherwise that slot hangs
    /// until the next operating phase (or forever in the given-up state).
    async fn wait(&mut self, until: Instant) -> Input {
        loop {
            // ★It reports here too★ — otherwise losses piled up during backoff · give-up come out nowhere until the user presses
            //   「다시 연결」 (reconnect).
            let _ = self.drain_overflow();
            self.report_drops();
            // If the receiving side is gone or the app said to drain and close, it does not re-attach and folds — this wait is outside operation so there is no
            //   connection to drain ([`Peer::close_after_flush`]).
            if self.receiver_gone() || self.flushing() {
                return Input::Close;
            }
            if let Some(signal) = self.drain_ctl() {
                return match signal {
                    Ctl::ReconnectNow => Input::ReconnectNow,
                    Ctl::Close => Input::Close,
                };
            }
            let delay = until.saturating_duration_since(self.clock.now());
            if delay.is_zero() {
                return Input::BackoffElapsed;
            }
            let sleep = self.clock.sleep(delay);
            let backlog = !self.order.is_empty();
            let woken = {
                let ctl = &mut self.ctl_rx;
                let data = &mut self.data_rx;
                let (lane, order) = (&self.inbound, &mut self.order);
                let lost = &self.loss_wake;
                // Wakes when the receiver end closes — the same arm as in `connecting`.
                tokio::select! {
                    signal = ctl.recv() => Down::Ctl(signal),
                    cmd = data.recv() => Down::Cmd(cmd),
                    open = drain_order(lane, order), if backlog => Down::Room(open),
                    () = lane.closed() => Down::Room(false),
                    () = lost.notified() => Down::Lost,
                    _ = sleep => Down::Elapsed,
                }
            };
            match woken {
                // If draining, it is dropped (`Self::drain_ctl`) — the start of the next turn folds.
                Down::Ctl(Some(Ctl::ReconnectNow)) if self.flushing() => {}
                Down::Ctl(Some(Ctl::ReconnectNow)) => return Input::ReconnectNow,
                Down::Ctl(Some(Ctl::Close)) | Down::Ctl(None) => return Input::Close,
                Down::Cmd(None) => return Input::Close,
                Down::Cmd(Some(cmd)) => self.reject_command(cmd),
                Down::Room(open) => self.note_room(open),
                Down::Lost => {}
                Down::Elapsed => return Input::BackoffElapsed,
            }
        }
    }

    /// Waits only for control while given up (or not yet started). Pending requests are failed at once here too.
    async fn park(&mut self) -> Input {
        loop {
            let _ = self.drain_overflow();
            self.report_drops();
            // The same line as in `wait`.
            if self.receiver_gone() || self.flushing() {
                return Input::Close;
            }
            if let Some(signal) = self.drain_ctl() {
                return match signal {
                    Ctl::ReconnectNow => Input::ReconnectNow,
                    Ctl::Close => Input::Close,
                };
            }
            let backlog = !self.order.is_empty();
            let woken = {
                let ctl = &mut self.ctl_rx;
                let data = &mut self.data_rx;
                let (lane, order) = (&self.inbound, &mut self.order);
                let lost = &self.loss_wake;
                // Wakes when the receiver end closes — the same arm as in `connecting`.
                tokio::select! {
                    signal = ctl.recv() => Down::Ctl(signal),
                    cmd = data.recv() => Down::Cmd(cmd),
                    open = drain_order(lane, order), if backlog => Down::Room(open),
                    () = lane.closed() => Down::Room(false),
                    () = lost.notified() => Down::Lost,
                }
            };
            match woken {
                // If draining, it is dropped (`Self::drain_ctl`) — the start of the next turn folds.
                Down::Ctl(Some(Ctl::ReconnectNow)) if self.flushing() => {}
                Down::Ctl(Some(Ctl::ReconnectNow)) => return Input::ReconnectNow,
                Down::Ctl(Some(Ctl::Close)) | Down::Ctl(None) => return Input::Close,
                Down::Cmd(None) => return Input::Close,
                Down::Cmd(Some(cmd)) => self.reject_command(cmd),
                Down::Room(open) => self.note_room(open),
                Down::Lost => {}
                Down::Elapsed => return Input::Close,
            }
        }
    }

    fn note_room(&mut self, open: bool) {
        if !open {
            self.consumer_gone.store(true, Ordering::Release);
        }
    }

    /// Is the receiving side gone — it has already seen it close while putting in, or the receiver end is closed now (released or
    /// [`crate::Inbox::close`]). If it sees it closed, it writes that down — after that it does not put in (`put`).
    fn receiver_gone(&self) -> bool {
        if self.inbound.is_closed() {
            self.consumer_gone.store(true, Ordering::Release);
        }
        self.consumer_gone.load(Ordering::Acquire)
    }

    /// Did the app call [`Peer::close_after_flush`] — if standing, do not reconnect.
    fn flushing(&self) -> bool {
        self.goodbye.get().is_some()
    }

    /// If it notices the pre-close flush for the first time, it sets that deadline. Even if called several times, it stays the time first set.
    fn note_flush(&mut self) {
        if self.flush_deadline.is_none() && self.flushing() {
            self.flush_deadline = Some(self.clock.now() + self.policy.flush_timeout);
        }
    }

    // ── Operating phase ──────────────────────────────────────────────────────

    async fn run_live(&mut self) -> Input {
        if std::mem::take(&mut self.fresh_link) {
            if let Some(input) = self.reissue_resumes().await {
                return input;
            }
        }
        let mut frames_in_row = 0usize;
        loop {
            // ★Look at the hold before the loss report★ — the report puts into the ordering area and loads as much as fits in the line (`put`).
            //   If the receiving side emptied the line while waiting on a write, that report empties the whole ordering area, and a hold that started during the write
            //   is never seen — the window past its deadline closes before the read-ahead queue's replay, and the replay is dropped as a number already passed.
            let now = self.clock.now();
            self.track_pause(now);
            self.note_flush();
            let overflow = self.drain_overflow();
            self.report_drops();
            // The same line as in `wait`. It comes before the overflow decision — a peer to fold is not sent to backoff.
            if self.receiver_gone() {
                return Input::Close;
            }
            if let Some(input) = overflow {
                return input;
            }
            // When the handshake opens the operating phase, both halves stand together — if one is missing, it is written as the end of the missing half.
            if self.tx.is_none() {
                return Input::LinkLost(DisconnectCause::WriteFailed);
            }
            if self.frames_rx.is_none() {
                return Input::LinkLost(DisconnectCause::ReadFailed);
            }
            // ① Control first on every turn — heard at once even if frames never stop.
            if let Some(signal) = self.drain_ctl() {
                return match signal {
                    Ctl::ReconnectNow => Input::ReconnectNow,
                    Ctl::Close => Input::Close,
                };
            }

            let wake = self.wake_at();
            // ②③ Anti-starvation rule. `next_turn` decides and here we only carry it out.
            match next_turn(
                now,
                wake,
                frames_in_row,
                self.data_rx.len(),
                self.policy.outbound_queue.max(1),
            ) {
                Turn::Timer => {
                    frames_in_row = 0;
                    if let Some(input) = self.on_timer().await {
                        return input;
                    }
                    // ★Check progress★ — if some timer cannot push its own deadline, this loop spins forever without passing through
                    //   `await` even once. On a current_thread runtime, at that moment **no other task can
                    //   do anything and the process stalls** (measured: a mutant that disabled `expire_resumes` hung
                    //   for over 10 minutes instead of failing the test).
                    if self.wake_at() <= now {
                        debug_assert!(
                            false,
                            "타이머가 자기 시한을 못 밀었다 — 운영 루프가 await 없이 돈다"
                        );
                        tokio::task::yield_now().await;
                    }
                    continue;
                }
                Turn::Outbound(budget) => {
                    frames_in_row = 0;
                    for _ in 0..budget {
                        let Ok(cmd) = self.data_rx.try_recv() else {
                            break;
                        };
                        if let Some(input) = self.on_cmd(cmd).await {
                            return input;
                        }
                    }
                    continue;
                }
                Turn::Wait => {}
            }

            let sleep = self.clock.sleep(wake.saturating_duration_since(now));
            let backlog = !self.order.is_empty();
            // Wakes when the read gate opens. ★If, at the start of the turn, it was seen as held by the read gate (`paused` · ordering area empty),
            //   this arm stands even if it has already cleared now★ — it wakes at once and the next turn writes the clearing. If it does not stand, the wake-up
            //   time follows `paused` with silence · window deadlines excluded, so if the peer is quiet the clearing is not written until the ping.
            //   If the report after the start (`report_drops`) emptied the ordering area, this arm stands even with the read gate open, but it
            //   wakes at once and the next turn writes the clearing — one extra wake-up, not a spin.
            let over = self.ledger.over() || (self.paused && !backlog);
            let woken = {
                let ctl = &mut self.ctl_rx;
                let data = &mut self.data_rx;
                let frames = self.frames_rx.as_mut().expect("checked above");
                let (lane, order) = (&self.inbound, &mut self.order);
                let ledger = &self.ledger;
                // ★Do not re-attach `biased`★ — declaration-order polling under a frame flood starves the later arms
                //   entirely (「운영 루프의 우선순위」 (priority of the operating loop) in this file's header).
                // ★If the ordering area holds anything, pull the frame arm (backpressure)★ — what that frame would produce could not be loaded into the line either, so
                //   it is better left in the channel. The threshold is set at one item because the ordering area exists for the places that put in synchronously (events · deadline events),
                //   not as a second buffer for frames. In exchange, the drain arm always stands alongside.
                // ★Wakes when the receiver end closes★ — folds at once even with nothing to put in before a quiet peer (the same arm as in `connecting`).
                tokio::select! {
                    signal = ctl.recv() => Wake::Ctl(signal),
                    msg = frames.recv(), if !backlog => Wake::Link(msg),
                    cmd = data.recv() => Wake::Cmd(cmd),
                    open = drain_order(lane, order), if backlog => Wake::Room(open),
                    () = lane.closed() => Wake::Room(false),
                    () = ledger.holder_room(), if over => Wake::Budget,
                    _ = sleep => Wake::Timer,
                }
            };

            let outcome = match woken {
                // If draining, it is dropped (`Self::drain_ctl`).
                Wake::Ctl(Some(Ctl::ReconnectNow)) if self.flushing() => None,
                Wake::Ctl(Some(Ctl::ReconnectNow)) => Some(Input::ReconnectNow),
                Wake::Ctl(Some(Ctl::Close)) | Wake::Ctl(None) => Some(Input::Close),
                Wake::Link(None) | Wake::Link(Some(LinkMsg::Failed)) => {
                    Some(Input::LinkLost(DisconnectCause::ReadFailed))
                }
                Wake::Link(Some(LinkMsg::Closed)) => {
                    Some(Input::LinkLost(DisconnectCause::PeerClosed))
                }
                Wake::Link(Some(LinkMsg::Frame(frame, charge))) => {
                    frames_in_row += 1;
                    match self.on_frame(frame, charge) {
                        Ok(Some(resume)) => self.write(resume).await.err(),
                        Ok(None) => None,
                        Err(input) => Some(input),
                    }
                }
                Wake::Cmd(None) => Some(Input::Close),
                Wake::Cmd(Some(cmd)) => {
                    frames_in_row = 0;
                    self.on_cmd(cmd).await
                }
                Wake::Room(open) => {
                    self.note_room(open);
                    None
                }
                // The next turn's `track_pause` sees the clearing.
                Wake::Budget => None,
                Wake::Timer => {
                    frames_in_row = 0;
                    self.on_timer().await
                }
            };
            if let Some(input) = outcome {
                return input;
            }
        }
    }

    /// Sends out again the resumes that were left disconnected.
    ///
    /// ★Without this, the hole stays forever★ — the re-request that was going out at the moment of disconnect may not have reached the peer, and
    /// only the state waiting for its answer remains, so all the following pieces are held back and then folded into a truncation.
    async fn reissue_resumes(&mut self) -> Option<Input> {
        let outstanding = self.streams.pending_resumes();
        if outstanding.is_empty() {
            return None;
        }
        for (key, after) in outstanding {
            let out = self.wire.resume_request(&key, Some(after));
            let frame = self.wire.encode(&out);
            if let Err(input) = self.write(frame).await {
                return Some(input);
            }
        }
        // Wind it with the window clock — if it was held and disconnected on the previous channel, that clock is still stopped, and it is pushed by the held time
        //   when it clears (`track_pause`). If wound with the current time, that push would push once more through the time it was disconnected.
        let now = self.clock.now();
        self.streams.rearm_resumes(self.resume_clock(now));
        None
    }

    /// The next time to wake. ★Every timer source gathers in this one place★ — if you add a new source and do not fix this function,
    /// that deadline wakes nobody, and the progress check of `Turn::Timer` cannot see it either.
    ///
    /// ★Deadlines that stop while held (silence · resume window) are masked by `paused`★ — `on_timer` decides by that field, so
    /// if it re-looks at the backpressure at its own place the two diverge and the progress check breaks (if a clearing slips in after the start, it sets the wake
    /// time with a stale `last_rx`, and `on_timer` sees it as still held and does not cut).
    fn wake_at(&self) -> Instant {
        let mut wake = self.last_ping + self.policy.ping_interval;
        if !self.paused {
            wake = wake.min(self.last_rx + self.policy.idle_timeout);
        }
        if let Some(deadline) = self.pending.next_deadline() {
            wake = wake.min(deadline);
        }
        // ★Without it, nobody can close the re-request window★ — if the truncation decision hangs only on piece arrival,
        //   in the ordinary case where the stream goes quiet after a hole (the turn ended) that window stays open forever.
        if !self.paused {
            if let Some(deadline) = self.streams.next_resume_deadline() {
                wake = wake.min(deadline);
            }
        }
        // ★Unrelated to holding★ — the drain is the outbound side's work, so its deadline runs even when the receiving line is backed up.
        if let Some(deadline) = self.flush_deadline {
            wake = wake.min(deadline);
        }
        wake
    }

    /// At the moment backpressure clears, it sets `last_rx` to that time and pushes the requested time of an open resume window by the held time (TRD 3 §3-0
    /// ① invariant 2). While held, we can read neither the peer's liveness nor the answer to a re-request, so if that time were counted as silence · window deadline,
    /// we would cut the peer after having blocked it ourselves, or fold a hole that could have been filled into a truncation — replay that arrives late is
    /// dropped as a number already passed. Hold = the ordering area is holding (the frame arm was pulled) or the read gate closed (the read task does not
    /// read the channel).
    ///
    /// ★The held time is counted from when the hold started, not from when it was noticed★ — the supervisor cannot get to the start of the turn
    /// while waiting on a write. If a hold started in that time and we counted from when it was noticed, the time spent waiting on the write would be counted as window deadline and
    /// the window would close as soon as it clears. ★If the read gate closed and reopened in that time, the supervisor never sees that hold★ — the count
    /// gathers that span and hands it over ([`Ledger::take_shut`]) and here we push by that much. If we do not push, the window past its deadline closes before the read-ahead queue's
    /// replay and lower numbers are dropped as numbers already passed.
    ///
    /// ★One time is never pushed twice★ — a span that closed and opened while we knew it was held (`paused`) all falls after that hold started,
    /// so it is inside the time pushed at clearing, and if a span we did not see straddles a hold we just noticed (the ordering area held first),
    /// we do not push it separately and treat this hold as having started when that first span closed.
    ///
    /// It diverges only toward the window closing later than the held time (nothing is lost) — the clearing time is taken as the noticing time, the unheld gap between
    /// straddling spans is also counted as held, and a span before the window opened also pushes that window ([`Self::resume_clock`]).
    /// The silence decision takes the time it noticed the clearing as the last receive, so it is independent of the start time.
    ///
    /// It is also called at the start of `on_timer` — if it cleared while asleep (the clearing arm and the deadline arm were ready together and the deadline arm won),
    /// it makes that decision use the current state instead of the stale 「붙듦」 (held) seen at the start of the turn.
    /// ★The silence decision is made
    /// with this field (`paused`) — re-looking at the hold at that place would measure with the old `last_rx` at the moment of clearing and cut a peer held for over 50 s straight to `Idle`★.
    fn track_pause(&mut self, now: Instant) {
        let shut = self.ledger.take_shut();
        let order = (!self.order.is_empty()).then_some(self.order_since);
        let began = match (order, shut.since) {
            (Some(order), Some(gate)) => Some(order.min(gate)),
            (order, gate) => order.or(gate),
        };
        let closed = shut.closed;
        match (self.paused, began) {
            // A span that closed and opened in that time is within the time we knew it was held — pushed together at clearing.
            (true, None) => {
                self.last_rx = now;
                self.streams
                    .shift_resumes(now.saturating_duration_since(self.paused_since));
            }
            (true, Some(_)) => {}
            (false, Some(began)) if closed.until.is_some_and(|until| until > began) => {
                self.paused_since = closed.from.map_or(began, |from| from.min(began)).min(now);
            }
            (false, began) => {
                // A hold we did not see clearing is also a clearing — we take the noticing time now as the last receive.
                if closed.until.is_some() {
                    self.last_rx = now;
                    self.streams.shift_resumes(closed.total);
                }
                if let Some(began) = began {
                    self.paused_since = began.min(now);
                }
            }
        }
        self.paused = began.is_some();
    }

    /// The clock the resume window reads — while held, it stands still at the time the hold started. A window opened in that time is also written as opened
    /// at that time, and at clearing it counts from the cleared time by the amount pushed (`track_pause`).
    /// ★The pushed time may delay the window too much (the safe side)★ — the time of a read gate that had already closed and opened or been closed before the window opened
    /// also pushes every open window.
    fn resume_clock(&self, now: Instant) -> Instant {
        if self.paused {
            self.paused_since
        } else {
            now
        }
    }

    async fn on_timer(&mut self) -> Option<Input> {
        let now = self.clock.now();
        // The pre-close flush deadline has passed — close without writing the rest ([`Peer::close_after_flush`]).
        if self.flush_deadline.is_some_and(|deadline| deadline <= now) {
            return Some(Input::Close);
        }
        self.track_pause(now);
        if !self.paused && now.saturating_duration_since(self.last_rx) >= self.policy.idle_timeout {
            return Some(Input::LinkLost(DisconnectCause::Idle));
        }
        let after = self.policy.request_timeout;
        for (tag, slot) in self.pending.expire(now) {
            let _ = slot.send(Err(RequestError::TimedOut { after }));
            self.emit(TransportEvent::RequestTimedOut {
                peer: self.id.clone(),
                tag,
                after,
            });
        }
        // While held, it does not close the window — that clock is stopped (`resume_clock`).
        if !self.paused {
            let expired = self.streams.expire_resumes(now);
            self.hand_over_truncations(expired);
        }
        if now.saturating_duration_since(self.last_ping) >= self.policy.ping_interval {
            self.last_ping = now;
            if let Err(input) = self.ping().await {
                return Some(input);
            }
        }
        None
    }

    /// Handles one command taken out during operation.
    ///
    /// ★Just-before-write generation comparison (TRD 3 §3-0 ③)★ — if the stamped generation is not the current generation, do nothing and end it like a command that arrived
    /// while not in the operating phase (`reject_command` — a request is woken as not sent and the rest are counted). Between the comparison and the write there is no gap for the generation to
    /// change — only this one supervisor changes the generation and both are inside this function. The comparison comes before any other side effect (pending-table
    /// registration · encoding) — putting a stale request in the table would clash with the same number on the current connection. ★The pre-close flush's marker is
    /// not compared★ — it has no bound generation (`PeerOp::Close`). If rejected by the comparison, it would not close.
    async fn on_cmd(&mut self, cmd: PeerCmd<W>) -> Option<Input> {
        let bound = !matches!(cmd.op, PeerOp::Close { .. });
        if bound && cmd.generation != self.machine.generation() {
            self.reject_command(cmd);
            return None;
        }
        match cmd.op {
            PeerOp::Raw(frame) => self.write_or_count(frame).await,
            PeerOp::Notify { out } => {
                let frame = self.wire.encode(&out);
                self.write_or_count(frame).await
            }
            PeerOp::Resume { key, after } => {
                let out = self.wire.resume_request(&key, after);
                let frame = self.wire.encode(&out);
                self.write(frame).await.err()
            }
            PeerOp::Close { .. } => Some(Input::Close),
            PeerOp::DropLink { cause } => Some(Input::LinkLost(cause)),
            PeerOp::Request { out, reply } => {
                let Some(tag) = self.wire.request_tag(&out) else {
                    let _ = reply.send(Err(RequestError::NotARequest));
                    return None;
                };
                let deadline = self.clock.now() + self.policy.request_timeout;
                match self.pending.insert(tag, reply, deadline) {
                    Insert::Retired(reply) => {
                        let _ = reply.send(Err(RequestError::TagRetired));
                        return None;
                    }
                    Insert::Collided {
                        displaced,
                        rejected,
                    } => {
                        let _ = displaced.send(Err(RequestError::Superseded));
                        let _ = rejected.send(Err(RequestError::TagRetired));
                        return None;
                    }
                    Insert::Placed => {}
                }
                let frame = self.wire.encode(&out);
                self.write(frame).await.err()
            }
        }
    }

    /// Handles one received frame. ★It does not wait★ — the place that drains the read-ahead queue before dropping the channel
    /// ([`Self::salvage_read_ahead`]) calls this too. If a re-request that must be written arises, it returns that frame and the write is
    /// the caller's job. `Err` = cut this channel (decode-failure policy).
    fn on_frame(&mut self, frame: Frame, charge: Charge) -> Result<Option<Frame>, Input> {
        let now = self.clock.now();
        if matches!(frame, Frame::Keepalive) {
            self.last_rx = now;
            return Ok(None);
        }
        // ★Garbage is not proof of liveness★ — if we rewind before decoding, a peer sending only unreadable things
        //   never gets caught by the silence decision, and since the default policy only drops the frame, there is no valve to cut it anywhere.
        let msg = match self.wire.decode(frame) {
            Ok(msg) => {
                self.last_rx = now;
                msg
            }
            Err(err) => {
                let reason = err.class();
                let generation = self.machine.generation();
                self.emit(TransportEvent::DecodeFailed {
                    peer: self.id.clone(),
                    generation,
                    reason,
                });
                // Even if the channel is fine, we cut because we cannot read what was read — written as the reader's end (the same value as a channel error).
                return match self.policy.decode_failure {
                    DecodeFailurePolicy::DropFrame => Ok(None),
                    DecodeFailurePolicy::Disconnect => {
                        Err(Input::LinkLost(DisconnectCause::ReadFailed))
                    }
                };
            }
        };
        Ok(self.route(msg, charge))
    }

    /// If it is a reply, the share is released here — a reply goes not to the line but to the caller that was waiting. What it returns = the
    /// re-request that must be written ([`Self::on_frame`]).
    fn route(&mut self, msg: W::In, charge: Charge) -> Option<Frame> {
        let reply = self.wire.reply_tag(&msg);
        let mark = self.wire.stream_mark(&msg);
        if reply.is_some() && mark.is_some() {
            self.emit(TransportEvent::AmbiguousRole {
                peer: self.id.clone(),
            });
        }
        // The order is the contract — reply · stream piece · everything else.
        if let Some(tag) = reply {
            if let Some(slot) = self.pending.take(&tag) {
                let _ = slot.send(Ok(msg));
                return None;
            }
            // A late reply to a number retired by expiry · overlap is silently dropped, as designed (ADR-0181 decision 4).
            //   Anything else is a number we never requested, so we report it.
            if !self.pending.is_retired(&tag) {
                self.emit(TransportEvent::UnmatchedReply {
                    peer: self.id.clone(),
                    tag,
                });
            }
            return None;
        }
        let msg = Received {
            msg,
            charge,
            generation: self.machine.generation(),
        };
        let Some(mark) = mark else {
            self.deliver(msg);
            return None;
        };
        // This supervisor uses streams — it splits the budget into the read share and the window cap (`budget.rs` header). The first
        //   piece of a newly seen stream is always a delivery (`Streams::observe_capped`), so the split comes before any hold — its order relative to
        //   the window-cap measurement below does not matter.
        self.ledger.use_streams();

        // ★The window cap is measured before holding★ — if holding up to this piece would exceed it, we see it as no room to hold and send it to the same truncation
        //   as when the count limit is full (`Streams::observe_capped`). Cutting after holding would exceed the limit in between.
        let over_cap = !self.ledger.window_fits(msg.charge.bytes());
        let now = self.resume_clock(self.clock.now());
        let verdict = self.streams.observe_capped(&mark, msg, now, over_cap);
        if matches!(verdict, Verdict::Gap { .. } | Verdict::Held { .. }) {
            // Counts the share of the piece just held as the window share — it comes out of the read gate (`budget.rs` header). When leaving the window,
            //   `deliver` returns it, and when it is dropped inside the window (the answer came · the incarnation changed), the share is released and subtracted from both.
            if let Some(held) = self.streams.held_mut(&mark.key, mark.seq) {
                held.charge.hold();
            }
        }
        let (event, resume_after, delivered) = match verdict {
            Verdict::Deliver(msg) => (None, None, vec![msg]),
            Verdict::Duplicate | Verdict::Held { .. } => (None, None, Vec::new()),
            Verdict::GenerationChanged {
                from,
                to,
                msg,
                discarded,
            } => (
                Some(TransportEvent::StreamGenerationChanged {
                    peer: self.id.clone(),
                    stream: mark.key.clone(),
                    from,
                    to,
                    discarded: discarded as u64,
                }),
                None,
                vec![msg],
            ),
            Verdict::Gap {
                expected,
                got,
                resume_from,
            } => (
                Some(TransportEvent::StreamGap {
                    peer: self.id.clone(),
                    stream: mark.key.clone(),
                    generation: mark.generation,
                    expected,
                    got,
                    resumed_from: Some(resume_from),
                }),
                Some(resume_from),
                Vec::new(),
            ),
            Verdict::Truncated {
                oldest_valid,
                flushed,
            } => (
                Some(TransportEvent::StreamTruncated {
                    peer: self.id.clone(),
                    stream: mark.key.clone(),
                    generation: mark.generation,
                    oldest_valid,
                }),
                None,
                flushed,
            ),
        };
        if let Some(event) = event {
            self.emit(event);
        }
        for msg in delivered {
            self.deliver(msg);
        }
        self.flush_evictions();
        resume_after.map(|after| {
            let out = self.wire.resume_request(&mark.key, Some(after));
            self.wire.encode(&out)
        })
    }

    /// Reports the forgotten stream to make room, and sends out the pieces it was holding.
    ///
    /// ★If eviction swallows a pending re-request whole, that hole is never filled★ — it vanishes from `pending_resumes`
    /// so a reconnect cannot request it again, and no truncation event is emitted either.
    fn flush_evictions(&mut self) {
        for evicted in self.streams.drain_evicted() {
            self.emit(TransportEvent::StreamForgotten {
                peer: self.id.clone(),
                stream: evicted.key.clone(),
                generation: evicted.generation,
                last_seq: evicted.last_seq,
            });
            if let Some(oldest_valid) = evicted.oldest_valid {
                self.emit(TransportEvent::StreamTruncated {
                    peer: self.id.clone(),
                    stream: evicted.key,
                    generation: evicted.generation,
                    oldest_valid,
                });
            }
            for msg in evicted.flushed {
                self.deliver(msg);
            }
        }
    }

    /// Reports the closed resume window and sends up what it held, in order.
    fn hand_over_truncations(&mut self, collapsed: Vec<Truncation<W::StreamKey, Received<W::In>>>) {
        for collapsed in collapsed {
            self.emit(TransportEvent::StreamTruncated {
                peer: self.id.clone(),
                stream: collapsed.key,
                generation: collapsed.generation,
                oldest_valid: collapsed.oldest_valid,
            });
            for msg in collapsed.flushed {
                self.deliver(msg);
            }
        }
    }

    // ── 쓰기 ────────────────────────────────────────────────────────────────

    /// ★Also listens for control and receiver-end closing while writing★ — if not, `close()` · the receiving side's disappearance is heard late by `write_deadline`
    /// (multiplied by the number of writes where the queue is drained back to back). If that side wins, it drops the future being written and
    /// drops the channel (the machine emits `DropLink`).
    /// ★We do not know how far that frame got★ — it may never have been polled, or it may have gone into the adapter buffer and
    /// go out ahead of `drop_link`'s close. If it is a notification · fanout, [`Self::write_or_count`] counts it.
    async fn write(&mut self, frame: Frame) -> Result<(), Input> {
        let Some(mut tx) = self.tx.take() else {
            return Err(Input::LinkLost(DisconnectCause::WriteFailed));
        };
        let result = self.guard(tx.send(frame)).await;
        self.tx = Some(tx);
        result
    }

    /// Writes a frame with no reply. ★If it does not end in `Ok`, it counts regardless of the reason★ — the handle already took an
    /// `Ok` when it queued, so not counting here is a silent loss (that rule of `fail_pending`). A frame left half-written
    /// may in fact have gone out, so it can be inflated ([`Direction::Outbound`] rustdoc).
    async fn write_or_count(&mut self, frame: Frame) -> Option<Input> {
        let result = self.write(frame).await;
        if result.is_err() {
            self.dropped_out += 1;
        }
        result.err()
    }

    async fn ping(&mut self) -> Result<(), Input> {
        let Some(mut tx) = self.tx.take() else {
            return Err(Input::LinkLost(DisconnectCause::WriteFailed));
        };
        let result = self.guard(tx.ping()).await;
        self.tx = Some(tx);
        result
    }

    /// Pushes one write (frame · keepalive) to the end within `write_deadline` while listening for control · receiver-end closing · the pre-close flush's
    /// deadline (`write`'s rustdoc). The channel's write end is briefly taken out and held by the caller — meanwhile `self.tx` is
    /// empty.
    ///
    /// ★The flush deadline cuts the write even if it is set mid-write★ — it hears the wake announcing the start (`flush_wake`), sets the deadline, and
    /// keeps waiting on the same write. If it does not listen, one write set before the drain started holds the close until `write_deadline`
    /// (TRD 3 §3-0 ④ 「시한 하나」 (one deadline)). If the deadline has already passed, it does not start the write.
    ///
    /// ★During a drain, 「지금 다시 해라」 (retry now) is dropped★ — also one called before the drain and not yet read (`Self::drain_ctl`).
    /// If heard, it would drop what was being written and go to redial, so that drain would end without the close it was given ([`Peer::close_after_flush`]).
    async fn guard(
        &mut self,
        op: impl Future<Output = Result<(), LinkError>>,
    ) -> Result<(), Input> {
        self.note_flush();
        let clock = self.clock.clone();
        if self
            .flush_deadline
            .is_some_and(|deadline| deadline <= clock.now())
        {
            return Err(Input::Close);
        }
        let op = with_deadline(&*clock, self.policy.write_deadline, op);
        tokio::pin!(op);
        loop {
            let flush_deadline = self.flush_deadline;
            // ★Make the sleeper only when a deadline is set★ — we do not stand a timer per write. If not set, the arm below is
            //   off and is not polled.
            let flush_due = OptionFuture::from(
                flush_deadline
                    .map(|deadline| clock.sleep(deadline.saturating_duration_since(clock.now()))),
            );
            let woken = {
                let ctl = &mut self.ctl_rx;
                let lane = &self.inbound;
                let flush = &self.flush_wake;
                let goodbye = &self.goodbye;
                // The write arm being ahead of the deadline arm is intended — if both are ready in the same turn (the write finished at the deadline), we usually take the finished write
                //   and do not count a frame that already went out as dropped (there is no `biased` so the starting arm is random).
                tokio::select! {
                    signal = ctl.recv() => match signal {
                        // Drop it and keep waiting on the same write — we are draining, so set the deadline together (`Guarded::Flush`).
                        Some(Ctl::ReconnectNow) if goodbye.get().is_some() => Guarded::Flush,
                        Some(Ctl::ReconnectNow) => Guarded::Done(Err(Input::ReconnectNow)),
                        Some(Ctl::Close) | None => Guarded::Done(Err(Input::Close)),
                    },
                    () = lane.closed() => Guarded::Done(Err(Input::Close)),
                    () = flush.notified(), if flush_deadline.is_none() => Guarded::Flush,
                    result = &mut op => Guarded::Done(match result {
                        Ok(Ok(())) => Ok(()),
                        Ok(Err(_)) => Err(Input::LinkLost(DisconnectCause::WriteFailed)),
                        Err(_) => Err(Input::LinkLost(DisconnectCause::WriteDeadline)),
                    }),
                    _ = flush_due, if flush_deadline.is_some() => Guarded::Done(Err(Input::Close)),
                }
            };
            match woken {
                Guarded::Done(result) => return result,
                Guarded::Flush => self.note_flush(),
            }
        }
    }

    // ── 정리 ────────────────────────────────────────────────────────────────

    /// Drops the channel — tries to send a close code, and folds the reading task.
    ///
    /// ★Here we do not listen to control — we push the close through to the end★. Without `biased`, `select!` picks
    /// the starting arm at random on every poll (external fact), and if a control arm is set, **while even one signal
    /// is left in the queue** that arm is immediately `Ready`, so the close future is dropped without being polled even once.
    /// Measured 2026-09-07: if `reconnect_now()` is called twice (the second signal stays in the queue), the goodbye did not go out
    /// in 41 · 54 · 49 · 49 of 100 runs even with no congestion at all. ★Do not revert★ — that loss flips not the text but the value the consumer
    /// branches on: if the close frame goes out, the peer reads 「말하고 갔다」 (said goodbye and left), and if not, tungstenite raises
    /// `ResetWithoutClosingHandshake` and it reads 「통로 오류」 (channel error).
    ///
    /// ★What the control arm earned was only delay, and that delay was already bounded★ — even if control cuts a pending close, the close code
    /// does not go out anyway, and if it does not cut, it is cut at `write_deadline`. So that arm was **delivering not one more frame**
    /// while dropping half of the frames that are delivered.
    ///
    /// ★When congested, the code is still lost★ — `write_deadline` cuts this wait (`lib.rs` 「닫기 코드가
    /// 상대에게 닿는 자리는 좁다」 — the place where the close code reaches the peer is narrow).
    ///
    /// ★The only thing we listen for while waiting is the start of the pre-close flush★ — during a pre-close flush this wait is the shorter of `write_deadline`
    /// and the time left on that deadline (that deadline covers through the close frame — [`Peer::close_after_flush`]). If the drain starts during this
    /// wait, we set the deadline on the spot and cut the remaining wait to it — if we do not listen, the end of that drain
    /// is held up by this close until `write_deadline`. The close future is held, not dropped, and pushed on, so the loss of the random arm above does
    /// not occur.
    ///
    /// ★Known limit — this wait is tied to the clock★. In a harness that pushes its own clock by hand, if the channel is dropped while a write is blocked,
    /// the supervisor stands here until the clock is pushed by `write_deadline` or the stall is released
    /// (symptom: `close()` was called but the peer does not fold). The operating clock always flows, so there
    /// `write_deadline` is the limit. The canonical way to compute it on the harness side = `testing::MemoryEndpoint::stall_writes`.
    async fn drop_link(&mut self, code: CloseCode) {
        self.note_flush();
        if let Some(mut tx) = self.tx.take() {
            let clock = self.clock.clone();
            // Loads the close that [`Peer::close_after_flush`] wrote only when dropping because of the app's close. What decides it is the cause that just woke the waiters
            //   (`last_cause`) — the machine wakes the waiters with that cause before dropping the channel (`FailPending` comes before
            //   `DropLink`) and `fail_pending` writes it. The app's close is `Explicit`, and when the channel is lost mid-drain and
            //   folded into a close (`fold_into_close`), it is that disconnect's cause so it loads the `code` the machine chose. ★Do not decide by machine state★ —
            //   folding sends the machine to `Closed` first so the two look the same.
            let lost = cause_from_code(self.last_cause.load(Ordering::Acquire))
                != DisconnectCause::Explicit;
            let close = match self.goodbye.get() {
                Some(goodbye) if !lost => goodbye.clone(),
                _ => Close::new(code, "closing"),
            };
            let closing = with_deadline(&*clock, self.policy.write_deadline, tx.close(close));
            tokio::pin!(closing);
            loop {
                let flush_deadline = self.flush_deadline;
                let flush_due =
                    OptionFuture::from(flush_deadline.map(|deadline| {
                        clock.sleep(deadline.saturating_duration_since(clock.now()))
                    }));
                let flush = &self.flush_wake;
                // ★The close arm is first and `biased`★ — even if the flush deadline has already passed, we push the close once
                //   (the same promise as 0 in `with_deadline`). The flush-start arm does not drop the close, so being ahead of it is harmless.
                let started = tokio::select! {
                    biased;
                    _ = &mut closing => false,
                    () = flush.notified(), if flush_deadline.is_none() => true,
                    _ = flush_due, if flush_deadline.is_some() => false,
                };
                if !started {
                    break;
                }
                self.note_flush();
            }
        }
        self.rx_raw = None;
        // For a branch that left the operating phase, `salvage_read_ahead` already drained and folded — what is left here is only the case where the receiving side
        //   is gone and there is nowhere to hand over.
        self.frames_rx = None;
        self.fresh_link = false;
        if let Some(reader) = self.reader.take() {
            reader.abort();
            // ★Wait for the cancellation★ — the reading task holds the other half of the channel, so until it is dropped
            //   the socket does not close (that contract of `ws::split_link`). If we do not wait, the closing moment becomes the runtime's reclaim
            //   moment, and if we count channels the peer attached to, it looks like two for a while. `read_link` always sits at an `await`
            //   point, so this wait does not exceed one turn.
            let _ = reader.await;
        }
    }

    /// ★What was already read is handed over before dropping the channel★ — it is called before feeding the machine input on every branch that leaves the operating phase
    ///   (the app closed · reconnect · write failure · silence · overflow …) (`run`). Because of that place, it comes before the waiter wake-up · disconnect event the machine
    ///   would emit: replies go to the waiting caller, and everything else stands in the ordering area ahead of the disconnect event. The generation
    ///   marker is also the same current generation as during operation (the new generation is drawn by the next handshake). Under backpressure
    ///   the read-ahead queue may be full, so without this that queue ([`Policy::frame_buffer`] frames) would be silently dropped.
    ///
    /// ★Fold the read task first★ — that task reads only after taking a read-ahead queue slot (`read_link`), so after folding
    /// what is in the queue is all that was read. If we drained without folding, it would read more of the channel into the freed slots.
    /// ★We wait for nothing except one turn for that cancellation★ (that same wait of `drop_link` moved earlier — TRD 3
    /// §3-0 ② 「여기까지 기다림 없이」 (no waiting up to here)). It does not write re-requests either — that window stays open and goes out again on the next channel
    /// (`reissue_resumes`).
    ///
    /// What it returns = the frontmost reader's end already in the read-ahead queue (the one frame that cuts on decode failure · the peer's close · channel
    /// error) — whether to rewrite the disconnect cause to that is decided by [`earlier_end`].
    async fn salvage_read_ahead(&mut self) -> Option<DisconnectCause> {
        let mut frames = self.frames_rx.take()?;
        if let Some(reader) = self.reader.take() {
            reader.abort();
            let _ = reader.await;
        }
        // The frontmost read end — the one frame that cuts on decode failure comes before the close after it (the same cause as when it was cut at that one frame
        //   during operation).
        let mut cut = None;
        while let Ok(msg) = frames.try_recv() {
            match msg {
                // ★Even under a policy that cuts on decode failure, we do not stop and hand over what is after it too★ — during operation, if it cuts under that policy,
                //   `run_live` returns at that one frame and what was already read after it is handed over by this drain. If it stopped only here,
                //   the same read-ahead queue would be handed over or dropped depending on whether the place where it cut was operating or draining.
                //   Only the disconnect cause is written as that one frame's. The re-request returned is not written (rustdoc above).
                LinkMsg::Frame(frame, charge) => {
                    if let Err(Input::LinkLost(cause)) = self.on_frame(frame, charge) {
                        cut.get_or_insert(cause);
                    }
                }
                // There is nothing after that (the reading task ended there).
                LinkMsg::Closed => return cut.or(Some(DisconnectCause::PeerClosed)),
                LinkMsg::Failed => return cut.or(Some(DisconnectCause::ReadFailed)),
            }
        }
        cut
    }

    fn fail_pending(&mut self, cause: DisconnectCause) {
        self.last_cause.store(cause_code(cause), Ordering::Release);
        let generation = self.machine.generation();
        // ★What is in the pending table is sent★ — what gets into the table is just before the write (`on_cmd`), so it may have gone to the peer. It must pass the stamped
        //   generation comparison to get in, so that generation is the current generation.
        for (_, slot) in self.pending.drain() {
            let _ = slot.send(Err(RequestError::Disconnected {
                generation,
                cause,
                sent: true,
            }));
        }
        // ★It also collects what sleeps in the queue★ — requests are woken as not sent, and those with no reply are **counted** (the handle already took an `Ok`,
        //   so not counting here is a silent loss as is).
        while let Ok(cmd) = self.data_rx.try_recv() {
            self.reject_command(cmd);
        }
    }

    /// Ends one command that will not be written — it arrived while not in the operating phase, or the stamped generation is not the current one (`on_cmd`).
    /// A request is woken with the generation it is bound to (the stamped value) and the last disconnect cause the supervisor knows — that cause may not be why the stamped generation ended
    /// but a later disconnect's (if it re-attached several times in between). Those with no reply are counted.
    fn reject_command(&mut self, cmd: PeerCmd<W>) {
        match cmd.op {
            PeerOp::Request { reply, .. } => {
                let _ = reply.send(Err(RequestError::Disconnected {
                    generation: cmd.generation,
                    cause: cause_from_code(self.last_cause.load(Ordering::Acquire)),
                    sent: false,
                }));
            }
            // ★Do not fold with `_`★ — so that when a new branch is dropped, it is decided on the spot whether to count it · wake it · end it separately.
            PeerOp::Notify { .. } | PeerOp::Raw(_) | PeerOp::Resume { .. } => self.dropped_out += 1,
            // The marker is not a message so it is not counted. Even if dropped, the close is not lost — the `goodbye` the handle wrote before the marker is
            //   seen and folded by every waiting loop outside operation (the places that call this are outside operation or already closing).
            PeerOp::Close { .. } => {}
            // Not a message, and its connection is gone already — nothing to drop, nothing to count.
            PeerOp::DropLink { .. } => {}
        }
    }

    /// ★Draining the ordering area comes last★ — only that waits on the receiving side, so what comes before it (waking the waiters · closing the outbound
    /// queue · rejecting the remaining commands) is not tied to the receiving side's speed. If it came first, requests · notifications that entered the queue in the meantime would hang until the receiving
    /// side drains (at most [`CLOSE_DRAIN_LIMIT`]).
    async fn teardown(&mut self) {
        let cause = self.end_cause;
        self.drop_link(CloseCode::GOING_AWAY).await;
        self.fail_pending(cause);
        self.data_rx.close();
        // Wake the rest too — anything that comes in after the close is rejected by the channel as `Closed`.
        while let Ok(cmd) = self.data_rx.try_recv() {
            self.reject_command(cmd);
        }
        // ★Fold the open resume window★ — the ledger disappears with this supervisor, so if not folded, held pieces vanish with neither delivery nor truncation
        //   report. It goes out with the same report as when a window closes on its deadline.
        let open = self.streams.close_resumes();
        self.hand_over_truncations(open);
        self.report_drops();
        self.emit(TransportEvent::Closed {
            peer: self.id.clone(),
            generation: self.machine.generation(),
            cause,
        });
        self.hand_over_held().await;
    }

    async fn hand_over_held(&mut self) {
        if self.order.is_empty() {
            return;
        }
        let clock = self.clock.clone();
        let (lane, order) = (&self.inbound, &mut self.order);
        let all = async move {
            while !order.is_empty() {
                if !drain_order(lane, order).await {
                    return false;
                }
            }
            true
        };
        match with_deadline(&*clock, CLOSE_DRAIN_LIMIT, all).await {
            Ok(open) => self.note_room(open),
            Err(_) => self.order.clear(),
        }
    }

    // ── Sending upward ───────────────────────────────────────────────────────

    fn emit_machine(&mut self, event: MachineEvent) {
        let peer = self.id.clone();
        let event = match event {
            MachineEvent::Connected { generation } => {
                TransportEvent::Connected { peer, generation }
            }
            MachineEvent::Disconnected { generation, cause } => {
                self.last_cause.store(cause_code(cause), Ordering::Release);
                TransportEvent::Disconnected {
                    peer,
                    generation,
                    cause,
                }
            }
            MachineEvent::ConnectFailed {
                attempt,
                of,
                cause,
                retry_in,
            } => TransportEvent::ConnectFailed {
                peer,
                attempt,
                of,
                cause,
                retry_in,
            },
            MachineEvent::Rejected { code, reason } => {
                TransportEvent::Rejected { peer, code, reason }
            }
            MachineEvent::GaveUp { reason } => TransportEvent::GaveUp { peer, reason },
        };
        self.emit(event);
    }

    fn emit(&mut self, event: TransportEvent<W>) {
        self.put(Charged::bare(Incoming::Event(event)));
    }

    fn deliver(&mut self, msg: Received<W::In>) {
        let Received {
            msg,
            mut charge,
            generation,
        } = msg;
        // ★Everything going up stands at the read gate★ — a piece that came out of the window is subtracted from the window share (`budget.rs` header). If left
        //   in the window share, the window share fills just because the receiving side is slow and the next hole cannot be held.
        charge.unhold();
        let arrival = Arrival {
            peer: self.id.clone(),
            generation,
            msg,
        };
        self.put(Charged {
            item: Incoming::Message(arrival),
            charge,
        });
    }

    /// Puts at the end of the ordering area and loads as much as fits in the line. ★It does not wait★ — because the places that put in (events · packets · deadline
    /// events · reports) are all synchronous. If it waited there, control and `fail_pending` would stall and teardown would hang.
    ///
    /// ★A full line does not cut or drop this connection★ — that line belongs to this handle (`PeerConfig::inbound`), so
    /// being full means the consumer cannot keep up with this peer, not that the connection is at fault (ADR-0310 decision 17). Instead, while the ordering
    /// area holds, the operating loop stops that peer's reading (「배압」 (backpressure) in this file's header).
    fn put(&mut self, item: Charged<Incoming<W>>) {
        if self.consumer_gone.load(Ordering::Acquire) {
            return;
        }
        let was_empty = self.order.is_empty();
        self.order.push_back(item);
        let open = flush(&self.inbound, &mut self.order);
        // The ordering area just started holding — this is the time this peer's backpressure started (`track_pause`).
        if was_empty && !self.order.is_empty() {
            self.order_since = self.clock.now();
        }
        self.note_room(open);
    }

    /// Reports the counted outbound losses. ★It reports regardless of policy★ — if it stopped counting, it would
    /// be a worse choice than `Disconnect` (that sentence in policy.rs).
    fn report_drops(&mut self) {
        let count = std::mem::take(&mut self.dropped_out);
        if count == 0 {
            return;
        }
        // If a report not yet loaded is at the end of the ordering area, add to it — so one does not pile up per turn while the line is backed up.
        if let Some(Charged {
            item: Incoming::Event(TransportEvent::Dropped { count: held, .. }),
            ..
        }) = self.order.back_mut()
        {
            *held += count;
            return;
        }
        self.emit(TransportEvent::Dropped {
            peer: self.id.clone(),
            direction: Direction::Outbound,
            count,
        });
    }

    /// Collects and handles what the handle could not put in.
    ///
    /// ★Only outbound-queue saturation cuts the connection★ — 「내 큐가 찼다」 (my queue is full) is this connection's problem, and 「위로 올라가는 줄이
    /// 찼다」 (the upward line is full) is the consumer's speed (`put`). Merging the two into one count field would cut the connection just because the consumer is slow.
    ///
    /// ★Cut only when that overflow belongs to the current generation★ — the count outlives generations: an overflow counted in generation G is collected by G+1's first turn after that generation ended,
    /// passing the connection-establishment phase (which does not call this). Enforcing the policy there would
    /// cut the connection that just attached over the old connection's overflow. The count is reported regardless of generation.
    #[must_use = "운영 중에는 이 판정이 연결을 끊는다"]
    fn drain_overflow(&mut self) -> Option<Input> {
        let refused = self.refused.swap(0, Ordering::AcqRel);
        if refused > 0 {
            self.dropped_out += refused;
        }
        let full = self.overflow.swap(0, Ordering::AcqRel);
        if full == 0 {
            return None;
        }
        self.dropped_out += full;
        self.report_drops();
        // Read after collecting the count — a generation at least that of the collected overflow is visible (`Peer::count_overflow`).
        let latest = Generation(self.overflow_generation.load(Ordering::Acquire));
        if latest != self.machine.generation() {
            return None;
        }
        match self.policy.full_queue {
            QueueFullPolicy::Disconnect => Some(Input::LinkLost(DisconnectCause::QueueFull)),
            QueueFullPolicy::DropAndReport => None,
        }
    }
}

/// The input to feed the machine when leaving the operating phase — if the reader's end was already in the read-ahead queue salvaged while dropping the channel ([`Supervisor::salvage_read_ahead`])
/// (the frontmost one — it may be the one frame in the middle of the line that cuts on decode failure), that end is written as the disconnect cause instead of the
/// reason we left (write failure · silence · overflow …) — it is an end that was already read before dropping the channel. If the peer closes while the frame arm is pulled by backpressure, the supervisor may notice that closing
/// first as a write that failed because of it (the line item in crate header 「알려진 한계」 (known limits)).
///
/// ★What is not rewritten★ — if the app closed (`Input::Close`) or asked to reconnect (`Input::ReconnectNow`), it stays as is: in both
/// the disconnect cause is [`DisconnectCause::Explicit`] and the caller knows that reason. Turning a reconnect into a disconnect would lose that entry point's promise to refill the budget
/// and dial at once (ADR-0180 decision 7). If our cause is already the reader's end (cut on decode
/// failure), that too stays as is — within the line that one frame comes before the end.
fn earlier_end(input: Input, read_end: Option<DisconnectCause>) -> Input {
    match (input, read_end) {
        (Input::LinkLost(ours), Some(theirs)) if !is_read_end(ours) => Input::LinkLost(theirs),
        (input, _) => input,
    }
}

/// Is it the reader's end — the peer's close · a channel read error (including cut on decode failure).
fn is_read_end(cause: DisconnectCause) -> bool {
    matches!(
        cause,
        DisconnectCause::PeerClosed | DisconnectCause::ReadFailed
    )
}

/// Loads the ordering area from the front as much as fits in the line — it does not wait. `false` = the receiving side closed (the ordering
/// area is emptied — there is nowhere to load).
fn flush<T>(lane: &mpsc::Sender<T>, order: &mut VecDeque<T>) -> bool {
    while let Some(item) = order.pop_front() {
        match lane.try_send(item) {
            Ok(()) => {}
            Err(mpsc::error::TrySendError::Full(item)) => {
                order.push_front(item);
                return true;
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                order.clear();
                return false;
            }
        }
    }
    true
}

/// Waits until a slot opens in the line and then loads as much of the ordering area as fits. `false` = the receiving side closed.
///
/// ★Cancel-safe — put it on a `select!` arm★: if dropped while waiting for a slot, the ordering area is unchanged, and after taking the slot the
/// take-out · loading finishes at once without `await`. Slots go around among the parties putting into the line in the order they waited
/// (external fact — slot reservation on a tokio bounded channel is FIFO).
async fn drain_order<T>(lane: &mpsc::Sender<T>, order: &mut VecDeque<T>) -> bool {
    let Ok(permit) = lane.reserve().await else {
        order.clear();
        return false;
    };
    if let Some(item) = order.pop_front() {
        permit.send(item);
    }
    flush(lane, order)
}

/// Opens only the channel. ★The deadline is used by the dial and the handshake **together**★ — the current `HANDSHAKE_TIMEOUT` has the same shape,
/// so if the dial drags on, the time left for the handshake shrinks by that much.
async fn dial_once(
    dialer: Arc<dyn Dialer>,
    addr: Address,
    clock: Arc<dyn Clock>,
    deadline: Instant,
) -> Result<(Box<dyn LinkTx>, Box<dyn LinkRx>), ConnectFailure> {
    let left = deadline.saturating_duration_since(clock.now());
    match with_deadline(&*clock, left, dialer.dial(&addr)).await {
        Ok(Ok(pair)) => Ok(pair),
        Ok(Err(DialFailure::Unreachable(err))) => Err(ConnectFailure::Unreachable(err)),
        // The dialing side already knows — attached but the same path as a reject (give up without budget · TRD 3 §3-0 ⑤).
        Ok(Err(DialFailure::Refused { code, reason })) => {
            debug_assert_eq!(
                RejectKind::of(code),
                Some(RejectKind::Terminal),
                "DialFailure::Refused 는 종결 거절 코드만 싣는다: {code:?}"
            );
            Err(ConnectFailure::Rejected {
                kind: RejectKind::Terminal,
                code,
                reason,
            })
        }
        Err(_) => Err(ConnectFailure::Unreachable(LinkError::new(
            "connect timed out",
        ))),
    }
}

/// The handshake did not finish — the dialing side feeds `failure` to the machine, and the accepted peer writes `end` as the end cause
/// (`Supervisor::end_cause`). Both are decided together at each failure place — the dialing-side branch (`ConnectFailure`) carries only the text of failing to attach
/// and does not tell read end from write end.
struct HandshakeFailed {
    failure: ConnectFailure,
    end: DisconnectCause,
}

impl HandshakeFailed {
    fn unreachable(err: LinkError, end: DisconnectCause) -> Self {
        Self {
            failure: ConnectFailure::Unreachable(err),
            end,
        }
    }
}

/// Runs the handshake to the end on the opened channel.
///
/// An accepted peer's end cause: our handshake's reject = `Explicit` · the peer closed (including a reject code) = `PeerClosed` · read error =
/// `ReadFailed` · write error = `WriteFailed` · write exceeded the deadline = `WriteDeadline` · the peer's frame did not arrive by the deadline =
/// `Idle`.
async fn handshake_once(
    mut tx: Box<dyn LinkTx>,
    mut rx: Box<dyn LinkRx>,
    handshake: Arc<dyn Handshake>,
    clock: Arc<dyn Clock>,
    deadline: Instant,
) -> Result<(Box<dyn LinkTx>, Box<dyn LinkRx>), HandshakeFailed> {
    let left = |clock: &dyn Clock| deadline.saturating_duration_since(clock.now());
    let mut step = handshake.start();
    loop {
        match step {
            HandshakeStep::Done => return Ok((tx, rx)),
            HandshakeStep::Reject { kind, last, reason } => {
                let code = kind.close_code();
                let mut link_usable = true;
                if let Some(frame) = last {
                    // Nothing more is written after a half-sent frame — the channel is simply dropped.
                    link_usable = matches!(
                        with_deadline(&*clock, left(&*clock), tx.send(frame)).await,
                        Ok(Ok(()))
                    );
                }
                if link_usable {
                    let close = Close::new(code, reason.clone());
                    let _ = with_deadline(&*clock, left(&*clock), tx.close(close)).await;
                }
                return Err(HandshakeFailed {
                    failure: ConnectFailure::Rejected { kind, code, reason },
                    end: DisconnectCause::Explicit,
                });
            }
            HandshakeStep::Send(frame) => {
                match with_deadline(&*clock, left(&*clock), tx.send(frame)).await {
                    Ok(Ok(())) => {}
                    Ok(Err(err)) => {
                        return Err(HandshakeFailed::unreachable(
                            err,
                            DisconnectCause::WriteFailed,
                        ))
                    }
                    Err(_) => {
                        return Err(HandshakeFailed::unreachable(
                            LinkError::new("handshake write timed out"),
                            DisconnectCause::WriteDeadline,
                        ))
                    }
                }
                step = HandshakeStep::Await;
            }
            HandshakeStep::Await => match with_deadline(&*clock, left(&*clock), rx.recv()).await {
                Ok(Ok(LinkRead::Frame(Frame::Keepalive))) => step = HandshakeStep::Await,
                Ok(Ok(LinkRead::Frame(frame))) => step = handshake.on_frame(&frame),
                // ★The peer closed carrying a code = a reject, and the kind is decided by that code★ (TRD 3 §3-0 ⑤ — text does not
                //   decide). Without this code, 「종결 거절은 예산을 안 쓴다」 (a terminal reject does not use budget) (ADR-0180 decision 5) does not hold in code.
                //   ★A bare close · a normal close code is not a 「거절」 (reject)★ — a silent drop is 「못 붙었다」 (failed to attach), so it uses budget (if the daemon
                //   is restarting, the next attempt attaches).
                Ok(Ok(LinkRead::Closed(close))) => {
                    let failure = match close {
                        Some(close) => match RejectKind::of(close.code) {
                            Some(kind) => ConnectFailure::Rejected {
                                kind,
                                code: close.code,
                                reason: close.reason,
                            },
                            None => peer_closed_during_handshake(),
                        },
                        None => peer_closed_during_handshake(),
                    };
                    return Err(HandshakeFailed {
                        failure,
                        end: DisconnectCause::PeerClosed,
                    });
                }
                Ok(Err(err)) => {
                    return Err(HandshakeFailed::unreachable(
                        err,
                        DisconnectCause::ReadFailed,
                    ))
                }
                Err(_) => {
                    return Err(HandshakeFailed::unreachable(
                        LinkError::new("handshake timed out"),
                        DisconnectCause::Idle,
                    ))
                }
            },
        }
    }
}

fn peer_closed_during_handshake() -> ConnectFailure {
    ConnectFailure::Unreachable(LinkError::new("peer closed during handshake"))
}

/// Reads from the channel and hands it to the supervisor.
///
/// ★Why a separate task★ — if `recv()` were put directly on a `select!` arm, each time another arm wins that future is
/// canceled, and on a transport that is not cancel-safe a frame vanishes at that moment.
///
/// ★If the read gate is closed, it does not do the next read★ (「더하기 전 물높이」 (fill level before adding) — `budget.rs`) — with only the slot count of the read-ahead queue,
/// that line would hold up to slot count × one-message limit. The share is added right where it is read.
///
/// ★Take the read-ahead queue slot before reading★ — it does not wait for a slot holding one frame it read. So when the supervisor
/// folds this task, everything already read is in the read-ahead queue (`Supervisor::salvage_read_ahead`). If it folded while holding one in hand,
/// that frame was already taken out of the channel but goes nowhere.
async fn read_link(mut rx: Box<dyn LinkRx>, out: mpsc::Sender<LinkMsg>, ledger: Arc<Ledger>) {
    loop {
        let Ok(slot) = out.reserve().await else {
            return;
        };
        ledger.reader_room().await;
        let (msg, last) = match rx.recv().await {
            Ok(LinkRead::Frame(frame)) => {
                let charge = ledger.charge(frame.payload_len());
                (LinkMsg::Frame(frame, charge), false)
            }
            Ok(LinkRead::Closed(_)) => (LinkMsg::Closed, true),
            Err(_) => (LinkMsg::Failed, true),
        };
        slot.send(msg);
        if last {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::budget::split;
    use crate::policy::ReconnectPolicy;
    use crate::testing::{
        settle, settle_until, ClientMsg, HelloHandshake, ImmediateHandshake, ListeningHandshake,
        ManualClock, MemoryEndpoint, MemoryNetwork, RejectingHandshake, TestIn, TestOut, TestWire,
    };
    use futures_util::future::poll_immediate;
    use std::time::Duration;

    struct Harness {
        peer: Peer<TestWire>,
        net: MemoryNetwork,
        clock: ManualClock,
        inbound: mpsc::Receiver<Charged<Incoming<TestWire>>>,
        /// Things taken out in advance to look at the fence. The assertion that follows reads these first.
        buffered: Vec<Incoming<TestWire>>,
        /// The supervisor task. ★Channels and the clock cannot tell 「끝났다」 (finished) from 「패닉했다」 (panicked)★ — a panic also drops the supervisor,
        /// closes all channels and releases all sleeping clocks.
        task: tokio::task::JoinHandle<()>,
    }

    impl Harness {
        fn events(&mut self) -> Vec<TransportEvent<TestWire>> {
            self.drain()
                .into_iter()
                .filter_map(|item| match item {
                    Incoming::Event(event) => Some(event),
                    Incoming::Message(_) => None,
                })
                .collect()
        }

        fn kinds(&mut self) -> Vec<&'static str> {
            self.events().iter().map(|e| e.kind()).collect()
        }

        fn messages(&mut self) -> Vec<TestIn> {
            self.drain()
                .into_iter()
                .filter_map(|item| match item {
                    Incoming::Message(arrival) => Some(arrival.msg),
                    Incoming::Event(_) => None,
                })
                .collect()
        }

        fn drain(&mut self) -> Vec<Incoming<TestWire>> {
            let mut out = std::mem::take(&mut self.buffered);
            while let Ok(item) = self.inbound.try_recv() {
                out.push(item.into_item());
            }
            out
        }

        /// Did a notification carrying this text come up. ★Do not discard what was taken out★ — the assertion that follows sees it.
        fn seen_notice(&mut self, body: &str) -> bool {
            let mut hit = false;
            let mut kept = Vec::new();
            while let Ok(item) = self.inbound.try_recv() {
                let item = item.into_item();
                if let Some(TestIn::Notice(text)) = item.as_message().map(|a| &a.msg) {
                    if text == body {
                        hit = true;
                    }
                }
                kept.push(item);
            }
            self.buffered.extend(kept);
            hit
        }

        fn disconnect_cause(&mut self) -> Option<DisconnectCause> {
            self.events().iter().find_map(|e| match e {
                TransportEvent::Disconnected { cause, .. } => Some(*cause),
                _ => None,
            })
        }

        /// Did teardown close the outbound queue. ★If the line had room, at the moment this is seen the last loss report and `Closed` have
        /// come up too★ — `teardown` puts in and loads straight away without `await` after closing the queue, so on a current_thread
        /// runtime this line cannot cut in between. If the line was full, teardown is still waiting in the ordering area.
        fn supervisor_done(&self) -> bool {
            self.peer.inner.data_tx.is_closed()
        }

        /// Did the supervisor take everything out of the outbound queue. ★「통로로 아무것도 안 나갔다」 (nothing went out on the channel) alone does not make 「쓰는 중」 (writing)
        /// stand★ — it is true even with things left in the queue, and what is left in the queue is counted or woken separately by `fail_pending`. Then
        /// an assertion that measures the count mid-write holds regardless of the write.
        fn queue_drained(&self) -> bool {
            let tx = &self.peer.inner.data_tx;
            tx.capacity() == tx.max_capacity()
        }

        /// Has the supervisor task ended now without panicking. Polling an ended handle again makes tokio panic and
        /// eats the harness — use it as the last assertion.
        async fn exited_cleanly(self) -> bool {
            matches!(poll_immediate(self.task).await, Some(Ok(())))
        }
    }

    fn no_jitter(policy: Policy) -> Policy {
        Policy {
            reconnect: ReconnectPolicy {
                jitter: 0.0,
                ..policy.reconnect
            },
            ..policy
        }
    }

    fn build_with(policy: Policy, handshake: Arc<dyn Handshake>, inbound_cap: usize) -> Harness {
        build_budgeted(policy, handshake, inbound_cap, BYTE_BUDGET)
    }

    fn build_budgeted(
        policy: Policy,
        handshake: Arc<dyn Handshake>,
        inbound_cap: usize,
        byte_budget: usize,
    ) -> Harness {
        let net = MemoryNetwork::new();
        let clock = ManualClock::new();
        let (inbound_tx, inbound) = mpsc::channel(inbound_cap);
        let (peer, supervisor) = new_peer(PeerConfig {
            id: PeerId::new("d1"),
            origin: Origin::Dial {
                addr: Address::new("mem://d1"),
                dialer: net.dialer(),
            },
            wire: Arc::new(TestWire),
            clock: clock.handle(),
            handshake,
            policy: no_jitter(policy),
            inbound: inbound_tx,
            byte_budget,
            generations: Arc::new(AtomicU64::new(0)),
        });
        Harness {
            peer,
            net,
            clock,
            inbound,
            buffered: Vec::new(),
            task: tokio::spawn(supervisor.run()),
        }
    }

    fn build(policy: Policy, handshake: Arc<dyn Handshake>) -> Harness {
        build_with(policy, handshake, 4096)
    }

    async fn live() -> (Harness, Arc<MemoryEndpoint>) {
        let mut h = build(Policy::default(), Arc::new(ImmediateHandshake));
        settle_until("첫 연결", || h.peer.peer_state() == PeerState::Live).await;
        assert_eq!(h.kinds(), vec!["Connected"]);
        let endpoint = h.net.accept().expect("통로가 열렸어야 한다");
        (h, endpoint)
    }

    /// Creates a state where frames **never stop**.
    ///
    /// ★A finite bundle cannot create starvation★ — once the supervisor drains it all, the frame arm becomes `Pending`
    /// and the later arms get polled then. So a test that does 「1000개를 밀어 넣고 settle」 (push in 1000 and settle) **passes the same
    /// whether `biased` or not** (measured — the first edition written in that form caught not a single mutant). Starvation arises
    /// only when **fill rate ≥ drain rate**, so we keep the supply running.
    fn flood(endpoint: &Arc<MemoryEndpoint>) -> tokio::task::JoinHandle<()> {
        let endpoint = endpoint.clone();
        tokio::spawn(async move {
            let mut seq = 1u64;
            loop {
                for _ in 0..256 {
                    endpoint.push(TestWire::chunk(1, 1, seq));
                    seq += 1;
                }
                tokio::task::yield_now().await;
            }
        })
    }

    /// Runs the supervisor a few turns under the flood. The 512 turns of [`settle`] are excessive here (the supply piles up that much).
    async fn pump(rounds: usize) {
        for _ in 0..rounds {
            tokio::task::yield_now().await;
        }
    }

    /// ★If the defect revives, it **fails cleanly instead of hanging**★ — libtest has no per-test deadline, so
    /// a regression that becomes a permanent wait would halt the whole CI without this net. The real-time 5 seconds flows
    /// **only on the failure path** (the precedent for the same net is the shell package's `lib_unit`).
    async fn joined(
        task: tokio::task::JoinHandle<Result<TestIn, RequestError>>,
    ) -> Result<TestIn, RequestError> {
        tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .expect("요청이 안 깨어났다 — 영구 대기 회귀다")
            .expect("요청 태스크가 패닉했다")
    }

    fn spawn_request(
        peer: &Peer<TestWire>,
        tag: u64,
    ) -> tokio::task::JoinHandle<Result<TestIn, RequestError>> {
        let peer = peer.clone();
        tokio::spawn(async move {
            peer.request(TestOut::Request {
                tag,
                body: format!("body{tag}"),
            })
            .await
        })
    }

    // ── C2/F1: anti-starvation rule ──
    //
    // ★These eight are the regression net for C2·F1★ — not the flood tests below. The flood side checks 「부하 아래에서 살아
    //   있나」 (does it stay alive under load), and **starvation itself cannot be created with this harness** (the read task is a structural bottleneck so
    //   the frame queue is always empty — the measurement in the `next_turn` comment).
    const CAP: usize = 512;

    #[test]
    fn a_due_deadline_beats_a_ready_frame_however_many_are_queued() {
        let t0 = Instant::now();
        for frames_in_row in [0usize, 1, FRAME_BURST, 10_000] {
            for outbound in [0usize, 1, CAP] {
                assert_eq!(
                    next_turn(t0, t0, frames_in_row, outbound, CAP),
                    Turn::Timer,
                    "frames={frames_in_row} outbound={outbound}"
                );
                assert_eq!(
                    next_turn(
                        t0 + Duration::from_secs(1),
                        t0,
                        frames_in_row,
                        outbound,
                        CAP
                    ),
                    Turn::Timer,
                    "이미 지난 시한은 더 급하다"
                );
            }
        }
    }

    #[test]
    fn a_frame_burst_yields_a_turn_to_the_outbound_queue() {
        let t0 = Instant::now();
        let future = t0 + Duration::from_secs(1);
        assert_eq!(
            next_turn(t0, future, FRAME_BURST, 1, CAP),
            Turn::Outbound(1)
        );
        assert_eq!(
            next_turn(t0, future, FRAME_BURST + 1, 3, CAP),
            Turn::Outbound(3)
        );
    }

    #[test]
    fn below_the_burst_and_before_the_deadline_the_loop_just_waits() {
        let t0 = Instant::now();
        let future = t0 + Duration::from_secs(1);
        assert_eq!(next_turn(t0, future, 0, 1, CAP), Turn::Wait);
        assert_eq!(next_turn(t0, future, FRAME_BURST - 1, 1, CAP), Turn::Wait);
    }

    #[test]
    fn an_empty_outbound_queue_never_takes_a_turn() {
        let t0 = Instant::now();
        let future = t0 + Duration::from_secs(1);
        assert_eq!(next_turn(t0, future, FRAME_BURST * 10, 0, CAP), Turn::Wait);
    }

    #[test]
    fn the_deadline_outranks_the_burst_when_both_are_due() {
        let t0 = Instant::now();
        assert_eq!(next_turn(t0, t0, FRAME_BURST, 5, CAP), Turn::Timer);
    }

    #[test]
    fn the_burst_bound_is_finite_so_the_outbound_queue_cannot_be_starved() {
        assert!(FRAME_BURST > 0);
        let t0 = Instant::now();
        let future = t0 + Duration::from_secs(1);
        // ★Search with a limit★ — an unbounded search hangs when the rule is broken.
        let first_yield = (0..FRAME_BURST * 4)
            .find(|n| matches!(next_turn(t0, future, *n, 1, CAP), Turn::Outbound(_)))
            .expect("연달아 처리하는 프레임 수에 상한이 없으면 나가는 큐가 영원히 굶는다");
        assert_eq!(first_yield, FRAME_BURST);
    }

    // ── F1: a fixed ratio alone is not enough ──
    #[test]
    fn a_backlog_past_half_capacity_does_not_wait_for_a_frame_burst() {
        let t0 = Instant::now();
        let future = t0 + Duration::from_secs(1);
        let half = CAP / 2;
        for outbound in [half, half + 1, CAP] {
            assert_eq!(
                next_turn(t0, future, 0, outbound, CAP),
                Turn::Outbound(outbound),
                "★깊이를 안 보면 비율을 넘는 생산자 앞에서 큐가 계속 자라 QueueFull 로 끝난다★"
            );
        }
        assert_eq!(
            next_turn(t0, future, 0, half - 1, CAP),
            Turn::Wait,
            "절반 아래에서는 평소대로 기다린다"
        );
    }

    #[test]
    fn a_yield_drains_the_whole_backlog_not_one_item() {
        let t0 = Instant::now();
        let future = t0 + Duration::from_secs(1);
        for outbound in [1usize, 7, 64, CAP] {
            assert_eq!(
                next_turn(t0, future, FRAME_BURST, outbound, CAP),
                Turn::Outbound(outbound),
                "한 바퀴에 한 개씩만 빼면 문턱만 32배 높아진 같은 결함이다"
            );
        }
    }

    // ── Connection establishment ──
    #[tokio::test]
    async fn a_peer_dials_and_reaches_live_on_its_own() {
        let (h, _endpoint) = live().await;
        assert_eq!(h.peer.generation(), Generation(1));
        assert_eq!(h.net.dials(), 1);
    }

    #[tokio::test]
    async fn a_two_step_handshake_round_trips_before_going_live() {
        let h = build(Policy::default(), Arc::new(HelloHandshake));
        settle_until("핸드셰이크 진입", || {
            h.peer.peer_state() == PeerState::Handshaking
        })
        .await;
        let endpoint = h.net.accept().expect("통로");
        assert_eq!(
            endpoint.frames(),
            vec![Frame::Text(HelloHandshake::HELLO.into())]
        );
        endpoint.push(Frame::Text(HelloHandshake::WELCOME.into()));
        settle_until("운영 진입", || h.peer.peer_state() == PeerState::Live).await;
    }

    // A dial that lands but whose handshake never answers fails at the connect deadline (dial and handshake share it)
    //   as a network-class failure, and the peer backs off.
    #[tokio::test]
    async fn a_handshake_that_never_answers_fails_at_the_connect_deadline_and_backs_off() {
        let mut h = build(Policy::default(), Arc::new(HelloHandshake));
        settle_until("handshaking", || {
            h.peer.peer_state() == PeerState::Handshaking
        })
        .await;
        // Held and never answered.
        let _endpoint = h.net.accept().expect("a link was opened");
        let deadline = Policy::default().connect_timeout;

        h.clock.advance(deadline - Duration::from_millis(1));
        settle().await;
        assert_eq!(
            h.peer.peer_state(),
            PeerState::Handshaking,
            "not before the deadline"
        );

        h.clock.advance(Duration::from_millis(1));
        settle_until("backoff", || h.peer.peer_state() == PeerState::Backoff).await;
        let cause = h
            .events()
            .into_iter()
            .find_map(|event| match event {
                TransportEvent::ConnectFailed { cause, .. } => Some(cause),
                _ => None,
            })
            .expect("the attempt failed");
        assert!(
            matches!(&cause, ConnectFailure::Unreachable(e) if e.message == "handshake timed out"),
            "{cause:?}"
        );
    }

    // ── D1: generations come from the per-name-tag shared counter ──
    #[tokio::test]
    async fn the_supervisor_mints_its_generation_from_the_shared_counter() {
        let net = MemoryNetwork::new();
        let clock = ManualClock::new();
        let (inbound_tx, _inbound) = mpsc::channel(64);
        let counter = Arc::new(AtomicU64::new(41));
        let peer = spawn_peer(PeerConfig {
            id: PeerId::new("d1"),
            origin: Origin::Dial {
                addr: Address::new("mem://d1"),
                dialer: net.dialer(),
            },
            wire: Arc::new(TestWire),
            clock: clock.handle(),
            handshake: Arc::new(ImmediateHandshake),
            policy: Policy::default(),
            inbound: inbound_tx,
            byte_budget: BYTE_BUDGET,
            generations: counter.clone(),
        });
        settle_until("연결", || peer.peer_state() == PeerState::Live).await;
        assert_eq!(peer.generation(), Generation(42));
        assert_eq!(
            counter.load(Ordering::Acquire),
            42,
            "계수기가 함께 올라간다"
        );
    }

    #[tokio::test]
    async fn two_supervisors_sharing_a_counter_never_mint_the_same_generation() {
        let net = MemoryNetwork::new();
        let clock = ManualClock::new();
        let (inbound_tx, _inbound) = mpsc::channel(256);
        let counter = Arc::new(AtomicU64::new(0));
        let spawn = |tag: &str| {
            spawn_peer(PeerConfig {
                id: PeerId::new(tag),
                origin: Origin::Dial {
                    addr: Address::new("mem://d1"),
                    dialer: net.dialer(),
                },
                wire: Arc::new(TestWire),
                clock: clock.handle(),
                handshake: Arc::new(ImmediateHandshake),
                policy: Policy::default(),
                inbound: inbound_tx.clone(),
                byte_budget: BYTE_BUDGET,
                generations: counter.clone(),
            })
        };
        // A situation where a new supervisor attaches while the old supervisor is still alive (a succession race on the same name tag).
        let first = spawn("d1");
        let second = spawn("d1");
        settle_until("둘 다 연결", || {
            first.peer_state() == PeerState::Live && second.peer_state() == PeerState::Live
        })
        .await;
        assert_ne!(
            first.generation(),
            second.generation(),
            "★같은 번호를 두 감독이 발행하면 소비자가 어느 소켓의 것인지 못 가린다★"
        );
    }

    // ── Request/response ──
    #[tokio::test]
    async fn a_request_is_registered_before_it_is_written() {
        let (h, endpoint) = live().await;
        let task = spawn_request(&h.peer, 5);
        settle().await;
        assert_eq!(endpoint.frames(), vec![Frame::Text("req|5|body5".into())]);
        endpoint.push(TestWire::reply(5));
        settle().await;
        assert_eq!(
            joined(task).await,
            Ok(TestIn::Reply {
                tag: 5,
                body: "ok".into()
            })
        );
    }

    #[tokio::test]
    async fn a_request_that_takes_no_reply_fails_at_once() {
        let (h, _endpoint) = live().await;
        let out = h.peer.request(TestOut::Notice("hi".into())).await;
        assert_eq!(out, Err(RequestError::NotARequest));
    }

    #[tokio::test]
    async fn only_the_silent_request_times_out() {
        let (mut h, endpoint) = live().await;
        let a = spawn_request(&h.peer, 1);
        settle().await;
        let b = spawn_request(&h.peer, 2);
        settle().await;
        endpoint.push(TestWire::reply(2));
        settle().await;
        assert!(joined(b).await.is_ok());

        h.clock.advance(Duration::from_secs(8));
        settle().await;
        assert_eq!(
            joined(a).await,
            Err(RequestError::TimedOut {
                after: Duration::from_secs(8)
            })
        );
        assert!(h.kinds().contains(&"RequestTimedOut"));
        assert_eq!(
            h.peer.peer_state(),
            PeerState::Live,
            "시한 만료는 연결을 건드리지 않는다"
        );
    }

    #[tokio::test]
    async fn a_reply_that_arrives_after_the_timeout_is_dropped_and_the_tag_is_retired() {
        let (mut h, endpoint) = live().await;
        let first = spawn_request(&h.peer, 9);
        settle().await;
        h.clock.advance(Duration::from_secs(8));
        settle().await;
        assert!(matches!(
            joined(first).await,
            Err(RequestError::TimedOut { .. })
        ));
        h.drain();

        endpoint.push(TestWire::reply(9));
        settle().await;
        assert!(h.messages().is_empty(), "늦은 답장은 조용히 버려진다");
        assert!(
            !h.kinds().contains(&"UnmatchedReply"),
            "설계된 늦은 답장은 신고 대상이 아니다"
        );

        let again = h.peer.request(TestOut::Request {
            tag: 9,
            body: "b".into(),
        });
        assert_eq!(again.await, Err(RequestError::TagRetired));
    }

    // ── M1: an overlapping number cannot tell deliveries apart, so both sides are failed ──
    #[tokio::test]
    async fn a_colliding_tag_wakes_the_older_waiter_and_refuses_the_newer_one() {
        let (h, endpoint) = live().await;
        let old = spawn_request(&h.peer, 3);
        settle().await;
        let new = spawn_request(&h.peer, 3);
        settle().await;
        assert_eq!(joined(old).await, Err(RequestError::Superseded));
        assert_eq!(joined(new).await, Err(RequestError::TagRetired));

        // Even if the late reply to the request that went out first arrives, it is delivered to no one.
        endpoint.push(TestWire::reply(3));
        settle().await;
    }

    #[tokio::test]
    async fn a_reply_nobody_asked_for_is_reported() {
        let (mut h, endpoint) = live().await;
        h.drain();
        endpoint.push(TestWire::reply(77));
        settle().await;
        assert!(h.kinds().contains(&"UnmatchedReply"));
    }

    // ── C1: no branch that drops the connection leaves a waiter hanging ──
    #[tokio::test]
    async fn a_pending_request_is_woken_when_the_shell_asks_to_reconnect() {
        let (h, _endpoint) = live().await;
        let pending = spawn_request(&h.peer, 11);
        settle().await;
        h.peer.reconnect_now();
        settle().await;
        assert!(
            matches!(
                joined(pending).await,
                Err(RequestError::Disconnected { .. })
            ),
            "reconnect_now 만 FailPending 을 안 내면 이 슬롯은 영구 대기가 된다"
        );
    }

    #[tokio::test]
    async fn a_pending_request_does_not_hang_when_reconnect_then_exhausts_the_budget() {
        let (h, _endpoint) = live().await;
        let pending = spawn_request(&h.peer, 12);
        settle().await;

        h.net.fail_forever();
        h.peer.reconnect_now();
        settle().await;
        for delay in [500u64, 1000, 2000] {
            h.clock.advance(Duration::from_millis(delay));
            settle().await;
        }
        assert_eq!(
            h.peer.peer_state(),
            PeerState::GaveUp(crate::machine::GaveUpReason::BudgetExhausted)
        );
        assert!(matches!(
            joined(pending).await,
            Err(RequestError::Disconnected { .. })
        ));
    }

    #[tokio::test]
    async fn a_request_that_lands_after_the_link_is_gone_is_refused_not_parked() {
        let (h, endpoint) = live().await;
        // Imitates a race where it goes into the queue but the supervisor has already left the operating phase.
        let late = {
            let peer = h.peer.clone();
            tokio::spawn(async move {
                peer.request(TestOut::Request {
                    tag: 21,
                    body: "x".into(),
                })
                .await
            })
        };
        endpoint.close();
        settle().await;
        let outcome = joined(late).await;
        assert!(
            matches!(outcome, Err(RequestError::Disconnected { .. })),
            "{outcome:?}"
        );
    }

    #[tokio::test]
    async fn losing_the_link_wakes_every_waiter() {
        let (mut h, endpoint) = live().await;
        let pending = spawn_request(&h.peer, 1);
        settle().await;
        endpoint.close();
        settle().await;
        assert!(matches!(
            joined(pending).await,
            Err(RequestError::Disconnected { .. })
        ));
        assert!(h.kinds().contains(&"Disconnected"));
    }

    // ── Generation stamp · the sent field (TRD 3 §3-0 ③ · D12 — the transport edition of the shell's two ADR-0195 tests) ──

    /// The data frames the peer received, as text — the harness vocabulary is all `Frame::Text`.
    fn texts(frames: Vec<Frame>) -> Vec<String> {
        frames
            .into_iter()
            .filter_map(|frame| match frame {
                Frame::Text(text) => Some(text),
                _ => None,
            })
            .collect()
    }

    fn spawn_request_in(
        peer: &Peer<TestWire>,
        generation: Generation,
        tag: u64,
    ) -> tokio::task::JoinHandle<Result<TestIn, RequestError>> {
        let peer = peer.clone();
        tokio::spawn(async move {
            peer.request_in(
                generation,
                TestOut::Request {
                    tag,
                    body: format!("body{tag}"),
                },
            )
            .await
        })
    }

    // The `request_in` contract a consumer relies on: one poll queues the request, so what it queues next goes out
    //   after it, while the reply is awaited later.
    #[tokio::test]
    async fn the_first_poll_of_request_in_queues_the_request() {
        let (h, endpoint) = live().await;
        let g = h.peer.generation();
        let mut asked = Box::pin(h.peer.request_in(
            g,
            TestOut::Request {
                tag: 7,
                body: "body7".into(),
            },
        ));
        assert!(
            poll_immediate(asked.as_mut()).await.is_none(),
            "waits for the reply"
        );
        assert_eq!(h.peer.notify_in(g, TestOut::Notice("after".into())), Ok(()));
        settle().await;
        assert_eq!(texts(endpoint.frames()), vec!["req|7|body7", "not|after"]);
        endpoint.push(TestWire::reply(7));
        settle().await;
        assert_eq!(
            asked.await,
            Ok(TestIn::Reply {
                tag: 7,
                body: "ok".into()
            })
        );
    }

    // The transport edition of the shell's `buffered_command_on_disconnect_is_drained_not_executed_post_reconnect` — a request that was waiting for its answer
    //   and a request that was in the queue both wake on the disconnect and are not executed on the next connection. ★Pin the branches deterministically★:
    //   if we block the write of the preceding notification and stand the supervisor on it, the second request stays in the queue (a write does not
    //   read the data queue). So the two each take one of the sent field's two branches (D12).
    #[tokio::test]
    async fn a_waiting_request_ends_as_sent_a_queued_one_as_not_sent_and_neither_runs_on_the_next_link(
    ) {
        let (h, first) = live().await;
        let g1 = h.peer.generation();
        let waiting = spawn_request(&h.peer, 1);
        settle().await;
        assert_eq!(
            texts(first.frames()),
            vec!["req|1|body1"],
            "첫 요청이 나가 답을 기다린다"
        );

        first.stall_writes(true);
        h.peer
            .notify(TestOut::Notice("stuck".into()))
            .expect("운영 중이다");
        settle().await;
        let queued = spawn_request(&h.peer, 2);
        settle().await;
        assert!(
            !h.queue_drained(),
            "감독이 막힌 쓰기 위에 서 있어 둘째 요청은 큐에 있다"
        );

        // The channel is cut — the blocked write ends in failure.
        first.fail_writes();
        first.stall_writes(false);
        settle().await;
        let waiting = joined(waiting).await;
        assert!(
            matches!(
                waiting,
                Err(RequestError::Disconnected {
                    generation,
                    sent: true,
                    ..
                }) if generation == g1
            ),
            "★답을 기다리던 요청은 보냄이다★ — 상대가 처리했을 수 있다: {waiting:?}"
        );
        let queued = joined(queued).await;
        assert!(
            matches!(
                queued,
                Err(RequestError::Disconnected {
                    generation,
                    sent: false,
                    ..
                }) if generation == g1
            ),
            "★큐에 든 채였던 요청은 안 보냄이다★ — 다시 보내도 두 번 적용되지 않는다: {queued:?}"
        );

        h.clock.advance(Duration::from_millis(500));
        settle_until("재연결", || {
            h.peer.peer_state() == PeerState::Live && h.peer.generation() != g1
        })
        .await;
        let next = h.net.accept().expect("둘째 통로");
        settle().await;
        assert_eq!(
            texts(next.frames()),
            Vec::<String>::new(),
            "끊긴 명령이 다음 연결에서 실행되면 안 된다(ADR-0195)"
        );
    }

    // The transport edition of the shell's `held_clone_enqueued_after_disconnect_is_rejected_and_never_runs` — ★it measures the supervisor's just-before-write
    //   comparison★. The machine empties the queue when it disconnects but cannot see what comes after, and the connection-establishment phase does not read the queue so that
    //   stays until the next operating phase. To create that gap we put in the queue directly — the handle entry rejects because it is not operating now, so
    //   we imitate a race that re-attached right after passing the entry (in the shell, the path of holding an endpoint clone and then putting in).
    #[tokio::test]
    async fn a_command_stamped_before_a_reconnect_is_not_written_on_the_next_link() {
        let mut h = build(Policy::default(), Arc::new(ListeningHandshake));
        settle_until("핸드셰이크 대기", || {
            h.peer.peer_state() == PeerState::Handshaking
        })
        .await;
        // ★Keep hold of the other side★ — if dropped, it is a channel that vanished silently, so it is cut as a read error (`MemoryEndpoint::close`).
        let first = h.net.accept().expect("첫 통로");
        first.push(Frame::Text("go".into()));
        settle_until("첫 연결", || h.peer.peer_state() == PeerState::Live).await;
        let g1 = h.peer.generation();

        h.peer.reconnect_now();
        settle_until("다음 핸드셰이크 대기", || {
            h.net.dials() == 2 && h.peer.peer_state() == PeerState::Handshaking
        })
        .await;
        h.drain();
        let (reply, mut woken) = oneshot::channel();
        let stale = |op| PeerCmd { generation: g1, op };
        let data = &h.peer.inner.data_tx;
        assert!(data
            .try_send(stale(PeerOp::Request {
                out: TestOut::Request {
                    tag: 5,
                    body: "stale".into(),
                },
                reply,
            }))
            .is_ok());
        assert!(data
            .try_send(stale(PeerOp::Notify {
                out: TestOut::Notice("stale".into()),
            }))
            .is_ok());
        // Fanout · resume also pass through the same comparison — put them in together so no branch slips out separately.
        assert!(data
            .try_send(stale(PeerOp::Raw(Frame::Text("raw|stale".into()))))
            .is_ok());
        assert!(data
            .try_send(stale(PeerOp::Resume {
                key: 7,
                after: Some(1),
            }))
            .is_ok());
        // A drop bound to the old connection must not cut the next one (`Peer::drop_link_in`).
        assert!(data
            .try_send(stale(PeerOp::DropLink {
                cause: DisconnectCause::ReadFailed,
            }))
            .is_ok());
        settle().await;
        assert!(
            matches!(woken.try_recv(), Err(oneshot::error::TryRecvError::Empty)),
            "★틈이 실재한다★ — 연결 수립 구간은 큐를 안 읽어 낡은 명령이 다음 운영 단계까지 남는다"
        );

        let next = h.net.accept().expect("둘째 통로");
        next.push(Frame::Text("go".into()));
        settle_until("재연결", || h.peer.peer_state() == PeerState::Live).await;
        settle().await;
        assert_ne!(h.peer.generation(), g1);
        let woken = woken.try_recv();
        assert!(
            matches!(
                woken,
                Ok(Err(RequestError::Disconnected {
                    generation,
                    sent: false,
                    ..
                })) if generation == g1
            ),
            "낡은 요청은 안 보냄으로 깨어난다: {woken:?}"
        );
        assert_eq!(
            texts(next.frames()),
            Vec::<String>::new(),
            "★옛 세대에 묶인 명령은 다음 연결에 안 쓰인다★"
        );
        assert_eq!(
            h.peer.peer_state(),
            PeerState::Live,
            "the stale drop left the next link up"
        );
        let events = h.events();
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, TransportEvent::Disconnected { .. })),
            "{events:?}"
        );
        // How many reports it splits into depends on how many the supervisor takes out per turn, so we look only at the sum.
        assert_eq!(
            outbound_dropped(&events),
            3,
            "★부르는 쪽이 `Ok` 를 받아 간 알림 · 팬아웃 · 이어받기는 버린 만큼 센다★ — 안 세면 조용한 유실이다"
        );
    }

    // `request` takes the generation at the place of the call — even if it re-attaches after the future is made and is first polled then, it does not go to the new connection.
    #[tokio::test]
    async fn a_request_future_made_before_a_reconnect_stays_bound_to_the_old_generation() {
        let (h, first) = live().await;
        let g1 = h.peer.generation();
        let peer = h.peer.clone();
        let mut asked = Box::pin(peer.request(TestOut::Request {
            tag: 1,
            body: "early".into(),
        }));
        first.close();
        settle_until("백오프", || h.peer.peer_state() == PeerState::Backoff).await;
        h.clock.advance(Duration::from_millis(500));
        settle_until("재연결", || h.peer.peer_state() == PeerState::Live).await;
        let next = h.net.accept().expect("둘째 통로");
        assert_ne!(h.peer.generation(), g1);

        let first_poll = poll_immediate(&mut asked).await;
        assert!(
            matches!(
                first_poll,
                Some(Err(RequestError::Disconnected {
                    generation,
                    sent: false,
                    ..
                })) if generation == g1
            ),
            "★처음 폴링이 다시 붙은 뒤여도 만든 때의 세대에 묶인다★: {first_poll:?}"
        );
        settle().await;
        assert_eq!(
            texts(next.frames()),
            Vec::<String>::new(),
            "만든 때의 연결이 끝났으니 다음 연결에 안 쓰인다"
        );
    }

    // A queue filled in an ended generation only reports and does not cut the next connection (`Supervisor::drain_overflow`). 「지금 다시 붙어라」 (reconnect now)
    //   dials straight away without going through the backoff wait, so the first turn of the new connection is the first to collect that overflow count.
    #[tokio::test]
    async fn a_queue_that_overflowed_on_the_old_generation_does_not_cut_the_next_link() {
        let mut h = build(
            Policy {
                outbound_queue: 2,
                ..Policy::default()
            },
            Arc::new(ImmediateHandshake),
        );
        settle_until("첫 연결", || h.peer.peer_state() == PeerState::Live).await;
        let first = h.net.accept().expect("첫 통로");
        let g1 = h.peer.generation();

        // Stand the supervisor on a blocked write and fill the queue.
        first.stall_writes(true);
        let notice = |body: &str| TestOut::Notice(body.into());
        assert_eq!(h.peer.notify(notice("n1")), Ok(()));
        settle().await;
        assert_eq!(h.peer.notify(notice("n2")), Ok(()));
        assert_eq!(h.peer.notify(notice("n3")), Ok(()));
        assert_eq!(h.peer.notify(notice("n4")), Err(SendError::QueueFull));

        h.peer.reconnect_now();
        settle().await;
        // The close that drops the channel is standing on a blocked write — release it.
        first.stall_writes(false);
        settle_until("재연결", || {
            h.peer.peer_state() == PeerState::Live && h.peer.generation() != g1
        })
        .await;
        let _next = h.net.accept().expect("둘째 통로");
        settle().await;

        let g2 = h.peer.generation();
        assert_eq!(
            h.peer.peer_state(),
            PeerState::Live,
            "★옛 세대의 넘침이 새 연결을 끊으면 안 된다★"
        );
        let events = h.events();
        assert!(
            !events.iter().any(|e| matches!(
                e,
                TransportEvent::Disconnected { generation, .. } if *generation == g2
            )),
            "새 연결은 끊기지 않았다: {:?}",
            events.iter().map(|e| e.kind()).collect::<Vec<_>>()
        );
        assert_eq!(
            outbound_dropped(&events),
            4,
            "쓰다 버린 n1 · 큐에 든 채 버린 n2 · n3 · 넘친 n4 — 신고는 한다"
        );
    }

    // The transport edition of the shell `CommandOutcome` socket comparison — if you take the received connection's generation and give the answer, it goes out only on that connection.
    #[tokio::test]
    async fn notify_in_and_request_in_go_out_only_on_the_generation_they_name() {
        let (mut h, first) = live().await;
        let g1 = h.peer.generation();
        first.close();
        settle_until("백오프", || h.peer.peer_state() == PeerState::Backoff).await;
        h.clock.advance(Duration::from_millis(500));
        settle_until("재연결", || h.peer.peer_state() == PeerState::Live).await;
        let next = h.net.accept().expect("둘째 통로");
        let g2 = h.peer.generation();
        assert_ne!(g1, g2);
        h.drain();

        assert_eq!(
            h.peer.notify_in(g1, TestOut::Notice("old".into())),
            Err(SendError::Disconnected),
            "옛 연결의 답은 입구에서 거절된다"
        );
        let old = joined(spawn_request_in(&h.peer, g1, 1)).await;
        assert!(
            matches!(
                old,
                Err(RequestError::Disconnected {
                    generation,
                    sent: false,
                    ..
                }) if generation == g1
            ),
            "{old:?}"
        );
        settle().await;
        assert_eq!(texts(next.frames()), Vec::<String>::new());
        assert!(
            !h.kinds().contains(&"Dropped"),
            "입구의 거절은 부르는 쪽이 오류로 받았으니 세지 않는다"
        );
        assert_eq!(h.peer.refused_count(), 0);

        assert_eq!(h.peer.notify_in(g2, TestOut::Notice("new".into())), Ok(()));
        let asked = spawn_request_in(&h.peer, g2, 2);
        settle().await;
        assert_eq!(texts(next.frames()), vec!["not|new", "req|2|body2"]);
        next.push(TestWire::reply(2));
        assert_eq!(
            joined(asked).await,
            Ok(TestIn::Reply {
                tag: 2,
                body: "ok".into()
            })
        );
    }

    // ── keepalive · 시한 ──
    #[tokio::test]
    async fn silence_past_the_idle_timeout_cuts_the_link() {
        let (mut h, _endpoint) = live().await;
        h.clock.advance(Duration::from_secs(50));
        settle().await;
        assert_eq!(h.disconnect_cause(), Some(DisconnectCause::Idle));
    }

    #[tokio::test]
    async fn keepalive_goes_out_on_its_own_schedule() {
        let (h, endpoint) = live().await;
        h.clock.advance(Duration::from_secs(20));
        settle().await;
        assert!(endpoint.drain().contains(&ClientMsg::Ping));
        endpoint.push(Frame::Keepalive);
        settle().await;
        h.clock.advance(Duration::from_secs(20));
        settle().await;
        assert!(endpoint.drain().contains(&ClientMsg::Ping));
        assert_eq!(h.peer.peer_state(), PeerState::Live);
    }

    #[tokio::test]
    async fn an_inbound_keepalive_keeps_the_idle_timer_from_firing() {
        let (h, endpoint) = live().await;
        for _ in 0..3 {
            h.clock.advance(Duration::from_secs(40));
            endpoint.push(Frame::Keepalive);
            settle().await;
        }
        assert_eq!(h.peer.peer_state(), PeerState::Live);
    }

    // ── C2: a frame flood does not starve the timer · the outbound queue ──
    // ── Survival under load (★this is not the C2 regression net — the five `next_turn` ones above are★) ──
    #[tokio::test]
    async fn keepalive_still_goes_out_under_load() {
        let (h, endpoint) = live().await;
        let feeder = flood(&endpoint);
        pump(8).await;
        h.clock.advance(Duration::from_secs(20));
        pump(64).await;
        feeder.abort();
        assert!(
            endpoint.drain().contains(&ClientMsg::Ping),
            "★출력이 가장 많을 때 ping 이 멎으면 상대가 우리를 침묵으로 끊는다★"
        );
        assert_eq!(h.peer.peer_state(), PeerState::Live);
    }

    #[tokio::test]
    async fn the_outbound_queue_still_drains_under_load() {
        let (h, endpoint) = live().await;
        let feeder = flood(&endpoint);
        pump(8).await;
        h.peer.notify(TestOut::Notice("let-me-out".into())).unwrap();
        pump(64).await;
        feeder.abort();
        assert!(
            endpoint
                .frames()
                .contains(&Frame::Text("not|let-me-out".into())),
            "★프레임 팔이 계속 준비돼 있어도 나가는 큐가 굶으면 안 된다★"
        );
    }

    #[tokio::test]
    async fn the_request_deadline_still_fires_under_load() {
        let (h, endpoint) = live().await;
        let pending = spawn_request(&h.peer, 31);
        settle().await;
        let feeder = flood(&endpoint);
        pump(8).await;
        h.clock.advance(Duration::from_secs(8));
        pump(64).await;
        feeder.abort();
        assert!(
            matches!(joined(pending).await, Err(RequestError::TimedOut { .. })),
            "★시한이 안 돌면 요청이 상한을 넘겨 매달린다★"
        );
    }

    // ── Stream ──
    #[tokio::test]
    async fn a_stream_gap_is_reported_and_resumed_from_the_last_good_seq() {
        let (mut h, endpoint) = live().await;
        endpoint.push(TestWire::chunk(1, 4, 1));
        settle().await;
        assert_eq!(h.messages().len(), 1);

        endpoint.push(TestWire::chunk(1, 4, 5));
        settle().await;
        let gap = h.events().iter().find_map(|e| match e {
            TransportEvent::StreamGap {
                expected,
                got,
                resumed_from,
                stream,
                ..
            } => Some((*stream, *expected, *got, *resumed_from)),
            _ => None,
        });
        assert_eq!(gap, Some((1, 2, 5, Some(1))));
        assert_eq!(
            endpoint.frames(),
            vec![Frame::Text("res|1|1".into())],
            "구멍을 만나면 그 지점부터 다시 청한다"
        );
    }

    // ── C3: a re-request's answer is not misread as a truncation ──
    #[tokio::test]
    async fn chunks_in_flight_before_the_resume_do_not_look_like_truncation() {
        let (mut h, endpoint) = live().await;
        endpoint.push(TestWire::chunk(1, 4, 1));
        endpoint.push(TestWire::chunk(1, 4, 5));
        settle().await;
        h.drain();

        // A piece the peer had already sent before receiving the re-request. ★Then put up one fence after it★ — if we just assert 「없다」 (there is none) right after
        //   `settle()`, it passes even when processing has not happened yet, and the test blesses the defect.
        endpoint.push(TestWire::chunk(1, 4, 6));
        endpoint.push(TestWire::encode_in(&TestIn::Notice("fence".into())));
        settle_until("6번 조각과 울타리가 처리될 때까지", || {
            h.seen_notice("fence")
        })
        .await;
        assert!(
            !h.kinds().contains(&"StreamTruncated"),
            "★상대가 못 준다★ 가 아니라 ★아직 안 왔다★ 다"
        );

        // The peer's replay comes up in order.
        for seq in 2..=6 {
            endpoint.push(TestWire::chunk(1, 4, seq));
        }
        settle().await;
        let seqs: Vec<u64> = h
            .messages()
            .into_iter()
            .filter_map(|m| match m {
                TestIn::Chunk { seq, .. } => Some(seq),
                _ => None,
            })
            .collect();
        assert_eq!(seqs, vec![2, 3, 4, 5, 6], "잃은 구간이 실제로 돌아온다");
    }

    #[tokio::test]
    async fn a_resume_window_closes_without_any_further_arrival() {
        let (mut h, endpoint) = live().await;
        endpoint.push(TestWire::chunk(1, 4, 1));
        endpoint.push(TestWire::chunk(1, 4, 5));
        settle().await;
        h.drain();

        // ★The window must close even if no more pieces come★ — the turn ending and the stream going quiet is the ordinary case,
        //   and a decision that hangs only on arrival stays open forever then.
        h.clock.advance(Duration::from_secs(2));
        settle().await;
        let items = h.drain();
        let oldest = items.iter().find_map(|i| match i.as_event() {
            Some(TransportEvent::StreamTruncated { oldest_valid, .. }) => Some(*oldest_valid),
            _ => None,
        });
        assert_eq!(oldest, Some(5), "★붙들고 있던 첫 번호가 하한이다★");
        let seqs: Vec<u64> = items
            .iter()
            .filter_map(|i| match i.as_message().map(|a| &a.msg) {
                Some(TestIn::Chunk { seq, .. }) => Some(*seq),
                _ => None,
            })
            .collect();
        assert_eq!(
            seqs,
            vec![5],
            "★신고한 하한부터 실제로 배달해야 그 값이 참이다★"
        );
    }

    #[tokio::test]
    async fn an_outstanding_resume_is_sent_again_after_a_reconnect() {
        let (mut h, endpoint) = live().await;
        endpoint.push(TestWire::chunk(1, 4, 1));
        endpoint.push(TestWire::chunk(1, 4, 5));
        settle().await;
        assert_eq!(endpoint.frames(), vec![Frame::Text("res|1|1".into())]);
        h.drain();

        endpoint.close();
        settle().await;
        h.clock.advance(Duration::from_millis(500));
        settle_until("재연결", || h.peer.peer_state() == PeerState::Live).await;

        let reconnected = h.net.accept().expect("새 통로");
        assert_eq!(
            reconnected.frames(),
            vec![Frame::Text("res|1|1".into())],
            "★재연결 뒤 다시 청하지 않으면 그 구멍은 영영 안 메워진다★"
        );
    }

    // ── B3: eviction does not swallow a pending re-request ──
    #[tokio::test]
    async fn forgetting_a_stream_is_reported_and_gives_back_what_it_held() {
        let mut h = build(
            Policy {
                max_streams: 1,
                ..Policy::default()
            },
            Arc::new(ImmediateHandshake),
        );
        settle_until("연결", || h.peer.peer_state() == PeerState::Live).await;
        let endpoint = h.net.accept().unwrap();

        endpoint.push(TestWire::chunk(1, 4, 1));
        endpoint.push(TestWire::chunk(1, 4, 5));
        settle().await;
        h.drain();

        // There is only one slot, so a new stream pushes out the old stream.
        endpoint.push(TestWire::chunk(2, 4, 1));
        settle().await;
        let items = h.drain();
        let kinds: Vec<&str> = items
            .iter()
            .filter_map(|i| i.as_event().map(|e| e.kind()))
            .collect();
        assert!(
            kinds.contains(&"StreamForgotten"),
            "★무엇을 잊었는지 말하지 않으면 그 스트림의 장부가 조용히 처음부터 다시 시작된다★"
        );
        assert!(
            kinds.contains(&"StreamTruncated"),
            "붙들고 있던 조각이 축출과 함께 사라지면 안 된다"
        );
        let seqs: Vec<u64> = items
            .iter()
            .filter_map(|i| match i.as_message().map(|a| &a.msg) {
                Some(TestIn::Chunk { key: 1, seq, .. }) => Some(*seq),
                _ => None,
            })
            .collect();
        assert_eq!(seqs, vec![5], "붙들고 있던 5 를 돌려받는다");
    }

    #[tokio::test]
    async fn a_duplicate_chunk_never_reaches_the_consumer() {
        let (mut h, endpoint) = live().await;
        endpoint.push(TestWire::chunk(1, 4, 1));
        endpoint.push(TestWire::chunk(1, 4, 2));
        endpoint.push(TestWire::chunk(1, 4, 2));
        endpoint.push(TestWire::chunk(1, 4, 1));
        settle().await;
        assert_eq!(h.messages().len(), 2);
    }

    #[tokio::test]
    async fn a_generation_change_is_announced_and_restarts_the_ledger() {
        let (mut h, endpoint) = live().await;
        endpoint.push(TestWire::chunk(1, 4, 100));
        settle().await;
        h.drain();
        endpoint.push(TestWire::chunk(1, 5, 1));
        settle().await;
        assert!(h.kinds().contains(&"StreamGenerationChanged"));
    }

    #[tokio::test]
    async fn a_message_claiming_both_roles_is_reported_and_treated_as_a_reply() {
        let (mut h, endpoint) = live().await;
        let waiting = spawn_request(&h.peer, 8);
        settle().await;
        endpoint.push(TestWire::encode_in(&TestIn::Ambiguous {
            tag: 8,
            key: 1,
            generation: 1,
            seq: 1,
        }));
        settle().await;
        assert!(h.kinds().contains(&"AmbiguousRole"));
        assert!(joined(waiting).await.is_ok(), "답장으로 먼저 처리한다");
    }

    #[tokio::test]
    async fn resume_stream_writes_the_consumers_own_envelope() {
        let (h, endpoint) = live().await;
        let generation = h.peer.resume_stream(7, Some(42));
        settle().await;
        assert_eq!(generation, Generation(1));
        assert_eq!(endpoint.frames(), vec![Frame::Text("res|7|42".into())]);
    }

    // ── M2: a resume while not in the operating phase does not vanish silently ──
    #[tokio::test]
    async fn a_resume_asked_while_down_is_counted_and_reported() {
        let (mut h, endpoint) = live().await;
        endpoint.close();
        settle().await;
        assert_ne!(h.peer.peer_state(), PeerState::Live);
        h.drain();

        h.peer.resume_stream(7, Some(1));
        h.clock.advance(Duration::from_millis(500));
        settle_until("재연결", || h.peer.peer_state() == PeerState::Live).await;

        let dropped = h.events().iter().find_map(|e| match e {
            TransportEvent::Dropped {
                direction, count, ..
            } => Some((*direction, *count)),
            _ => None,
        });
        assert!(matches!(dropped, Some((Direction::Outbound, n)) if n >= 1));
    }

    // ── Decode ──
    #[tokio::test]
    async fn a_decode_failure_reports_only_the_wire_class_never_the_value_it_quotes() {
        // ★The decoder error quotes the received value★ — if that text rode on an event, the peer would plant text on the screen · the LLM surface (TRD 3 §5 19).
        let (mut h, endpoint) = live().await;
        endpoint.push(Frame::Text("rep|PEER-SAYS-\u{1b}[31mHI|x".into()));
        settle().await;
        let reason = h.events().into_iter().find_map(|e| match e {
            TransportEvent::DecodeFailed { reason, .. } => Some(reason),
            _ => None,
        });
        assert_eq!(reason, Some("bad number"), "분류 낱말만 실린다");
    }

    #[tokio::test]
    async fn a_frame_the_wire_cannot_read_is_reported_without_cutting_the_link() {
        let (mut h, endpoint) = live().await;
        endpoint.push(Frame::Text("garbage".into()));
        settle().await;
        assert!(h.kinds().contains(&"DecodeFailed"));
        assert_eq!(h.peer.peer_state(), PeerState::Live);
    }

    #[tokio::test]
    async fn the_disconnect_on_decode_failure_policy_cuts_the_link() {
        let mut h = build(
            Policy {
                decode_failure: DecodeFailurePolicy::Disconnect,
                ..Policy::default()
            },
            Arc::new(ImmediateHandshake),
        );
        settle_until("연결", || h.peer.peer_state() == PeerState::Live).await;
        let endpoint = h.net.accept().unwrap();
        h.drain();
        endpoint.push(Frame::Text("garbage".into()));
        settle().await;
        assert_eq!(
            h.disconnect_cause(),
            Some(DisconnectCause::ReadFailed),
            "읽은 것을 못 읽어 끊었다 — 읽는 쪽의 끝이다"
        );
    }

    // ── Reconnect budget ──
    #[tokio::test]
    async fn being_unreachable_burns_the_budget_and_then_gives_up() {
        let mut h = build(Policy::default(), Arc::new(ImmediateHandshake));
        h.net.fail_forever();
        settle_until("첫 실패", || h.peer.peer_state() == PeerState::Backoff).await;

        for delay in [500u64, 1000, 2000] {
            h.clock.advance(Duration::from_millis(delay));
            settle().await;
        }
        assert_eq!(
            h.peer.peer_state(),
            PeerState::GaveUp(crate::machine::GaveUpReason::BudgetExhausted)
        );
        assert_eq!(h.net.dials(), 4, "첫 시도 + 재시도 3회");
        let kinds = h.kinds();
        assert_eq!(kinds.iter().filter(|k| **k == "ConnectFailed").count(), 4);
        assert!(kinds.contains(&"GaveUp"));
    }

    #[tokio::test]
    async fn a_rejection_spends_no_budget_and_dials_only_once() {
        let mut h = build(Policy::default(), RejectingHandshake::new("version 3 != 4"));
        settle_until("거절", || h.peer.peer_state().is_terminal()).await;
        assert_eq!(
            h.peer.peer_state(),
            PeerState::GaveUp(crate::machine::GaveUpReason::Rejected)
        );
        assert_eq!(h.net.dials(), 1, "붙었는데 거절 = 재시도하지 않는다");
        let events = h.events();
        let reason = events.iter().find_map(|e| match e {
            TransportEvent::Rejected { reason, .. } => Some(reason.clone()),
            _ => None,
        });
        assert_eq!(reason.as_deref(), Some("version 3 != 4"));
        assert!(events.iter().any(|e| e.kind() == "GaveUp"));
        assert!(
            !events.iter().any(|e| e.kind() == "ConnectFailed"),
            "예산을 쓴 적이 없으므로 시도 실패 사건도 없다"
        );
    }

    // ── N2: if the peer closes carrying a code, that is a reject ──
    #[tokio::test]
    async fn a_peer_that_closes_with_a_code_is_a_rejection_not_a_retry() {
        let mut h = build(Policy::default(), Arc::new(ListeningHandshake));
        settle_until("핸드셰이크 대기", || {
            h.peer.peer_state() == PeerState::Handshaking
        })
        .await;
        let endpoint = h.net.accept().unwrap();
        endpoint.reject(Close::new(CloseCode::HANDSHAKE_REJECTED, "protocol 3 != 4"));
        settle_until("거절 판정", || h.peer.peer_state().is_terminal()).await;

        assert_eq!(
            h.peer.peer_state(),
            PeerState::GaveUp(crate::machine::GaveUpReason::Rejected),
            "★코드를 읽지 못하면 여기가 BudgetExhausted 가 된다★"
        );
        assert_eq!(h.net.dials(), 1, "거절은 예산을 안 쓴다");
        let reported = h.events().into_iter().find_map(|e| match e {
            TransportEvent::Rejected { code, reason, .. } => Some((code, reason)),
            _ => None,
        });
        assert_eq!(
            reported,
            Some((CloseCode::HANDSHAKE_REJECTED, "protocol 3 != 4".into()))
        );
    }

    #[tokio::test]
    async fn a_peer_that_closes_without_a_code_is_treated_as_unreachable() {
        let mut h = build(Policy::default(), Arc::new(ListeningHandshake));
        settle_until("핸드셰이크 대기", || {
            h.peer.peer_state() == PeerState::Handshaking
        })
        .await;
        h.net.accept().unwrap().close();
        settle_until("백오프", || h.peer.peer_state() == PeerState::Backoff).await;
        assert!(h.kinds().contains(&"ConnectFailed"), "예산을 쓰는 쪽이다");
    }

    // ── Reject kinds (TRD 3 §3-0 ⑤ · D20 ①) — the kind decides the budget, and the close code carries the kind ──
    fn connect_failure(h: &mut Harness) -> Option<ConnectFailure> {
        h.events().into_iter().find_map(|e| match e {
            TransportEvent::ConnectFailed { cause, .. } => Some(cause),
            _ => None,
        })
    }

    fn sent_close(endpoint: &MemoryEndpoint) -> Option<CloseCode> {
        endpoint.drain().into_iter().find_map(|msg| match msg {
            ClientMsg::Close(close) => Some(close.code),
            _ => None,
        })
    }

    #[tokio::test]
    async fn each_reject_kind_of_our_handshake_closes_with_its_own_code() {
        for (handshake, expected) in [
            (
                RejectingHandshake::new("version 3 != 4"),
                CloseCode::HANDSHAKE_REJECTED,
            ),
            (
                RejectingHandshake::retryable("stale token"),
                CloseCode::HANDSHAKE_RETRYABLE,
            ),
        ] {
            let h = build(Policy::default(), handshake);
            settle_until("첫 시도의 끝", || {
                matches!(
                    h.peer.peer_state(),
                    PeerState::Backoff | PeerState::GaveUp(_)
                )
            })
            .await;
            let endpoint = h.net.accept().expect("통로가 열렸어야 한다");
            assert_eq!(sent_close(&endpoint), Some(expected), "{expected:?}");
        }
    }

    #[tokio::test]
    async fn a_retryable_rejection_by_our_handshake_spends_budget_and_dials_again() {
        let mut h = build(
            Policy::default(),
            RejectingHandshake::retryable("stale token"),
        );
        settle_until("백오프", || h.peer.peer_state() == PeerState::Backoff).await;
        let events = h.events();
        assert!(
            !events.iter().any(|e| e.kind() == "Rejected"),
            "다시 거는 거절은 종결 거절 사건을 안 낸다"
        );
        let cause = events.into_iter().find_map(|e| match e {
            TransportEvent::ConnectFailed { cause, .. } => Some(cause),
            _ => None,
        });
        assert_eq!(
            cause,
            Some(ConnectFailure::Rejected {
                kind: RejectKind::Retryable,
                code: CloseCode::HANDSHAKE_RETRYABLE,
                reason: "stale token".into(),
            })
        );
        for (delay, dials) in [(500u64, 2usize), (1000, 3), (2000, 4)] {
            settle_until("백오프", || h.peer.peer_state() == PeerState::Backoff).await;
            h.clock.advance(Duration::from_millis(delay));
            settle_until("다시 걸기", || h.net.dials() == dials).await;
        }
        settle_until("포기", || h.peer.peer_state().is_terminal()).await;
        assert_eq!(
            h.peer.peer_state(),
            PeerState::GaveUp(crate::machine::GaveUpReason::BudgetExhausted),
            "다시 거는 거절은 예산이 바닥나야 포기한다"
        );
        assert_eq!(h.net.dials(), 4, "첫 시도 + 재시도 3회");
    }

    #[tokio::test]
    async fn a_close_code_other_than_the_terminal_one_is_a_retryable_rejection() {
        // ★A reject that came with only a code★ — only 4001 is terminal, and a reconnect code · an unknown code uses budget and dials again.
        for code in [
            CloseCode::HANDSHAKE_RETRYABLE,
            CloseCode(1008),
            CloseCode(CloseCode::CONSUMER_BASE + 7),
        ] {
            let mut h = build(Policy::default(), Arc::new(ListeningHandshake));
            settle_until("핸드셰이크 대기", || {
                h.peer.peer_state() == PeerState::Handshaking
            })
            .await;
            h.net
                .accept()
                .unwrap()
                .reject(Close::new(code, "try again"));
            settle_until("백오프", || h.peer.peer_state() == PeerState::Backoff).await;
            assert_eq!(
                connect_failure(&mut h),
                Some(ConnectFailure::Rejected {
                    kind: RejectKind::Retryable,
                    code,
                    reason: "try again".into(),
                }),
                "{code:?}"
            );
            h.clock.advance(Duration::from_millis(500));
            settle_until("다시 걸기", || h.net.dials() == 2).await;
        }
    }

    #[tokio::test]
    async fn a_graceful_close_during_the_handshake_is_could_not_connect() {
        let mut h = build(Policy::default(), Arc::new(ListeningHandshake));
        settle_until("핸드셰이크 대기", || {
            h.peer.peer_state() == PeerState::Handshaking
        })
        .await;
        h.net
            .accept()
            .unwrap()
            .reject(Close::new(CloseCode(1000), "bye"));
        settle_until("백오프", || h.peer.peer_state() == PeerState::Backoff).await;
        assert_eq!(
            connect_failure(&mut h),
            Some(ConnectFailure::Unreachable(LinkError::new(
                "peer closed during handshake"
            )))
        );
    }

    #[tokio::test]
    async fn a_handshake_that_waits_out_an_error_frame_lets_the_close_code_decide() {
        /// If it receives the peer's error frame, it does not reject and waits for the close (TRD 3 §3-0 ⑤).
        struct WaitsOutErrors;
        impl Handshake for WaitsOutErrors {
            fn start(&self) -> HandshakeStep {
                HandshakeStep::Send(Frame::Text(HelloHandshake::HELLO.into()))
            }
            fn on_frame(&self, frame: &Frame) -> HandshakeStep {
                match frame {
                    Frame::Text(t) if t == HelloHandshake::WELCOME => HandshakeStep::Done,
                    _ => HandshakeStep::Await,
                }
            }
        }
        for (code, kind) in [
            (CloseCode::HANDSHAKE_RETRYABLE, RejectKind::Retryable),
            (CloseCode::HANDSHAKE_REJECTED, RejectKind::Terminal),
        ] {
            let mut h = build(Policy::default(), Arc::new(WaitsOutErrors));
            settle_until("핸드셰이크 대기", || {
                h.peer.peer_state() == PeerState::Handshaking
            })
            .await;
            let endpoint = h.net.accept().unwrap();
            endpoint.push(Frame::Text("error: auth failed".into()));
            settle().await;
            assert_eq!(
                h.peer.peer_state(),
                PeerState::Handshaking,
                "오류 프레임 하나로 판정하지 않는다"
            );
            endpoint.reject(Close::new(code, "auth failed"));
            settle_until("판정", || {
                matches!(
                    h.peer.peer_state(),
                    PeerState::Backoff | PeerState::GaveUp(_)
                )
            })
            .await;
            let expected = match kind {
                RejectKind::Terminal => PeerState::GaveUp(crate::machine::GaveUpReason::Rejected),
                RejectKind::Retryable => PeerState::Backoff,
            };
            assert_eq!(h.peer.peer_state(), expected, "{code:?}");
            h.drain();
        }
    }

    #[tokio::test]
    async fn a_dialer_refusal_gives_up_as_rejected_without_budget_or_a_second_dial() {
        let mut h = build(Policy::default(), Arc::new(ImmediateHandshake));
        h.net.refuse(Close::new(
            CloseCode::HANDSHAKE_REJECTED,
            "daemon 0.4.0 != 0.4.1",
        ));
        settle_until("포기", || h.peer.peer_state().is_terminal()).await;
        assert_eq!(
            h.peer.peer_state(),
            PeerState::GaveUp(crate::machine::GaveUpReason::Rejected)
        );
        let events = h.events();
        let rejected = events.iter().find_map(|e| match e {
            TransportEvent::Rejected { code, reason, .. } => Some((*code, reason.clone())),
            _ => None,
        });
        assert_eq!(
            rejected,
            Some((
                CloseCode::HANDSHAKE_REJECTED,
                "daemon 0.4.0 != 0.4.1".into()
            ))
        );
        assert!(
            !events.iter().any(|e| e.kind() == "ConnectFailed"),
            "예산을 쓴 적이 없으므로 시도 실패 사건도 없다"
        );
        h.clock.advance(Duration::from_secs(60));
        settle().await;
        assert_eq!(h.net.dials(), 1, "다시 걸지 않는다");
    }

    #[tokio::test]
    async fn a_rejection_reason_from_the_peer_is_masked_before_it_surfaces() {
        let noisy = format!(
            "evil\u{1b}[31m\nline\u{202e}{}",
            "가".repeat(crate::mask::REASON_MAX_BYTES)
        );
        for code in [
            CloseCode::HANDSHAKE_REJECTED,
            CloseCode::HANDSHAKE_RETRYABLE,
        ] {
            let mut h = build(Policy::default(), Arc::new(ListeningHandshake));
            settle_until("핸드셰이크 대기", || {
                h.peer.peer_state() == PeerState::Handshaking
            })
            .await;
            h.net
                .accept()
                .unwrap()
                .reject(Close::new(code, noisy.clone()));
            settle_until("판정", || {
                matches!(
                    h.peer.peer_state(),
                    PeerState::Backoff | PeerState::GaveUp(_)
                )
            })
            .await;
            let reason = h
                .events()
                .into_iter()
                .find_map(|e| match e {
                    TransportEvent::Rejected { reason, .. } => Some(reason),
                    TransportEvent::ConnectFailed {
                        cause: ConnectFailure::Rejected { reason, .. },
                        ..
                    } => Some(reason),
                    _ => None,
                })
                .expect("거절 사건");
            assert_eq!(reason, crate::mask_reason(&noisy), "{code:?}");
            assert!(reason.starts_with("evil[31mline"), "{code:?}: {reason}");
            assert!(reason.len() <= crate::mask::REASON_MAX_BYTES);
        }
    }

    #[tokio::test]
    async fn reconnect_now_refills_the_budget_and_dials_again() {
        let mut h = build(Policy::default(), Arc::new(ImmediateHandshake));
        h.net.fail_forever();
        settle().await;
        for delay in [500u64, 1000, 2000] {
            h.clock.advance(Duration::from_millis(delay));
            settle().await;
        }
        assert!(h.peer.peer_state().is_terminal());
        h.drain();

        h.net.allow();
        h.peer.reconnect_now();
        settle_until("재연결", || h.peer.peer_state() == PeerState::Live).await;
        assert!(h.kinds().contains(&"Connected"));
    }

    #[tokio::test]
    async fn a_burst_of_reconnect_requests_still_lets_a_dial_happen() {
        let (h, _endpoint) = live().await;
        for _ in 0..50 {
            h.peer.reconnect_now();
        }
        settle_until(
            "쌓인 신호를 접지 않으면 dial 이 폴링되기도 전에 매번 취소된다",
            || h.net.dials() >= 2,
        )
        .await;
        settle_until("재연결", || h.peer.peer_state() == PeerState::Live).await;
    }

    #[tokio::test]
    async fn a_lost_link_comes_back_on_the_first_backoff_step() {
        let (h, endpoint) = live().await;
        endpoint.close();
        settle_until("백오프", || h.peer.peer_state() == PeerState::Backoff).await;
        h.clock.advance(Duration::from_millis(500));
        settle_until("재연결", || h.peer.peer_state() == PeerState::Live).await;
        assert_eq!(h.peer.generation(), Generation(2), "세대가 하나 올랐다");
    }

    // ── drop_link_in ──
    #[tokio::test]
    async fn drop_link_in_ignores_a_connection_that_is_not_the_live_one() {
        let (mut h, first) = live().await;
        let g1 = h.peer.generation();
        first.close();
        settle_until("backoff", || h.peer.peer_state() == PeerState::Backoff).await;
        h.peer.drop_link_in(g1, DisconnectCause::ReadFailed);
        settle().await;
        assert_eq!(
            h.peer.peer_state(),
            PeerState::Backoff,
            "not live — nothing to drop"
        );
        h.clock.advance(Duration::from_millis(500));
        settle_until("reconnected", || h.peer.peer_state() == PeerState::Live).await;
        let _next = h.net.accept().expect("the second link");
        let g2 = h.peer.generation();
        assert_ne!(g1, g2);
        h.drain();

        h.peer.drop_link_in(g1, DisconnectCause::ReadFailed);
        settle().await;
        assert_eq!(h.peer.peer_state(), PeerState::Live);
        assert_eq!(h.peer.generation(), g2, "the newer connection stays");
        assert_eq!(
            h.kinds(),
            Vec::<&str>::new(),
            "no Disconnected, nothing counted"
        );
        assert_eq!(h.net.dials(), 2);
    }

    #[tokio::test]
    async fn drop_link_in_loses_the_live_link_and_waits_the_backoff_before_the_next_dial() {
        let (mut h, _endpoint) = live().await;
        let g1 = h.peer.generation();
        h.drain();
        h.peer.drop_link_in(g1, DisconnectCause::ReadFailed);
        settle_until("backoff", || h.peer.peer_state() == PeerState::Backoff).await;
        assert_eq!(h.disconnect_cause(), Some(DisconnectCause::ReadFailed));
        h.clock.advance(Duration::from_millis(499));
        settle().await;
        assert_eq!(h.net.dials(), 1, "no dial before the base delay");
        h.clock.advance(Duration::from_millis(1));
        settle_until("reconnected", || h.peer.peer_state() == PeerState::Live).await;
        assert_eq!(h.net.dials(), 2);
    }

    #[tokio::test]
    async fn repeated_drops_after_short_connects_keep_waiting_the_base_delay_without_jitter() {
        // `build` turns jitter off (`no_jitter`) — with jitter the wait can fall below the base.
        let mut policy = Policy::default();
        policy.reconnect.max_attempts = u32::MAX;
        let h = build(policy, Arc::new(ImmediateHandshake));
        settle_until("first connect", || h.peer.peer_state() == PeerState::Live).await;
        for round in 1..=5 {
            h.peer
                .drop_link_in(h.peer.generation(), DisconnectCause::ReadFailed);
            settle_until("backoff", || h.peer.peer_state() == PeerState::Backoff).await;
            h.clock.advance(Duration::from_millis(499));
            settle().await;
            assert_eq!(
                h.net.dials(),
                round,
                "round {round}: no dial before the base delay"
            );
            h.clock.advance(Duration::from_millis(1));
            settle_until("reconnected", || h.peer.peer_state() == PeerState::Live).await;
        }
    }

    // ── N8: a peer that is cut as soon as it attaches also reaches give-up in the end (popup eligibility) ──
    #[tokio::test]
    async fn a_flapping_peer_reaches_gave_up_so_the_popup_can_fire() {
        let h = build(Policy::default(), Arc::new(ImmediateHandshake));
        let mut backoffs = [500u64, 1000, 2000].into_iter();
        for _ in 0..4 {
            settle().await;
            if h.peer.peer_state().is_terminal() {
                break;
            }
            if let Some(endpoint) = h.net.accept() {
                // Cuts as soon as it attaches — it never fills the dwell even once.
                endpoint.close();
            }
            settle().await;
            if let Some(delay) = backoffs.next() {
                h.clock.advance(Duration::from_millis(delay));
            }
        }
        settle().await;
        assert_eq!(
            h.peer.peer_state(),
            PeerState::GaveUp(crate::machine::GaveUpReason::BudgetExhausted),
            "★여기 못 닿으면 ADR-0180 결정 7 의 팝업이 영영 안 뜬다★"
        );
    }

    #[tokio::test]
    async fn a_connection_that_holds_long_enough_gets_its_budget_back() {
        let (h, endpoint) = live().await;
        h.clock.advance(Policy::default().live_dwell);
        settle().await;
        endpoint.push(Frame::Keepalive);
        settle().await;
        endpoint.close();
        settle_until("백오프", || h.peer.peer_state() == PeerState::Backoff).await;
        h.clock.advance(Duration::from_millis(500));
        settle_until("재연결", || h.peer.peer_state() == PeerState::Live).await;
    }

    // ── 큐 ──
    #[tokio::test]
    async fn requests_made_while_down_fail_without_waiting() {
        let (h, endpoint) = live().await;
        endpoint.close();
        settle().await;
        let out = h
            .peer
            .request(TestOut::Request {
                tag: 1,
                body: "x".into(),
            })
            .await;
        assert!(matches!(out, Err(RequestError::Disconnected { .. })));
        assert_eq!(
            h.peer.notify(TestOut::Notice("x".into())),
            Err(SendError::Disconnected)
        );
    }

    #[tokio::test]
    async fn a_full_outbound_queue_cuts_the_link_under_the_default_policy() {
        let mut h = build(
            Policy {
                outbound_queue: 1,
                ..Policy::default()
            },
            Arc::new(ImmediateHandshake),
        );
        settle_until("연결", || h.peer.peer_state() == PeerState::Live).await;
        let endpoint = h.net.accept().unwrap();
        h.drain();
        endpoint.stall_writes(true);

        let mut refused = 0;
        for n in 0..8 {
            if h.peer.notify(TestOut::Notice(format!("n{n}"))).is_err() {
                refused += 1;
            }
            settle().await;
        }
        assert!(refused > 0, "유계 큐가 실제로 찼어야 한다");
        assert!(
            endpoint.frames().is_empty(),
            "막힌 동안에는 한 프레임도 안 나간다"
        );

        endpoint.stall_writes(false);
        settle().await;
        assert_eq!(h.disconnect_cause(), Some(DisconnectCause::QueueFull));
    }

    #[tokio::test]
    async fn the_drop_and_report_policy_keeps_the_link_and_counts_the_loss() {
        let mut h = build(
            Policy {
                outbound_queue: 1,
                full_queue: QueueFullPolicy::DropAndReport,
                ..Policy::default()
            },
            Arc::new(ImmediateHandshake),
        );
        settle_until("연결", || h.peer.peer_state() == PeerState::Live).await;
        let endpoint = h.net.accept().unwrap();
        h.drain();
        endpoint.stall_writes(true);
        for n in 0..8 {
            let _ = h.peer.notify(TestOut::Notice(format!("n{n}")));
            settle().await;
        }
        assert!(endpoint.frames().is_empty());
        endpoint.stall_writes(false);
        settle().await;
        let dropped = h.events().iter().find_map(|e| match e {
            TransportEvent::Dropped {
                direction, count, ..
            } => Some((*direction, *count)),
            _ => None,
        });
        assert!(matches!(dropped, Some((Direction::Outbound, n)) if n > 0));
        assert_eq!(h.peer.peer_state(), PeerState::Live);
    }

    // ── N6a: what was collected and discarded from the queue is also counted ──
    #[tokio::test]
    async fn commands_discarded_from_the_queue_are_reported_not_dropped_silently() {
        let (mut h, endpoint) = live().await;
        endpoint.stall_writes(true);
        h.peer.notify(TestOut::Notice("first".into())).unwrap();
        settle().await;
        assert!(endpoint.frames().is_empty(), "감독이 첫 쓰기에서 멈췄다");

        // These are places where things are dropped while still in the queue — the handle already took an `Ok`.
        for n in 0..3 {
            h.peer.notify(TestOut::Notice(format!("n{n}"))).unwrap();
        }
        h.drain();

        h.clock.advance(Duration::from_secs(5));
        settle().await;
        // ★On a blocked channel, the close of `drop_link` is stuck too★ — if we do not release it, the backoff wait does not
        //   start and the reconnect below does not come (that calculation in the `stall_writes` rustdoc).
        endpoint.stall_writes(false);
        settle().await;
        h.clock.advance(Duration::from_millis(500));
        settle_until("재연결", || h.peer.peer_state() == PeerState::Live).await;

        let dropped = h.events().iter().find_map(|e| match e {
            TransportEvent::Dropped {
                direction, count, ..
            } => Some((*direction, *count)),
            _ => None,
        });
        // 4 = the first one cut mid-write 1 + 3 left in the queue.
        assert!(
            matches!(dropped, Some((Direction::Outbound, 4))),
            "핸들이 이미 Ok 를 받아 갔으므로 여기서 안 세면 조용한 유실이다: {dropped:?}"
        );
    }

    // ── N4: if the place to send upward closes, it folds instead of re-attaching ──
    #[tokio::test]
    async fn a_closed_consumer_ends_the_peer_instead_of_triggering_reconnects() {
        let mut h = build(Policy::default(), Arc::new(ImmediateHandshake));
        settle_until("연결", || h.peer.peer_state() == PeerState::Live).await;
        let endpoint = h.net.accept().unwrap();
        let dials_before = h.net.dials();

        // The consumer disappears.
        h.inbound.close();
        while h.inbound.try_recv().is_ok() {}
        drop(std::mem::replace(&mut h.inbound, mpsc::channel(1).1));

        endpoint.push(TestWire::encode_in(&TestIn::Notice("x".into())));
        settle_until("종료", || h.peer.peer_state() == PeerState::Closed).await;
        assert_eq!(
            h.net.dials(),
            dials_before,
            "소비자 부재는 재연결할 일이 아니다"
        );
    }

    // ── F6: a graceful shutdown is not a reject ──
    #[tokio::test]
    async fn a_peer_that_shuts_down_gracefully_is_not_a_rejection() {
        for code in [CloseCode(1000), CloseCode(1001), CloseCode::GOING_AWAY] {
            let mut h = build(Policy::default(), Arc::new(ListeningHandshake));
            settle_until("핸드셰이크 대기", || {
                h.peer.peer_state() == PeerState::Handshaking
            })
            .await;
            h.net
                .accept()
                .unwrap()
                .reject(Close::new(code, "shutting down"));
            settle_until("판정", || h.peer.peer_state() != PeerState::Handshaking).await;
            assert_eq!(
                h.peer.peer_state(),
                PeerState::Backoff,
                "★{code:?} 는 「나 이제 간다」이지 「너를 안 받는다」가 아니다 —                  거절로 읽으면 데몬이 곱게 종료할 때마다 재시도 없이 포기한다★"
            );
            assert!(h.kinds().contains(&"ConnectFailed"), "예산을 쓰는 쪽이다");
        }
    }

    // ── F5: garbage is not proof of liveness ──
    #[tokio::test]
    async fn a_peer_sending_only_garbage_is_eventually_cut_by_the_idle_timer() {
        let (mut h, endpoint) = live().await;
        h.clock.advance(Duration::from_secs(25));
        for n in 0..4 {
            endpoint.push(Frame::Text(format!("garbage{n}")));
        }
        settle().await;
        assert!(h.kinds().contains(&"DecodeFailed"));

        // idle_timeout (50s) has passed since the time of connection. If garbage had rewound the clock, it would not be cut.
        h.clock.advance(Duration::from_secs(26));
        settle().await;
        assert_eq!(
            h.disconnect_cause(),
            Some(DisconnectCause::Idle),
            "★못 읽을 것만 보내는 상대를 끊는 밸브가 이것뿐이다★"
        );
    }

    // ── F4: it reports losses even while down ──
    #[tokio::test]
    async fn losses_are_reported_while_the_peer_is_still_down() {
        let (mut h, endpoint) = live().await;
        endpoint.stall_writes(true);
        h.peer.notify(TestOut::Notice("first".into())).unwrap();
        settle().await;
        for n in 0..3 {
            h.peer.notify(TestOut::Notice(format!("n{n}"))).unwrap();
        }
        h.drain();

        h.clock.advance(Duration::from_secs(5));
        settle().await;
        // ★On a blocked channel, the close of `drop_link` is stuck too★ — the loss report comes out at the next stage (the backoff wait),
        //   so if we do not release it, the assertion below meets it before the report has come out. The reason we do not cut with the clock is
        //   that then even the backoff would pass and 「아직 안 붙었다」 (not yet attached) would break.
        endpoint.stall_writes(false);
        settle().await;
        assert_eq!(
            h.peer.peer_state(),
            PeerState::Backoff,
            "아직 다시 붙지 않은 상태여야 이 테스트가 뜻을 갖는다"
        );
        let dropped = h.events().iter().find_map(|e| match e {
            TransportEvent::Dropped {
                direction, count, ..
            } => Some((*direction, *count)),
            _ => None,
        });
        // 4 = the first one cut mid-write 1 + 3 left in the queue.
        assert!(
            matches!(dropped, Some((Direction::Outbound, 4))),
            "★다시 붙을 때까지 기다리면 사용자가 「다시 연결」을 누를 때까지 아무 데도 안 나온다★: {dropped:?}"
        );
    }

    // ── 3-0 U1d: the queue-saturation count also wakes a sleeping supervisor ──
    //
    // The end shape of a race where the handle put in after seeing a stale `Live` and the queue filled, and that count arrived after the supervisor had emptied the queue and fallen asleep in the give-up wait
    //   — the queue is empty and only the count remains. If it does not wake, the report does not come out until the next control.
    #[tokio::test]
    async fn a_queue_full_count_wakes_a_parked_supervisor() {
        let mut h = build(Policy::default(), RejectingHandshake::new("nope"));
        settle_until("포기", || {
            matches!(h.peer.peer_state(), PeerState::GaveUp(_))
        })
        .await;
        h.drain();

        h.peer.count_overflow(h.peer.generation());
        settle().await;
        assert_eq!(
            outbound_dropped(&h.events()),
            1,
            "★잠든 감독이 스스로 깨어 큐 포화 셈을 신고한다★"
        );
    }

    // ── Reply-less frames cut mid-write are counted too ──
    //
    // ★The handle already took an `Ok` when it queued★ — if notification · fanout frames whose write did not end in `Ok` are not counted,
    //   it is a silent loss as is (that rule of `fail_pending`). We measure for each cutting reason (the two controls · channel error · write deadline),
    //   and also measure together the two that must not be counted (request · keepalive).

    /// The sum of outbound loss reports — summed even if the reports come up split across several times.
    fn outbound_dropped(events: &[TransportEvent<TestWire>]) -> u64 {
        events
            .iter()
            .map(|e| match e {
                TransportEvent::Dropped {
                    direction: Direction::Outbound,
                    count,
                    ..
                } => *count,
                _ => 0,
            })
            .sum()
    }

    type Push = fn(&Peer<TestWire>) -> Result<(), SendError>;

    /// The two entry points for reply-less frames — notification and fanout.
    fn reply_less_pushes() -> [(&'static str, Push); 2] {
        [
            ("알림", |p| p.notify(TestOut::Notice("in flight".into()))),
            ("팬아웃", |p| {
                p.send_frame(Frame::Text("in flight".into()))
            }),
        ]
    }

    /// Blocks the write and pushes in one to stand the supervisor on that write. Empties the events that had come up.
    async fn park_in_flight(h: &mut Harness, endpoint: &MemoryEndpoint, push: Push) {
        endpoint.stall_writes(true);
        push(&h.peer).unwrap();
        settle().await;
        assert!(h.queue_drained(), "감독이 그것을 큐에서 꺼냈어야 한다");
        assert!(
            endpoint.drain().is_empty(),
            "감독이 그 쓰기 위에 서 있어야 한다"
        );
        h.drain();
    }

    #[tokio::test]
    async fn a_reply_less_frame_cut_mid_write_by_close_is_counted() {
        for (what, push) in reply_less_pushes() {
            let (mut h, endpoint) = live().await;
            park_in_flight(&mut h, &endpoint, push).await;
            h.peer.close();
            settle_until("종료", || h.peer.peer_state() == PeerState::Closed).await;
            // Releases the stuck goodbye — if not released, it cannot get to `teardown`'s report.
            endpoint.stall_writes(false);
            settle_until("감독이 끝났다", || h.supervisor_done()).await;
            let events = h.events();
            assert_eq!(outbound_dropped(&events), 1, "{what}");
            let tail: Vec<&str> = events.iter().rev().take(2).map(|e| e.kind()).collect();
            assert_eq!(
                tail,
                vec!["Closed", "Dropped"],
                "{what}: 정리의 유실 신고도 Closed 앞이다"
            );
        }
    }

    #[tokio::test]
    async fn a_reply_less_frame_cut_mid_write_by_reconnect_now_is_counted() {
        for (what, push) in reply_less_pushes() {
            let (mut h, endpoint) = live().await;
            park_in_flight(&mut h, &endpoint, push).await;
            h.peer.reconnect_now();
            // ★Release after control wins★ — if released first, the stuck write becomes ready together with control and may go out as is.
            settle_until("제어가 이겼다", || {
                h.peer.peer_state() == PeerState::Dialing
            })
            .await;
            endpoint.stall_writes(false);
            settle_until("다시 붙었다", || {
                h.net.dials() == 2 && h.peer.peer_state() == PeerState::Live
            })
            .await;
            settle().await;
            assert_eq!(outbound_dropped(&h.events()), 1, "{what}");
        }
    }

    #[tokio::test]
    async fn a_reply_less_frame_whose_write_fails_on_a_dead_link_is_counted() {
        for (what, push) in reply_less_pushes() {
            let (mut h, endpoint) = live().await;
            park_in_flight(&mut h, &endpoint, push).await;
            // ★If we cut only the writing side and release the stall, the stuck write ends in a channel error★ — the receiving side is closed so it fails
            //   (`testing::MemTx`). `fail` does not stand this branch: it reaches only the reading side, and during a write the supervisor does not look at the reading
            //   side. Dropping the endpoint is not it either — the reading side is cut with it, and if that end gets into the read-ahead queue first,
            //   the disconnect cause comes out as the read end (`salvage_read_ahead`).
            endpoint.fail_writes();
            endpoint.stall_writes(false);
            settle_until("백오프", || h.peer.peer_state() == PeerState::Backoff).await;
            settle().await;
            let events = h.events();
            assert!(
                events.iter().any(|e| matches!(
                    e,
                    TransportEvent::Disconnected {
                        cause: DisconnectCause::WriteFailed,
                        ..
                    }
                )),
                "{what}: 통로 오류 갈래를 재야 한다"
            );
            assert_eq!(outbound_dropped(&events), 1, "{what}");
        }
    }

    #[tokio::test]
    async fn a_reply_less_frame_cut_by_the_write_deadline_is_counted() {
        for (what, push) in reply_less_pushes() {
            let (mut h, endpoint) = live().await;
            park_in_flight(&mut h, &endpoint, push).await;
            h.clock.advance(Policy::default().write_deadline);
            settle_until("백오프", || h.peer.peer_state() == PeerState::Backoff).await;
            // Releases the stuck goodbye — the loss report comes out at the next stage (the backoff wait).
            endpoint.stall_writes(false);
            settle().await;
            let events = h.events();
            assert!(
                events.iter().any(|e| matches!(
                    e,
                    TransportEvent::Disconnected {
                        cause: DisconnectCause::WriteDeadline,
                        ..
                    }
                )),
                "{what}: 쓰기 시한 갈래를 재야 한다"
            );
            assert_eq!(outbound_dropped(&events), 1, "{what}");
        }
    }

    #[tokio::test]
    async fn a_request_cut_mid_write_is_woken_not_counted() {
        let (mut h, endpoint) = live().await;
        endpoint.stall_writes(true);
        // ★Poll once on the spot instead of starting a separate task★ — if started, at a moment when that task has not yet run
        //   the 「큐에서 꺼냈다」 (taken out of the queue) below becomes true on an empty queue. The first poll does everything up to putting in the queue synchronously.
        let peer = h.peer.clone();
        let mut waiting = Box::pin(peer.request(TestOut::Request {
            tag: 1,
            body: "x".into(),
        }));
        assert!(poll_immediate(&mut waiting).await.is_none());
        assert!(!h.queue_drained(), "요청이 큐에 들어가 있어야 한다");
        settle().await;
        assert!(h.queue_drained(), "감독이 그것을 큐에서 꺼냈어야 한다");
        // ★It must not have woken yet for 「쓰는 중」 (writing) to stand★ — a request rejected before writing also wakes below with the same error,
        //   so without this the last assertion holds regardless of the write.
        assert!(
            poll_immediate(&mut waiting).await.is_none(),
            "쓰기 전에 끝났다"
        );
        assert!(
            endpoint.drain().is_empty(),
            "감독이 그 쓰기 위에 서 있어야 한다"
        );
        h.drain();
        h.peer.close();
        settle_until("종료", || h.peer.peer_state() == PeerState::Closed).await;
        endpoint.stall_writes(false);
        settle_until("감독이 끝났다", || h.supervisor_done()).await;
        let woken = poll_immediate(&mut waiting)
            .await
            .expect("감독이 끝났는데 요청이 안 깨어났다");
        assert!(matches!(woken, Err(RequestError::Disconnected { .. })));
        assert_eq!(
            outbound_dropped(&h.events()),
            0,
            "요청은 오류로 깨어나므로 유실로 또 세지 않는다"
        );
    }

    #[tokio::test]
    async fn a_keepalive_cut_mid_write_is_not_counted() {
        let (mut h, endpoint) = live().await;
        endpoint.stall_writes(true);
        h.clock.advance(Policy::default().ping_interval);
        settle().await;
        assert!(endpoint.drain().is_empty(), "ping 이 걸려 있어야 한다");
        h.drain();
        h.peer.close();
        settle_until("종료", || h.peer.peer_state() == PeerState::Closed).await;
        endpoint.stall_writes(false);
        settle_until("감독이 끝났다", || h.supervisor_done()).await;
        assert_eq!(
            outbound_dropped(&h.events()),
            0,
            "keepalive 는 소비자가 넣은 프레임이 아니다"
        );
    }

    // ── 종료 ──
    #[tokio::test]
    async fn closing_is_final() {
        let h = build(Policy::default(), Arc::new(ImmediateHandshake));
        settle_until("연결", || h.peer.peer_state() == PeerState::Live).await;
        h.peer.close();
        settle_until("종료", || h.peer.peer_state() == PeerState::Closed).await;
        h.peer.reconnect_now();
        settle().await;
        assert_eq!(h.peer.peer_state(), PeerState::Closed);
        assert_eq!(h.net.dials(), 1);
    }

    #[tokio::test]
    async fn a_write_that_never_completes_is_cut_by_the_write_deadline() {
        let (mut h, endpoint) = live().await;
        endpoint.stall_writes(true);
        let _ = h.peer.notify(TestOut::Notice("stuck".into()));
        settle().await;
        assert!(endpoint.frames().is_empty(), "실제로 막혀 있어야 한다");
        h.clock.advance(Duration::from_secs(5));
        settle().await;
        assert_eq!(h.disconnect_cause(), Some(DisconnectCause::WriteDeadline));
    }

    // ── M4: it listens for control during a write too ──
    //
    // ★The published state being `Closed` does not mean the supervisor has folded★ — `run` publishes the state **before applying**
    //   the actions (between that loop's two `publish`es), so `settle_until` below passes even while teardown is
    //   still stopped on a stuck close. So we release the stall and measure **past that**.
    #[tokio::test]
    async fn close_is_heard_while_a_write_is_parked() {
        let (h, endpoint) = live().await;
        endpoint.stall_writes(true);
        let _ = h.peer.notify(TestOut::Notice("stuck".into()));
        settle().await;
        h.peer.close();
        settle_until("종료", || h.peer.peer_state() == PeerState::Closed).await;

        endpoint.stall_writes(false);
        let mut seen = Vec::new();
        settle_until("걸려 있던 goodbye 가 나갔다", || {
            seen.extend(endpoint.drain());
            seen.iter()
                .any(|m| matches!(m, ClientMsg::Close(c) if c.code == CloseCode::GOING_AWAY))
        })
        .await;
        // ★This assertion is a property of the harness, not the [`LinkTx::send`] contract★ — `testing::MemTx` stops a blocked write
        //   before handing it over, so if cut nothing remains. On the WS adapter, the cut frame may go out ahead of the goodbye
        //   (`ws.rs` 「통로 성질」 (channel properties)).
        assert!(
            !seen.iter().any(|m| matches!(m, ClientMsg::Frame(_))),
            "취소된 쓰기가 뒤늦게 나갔다: {seen:?}"
        );
    }

    // ── Shutdown: the goodbye code **really** goes out on the channel ──
    //
    // ★That it drops even the last handle is intended★ — `Registry::remove` has that shape (it drops the handle
    //   after telling it to close), and in that state the control channel immediately yields `Ready(None)`. In the edition where `drop_link` had a control arm,
    //   `select!`'s random arm choice counted that `None` as the winner and the close future was dropped before it was even polled — about
    //   half the time by measurement (2026-09-07).
    // ★Do not measure in one turn★ — so this net catches only half at a time. The repetition below makes it
    //   deterministic.
    #[tokio::test]
    async fn closing_speaks_the_goodbye_code_on_the_wire() {
        for round in 0..24 {
            let (h, endpoint) = live().await;
            h.peer.close();
            drop(h);
            let what = format!("{round}번째 바퀴: goodbye 가 통로로 나갔다");
            let mut seen = Vec::new();
            settle_until(&what, || {
                seen.extend(endpoint.drain());
                seen.iter()
                    .any(|m| matches!(m, ClientMsg::Close(c) if c.code == CloseCode::GOING_AWAY))
            })
            .await;
        }
    }

    // ── A stuck close is cut by the **deadline**, not by control ──
    //
    // ★On a channel where the write is already blocked, the close is blocked too★ — the harness's `stall_writes` blocks all three of `send`·`ping`·`close`
    //   (`testing::MemoryEndpoint::stall_writes`), so the real socket's shape is measured here.
    // ★Do not remove the line that pushes the clock★ — `drop_link` no longer cuts that wait with control (「알려진 한계」 (known limits) in
    //   the rustdoc there), so without it the supervisor stays standing on the stuck close and `settle_until` dies at the limit.
    #[tokio::test]
    async fn a_parked_closing_frame_is_cut_by_the_write_deadline_not_by_control() {
        let (h, endpoint) = live().await;
        endpoint.stall_writes(true);
        endpoint.fail("link died");
        settle_until("통로가 죽어 백오프로 내려갔다", || {
            h.peer.peer_state() == PeerState::Backoff
        })
        .await;
        assert!(
            !endpoint
                .drain()
                .iter()
                .any(|m| matches!(m, ClientMsg::Close(_))),
            "★닫기가 실제로 걸려 있어야 이 테스트가 무언가를 잰다★ — 나갔으면 아래 단언은 공짜다"
        );
        h.peer.close();
        // ★This assertion measures the 「제어가 아니라」 (not by control) in the title★ — the assertions below are green even in the edition where the control arm
        //   cut the stuck close. Before pushing the clock, it must not fold from the close request alone.
        settle().await;
        assert_ne!(
            h.peer.peer_state(),
            PeerState::Closed,
            "시계를 밀기 전에 접혔다 — 걸린 닫기를 제어가 잘랐다"
        );
        // What cuts the stuck close is this line. Control stays in the queue and is read after that.
        h.clock.advance(Duration::from_secs(5));
        settle_until("걸린 닫기 뒤에도 제어가 살아 있다", || {
            h.peer.peer_state() == PeerState::Closed
        })
        .await;
        assert!(
            !endpoint
                .drain()
                .iter()
                .any(|m| matches!(m, ClientMsg::Close(_))),
            "혼잡해서 시한에 잘리면 닫기 코드는 못 나간다 — `lib.rs` 가 인정한 그 한계다"
        );
    }

    // ── F1 regression: the goodbye is not lost even if a signal stays in the queue ──
    //
    // ★Calling `reconnect_now()` **twice** is the point★ — the supervisor picks only one signal per turn so
    //   the second stays in the queue, and then if `drop_link` sets a control arm, that arm is immediately `Ready` so the close future is
    //   dropped without even being polled. There is no congestion at all — by measurement 2026-09-07, 41·54·49·49 of 100 runs
    //   lost the goodbye that way, and after the fix it is 0.
    // ★Do not measure in one turn★ — even if the regression revives, half pass, so the net for this defect, the third recurrence,
    //   becomes a coin toss. The repetition below makes it deterministic.
    #[tokio::test]
    async fn a_queued_control_signal_does_not_swallow_the_goodbye_code() {
        let (mut h, mut endpoint) = live().await;
        for round in 0..24 {
            let dials = h.net.dials();
            h.peer.reconnect_now();
            h.peer.reconnect_now();
            settle_until("다시 붙었다", || {
                h.net.dials() == dials + 1 && h.peer.peer_state() == PeerState::Live
            })
            .await;
            assert!(
                endpoint
                    .drain()
                    .iter()
                    .any(|m| matches!(m, ClientMsg::Close(c) if c.code == CloseCode::GOING_AWAY)),
                "{round}번째 바퀴에서 goodbye 가 통로로 안 나갔다"
            );
            h.drain();
            endpoint = h.net.accept().expect("새 통로");
        }
    }

    // ── 3-0 U1b: the ordering area ──
    //
    // ★Set up with line length 1★ — the first `Connected` fills the line, so everything that comes up after goes through the supervisor's ordering
    //   area. If the line has room, the ordering area empties right where it puts in, so nothing is measured.

    /// Runs the supervisor and drains the receiving side — until nothing comes up in a turn.
    async fn drain_until_quiet(h: &mut Harness) -> Vec<Incoming<TestWire>> {
        let mut out = Vec::new();
        loop {
            settle().await;
            let batch = h.drain();
            if batch.is_empty() {
                return out;
            }
            out.extend(batch);
        }
    }

    fn label(item: &Incoming<TestWire>) -> String {
        match item {
            Incoming::Message(arrival) => match &arrival.msg {
                TestIn::Notice(body) => format!("msg:{body}"),
                other => format!("msg:{other:?}"),
            },
            Incoming::Event(TransportEvent::Connected { generation, .. }) => {
                format!("Connected:{}", generation.0)
            }
            Incoming::Event(TransportEvent::Disconnected {
                generation, cause, ..
            }) => format!("Disconnected:{}:{cause:?}", generation.0),
            Incoming::Event(TransportEvent::Closed {
                generation, cause, ..
            }) => format!("Closed:{}:{cause:?}", generation.0),
            Incoming::Event(other) => other.kind().to_string(),
        }
    }

    fn labels(items: &[Incoming<TestWire>]) -> Vec<String> {
        items.iter().map(label).collect()
    }

    async fn live_on_a_full_line() -> (Harness, Arc<MemoryEndpoint>) {
        let h = build_with(Policy::default(), Arc::new(ImmediateHandshake), 1);
        settle_until("첫 연결", || h.peer.peer_state() == PeerState::Live).await;
        let endpoint = h.net.accept().expect("통로가 열렸어야 한다");
        (h, endpoint)
    }

    #[tokio::test]
    async fn lifecycle_events_are_never_lost_while_the_line_is_full() {
        let (mut h, endpoint) = live_on_a_full_line().await;
        h.net.fail_forever();
        endpoint.close();
        settle_until("백오프", || h.peer.peer_state() == PeerState::Backoff).await;
        for delay in [500u64, 1000, 2000] {
            h.clock.advance(Duration::from_millis(delay));
            settle().await;
        }
        assert_eq!(
            h.peer.peer_state(),
            PeerState::GaveUp(crate::machine::GaveUpReason::BudgetExhausted)
        );

        let items = drain_until_quiet(&mut h).await;
        assert_eq!(
            labels(&items),
            vec![
                "Connected:1",
                "Disconnected:1:PeerClosed",
                "ConnectFailed",
                "ConnectFailed",
                "ConnectFailed",
                "GaveUp",
            ],
            "★받는 쪽이 밀려 있어도 생애 사건은 하나도 안 버려지고 낸 순서대로 온다★"
        );
    }

    #[tokio::test]
    async fn a_message_then_an_immediate_close_arrives_message_first_then_the_disconnect() {
        let (mut h, endpoint) = live_on_a_full_line().await;
        endpoint.push(TestWire::encode_in(&TestIn::Notice("last words".into())));
        endpoint.close();
        settle().await;
        assert_eq!(
            h.peer.peer_state(),
            PeerState::Live,
            "★닫힘은 붙든 말 뒤에 줄 서 있다★ — 받는 쪽이 그 말을 꺼내기 전에는 감독이 닫힘을 읽지 않는다"
        );

        let items = drain_until_quiet(&mut h).await;
        assert_eq!(
            labels(&items),
            vec!["Connected:1", "msg:last words", "Disconnected:1:PeerClosed"],
            "★상대가 보내고 곧바로 닫아도 그 말은 안 버려지고 끊김보다 먼저 온다★"
        );
        assert_eq!(h.peer.peer_state(), PeerState::Backoff);
    }

    #[tokio::test]
    async fn held_items_keep_flowing_while_the_supervisor_is_mid_handshake() {
        let mut h = build_with(Policy::default(), Arc::new(ListeningHandshake), 1);
        settle_until("핸드셰이크 대기", || {
            h.peer.peer_state() == PeerState::Handshaking
        })
        .await;
        let first = h.net.accept().unwrap();
        first.push(Frame::Text("welcome".into()));
        settle_until("연결", || h.peer.peer_state() == PeerState::Live).await;
        first.push(TestWire::encode_in(&TestIn::Notice("n0".into())));
        settle().await;
        // The peer's close lines up behind the held n0 and is not read (backpressure), so make it reconnect by control.
        h.peer.reconnect_now();
        settle_until("둘째 핸드셰이크 대기", || {
            h.peer.peer_state() == PeerState::Handshaking
        })
        .await;
        let _second = h.net.accept().unwrap();

        let items = drain_until_quiet(&mut h).await;
        assert_eq!(
            labels(&items),
            vec!["Connected:1", "msg:n0", "Disconnected:1:Explicit"],
            "★핸드셰이크가 상대를 기다리는 동안에도 받는 쪽이 비우는 대로 붙든 것이 실린다★"
        );
        assert_eq!(h.peer.peer_state(), PeerState::Handshaking);
        assert_eq!(
            h.net.dials(),
            2,
            "싣는 동안 걸던 통로를 버리고 다시 걸지 않았다"
        );
    }

    #[tokio::test]
    async fn closing_hands_over_everything_held_and_ends_with_closed() {
        let (mut h, endpoint) = live_on_a_full_line().await;
        for n in 0..3 {
            endpoint.push(TestWire::encode_in(&TestIn::Notice(format!("n{n}"))));
        }
        settle().await;
        assert_eq!(
            h.peer.held_bytes(),
            3 * "not|n0".len(),
            "n0 은 순서 칸에, n1 · n2 는 앞잡이 줄에 있다 — 순서 칸이 붙든 동안 프레임 팔을 뺐다"
        );
        h.peer.close();
        settle_until("접힘", || h.peer.peer_state() == PeerState::Closed).await;

        let items = drain_until_quiet(&mut h).await;
        assert_eq!(
            labels(&items),
            vec![
                "Connected:1",
                "msg:n0",
                "msg:n1",
                "msg:n2",
                "Disconnected:1:Explicit",
                "Closed:1:Explicit",
            ],
            "★붙든 것을 다 실은 뒤 Closed 가 맨 끝이다★ — 앞잡이 줄의 n1 · n2 도 통로를 버리기 전에 건져 \
             끊김 앞에 실린다"
        );
        assert_eq!(h.peer.held_bytes(), 0, "꺼낸 몫은 셈에서 빠졌다");
        assert!(
            matches!(
                h.inbound.try_recv(),
                Err(mpsc::error::TryRecvError::Disconnected)
            ),
            "Closed 뒤에는 감독이 넣는 끝을 놓는다"
        );
    }

    // ★Measures the order inside teardown (waking the waiters · closing the queue → draining the ordering area)★. The fold is
    //   visible to the handle before the channel is dropped (`run` emits it before `apply`) and the machine's `FailPending` empties the pending slots and the queue before that,
    //   so a request that `fail_pending` in teardown catches arises only from a race that folded just after passing the handle's entry check (is the bound generation operating).
    //   We make that request by putting it directly in the queue, and slip it in with the write blocked and the supervisor standing on the machine's `DropLink`.
    #[tokio::test]
    async fn requests_end_at_once_while_the_teardown_waits_on_the_held_items() {
        let (mut h, endpoint) = live_on_a_full_line().await;
        endpoint.push(TestWire::encode_in(&TestIn::Notice("held".into())));
        let asked = spawn_request(&h.peer, 7);
        settle().await;
        assert!(
            matches!(endpoint.take(), Some(ClientMsg::Frame(_))),
            "요청이 나가 답을 기다리는 중이어야 한다"
        );
        endpoint.stall_writes(true);
        h.peer.close();
        settle_until("접힘", || h.peer.peer_state() == PeerState::Closed).await;
        assert!(
            matches!(joined(asked).await, Err(RequestError::Disconnected { .. })),
            "★받는 쪽이 안 비워도 대기 요청은 곧바로 깨어난다★"
        );

        let (reply, mut raced) = oneshot::channel();
        h.peer
            .inner
            .data_tx
            .try_send(PeerCmd {
                generation: h.peer.generation(),
                op: PeerOp::Request {
                    out: TestOut::Request {
                        tag: 9,
                        body: "raced".into(),
                    },
                    reply,
                },
            })
            .unwrap_or_else(|_| panic!("나가는 큐에 자리가 있어야 한다"));
        settle().await;
        assert!(
            matches!(raced.try_recv(), Err(oneshot::error::TryRecvError::Empty)),
            "감독이 막힌 닫기 위에 서 있어야 한다 — 이 요청은 정리의 몫이다"
        );
        endpoint.stall_writes(false);
        settle().await;
        assert!(h.supervisor_done(), "정리가 나가는 큐를 닫았다");
        assert!(
            matches!(raced.try_recv(), Ok(Err(RequestError::Disconnected { .. }))),
            "★정리에 걸린 요청은 순서 칸 비우기를 기다리지 않고 곧바로 끝난다★"
        );
        assert!(
            matches!(
                joined(spawn_request(&h.peer, 8)).await,
                Err(RequestError::Disconnected { .. })
            ),
            "정리 중에 낸 요청도 곧바로 끝난다"
        );
        assert!(
            h.peer.notify(TestOut::Notice("late".into())).is_err(),
            "정리 중에 든 알림은 곧바로 거절된다"
        );
        assert!(
            !h.inbound.is_closed(),
            "정리가 아직 순서 칸에서 기다린다 — 감독이 넣는 끝을 놓지 않았다"
        );

        let items = drain_until_quiet(&mut h).await;
        assert_eq!(
            labels(&items),
            vec![
                "Connected:1",
                "msg:held",
                "Disconnected:1:Explicit",
                "Closed:1:Explicit",
            ]
        );
    }

    #[tokio::test]
    async fn the_teardown_gives_up_on_a_line_nobody_drains_only_at_the_limit() {
        for (waited, expected) in [
            (
                CLOSE_DRAIN_LIMIT - Duration::from_secs(1),
                vec![
                    "Connected:1",
                    "msg:held",
                    "Disconnected:1:Explicit",
                    "Closed:1:Explicit",
                ],
            ),
            (CLOSE_DRAIN_LIMIT, vec!["Connected:1"]),
        ] {
            let (mut h, endpoint) = live_on_a_full_line().await;
            endpoint.push(TestWire::encode_in(&TestIn::Notice("held".into())));
            settle().await;
            h.peer.close();
            settle_until("접힘", || h.peer.peer_state() == PeerState::Closed).await;
            h.clock.advance(waited);
            settle().await;

            let items = drain_until_quiet(&mut h).await;
            assert_eq!(labels(&items), expected, "{waited:?} 기다린 뒤");
            assert!(
                matches!(
                    h.inbound.try_recv(),
                    Err(mpsc::error::TryRecvError::Disconnected)
                ),
                "{waited:?}: 감독이 끝났다"
            );
        }
    }

    #[tokio::test]
    async fn a_teardown_ends_at_once_when_the_consumer_is_gone() {
        let (mut h, endpoint) = live_on_a_full_line().await;
        endpoint.push(TestWire::encode_in(&TestIn::Notice("held".into())));
        settle().await;
        h.peer.close();
        settle_until("접힘", || h.peer.peer_state() == PeerState::Closed).await;
        assert_eq!(h.clock.sleepers(), 1, "정리가 상한 시계를 걸고 기다린다");

        drop_the_consumer(&mut h);
        settle_until(
            "받는 쪽이 사라지면 정리가 기다리지 않는다",
            || h.task.is_finished(),
        )
        .await;
        assert_eq!(h.clock.sleepers(), 0, "상한 시계를 놓았다");
        assert!(h.exited_cleanly().await, "감독이 패닉 없이 끝났다");
    }

    fn drop_the_consumer(h: &mut Harness) {
        drop(std::mem::replace(&mut h.inbound, mpsc::channel(1).1));
    }

    // ── 3-0 U1b: if the receiving side disappears, from any waiting loop it folds without waiting for the clock ──
    //
    // ★Do not push the clock★ — if pushed, the backoff ends or the deadline hits and the waiting loop gets out even without that decision.

    #[tokio::test]
    async fn a_backing_off_peer_holding_items_folds_when_the_consumer_goes() {
        let (mut h, endpoint) = live_on_a_full_line().await;
        // What is held is the disconnect event — if a notification were put before it, the close would line up behind it and not be read (backpressure).
        endpoint.close();
        settle_until("백오프", || h.peer.peer_state() == PeerState::Backoff).await;
        assert_eq!(
            h.inbound.capacity(),
            0,
            "줄이 차 있어 순서 칸이 붙들고 있다"
        );

        drop_the_consumer(&mut h);
        settle_until("접힘", || h.peer.peer_state() == PeerState::Closed).await;
        assert_eq!(h.net.dials(), 1, "다시 걸지 않았다");
        settle_until("감독이 끝났다", || h.task.is_finished()).await;
        assert!(h.exited_cleanly().await);
    }

    #[tokio::test]
    async fn a_peer_that_gave_up_holding_items_folds_when_the_consumer_goes() {
        let (mut h, endpoint) = live_on_a_full_line().await;
        h.net.fail_forever();
        endpoint.close();
        settle_until("백오프", || h.peer.peer_state() == PeerState::Backoff).await;
        for delay in [500u64, 1000, 2000] {
            h.clock.advance(Duration::from_millis(delay));
            settle().await;
        }
        assert!(matches!(h.peer.peer_state(), PeerState::GaveUp(_)));
        assert_eq!(
            h.inbound.capacity(),
            0,
            "줄이 차 있어 순서 칸이 붙들고 있다"
        );

        drop_the_consumer(&mut h);
        settle_until("접힘", || h.peer.peer_state() == PeerState::Closed).await;
        settle_until("감독이 끝났다", || h.task.is_finished()).await;
        assert!(h.exited_cleanly().await);
    }

    #[tokio::test]
    async fn a_mid_handshake_peer_holding_items_folds_when_the_consumer_goes() {
        let mut h = build_with(Policy::default(), Arc::new(ListeningHandshake), 1);
        settle_until("핸드셰이크 대기", || {
            h.peer.peer_state() == PeerState::Handshaking
        })
        .await;
        let first = h.net.accept().unwrap();
        first.push(Frame::Text("welcome".into()));
        settle_until("연결", || h.peer.peer_state() == PeerState::Live).await;
        first.push(TestWire::encode_in(&TestIn::Notice("n0".into())));
        settle().await;
        // The same line as in `held_items_keep_flowing_while_the_supervisor_is_mid_handshake` — make it reconnect by control.
        h.peer.reconnect_now();
        settle_until("둘째 핸드셰이크 대기", || {
            h.peer.peer_state() == PeerState::Handshaking
        })
        .await;
        assert_eq!(
            h.inbound.capacity(),
            0,
            "줄이 차 있어 순서 칸이 붙들고 있다"
        );

        drop_the_consumer(&mut h);
        settle_until("접힘", || h.peer.peer_state() == PeerState::Closed).await;
        assert_eq!(
            h.peer.generation(),
            Generation(1),
            "둘째 세대를 뽑지 않았다"
        );
        settle_until("감독이 끝났다", || h.task.is_finished()).await;
        assert!(h.exited_cleanly().await);
    }

    // ── 3-0 U1d: even with nothing held, if the receiving side disappears it folds on the spot ──
    //
    // In the bundle above the ordering area holds, so the drain arm sees the disappearance. Here nothing is held, so only the receiver-end
    // closing arm (`Sender::closed`) placed in each waiting loop sees it — without that arm, backoff · give-up · handshake · quiet operation would stay
    // as they are until the clock or the next event. The clock is not pushed here either.

    #[tokio::test]
    async fn a_waiting_peer_with_nothing_held_folds_when_the_consumer_goes() {
        for gave_up in [false, true] {
            let (mut h, endpoint) = live().await;
            if gave_up {
                h.net.fail_forever();
            }
            endpoint.close();
            settle_until("백오프", || h.peer.peer_state() == PeerState::Backoff).await;
            if gave_up {
                for delay in [500u64, 1000, 2000] {
                    h.clock.advance(Duration::from_millis(delay));
                    settle().await;
                }
                assert!(matches!(h.peer.peer_state(), PeerState::GaveUp(_)));
            }
            assert!(
                h.inbound.capacity() > 0,
                "gave_up={gave_up}: 줄에 자리가 있다 — 순서 칸은 붙든 것이 없다"
            );
            let dials = h.net.dials();
            drop_the_consumer(&mut h);
            settle_until("접힘", || h.peer.peer_state() == PeerState::Closed).await;
            assert_eq!(h.net.dials(), dials, "gave_up={gave_up}: 다시 걸지 않았다");
            settle_until("감독이 끝났다", || h.task.is_finished()).await;
            assert!(h.exited_cleanly().await, "gave_up={gave_up}");
        }
    }

    #[tokio::test]
    async fn a_mid_handshake_peer_with_nothing_held_folds_when_the_consumer_goes() {
        let mut h = build(Policy::default(), Arc::new(ListeningHandshake));
        settle_until("핸드셰이크 대기", || {
            h.peer.peer_state() == PeerState::Handshaking
        })
        .await;
        drop_the_consumer(&mut h);
        settle_until("접힘", || h.peer.peer_state() == PeerState::Closed).await;
        assert_eq!(h.net.dials(), 1, "다시 걸지 않았다");
        assert_eq!(h.peer.generation(), Generation(0), "세대를 뽑지 않았다");
        settle_until("감독이 끝났다", || h.task.is_finished()).await;
        assert!(h.exited_cleanly().await);
    }

    #[tokio::test]
    async fn a_silent_live_peer_folds_without_another_frame_when_the_consumer_goes() {
        let (mut h, endpoint) = live().await;
        drop_the_consumer(&mut h);
        settle_until("접힘", || h.peer.peer_state() == PeerState::Closed).await;
        settle_until(
            "감독이 끝났다 — 상한을 기다리지 않았다",
            || h.task.is_finished(),
        )
        .await;
        assert_eq!(
            endpoint.drain(),
            vec![ClientMsg::Close(Close::new(
                CloseCode::GOING_AWAY,
                "closing"
            ))],
            "★상대는 조용했고 우리가 보낸 것은 닫기뿐이다★"
        );
        assert_eq!(h.net.dials(), 1, "다시 걸지 않았다");
        assert!(h.exited_cleanly().await);
    }

    // Even if the receiving side's disappearance and a 「지금 다시 붙어라」 (reconnect now) are ready together — whichever arm wakes first — it does not dial again.
    #[tokio::test]
    async fn a_reconnect_racing_the_consumers_exit_does_not_dial() {
        let (mut h, _endpoint) = live().await;
        drop_the_consumer(&mut h);
        h.peer.reconnect_now();
        settle_until("접힘", || h.peer.peer_state() == PeerState::Closed).await;
        assert_eq!(h.net.dials(), 1, "아무도 안 들을 데몬에 다시 걸지 않았다");
        assert_eq!(h.peer.generation(), Generation(1));
        settle_until("감독이 끝났다", || h.task.is_finished()).await;
        assert!(h.exited_cleanly().await);
    }

    // ── 3-0 U1d: end cause — tell the read end from the write end ──
    //
    // The daemon adapter uses this value to tell the read end (finish running the remaining commands) from the write end (cut at once) (TRD 3 §3-0 ①).
    // We do not drop the endpoint — both sides would be cut together and which one is learned first would depend on scheduling.

    type Cut = fn(&Harness, &MemoryEndpoint);
    type ReadEnd = fn(&MemoryEndpoint);
    type AppAct = fn(&Harness);

    #[tokio::test]
    async fn a_read_failure_and_a_write_failure_arrive_as_different_causes() {
        let cases: [(&str, Cut, DisconnectCause); 3] = [
            (
                "읽기 오류",
                |_, e| e.fail("boom"),
                DisconnectCause::ReadFailed,
            ),
            (
                "알림 쓰기 실패",
                |h, e| {
                    e.fail_writes();
                    h.peer.notify(TestOut::Notice("x".into())).unwrap();
                },
                DisconnectCause::WriteFailed,
            ),
            (
                "keepalive 쓰기 실패",
                |h, e| {
                    e.fail_writes();
                    h.clock.advance(Policy::default().ping_interval);
                },
                DisconnectCause::WriteFailed,
            ),
        ];
        for (what, cut, expected) in cases {
            let (mut h, endpoint) = live().await;
            cut(&h, &endpoint);
            settle_until("백오프", || h.peer.peer_state() == PeerState::Backoff).await;
            assert_eq!(h.disconnect_cause(), Some(expected), "{what}");
        }
    }

    // ── 3-0 U1d: when dropping the channel, a read end already in the read-ahead queue becomes the disconnect cause ──
    //
    // Line length 1 — the first notification is held in the ordering area and the operating loop pulls the frame arm (backpressure). The peer's close · read
    // error that comes after stays in the read-ahead queue unread, and the supervisor leaves first with its own cause (write failure · write deadline · overflow …).

    /// Stand the supervisor in the operating phase with one read end placed in the read-ahead queue.
    async fn a_read_end_behind_a_held_line(
        policy: Policy,
        end: ReadEnd,
    ) -> (Harness, Arc<MemoryEndpoint>) {
        let h = build_with(policy, Arc::new(ImmediateHandshake), 1);
        settle_until("첫 연결", || h.peer.peer_state() == PeerState::Live).await;
        let endpoint = h.net.accept().expect("통로가 열렸어야 한다");
        endpoint.push(TestWire::encode_in(&TestIn::Notice("held".into())));
        settle().await;
        end(&endpoint);
        settle().await;
        assert_eq!(endpoint.unread(), 0, "그 끝은 앞잡이 줄에 들었다");
        assert_eq!(
            h.peer.peer_state(),
            PeerState::Live,
            "프레임 팔을 뺀 감독은 그 끝을 아직 못 읽었다"
        );
        (h, endpoint)
    }

    fn disconnects(items: &[Incoming<TestWire>]) -> Vec<String> {
        labels(items)
            .into_iter()
            .filter(|label| label.starts_with("Disconnected"))
            .collect()
    }

    /// One of our-side causes for the supervisor leaving the operating phase — the policy, and the steps taken while running the supervisor in between.
    type Leave = (&'static str, fn() -> Policy, &'static [Cut]);

    const LEAVES: [Leave; 4] = [
        (
            "알림 쓰기 실패",
            Policy::default,
            &[|h, e| {
                e.fail_writes();
                h.peer.notify(TestOut::Notice("x".into())).unwrap();
            }],
        ),
        (
            "keepalive 쓰기 실패",
            Policy::default,
            &[|h, e| {
                e.fail_writes();
                h.clock.advance(Policy::default().ping_interval);
            }],
        ),
        // Release the stall after the deadline cuts the stuck write — the close that drops the channel must go out before entering backoff.
        (
            "쓰기 시한",
            Policy::default,
            &[
                |h, e| {
                    e.stall_writes(true);
                    h.peer.notify(TestOut::Notice("x".into())).unwrap();
                },
                |h, _| h.clock.advance(Policy::default().write_deadline),
                |_, e| e.stall_writes(false),
            ],
        ),
        // One slot in the outbound queue — the second notification cannot get in and is counted, and the supervisor cuts at the start of the turn after writing the first (default policy).
        (
            "나가는 큐 넘침",
            || Policy {
                outbound_queue: 1,
                ..Policy::default()
            },
            &[|h, _| {
                h.peer.notify(TestOut::Notice("x".into())).unwrap();
                let _ = h.peer.notify(TestOut::Notice("y".into()));
            }],
        ),
    ];

    #[tokio::test]
    async fn a_read_end_already_read_ahead_outranks_our_own_reason_for_leaving() {
        let ends: [(&str, ReadEnd, &str); 2] = [
            ("상대의 닫힘", |e| e.close(), "Disconnected:1:PeerClosed"),
            ("읽기 오류", |e| e.fail("boom"), "Disconnected:1:ReadFailed"),
        ];
        for (end_what, end, expected) in ends {
            for (leave_what, policy, steps) in LEAVES {
                let (mut h, endpoint) = a_read_end_behind_a_held_line(policy(), end).await;
                for step in steps {
                    step(&h, &endpoint);
                    settle().await;
                }
                settle_until("백오프", || h.peer.peer_state() == PeerState::Backoff).await;
                let items = drain_until_quiet(&mut h).await;
                assert_eq!(
                    disconnects(&items),
                    vec![expected.to_string()],
                    "{end_what} 뒤 {leave_what}: ★먼저 난 읽기 끝이 원인이다★"
                );
            }
        }
    }

    // If the app closed or asked to reconnect, that cause (`Explicit`) stays — the caller knows.
    #[tokio::test]
    async fn an_app_close_or_reconnect_stays_explicit_with_a_read_end_read_ahead() {
        let acts: [(&str, AppAct); 2] = [
            ("닫기", |h| h.peer.close()),
            ("다시 걸기", |h| h.peer.reconnect_now()),
        ];
        for (what, act) in acts {
            let (mut h, _endpoint) =
                a_read_end_behind_a_held_line(Policy::default(), |e| e.close()).await;
            act(&h);
            settle_until("접었거나 다시 걸었다", || {
                h.peer.peer_state() == PeerState::Closed || h.net.dials() == 2
            })
            .await;
            let items = drain_until_quiet(&mut h).await;
            assert_eq!(
                disconnects(&items),
                vec!["Disconnected:1:Explicit"],
                "{what}"
            );
        }
    }

    // If our cause is already the reader's end (cut on decode failure), it stays as is — within the line that one frame comes before the close that followed.
    #[tokio::test]
    async fn a_decode_failure_cut_keeps_its_cause_over_a_close_read_behind_it() {
        let mut h = build(
            Policy {
                decode_failure: DecodeFailurePolicy::Disconnect,
                ..Policy::default()
            },
            Arc::new(ImmediateHandshake),
        );
        settle_until("연결", || h.peer.peer_state() == PeerState::Live).await;
        let endpoint = h.net.accept().unwrap();
        h.drain();
        endpoint.push(Frame::Text("garbage".into()));
        endpoint.close();
        settle_until("백오프", || h.peer.peer_state() == PeerState::Backoff).await;
        assert_eq!(endpoint.unread(), 0, "닫힘도 읽혀 앞잡이 줄에 들었다");
        assert_eq!(h.disconnect_cause(), Some(DisconnectCause::ReadFailed));
    }

    // The same holds inside the line salvaged while dropping the channel — the one frame that cuts on decode failure comes before the close that followed, so that cause stays.
    //   What was already read after it is still handed over (`salvaging_carries_on_past_a_frame_it_cannot_decode`).
    #[tokio::test]
    async fn a_decode_cut_salvaged_from_the_read_ahead_outranks_a_close_behind_it() {
        let (mut h, endpoint) = a_read_end_behind_a_held_line(
            Policy {
                decode_failure: DecodeFailurePolicy::Disconnect,
                ..Policy::default()
            },
            |e| {
                e.push(Frame::Text("garbage".into()));
                e.push(TestWire::encode_in(&TestIn::Notice("after".into())));
                e.close();
            },
        )
        .await;
        endpoint.fail_writes();
        h.peer.notify(TestOut::Notice("x".into())).unwrap();
        settle_until("백오프", || h.peer.peer_state() == PeerState::Backoff).await;
        let items = drain_until_quiet(&mut h).await;
        let seen = labels(&items);
        assert!(
            seen.contains(&"msg:after".to_string()),
            "끊는 한 장 뒤에 이미 읽은 것도 넘겼다: {seen:?}"
        );
        assert_eq!(disconnects(&items), vec!["Disconnected:1:ReadFailed"]);
    }

    // ── 3-0 U1d: even if the receiving side disappears mid-write, it does not wait for that write ──
    //
    // One stuck write and one stuck keepalive at the place that drains the queue back to back (`Turn::Outbound`) — both must fold without pushing the clock.
    // Without the write arm, it stands for each stuck write up to `write_deadline`.

    #[tokio::test]
    async fn a_consumer_leaving_mid_batch_folds_without_waiting_out_the_stalled_write() {
        let mut h = build(
            Policy {
                outbound_queue: 4,
                ..Policy::default()
            },
            Arc::new(ImmediateHandshake),
        );
        settle_until("연결", || h.peer.peer_state() == PeerState::Live).await;
        let endpoint = h.net.accept().unwrap();
        // Stand it on a keepalive and fill the queue past half, then block the write and release the keepalive — the next turn drains the queue back to back
        //   and stands on the first write.
        endpoint.stall_pings(true);
        h.clock.advance(Policy::default().ping_interval);
        settle().await;
        for n in 0..3 {
            h.peer
                .notify(TestOut::Notice(format!("n{n}")))
                .expect("나가는 큐");
        }
        endpoint.stall_writes(true);
        endpoint.stall_pings(false);
        settle().await;
        assert_eq!(endpoint.drain(), vec![ClientMsg::Ping]);
        assert_eq!(
            h.peer.inner.data_tx.capacity(),
            2,
            "감독이 하나를 꺼내 쓰다 섰고 둘이 큐에 남았다"
        );

        drop_the_consumer(&mut h);
        settle_until("접힘 — 시계를 밀지 않았다", || {
            h.peer.peer_state() == PeerState::Closed
        })
        .await;
        // The close that drops the channel is caught by the write block — that one is a place we deliberately do not listen at, so release it (`drop_link`).
        endpoint.stall_writes(false);
        settle_until("감독이 끝났다", || h.task.is_finished()).await;
        assert_eq!(
            endpoint.drain(),
            vec![ClientMsg::Close(Close::new(
                CloseCode::GOING_AWAY,
                "closing"
            ))],
            "★걸린 쓰기도 남은 둘도 안 나갔다★"
        );
        assert!(h.exited_cleanly().await);
    }

    #[tokio::test]
    async fn a_consumer_leaving_mid_keepalive_folds_without_waiting_out_the_stalled_ping() {
        let (mut h, endpoint) = live().await;
        endpoint.stall_pings(true);
        h.clock.advance(Policy::default().ping_interval);
        settle().await;
        assert!(endpoint.drain().is_empty(), "keepalive 위에 서 있다");

        drop_the_consumer(&mut h);
        settle_until("접힘 — 시계를 밀지 않았다", || {
            h.peer.peer_state() == PeerState::Closed
        })
        .await;
        settle_until("감독이 끝났다", || h.task.is_finished()).await;
        assert_eq!(
            endpoint.drain(),
            vec![ClientMsg::Close(Close::new(
                CloseCode::GOING_AWAY,
                "closing"
            ))]
        );
        assert!(h.exited_cleanly().await);
    }

    // ── 3-0 U1c: backpressure · byte count · stopping the silence decision ──

    /// One frame whose payload is exactly `len` bytes.
    fn blob(tag: u8, len: usize) -> Frame {
        Frame::Binary(vec![tag; len])
    }

    fn blob_tags(items: &[Incoming<TestWire>]) -> Vec<u8> {
        items
            .iter()
            .filter_map(|item| match item.as_message().map(|a| &a.msg) {
                Some(TestIn::Blob(bytes)) => Some(bytes[0]),
                _ => None,
            })
            .collect()
    }

    fn notices(items: &[Incoming<TestWire>]) -> Vec<String> {
        items
            .iter()
            .filter_map(|item| match item.as_message().map(|a| &a.msg) {
                Some(TestIn::Notice(body)) => Some(body.clone()),
                _ => None,
            })
            .collect()
    }

    #[tokio::test]
    async fn a_held_line_stops_reading_the_link_instead_of_growing_the_order_slot() {
        let frame_buffer = 4;
        let mut h = build_with(
            Policy {
                frame_buffer,
                ..Policy::default()
            },
            Arc::new(ImmediateHandshake),
            1,
        );
        settle_until("첫 연결", || h.peer.peer_state() == PeerState::Live).await;
        let endpoint = h.net.accept().unwrap();
        for n in 0..20 {
            endpoint.push(TestWire::encode_in(&TestIn::Notice(format!("n{n:02}"))));
        }
        settle().await;

        // n00 = the ordering area · n01~n04 = the read-ahead queue. The read task reads only after taking a slot, so there is no frame in hand.
        let read = 1 + frame_buffer;
        assert_eq!(
            endpoint.unread(),
            20 - read,
            "★순서 칸이 붙든 동안 프레임을 더 안 꺼내 통로가 밀린다★ — 순서 칸이 자라지 않는다"
        );
        assert_eq!(h.peer.held_bytes(), read * "not|n00".len());
        assert_eq!(
            h.peer.peer_state(),
            PeerState::Live,
            "밀린 것은 끊을 일이 아니다"
        );

        let items = drain_until_quiet(&mut h).await;
        assert_eq!(
            notices(&items),
            (0..20).map(|n| format!("n{n:02}")).collect::<Vec<_>>(),
            "★배압 아래에서도 하나도 안 버려지고 온 순서대로 나온다★"
        );
        assert_eq!(endpoint.unread(), 0);
        assert_eq!(h.peer.held_bytes(), 0, "꺼낸 몫은 셈에서 빠진다");
    }

    #[tokio::test]
    async fn a_frame_past_the_byte_budget_stops_the_reads_and_loses_nothing() {
        let budget = 100;
        let mut h = build_budgeted(
            Policy::default(),
            Arc::new(ImmediateHandshake),
            4096,
            budget,
        );
        settle_until("첫 연결", || h.peer.peer_state() == PeerState::Live).await;
        let endpoint = h.net.accept().unwrap();
        h.drain();
        for tag in 0..5u8 {
            endpoint.push(blob(tag, 150));
        }
        settle().await;
        assert_eq!(
            endpoint.unread(),
            4,
            "★빈 셈에는 예산보다 큰 한 장도 들어가고, 그 뒤로는 통로를 안 읽는다★(더하기 전 물높이)"
        );
        assert_eq!(h.peer.held_bytes(), 150);

        let mut tags = Vec::new();
        for _ in 0..5 {
            let taken = h.drain();
            assert_eq!(blob_tags(&taken).len(), 1, "한 번에 한 장씩만 읽힌다");
            tags.extend(blob_tags(&taken));
            settle().await;
        }
        assert_eq!(tags, vec![0, 1, 2, 3, 4], "★손실 0 · 순서 그대로★");
        assert_eq!(endpoint.unread(), 0);
        assert_eq!(h.peer.held_bytes(), 0);
    }

    // ★Line length 1 uses all three layers★ — the first frame stands in the ordering area and the second in the read-ahead queue. The worst case is reading one more frame right under the read share (since streams
    //   are not used, the whole budget), so we make one frame 1 smaller than the read share. The ceiling that uses streams and fills up to the window
    //   share is `holding_a_window_full_and_a_read_share_full_still_fits_the_budget_plus_one_message` below.
    #[tokio::test]
    async fn what_a_peer_holds_never_exceeds_the_budget_plus_one_message() {
        let budget = 1000;
        let message = budget - 1;
        let mut h = build_budgeted(Policy::default(), Arc::new(ImmediateHandshake), 1, budget);
        settle_until("첫 연결", || h.peer.peer_state() == PeerState::Live).await;
        let endpoint = h.net.accept().unwrap();
        let sent = 10u8;
        for tag in 0..sent {
            endpoint.push(blob(tag, message));
            settle().await;
            assert!(
                h.peer.held_bytes() <= budget + message,
                "{tag}번째를 보낸 뒤 {} 바이트를 쥐었다 — 앞잡이 줄 · 순서 칸 · 줄의 합이 B + M 을 넘었다",
                h.peer.held_bytes()
            );
        }
        assert_eq!(
            h.peer.held_bytes(),
            2 * message,
            "★스트림을 안 쓰는 감독은 예산 전부를 읽기 몫으로 쓴다★ — 그 밑에서 한 장 더 읽고 거기서 멈췄다"
        );
        assert_eq!(endpoint.unread(), usize::from(sent) - 2);

        let mut tags = Vec::new();
        loop {
            settle().await;
            assert!(h.peer.held_bytes() <= budget + message);
            let taken = h.drain();
            if taken.is_empty() {
                break;
            }
            tags.extend(blob_tags(&taken));
        }
        assert_eq!(
            tags,
            (0..sent).collect::<Vec<_>>(),
            "★손실 0 · 순서 그대로★"
        );
        assert_eq!(h.peer.held_bytes(), 0);
    }

    #[tokio::test]
    async fn keepalive_goes_out_and_close_is_heard_while_the_line_is_held() {
        let (mut h, endpoint) = live_on_a_full_line().await;
        for n in 0..8 {
            endpoint.push(TestWire::encode_in(&TestIn::Notice(format!("n{n}"))));
        }
        endpoint.push(Frame::Keepalive);
        settle().await;
        assert!(h.peer.held_bytes() > 0, "순서 칸이 붙들고 있다");

        for _ in 0..3 {
            h.clock.advance(Duration::from_secs(20));
            settle().await;
            assert!(
                endpoint.drain().contains(&ClientMsg::Ping),
                "★붙든 동안에도 우리 ping 은 나간다★ — 안 나가면 상대가 우리를 침묵으로 끊는다"
            );
        }
        assert_eq!(h.peer.peer_state(), PeerState::Live);

        h.peer.close();
        settle_until("붙든 채로도 닫기가 들린다", || {
            h.peer.peer_state() == PeerState::Closed
        })
        .await;
        let items = drain_until_quiet(&mut h).await;
        assert_eq!(
            labels(&items).last().map(String::as_str),
            Some("Closed:1:Explicit")
        );
    }

    /// Even if released after holding for over 50 s, it is not `Idle`, and the silence decision counts again from the moment of release.
    async fn idle_restarts_from_the_release(mut h: Harness, what: &str) {
        h.clock.advance(Duration::from_secs(60));
        settle().await;
        assert_eq!(
            h.peer.peer_state(),
            PeerState::Live,
            "{what}: ★막은 것은 우리 쪽이다★ — 붙든 시간을 상대의 침묵으로 세면 안 된다"
        );

        drain_until_quiet(&mut h).await;
        h.clock.advance(Duration::from_secs(49));
        settle().await;
        assert_eq!(
            h.peer.peer_state(),
            PeerState::Live,
            "{what}: ★풀린 순간이 `last_rx` 다★ — 그 앞의 마지막 수신부터 세면 곧바로 끊긴다"
        );
        h.clock.advance(Duration::from_secs(1));
        settle().await;
        assert_eq!(
            h.disconnect_cause(),
            Some(DisconnectCause::Idle),
            "{what}: 풀린 뒤에는 침묵 판정이 다시 돈다"
        );
    }

    #[tokio::test]
    async fn holding_past_the_idle_timeout_on_a_full_line_is_not_silence() {
        let (h, endpoint) = live_on_a_full_line().await;
        endpoint.push(TestWire::encode_in(&TestIn::Notice("held".into())));
        settle().await;
        idle_restarts_from_the_release(h, "순서 칸").await;
    }

    #[tokio::test]
    async fn holding_past_the_idle_timeout_over_the_byte_budget_is_not_silence() {
        let budget = 100;
        let mut h = build_budgeted(
            Policy::default(),
            Arc::new(ImmediateHandshake),
            4096,
            budget,
        );
        settle_until("첫 연결", || h.peer.peer_state() == PeerState::Live).await;
        let endpoint = h.net.accept().unwrap();
        h.drain();
        // ★Do not put a liveness check after it★ — if that were read right at release and rewound `last_rx`, the release's rewind
        //   could not be measured.
        endpoint.push(blob(0, 2 * budget));
        settle().await;
        assert!(
            h.peer.held_bytes() >= budget,
            "줄에 든 한 장이 예산을 넘겼다"
        );
        idle_restarts_from_the_release(h, "바이트 예산").await;
    }

    // ★Make the release slip into a timer turn (`Turn::Timer`) that is followed by a progress check★ — `on_timer` skipped the silence decision
    //   by the 「붙듦」 (held) at its start, but if the wake-up time were re-measured by the backpressure at that place, it would be set by the stale `last_rx`'s silence deadline (already
    //   passed) and the progress check (`debug_assert`) would panic the supervisor.
    //   To get to that turn, the ping time must pass while the supervisor is outside `select` — we stand it on a blocked notification write. When that write
    //   is released the supervisor goes through to that turn's ping without yielding, so we block only the ping and let the receiving side drain inside it.
    #[tokio::test]
    async fn a_release_inside_a_timer_round_does_not_trip_the_progress_check() {
        let budget = 100;
        let mut h = build_budgeted(
            Policy::default(),
            Arc::new(ImmediateHandshake),
            4096,
            budget,
        );
        settle_until("첫 연결", || h.peer.peer_state() == PeerState::Live).await;
        let endpoint = h.net.accept().unwrap();
        endpoint.push(blob(0, 2 * budget));
        settle().await;
        assert!(h.peer.held_bytes() >= budget, "예산만으로 붙든다");
        // Passes the silence deadline (50 s) — it does not cut because it is held, and the ping goes out every 20 s.
        for _ in 0..2 {
            h.clock.advance(Duration::from_secs(20));
            settle().await;
        }
        h.clock.advance(Duration::from_secs(16));
        settle().await;
        assert_eq!(h.peer.peer_state(), PeerState::Live);

        endpoint.stall_writes(true);
        h.peer.notify(TestOut::Notice("slow".into())).unwrap();
        settle().await;
        // Passes the next ping time (60 s) while standing on a write — within the write deadline (5 s).
        h.clock.advance(Duration::from_millis(4500));
        settle().await;
        endpoint.stall_pings(true);
        endpoint.stall_writes(false);
        settle().await;
        assert!(!h.task.is_finished(), "타이머 바퀴의 ping 에서 기다린다");
        h.drain();
        endpoint.stall_pings(false);
        settle().await;
        assert!(
            !h.task.is_finished(),
            "★붙든 채 들어간 타이머 바퀴 안에서 풀려도 진도 확인이 안 터진다★"
        );
        assert_eq!(h.peer.peer_state(), PeerState::Live);
        assert!(endpoint.drain().contains(&ClientMsg::Ping));
    }

    // ── 3-0 U1c: resume window · backpressure ──

    /// A stream piece whose payload is exactly `len` bytes (key 1 · incarnation 1).
    fn sized_chunk(seq: u64, len: usize) -> Frame {
        let head = format!("chk|1|1|{seq}|");
        assert!(len >= head.len(), "{len} 바이트로는 머리도 못 싣는다");
        Frame::Text(format!("{head}{}", "x".repeat(len - head.len())))
    }

    fn chunk_seqs(items: &[Incoming<TestWire>]) -> Vec<u64> {
        items
            .iter()
            .filter_map(|item| match item.as_message().map(|a| &a.msg) {
                Some(TestIn::Chunk { seq, .. }) => Some(*seq),
                _ => None,
            })
            .collect()
    }

    fn truncations(items: &[Incoming<TestWire>]) -> Vec<u64> {
        items
            .iter()
            .filter_map(|item| match item.as_event() {
                Some(TransportEvent::StreamTruncated { oldest_valid, .. }) => Some(*oldest_valid),
                _ => None,
            })
            .collect()
    }

    /// A supervisor that received piece 1 and took it out — from the next piece on, the hole decision stands.
    async fn streaming(budget: usize, inbound_cap: usize) -> (Harness, Arc<MemoryEndpoint>) {
        let mut h = build_budgeted(
            Policy::default(),
            Arc::new(ImmediateHandshake),
            inbound_cap,
            budget,
        );
        settle_until("첫 연결", || h.peer.peer_state() == PeerState::Live).await;
        let endpoint = h.net.accept().unwrap();
        endpoint.push(TestWire::chunk(1, 1, 1));
        assert_eq!(chunk_seqs(&drain_until_quiet(&mut h).await), vec![1]);
        (h, endpoint)
    }

    // ★Grow the held pieces by the window cap★ — the window cap is at least the read share, so if that share went into the read gate the gate would close,
    //   the replay would stay in the channel and could not fill the hole, and replay arriving late after the window closes on the deadline is dropped as a number already passed.
    #[tokio::test]
    async fn a_held_chunk_never_stops_the_reads_that_fill_its_gap() {
        let budget = 100;
        let (read_share, window_cap) = split(budget);
        assert!(window_cap >= read_share);
        let (mut h, endpoint) = streaming(budget, 4096).await;
        endpoint.push(sized_chunk(3, window_cap));
        settle().await;
        assert_eq!(
            endpoint.frames(),
            vec![Frame::Text("res|1|1".into())],
            "구멍을 보고 재요청을 냈다"
        );
        assert_eq!(
            h.peer.window_bytes(),
            window_cap,
            "붙든 조각은 창 몫에 선다"
        );

        endpoint.push(TestWire::chunk(1, 1, 2));
        endpoint.push(TestWire::chunk(1, 1, 3));
        settle().await;
        assert_eq!(
            endpoint.unread(),
            0,
            "★붙든 조각이 창 상한만큼이어도 재생분은 읽힌다★"
        );
        let items = h.drain();
        assert_eq!(
            chunk_seqs(&items),
            vec![2, 3],
            "★구멍이 메워져 순서대로 온다★"
        );
        assert_eq!(truncations(&items), Vec::<u64>::new(), "잘림이 아니다");
        assert_eq!(h.peer.window_bytes(), 0, "답이 와 붙든 것을 버렸다");
    }

    // ★The receiving side stops after the window opens★ — fill line length 1 with one notification so the hole event stands in the ordering area, and meanwhile the frame
    //   arm is pulled and the replay stays in the read-ahead queue. If the window deadline were counted as is, the window would close in between and the replay would be dropped.
    #[tokio::test]
    async fn a_stalled_consumer_does_not_run_the_resume_window_out() {
        let (mut h, endpoint) = streaming(BYTE_BUDGET, 1).await;
        endpoint.push(TestWire::encode_in(&TestIn::Notice("fill".into())));
        endpoint.push(TestWire::chunk(1, 1, 3));
        settle().await;
        assert_eq!(endpoint.frames(), vec![Frame::Text("res|1|1".into())]);
        endpoint.push(TestWire::chunk(1, 1, 2));
        endpoint.push(TestWire::chunk(1, 1, 3));
        settle().await;
        assert!(
            h.peer.held_bytes() > h.peer.window_bytes(),
            "재생분이 앞잡이 줄에 있다"
        );

        // Passes the window deadline (2 s) and the ping time (20 s) too, letting the timer run once while held.
        h.clock.advance(Duration::from_secs(3));
        settle().await;
        h.clock.advance(Duration::from_secs(20));
        settle().await;
        assert!(
            endpoint.drain().contains(&ClientMsg::Ping),
            "붙든 채 타이머가 돌았다"
        );
        assert_eq!(h.peer.peer_state(), PeerState::Live);

        let items = drain_until_quiet(&mut h).await;
        assert_eq!(
            truncations(&items),
            Vec::<u64>::new(),
            "★막아 둔 것은 우리 쪽이다★ — 붙든 시간을 창 시한으로 세면 메울 수 있던 구멍을 잘림으로 접는다"
        );
        assert_eq!(
            chunk_seqs(&items),
            vec![2, 3],
            "★풀린 뒤 재생분이 구멍을 메운다★"
        );
    }

    // Budget 100 = window cap 50 — two 30s exceed it at the second.
    #[tokio::test]
    async fn holding_past_the_window_cap_takes_the_truncation_path() {
        let budget = 100;
        let (mut h, endpoint) = streaming(budget, 4096).await;
        endpoint.push(sized_chunk(3, 30));
        settle().await;
        assert_eq!(h.peer.window_bytes(), 30);

        endpoint.push(sized_chunk(4, 30));
        settle().await;
        assert_eq!(
            (h.peer.held_bytes(), h.peer.window_bytes()),
            (60, 0),
            "★창에서 나온 조각의 몫은 읽기 문으로 돌아간다★ — 창 몫에 남으면 받는 쪽이 느린 것만으로 다음 구멍을 \
             못 붙든다"
        );
        let items = h.drain();
        assert_eq!(
            truncations(&items),
            vec![3],
            "★창 상한을 넘기면 말없이 버리지 않고 잘림으로 신고한다★ — 붙든 첫 번호가 하한이다"
        );
        assert_eq!(
            chunk_seqs(&items),
            vec![3, 4],
            "붙든 것을 순서대로 내보낸다"
        );
        assert_eq!(h.peer.held_bytes(), 0);
    }

    #[tokio::test]
    async fn held_bytes_counts_a_held_chunk_until_its_window_resolves() {
        let (mut h, endpoint) = streaming(BYTE_BUDGET, 4096).await;
        let held = TestWire::chunk(1, 1, 3).payload_len();
        endpoint.push(TestWire::chunk(1, 1, 3));
        settle().await;
        h.drain();
        assert_eq!(
            (h.peer.held_bytes(), h.peer.window_bytes()),
            (held, held),
            "★창이 붙든 조각도 감독이 쥔 바이트다★"
        );

        let answer = TestWire::chunk(1, 1, 2).payload_len();
        endpoint.push(TestWire::chunk(1, 1, 2));
        settle().await;
        assert_eq!(
            (h.peer.held_bytes(), h.peer.window_bytes()),
            (answer, 0),
            "답이 와 창이 닫히면 붙든 몫이 빠지고 올라간 답만 남는다"
        );
        h.drain();
        assert_eq!(h.peer.held_bytes(), 0);
    }

    // ── 3-0 U1c: salvage the read-ahead queue when dropping the channel ──
    //
    // ★Set up with line length 1★ — n0 stands in the ordering area and n1 · n2 stay in the read-ahead queue with the frame arm pulled.

    async fn three_notices_held(h: &Harness, endpoint: &MemoryEndpoint) {
        for n in 0..3 {
            endpoint.push(TestWire::encode_in(&TestIn::Notice(format!("n{n}"))));
        }
        settle().await;
        assert_eq!(
            h.peer.held_bytes(),
            3 * "not|n0".len(),
            "n0 은 순서 칸에, n1 · n2 는 앞잡이 줄에 있다"
        );
        assert_eq!(endpoint.unread(), 0);
    }

    // ★Fill the read-ahead queue completely (2 slots)★ — while the read task waits for the next slot there must be no frame in hand, so that salvaged plus
    //   what stayed in the channel adds up to everything sent.
    #[tokio::test]
    async fn reconnecting_hands_over_what_was_already_read_before_the_new_link() {
        let mut h = build_with(
            Policy {
                frame_buffer: 2,
                ..Policy::default()
            },
            Arc::new(ImmediateHandshake),
            1,
        );
        settle_until("첫 연결", || h.peer.peer_state() == PeerState::Live).await;
        let endpoint = h.net.accept().unwrap();
        for n in 0..5 {
            endpoint.push(TestWire::encode_in(&TestIn::Notice(format!("n{n}"))));
        }
        settle().await;
        assert_eq!(
            endpoint.unread(),
            2,
            "n0 은 순서 칸에, n1 · n2 는 앞잡이 줄에"
        );
        h.peer.reconnect_now();
        settle_until("새 통로", || h.net.dials() == 2).await;

        let items = drain_until_quiet(&mut h).await;
        assert_eq!(
            labels(&items),
            vec![
                "Connected:1",
                "msg:n0",
                "msg:n1",
                "msg:n2",
                "Disconnected:1:Explicit",
                "Connected:2",
            ],
            "★다시 걸어도 이미 읽은 것은 버리지 않고 옛 세대로 끊김 앞에 싣는다★"
        );
        assert_eq!(
            endpoint.unread(),
            2,
            "n3 · n4 는 옛 통로에 남았다 — 읽고서 버린 것은 없다"
        );
        assert_eq!(h.peer.held_bytes(), 0);
    }

    #[tokio::test]
    async fn a_write_failure_hands_over_what_was_already_read_before_the_disconnect() {
        let (mut h, endpoint) = live_on_a_full_line().await;
        three_notices_held(&h, &endpoint).await;
        endpoint.stall_writes(true);
        h.peer.notify(TestOut::Notice("stuck".into())).unwrap();
        settle().await;
        h.clock.advance(Policy::default().write_deadline);
        settle().await;
        // The close of the channel being dropped is caught by the same stall — release it to finish teardown (`stall_writes` rustdoc).
        endpoint.stall_writes(false);

        let items = drain_until_quiet(&mut h).await;
        assert_eq!(
            labels(&items),
            vec![
                "Connected:1",
                "msg:n0",
                "msg:n1",
                "msg:n2",
                "Disconnected:1:WriteDeadline",
                "Dropped",
            ],
            "★쓰기가 실패해도 이미 읽은 것은 끊김 앞에 온다★ — 못 나간 알림의 신고는 백오프가 낸다"
        );
    }

    // ★A re-attached channel uses the same count★ — it does not read the new channel until the share the old channel left drops below the read share.
    #[tokio::test]
    async fn a_new_link_waits_for_what_the_old_link_left_to_drop_below_the_budget() {
        let budget = 100;
        let mut h = build_budgeted(
            Policy::default(),
            Arc::new(ImmediateHandshake),
            4096,
            budget,
        );
        settle_until("첫 연결", || h.peer.peer_state() == PeerState::Live).await;
        let first = h.net.accept().unwrap();
        first.push(blob(0, 2 * budget));
        settle().await;
        assert_eq!(h.peer.held_bytes(), 2 * budget, "줄에 남겨 둔다");

        h.peer.reconnect_now();
        settle_until("다시 붙음", || h.net.dials() == 2).await;
        settle_until("운영", || h.peer.peer_state() == PeerState::Live).await;
        let second = h.net.accept().unwrap();
        second.push(blob(1, 10));
        settle().await;
        assert_eq!(
            second.unread(),
            1,
            "★옛 통로의 몫이 읽기 몫 이상이면 새 통로를 안 읽는다★"
        );

        let items = drain_until_quiet(&mut h).await;
        assert_eq!(blob_tags(&items), vec![0, 1]);
        assert_eq!(second.unread(), 0, "옛 몫이 빠지자 새 통로를 읽었다");
        assert_eq!(h.peer.held_bytes(), 0);
    }

    // ★Even if it redials with a reply in the read-ahead queue, the waiting caller receives it★ — salvaging comes before the machine's waiter wake-up.
    #[tokio::test]
    async fn a_reply_already_read_reaches_its_caller_when_reconnecting() {
        let (mut h, endpoint) = live_on_a_full_line().await;
        let asked = spawn_request(&h.peer, 7);
        settle().await;
        endpoint.push(TestWire::encode_in(&TestIn::Notice("held".into())));
        endpoint.push(TestWire::reply(7));
        settle().await;
        assert_eq!(endpoint.unread(), 0);
        assert!(!asked.is_finished(), "답장은 앞잡이 줄에 있다");

        h.peer.reconnect_now();
        assert_eq!(
            joined(asked).await,
            Ok(TestIn::Reply {
                tag: 7,
                body: "ok".into(),
            }),
            "★이미 읽은 답장은 끊김으로 실패하지 않는다★"
        );
        drain_until_quiet(&mut h).await;
    }

    // ── 3-0 U1c: one budget covers both the read share and the window cap (B + M) ──

    // ★Fill the read share too while the window is full★ — if the window cap were the budget (two counts), it would hold four (96) in the window and hold two more (98)
    //   by reading, exceeding B + M (149). If the window cap is half, the window closes as a truncation at the third piece and that share closes the read gate.
    #[tokio::test]
    async fn holding_a_window_full_and_a_read_share_full_still_fits_the_budget_plus_one_message() {
        let budget = 100;
        let message = 49;
        let (mut h, endpoint) = streaming(budget, 1).await;
        let pushes: Vec<Frame> = (3..=6)
            .map(|seq| sized_chunk(seq, 24))
            .chain((0..3).map(|tag| blob(tag, message)))
            .collect();
        for frame in pushes {
            endpoint.push(frame);
            settle().await;
            assert!(
                h.peer.held_bytes() <= budget + message,
                "{} 바이트를 쥐었다 — 셈 전체가 B + M 을 넘었다",
                h.peer.held_bytes()
            );
            assert!(h.peer.window_bytes() <= split(budget).1);
        }

        let items = drain_until_quiet(&mut h).await;
        assert_eq!(chunk_seqs(&items), vec![3, 4, 5, 6]);
        assert_eq!(truncations(&items), vec![3]);
        assert_eq!(blob_tags(&items), vec![0, 1, 2], "★손실 0 · 순서 그대로★");
        assert_eq!(h.peer.held_bytes(), 0);
    }

    // ★Only a supervisor that has seen a stream piece halves the read gate★ — send the same three frames (30 · 30 · 30) at budget 100. A supervisor that did not use
    //   streams reads the third too (60 < 100), and a supervisor that saw one piece reaches the read share (50) at the second and leaves the third in the channel.
    #[tokio::test]
    async fn only_a_peer_that_has_seen_a_stream_chunk_splits_its_read_gate() {
        let budget = 100;
        for streams in [false, true] {
            let mut h = build_budgeted(
                Policy::default(),
                Arc::new(ImmediateHandshake),
                4096,
                budget,
            );
            settle_until("첫 연결", || h.peer.peer_state() == PeerState::Live).await;
            let endpoint = h.net.accept().unwrap();
            if streams {
                endpoint.push(TestWire::chunk(1, 1, 1));
            }
            drain_until_quiet(&mut h).await;
            for tag in 0..3u8 {
                endpoint.push(blob(tag, 30));
            }
            settle().await;
            assert_eq!(
                endpoint.unread(),
                usize::from(streams),
                "스트림 조각을 봤나 = {streams}"
            );
            assert_eq!(
                blob_tags(&drain_until_quiet(&mut h).await),
                vec![0, 1, 2],
                "★손실 0 · 순서 그대로★"
            );
        }
    }

    // ── 3-0 U1c: progress · clock · generation · teardown of held pieces ──

    // ★The piece that hit the byte limit is not the largest number among those held★ — if progress is set to that piece's number, replay 4 · 5 goes up
    //   again after 5 (order reversed · overlap).
    #[tokio::test]
    async fn a_descending_arrival_past_the_window_cap_never_rewinds_the_stream() {
        let budget = 100;
        let (mut h, endpoint) = streaming(budget, 4096).await;
        endpoint.push(sized_chunk(5, 30));
        endpoint.push(sized_chunk(3, 30));
        settle().await;
        for seq in 4..=6 {
            endpoint.push(TestWire::chunk(1, 1, seq));
        }
        let items = drain_until_quiet(&mut h).await;
        assert_eq!(truncations(&items), vec![3]);
        assert_eq!(
            chunk_seqs(&items),
            vec![3, 5, 6],
            "★늘 오름차순 · 겹침 없음★"
        );
    }

    // ★The hold starts while waiting on a write★ — the supervisor can come to the start of the turn and notice the hold only after that write ends. If we count from when
    //   it was noticed, the 3 s standing on the write counts as the window deadline (2 s), the window closes as soon as it releases, and the replay is dropped as a number already passed.
    #[tokio::test]
    async fn backpressure_that_begins_during_a_stalled_write_still_stops_the_resume_clock() {
        let budget = 100;
        let (mut h, endpoint) = streaming(budget, 4096).await;
        endpoint.push(TestWire::chunk(1, 1, 3));
        settle().await;
        assert_eq!(endpoint.frames(), vec![Frame::Text("res|1|1".into())]);

        endpoint.stall_writes(true);
        h.peer.notify(TestOut::Notice("slow".into())).unwrap();
        settle().await;
        endpoint.push(blob(0, 2 * budget));
        endpoint.push(TestWire::chunk(1, 1, 2));
        endpoint.push(TestWire::chunk(1, 1, 3));
        settle().await;
        assert_eq!(
            endpoint.unread(),
            2,
            "쓰기에 선 사이 읽기 문이 닫혀 재생분이 통로에 남았다"
        );
        let resume_timeout = Policy::default().resume_timeout;
        let stalled = resume_timeout + Duration::from_secs(1);
        assert!(stalled < Policy::default().write_deadline);
        h.clock.advance(stalled);
        settle().await;
        endpoint.stall_writes(false);

        let items = drain_until_quiet(&mut h).await;
        assert_eq!(
            truncations(&items),
            Vec::<u64>::new(),
            "★붙듦은 시작된 때부터 센다★ — 알아챈 때부터 세면 쓰기를 기다린 시간이 창 시한을 다 먹는다"
        );
        assert_eq!(
            chunk_seqs(&items),
            vec![2, 3],
            "★풀린 뒤 재생분이 구멍을 메운다★"
        );
        assert_eq!(blob_tags(&items), vec![0]);
        assert_eq!(h.peer.peer_state(), PeerState::Live);
    }

    // The ordering-area edition of the test above — at the place where the hole event stands in the ordering area because of a full line, the re-request write is blocked. We must count from when the ordering area
    //   started holding so that the time standing on the write is not counted as the window deadline.
    #[tokio::test]
    async fn a_held_line_that_begins_before_a_stalled_resume_write_still_stops_the_resume_clock() {
        let (mut h, endpoint) = streaming(BYTE_BUDGET, 1).await;
        endpoint.push(TestWire::encode_in(&TestIn::Notice("fill".into())));
        settle().await;
        endpoint.stall_writes(true);
        endpoint.push(TestWire::chunk(1, 1, 3));
        settle().await;
        endpoint.push(TestWire::chunk(1, 1, 2));
        endpoint.push(TestWire::chunk(1, 1, 3));
        let stalled = Policy::default().resume_timeout + Duration::from_secs(1);
        h.clock.advance(stalled);
        settle().await;
        endpoint.stall_writes(false);
        settle().await;
        assert_eq!(endpoint.frames(), vec![Frame::Text("res|1|1".into())]);

        let items = drain_until_quiet(&mut h).await;
        assert_eq!(truncations(&items), Vec::<u64>::new());
        assert_eq!(chunk_seqs(&items), vec![2, 3]);
    }

    // The loss-report edition of the test above — while standing on the write, the receiving side empties the line and the outbound queue overflows. The report at the start of the turn after the write
    //   loads even the hole event into the line and empties the ordering area — if the hold is not seen before that report, the hold that started mid-write is lost,
    //   and the deadline comes before the read-ahead queue's replay and closes the window.
    #[tokio::test]
    async fn a_drop_report_that_empties_the_held_line_does_not_hide_the_pause() {
        let mut h = build_budgeted(
            Policy {
                outbound_queue: 1,
                full_queue: QueueFullPolicy::DropAndReport,
                ..Policy::default()
            },
            Arc::new(ImmediateHandshake),
            2,
            BYTE_BUDGET,
        );
        settle_until("첫 연결", || h.peer.peer_state() == PeerState::Live).await;
        let endpoint = h.net.accept().unwrap();
        endpoint.push(TestWire::chunk(1, 1, 1));
        assert_eq!(chunk_seqs(&drain_until_quiet(&mut h).await), vec![1]);
        for body in ["fill0", "fill1"] {
            endpoint.push(TestWire::encode_in(&TestIn::Notice(body.into())));
        }
        settle().await;
        endpoint.stall_writes(true);
        endpoint.push(TestWire::chunk(1, 1, 3));
        settle().await;
        endpoint.push(TestWire::chunk(1, 1, 2));
        endpoint.push(TestWire::chunk(1, 1, 3));
        h.peer.notify(TestOut::Notice("queued".into())).unwrap();
        assert_eq!(
            h.peer.notify(TestOut::Notice("lost".into())),
            Err(SendError::QueueFull)
        );
        assert_eq!(
            labels(&h.drain()),
            vec!["msg:fill0", "msg:fill1"],
            "쓰기에 선 사이 받는 쪽이 줄을 비웠다 — 구멍 사건은 아직 순서 칸에 있다"
        );
        let stalled = Policy::default().resume_timeout + Duration::from_secs(1);
        assert!(stalled < Policy::default().write_deadline);
        h.clock.advance(stalled);
        settle().await;
        endpoint.stall_writes(false);

        let items = drain_until_quiet(&mut h).await;
        assert_eq!(truncations(&items), Vec::<u64>::new());
        assert_eq!(chunk_seqs(&items), vec![2, 3]);
        assert!(labels(&items).contains(&"Dropped".to_string()));
    }

    // ★The read gate closes while waiting on a write and reopens before that write ends★ — the supervisor sees only the open gate at the start of the turn
    //   and never sees the hold. If that time is not pushed, the 3 s standing on the write exceeds the window deadline (2 s), the deadline comes before the read-ahead queue's
    //   replay and closes the window, and replay 2 is dropped as a number already passed.
    #[tokio::test]
    async fn a_gate_that_shuts_and_reopens_inside_a_stalled_write_still_stops_the_resume_clock() {
        let budget = 100;
        let (mut h, endpoint) = streaming(budget, 4096).await;
        endpoint.push(TestWire::chunk(1, 1, 3));
        endpoint.push(blob(0, 40));
        settle().await;
        assert_eq!(endpoint.frames(), vec![Frame::Text("res|1|1".into())]);

        endpoint.stall_writes(true);
        h.peer.notify(TestOut::Notice("slow".into())).unwrap();
        settle().await;
        endpoint.push(blob(1, 20));
        endpoint.push(TestWire::chunk(1, 1, 2));
        endpoint.push(TestWire::chunk(1, 1, 3));
        settle().await;
        assert_eq!(
            endpoint.unread(),
            2,
            "쓰기에 선 사이 읽기 문이 닫혀 재생분이 통로에 남았다"
        );
        let stalled = Policy::default().resume_timeout + Duration::from_secs(1);
        assert!(stalled < Policy::default().write_deadline);
        h.clock.advance(stalled);
        settle().await;
        assert_eq!(blob_tags(&h.drain()), vec![0]);
        settle().await;
        assert_eq!(
            endpoint.unread(),
            0,
            "받는 쪽이 비워 읽기 문이 다시 열렸다 — 쓰기는 아직 서 있다"
        );
        endpoint.stall_writes(false);

        let items = drain_until_quiet(&mut h).await;
        assert_eq!(
            truncations(&items),
            Vec::<u64>::new(),
            "★끝내 못 본 붙듦도 창 시한에서 뺀다★"
        );
        assert_eq!(chunk_seqs(&items), vec![2, 3], "★재생분이 구멍을 메운다★");
        assert_eq!(blob_tags(&items), vec![1]);
        assert_eq!(h.peer.peer_state(), PeerState::Live);
    }

    // ── 3-0 U1c: counting the held time (`track_pause`) — place the fields by hand on a supervisor that was not run ──

    fn idle_supervisor() -> (Supervisor<TestWire>, ManualClock) {
        let net = MemoryNetwork::new();
        let clock = ManualClock::new();
        let (inbound, _) = mpsc::channel(1);
        let (_, supervisor) = new_peer(PeerConfig {
            id: PeerId::new("d1"),
            origin: Origin::Dial {
                addr: Address::new("mem://d1"),
                dialer: net.dialer(),
            },
            wire: Arc::new(TestWire),
            clock: clock.handle(),
            handshake: Arc::new(ImmediateHandshake),
            policy: Policy::default(),
            inbound,
            byte_budget: 100,
            generations: Arc::new(AtomicU64::new(0)),
        });
        (supervisor, clock)
    }

    /// A window that received 1 and opened a hole at 3 — written as requested now. What it returns = that window's deadline.
    fn open_window(sup: &mut Supervisor<TestWire>) -> Instant {
        let now = sup.clock.now();
        for seq in [1, 3] {
            let msg = sup.wire.decode(TestWire::chunk(1, 1, seq)).unwrap();
            let mark = sup.wire.stream_mark(&msg).unwrap();
            let held = Received {
                msg,
                charge: Charge::none(),
                generation: Generation(1),
            };
            let _ = sup.streams.observe_capped(&mark, held, now, false);
        }
        sup.streams.next_resume_deadline().expect("창이 열렸다")
    }

    /// Closes the read gate for `secs` seconds and then opens it — budget 100 · streams unused, so it closes at 100 bytes.
    fn shut_for(sup: &Supervisor<TestWire>, clock: &ManualClock, secs: u64) {
        let gate = sup.ledger.charge(100);
        clock.advance(Duration::from_secs(secs));
        drop(gate);
    }

    #[test]
    fn unseen_shut_intervals_shift_the_window_by_exactly_their_length() {
        let (mut sup, clock) = idle_supervisor();
        let deadline = open_window(&mut sup);
        clock.advance(Duration::from_secs(1));
        shut_for(&sup, &clock, 2);
        clock.advance(Duration::from_secs(1));
        shut_for(&sup, &clock, 3);
        sup.track_pause(clock.now());
        assert!(!sup.paused);
        assert_eq!(
            sup.streams.next_resume_deadline(),
            Some(deadline + Duration::from_secs(5)),
            "★두 구간을 다 민다★ — 그 사이 열려 있던 1 s 는 안 민다"
        );
        assert_eq!(sup.last_rx, clock.now(), "못 본 붙듦이 풀린 것도 풀림이다");
    }

    // ★If an unseen span straddles the ordering area's hold, treat it as held from that first span★ — if we looked only at when the ordering area started holding,
    //   we would lose the 1 s it was closed before that, and if we pushed the two separately we would push the overlapping 1 s twice.
    #[test]
    fn an_unseen_interval_reaching_into_a_held_line_starts_the_pause_at_its_own_start() {
        let (mut sup, clock) = idle_supervisor();
        let deadline = open_window(&mut sup);
        clock.advance(Duration::from_secs(1));
        let gate = sup.ledger.charge(100);
        clock.advance(Duration::from_secs(1));
        sup.order.push_back(Charged::bare(Incoming::Event(
            TransportEvent::AmbiguousRole {
                peer: PeerId::new("d1"),
            },
        )));
        sup.order_since = clock.now();
        clock.advance(Duration::from_secs(1));
        drop(gate);
        clock.advance(Duration::from_secs(1));
        sup.track_pause(clock.now());
        assert!(sup.paused);

        sup.order.clear();
        clock.advance(Duration::from_secs(1));
        sup.track_pause(clock.now());
        assert_eq!(
            sup.streams.next_resume_deadline(),
            Some(deadline + Duration::from_secs(4)),
            "읽기 문이 닫힌 때(1 s)부터 풀림을 알아챈 때(5 s)까지다"
        );
    }

    // ★If an unseen span ended before the current hold, push it separately★ — if we pulled the current hold's start back to that span's start,
    //   we would push even the time it was open in between.
    #[test]
    fn an_unseen_interval_that_ended_before_the_current_pause_is_shifted_on_its_own() {
        let (mut sup, clock) = idle_supervisor();
        let deadline = open_window(&mut sup);
        clock.advance(Duration::from_secs(1));
        shut_for(&sup, &clock, 2);
        clock.advance(Duration::from_secs(1));
        let gate = sup.ledger.charge(100);
        clock.advance(Duration::from_secs(1));
        sup.track_pause(clock.now());
        assert!(sup.paused);

        drop(gate);
        clock.advance(Duration::from_secs(1));
        sup.track_pause(clock.now());
        assert_eq!(
            sup.streams.next_resume_deadline(),
            Some(deadline + Duration::from_secs(4)),
            "못 본 2 s + 지금 붙듦(4 s 에 닫혀 6 s 에 알아챘다) 2 s — 그 사이 열린 1 s 는 안 민다"
        );
    }

    // ★A span closed and opened while we knew it was held is not pushed again even if the count hands it over separately★ — that span is all after
    //   the hold started, so it is already inside the time pushed at release.
    #[test]
    fn intervals_inside_a_known_pause_are_not_shifted_twice() {
        let (mut sup, clock) = idle_supervisor();
        let deadline = open_window(&mut sup);
        clock.advance(Duration::from_secs(1));
        let gate = sup.ledger.charge(100);
        sup.track_pause(clock.now());
        assert!(sup.paused);
        clock.advance(Duration::from_secs(1));
        drop(gate);
        clock.advance(Duration::from_secs(1));
        shut_for(&sup, &clock, 1);
        clock.advance(Duration::from_secs(1));
        sup.track_pause(clock.now());
        assert!(!sup.paused);
        assert_eq!(
            sup.streams.next_resume_deadline(),
            Some(deadline + Duration::from_secs(4)),
            "붙듦이 시작된 1 s 부터 풀림을 알아챈 5 s 까지 한 번만"
        );
    }

    // ★Even under a policy that cuts on decode failure, salvaging hands over what is after it too★ — during operation what comes after that one frame is handed over by
    //   salvaging, so if it stopped mid-salvage, the same read-ahead queue would be handed over or dropped depending on where it was cut.
    #[tokio::test]
    async fn salvaging_carries_on_past_a_frame_it_cannot_decode() {
        let mut h = build_with(
            Policy {
                decode_failure: DecodeFailurePolicy::Disconnect,
                ..Policy::default()
            },
            Arc::new(ImmediateHandshake),
            1,
        );
        settle_until("첫 연결", || h.peer.peer_state() == PeerState::Live).await;
        let endpoint = h.net.accept().unwrap();
        endpoint.push(TestWire::encode_in(&TestIn::Notice("n0".into())));
        endpoint.push(Frame::Text("garbage".into()));
        endpoint.push(TestWire::encode_in(&TestIn::Notice("n1".into())));
        settle().await;
        assert_eq!(
            endpoint.unread(),
            0,
            "n0 은 순서 칸에, 나머지는 앞잡이 줄에"
        );
        h.peer.reconnect_now();
        settle_until("새 통로", || h.net.dials() == 2).await;

        let items = drain_until_quiet(&mut h).await;
        assert_eq!(
            labels(&items),
            vec![
                "Connected:1",
                "msg:n0",
                "DecodeFailed",
                "msg:n1",
                "Disconnected:1:Explicit",
                "Connected:2",
            ]
        );
    }

    #[tokio::test]
    async fn a_chunk_held_across_a_reconnect_keeps_the_generation_that_carried_it() {
        let (mut h, endpoint) = streaming(BYTE_BUDGET, 4096).await;
        endpoint.push(TestWire::chunk(1, 1, 3));
        settle().await;
        h.peer.reconnect_now();
        settle_until("다시 붙음", || h.net.dials() == 2).await;
        settle_until("운영", || h.peer.peer_state() == PeerState::Live).await;
        let second = h.net.accept().unwrap();
        settle().await;
        assert_eq!(
            second.frames(),
            vec![Frame::Text("res|1|1".into())],
            "창이 살아남아 새 통로에서 다시 청했다"
        );

        h.clock.advance(Policy::default().resume_timeout);
        let items = drain_until_quiet(&mut h).await;
        let held: Vec<(u64, u64)> = items
            .iter()
            .filter_map(Incoming::as_message)
            .filter_map(|arrival| match &arrival.msg {
                TestIn::Chunk { seq, .. } => Some((*seq, arrival.generation.0)),
                _ => None,
            })
            .collect();
        assert_eq!(truncations(&items), vec![3], "둘째 통로에서 창이 닫혔다");
        assert_eq!(
            held,
            vec![(3, 1)],
            "★실어 온 연결의 세대다★ — 창이 닫힌 연결(2)의 세대를 찍으면 `Arrival::generation` 계약이 깨진다"
        );
    }

    // ★Teardown folds the open window before dropping the ledger★ — if not, held pieces vanish with neither delivery nor truncation report.
    #[tokio::test]
    async fn closing_hands_over_a_held_chunk_as_a_truncation_before_closed() {
        let (mut h, endpoint) = streaming(BYTE_BUDGET, 4096).await;
        endpoint.push(TestWire::chunk(1, 1, 3));
        settle().await;
        h.peer.close();
        settle_until("접힘", || h.peer.peer_state() == PeerState::Closed).await;

        let items = drain_until_quiet(&mut h).await;
        let seen: Vec<String> = items
            .iter()
            .map(|item| match item.as_message().map(|a| &a.msg) {
                Some(TestIn::Chunk { seq, .. }) => format!("chunk:{seq}"),
                _ => label(item),
            })
            .collect();
        assert_eq!(
            seen,
            vec![
                "StreamGap",
                "Disconnected:1:Explicit",
                "StreamTruncated",
                "chunk:3",
                "Closed:1:Explicit",
            ]
        );
        assert_eq!(truncations(&items), vec![3], "붙든 첫 번호가 하한이다");
        assert_eq!(h.peer.held_bytes(), 0);
    }

    // ── 3-0 U3: pre-close flush ──
    //
    // ★Put the close code outside the crate's reserved range★ — if it equals the default close (`GOING_AWAY` · "closing"), then 「준 닫기를 실었다」 (loaded the close we were given)
    //   cannot be measured.

    const BYE: CloseCode = CloseCode(CloseCode::CONSUMER_BASE);

    /// What went out on the channel in a readable form — a frame as its body, a close as `close:코드:문구`.
    fn wire_labels(msgs: &[ClientMsg]) -> Vec<String> {
        msgs.iter()
            .map(|msg| match msg {
                ClientMsg::Frame(Frame::Text(text)) => text.clone(),
                ClientMsg::Frame(other) => format!("{other:?}"),
                ClientMsg::Ping => "ping".into(),
                ClientMsg::Close(close) => format!("close:{}:{}", close.code.0, close.reason),
            })
            .collect()
    }

    fn dropped_out(items: &[Incoming<TestWire>]) -> u64 {
        items
            .iter()
            .map(|item| match item.as_event() {
                Some(TransportEvent::Dropped {
                    direction: Direction::Outbound,
                    count,
                    ..
                }) => *count,
                _ => 0,
            })
            .sum()
    }

    fn notify_all(peer: &Peer<TestWire>, n: usize) {
        for i in 0..n {
            peer.notify(TestOut::Notice(format!("n{i}")))
                .expect("나가는 큐");
        }
    }

    #[tokio::test]
    async fn a_flush_writes_every_notify_accepted_before_it_then_the_given_close() {
        let (mut h, endpoint) = live().await;
        notify_all(&h.peer, 5);
        let done = h.peer.close_after_flush(BYE, "bye");
        settle_until("감독이 끝났다", || h.task.is_finished()).await;

        assert_eq!(
            wire_labels(&endpoint.drain()),
            vec![
                "not|n0",
                "not|n1",
                "not|n2",
                "not|n3",
                "not|n4",
                "close:4100:bye"
            ],
            "★앞서 받아 둔 다섯이 다 나간 뒤 준 닫기가 나간다★"
        );
        assert_eq!(
            poll_immediate(done).await,
            Some(()),
            "끝남 future 가 풀렸다"
        );
        assert_eq!(
            labels(&drain_until_quiet(&mut h).await),
            vec!["Disconnected:1:Explicit", "Closed:1:Explicit"],
            "다 나갔으니 유실 신고가 없다"
        );
        assert!(h.exited_cleanly().await);
    }

    // ★Push the clock to 1 ms before the deadline and look★ — this separates a regression that folds at the write deadline (5 s) or the teardown limit without a deadline.
    // ★Make the first write already blocked before the drain★ — the deadline must be set mid-write (the wake of `Supervisor::guard`).
    #[tokio::test]
    async fn a_peer_that_does_not_read_is_closed_at_the_flush_deadline() {
        let (mut h, endpoint) = live().await;
        endpoint.stall_writes(true);
        notify_all(&h.peer, 3);
        settle().await;
        assert!(h.peer.inner.data_tx.capacity() < h.peer.inner.data_tx.max_capacity());
        let done = h.peer.close_after_flush(BYE, "bye");
        tokio::pin!(done);
        settle().await;
        assert!(endpoint.drain().is_empty(), "첫 쓰기가 막혀 있다");

        let flush = Policy::default().flush_timeout;
        h.clock.advance(flush - Duration::from_millis(1));
        settle().await;
        assert_eq!(
            h.peer.peer_state(),
            PeerState::Live,
            "시한 전에는 안 접는다"
        );
        assert_eq!(poll_immediate(done.as_mut()).await, None);

        h.clock.advance(Duration::from_millis(1));
        settle_until(
            "시한에 감독이 끝났다 — 막힌 쓰기를 풀지 않았다",
            || h.task.is_finished(),
        )
        .await;
        assert_eq!(poll_immediate(done.as_mut()).await, Some(()));
        assert!(
            endpoint.drain().is_empty(),
            "★닫기 프레임도 같은 시한 안이다★ — 남은 몫이 0 이라 막힌 통로에서는 못 나간다"
        );
        let items = drain_until_quiet(&mut h).await;
        assert_eq!(
            labels(&items),
            vec!["Disconnected:1:Explicit", "Dropped", "Closed:1:Explicit"]
        );
        assert_eq!(dropped_out(&items), 3, "쓰다 잘린 하나 + 못 쓴 둘을 센다");
        assert!(h.exited_cleanly().await);
    }

    // ★Even if the receiving line is backed up (even if the ordering area is holding), the drain's deadline runs★ — and the end does not
    //   come until the receiving side drains (teardown loads what it holds).
    #[tokio::test]
    async fn a_held_inbound_line_neither_holds_back_the_flush_deadline_nor_loses_what_was_held() {
        let (mut h, endpoint) = live_on_a_full_line().await;
        endpoint.push(TestWire::encode_in(&TestIn::Notice("held".into())));
        settle().await;
        assert!(h.peer.held_bytes() > 0, "순서 칸이 붙들고 있다");
        endpoint.stall_writes(true);
        notify_all(&h.peer, 1);
        let done = h.peer.close_after_flush(BYE, "bye");
        tokio::pin!(done);
        settle().await;

        h.clock.advance(Policy::default().flush_timeout);
        settle_until("시한에 접혔다", || {
            h.peer.peer_state() == PeerState::Closed
        })
        .await;
        settle().await;
        assert_eq!(
            poll_immediate(done.as_mut()).await,
            None,
            "정리가 붙든 것을 받는 쪽에 싣기를 기다린다"
        );

        let items = drain_until_quiet(&mut h).await;
        assert_eq!(
            labels(&items),
            vec![
                "Connected:1",
                "msg:held",
                "Disconnected:1:Explicit",
                "Dropped",
                "Closed:1:Explicit",
            ]
        );
        settle_until("받는 쪽이 비우자 감독이 끝났다", || {
            h.task.is_finished()
        })
        .await;
        assert_eq!(poll_immediate(done.as_mut()).await, Some(()));
    }

    // ★The deadline and the write end become ready in the same turn★ — it must close whichever `select!` picks. If the write wins it returns to the start of the turn
    //   and sees the past deadline as a timer (if put into `wake_at` and `on_timer` does not look, the progress check blows up).
    //   The pick is random so it runs many turns.
    #[tokio::test]
    async fn a_write_that_ends_on_the_flush_deadline_still_closes_cleanly() {
        for round in 0..24 {
            let (h, endpoint) = live().await;
            endpoint.stall_writes(true);
            notify_all(&h.peer, 1);
            let done = h.peer.close_after_flush(BYE, "bye");
            settle().await;
            endpoint.stall_writes(false);
            h.clock.advance(Policy::default().flush_timeout);
            settle_until(
                &format!("{round}번째 바퀴: 감독이 끝났다"),
                || h.task.is_finished(),
            )
            .await;
            let sent = wire_labels(&endpoint.drain());
            assert!(
                sent == ["not|n0", "close:4100:bye"] || sent == ["close:4100:bye"],
                "{round}번째 바퀴: {sent:?}"
            );
            assert_eq!(poll_immediate(done).await, Some(()));
            assert!(
                h.exited_cleanly().await,
                "{round}번째 바퀴: 감독이 패닉했다(진도 확인)"
            );
        }
    }

    // ★If the deadline passes while draining the queue back to back (`Turn::Outbound`), it does not start the next write★ (start of `Supervisor::guard`)
    //   — that batch does not return to the start of the turn so the timer cannot cut in. Stand it on a keepalive, fill the queue, then release, so the next
    //   turn drains in a batch and stands on the first write (that arrangement of `a_consumer_leaving_mid_batch_…`). The first write and the deadline are ready in the same
    //   turn and the pick is random so it runs many turns.
    #[tokio::test]
    async fn no_write_starts_after_the_flush_deadline_in_the_middle_of_a_batch() {
        for round in 0..24 {
            let h = build(
                Policy {
                    outbound_queue: 4,
                    ..Policy::default()
                },
                Arc::new(ImmediateHandshake),
            );
            settle_until("연결", || h.peer.peer_state() == PeerState::Live).await;
            let endpoint = h.net.accept().unwrap();
            endpoint.stall_pings(true);
            h.clock.advance(Policy::default().ping_interval);
            settle().await;
            notify_all(&h.peer, 3);
            let done = h.peer.close_after_flush(BYE, "bye");
            settle().await;
            endpoint.stall_writes(true);
            endpoint.stall_pings(false);
            settle().await;
            assert_eq!(endpoint.drain(), vec![ClientMsg::Ping]);
            assert_eq!(
                h.peer.inner.data_tx.capacity(),
                1,
                "{round}번째 바퀴: 감독이 묶음으로 n0 을 꺼내 쓰다 섰다"
            );

            endpoint.stall_writes(false);
            h.clock.advance(Policy::default().flush_timeout);
            settle_until(
                &format!("{round}번째 바퀴: 감독이 끝났다"),
                || h.task.is_finished(),
            )
            .await;
            let sent = wire_labels(&endpoint.drain());
            assert!(
                sent == ["not|n0", "close:4100:bye"] || sent == ["close:4100:bye"],
                "{round}번째 바퀴: ★시한 뒤에 시작한 쓰기가 없다★ — {sent:?}"
            );
            assert_eq!(poll_immediate(done).await, Some(()));
            assert!(h.exited_cleanly().await);
        }
    }

    // ★Measure while still operating★ — keep the supervisor standing on a write before the marker. If measured after folding, it cannot be told apart from rejected because it is outside operation.
    #[tokio::test]
    async fn every_send_after_the_marker_is_refused_on_the_spot_and_not_counted() {
        let (mut h, endpoint) = live().await;
        endpoint.stall_writes(true);
        notify_all(&h.peer, 1);
        settle().await;
        assert!(h.queue_drained(), "감독이 n0 을 꺼내 쓰다 섰다");
        let done = h.peer.close_after_flush(BYE, "bye");
        let after_marker = h.peer.inner.data_tx.capacity();
        assert_eq!(h.peer.peer_state(), PeerState::Live);

        assert_eq!(
            h.peer.notify(TestOut::Notice("late".into())),
            Err(SendError::Closed)
        );
        assert_eq!(
            h.peer
                .notify_in(h.peer.generation(), TestOut::Notice("late".into())),
            Err(SendError::Closed)
        );
        assert_eq!(
            h.peer.send_frame(Frame::Text("fan".into())),
            Err(SendError::Closed)
        );
        h.peer.resume_stream(1, Some(3));
        h.peer.reconnect_now();
        let asked = spawn_request(&h.peer, 7);
        settle().await;
        assert!(asked.is_finished(), "요청은 큐에 안 들고 곧바로 끝났다");
        assert!(matches!(
            joined(asked).await,
            Err(RequestError::Disconnected {
                cause: DisconnectCause::Explicit,
                sent: false,
                ..
            })
        ));
        assert_eq!(
            h.peer.inner.data_tx.capacity(),
            after_marker,
            "표지 뒤로 아무것도 안 들었다"
        );
        assert_eq!(
            h.peer.refused_count(),
            0,
            "앱이 닫겠다고 한 뒤라 세지 않는다"
        );

        endpoint.stall_writes(false);
        settle_until("감독이 끝났다", || h.task.is_finished()).await;
        assert_eq!(
            wire_labels(&endpoint.drain()),
            vec!["not|n0", "close:4100:bye"],
            "★다시 걸기도 안 들었다★ — 표지 앞 것이 나가고 준 닫기로 끝났다"
        );
        assert_eq!(h.net.dials(), 1);
        assert_eq!(
            labels(&drain_until_quiet(&mut h).await),
            vec!["Disconnected:1:Explicit", "Closed:1:Explicit"]
        );
        assert_eq!(poll_immediate(done).await, Some(()));
    }

    // ★Even if it closes at once, that close frame is within the same deadline★ — left blocked, it ends at `flush_timeout` (not the write deadline of 5 s),
    //   and released, the close we were given goes out. Control and the drain-start wake are ready in the same turn and which one is caught first is random,
    //   so the branch left blocked runs many turns.
    #[tokio::test]
    async fn a_full_outbound_queue_closes_at_once_without_writing_what_it_held() {
        for (round, unstall) in (0..12).map(|round| (round, round == 0)) {
            let mut h = build(
                Policy {
                    outbound_queue: 2,
                    ..Policy::default()
                },
                Arc::new(ImmediateHandshake),
            );
            settle_until("연결", || h.peer.peer_state() == PeerState::Live).await;
            let endpoint = h.net.accept().unwrap();
            h.drain();
            endpoint.stall_writes(true);
            notify_all(&h.peer, 1);
            settle().await;
            h.peer.notify(TestOut::Notice("n1".into())).unwrap();
            h.peer.notify(TestOut::Notice("n2".into())).unwrap();
            assert_eq!(h.peer.inner.data_tx.capacity(), 0, "큐가 찼다");

            let done = h.peer.close_after_flush(BYE, "bye");
            settle_until("시계를 밀지 않고 곧바로 접혔다", || {
                h.peer.peer_state() == PeerState::Closed
            })
            .await;
            if unstall {
                // The close that drops the channel is caught by the write block — that one is a place we deliberately do not listen at, so release it (`drop_link`).
                endpoint.stall_writes(false);
            } else {
                h.clock.advance(Policy::default().flush_timeout);
            }
            settle_until(
                &format!("{round}번째 바퀴: 감독이 끝났다"),
                || h.task.is_finished(),
            )
            .await;
            let expected: &[&str] = if unstall { &["close:4100:bye"] } else { &[] };
            assert_eq!(
                wire_labels(&endpoint.drain()),
                expected,
                "{round}번째 바퀴: 큐에 든 것은 안 쓰고, 닫기는 준 것이거나 시한에 잘렸다"
            );
            let items = drain_until_quiet(&mut h).await;
            assert_eq!(
                labels(&items),
                vec!["Disconnected:1:Explicit", "Dropped", "Closed:1:Explicit"]
            );
            assert_eq!(dropped_out(&items), 3, "표지는 유실로 세지 않는다");
            assert_eq!(poll_immediate(done).await, Some(()));
        }
    }

    // ★The finished future is awaited by a task that does not drain the receiver end (this test body)★ — start the draining side separately.
    #[tokio::test]
    async fn a_consumer_that_drains_to_the_end_sees_every_notify_go_out() {
        let (mut h, endpoint) = live_on_a_full_line().await;
        endpoint.push(TestWire::encode_in(&TestIn::Notice("held".into())));
        settle().await;
        notify_all(&h.peer, 3);
        let mut inbound = std::mem::replace(&mut h.inbound, mpsc::channel(1).1);
        let consumer = tokio::spawn(async move {
            let mut seen = Vec::new();
            while let Some(item) = inbound.recv().await {
                seen.push(label(&item.into_item()));
            }
            seen
        });
        let done = h.peer.close_after_flush(BYE, "bye");
        tokio::time::timeout(Duration::from_secs(5), done)
            .await
            .expect("끝남 future 가 안 풀렸다");

        assert_eq!(
            wire_labels(&endpoint.drain()),
            vec!["not|n0", "not|n1", "not|n2", "close:4100:bye"]
        );
        assert_eq!(
            consumer.await.expect("비우는 task"),
            vec![
                "Connected:1",
                "msg:held",
                "Disconnected:1:Explicit",
                "Closed:1:Explicit",
            ],
            "끝(`None`)까지 비웠다"
        );
    }

    #[tokio::test]
    async fn a_consumer_that_drops_its_inbox_mid_flush_folds_at_once_and_the_rest_is_discarded() {
        let (mut h, endpoint) = live().await;
        endpoint.stall_writes(true);
        notify_all(&h.peer, 3);
        let done = h.peer.close_after_flush(BYE, "bye");
        settle().await;

        drop_the_consumer(&mut h);
        settle_until("시계를 밀지 않고 접혔다", || {
            h.peer.peer_state() == PeerState::Closed
        })
        .await;
        endpoint.stall_writes(false);
        settle_until("감독이 끝났다", || h.task.is_finished()).await;
        assert_eq!(
            wire_labels(&endpoint.drain()),
            vec!["close:4100:bye"],
            "★받아 둔 셋은 버려졌다★(crate 헤더 「받는 쪽 불변식」 5)"
        );
        assert_eq!(poll_immediate(done).await, Some(()));
        assert!(h.exited_cleanly().await);
    }

    #[tokio::test]
    async fn a_link_lost_mid_flush_is_not_redialled() {
        let (mut h, endpoint) = live().await;
        endpoint.stall_writes(true);
        notify_all(&h.peer, 3);
        let done = h.peer.close_after_flush(BYE, "bye");
        settle().await;

        endpoint.fail_writes();
        endpoint.stall_writes(false);
        settle_until("백오프를 기다리지 않고 접혔다", || {
            h.peer.peer_state() == PeerState::Closed
        })
        .await;
        h.clock.advance(Policy::default().reconnect.cap);
        settle().await;
        assert_eq!(h.net.dials(), 1, "다시 걸지 않았다");
        settle_until("감독이 끝났다", || h.task.is_finished()).await;
        let items = drain_until_quiet(&mut h).await;
        assert_eq!(
            labels(&items),
            vec!["Disconnected:1:WriteFailed", "Dropped", "Closed:1:Explicit"],
            "끊김은 그 까닭 그대로 싣는다"
        );
        assert_eq!(dropped_out(&items), 3);
        assert_eq!(poll_immediate(done).await, Some(()));
    }

    #[tokio::test]
    async fn a_flush_asked_mid_handshake_abandons_the_attempt() {
        let mut h = build(Policy::default(), Arc::new(ListeningHandshake));
        settle_until("핸드셰이크 중", || {
            h.peer.peer_state() == PeerState::Handshaking
        })
        .await;
        let done = h.peer.close_after_flush(BYE, "bye");
        settle_until("핸드셰이크를 기다리지 않고 끝났다", || {
            h.task.is_finished()
        })
        .await;
        assert_eq!(h.kinds(), vec!["Closed"]);
        assert_eq!(h.net.dials(), 1);
        assert_eq!(poll_immediate(done).await, Some(()));
    }

    #[tokio::test]
    async fn a_flush_asked_after_giving_up_closes_at_once() {
        let mut h = build(Policy::default(), RejectingHandshake::new("no"));
        settle_until("포기", || h.peer.peer_state().is_terminal()).await;
        h.drain();
        let done = h.peer.close_after_flush(BYE, "bye");
        settle_until("포기 대기에서 곧바로 끝났다", || {
            h.task.is_finished()
        })
        .await;
        assert_eq!(h.kinds(), vec!["Closed"]);
        assert_eq!(poll_immediate(done).await, Some(()));
    }

    // ★The marker is not bound to a generation★ — a marker stamped with an old generation, crossing a re-attach, also closes. We create that crossing directly (in the order the handle
    //   stamps, it occurs only in a narrow race). We do not push the clock — if pushed, the drain's deadline closes even without that decision.
    #[tokio::test]
    async fn a_marker_stamped_with_an_older_generation_still_closes() {
        let (mut h, endpoint) = live().await;
        assert_eq!(h.peer.generation(), Generation(1));
        h.peer
            .inner
            .goodbye
            .set(Close::new(BYE, "bye"))
            .expect("처음 적는다");
        h.peer
            .inner
            .data_tx
            .try_send(PeerCmd {
                generation: Generation(0),
                op: PeerOp::Close {
                    _keep_ctl: h.peer.inner.ctl_tx.clone(),
                },
            })
            .unwrap_or_else(|_| panic!("나가는 큐에 자리가 있어야 한다"));
        settle_until("옛 세대의 표지로 닫혔다", || h.task.is_finished()).await;
        assert_eq!(wire_labels(&endpoint.drain()), vec!["close:4100:bye"]);
        assert_eq!(
            labels(&drain_until_quiet(&mut h).await),
            vec!["Disconnected:1:Explicit", "Closed:1:Explicit"]
        );
    }

    // ★Call before the supervisor runs★ — the `select!` in the dial phase picks at random, so without a check that looks ahead, the dial could be
    //   polled once. It runs many turns.
    #[tokio::test]
    async fn a_flush_asked_before_the_supervisor_runs_never_dials() {
        for round in 0..16 {
            let mut h = build(Policy::default(), Arc::new(ImmediateHandshake));
            let done = h.peer.close_after_flush(BYE, "bye");
            settle_until(
                &format!("{round}번째 바퀴: 감독이 끝났다"),
                || h.task.is_finished(),
            )
            .await;
            assert_eq!(h.net.dials(), 0, "{round}번째 바퀴");
            assert_eq!(h.kinds(), vec!["Closed"]);
            assert_eq!(poll_immediate(done).await, Some(()));
        }
    }

    // ★The drain continues even if all handles are released★ — if all are released, the control channel closes and the supervisor reads it as 「닫아라」 (close). While the marker
    //   holds that channel, it does not close.
    #[tokio::test]
    async fn dropping_every_handle_mid_flush_still_writes_what_was_accepted() {
        let (h, endpoint) = live().await;
        endpoint.stall_writes(true);
        notify_all(&h.peer, 3);
        let done = h.peer.close_after_flush(BYE, "bye");
        settle().await;
        let Harness {
            peer,
            inbound: _inbound,
            task,
            ..
        } = h;
        drop(peer);
        settle().await;

        endpoint.stall_writes(false);
        settle_until("감독이 끝났다", || task.is_finished()).await;
        assert_eq!(
            wire_labels(&endpoint.drain()),
            vec!["not|n0", "not|n1", "not|n2", "close:4100:bye"]
        );
        assert_eq!(poll_immediate(done).await, Some(()));
    }

    // ★A bare close does not wait for the marker (the control channel overtakes)★ — the close that is loaded is the one written earlier.
    #[tokio::test]
    async fn a_plain_close_during_a_flush_jumps_the_queue_and_speaks_the_flush_goodbye() {
        let (h, endpoint) = live().await;
        endpoint.stall_writes(true);
        notify_all(&h.peer, 2);
        let done = h.peer.close_after_flush(BYE, "bye");
        settle().await;
        h.peer.close();
        settle_until("곧바로 접혔다", || {
            h.peer.peer_state() == PeerState::Closed
        })
        .await;
        endpoint.stall_writes(false);
        settle_until("감독이 끝났다", || h.task.is_finished()).await;
        assert_eq!(wire_labels(&endpoint.drain()), vec!["close:4100:bye"]);
        assert_eq!(poll_immediate(done).await, Some(()));
    }

    #[tokio::test]
    async fn the_end_future_resolves_at_once_when_the_supervisor_is_already_gone() {
        let mut h = build(Policy::default(), Arc::new(ImmediateHandshake));
        settle_until("연결", || h.peer.peer_state() == PeerState::Live).await;
        drop_the_consumer(&mut h);
        settle_until("감독이 끝났다", || h.task.is_finished()).await;
        assert_eq!(
            poll_immediate(h.peer.close_after_flush(BYE, "bye")).await,
            Some(())
        );
    }

    // ★Call the drain while the close that drops the channel to redial is blocked★ — that close was caught by the write deadline (5 s) before the drain,
    //   but the drain's deadline (2 s) covers it too (`drop_link` listens for the drain start). Pushing the clock to 1 ms before the deadline and looking
    //   separates it from a regression that ends at the write deadline.
    #[tokio::test]
    async fn a_flush_asked_while_a_dropped_link_is_still_closing_ends_at_the_flush_deadline() {
        let (mut h, endpoint) = live().await;
        endpoint.stall_writes(true);
        h.peer.reconnect_now();
        settle().await;
        assert_eq!(h.net.dials(), 1, "막힌 닫기에 서 있어 아직 다시 안 걸었다");
        assert!(endpoint.drain().is_empty(), "닫기가 막혀 있다");
        let done = h.peer.close_after_flush(BYE, "bye");
        tokio::pin!(done);
        settle().await;

        let flush = Policy::default().flush_timeout;
        assert!(flush < Policy::default().write_deadline);
        h.clock.advance(flush - Duration::from_millis(1));
        settle().await;
        assert_eq!(poll_immediate(done.as_mut()).await, None, "시한 전이다");
        h.clock.advance(Duration::from_millis(1));
        settle_until("비우기의 시한에 감독이 끝났다", || {
            h.task.is_finished()
        })
        .await;
        assert_eq!(poll_immediate(done.as_mut()).await, Some(()));
        assert_eq!(h.net.dials(), 1, "다시 걸지 않았다");
        assert!(endpoint.drain().is_empty(), "막힌 닫기는 시한에 잘렸다");
        assert_eq!(
            labels(&drain_until_quiet(&mut h).await),
            vec!["Disconnected:1:Explicit", "Closed:1:Explicit"]
        );
        assert!(h.exited_cleanly().await);
    }

    // ★Call the reconnect right before the drain★ — the handle refuses only calls after the drain, so that one stays in the control queue.
    //   Wherever the supervisor reads it, it must drop it for the drain to end with the close we were given. Three arrangements split the places it reads — a call
    //   on a blocked write (mid-write · deterministic) · a call without a block (the operating wait mostly · mid-write sometimes) · a call on a blocked write and
    //   release at once (the write end and control are ready in the same turn so the control collection at the start of the turn is about half). The pick is random so each arrangement
    //   runs 16 turns (the probability of never stepping on the rarest branch — the third arrangement's start of the turn — even once is under 1e-4).
    #[tokio::test]
    async fn a_reconnect_asked_just_before_a_flush_does_not_cancel_it() {
        for round in 0..48 {
            let stalled = round % 3 != 1;
            let unstall_at_once = round % 3 == 2;
            let (mut h, endpoint) = live().await;
            endpoint.stall_writes(stalled);
            notify_all(&h.peer, 2);
            if stalled {
                settle().await;
                assert!(
                    endpoint.drain().is_empty(),
                    "{round}: 감독이 n0 을 쓰다 섰다"
                );
            }
            h.peer.reconnect_now();
            let done = h.peer.close_after_flush(BYE, "bye");
            if unstall_at_once {
                endpoint.stall_writes(false);
            }
            settle().await;
            endpoint.stall_writes(false);
            settle_until(
                &format!("{round}번째 바퀴: 감독이 끝났다"),
                || h.task.is_finished(),
            )
            .await;
            assert_eq!(
                wire_labels(&endpoint.drain()),
                vec!["not|n0", "not|n1", "close:4100:bye"],
                "{round}번째 바퀴: ★다시 걸기가 비우기를 자르지 않았다★"
            );
            assert_eq!(h.net.dials(), 1, "{round}번째 바퀴");
            assert_eq!(
                labels(&drain_until_quiet(&mut h).await),
                vec!["Disconnected:1:Explicit", "Closed:1:Explicit"],
                "{round}번째 바퀴: 다 나갔으니 유실이 없다"
            );
            assert_eq!(poll_immediate(done).await, Some(()));
            assert!(h.exited_cleanly().await);
        }
    }

    // ★If the connection is lost mid-drain, it does not show the reconnect machine's give-up · reconnect★ — the shell shows a reconnect popup on
    //   `GaveUp` (ADR-0180). With one budget that disconnect is the give-up (`GaveUp`) and with three it is `Backoff` — we measure both.
    // ★Make the disconnect from the peer's close★ — while the write is blocked the peer closes (it goes into the read-ahead queue) and the write deadline cuts that write.
    //   The disconnect cause is the peer's close read first (`earlier_end`), and the close that drops the channel is still blocked, so we look at the state
    //   published in between. Set the drain's deadline longer than the write deadline — if that deadline closes first, no disconnect occurs.
    // ★The close frame carries the code of the disconnect cause, not the close we were given★ (`drop_link`) — it is a channel where the write works once the block is released, so that code
    //   really goes out.
    #[tokio::test]
    async fn a_link_lost_mid_flush_neither_gives_up_nor_backs_off_and_closes_with_the_cause_code() {
        let write_deadline = Policy::default().write_deadline;
        for max_attempts in [1, 3] {
            let policy = Policy {
                flush_timeout: write_deadline * 4,
                reconnect: ReconnectPolicy {
                    max_attempts,
                    ..Policy::default().reconnect
                },
                ..Policy::default()
            };
            let mut h = build(policy, Arc::new(ImmediateHandshake));
            settle_until("연결", || h.peer.peer_state() == PeerState::Live).await;
            let endpoint = h.net.accept().unwrap();
            h.drain();
            endpoint.stall_writes(true);
            notify_all(&h.peer, 2);
            let done = h.peer.close_after_flush(BYE, "bye");
            settle().await;
            endpoint.close();
            settle().await;
            h.clock.advance(write_deadline);
            settle_until("통로를 버리러 갔다", || {
                h.peer.peer_state() != PeerState::Live
            })
            .await;
            settle().await;
            assert_eq!(
                h.peer.peer_state(),
                PeerState::Closed,
                "예산 {max_attempts}: ★Backoff · GaveUp 을 거치지 않았다★"
            );
            assert!(
                !h.task.is_finished() && endpoint.drain().is_empty(),
                "예산 {max_attempts}: 통로를 버리는 닫기가 아직 막혀 있다 — 그 사이 상태를 본 것이다"
            );

            endpoint.stall_writes(false);
            settle_until("감독이 끝났다", || h.task.is_finished()).await;
            assert_eq!(
                wire_labels(&endpoint.drain()),
                vec![format!("close:{}:closing", CloseCode::GOING_AWAY.0)],
                "예산 {max_attempts}: 준 닫기가 아니라 끊김 까닭의 코드다"
            );
            let items = drain_until_quiet(&mut h).await;
            assert_eq!(
                labels(&items),
                vec!["Disconnected:1:PeerClosed", "Dropped", "Closed:1:Explicit"],
                "예산 {max_attempts}: GaveUp · ConnectFailed 가 없다"
            );
            assert_eq!(dropped_out(&items), 2, "쓰다 잘린 하나 + 못 쓴 하나");
            assert_eq!(h.net.dials(), 1);
            assert_eq!(poll_immediate(done).await, Some(()));
            assert!(h.exited_cleanly().await);
        }
    }

    // ★If, after deciding to give up, a drain is called while the close that drops that channel is blocked, `GaveUp` does not go up★ — when the machine
    //   emits `[.., DropLink, Emit(GaveUp)]` it is before the drain so `fold_into_close` cannot collect it (`apply` collects it when emitting).
    //   Cut by the write deadline with one budget so it decides to give up, and call the drain while the close is blocked.
    #[tokio::test]
    async fn a_flush_asked_while_dropping_a_link_after_giving_up_never_reports_the_give_up() {
        let policy = Policy {
            reconnect: ReconnectPolicy {
                max_attempts: 1,
                ..Policy::default().reconnect
            },
            ..Policy::default()
        };
        let mut h = build(policy, Arc::new(ImmediateHandshake));
        settle_until("연결", || h.peer.peer_state() == PeerState::Live).await;
        let endpoint = h.net.accept().unwrap();
        h.drain();
        endpoint.stall_writes(true);
        notify_all(&h.peer, 1);
        settle().await;
        h.clock.advance(Policy::default().write_deadline);
        settle_until("포기를 정했다", || {
            h.peer.peer_state() == PeerState::GaveUp(crate::machine::GaveUpReason::BudgetExhausted)
        })
        .await;
        settle().await;
        assert!(
            !h.task.is_finished() && endpoint.drain().is_empty(),
            "통로를 버리는 닫기가 막혀 있다"
        );

        let done = h.peer.close_after_flush(BYE, "bye");
        settle().await;
        endpoint.stall_writes(false);
        settle_until("감독이 끝났다", || h.task.is_finished()).await;
        assert_eq!(
            wire_labels(&endpoint.drain()),
            vec![format!("close:{}:closing", CloseCode::WRITE_DEADLINE.0)],
            "끊김 까닭의 코드다"
        );
        let items = drain_until_quiet(&mut h).await;
        assert_eq!(
            labels(&items),
            vec![
                "Disconnected:1:WriteDeadline",
                "Dropped",
                "Closed:1:Explicit"
            ],
            "★GaveUp 이 없다★"
        );
        assert_eq!(dropped_out(&items), 1);
        assert_eq!(h.net.dials(), 1, "다시 걸지 않았다");
        assert_eq!(poll_immediate(done).await, Some(()));
        assert!(h.exited_cleanly().await);
    }

    // ★The close frame of a non-drain close waits up to the write deadline★ — this separates it from a regression that shortens it to the drain's deadline.
    #[tokio::test]
    async fn a_plain_close_waits_for_its_closing_frame_up_to_the_write_deadline() {
        let (h, endpoint) = live().await;
        endpoint.stall_writes(true);
        h.peer.close();
        settle_until("접혔다", || h.peer.peer_state() == PeerState::Closed).await;
        let flush = Policy::default().flush_timeout;
        let write = Policy::default().write_deadline;
        assert!(flush < write);
        h.clock.advance(flush);
        settle().await;
        assert!(!h.task.is_finished(), "비우기의 시한으로 자르지 않았다");
        h.clock.advance(write - flush - Duration::from_millis(1));
        settle().await;
        assert!(!h.task.is_finished(), "쓰기 시한 전이다");
        h.clock.advance(Duration::from_millis(1));
        settle_until("쓰기 시한에 닫기를 잘랐다", || {
            h.task.is_finished()
        })
        .await;
        assert!(endpoint.drain().is_empty());
        assert!(h.exited_cleanly().await);
    }

    // ★A request before the marker is written and its reply is not waited for★ — it closes at the marker and wakes as a sent disconnect (`Explicit`). We do not
    //   push the clock — this separates it from a regression that wakes on the request deadline.
    #[tokio::test]
    async fn requests_ahead_of_the_marker_are_written_then_woken_as_sent_without_their_reply() {
        let (mut h, endpoint) = live().await;
        endpoint.stall_writes(true);
        let first = spawn_request(&h.peer, 7);
        let second = spawn_request(&h.peer, 8);
        settle().await;
        let done = h.peer.close_after_flush(BYE, "bye");
        endpoint.stall_writes(false);
        settle_until("감독이 끝났다", || h.task.is_finished()).await;
        assert_eq!(
            wire_labels(&endpoint.drain()),
            vec!["req|7|body7", "req|8|body8", "close:4100:bye"]
        );
        for asked in [first, second] {
            assert_eq!(
                joined(asked).await,
                Err(RequestError::Disconnected {
                    generation: Generation(1),
                    cause: DisconnectCause::Explicit,
                    sent: true,
                })
            );
        }
        assert_eq!(
            labels(&drain_until_quiet(&mut h).await),
            vec!["Disconnected:1:Explicit", "Closed:1:Explicit"]
        );
        assert_eq!(poll_immediate(done).await, Some(()));
    }

    #[tokio::test]
    async fn a_second_flush_keeps_the_first_goodbye_and_waits_for_the_same_end() {
        let (mut h, endpoint) = live().await;
        endpoint.stall_writes(true);
        notify_all(&h.peer, 1);
        settle().await;
        let first = h.peer.close_after_flush(BYE, "bye");
        let queued = h.peer.inner.data_tx.capacity();
        let second = h.peer.close_after_flush(CloseCode(BYE.0 + 1), "again");
        assert_eq!(
            h.peer.inner.data_tx.capacity(),
            queued,
            "둘째 표지는 안 들었다"
        );
        tokio::pin!(first);
        tokio::pin!(second);
        settle().await;
        assert_eq!(
            poll_immediate(second.as_mut()).await,
            None,
            "둘째도 곧바로 풀리지 않고 같은 끝을 기다린다"
        );

        endpoint.stall_writes(false);
        settle_until("감독이 끝났다", || h.task.is_finished()).await;
        assert_eq!(
            wire_labels(&endpoint.drain()),
            vec!["not|n0", "close:4100:bye"],
            "처음 닫기가 그대로다"
        );
        assert_eq!(poll_immediate(first.as_mut()).await, Some(()));
        assert_eq!(poll_immediate(second.as_mut()).await, Some(()));
        assert_eq!(
            labels(&drain_until_quiet(&mut h).await),
            vec!["Disconnected:1:Explicit", "Closed:1:Explicit"]
        );
    }
}
