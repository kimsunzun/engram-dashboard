//! Supervisor task for one peer — ★the driving layer★.
//!
//! Carries out, on top of [`crate::link`] and [`crate::clock`], what [`crate::machine`] decided (the pure layer is "what", this file "how").
//!
//! ## Invariants this file upholds
//!
//! - ★Single writer★ — only the supervisor task holds [`LinkTx`]; everything from outside goes through the queue.
//! - ★The pending slot is registered **before sending**★ — reversed, a fast reply cannot find its slot.
//! - ★**Every** branch that drops the connection wakes the waiters★ — table entries, queued requests, and requests arriving after leaving the
//!   operating phase. One miss and that slot waits forever: the given-up state has no loop to run the deadline, and the waiting caller holds the
//!   handle, so the channel never closes either.
//! - ★Every write carries a deadline★ — `write_deadline < ping_interval`, so a write cannot starve keepalive (the deleted `net` crate did).
//! - ★The handle is immortal across reconnects★ — a generation boundary shows up as a marker on [`crate::Arrival::generation`].
//! - ★A command never leaves the generation it is bound to (TRD 3 §3-0 ③)★ — every queued command is stamped with a generation and the supervisor
//!   compares it with the current one just before writing (`Supervisor::on_cmd`); the handle entry check is only a fast path. The queue survives
//!   reconnects but the machine empties only the queue of the moment the link drops, so this closes the gap where a later entry crossed the
//!   establishment phase and was written on the next connection.
//! - ★Everything sent upward passes through the ordering area (supervisor-local queue) and is never dropped★ (TRD 3 §3-0 ②) — enqueue sites
//!   only enqueue synchronously and load right there if the line has room; otherwise the 「room in the line → drain the ordering area」 branch of
//!   **every waiting loop** (establishment · operating · backoff · given-up) loads it — one missing and held items stall in that state. While a
//!   write is pushed that branch is absent (up to `write_deadline` per write; back-to-back writes in `Turn::Outbound` · `reissue_resumes` multiply
//!   it). Upward output keeps produced order. Items are dropped only when the receiving side has closed or `CLOSE_DRAIN_LIMIT` is exceeded;
//!   ★what is dropped past the limit is neither counted nor reported★ — teardown releases the enqueue end and the receiver end (`None`) reports it.
//! - ★When the line is full, that peer's reading stops (backpressure — TRD 3 §3-0 ①)★ — while the ordering area holds anything the operating
//!   loop pulls the frame arm; the read-ahead queue ([`Policy::frame_buffer`]) fills and the read task stops. Separately, if received bytes (minus
//!   the resume-window share) reach the read share (all of `BYTE_BUDGET` without streams, half with), the read task skips its next read
//!   (`budget.rs`). The ordering area thus holds what one frame (or timer turn) produced after the line filled, a few lifecycle events, and what
//!   was salvaged from the read-ahead queue on channel drop, so a draining receiver finishes teardown within `CLOSE_DRAIN_LIMIT`.
//! - ★Stream pieces the resume window holds do not stop reading★ — counted separately as the window share, subtracted from the read gate, and
//!   measured against the window cap (half the stream budget); a piece that would exceed it is not held and the window closes as truncated
//!   (`Supervisor::route`). Stopping reading would leave the filling replay unread and turn a fillable hole into a permanent loss. Byte ceiling per
//!   supervisor = `BYTE_BUDGET` + one message (`budget.rs` header).
//! - ★While holding, the silence decision and the resume window's clock stop★ (`Supervisor::track_pause`; reasoning there) — our side is blocked,
//!   so we see neither the peer's liveness nor a re-request's answer. Our ping still goes out. Cost: that peer's replies cannot come in either, so
//!   awaiting [`Peer::request`] on the draining task hangs until its deadline (crate header 「Receiver-side invariants」 1).
//! - ★If the receiving side drops or closes the receiver end, fold immediately★ (crate header 「Receiver-side invariants」 5) — every waiting loop and
//!   write (`write` · `ping`) watches `Sender::closed`, so even with nothing to enqueue they go straight to closing and never reconnect. A dial or
//!   handshake in progress is discarded as the closed arm wins; if a just-finished handshake wins (`select!` is not biased), `shake_hands` draws a
//!   generation and the first operating turn sends the close and folds (harmless). The check also runs before polling the dial phase. A reply-less
//!   frame being written is counted like any other drop (`write_or_count`). `drop_link`'s single close does not watch — it pushes the code through,
//!   listening only to the pre-close flush start, at most `write_deadline`. Teardown does not wait on the ordering area (`Closed` is not loaded).
//! - ★What was already read is handed over before dropping the channel★ (`Supervisor::salvage_read_ahead`; details there) — replies reach the
//!   waiting caller and the rest goes ahead of the disconnect event.
//! - ★A held stream piece goes up under the generation of the connection that first carried it★ — the resume window may close on a later
//!   connection, but [`crate::Arrival::generation`] points to the carrying one.
//! - ★Teardown order = drop the channel → wake waiters · close the outbound queue and reject remaining commands → fold the open resume window
//!   (held pieces go up with the truncation report) → drain the ordering area → [`TransportEvent::Closed`] last → release the enqueue end★
//!   (`teardown`). The enqueue end is held by one supervisor alone (`PeerConfig::inbound`), so the next `recv` is `None`.
//! - ★An accepted peer (`Registry::attach`) does not reconnect (TRD 3 §3-1 ①)★ — it starts from the handshake holding the channel (the machine's
//!   dial is repaid on the spot as 「opened」 — `Supervisor::run`), and every end where the machine would reconnect · give up closes immediately
//!   via `Supervisor::fold_into_close`. ★The machine keeps its dialing-side shape★ — an accepted-peer branch there would add a branch to every
//!   end while the supervisor already has the 「close without reconnecting」 fold. The handshake deadline runs from the end of the upgrade
//!   (`Supervisor::shake_hands`); the end cause rides on [`TransportEvent::Closed`] (`Supervisor::end_cause`); on ending it removes itself from
//!   the registry (`RosterExit`).
//! - ★The pre-close flush uses one marker in the data queue (TRD 3 §3-0 ④)★ — [`Peer::close_after_flush`] writes the close to load (the
//!   「draining」 indication; afterwards the handle's sends are rejected) and puts the marker at the queue's end. While operating the supervisor
//!   writes the commands before the marker and closes at it (`Supervisor::on_cmd`; not generation-bound). Waiting loops outside operation see
//!   the indication and fold immediately via `Supervisor::fold_into_close`, withdrawing the machine's reconnect · give-up output; 「retry now」
//!   is dropped while draining (`Supervisor::drain_ctl`). One deadline ([`Policy::flush_timeout`]) covers later writes · keepalive · close frame
//!   (`Supervisor::guard` · `drop_link`), and runs even when the receiving line is backed up (`wake_at`).
//!
//! ## Priority of the operating loop (do not revive)
//!
//! `select!` must not be `biased` with the frame arm ahead of the timer and outbound queue: `biased` stops at the first ready arm, so with
//! frames arriving as fast as they are processed the later two arms are never polled — no ping (the peer cuts us for silence), no request
//! deadlines, and the outbound queue fills until the default policy disconnects at the moment of greatest output. The loop ① collects control
//! first every turn, ② skips `select` if a deadline has passed, ③ yields one turn to the outbound queue after every [`FRAME_BURST`] frames.
//!
//! ## Why there are two channels
//!
//! The data queue (bounded) and the control queue (unbounded) are separate: ★「close」 and 「reconnect now」 must get in even when the queue is
//! full★ — with one channel, backpressure blocks control too.

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

const FRAME_BURST: usize = 32;

/// Upper limit on how long teardown waits for the ordering area to be fully loaded; past it the rest up to `Closed` is dropped (the receiver end's `None` reports it).
/// 60 s covers the daemon read task's worst case on the priority line (8 slots × kill-join 5 s) (TRD 3 §3-0 ②).
pub(crate) const CLOSE_DRAIN_LIMIT: Duration = Duration::from_secs(60);

/// Payload-byte budget one supervisor may hold (B in `budget.rs`; split, read gate and B + M ceiling are defined there — TRD 3 §3-0 ① 「B + M」).
/// 16 MiB = the daemon's per-connection receive-lane budget (`CONN_RX_MAX_BYTES` in `link::server`). A flood of small frames is bounded by the count limits
/// ([`Policy::frame_buffer`] · [`Policy::inbound_queue`]).
pub(crate) const BYTE_BUDGET: usize = 16 << 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Turn {
    /// A deadline has already passed — skip `select` and look at the timer first.
    Timer,
    /// Drain this many from the outbound queue.
    Outbound(usize),
    /// Normal — wait fairly on the four arms.
    Wait,
}

/// ★The only place where the anti-starvation rule lives★. A pure function on purpose: an in-memory harness cannot reproduce what it protects (its read task
/// keeps the frame queue empty, so the later arms get polled whatever `select` does; a mutant reviving `biased` turned no flood test red), so the rule is asserted here.
fn next_turn(
    now: Instant,
    wake: Instant,
    frames_in_row: usize,
    outbound_len: usize,
    outbound_cap: usize,
) -> Turn {
    // TODO(stream-replay-vs-timer): a window deadline passing mid-write closes the window before a replay already in the read-ahead queue (crate header 「known limits」).
    if wake <= now {
        return Turn::Timer;
    }
    if outbound_len == 0 {
        return Turn::Wait;
    }
    // ★A fixed ratio alone is not enough★ — a faster producer still grows the queue into `QueueFull`; so look at depth: past half, yield and drain what piled up.
    if outbound_len.saturating_mul(2) >= outbound_cap || frames_in_row >= FRAME_BURST {
        return Turn::Outbound(outbound_len);
    }
    Turn::Wait
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendError {
    QueueFull,
    /// Not in the operating phase (connecting · backing off · given up) — or the bound connection is not the current one ([`Peer::notify_in`]).
    Disconnected,
    /// Nothing more can be sent — the supervisor is gone or [`Peer::close_after_flush`] put in the marker. ★Waiting for a re-attach will not help★ (unlike [`SendError::Disconnected`]).
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

/// One data command going to the supervisor — ★carries the generation the caller captured★ (TRD 3 §3-0 ③). The queue outlives the connection and the machine
/// cannot see later entries, so a command from generation G could be written on G+1 (the defect the shell blocked with ADR-0195); `Supervisor::on_cmd` compares before writing.
struct PeerCmd<W: Wire> {
    /// The only connection it may go out on: `*_in` = the caller's value, otherwise the current generation at the call.
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
    /// The marker of the pre-close flush (TRD 3 §3-0 ④) — an operating supervisor that reaches it closes; what to close with is `PeerInner::goodbye`.
    /// ★Not generation-bound★ (else a re-attach would reject it and the peer would never close). ★Holds the control channel's sender end★ without sending,
    /// so a side that released its handle to flush and leave does not close the channel early.
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
    /// Newest generation stamped on an unenqueued command (`fetch_max`, written before the count). ★The supervisor enforces [`QueueFullPolicy`] only when it is the current
    /// generation★, so a new connection is not cut over an ended one's overflow (`Supervisor::drain_overflow`).
    overflow_generation: Arc<AtomicU64>,
    /// Times the handle dropped something because its bound generation was not operating. ★Does not cut the connection★ — the bound one is already detached.
    refused: Arc<AtomicU64>,
    /// Wakes the supervisor on `overflow` · `refused` — given-up and backoff waits would otherwise report only at the next 「reconnect」 or backoff end. The
    /// establishment phase does not listen; its first turn reports.
    loss_wake: Arc<Notify>,
    /// Last disconnect cause; before any attach `WriteFailed` (neither end is accurate; the outbound end is closer to this value's receiver).
    last_cause: Arc<AtomicU8>,
    /// The app called [`Peer::close`] · [`Peer::close_after_flush`]. It is set at the moment of the call, regardless of whether the supervisor has
    /// handled it.
    close_requested: AtomicBool,
    /// The close [`Peer::close_after_flush`] asked to load — first call only. ★Being set is itself the 「draining」 indication★ (sends rejected, no reconnect); written before the marker.
    goodbye: Arc<OnceLock<Close>>,
    /// Wakes the supervisor when `goodbye` is written, for places that do not read the data queue.
    flush_wake: Arc<Notify>,
    /// It is an accepted peer (`Origin::Accepted`) — it does not reconnect.
    accepted: bool,
    /// The counter from which this supervisor draws generations (`PeerConfig::generations`) — whoever replaces the same name tag inherits it.
    generations: Arc<AtomicU64>,
    #[cfg(test)]
    ledger: Arc<Ledger>,
}

/// A cheap-to-clone handle = the command channel to the supervisor task. ★It survives reconnects★; a **generation marker on the data side** prevents silent loss (ADR-0163/0164).
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

    pub fn peer_state(&self) -> PeerState {
        *self.inner.state_rx.borrow()
    }

    pub fn state(&self) -> watch::Receiver<PeerState> {
        self.inner.state_rx.clone()
    }

    /// The current connection generation. If it never attached, it is the floor value the registry gave.
    pub fn generation(&self) -> Generation {
        Generation(self.inner.generation.load(Ordering::Acquire))
    }

    /// Waits for the reply — bound to the current generation at the call.
    ///
    /// Errors: deadline expiry · disconnect · queue saturation · number clash · reuse of an expired number · [`Wire::request_tag`] is `None`
    /// ([`RequestError::NotARequest`] — use `notify`). ★The deadline arrives only when the connection is fine and just that request is silent★ (every connection drop wakes waiters).
    ///
    /// ★A call overlapping a reconnect never carries over★ — a re-attach before the entry check ends the call there, one after it at the supervisor's
    /// just-before-write comparison (both not-sent errors).
    ///
    /// ★Not an `async fn` — do not revive it★: the generation must be read at the call; an `async fn` body runs on first poll and would read a newer generation.
    pub fn request(&self, out: W::Out) -> impl Future<Output = Result<W::In, RequestError>> + '_ {
        let generation = self.generation();
        self.request_in(generation, out)
    }

    /// Waits for the reply — ★goes out only on the `generation` connection★ (TRD 3 §3-0 ③). If that connection is not operating (dropped · re-attached)
    /// nothing is sent: [`RequestError::Disconnected`] with `sent` false. ★The entry check is a fast path; the authority is the supervisor's just-before-write comparison★.
    /// Use: pass a received message's [`crate::Arrival::generation`] to tie a request to the connection that carried it. After [`Peer::close_after_flush`] it is a
    /// not-sent disconnect (`cause` = [`DisconnectCause::Explicit`]); other errors as [`Peer::request`].
    ///
    /// ★The first poll queues the request (or ends the call) before the first wait★ — a consumer may poll once to place it ahead of what it queues next and await the
    /// reply elsewhere (the shell driver's registration); keep every check and the push before the reply wait.
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
        // Wakes with an error even if the supervisor vanishes. ★`sent` is true★: queue vs wait table is unknowable, so err toward 「may have gone」.
        rx.await
            .unwrap_or_else(|_| Err(self.disconnected(generation, true)))
    }

    /// Does not wait for a reply — bound to the current generation at the call. ★No await★: `Err` at once if it cannot be queued; a reconnect-overlapping call
    /// is `Err` at the entry or counted as [`TransportEvent::Dropped`] at the just-before-write comparison.
    pub fn notify(&self, out: W::Out) -> Result<(), SendError> {
        self.notify_in(self.generation(), out)
    }

    /// Does not wait for a reply — ★goes out only on the `generation` connection★ (TRD 3 §3-0 ③). ★No await★.
    /// Not operating at the entry: `Err` ([`SendError::Disconnected`], uncounted). After [`Peer::close_after_flush`]: [`SendError::Closed`] (uncounted). ★A generation
    /// change after the entry makes the supervisor skip the write and count [`TransportEvent::Dropped`] ([`crate::Direction::Outbound`])★ — the caller already took an `Ok`.
    /// Use: pass a received request's [`crate::Arrival::generation`] to answer only on its connection (the shell's `CommandOutcome` — TRD 3 §3-0 ③).
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

    /// Resumes a stream. Returns the generation this request is bound to (current at the call); it goes out only on that connection.
    /// ★The return value does not mean accepted★ — not operating, queue full, or a generation change drops it, visible only as [`TransportEvent::Dropped`];
    /// ★disconnected mid-write it is neither counted nor reissued★ (only the crate's own hole re-requests are reissued).
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

    /// Is the `generation` connection operating — the fast path, ★not the authority★ (`Supervisor::on_cmd`). ★Look at the state first★: the supervisor publishes the
    /// generation before the state (`Supervisor::publish`); read first, a later re-attach would be misread as the old generation.
    fn bound_to(&self, generation: Generation) -> bool {
        self.peer_state().is_live() && self.generation() == generation
    }

    fn flushing(&self) -> bool {
        self.inner.goodbye.get().is_some()
    }

    /// ★The 「retry now」 entry★ — resets the budget and tries at once (ADR-0180 decision 7); listens even when given up. ★Not listened to after
    /// [`Peer::close_after_flush`]★ (one called earlier and unread is dropped too — listening would drop the channel and lose the drain's close). ★An accepted peer
    /// ([`crate::Registry::attach`]) does not listen★ — no address to dial.
    pub fn reconnect_now(&self) {
        if self.flushing() || self.inner.accepted {
            return;
        }
        let _ = self.inner.ctl_tx.send(Ctl::ReconnectNow);
    }

    /// Ends connection `generation` as a lost link with `cause` — the read-failure path (`Disconnected`, flap accounting, backoff before the next dial; an accepted peer
    /// ends). ★Unlike [`Peer::reconnect_now`] it keeps the budget and backoff★ so a remote that keeps sending the offending input cannot cause a reconnect loop.
    /// ★Only that connection★ — a no-op unless `generation` is live, at the entry and when taken. It goes through the outbound queue; a full queue counts an
    /// overflow of `generation` and [`crate::QueueFullPolicy`] decides (★under `DropAndReport` the drop itself is lost★). Ignored after [`Peer::close_after_flush`];
    /// ★a drop queued before it ends that flush early★ (closes with the drop's `cause`, queued writes behind it unwritten).
    pub fn drop_link_in(&self, generation: Generation, cause: DisconnectCause) {
        if self.flushing() || !self.bound_to(generation) {
            return;
        }
        let _ = self.push(PeerCmd {
            generation,
            op: PeerOp::DropLink { cause },
        });
    }

    /// Explicit close — no re-attach, does not wait for the queue (control overtakes the data queue); to drain and close use [`Peer::close_after_flush`] (if already
    /// called, this loads its close without waiting for the marker). ★No completion signal★ — the supervisor may still be closing and may issue **one more generation**;
    /// the per-name-tag shared counter (`Registry::add_with_policy`) keeps numbers from clashing.
    pub fn close(&self) {
        self.inner.close_requested.store(true, Ordering::Release);
        let _ = self.inner.ctl_tx.send(Ctl::Close);
    }

    /// After writing all outbound work accepted so far, loads `code` · `reason` and closes (TRD 3 §3-0 ④). The returned future is released when the supervisor
    /// ends (teardown done and `Closed` loaded, receiving side gone, or abnormal end), at once if it is already gone.
    ///
    /// ★The marker enters the data queue at the call★ — the close proceeds even if the future is never awaited or is dropped. Commands before the marker are
    /// written in order (each with its generation comparison); ★replies are not awaited★ (a request before the marker wakes at the close as a sent disconnect).
    /// ★One deadline★ — [`Policy::flush_timeout`] covers from the supervisor's notice through the close frame (not `write_deadline` per write); past it, the frame being
    /// written is dropped, remaining commands rejected and the close frame gets the remaining time (one push if past), so a blocked channel disconnects with no code.
    ///
    /// ★Closes at once★ when the queue is full (marker not put in; acts like [`Peer::close`], queued items unwritten) or the peer is not operating. ★A connection
    /// lost mid-drain closes without reconnecting★: `Disconnected` goes up with its cause (and the close frame carries that cause's code), then `Closed`; ★no
    /// [`TransportEvent::GaveUp`] · [`TransportEvent::ConnectFailed`], and [`Peer::state`] skips [`PeerState::Backoff`] · [`PeerState::GaveUp`]★ (the shell shows a reconnect popup on `GaveUp` — ADR-0180).
    ///
    /// ★A send after the marker is rejected on every clone, uncounted★ (`lib.rs` 「known limits」): `notify` · `notify_in` · fanout = [`SendError::Closed`]; `request` ·
    /// `request_in` = [`RequestError::Disconnected`] (`Explicit` · `sent` false); `resume_stream` = not sent; `reconnect_now` is ignored. Another clone's send crossing this
    /// call may land after the marker — the supervisor rejects it and counts [`TransportEvent::Dropped`].
    ///
    /// ★Until it ends, hold and drain that handle's receiver end, and never await this future on the task that drains it★ (crate header 「Receiver-side invariants」 5 ·
    /// 「known limits」) — dropping it folds at once and drops pre-marker commands; awaiting it there stalls up to the teardown limit.
    /// Calling again waits for the same end; a later [`Peer::close`] closes at once but loads the first close. The drain survives all handles being released.
    /// ★`()` is intended★ — what could not be written is counted by [`TransportEvent::Dropped`] ([`Direction::Outbound`]) before `Closed`; no second source.
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

    /// Released when the supervisor ends (only it holds the state sender: teardown · panic · shutdown alike); at once if already ended.
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

    /// Test handle — all payload bytes this supervisor holds; the window part is [`Peer::window_bytes`].
    #[cfg(test)]
    pub(crate) fn held_bytes(&self) -> usize {
        self.inner.ledger.level()
    }

    #[cfg(test)]
    pub(crate) fn window_bytes(&self) -> usize {
        self.inner.ledger.held()
    }

    /// One fanout frame (`Registry::broadcast`), bound to the current generation. ★What could not be put in is counted by this peer's supervisor and reported on this
    /// handle's receiver end as [`TransportEvent::Dropped`]★ — the registry holds no enqueue end (TRD 3 §3-0 ②); queue saturation is counted by `push`. ★Uncounted when
    /// the peer is not operating after [`Peer::close`]★ (`lib.rs` 「known limits」).
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

    /// Counts one unenqueued (queue full) item bound to `generation`; the supervisor reports it and, if that generation is current, applies [`QueueFullPolicy`].
    /// ★The generation is written before the count★ and ★it wakes★ (a count after a stale `Live` may arrive after the supervisor slept).
    fn count_overflow(&self, generation: Generation) {
        self.inner
            .overflow_generation
            .fetch_max(generation.0, Ordering::AcqRel);
        self.inner.overflow.fetch_add(1, Ordering::AcqRel);
        self.inner.loss_wake.notify_one();
    }

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

pub(crate) enum Origin<W: Wire> {
    /// Dials with the registry's [`Dialer`] (`Registry::add`) — if disconnected it dials again after backoff, and the only end is a close.
    Dial {
        addr: Address,
        dialer: Arc<dyn Dialer>,
    },
    /// One already accepted channel (`Registry::attach`) — starts from the handshake and does not reconnect (`Supervisor::run`).
    Accepted(AcceptedLink<W>),
}

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

pub(crate) struct PeerConfig<W: Wire> {
    pub id: PeerId,
    pub origin: Origin<W>,
    pub wire: Arc<W>,
    pub clock: Arc<dyn Clock>,
    pub handshake: Arc<dyn Handshake>,
    pub policy: Policy,
    /// ★The enqueue end of this handle's receiver end — only this one supervisor holds it★ (`registry.rs` header; shared, one noisy peer's full line would hold a quiet peer's
    /// items and the receiver end would not end with a supervisor — TRD 3 §3-0 ②). Items carry the byte share of the supervisor that enqueued them (`budget.rs`).
    pub inbound: mpsc::Sender<Charged<Incoming<W>>>,
    /// The byte budget to hold — the registry puts in [`BYTE_BUDGET`]. It is a field so that tests can put in a small value.
    pub byte_budget: usize,
    /// ★This name tag's generation counter — re-raising hands down **the same Arc**★. Passing the old supervisor's last generation as a floor raced (D1): a handshake and a
    /// queued `Close` both ready in `shake_hands` could issue one more generation after the registry read it.
    pub generations: Arc<AtomicU64>,
}

/// A supervisor not yet started — the registry starts it after raising that handle (`Registry::launch`).
pub(crate) struct Unstarted<W: Wire>(Supervisor<W>);

impl<W: Wire> Unstarted<W> {
    /// ★Drop the `JoinHandle`★ — the end is announced by the state watch (`Peer::finished`). If the runtime is folding, an accepted peer's Drop takes the registry lock
    /// (`RosterExit`) — do not call holding it.
    pub(crate) fn start(self, runtime: &tokio::runtime::Handle) {
        runtime.spawn(self.0.run());
    }
}

pub(crate) fn prepare_peer<W: Wire>(cfg: PeerConfig<W>) -> (Peer<W>, Unstarted<W>) {
    let (handle, supervisor) = new_peer(cfg);
    (handle, Unstarted(supervisor))
}

#[cfg(test)]
pub(crate) fn spawn_peer<W: Wire>(cfg: PeerConfig<W>) -> Peer<W> {
    let (handle, supervisor) = new_peer(cfg);
    tokio::spawn(supervisor.run());
    handle
}

fn new_peer<W: Wire>(cfg: PeerConfig<W>) -> (Peer<W>, Supervisor<W>) {
    // ★Everything that can panic comes before building `exit`★ — the registry calls this holding its lock; a panic after `exit` exists would re-take it while unwinding (deadlock).
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
    /// The peer closed. ★The close code is not carried★ — a disconnect during operation carries only [`DisconnectCause`] (TRD §7); the code decides in the handshake.
    Closed,
    Failed,
}

/// `Room` is the answer of [`drain_order`] (the same field of `Down` · `Connecting` too) — `false` = the receiving side has closed. The arm that watches the receiver end
/// closing (`Sender::closed`) also yields `Room(false)`.
enum Wake<W: Wire> {
    Ctl(Option<Ctl>),
    Link(Option<LinkMsg>),
    Cmd(Option<PeerCmd<W>>),
    Room(bool),
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

enum Guarded {
    Done(Result<(), Input>),
    /// The pre-close flush started (or a 「retry now」 that came during the drain was dropped) — put a deadline and keep
    /// waiting on the same write.
    Flush,
}

/// One received, decoded message with its byte share and the generation of the connection that carried it — ★stamped at receive time★ (a window-held message may
/// close on a later connection; [`Arrival::generation`] must name the carrier).
struct Received<M> {
    msg: M,
    charge: Charge,
    generation: Generation,
}

enum Mode {
    Dial {
        addr: Address,
        dialer: Arc<dyn Dialer>,
    },
    /// `auth_deadline` = upgrade end + [`Policy::connect_timeout`] — the handshake's deadline.
    Accepted { auth_deadline: Instant },
}

/// How an accepted peer removes itself from the registry when it ends (empty for a dialed peer). ★Removes on drop too★ — no ended handle stays if the supervisor panics or the runtime folds.
struct RosterExit<W: Wire> {
    leave: Option<LeaveRoster<W>>,
    /// ★Weak★ — a strong handle would keep the control channel open after the consumer releases all handles.
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
    /// Bytes this supervisor holds — one count across connection replacement (a re-attached channel waits for the old share to open the read gate).
    ledger: Arc<Ledger>,
    /// Held by backpressure at the turn start (`track_pause`). ★The deadline decision · wake-up time · window clock all read this field★ — separate re-looks would measure
    /// progress against different states.
    paused: bool,
    /// The time this hold started — the time it actually started, not when it was noticed (`track_pause`). While held, the window clock is stopped
    /// here (`resume_clock`).
    paused_since: Instant,
    last_rx: Instant,
    last_ping: Instant,
    dropped_out: u64,
    /// The accepted peer's upgrade permit (TRD 3 §5 14) — released on entering the operating phase (`shake_hands`) or where the channel is dropped (`run`, before teardown),
    /// never tied to teardown's wait. `None` for a dialed peer. The auth-wait and operating permits do not exist yet; this field covers their role.
    admission: Option<ConnPermit>,
    exit: RosterExit<W>,
    /// The `cause` of [`TransportEvent::Closed`]: dialed peer = `Explicit` (the preceding `Disconnected` carries the cause); accepted peer = the real cause.
    end_cause: DisconnectCause,
}

impl<W: Wire> Supervisor<W> {
    async fn run(mut self) {
        let now = self.clock.now();
        let mut acts = self.machine.on(now, Input::Start);
        if self.accepted() {
            // ★An accepted peer already holds the channel★ — the machine's dial is repaid on the spot as 「opened」.
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
                        // ★Do not kill the peer here★ — reaching this is our bug.
                        debug_assert!(false, "fell to {other:?} without a follow-up input");
                        Input::ReconnectNow
                    }
                },
            };
            let now = self.clock.now();
            if let (true, Input::LinkLost(cause)) = (self.accepted(), &input) {
                self.end_cause = *cause;
            }
            acts = self.machine.on(now, input);
            // ★Before the publish★ — publishing before folding would show `Backoff` · `GaveUp` once.
            // ★An accepted peer always folds★ (no address to redial): every machine end toward reconnect · give-up becomes a close here.
            if (self.flushing() || self.accepted())
                && matches!(
                    self.machine.state(),
                    PeerState::Backoff | PeerState::GaveUp(_)
                )
            {
                acts = self.fold_into_close(now, acts);
            }
        }
        // The channel is already dropped — release the permit and registry removal before teardown waits on the receiver.
        self.admission = None;
        self.exit.leave();
        self.teardown().await;
        // Here the supervisor is dropped and releases the enqueue end — the next `recv` on that handle's receiver end is `None`.
    }

    fn accepted(&self) -> bool {
        matches!(self.mode, Mode::Accepted { .. })
    }

    /// The connection was lost during the pre-close flush: instead of reconnecting or giving up, fold into the close ([`Peer::close_after_flush`]). The machine stays pure;
    /// only its reconnect · give-up part (wait · `GaveUp` · `ConnectFailed`) is withdrawn; the disconnect cause and close code are kept (`drop_link`).
    /// ★Filtering `GaveUp` is the point★ — the shell shows a reconnect popup on it (ADR-0180), false for a peer the app is closing. A drain that began during the drop of a
    /// give-up batch is collected in `apply`. ★An accepted peer always comes here on loss★ (`run`); the terminal reject ([`MachineEvent::Rejected`]) is filtered too
    /// (it promises a `GaveUp`; the end rides on `Closed` — `Supervisor::end_cause`).
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
                // ★Look again when emitting★ — a give-up batch comes out before the drain and `fold_into_close` cannot collect it.
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
                        "DropLink came without FailPending while holding the link"
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

    /// Collects piled-up control signals; `Close` beats `ReconnectNow`. ★Folding consecutive `ReconnectNow` into one is the point★ — otherwise each press cancels the dial
    /// before it is polled and 「cut → stop mid-dial」 repeats forever. ★During a pre-close flush `ReconnectNow` is dropped★ (also entries from before the drain): listening
    /// would drop the channel and lose the drain's close; every control reader follows this.
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

    /// ★Listens to control throughout both phases★ — else `close()` is heard only after `connect_timeout`.
    async fn dial(&mut self, deadline: Instant) -> Input {
        let Mode::Dial { addr, dialer } = &self.mode else {
            // Blocked by the `run` fold · `Peer::reconnect_now`; reaching this is our bug, and with no address we fold.
            debug_assert!(false, "a dial came to an accepted peer");
            return Input::Close;
        };
        let (addr, dialer) = (addr.clone(), dialer.clone());
        // A 「reconnect now」 that was already piled up is exactly what we are about to do, so we swallow it.
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
        // ★Pairs with the same line in `dial`★ — it folds the piled-up 「reconnect now」 here too.
        if let Some(Ctl::Close) = self.drain_ctl() {
            return Input::Close;
        }
        let (Some(tx), Some(rx)) = (self.tx.take(), self.rx_raw.take()) else {
            return Input::ConnectFailed(ConnectFailure::Unreachable(LinkError::new(
                "no link to hand shake on",
            )));
        };
        let deadline = match self.mode {
            // ★An accepted peer's deadline runs from the end of the upgrade★ (TRD 3 §5 3), capped by the machine's deadline.
            Mode::Accepted { auth_deadline } => auth_deadline.min(deadline),
            Mode::Dial { .. } => deadline,
        };
        // ★No handshake past the deadline★ — `with_deadline` polls once even on a past deadline, so queued frames could get through; the channel is dropped without a close.
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
                // The number is drawn here — this place is the only point of 「this link entered the operating phase」.
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

    /// Runs one establishment phase while listening to control, loading the ordering area and watching for receiver closure; the returned `Room` is always `false`.
    /// ★The phase future is pinned outside the turn★ — dropping it each time the ordering-area branch wins would restart the dial and handshake.
    async fn connecting<T>(&mut self, step: impl Future<Output = T>) -> Connecting<T> {
        // ★Look before polling the phase★ — else a gone receiver still gets the dial future polled once (`select!` order is unspecified).
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
                // ★Wakes on receiver closure★ (crate header 「Receiver-side invariants」 5; `Sender::closed` is cancel-safe). ★Also listens for the drain start★ — this phase does not read the data queue.
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
                // A 「retry now」 read during a drain is dropped (`Self::drain_ctl`) — there is no connection to drain, so it folds.
                Connecting::Ctl(Some(Ctl::ReconnectNow)) if self.flushing() => {
                    return Connecting::Flush;
                }
                other => return other,
            }
        }
    }

    fn attach_reader(&mut self, rx: Box<dyn LinkRx>) {
        let (frames_tx, frames_rx) = mpsc::channel(self.policy.frame_buffer.max(1));
        self.reader = Some(tokio::spawn(read_link(rx, frames_tx, self.ledger.clone())));
        self.frames_rx = Some(frames_rx);
        self.fresh_link = true;
        let now = self.clock.now();
        self.last_rx = now;
        self.last_ping = now;
    }

    /// Backoff wait. ★Requests entering the queue fail at once even while waiting★ — else the slot hangs until the next operating phase (forever when given up).
    async fn wait(&mut self, until: Instant) -> Input {
        loop {
            // ★Report here too★ — else losses during backoff · give-up surface nowhere until 「reconnect」.
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

    async fn park(&mut self) -> Input {
        loop {
            let _ = self.drain_overflow();
            self.report_drops();
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
                tokio::select! {
                    signal = ctl.recv() => Down::Ctl(signal),
                    cmd = data.recv() => Down::Cmd(cmd),
                    open = drain_order(lane, order), if backlog => Down::Room(open),
                    () = lane.closed() => Down::Room(false),
                    () = lost.notified() => Down::Lost,
                }
            };
            match woken {
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

    fn flushing(&self) -> bool {
        self.goodbye.get().is_some()
    }

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
            // ★Look at the hold before the loss report★ — the report may empty the ordering area, hiding a hold that began during the write, and the window would close before the read-ahead replay.
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
            if let Some(signal) = self.drain_ctl() {
                return match signal {
                    Ctl::ReconnectNow => Input::ReconnectNow,
                    Ctl::Close => Input::Close,
                };
            }

            let wake = self.wake_at();
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
                    // ★Check progress★ — a timer that cannot push its deadline spins without `await`; on current_thread nothing else runs (measured: a mutant hung over 10 minutes).
                    if self.wake_at() <= now {
                        debug_assert!(
                            false,
                            "the timer could not push its own deadline — the operating loop spins without await"
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
            // Wakes when the read gate opens. ★If the turn start saw a hold by the read gate (`paused` · ordering area empty), the arm stands even if it has since cleared★ — it
            //   wakes at once and the next turn writes the clearing; otherwise a quiet peer's clearing waits for the ping. An extra wake-up is not a spin.
            let over = self.ledger.over() || (self.paused && !backlog);
            let woken = {
                let ctl = &mut self.ctl_rx;
                let data = &mut self.data_rx;
                let frames = self.frames_rx.as_mut().expect("checked above");
                let (lane, order) = (&self.inbound, &mut self.order);
                let ledger = &self.ledger;
                // ★Do not re-attach `biased`★ (header 「priority of the operating loop」). ★If the ordering area holds anything, pull the frame arm (backpressure)★ —
                //   its output could not be loaded either; the threshold is one item because the ordering area serves synchronous puts, not as a frame buffer.
                // ★Wakes on receiver closure★ (same arm as in `connecting`).
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

    /// Sends out again the resumes left disconnected. ★Without this the hole stays forever★ — the in-flight re-request may not have reached the peer.
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
        // Wind with the window clock — a previously held, disconnected clock is still stopped and is pushed at clearing (`track_pause`).
        let now = self.clock.now();
        self.streams.rearm_resumes(self.resume_clock(now));
        None
    }

    /// The next time to wake. ★Every timer source gathers here★ — one missed wakes nobody. ★Deadlines that stop while held (silence · resume window) are masked by `paused`★
    /// (`on_timer` decides by it; re-looking at backpressure here would break the progress check).
    fn wake_at(&self) -> Instant {
        let mut wake = self.last_ping + self.policy.ping_interval;
        if !self.paused {
            wake = wake.min(self.last_rx + self.policy.idle_timeout);
        }
        if let Some(deadline) = self.pending.next_deadline() {
            wake = wake.min(deadline);
        }
        // ★Without it nobody can close the re-request window★ — a stream that goes quiet after a hole would keep it open forever.
        if !self.paused {
            if let Some(deadline) = self.streams.next_resume_deadline() {
                wake = wake.min(deadline);
            }
        }
        // ★Unrelated to holding★ — the drain deadline runs even when the receiving line is backed up.
        if let Some(deadline) = self.flush_deadline {
            wake = wake.min(deadline);
        }
        wake
    }

    /// On backpressure release, sets `last_rx` to that time and pushes an open resume window's requested time by the held time (TRD 3 §3-0 ① invariant 2) —
    /// otherwise that time counts as silence · window deadline. Hold = ordering area holding (frame arm pulled) or read gate closed.
    ///
    /// ★Held time counts from when the hold started, not when noticed★ (the supervisor cannot reach the turn start during a write wait). ★A hold in which the read gate
    /// closed and reopened is never seen★ — the count gathers that span ([`Ledger::take_shut`]) and we push by it. ★One time is never pushed twice★ — a span closed and
    /// opened while we knew we were held is inside the release push; an unseen span straddling a newly noticed hold is not pushed separately and the hold is treated
    /// as starting when it closed. It errs only toward the window closing later ([`Self::resume_clock`]). The silence decision takes the clearing's notice as the last receive.
    ///
    /// Also called at the start of `on_timer` (if it cleared while asleep). ★The silence decision uses `paused`★ — re-looking would use the old `last_rx` and cut a
    /// peer held for over 50 s straight to `Idle`.
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

    /// The clock the resume window reads — while held it stands still at the hold's start (`track_pause`). ★It may delay the window too much (safe side)★.
    fn resume_clock(&self, now: Instant) -> Instant {
        if self.paused {
            self.paused_since
        } else {
            now
        }
    }

    async fn on_timer(&mut self) -> Option<Input> {
        let now = self.clock.now();
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

    /// Handles one command taken out during operation. ★Just-before-write generation comparison (TRD 3 §3-0 ③)★ — a stale command ends as if it arrived outside the
    /// operating phase (`reject_command`); only this supervisor changes the generation, so no gap exists. It precedes every other side effect (a stale request in the
    /// pending table would clash with the current connection's number). ★The pre-close marker is not compared★ — no bound generation (`PeerOp::Close`).
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

    /// Handles one received frame. ★Does not wait★ ([`Self::salvage_read_ahead`] calls it too); returns a re-request to write; `Err` = cut this channel (decode-failure policy).
    fn on_frame(&mut self, frame: Frame, charge: Charge) -> Result<Option<Frame>, Input> {
        let now = self.clock.now();
        if matches!(frame, Frame::Keepalive) {
            self.last_rx = now;
            return Ok(None);
        }
        // ★Garbage is not proof of liveness★ — rewinding before decoding would let an unreadable-only peer evade the silence decision.
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

    /// A reply releases its share here (it goes to the waiting caller). Returns the re-request to write ([`Self::on_frame`]).
    fn route(&mut self, msg: W::In, charge: Charge) -> Option<Frame> {
        let reply = self.wire.reply_tag(&msg);
        let mark = self.wire.stream_mark(&msg);
        if reply.is_some() && mark.is_some() {
            self.emit(TransportEvent::AmbiguousRole {
                peer: self.id.clone(),
            });
        }
        if let Some(tag) = reply {
            if let Some(slot) = self.pending.take(&tag) {
                let _ = slot.send(Ok(msg));
                return None;
            }
            // A late reply to a retired number is dropped silently by design (ADR-0181 decision 4); any other number was never requested — report.
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
        // This supervisor uses streams — split the budget (`budget.rs` header); the split precedes any hold.
        self.ledger.use_streams();

        // ★The window cap is measured before holding★ — over it, send to the same truncation as a full count limit (`Streams::observe_capped`).
        let over_cap = !self.ledger.window_fits(msg.charge.bytes());
        let now = self.resume_clock(self.clock.now());
        let verdict = self.streams.observe_capped(&mark, msg, now, over_cap);
        if matches!(verdict, Verdict::Gap { .. } | Verdict::Held { .. }) {
            // Count the held piece's share as the window share (leaves the read gate — `budget.rs` header); `deliver` or an in-window drop returns it.
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

    /// Reports the stream forgotten for room and sends up its held pieces. ★An evicted pending re-request is never refilled★ (gone from `pending_resumes`, no truncation event).
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

    // ── Writing ────────────────────────────────────────────────────────────────

    /// ★Also listens for control and receiver closure while writing★ — else they are heard only after `write_deadline` (times back-to-back writes). ★How far the frame got is
    /// unknown★ (it may sit in the adapter buffer and go out ahead of `drop_link`'s close); a notification · fanout is counted by [`Self::write_or_count`].
    async fn write(&mut self, frame: Frame) -> Result<(), Input> {
        let Some(mut tx) = self.tx.take() else {
            return Err(Input::LinkLost(DisconnectCause::WriteFailed));
        };
        let result = self.guard(tx.send(frame)).await;
        self.tx = Some(tx);
        result
    }

    /// Writes a reply-less frame. ★Not ending in `Ok` counts, whatever the reason★ — the handle already took an `Ok` (the `fail_pending` rule); a half-written frame may have gone out, so the count can be inflated.
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

    /// Pushes one write (frame · keepalive) within `write_deadline` while listening for control · receiver closure · the pre-close flush deadline. The write end is briefly
    /// taken out, so `self.tx` is empty. ★The flush deadline cuts the write even if set mid-write★ (`flush_wake`; TRD 3 §3-0 ④ 「one deadline」); a passed deadline means the
    /// write does not start. ★During a drain 「retry now」 is dropped★ (`Self::drain_ctl`).
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
                // The write arm precedes the deadline arm on purpose — a write finishing at the deadline is not counted as dropped (random pick, no `biased`).
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

    // ── Cleanup ────────────────────────────────────────────────────────────────

    /// Drops the channel — tries to send a close code and folds the reading task.
    ///
    /// ★Does not listen to control — it pushes the close through to the end★ — without `biased`, `select!` picks the start arm at random, so a control arm with any signal
    /// left in the queue could drop the close future unpolled (measured: after `reconnect_now()` twice the goodbye was lost in about half of 100 runs with no congestion).
    /// ★Do not revert★ — the consumer branches on it: a sent close reads 「said goodbye and left」, otherwise 「link error」. When congested the code is still lost
    /// (`write_deadline`; `lib.rs` 「known limits」).
    ///
    /// ★The only thing listened for is the start of the pre-close flush★ — then the wait is the shorter of `write_deadline` and the flush deadline's remainder
    /// ([`Peer::close_after_flush`]); the close future is held, not dropped.
    ///
    /// ★Known limit — this wait is tied to the clock★: in a hand-advanced harness a channel dropped during a blocked write keeps the supervisor here until the clock
    /// advances by `write_deadline` or the stall is released (`testing::MemoryEndpoint::stall_writes`); the real clock always flows.
    async fn drop_link(&mut self, code: CloseCode) {
        self.note_flush();
        if let Some(mut tx) = self.tx.take() {
            let clock = self.clock.clone();
            // Load the close [`Peer::close_after_flush`] wrote only when dropping because of the app's close — decide by `last_cause` (the machine wakes waiters with it
            //   before `DropLink`); a channel lost mid-drain and folded carries that disconnect's cause. ★Do not decide by machine state★ — folding sends it to `Closed` first.
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
                // ★The close arm is first and `biased`★ — even past the flush deadline the close is pushed once (0 in `with_deadline`).
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
        // A branch that left the operating phase was already folded by `salvage_read_ahead`; what is left is a gone receiver.
        self.frames_rx = None;
        self.fresh_link = false;
        if let Some(reader) = self.reader.take() {
            reader.abort();
            // ★Wait for the cancellation★ — the reading task holds the other half, so the socket does not close until it drops (`ws::split_link`); at most one turn.
            let _ = reader.await;
        }
    }

    /// ★What was already read is handed over before dropping the channel★ — called on every branch that leaves the operating phase (`run`), before the machine's waiter
    /// wake-up · disconnect event: replies reach the waiting caller, the rest goes into the ordering area ahead of the disconnect event under the current generation marker.
    /// Without it the full read-ahead queue ([`Policy::frame_buffer`] frames) would be silently dropped.
    ///
    /// ★Fold the read task first★ (it reads only after taking a read-ahead slot) so the queue holds everything read; wait one turn for the cancellation (TRD 3 §3-0 ②).
    /// It writes no re-requests — the window stays open and goes out on the next link (`reissue_resumes`).
    ///
    /// Returns the frontmost reader's end already in the queue (decode-failure frame · peer close · channel error); [`earlier_end`] decides the cause rewrite.
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
                // ★Even under cut-on-decode-failure, keep handing over what follows★ — same as `run_live` would; only the cause is that frame's. The returned re-request is not written.
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
        // ★The pending table is sent★ — entries were written just before (`on_cmd`) and passed the generation comparison.
        for (_, slot) in self.pending.drain() {
            let _ = slot.send(Err(RequestError::Disconnected {
                generation,
                cause,
                sent: true,
            }));
        }
        // ★Also collect what sleeps in the queue★ — requests wake as not sent, reply-less ones are **counted** (the handle took an `Ok`).
        while let Ok(cmd) = self.data_rx.try_recv() {
            self.reject_command(cmd);
        }
    }

    /// Ends one command that will not be written (arrived outside the operating phase, or stale generation — `on_cmd`). A request wakes with its stamped generation and
    /// the last known disconnect cause (possibly a later disconnect's); reply-less ones are counted.
    fn reject_command(&mut self, cmd: PeerCmd<W>) {
        match cmd.op {
            PeerOp::Request { reply, .. } => {
                let _ = reply.send(Err(RequestError::Disconnected {
                    generation: cmd.generation,
                    cause: cause_from_code(self.last_cause.load(Ordering::Acquire)),
                    sent: false,
                }));
            }
            // ★Do not fold with `_`★ — decide each new branch: count · wake · end.
            PeerOp::Notify { .. } | PeerOp::Raw(_) | PeerOp::Resume { .. } => self.dropped_out += 1,
            // Not a message, so uncounted; dropping it loses no close — `goodbye` is seen by every waiting loop outside operation.
            PeerOp::Close { .. } => {}
            // Not a message, and its connection is gone already — nothing to drop, nothing to count.
            PeerOp::DropLink { .. } => {}
        }
    }

    /// ★Draining the ordering area comes last★ — only it waits on the receiver; first, entries queued meanwhile would hang until it drains (at most [`CLOSE_DRAIN_LIMIT`]).
    async fn teardown(&mut self) {
        let cause = self.end_cause;
        self.drop_link(CloseCode::GOING_AWAY).await;
        self.fail_pending(cause);
        self.data_rx.close();
        // Wake the rest too — anything that comes in after the close is rejected by the channel as `Closed`.
        while let Ok(cmd) = self.data_rx.try_recv() {
            self.reject_command(cmd);
        }
        // ★Fold the open resume window★ — else held pieces vanish without delivery or truncation report.
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
        // ★Everything going up stands at the read gate★ — a piece leaving the window is subtracted from the window share (`budget.rs` header).
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

    /// Puts at the end of the ordering area and loads what fits in the line. ★Does not wait★ (synchronous put sites; waiting would stall control and hang teardown).
    /// ★A full line does not cut or drop the connection★ — it is this handle's line, so full means the consumer is slow (ADR-0310 decision 17); the operating loop stops reading that peer instead.
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

    /// Reports counted outbound losses ★regardless of policy★ (policy.rs).
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

    /// Collects and handles what the handle could not put in. ★Only outbound-queue saturation cuts the connection★ ('my queue is full'), not 'the upward line is full' (`put`).
    /// ★Cut only when the overflow belongs to the current generation★ — the count outlives generations and would cut a new connection over an old one's overflow;
    /// the count is reported regardless.
    #[must_use = "during operation this decision cuts the connection"]
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

/// The input to feed the machine on leaving the operating phase: if a reader's end was already in the read-ahead queue salvaged on channel drop ([`Supervisor::salvage_read_ahead`]),
/// it becomes the disconnect cause instead of why we left (write failure · silence · overflow …). ★Not rewritten★ — an app close (`Input::Close`) or reconnect request
/// (`Input::ReconnectNow`) stays `Explicit` (ADR-0180 decision 7: a reconnect must refill the budget and dial at once), and a cause already the reader's end stays.
fn earlier_end(input: Input, read_end: Option<DisconnectCause>) -> Input {
    match (input, read_end) {
        (Input::LinkLost(ours), Some(theirs)) if !is_read_end(ours) => Input::LinkLost(theirs),
        (input, _) => input,
    }
}

fn is_read_end(cause: DisconnectCause) -> bool {
    matches!(
        cause,
        DisconnectCause::PeerClosed | DisconnectCause::ReadFailed
    )
}

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

/// Waits until a slot opens in the line, then loads as much of the ordering area as fits. `false` = the receiving side closed.
///
/// ★Cancel-safe — put it on a `select!` arm★: dropped while waiting for a slot, the ordering area is unchanged, and after taking the slot the
/// loading finishes at once without `await`. Slots go in waiting order (external fact — tokio bounded-channel slot reservation is FIFO).
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

/// Opens only the channel. ★The dial and the handshake **share** the deadline★ (the old `HANDSHAKE_TIMEOUT` had the same shape): a slow dial
/// shrinks the handshake's remaining time.
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
        // The dialing side already knows — attached but treated as a reject (give up without budget · TRD 3 §3-0 ⑤).
        Ok(Err(DialFailure::Refused { code, reason })) => {
            debug_assert_eq!(
                RejectKind::of(code),
                Some(RejectKind::Terminal),
                "DialFailure::Refused carries only terminal reject codes: {code:?}"
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

/// The handshake did not finish — the dialing side feeds `failure` to the machine, the accepted peer writes `end` as the end cause
/// (`Supervisor::end_cause`); both are decided at each failure place because `ConnectFailure` carries only the text and cannot tell read end from write end.
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
/// An accepted peer's end cause: our reject = `Explicit` · peer closed (including a reject code) = `PeerClosed` · read error = `ReadFailed` ·
/// write error = `WriteFailed` · write past deadline = `WriteDeadline` · peer frame not arrived by the deadline = `Idle`.
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
                    // Nothing more is written after a half-sent frame; the channel is simply dropped.
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
                // ★A peer close carrying a code = a reject, and the code decides the kind★ (TRD 3 §3-0 ⑤ — text does not). Without it 「a terminal
                //   reject does not use budget」 (ADR-0180 decision 5) does not hold in code. ★A bare close · normal close code is not a 「reject」★ —
                //   a silent drop is 「failed to attach」 and uses budget (a restarting daemon attaches next attempt).
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
/// ★Why a separate task★ — `recv()` directly on a `select!` arm is canceled whenever another arm wins, and on a transport that is not
/// cancel-safe a frame vanishes then.
///
/// ★A closed read gate stops the next read★ (「fill level before adding」 — `budget.rs`) — with only the read-ahead slot count the line would
/// hold up to slot count × one-message limit. The share is added right where it is read.
///
/// ★Take the read-ahead slot before reading★ — never wait for a slot holding a frame already read, so that when the supervisor folds this task
/// everything read is in the read-ahead queue (`Supervisor::salvage_read_ahead`); folded with one in hand, that frame would be out of the
/// channel and go nowhere.
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
        buffered: Vec<Incoming<TestWire>>,
        /// The supervisor task. ★Channels and the clock cannot tell 「finished」 from 「panicked」★ — a panic also drops the supervisor, closes all
        /// channels and releases all sleeping clocks.
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

        /// Did teardown close the outbound queue. ★If the line had room, the last loss report and `Closed` are already up when this is seen★ —
        /// `teardown` puts and loads without `await` after closing the queue, so on current_thread nothing cuts in between. If the line was full,
        /// teardown still waits in the ordering area.
        fn supervisor_done(&self) -> bool {
            self.peer.inner.data_tx.is_closed()
        }

        /// Did the supervisor take everything out of the outbound queue. ★「nothing went out on the link」 alone does not make 「writing」 stand★ —
        /// it is true with items left in the queue (counted or woken separately by `fail_pending`); this way a mid-write count assertion holds
        /// regardless of the write.
        fn queue_drained(&self) -> bool {
            let tx = &self.peer.inner.data_tx;
            tx.capacity() == tx.max_capacity()
        }

        /// Has the supervisor task ended without panicking. Polling an ended handle again panics tokio and eats the harness — use it as the last assertion.
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
        settle_until("first connection", || {
            h.peer.peer_state() == PeerState::Live
        })
        .await;
        assert_eq!(h.kinds(), vec!["Connected"]);
        let endpoint = h.net.accept().expect("the link must have opened");
        (h, endpoint)
    }

    /// Creates a state where frames **never stop**. ★A finite bundle cannot create starvation★ — once the supervisor drains it, the frame arm
    /// is `Pending` and the later arms get polled, so 「push in 1000 and settle」 **passes the same with or without `biased`** (measured — the first
    /// edition caught no mutant). Starvation needs **fill rate ≥ drain rate**, so the supply keeps running.
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

    async fn pump(rounds: usize) {
        for _ in 0..rounds {
            tokio::task::yield_now().await;
        }
    }

    /// ★If the defect revives, it **fails cleanly instead of hanging**★ — libtest has no per-test deadline and a permanent wait would halt CI.
    /// The real-time 5 seconds flows **only on the failure path** (precedent: the shell package's `lib_unit`).
    async fn joined(
        task: tokio::task::JoinHandle<Result<TestIn, RequestError>>,
    ) -> Result<TestIn, RequestError> {
        tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .expect("the request did not wake — a permanent wait regression")
            .expect("the request task panicked")
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
    // ★These eight are the regression net for C2·F1★, not the flood tests below: those check 「stays alive under load」, and **starvation itself
    //   cannot be created with this harness** (the read task is a structural bottleneck, so the frame queue is always empty — see `next_turn`).
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
                    "a deadline already passed is more urgent"
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
            .expect("without a cap on the number of frames processed in a row, the outbound queue starves forever");
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
                "★without looking at depth, the queue keeps growing in front of a producer above the ratio and ends in QueueFull★"
            );
        }
        assert_eq!(
            next_turn(t0, future, 0, half - 1, CAP),
            Turn::Wait,
            "below half, it waits as usual"
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
                "taking out only one per round is the same defect with a threshold only 32x higher"
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
        settle_until("handshake entered", || {
            h.peer.peer_state() == PeerState::Handshaking
        })
        .await;
        let endpoint = h.net.accept().expect("link");
        assert_eq!(
            endpoint.frames(),
            vec![Frame::Text(HelloHandshake::HELLO.into())]
        );
        endpoint.push(Frame::Text(HelloHandshake::WELCOME.into()));
        settle_until("operation entered", || {
            h.peer.peer_state() == PeerState::Live
        })
        .await;
    }

    // A dial that lands but whose handshake never answers fails at the connect deadline (they share it) as a network-class failure, and the peer backs off.
    #[tokio::test]
    async fn a_handshake_that_never_answers_fails_at_the_connect_deadline_and_backs_off() {
        let mut h = build(Policy::default(), Arc::new(HelloHandshake));
        settle_until("handshaking", || {
            h.peer.peer_state() == PeerState::Handshaking
        })
        .await;
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
        settle_until("connection", || peer.peer_state() == PeerState::Live).await;
        assert_eq!(peer.generation(), Generation(42));
        assert_eq!(
            counter.load(Ordering::Acquire),
            42,
            "the counter goes up together"
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
        // A new supervisor attaches while the old one is still alive (a succession race on the same name tag).
        let first = spawn("d1");
        let second = spawn("d1");
        settle_until("both connected", || {
            first.peer_state() == PeerState::Live && second.peer_state() == PeerState::Live
        })
        .await;
        assert_ne!(
            first.generation(),
            second.generation(),
            "★if two supervisors issue the same number, the consumer cannot tell which socket it belongs to★"
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
            "a deadline expiry does not touch the connection"
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
        assert!(
            h.messages().is_empty(),
            "a late reply is silently discarded"
        );
        assert!(
            !h.kinds().contains(&"UnmatchedReply"),
            "a designed late reply is not reported"
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
            "if only reconnect_now does not emit FailPending, this slot waits forever"
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
        // Imitates a race: enqueued, but the supervisor has already left the operating phase.
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

    // The `request_in` contract a consumer relies on: one poll queues the request, so what is queued next goes out after it while the reply is awaited later.
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

    // Transport edition of the shell's `buffered_command_on_disconnect_is_drained_not_executed_post_reconnect` — a request awaiting its answer and a
    //   queued request both wake on the disconnect and are not executed on the next connection. ★Pin the branches deterministically★: blocking the
    //   preceding notification's write stands the supervisor on it, so the second request stays in the queue (a write does not read the data queue);
    //   the two then take the two branches of the sent field (D12).
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
            "the first request goes out and waits for the answer"
        );

        first.stall_writes(true);
        h.peer
            .notify(TestOut::Notice("stuck".into()))
            .expect("operating");
        settle().await;
        let queued = spawn_request(&h.peer, 2);
        settle().await;
        assert!(
            !h.queue_drained(),
            "the supervisor is stuck on a blocked write, so the second request is in the queue"
        );

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
            "★a request that was waiting for an answer is sent★ — the peer may have processed it: {waiting:?}"
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
            "★a request that stayed in the queue is not sent★ — sending it again does not apply it twice: {queued:?}"
        );

        h.clock.advance(Duration::from_millis(500));
        settle_until("reconnect", || {
            h.peer.peer_state() == PeerState::Live && h.peer.generation() != g1
        })
        .await;
        let next = h.net.accept().expect("second link");
        settle().await;
        assert_eq!(
            texts(next.frames()),
            Vec::<String>::new(),
            "a cut command must not run on the next connection (ADR-0195)"
        );
    }

    // Transport edition of the shell's `held_clone_enqueued_after_disconnect_is_rejected_and_never_runs` — ★measures the supervisor's just-before-write
    //   comparison★. The machine empties the queue on disconnect but cannot see later entries, and the establishment phase does not read the queue.
    //   To create the gap we enqueue directly (the handle entry rejects when not operating), imitating a re-attach right after the entry check.
    #[tokio::test]
    async fn a_command_stamped_before_a_reconnect_is_not_written_on_the_next_link() {
        let mut h = build(Policy::default(), Arc::new(ListeningHandshake));
        settle_until("waiting for handshake", || {
            h.peer.peer_state() == PeerState::Handshaking
        })
        .await;
        // ★Keep hold of the other side★ — dropped, the channel vanishes silently and is cut as a read error (`MemoryEndpoint::close`).
        let first = h.net.accept().expect("first link");
        first.push(Frame::Text("go".into()));
        settle_until("first connection", || {
            h.peer.peer_state() == PeerState::Live
        })
        .await;
        let g1 = h.peer.generation();

        h.peer.reconnect_now();
        settle_until("waiting for the next handshake", || {
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
            "★the gap is real★ — the connection setup section does not read the queue, so stale commands remain until the next operating phase"
        );

        let next = h.net.accept().expect("second link");
        next.push(Frame::Text("go".into()));
        settle_until("reconnect", || h.peer.peer_state() == PeerState::Live).await;
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
            "a stale request wakes as not sent: {woken:?}"
        );
        assert_eq!(
            texts(next.frames()),
            Vec::<String>::new(),
            "★a command bound to an old generation is not written to the next connection★"
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
            "★notifications · fan-outs · resumes for which the caller took `Ok` are counted as much as they were dropped★ — not counting them is a silent loss"
        );
    }

    // `request` takes the generation at the call — even if it re-attaches after the future is made and first polled then, it does not go to the new connection.
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
        settle_until("backoff", || h.peer.peer_state() == PeerState::Backoff).await;
        h.clock.advance(Duration::from_millis(500));
        settle_until("reconnect", || h.peer.peer_state() == PeerState::Live).await;
        let next = h.net.accept().expect("second link");
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
            "★bound to the generation of its creation even if first polled after reattaching★: {first_poll:?}"
        );
        settle().await;
        assert_eq!(
            texts(next.frames()),
            Vec::<String>::new(),
            "the connection of its creation ended, so it is not written to the next connection"
        );
    }

    // A queue filled in an ended generation only reports and does not cut the next connection (`Supervisor::drain_overflow`); 「reconnect now」 dials without
    //   backoff, so the new connection's first turn is the first to collect that count.
    #[tokio::test]
    async fn a_queue_that_overflowed_on_the_old_generation_does_not_cut_the_next_link() {
        let mut h = build(
            Policy {
                outbound_queue: 2,
                ..Policy::default()
            },
            Arc::new(ImmediateHandshake),
        );
        settle_until("first connection", || {
            h.peer.peer_state() == PeerState::Live
        })
        .await;
        let first = h.net.accept().expect("first link");
        let g1 = h.peer.generation();

        first.stall_writes(true);
        let notice = |body: &str| TestOut::Notice(body.into());
        assert_eq!(h.peer.notify(notice("n1")), Ok(()));
        settle().await;
        assert_eq!(h.peer.notify(notice("n2")), Ok(()));
        assert_eq!(h.peer.notify(notice("n3")), Ok(()));
        assert_eq!(h.peer.notify(notice("n4")), Err(SendError::QueueFull));

        h.peer.reconnect_now();
        settle().await;
        first.stall_writes(false);
        settle_until("reconnect", || {
            h.peer.peer_state() == PeerState::Live && h.peer.generation() != g1
        })
        .await;
        let _next = h.net.accept().expect("second link");
        settle().await;

        let g2 = h.peer.generation();
        assert_eq!(
            h.peer.peer_state(),
            PeerState::Live,
            "★an overflow of an old generation must not cut the new connection★"
        );
        let events = h.events();
        assert!(
            !events.iter().any(|e| matches!(
                e,
                TransportEvent::Disconnected { generation, .. } if *generation == g2
            )),
            "the new connection was not cut: {:?}",
            events.iter().map(|e| e.kind()).collect::<Vec<_>>()
        );
        assert_eq!(
            outbound_dropped(&events),
            4,
            "n1 dropped mid-write · n2 · n3 dropped while queued · n4 overflowed — still reported"
        );
    }

    // Transport edition of the shell `CommandOutcome` socket comparison — an answer given with the received connection's generation goes out only on that connection.
    #[tokio::test]
    async fn notify_in_and_request_in_go_out_only_on_the_generation_they_name() {
        let (mut h, first) = live().await;
        let g1 = h.peer.generation();
        first.close();
        settle_until("backoff", || h.peer.peer_state() == PeerState::Backoff).await;
        h.clock.advance(Duration::from_millis(500));
        settle_until("reconnect", || h.peer.peer_state() == PeerState::Live).await;
        let next = h.net.accept().expect("second link");
        let g2 = h.peer.generation();
        assert_ne!(g1, g2);
        h.drain();

        assert_eq!(
            h.peer.notify_in(g1, TestOut::Notice("old".into())),
            Err(SendError::Disconnected),
            "an answer for an old connection is refused at the entrance"
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
            "a refusal at the entrance was received by the caller as an error, so it is not counted"
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

    // ── keepalive · deadline ──
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

    // Text and binary alternate 40 s apart, so the same kind recurs only after 80 s — each kind must reset the clock itself.
    #[tokio::test]
    async fn inbound_text_and_binary_data_keep_the_idle_timer_from_firing() {
        let (mut h, endpoint) = live().await;
        let text = TestIn::Notice("alive".into());
        let binary = TestIn::Blob(vec![7]);
        for _ in 0..2 {
            for msg in [&text, &binary] {
                h.clock.advance(Duration::from_secs(40));
                endpoint.push(TestWire::encode_in(msg));
                settle().await;
                assert_eq!(h.messages(), vec![msg.clone()]);
            }
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
            "★if pings stop when output is heaviest, the peer cuts us for silence★"
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
            "★the outbound queue must not starve even if the frame arm stays ready★"
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
            "★if the deadline does not run, the request hangs past the cap★"
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
            "on hitting a gap, it requests again from that point"
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

        // A piece the peer sent before receiving the re-request. ★Then put up a fence after it★ — asserting 「none」 right after `settle()` would pass
        //   even if processing had not happened, blessing the defect.
        endpoint.push(TestWire::chunk(1, 4, 6));
        endpoint.push(TestWire::encode_in(&TestIn::Notice("fence".into())));
        settle_until("until piece 6 and the fence are processed", || {
            h.seen_notice("fence")
        })
        .await;
        assert!(
            !h.kinds().contains(&"StreamTruncated"),
            "it is ★not here yet★, not ★the peer cannot give it★"
        );

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
        assert_eq!(
            seqs,
            vec![2, 3, 4, 5, 6],
            "the lost section actually comes back"
        );
    }

    #[tokio::test]
    async fn a_resume_window_closes_without_any_further_arrival() {
        let (mut h, endpoint) = live().await;
        endpoint.push(TestWire::chunk(1, 4, 1));
        endpoint.push(TestWire::chunk(1, 4, 5));
        settle().await;
        h.drain();

        // ★The window must close even if no more pieces come★ — a stream going quiet after the turn is the ordinary case; a decision hanging only on arrival stays open forever.
        h.clock.advance(Duration::from_secs(2));
        settle().await;
        let items = h.drain();
        let oldest = items.iter().find_map(|i| match i.as_event() {
            Some(TransportEvent::StreamTruncated { oldest_valid, .. }) => Some(*oldest_valid),
            _ => None,
        });
        assert_eq!(
            oldest,
            Some(5),
            "★the first number held is the lower bound★"
        );
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
            "★the reported lower bound is true only if delivery actually starts from it★"
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
        settle_until("reconnect", || h.peer.peer_state() == PeerState::Live).await;

        let reconnected = h.net.accept().expect("new link");
        assert_eq!(
            reconnected.frames(),
            vec![Frame::Text("res|1|1".into())],
            "★without requesting again after reconnecting, that gap never gets filled★"
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
        settle_until("connection", || h.peer.peer_state() == PeerState::Live).await;
        let endpoint = h.net.accept().unwrap();

        endpoint.push(TestWire::chunk(1, 4, 1));
        endpoint.push(TestWire::chunk(1, 4, 5));
        settle().await;
        h.drain();

        endpoint.push(TestWire::chunk(2, 4, 1));
        settle().await;
        let items = h.drain();
        let kinds: Vec<&str> = items
            .iter()
            .filter_map(|i| i.as_event().map(|e| e.kind()))
            .collect();
        assert!(
            kinds.contains(&"StreamForgotten"),
            "★without saying what was forgotten, that stream's ledger silently restarts from the beginning★"
        );
        assert!(
            kinds.contains(&"StreamTruncated"),
            "held pieces must not vanish with the eviction"
        );
        let seqs: Vec<u64> = items
            .iter()
            .filter_map(|i| match i.as_message().map(|a| &a.msg) {
                Some(TestIn::Chunk { key: 1, seq, .. }) => Some(*seq),
                _ => None,
            })
            .collect();
        assert_eq!(seqs, vec![5], "gets back the 5 it was holding");
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
        assert!(joined(waiting).await.is_ok(), "handled first as a reply");
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
        settle_until("reconnect", || h.peer.peer_state() == PeerState::Live).await;

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
        // ★The decoder error quotes the received value★ — if that text rode on an event, the peer could plant text on the screen · the LLM surface (TRD 3 §5 19).
        let (mut h, endpoint) = live().await;
        endpoint.push(Frame::Text("rep|PEER-SAYS-\u{1b}[31mHI|x".into()));
        settle().await;
        let reason = h.events().into_iter().find_map(|e| match e {
            TransportEvent::DecodeFailed { reason, .. } => Some(reason),
            _ => None,
        });
        assert_eq!(
            reason,
            Some("bad number"),
            "only the classification word is carried"
        );
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
        settle_until("connection", || h.peer.peer_state() == PeerState::Live).await;
        let endpoint = h.net.accept().unwrap();
        h.drain();
        endpoint.push(Frame::Text("garbage".into()));
        settle().await;
        assert_eq!(
            h.disconnect_cause(),
            Some(DisconnectCause::ReadFailed),
            "cut because what was read could not be parsed — the reading side's end"
        );
    }

    // ── Reconnect budget ──
    #[tokio::test]
    async fn being_unreachable_burns_the_budget_and_then_gives_up() {
        let mut h = build(Policy::default(), Arc::new(ImmediateHandshake));
        h.net.fail_forever();
        settle_until("first failure", || {
            h.peer.peer_state() == PeerState::Backoff
        })
        .await;

        for delay in [500u64, 1000, 2000] {
            h.clock.advance(Duration::from_millis(delay));
            settle().await;
        }
        assert_eq!(
            h.peer.peer_state(),
            PeerState::GaveUp(crate::machine::GaveUpReason::BudgetExhausted)
        );
        assert_eq!(h.net.dials(), 4, "first attempt + 3 retries");
        let kinds = h.kinds();
        assert_eq!(kinds.iter().filter(|k| **k == "ConnectFailed").count(), 4);
        assert!(kinds.contains(&"GaveUp"));
    }

    #[tokio::test]
    async fn a_rejection_spends_no_budget_and_dials_only_once() {
        let mut h = build(Policy::default(), RejectingHandshake::new("version 3 != 4"));
        settle_until("reject", || h.peer.peer_state().is_terminal()).await;
        assert_eq!(
            h.peer.peer_state(),
            PeerState::GaveUp(crate::machine::GaveUpReason::Rejected)
        );
        assert_eq!(h.net.dials(), 1, "attached but rejected = no retry");
        let events = h.events();
        let reason = events.iter().find_map(|e| match e {
            TransportEvent::Rejected { reason, .. } => Some(reason.clone()),
            _ => None,
        });
        assert_eq!(reason.as_deref(), Some("version 3 != 4"));
        assert!(events.iter().any(|e| e.kind() == "GaveUp"));
        assert!(
            !events.iter().any(|e| e.kind() == "ConnectFailed"),
            "the budget was never used, so there is no attempt failure event either"
        );
    }

    // ── N2: if the peer closes carrying a code, that is a reject ──
    #[tokio::test]
    async fn a_peer_that_closes_with_a_code_is_a_rejection_not_a_retry() {
        let mut h = build(Policy::default(), Arc::new(ListeningHandshake));
        settle_until("waiting for handshake", || {
            h.peer.peer_state() == PeerState::Handshaking
        })
        .await;
        let endpoint = h.net.accept().unwrap();
        endpoint.reject(Close::new(CloseCode::HANDSHAKE_REJECTED, "protocol 3 != 4"));
        settle_until("reject decision", || h.peer.peer_state().is_terminal()).await;

        assert_eq!(
            h.peer.peer_state(),
            PeerState::GaveUp(crate::machine::GaveUpReason::Rejected),
            "★if the code cannot be read, this becomes BudgetExhausted★"
        );
        assert_eq!(h.net.dials(), 1, "a reject does not use budget");
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
        settle_until("waiting for handshake", || {
            h.peer.peer_state() == PeerState::Handshaking
        })
        .await;
        h.net.accept().unwrap().close();
        settle_until("backoff", || h.peer.peer_state() == PeerState::Backoff).await;
        assert!(
            h.kinds().contains(&"ConnectFailed"),
            "it is the budget-using kind"
        );
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
            settle_until("end of the first attempt", || {
                matches!(
                    h.peer.peer_state(),
                    PeerState::Backoff | PeerState::GaveUp(_)
                )
            })
            .await;
            let endpoint = h.net.accept().expect("the link must have opened");
            assert_eq!(sent_close(&endpoint), Some(expected), "{expected:?}");
        }
    }

    #[tokio::test]
    async fn a_retryable_rejection_by_our_handshake_spends_budget_and_dials_again() {
        let mut h = build(
            Policy::default(),
            RejectingHandshake::retryable("stale token"),
        );
        settle_until("backoff", || h.peer.peer_state() == PeerState::Backoff).await;
        let events = h.events();
        assert!(
            !events.iter().any(|e| e.kind() == "Rejected"),
            "a reject that redials does not emit a terminal reject event"
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
            settle_until("backoff", || h.peer.peer_state() == PeerState::Backoff).await;
            h.clock.advance(Duration::from_millis(delay));
            settle_until("redial", || h.net.dials() == dials).await;
        }
        settle_until("give up", || h.peer.peer_state().is_terminal()).await;
        assert_eq!(
            h.peer.peer_state(),
            PeerState::GaveUp(crate::machine::GaveUpReason::BudgetExhausted),
            "a reject that redials gives up only when the budget runs out"
        );
        assert_eq!(h.net.dials(), 4, "first attempt + 3 retries");
    }

    #[tokio::test]
    async fn a_close_code_other_than_the_terminal_one_is_a_retryable_rejection() {
        // ★A reject with only a code★ — only 4001 is terminal; a reconnect code · unknown code uses budget and dials again.
        for code in [
            CloseCode::HANDSHAKE_RETRYABLE,
            CloseCode(1008),
            CloseCode(CloseCode::CONSUMER_BASE + 7),
        ] {
            let mut h = build(Policy::default(), Arc::new(ListeningHandshake));
            settle_until("waiting for handshake", || {
                h.peer.peer_state() == PeerState::Handshaking
            })
            .await;
            h.net
                .accept()
                .unwrap()
                .reject(Close::new(code, "try again"));
            settle_until("backoff", || h.peer.peer_state() == PeerState::Backoff).await;
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
            settle_until("redial", || h.net.dials() == 2).await;
        }
    }

    #[tokio::test]
    async fn a_graceful_close_during_the_handshake_is_could_not_connect() {
        let mut h = build(Policy::default(), Arc::new(ListeningHandshake));
        settle_until("waiting for handshake", || {
            h.peer.peer_state() == PeerState::Handshaking
        })
        .await;
        h.net
            .accept()
            .unwrap()
            .reject(Close::new(CloseCode(1000), "bye"));
        settle_until("backoff", || h.peer.peer_state() == PeerState::Backoff).await;
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
            settle_until("waiting for handshake", || {
                h.peer.peer_state() == PeerState::Handshaking
            })
            .await;
            let endpoint = h.net.accept().unwrap();
            endpoint.push(Frame::Text("error: auth failed".into()));
            settle().await;
            assert_eq!(
                h.peer.peer_state(),
                PeerState::Handshaking,
                "does not decide on a single error frame"
            );
            endpoint.reject(Close::new(code, "auth failed"));
            settle_until("decision", || {
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
        settle_until("give up", || h.peer.peer_state().is_terminal()).await;
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
            "the budget was never used, so there is no attempt failure event either"
        );
        h.clock.advance(Duration::from_secs(60));
        settle().await;
        assert_eq!(h.net.dials(), 1, "does not redial");
    }

    #[tokio::test]
    async fn a_rejection_reason_from_the_peer_is_masked_before_it_surfaces() {
        let noisy = format!(
            "evil\u{1b}[31m\nline\u{202e}{}",
            "가".repeat(crate::mask::REASON_MAX_BYTES) // hangul-ok
        );
        for code in [
            CloseCode::HANDSHAKE_REJECTED,
            CloseCode::HANDSHAKE_RETRYABLE,
        ] {
            let mut h = build(Policy::default(), Arc::new(ListeningHandshake));
            settle_until("waiting for handshake", || {
                h.peer.peer_state() == PeerState::Handshaking
            })
            .await;
            h.net
                .accept()
                .unwrap()
                .reject(Close::new(code, noisy.clone()));
            settle_until("decision", || {
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
                .expect("reject event");
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
        settle_until("reconnect", || h.peer.peer_state() == PeerState::Live).await;
        assert!(h.kinds().contains(&"Connected"));
    }

    #[tokio::test]
    async fn a_burst_of_reconnect_requests_still_lets_a_dial_happen() {
        let (h, _endpoint) = live().await;
        for _ in 0..50 {
            h.peer.reconnect_now();
        }
        settle_until(
            "without folding the piled-up signals, the dial is canceled every time before it is even polled",
            || h.net.dials() >= 2,
        )
        .await;
        settle_until("reconnect", || h.peer.peer_state() == PeerState::Live).await;
    }

    #[tokio::test]
    async fn a_lost_link_comes_back_on_the_first_backoff_step() {
        let (h, endpoint) = live().await;
        endpoint.close();
        settle_until("backoff", || h.peer.peer_state() == PeerState::Backoff).await;
        h.clock.advance(Duration::from_millis(500));
        settle_until("reconnect", || h.peer.peer_state() == PeerState::Live).await;
        assert_eq!(
            h.peer.generation(),
            Generation(2),
            "the generation went up by one"
        );
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
            "★if this is not reached, the popup of ADR-0180 decision 7 never shows★"
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
        settle_until("backoff", || h.peer.peer_state() == PeerState::Backoff).await;
        h.clock.advance(Duration::from_millis(500));
        settle_until("reconnect", || h.peer.peer_state() == PeerState::Live).await;
    }

    // ── Queue ──
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
        settle_until("connection", || h.peer.peer_state() == PeerState::Live).await;
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
        assert!(refused > 0, "the bounded queue must actually have filled");
        assert!(
            endpoint.frames().is_empty(),
            "not a single frame goes out while blocked"
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
        settle_until("connection", || h.peer.peer_state() == PeerState::Live).await;
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
        assert!(
            endpoint.frames().is_empty(),
            "the supervisor stopped at the first write"
        );

        // These are places where things are dropped while still in the queue — the handle already took an `Ok`.
        for n in 0..3 {
            h.peer.notify(TestOut::Notice(format!("n{n}"))).unwrap();
        }
        h.drain();

        h.clock.advance(Duration::from_secs(5));
        settle().await;
        // ★On a blocked channel `drop_link`'s close is stuck too★ — unreleased, the backoff wait does not start and the reconnect below does not come
        //   (`stall_writes` rustdoc).
        endpoint.stall_writes(false);
        settle().await;
        h.clock.advance(Duration::from_millis(500));
        settle_until("reconnect", || h.peer.peer_state() == PeerState::Live).await;

        let dropped = h.events().iter().find_map(|e| match e {
            TransportEvent::Dropped {
                direction, count, ..
            } => Some((*direction, *count)),
            _ => None,
        });
        assert!(
            matches!(dropped, Some((Direction::Outbound, 4))),
            "the handle already took Ok, so not counting here is a silent loss: {dropped:?}"
        );
    }

    // ── N4: if the place to send upward closes, it folds instead of re-attaching ──
    #[tokio::test]
    async fn a_closed_consumer_ends_the_peer_instead_of_triggering_reconnects() {
        let mut h = build(Policy::default(), Arc::new(ImmediateHandshake));
        settle_until("connection", || h.peer.peer_state() == PeerState::Live).await;
        let endpoint = h.net.accept().unwrap();
        let dials_before = h.net.dials();

        h.inbound.close();
        while h.inbound.try_recv().is_ok() {}
        drop(std::mem::replace(&mut h.inbound, mpsc::channel(1).1));

        endpoint.push(TestWire::encode_in(&TestIn::Notice("x".into())));
        settle_until("shutdown", || h.peer.peer_state() == PeerState::Closed).await;
        assert_eq!(
            h.net.dials(),
            dials_before,
            "a missing consumer is not a reason to reconnect"
        );
    }

    // ── F6: a graceful shutdown is not a reject ──
    #[tokio::test]
    async fn a_peer_that_shuts_down_gracefully_is_not_a_rejection() {
        for code in [CloseCode(1000), CloseCode(1001), CloseCode::GOING_AWAY] {
            let mut h = build(Policy::default(), Arc::new(ListeningHandshake));
            settle_until("waiting for handshake", || {
                h.peer.peer_state() == PeerState::Handshaking
            })
            .await;
            h.net
                .accept()
                .unwrap()
                .reject(Close::new(code, "shutting down"));
            settle_until("decision", || h.peer.peer_state() != PeerState::Handshaking).await;
            assert_eq!(
                h.peer.peer_state(),
                PeerState::Backoff,
                "★{code:?} means 「I am leaving now」, not 「I do not accept you」 — reading it as a reject would give up without retry every time the daemon shuts down cleanly★"
            );
            assert!(
                h.kinds().contains(&"ConnectFailed"),
                "it is the budget-using kind"
            );
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

        // idle_timeout (50s) has passed since connecting; garbage that rewound the clock would not be cut.
        h.clock.advance(Duration::from_secs(26));
        settle().await;
        assert_eq!(
            h.disconnect_cause(),
            Some(DisconnectCause::Idle),
            "★this is the only valve that cuts a peer sending only unreadable things★"
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
        // ★On a blocked channel `drop_link`'s close is stuck too★ — the loss report comes at the next stage (the backoff wait), so unreleased the assertion
        //   below meets it before the report. Not cut with the clock: the backoff would also pass and 「not yet attached」 would break.
        endpoint.stall_writes(false);
        settle().await;
        assert_eq!(
            h.peer.peer_state(),
            PeerState::Backoff,
            "this test means something only while not yet reattached"
        );
        let dropped = h.events().iter().find_map(|e| match e {
            TransportEvent::Dropped {
                direction, count, ..
            } => Some((*direction, *count)),
            _ => None,
        });
        assert!(
            matches!(dropped, Some((Direction::Outbound, 4))),
            "★waiting until reattached means nothing shows anywhere until the user presses 「reconnect」★: {dropped:?}"
        );
    }

    // ── 3-0 U1d: the queue-saturation count also wakes a sleeping supervisor ──
    //
    // End shape of a race: the handle enqueued after seeing a stale `Live`, the queue filled, and the count arrived after the supervisor emptied the
    //   queue and slept in the give-up wait — only the count remains. Without a wake the report waits for the next control.
    #[tokio::test]
    async fn a_queue_full_count_wakes_a_parked_supervisor() {
        let mut h = build(Policy::default(), RejectingHandshake::new("nope"));
        settle_until("give up", || {
            matches!(h.peer.peer_state(), PeerState::GaveUp(_))
        })
        .await;
        h.drain();

        h.peer.count_overflow(h.peer.generation());
        settle().await;
        assert_eq!(
            outbound_dropped(&h.events()),
            1,
            "★a sleeping supervisor wakes on its own and reports the queue saturation count★"
        );
    }

    // ── Reply-less frames cut mid-write are counted too ──
    //
    // ★The handle already took an `Ok` when it queued★ — uncounted notification · fanout frames whose write did not end in `Ok` are a silent loss
    //   (the `fail_pending` rule). Measured per cutting reason (the two controls · channel error · write deadline), together with the two that
    //   must not be counted (request · keepalive).

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

    fn reply_less_pushes() -> [(&'static str, Push); 2] {
        [
            ("notification", |p| {
                p.notify(TestOut::Notice("in flight".into()))
            }),
            ("fan-out", |p| p.send_frame(Frame::Text("in flight".into()))),
        ]
    }

    /// Blocks the write and pushes in one to stand the supervisor on that write. Empties the events that had come up.
    async fn park_in_flight(h: &mut Harness, endpoint: &MemoryEndpoint, push: Push) {
        endpoint.stall_writes(true);
        push(&h.peer).unwrap();
        settle().await;
        assert!(
            h.queue_drained(),
            "the supervisor must have taken it out of the queue"
        );
        assert!(
            endpoint.drain().is_empty(),
            "the supervisor must be stuck on that write"
        );
        h.drain();
    }

    #[tokio::test]
    async fn a_reply_less_frame_cut_mid_write_by_close_is_counted() {
        for (what, push) in reply_less_pushes() {
            let (mut h, endpoint) = live().await;
            park_in_flight(&mut h, &endpoint, push).await;
            h.peer.close();
            settle_until("shutdown", || h.peer.peer_state() == PeerState::Closed).await;
            // Releases the stuck goodbye — if not released, it cannot get to `teardown`'s report.
            endpoint.stall_writes(false);
            settle_until("the supervisor ended", || h.supervisor_done()).await;
            let events = h.events();
            assert_eq!(outbound_dropped(&events), 1, "{what}");
            let tail: Vec<&str> = events.iter().rev().take(2).map(|e| e.kind()).collect();
            assert_eq!(
                tail,
                vec!["Closed", "Dropped"],
                "{what}: the cleanup's loss report comes before Closed too"
            );
        }
    }

    #[tokio::test]
    async fn a_reply_less_frame_cut_mid_write_by_reconnect_now_is_counted() {
        for (what, push) in reply_less_pushes() {
            let (mut h, endpoint) = live().await;
            park_in_flight(&mut h, &endpoint, push).await;
            h.peer.reconnect_now();
            // ★Release after control wins★ — released first, the stuck write becomes ready together with control and may go out.
            settle_until("control won", || h.peer.peer_state() == PeerState::Dialing).await;
            endpoint.stall_writes(false);
            settle_until("reattached", || {
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
            // ★Cut only the writing side and release the stall — the stuck write ends in a channel error★ (receiving side closed; `testing::MemTx`).
            //   `fail` does not reach this branch (reading side only; the supervisor does not look at it during a write), and dropping the endpoint cuts the
            //   reading side too, whose end may reach the read-ahead queue first and make the cause the read end (`salvage_read_ahead`).
            endpoint.fail_writes();
            endpoint.stall_writes(false);
            settle_until("backoff", || h.peer.peer_state() == PeerState::Backoff).await;
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
                "{what}: must measure the link error branch"
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
            settle_until("backoff", || h.peer.peer_state() == PeerState::Backoff).await;
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
                "{what}: must measure the write deadline branch"
            );
            assert_eq!(outbound_dropped(&events), 1, "{what}");
        }
    }

    #[tokio::test]
    async fn a_request_cut_mid_write_is_woken_not_counted() {
        let (mut h, endpoint) = live().await;
        endpoint.stall_writes(true);
        // ★Poll once on the spot, not in a separate task★ — a task not yet run would make 「taken out of the queue」 below true on an empty queue.
        //   The first poll does everything up to enqueueing synchronously.
        let peer = h.peer.clone();
        let mut waiting = Box::pin(peer.request(TestOut::Request {
            tag: 1,
            body: "x".into(),
        }));
        assert!(poll_immediate(&mut waiting).await.is_none());
        assert!(!h.queue_drained(), "the request must be in the queue");
        settle().await;
        assert!(
            h.queue_drained(),
            "the supervisor must have taken it out of the queue"
        );
        // ★Must not have woken yet for 「writing」 to stand★ — a request rejected before writing wakes with the same error, so the last assertion would hold regardless.
        assert!(
            poll_immediate(&mut waiting).await.is_none(),
            "ended before writing"
        );
        assert!(
            endpoint.drain().is_empty(),
            "the supervisor must be stuck on that write"
        );
        h.drain();
        h.peer.close();
        settle_until("shutdown", || h.peer.peer_state() == PeerState::Closed).await;
        endpoint.stall_writes(false);
        settle_until("the supervisor ended", || h.supervisor_done()).await;
        let woken = poll_immediate(&mut waiting)
            .await
            .expect("the supervisor ended but the request did not wake");
        assert!(matches!(woken, Err(RequestError::Disconnected { .. })));
        assert_eq!(
            outbound_dropped(&h.events()),
            0,
            "the request wakes with an error, so it is not counted again as a loss"
        );
    }

    #[tokio::test]
    async fn a_keepalive_cut_mid_write_is_not_counted() {
        let (mut h, endpoint) = live().await;
        endpoint.stall_writes(true);
        h.clock.advance(Policy::default().ping_interval);
        settle().await;
        assert!(endpoint.drain().is_empty(), "a ping must be pending");
        h.drain();
        h.peer.close();
        settle_until("shutdown", || h.peer.peer_state() == PeerState::Closed).await;
        endpoint.stall_writes(false);
        settle_until("the supervisor ended", || h.supervisor_done()).await;
        assert_eq!(
            outbound_dropped(&h.events()),
            0,
            "keepalive is not a frame the consumer put in"
        );
    }

    // ── Shutdown ──
    #[tokio::test]
    async fn closing_is_final() {
        let h = build(Policy::default(), Arc::new(ImmediateHandshake));
        settle_until("connection", || h.peer.peer_state() == PeerState::Live).await;
        h.peer.close();
        settle_until("shutdown", || h.peer.peer_state() == PeerState::Closed).await;
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
        assert!(endpoint.frames().is_empty(), "must actually be blocked");
        h.clock.advance(Duration::from_secs(5));
        settle().await;
        assert_eq!(h.disconnect_cause(), Some(DisconnectCause::WriteDeadline));
    }

    // ── M4: it listens for control during a write too ──
    //
    // ★A published `Closed` state does not mean the supervisor has folded★ — `run` publishes the state **before applying** the actions, so
    //   `settle_until` passes while teardown is still stopped on a stuck close; release the stall and measure **past that**.
    #[tokio::test]
    async fn close_is_heard_while_a_write_is_parked() {
        let (h, endpoint) = live().await;
        endpoint.stall_writes(true);
        let _ = h.peer.notify(TestOut::Notice("stuck".into()));
        settle().await;
        h.peer.close();
        settle_until("shutdown", || h.peer.peer_state() == PeerState::Closed).await;

        endpoint.stall_writes(false);
        let mut seen = Vec::new();
        settle_until("the pending goodbye went out", || {
            seen.extend(endpoint.drain());
            seen.iter()
                .any(|m| matches!(m, ClientMsg::Close(c) if c.code == CloseCode::GOING_AWAY))
        })
        .await;
        // ★A harness property, not the [`LinkTx::send`] contract★ — `testing::MemTx` stops a blocked write before handing it over, so a cut leaves nothing;
        //   on the WS adapter the cut frame may go out ahead of the goodbye (`ws.rs` 「Link properties」).
        assert!(
            !seen.iter().any(|m| matches!(m, ClientMsg::Frame(_))),
            "a canceled write went out late: {seen:?}"
        );
    }

    // ── Shutdown: the goodbye code **really** goes out on the channel ──
    //
    // ★Dropping even the last handle is intended★ — `Registry::remove` has that shape (drops the handle after telling it to close), and the control
    //   channel then yields `Ready(None)` at once. With a control arm in `drop_link`, `select!`'s random choice counted that `None` as the winner and
    //   dropped the close future unpolled — about half the time (measured 2026-09-07).
    // ★Do not measure in one turn★ — it would catch only half; the repetition below makes it deterministic.
    #[tokio::test]
    async fn closing_speaks_the_goodbye_code_on_the_wire() {
        for round in 0..24 {
            let (h, endpoint) = live().await;
            h.peer.close();
            drop(h);
            let what = format!("round {round}: goodbye went out on the link");
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
    // ★On a channel whose write is blocked the close is blocked too★ — `stall_writes` blocks `send`·`ping`·`close` (`testing::MemoryEndpoint::stall_writes`),
    //   so the real socket's shape is measured. ★Keep the line that advances the clock★ — `drop_link` no longer cuts that wait with control, so without it
    //   the supervisor stays on the stuck close and `settle_until` dies at the limit.
    #[tokio::test]
    async fn a_parked_closing_frame_is_cut_by_the_write_deadline_not_by_control() {
        let (h, endpoint) = live().await;
        endpoint.stall_writes(true);
        endpoint.fail("link died");
        settle_until("the link died and went down to backoff", || {
            h.peer.peer_state() == PeerState::Backoff
        })
        .await;
        assert!(
            !endpoint
                .drain()
                .iter()
                .any(|m| matches!(m, ClientMsg::Close(_))),
            "★this test measures something only if the close is actually pending★ — if it went out, the assertions below are free"
        );
        h.peer.close();
        // ★Measures the 「not by control」 in the title★ — the assertions below stay green even where a control arm cut the stuck close; before advancing the clock it must not fold from the close request alone.
        settle().await;
        assert_ne!(
            h.peer.peer_state(),
            PeerState::Closed,
            "folded before the clock was advanced — control cut the pending close"
        );
        // What cuts the stuck close is this line. Control stays in the queue and is read after that.
        h.clock.advance(Duration::from_secs(5));
        settle_until("control is alive even after the pending close", || {
            h.peer.peer_state() == PeerState::Closed
        })
        .await;
        assert!(
            !endpoint
                .drain()
                .iter()
                .any(|m| matches!(m, ClientMsg::Close(_))),
            "if congestion makes the deadline cut it, the close code cannot go out — the limit `lib.rs` acknowledges"
        );
    }

    // ── F1 regression: the goodbye is not lost even if a signal stays in the queue ──
    //
    // ★Calling `reconnect_now()` **twice** is the point★ — the supervisor takes one signal per turn, so the second stays queued and a control arm in
    //   `drop_link` would be `Ready` at once, dropping the close future unpolled. No congestion; measured 2026-09-07: 41·54·49·49 of 100 runs lost
    //   the goodbye, 0 after the fix. ★Do not measure in one turn★ — half pass even if the regression revives; the repetition makes it deterministic.
    #[tokio::test]
    async fn a_queued_control_signal_does_not_swallow_the_goodbye_code() {
        let (mut h, mut endpoint) = live().await;
        for round in 0..24 {
            let dials = h.net.dials();
            h.peer.reconnect_now();
            h.peer.reconnect_now();
            settle_until("reattached", || {
                h.net.dials() == dials + 1 && h.peer.peer_state() == PeerState::Live
            })
            .await;
            assert!(
                endpoint
                    .drain()
                    .iter()
                    .any(|m| matches!(m, ClientMsg::Close(c) if c.code == CloseCode::GOING_AWAY)),
                "in round {round}, goodbye did not go out on the link"
            );
            h.drain();
            endpoint = h.net.accept().expect("new link");
        }
    }

    // ── 3-0 U1b: the ordering area ──
    //
    // ★Set up with line length 1★ — the first `Connected` fills the line, so everything after goes through the supervisor's ordering area; with room,
    //   the ordering area empties where it puts in and nothing is measured.

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
        settle_until("first connection", || {
            h.peer.peer_state() == PeerState::Live
        })
        .await;
        let endpoint = h.net.accept().expect("the link must have opened");
        (h, endpoint)
    }

    #[tokio::test]
    async fn lifecycle_events_are_never_lost_while_the_line_is_full() {
        let (mut h, endpoint) = live_on_a_full_line().await;
        h.net.fail_forever();
        endpoint.close();
        settle_until("backoff", || h.peer.peer_state() == PeerState::Backoff).await;
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
            "★even if the receiver lags, not a single lifecycle event is dropped and they come in the order emitted★"
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
            "★the close is queued behind the held message★ — the supervisor does not read the close before the receiver takes that message out"
        );

        let items = drain_until_quiet(&mut h).await;
        assert_eq!(
            labels(&items),
            vec!["Connected:1", "msg:last words", "Disconnected:1:PeerClosed"],
            "★even if the peer sends and closes right away, that message is not dropped and comes before the disconnect★"
        );
        assert_eq!(h.peer.peer_state(), PeerState::Backoff);
    }

    #[tokio::test]
    async fn held_items_keep_flowing_while_the_supervisor_is_mid_handshake() {
        let mut h = build_with(Policy::default(), Arc::new(ListeningHandshake), 1);
        settle_until("waiting for handshake", || {
            h.peer.peer_state() == PeerState::Handshaking
        })
        .await;
        let first = h.net.accept().unwrap();
        first.push(Frame::Text("welcome".into()));
        settle_until("connection", || h.peer.peer_state() == PeerState::Live).await;
        first.push(TestWire::encode_in(&TestIn::Notice("n0".into())));
        settle().await;
        // The peer's close lines up behind the held n0 and is not read (backpressure), so make it reconnect by control.
        h.peer.reconnect_now();
        settle_until("waiting for the second handshake", || {
            h.peer.peer_state() == PeerState::Handshaking
        })
        .await;
        let _second = h.net.accept().unwrap();

        let items = drain_until_quiet(&mut h).await;
        assert_eq!(
            labels(&items),
            vec!["Connected:1", "msg:n0", "Disconnected:1:Explicit"],
            "★even while the handshake waits for the peer, held items are delivered as the receiver drains★"
        );
        assert_eq!(h.peer.peer_state(), PeerState::Handshaking);
        assert_eq!(
            h.net.dials(),
            2,
            "did not abandon the dialing link and redial while delivering"
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
            "n0 is in the ordering area, n1 · n2 in the read-ahead queue — the frame arm was removed while the ordering area holds"
        );
        h.peer.close();
        settle_until("folded", || h.peer.peer_state() == PeerState::Closed).await;

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
            "★Closed comes at the very end after all held items are delivered★ — n1 · n2 in the read-ahead queue are also rescued before abandoning the link \
              and delivered before the disconnect"
        );
        assert_eq!(h.peer.held_bytes(), 0, "the share taken out left the count");
        assert!(
            matches!(
                h.inbound.try_recv(),
                Err(mpsc::error::TryRecvError::Disconnected)
            ),
            "after Closed, the supervisor releases the enqueue end"
        );
    }

    // ★Measures the order inside teardown (waking waiters · closing the queue → draining the ordering area)★. The fold is visible to the handle before
    //   the channel is dropped (`run` emits it before `apply`) and the machine's `FailPending` empties pending slots and the queue before that, so a
    //   request that teardown's `fail_pending` catches arises only from a race that folded just after the handle's entry check. We create it by
    //   enqueueing directly with the write blocked and the supervisor standing on the machine's `DropLink`.
    #[tokio::test]
    async fn requests_end_at_once_while_the_teardown_waits_on_the_held_items() {
        let (mut h, endpoint) = live_on_a_full_line().await;
        endpoint.push(TestWire::encode_in(&TestIn::Notice("held".into())));
        let asked = spawn_request(&h.peer, 7);
        settle().await;
        assert!(
            matches!(endpoint.take(), Some(ClientMsg::Frame(_))),
            "the request must have gone out and be waiting for the answer"
        );
        endpoint.stall_writes(true);
        h.peer.close();
        settle_until("folded", || h.peer.peer_state() == PeerState::Closed).await;
        assert!(
            matches!(joined(asked).await, Err(RequestError::Disconnected { .. })),
            "★waiting requests wake right away even if the receiver does not drain★"
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
            .unwrap_or_else(|_| panic!("the outbound queue must have room"));
        settle().await;
        assert!(
            matches!(raced.try_recv(), Err(oneshot::error::TryRecvError::Empty)),
            "the supervisor must be stuck on a blocked close — this request is the cleanup's job"
        );
        endpoint.stall_writes(false);
        settle().await;
        assert!(h.supervisor_done(), "the cleanup closed the outbound queue");
        assert!(
            matches!(raced.try_recv(), Ok(Err(RequestError::Disconnected { .. }))),
            "★a request caught in the cleanup ends right away without waiting for the ordering area to drain★"
        );
        assert!(
            matches!(
                joined(spawn_request(&h.peer, 8)).await,
                Err(RequestError::Disconnected { .. })
            ),
            "a request issued during cleanup also ends right away"
        );
        assert!(
            h.peer.notify(TestOut::Notice("late".into())).is_err(),
            "a notification put in during cleanup is refused right away"
        );
        assert!(
            !h.inbound.is_closed(),
            "the cleanup is still waiting on the ordering area — the supervisor did not release the enqueue end"
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
            settle_until("folded", || h.peer.peer_state() == PeerState::Closed).await;
            h.clock.advance(waited);
            settle().await;

            let items = drain_until_quiet(&mut h).await;
            assert_eq!(labels(&items), expected, "after waiting {waited:?}");
            assert!(
                matches!(
                    h.inbound.try_recv(),
                    Err(mpsc::error::TryRecvError::Disconnected)
                ),
                "{waited:?}: the supervisor ended"
            );
        }
    }

    #[tokio::test]
    async fn a_teardown_ends_at_once_when_the_consumer_is_gone() {
        let (mut h, endpoint) = live_on_a_full_line().await;
        endpoint.push(TestWire::encode_in(&TestIn::Notice("held".into())));
        settle().await;
        h.peer.close();
        settle_until("folded", || h.peer.peer_state() == PeerState::Closed).await;
        assert_eq!(
            h.clock.sleepers(),
            1,
            "the cleanup waits with the cap clock set"
        );

        drop_the_consumer(&mut h);
        settle_until(
            "if the receiver vanishes, the cleanup does not wait",
            || h.task.is_finished(),
        )
        .await;
        assert_eq!(h.clock.sleepers(), 0, "released the cap clock");
        assert!(
            h.exited_cleanly().await,
            "the supervisor ended without panicking"
        );
    }

    fn drop_the_consumer(h: &mut Harness) {
        drop(std::mem::replace(&mut h.inbound, mpsc::channel(1).1));
    }

    // ── 3-0 U1b: if the receiving side disappears, any waiting loop folds without waiting for the clock ──
    //
    // ★Do not advance the clock★ — the backoff would end or the deadline hit and the loop would get out without that decision.

    #[tokio::test]
    async fn a_backing_off_peer_holding_items_folds_when_the_consumer_goes() {
        let (mut h, endpoint) = live_on_a_full_line().await;
        // What is held is the disconnect event — a notification before it would line the close up behind it, unread (backpressure).
        endpoint.close();
        settle_until("backoff", || h.peer.peer_state() == PeerState::Backoff).await;
        assert_eq!(
            h.inbound.capacity(),
            0,
            "the queue is full, so the ordering area is holding"
        );

        drop_the_consumer(&mut h);
        settle_until("folded", || h.peer.peer_state() == PeerState::Closed).await;
        assert_eq!(h.net.dials(), 1, "did not redial");
        settle_until("the supervisor ended", || h.task.is_finished()).await;
        assert!(h.exited_cleanly().await);
    }

    #[tokio::test]
    async fn a_peer_that_gave_up_holding_items_folds_when_the_consumer_goes() {
        let (mut h, endpoint) = live_on_a_full_line().await;
        h.net.fail_forever();
        endpoint.close();
        settle_until("backoff", || h.peer.peer_state() == PeerState::Backoff).await;
        for delay in [500u64, 1000, 2000] {
            h.clock.advance(Duration::from_millis(delay));
            settle().await;
        }
        assert!(matches!(h.peer.peer_state(), PeerState::GaveUp(_)));
        assert_eq!(
            h.inbound.capacity(),
            0,
            "the queue is full, so the ordering area is holding"
        );

        drop_the_consumer(&mut h);
        settle_until("folded", || h.peer.peer_state() == PeerState::Closed).await;
        settle_until("the supervisor ended", || h.task.is_finished()).await;
        assert!(h.exited_cleanly().await);
    }

    #[tokio::test]
    async fn a_mid_handshake_peer_holding_items_folds_when_the_consumer_goes() {
        let mut h = build_with(Policy::default(), Arc::new(ListeningHandshake), 1);
        settle_until("waiting for handshake", || {
            h.peer.peer_state() == PeerState::Handshaking
        })
        .await;
        let first = h.net.accept().unwrap();
        first.push(Frame::Text("welcome".into()));
        settle_until("connection", || h.peer.peer_state() == PeerState::Live).await;
        first.push(TestWire::encode_in(&TestIn::Notice("n0".into())));
        settle().await;
        // The same line as in `held_items_keep_flowing_while_the_supervisor_is_mid_handshake` — make it reconnect by control.
        h.peer.reconnect_now();
        settle_until("waiting for the second handshake", || {
            h.peer.peer_state() == PeerState::Handshaking
        })
        .await;
        assert_eq!(
            h.inbound.capacity(),
            0,
            "the queue is full, so the ordering area is holding"
        );

        drop_the_consumer(&mut h);
        settle_until("folded", || h.peer.peer_state() == PeerState::Closed).await;
        assert_eq!(
            h.peer.generation(),
            Generation(1),
            "did not draw a second generation"
        );
        settle_until("the supervisor ended", || h.task.is_finished()).await;
        assert!(h.exited_cleanly().await);
    }

    // ── 3-0 U1d: even with nothing held, if the receiving side disappears it folds on the spot ──
    //
    // Above, the ordering area holds so the drain arm sees the disappearance; here nothing is held, so only the receiver-end closing arm
    // (`Sender::closed`) in each waiting loop sees it — without it backoff · give-up · handshake · quiet operation would stay until the clock or
    // the next event. The clock is not advanced here either.

    #[tokio::test]
    async fn a_waiting_peer_with_nothing_held_folds_when_the_consumer_goes() {
        for gave_up in [false, true] {
            let (mut h, endpoint) = live().await;
            if gave_up {
                h.net.fail_forever();
            }
            endpoint.close();
            settle_until("backoff", || h.peer.peer_state() == PeerState::Backoff).await;
            if gave_up {
                for delay in [500u64, 1000, 2000] {
                    h.clock.advance(Duration::from_millis(delay));
                    settle().await;
                }
                assert!(matches!(h.peer.peer_state(), PeerState::GaveUp(_)));
            }
            assert!(
                h.inbound.capacity() > 0,
                "gave_up={gave_up}: the queue has room — the ordering area holds nothing"
            );
            let dials = h.net.dials();
            drop_the_consumer(&mut h);
            settle_until("folded", || h.peer.peer_state() == PeerState::Closed).await;
            assert_eq!(h.net.dials(), dials, "gave_up={gave_up}: did not redial");
            settle_until("the supervisor ended", || h.task.is_finished()).await;
            assert!(h.exited_cleanly().await, "gave_up={gave_up}");
        }
    }

    #[tokio::test]
    async fn a_mid_handshake_peer_with_nothing_held_folds_when_the_consumer_goes() {
        let mut h = build(Policy::default(), Arc::new(ListeningHandshake));
        settle_until("waiting for handshake", || {
            h.peer.peer_state() == PeerState::Handshaking
        })
        .await;
        drop_the_consumer(&mut h);
        settle_until("folded", || h.peer.peer_state() == PeerState::Closed).await;
        assert_eq!(h.net.dials(), 1, "did not redial");
        assert_eq!(
            h.peer.generation(),
            Generation(0),
            "did not draw a generation"
        );
        settle_until("the supervisor ended", || h.task.is_finished()).await;
        assert!(h.exited_cleanly().await);
    }

    #[tokio::test]
    async fn a_silent_live_peer_folds_without_another_frame_when_the_consumer_goes() {
        let (mut h, endpoint) = live().await;
        drop_the_consumer(&mut h);
        settle_until("folded", || h.peer.peer_state() == PeerState::Closed).await;
        settle_until("the supervisor ended — did not wait for the cap", || {
            h.task.is_finished()
        })
        .await;
        assert_eq!(
            endpoint.drain(),
            vec![ClientMsg::Close(Close::new(
                CloseCode::GOING_AWAY,
                "closing"
            ))],
            "★the peer was quiet and all we sent was the close★"
        );
        assert_eq!(h.net.dials(), 1, "did not redial");
        assert!(h.exited_cleanly().await);
    }

    // Receiver disappearance and a 「reconnect now」 ready together — whichever arm wakes first, it does not dial again.
    #[tokio::test]
    async fn a_reconnect_racing_the_consumers_exit_does_not_dial() {
        let (mut h, _endpoint) = live().await;
        drop_the_consumer(&mut h);
        h.peer.reconnect_now();
        settle_until("folded", || h.peer.peer_state() == PeerState::Closed).await;
        assert_eq!(
            h.net.dials(),
            1,
            "did not redial a daemon nobody will listen to"
        );
        assert_eq!(h.peer.generation(), Generation(1));
        settle_until("the supervisor ended", || h.task.is_finished()).await;
        assert!(h.exited_cleanly().await);
    }

    // ── 3-0 U1d: end cause — tell the read end from the write end ──
    //
    // The daemon adapter uses this to tell the read end (finish remaining commands) from the write end (cut at once) (TRD 3 §3-0 ①). The endpoint is
    // not dropped — both sides would be cut together and which is learned first would depend on scheduling.

    type Cut = fn(&Harness, &MemoryEndpoint);
    type ReadEnd = fn(&MemoryEndpoint);
    type AppAct = fn(&Harness);

    #[tokio::test]
    async fn a_read_failure_and_a_write_failure_arrive_as_different_causes() {
        let cases: [(&str, Cut, DisconnectCause); 3] = [
            (
                "read error",
                |_, e| e.fail("boom"),
                DisconnectCause::ReadFailed,
            ),
            (
                "notification write failure",
                |h, e| {
                    e.fail_writes();
                    h.peer.notify(TestOut::Notice("x".into())).unwrap();
                },
                DisconnectCause::WriteFailed,
            ),
            (
                "keepalive write failure",
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
            settle_until("backoff", || h.peer.peer_state() == PeerState::Backoff).await;
            assert_eq!(h.disconnect_cause(), Some(expected), "{what}");
        }
    }

    // ── 3-0 U1d: when dropping the channel, a read end already in the read-ahead queue becomes the disconnect cause ──
    //
    // Line length 1 — the first notification is held in the ordering area and the loop pulls the frame arm (backpressure). The peer's close · read
    // error after it stays unread in the read-ahead queue, and the supervisor leaves first with its own cause (write failure · write deadline · overflow …).

    async fn a_read_end_behind_a_held_line(
        policy: Policy,
        end: ReadEnd,
    ) -> (Harness, Arc<MemoryEndpoint>) {
        let h = build_with(policy, Arc::new(ImmediateHandshake), 1);
        settle_until("first connection", || {
            h.peer.peer_state() == PeerState::Live
        })
        .await;
        let endpoint = h.net.accept().expect("the link must have opened");
        endpoint.push(TestWire::encode_in(&TestIn::Notice("held".into())));
        settle().await;
        end(&endpoint);
        settle().await;
        assert_eq!(
            endpoint.unread(),
            0,
            "that end went into the read-ahead queue"
        );
        assert_eq!(
            h.peer.peer_state(),
            PeerState::Live,
            "the supervisor with the frame arm removed has not read that end yet"
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
            "notification write failure",
            Policy::default,
            &[|h, e| {
                e.fail_writes();
                h.peer.notify(TestOut::Notice("x".into())).unwrap();
            }],
        ),
        (
            "keepalive write failure",
            Policy::default,
            &[|h, e| {
                e.fail_writes();
                h.clock.advance(Policy::default().ping_interval);
            }],
        ),
        // Release the stall after the deadline cuts the stuck write — the close that drops the channel must go out before entering backoff.
        (
            "write deadline",
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
        // One slot in the outbound queue — the second notification cannot get in and is counted; the supervisor cuts at the next turn start after writing the first (default policy).
        (
            "outbound queue overflow",
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
            (
                "the peer's close",
                |e| e.close(),
                "Disconnected:1:PeerClosed",
            ),
            (
                "read error",
                |e| e.fail("boom"),
                "Disconnected:1:ReadFailed",
            ),
        ];
        for (end_what, end, expected) in ends {
            for (leave_what, policy, steps) in LEAVES {
                let (mut h, endpoint) = a_read_end_behind_a_held_line(policy(), end).await;
                for step in steps {
                    step(&h, &endpoint);
                    settle().await;
                }
                settle_until("backoff", || h.peer.peer_state() == PeerState::Backoff).await;
                let items = drain_until_quiet(&mut h).await;
                assert_eq!(
                    disconnects(&items),
                    vec![expected.to_string()],
                    "{leave_what} after {end_what}: ★the read end that happened first is the cause★"
                );
            }
        }
    }

    // An app close or reconnect request keeps its cause (`Explicit`) — the caller knows.
    #[tokio::test]
    async fn an_app_close_or_reconnect_stays_explicit_with_a_read_end_read_ahead() {
        let acts: [(&str, AppAct); 2] = [
            ("close", |h| h.peer.close()),
            ("redial", |h| h.peer.reconnect_now()),
        ];
        for (what, act) in acts {
            let (mut h, _endpoint) =
                a_read_end_behind_a_held_line(Policy::default(), |e| e.close()).await;
            act(&h);
            settle_until("folded or redialed", || {
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

    // If our cause is already the reader's end (cut on decode failure) it stays — within the line that frame precedes the close that followed.
    #[tokio::test]
    async fn a_decode_failure_cut_keeps_its_cause_over_a_close_read_behind_it() {
        let mut h = build(
            Policy {
                decode_failure: DecodeFailurePolicy::Disconnect,
                ..Policy::default()
            },
            Arc::new(ImmediateHandshake),
        );
        settle_until("connection", || h.peer.peer_state() == PeerState::Live).await;
        let endpoint = h.net.accept().unwrap();
        h.drain();
        endpoint.push(Frame::Text("garbage".into()));
        endpoint.close();
        settle_until("backoff", || h.peer.peer_state() == PeerState::Backoff).await;
        assert_eq!(
            endpoint.unread(),
            0,
            "the close was read too and went into the read-ahead queue"
        );
        assert_eq!(h.disconnect_cause(), Some(DisconnectCause::ReadFailed));
    }

    // Same inside the salvaged line — the decode-failure frame precedes the close that followed, so that cause stays; what was read after it is still
    //   handed over (`salvaging_carries_on_past_a_frame_it_cannot_decode`).
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
        settle_until("backoff", || h.peer.peer_state() == PeerState::Backoff).await;
        let items = drain_until_quiet(&mut h).await;
        let seen = labels(&items);
        assert!(
            seen.contains(&"msg:after".to_string()),
            "also handed over what was already read after the one that cuts: {seen:?}"
        );
        assert_eq!(disconnects(&items), vec!["Disconnected:1:ReadFailed"]);
    }

    // ── 3-0 U1d: even if the receiving side disappears mid-write, it does not wait for that write ──
    //
    // One stuck write and one stuck keepalive at the back-to-back queue drain (`Turn::Outbound`) — both must fold without advancing the clock;
    // without the write arm it stands up to `write_deadline` per stuck write.

    #[tokio::test]
    async fn a_consumer_leaving_mid_batch_folds_without_waiting_out_the_stalled_write() {
        let mut h = build(
            Policy {
                outbound_queue: 4,
                ..Policy::default()
            },
            Arc::new(ImmediateHandshake),
        );
        settle_until("connection", || h.peer.peer_state() == PeerState::Live).await;
        let endpoint = h.net.accept().unwrap();
        // Stand on a keepalive and fill the queue past half, then block the write and release the keepalive — the next turn drains in a batch and stands on the first write.
        endpoint.stall_pings(true);
        h.clock.advance(Policy::default().ping_interval);
        settle().await;
        for n in 0..3 {
            h.peer
                .notify(TestOut::Notice(format!("n{n}")))
                .expect("outbound queue");
        }
        endpoint.stall_writes(true);
        endpoint.stall_pings(false);
        settle().await;
        assert_eq!(endpoint.drain(), vec![ClientMsg::Ping]);
        assert_eq!(
            h.peer.inner.data_tx.capacity(),
            2,
            "the supervisor took one out and stalled writing it, and two remain in the queue"
        );

        drop_the_consumer(&mut h);
        settle_until("folded — the clock was not advanced", || {
            h.peer.peer_state() == PeerState::Closed
        })
        .await;
        // The close that drops the channel is caught by the write block — a place we deliberately do not listen at; release it (`drop_link`).
        endpoint.stall_writes(false);
        settle_until("the supervisor ended", || h.task.is_finished()).await;
        assert_eq!(
            endpoint.drain(),
            vec![ClientMsg::Close(Close::new(
                CloseCode::GOING_AWAY,
                "closing"
            ))],
            "★neither the pending write nor the remaining two went out★"
        );
        assert!(h.exited_cleanly().await);
    }

    #[tokio::test]
    async fn a_consumer_leaving_mid_keepalive_folds_without_waiting_out_the_stalled_ping() {
        let (mut h, endpoint) = live().await;
        endpoint.stall_pings(true);
        h.clock.advance(Policy::default().ping_interval);
        settle().await;
        assert!(endpoint.drain().is_empty(), "stuck on keepalive");

        drop_the_consumer(&mut h);
        settle_until("folded — the clock was not advanced", || {
            h.peer.peer_state() == PeerState::Closed
        })
        .await;
        settle_until("the supervisor ended", || h.task.is_finished()).await;
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
        settle_until("first connection", || {
            h.peer.peer_state() == PeerState::Live
        })
        .await;
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
            "★while the ordering area holds, no more frames are taken out and the link backs up★ — the ordering area does not grow"
        );
        assert_eq!(h.peer.held_bytes(), read * "not|n00".len());
        assert_eq!(
            h.peer.peer_state(),
            PeerState::Live,
            "backing up is not a reason to cut"
        );

        let items = drain_until_quiet(&mut h).await;
        assert_eq!(
            notices(&items),
            (0..20).map(|n| format!("n{n:02}")).collect::<Vec<_>>(),
            "★even under backpressure, nothing is dropped and items come out in arrival order★"
        );
        assert_eq!(endpoint.unread(), 0);
        assert_eq!(
            h.peer.held_bytes(),
            0,
            "the share taken out leaves the count"
        );
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
        settle_until("first connection", || {
            h.peer.peer_state() == PeerState::Live
        })
        .await;
        let endpoint = h.net.accept().unwrap();
        h.drain();
        for tag in 0..5u8 {
            endpoint.push(blob(tag, 150));
        }
        settle().await;
        assert_eq!(
            endpoint.unread(),
            4,
            "★an empty count accepts even one item larger than the budget, and after that the link is not read★ (fill level before adding)"
        );
        assert_eq!(h.peer.held_bytes(), 150);

        let mut tags = Vec::new();
        for _ in 0..5 {
            let taken = h.drain();
            assert_eq!(
                blob_tags(&taken).len(),
                1,
                "only one item is read at a time"
            );
            tags.extend(blob_tags(&taken));
            settle().await;
        }
        assert_eq!(tags, vec![0, 1, 2, 3, 4], "★0 loss · order as is★");
        assert_eq!(endpoint.unread(), 0);
        assert_eq!(h.peer.held_bytes(), 0);
    }

    // ★Line length 1 uses all three layers★ — the first frame in the ordering area, the second in the read-ahead queue. Worst case is reading one more
    //   frame right under the read share (no streams, so the whole budget), hence a frame 1 smaller than the read share. The ceiling with streams and a
    //   full window share is `holding_a_window_full_and_a_read_share_full_still_fits_the_budget_plus_one_message`.
    #[tokio::test]
    async fn what_a_peer_holds_never_exceeds_the_budget_plus_one_message() {
        let budget = 1000;
        let message = budget - 1;
        let mut h = build_budgeted(Policy::default(), Arc::new(ImmediateHandshake), 1, budget);
        settle_until("first connection", || {
            h.peer.peer_state() == PeerState::Live
        })
        .await;
        let endpoint = h.net.accept().unwrap();
        let sent = 10u8;
        for tag in 0..sent {
            endpoint.push(blob(tag, message));
            settle().await;
            assert!(
                h.peer.held_bytes() <= budget + message,
                "held {} bytes after sending #{tag} — the sum of the read-ahead queue · ordering area · queue exceeded B + M",
                h.peer.held_bytes()
            );
        }
        assert_eq!(
            h.peer.held_bytes(),
            2 * message,
            "★a supervisor that does not use streams uses the whole budget as the read share★ — it read one more item below that and stopped there"
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
            "★0 loss · order as is★"
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
        assert!(h.peer.held_bytes() > 0, "the ordering area is holding");

        for _ in 0..3 {
            h.clock.advance(Duration::from_secs(20));
            settle().await;
            assert!(
                endpoint.drain().contains(&ClientMsg::Ping),
                "★our pings go out even while holding★ — otherwise the peer cuts us for silence"
            );
        }
        assert_eq!(h.peer.peer_state(), PeerState::Live);

        h.peer.close();
        settle_until("the close is heard even while holding", || {
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
            "{what}: ★we are the ones who blocked★ — the holding time must not count as the peer's silence"
        );

        drain_until_quiet(&mut h).await;
        h.clock.advance(Duration::from_secs(49));
        settle().await;
        assert_eq!(
            h.peer.peer_state(),
            PeerState::Live,
            "{what}: ★the moment of release is `last_rx`★ — counting from the last receipt before it would cut right away"
        );
        h.clock.advance(Duration::from_secs(1));
        settle().await;
        assert_eq!(
            h.disconnect_cause(),
            Some(DisconnectCause::Idle),
            "{what}: after release, the silence decision runs again"
        );
    }

    #[tokio::test]
    async fn holding_past_the_idle_timeout_on_a_full_line_is_not_silence() {
        let (h, endpoint) = live_on_a_full_line().await;
        endpoint.push(TestWire::encode_in(&TestIn::Notice("held".into())));
        settle().await;
        idle_restarts_from_the_release(h, "ordering area").await;
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
        settle_until("first connection", || {
            h.peer.peer_state() == PeerState::Live
        })
        .await;
        let endpoint = h.net.accept().unwrap();
        h.drain();
        // ★No liveness check after it★ — read right at release it would rewind `last_rx` and the release's rewind could not be measured.
        endpoint.push(blob(0, 2 * budget));
        settle().await;
        assert!(
            h.peer.held_bytes() >= budget,
            "one item in the queue exceeded the budget"
        );
        idle_restarts_from_the_release(h, "byte budget").await;
    }

    // ★Make the release slip into a timer turn (`Turn::Timer`) followed by a progress check★ — `on_timer` skipped the silence decision by the start's
    //   「held」, but a wake time re-measured by backpressure there would use the stale `last_rx`'s (already passed) silence deadline and the progress
    //   check (`debug_assert`) would panic the supervisor. To reach that turn the ping time must pass while the supervisor is outside `select`: stand it on
    //   a blocked notification write; on release it goes to that turn's ping without yielding, so block only the ping and let the receiving side drain inside it.
    #[tokio::test]
    async fn a_release_inside_a_timer_round_does_not_trip_the_progress_check() {
        let budget = 100;
        let mut h = build_budgeted(
            Policy::default(),
            Arc::new(ImmediateHandshake),
            4096,
            budget,
        );
        settle_until("first connection", || {
            h.peer.peer_state() == PeerState::Live
        })
        .await;
        let endpoint = h.net.accept().unwrap();
        endpoint.push(blob(0, 2 * budget));
        settle().await;
        assert!(h.peer.held_bytes() >= budget, "holds by the budget alone");
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
        assert!(
            !h.task.is_finished(),
            "waits at the ping of the timer round"
        );
        h.drain();
        endpoint.stall_pings(false);
        settle().await;
        assert!(
            !h.task.is_finished(),
            "★even if released inside a timer round entered while holding, the progress check does not blow up★"
        );
        assert_eq!(h.peer.peer_state(), PeerState::Live);
        assert!(endpoint.drain().contains(&ClientMsg::Ping));
    }

    // ── 3-0 U1c: resume window · backpressure ──

    fn sized_chunk(seq: u64, len: usize) -> Frame {
        let head = format!("chk|1|1|{seq}|");
        assert!(
            len >= head.len(),
            "{len} bytes cannot even carry the header"
        );
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
        settle_until("first connection", || {
            h.peer.peer_state() == PeerState::Live
        })
        .await;
        let endpoint = h.net.accept().unwrap();
        endpoint.push(TestWire::chunk(1, 1, 1));
        assert_eq!(chunk_seqs(&drain_until_quiet(&mut h).await), vec![1]);
        (h, endpoint)
    }

    // ★Grow the held pieces by the window cap★ — the window cap is at least the read share; if that share went into the read gate it would close, the
    //   replay would stay in the channel and could not fill the hole, and a replay arriving after the deadline close is dropped as a past number.
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
            "saw the gap and issued a re-request"
        );
        assert_eq!(
            h.peer.window_bytes(),
            window_cap,
            "held pieces stand in the window share"
        );

        endpoint.push(TestWire::chunk(1, 1, 2));
        endpoint.push(TestWire::chunk(1, 1, 3));
        settle().await;
        assert_eq!(
            endpoint.unread(),
            0,
            "★even if held pieces reach the window cap, the replay is read★"
        );
        let items = h.drain();
        assert_eq!(
            chunk_seqs(&items),
            vec![2, 3],
            "★the gap is filled and items come in order★"
        );
        assert_eq!(truncations(&items), Vec::<u64>::new(), "not a truncation");
        assert_eq!(
            h.peer.window_bytes(),
            0,
            "the answer came and the held items were discarded"
        );
    }

    // ★The receiving side stops after the window opens★ — fill line length 1 with a notification so the hole event stands in the ordering area while the
    //   frame arm is pulled and the replay stays in the read-ahead queue; counting the window deadline as is would close the window and drop the replay.
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
            "the replay is in the read-ahead queue"
        );

        // Passes the window deadline (2 s) and the ping time (20 s) too, letting the timer run once while held.
        h.clock.advance(Duration::from_secs(3));
        settle().await;
        h.clock.advance(Duration::from_secs(20));
        settle().await;
        assert!(
            endpoint.drain().contains(&ClientMsg::Ping),
            "the timer ran while holding"
        );
        assert_eq!(h.peer.peer_state(), PeerState::Live);

        let items = drain_until_quiet(&mut h).await;
        assert_eq!(
            truncations(&items),
            Vec::<u64>::new(),
            "★we are the ones who blocked★ — counting the holding time against the window deadline folds a fillable gap into a truncation"
        );
        assert_eq!(
            chunk_seqs(&items),
            vec![2, 3],
            "★after release, the replay fills the gap★"
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
            "★the share of pieces coming out of the window returns to the read gate★ — if it stayed in the window share, a slow receiver alone would keep the next gap \
              from being held"
        );
        let items = h.drain();
        assert_eq!(
            truncations(&items),
            vec![3],
            "★exceeding the window cap is reported as a truncation, not silently dropped★ — the first held number is the lower bound"
        );
        assert_eq!(
            chunk_seqs(&items),
            vec![3, 4],
            "emits the held items in order"
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
            "★pieces held by the window are bytes the supervisor holds too★"
        );

        let answer = TestWire::chunk(1, 1, 2).payload_len();
        endpoint.push(TestWire::chunk(1, 1, 2));
        settle().await;
        assert_eq!(
            (h.peer.held_bytes(), h.peer.window_bytes()),
            (answer, 0),
            "when the answer comes and the window closes, the held share leaves and only the forwarded answer remains"
        );
        h.drain();
        assert_eq!(h.peer.held_bytes(), 0);
    }

    // ── 3-0 U1c: salvage the read-ahead queue when dropping the channel ──
    //
    // ★Line length 1★ — n0 in the ordering area, n1 · n2 in the read-ahead queue with the frame arm pulled.

    async fn three_notices_held(h: &Harness, endpoint: &MemoryEndpoint) {
        for n in 0..3 {
            endpoint.push(TestWire::encode_in(&TestIn::Notice(format!("n{n}"))));
        }
        settle().await;
        assert_eq!(
            h.peer.held_bytes(),
            3 * "not|n0".len(),
            "n0 is in the ordering area, n1 · n2 in the read-ahead queue"
        );
        assert_eq!(endpoint.unread(), 0);
    }

    // ★Fill the read-ahead queue completely (2 slots)★ — while the read task waits for the next slot no frame may be in hand, so salvaged plus
    //   what stayed in the channel equals everything sent.
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
        settle_until("first connection", || {
            h.peer.peer_state() == PeerState::Live
        })
        .await;
        let endpoint = h.net.accept().unwrap();
        for n in 0..5 {
            endpoint.push(TestWire::encode_in(&TestIn::Notice(format!("n{n}"))));
        }
        settle().await;
        assert_eq!(
            endpoint.unread(),
            2,
            "n0 in the ordering area, n1 · n2 in the read-ahead queue"
        );
        h.peer.reconnect_now();
        settle_until("new link", || h.net.dials() == 2).await;

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
            "★even when redialing, what was already read is not dropped but delivered with the old generation before the disconnect★"
        );
        assert_eq!(
            endpoint.unread(),
            2,
            "n3 · n4 remained on the old link — nothing was read and then dropped"
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
            "★even if a write fails, what was already read comes before the disconnect★ — the report of the notification that could not go out is emitted by the backoff"
        );
    }

    // ★A re-attached channel uses the same count★ — the new channel is not read until the share the old one left drops below the read share.
    #[tokio::test]
    async fn a_new_link_waits_for_what_the_old_link_left_to_drop_below_the_budget() {
        let budget = 100;
        let mut h = build_budgeted(
            Policy::default(),
            Arc::new(ImmediateHandshake),
            4096,
            budget,
        );
        settle_until("first connection", || {
            h.peer.peer_state() == PeerState::Live
        })
        .await;
        let first = h.net.accept().unwrap();
        first.push(blob(0, 2 * budget));
        settle().await;
        assert_eq!(h.peer.held_bytes(), 2 * budget, "leaves it in the queue");

        h.peer.reconnect_now();
        settle_until("reattached", || h.net.dials() == 2).await;
        settle_until("operating", || h.peer.peer_state() == PeerState::Live).await;
        let second = h.net.accept().unwrap();
        second.push(blob(1, 10));
        settle().await;
        assert_eq!(
            second.unread(),
            1,
            "★if the old link's share is at least the read share, the new link is not read★"
        );

        let items = drain_until_quiet(&mut h).await;
        assert_eq!(blob_tags(&items), vec![0, 1]);
        assert_eq!(
            second.unread(),
            0,
            "once the old share left, the new link was read"
        );
        assert_eq!(h.peer.held_bytes(), 0);
    }

    // ★Redialing with a reply in the read-ahead queue, the waiting caller still receives it★ — salvaging precedes the machine's waiter wake-up.
    #[tokio::test]
    async fn a_reply_already_read_reaches_its_caller_when_reconnecting() {
        let (mut h, endpoint) = live_on_a_full_line().await;
        let asked = spawn_request(&h.peer, 7);
        settle().await;
        endpoint.push(TestWire::encode_in(&TestIn::Notice("held".into())));
        endpoint.push(TestWire::reply(7));
        settle().await;
        assert_eq!(endpoint.unread(), 0);
        assert!(!asked.is_finished(), "the reply is in the read-ahead queue");

        h.peer.reconnect_now();
        assert_eq!(
            joined(asked).await,
            Ok(TestIn::Reply {
                tag: 7,
                body: "ok".into(),
            }),
            "★a reply already read does not fail because of the disconnect★"
        );
        drain_until_quiet(&mut h).await;
    }

    // ── 3-0 U1c: one budget covers both the read share and the window cap (B + M) ──

    // ★Fill the read share too while the window is full★ — with the window cap = the budget (two counts), the window would hold four (96) and reading two
    //   more (98) would exceed B + M (149); with half, the window closes as a truncation at the third piece and that share closes the read gate.
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
                "held {} bytes — the whole count exceeded B + M",
                h.peer.held_bytes()
            );
            assert!(h.peer.window_bytes() <= split(budget).1);
        }

        let items = drain_until_quiet(&mut h).await;
        assert_eq!(chunk_seqs(&items), vec![3, 4, 5, 6]);
        assert_eq!(truncations(&items), vec![3]);
        assert_eq!(blob_tags(&items), vec![0, 1, 2], "★0 loss · order as is★");
        assert_eq!(h.peer.held_bytes(), 0);
    }

    // ★Only a supervisor that has seen a stream piece halves the read gate★ — three frames (30 · 30 · 30) at budget 100: a non-stream supervisor reads the
    //   third too (60 < 100); one that saw a piece hits the read share (50) at the second and leaves the third in the channel.
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
            settle_until("first connection", || {
                h.peer.peer_state() == PeerState::Live
            })
            .await;
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
                "saw stream pieces = {streams}"
            );
            assert_eq!(
                blob_tags(&drain_until_quiet(&mut h).await),
                vec![0, 1, 2],
                "★0 loss · order as is★"
            );
        }
    }

    // ── 3-0 U1c: progress · clock · generation · teardown of held pieces ──

    // ★The piece that hit the byte limit is not the largest number held★ — progress set to its number would let replay 4 · 5 go up again after 5 (order reversed · overlap).
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
            "★always ascending · no overlap★"
        );
    }

    // ★The hold starts while waiting on a write★ — the supervisor notices it only after the write ends. Counting from the notice would charge the 3 s on
    //   the write to the window deadline (2 s), close the window on release and drop the replay as a past number.
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
            "while stuck on the write, the read gate closed and the replay stayed on the link"
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
            "★holding counts from when it started★ — counting from when it was noticed lets the time waiting on the write eat up the window deadline"
        );
        assert_eq!(
            chunk_seqs(&items),
            vec![2, 3],
            "★after release, the replay fills the gap★"
        );
        assert_eq!(blob_tags(&items), vec![0]);
        assert_eq!(h.peer.peer_state(), PeerState::Live);
    }

    // Ordering-area edition of the test above — the hole event stands in the ordering area because of a full line and the re-request write is blocked;
    //   counting from when the ordering area started holding keeps the write time out of the window deadline.
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

    // Loss-report edition of the test above — during the write the receiving side empties the line and the outbound queue overflows. The report at
    //   the next turn start loads even the hole event and empties the ordering area; if the hold is not seen before it, the mid-write hold is lost and
    //   the deadline closes the window before the read-ahead queue's replay.
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
        settle_until("first connection", || {
            h.peer.peer_state() == PeerState::Live
        })
        .await;
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
            "while stuck on the write, the receiver drained the queue — the gap event is still in the ordering area"
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

    // ★The read gate closes during a write and reopens before it ends★ — the supervisor sees only the open gate at the turn start and never the hold.
    //   Unpushed, the 3 s on the write exceeds the window deadline (2 s), the deadline beats the read-ahead queue's replay and closes the window, and
    //   replay 2 is dropped as a past number.
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
            "while stuck on the write, the read gate closed and the replay stayed on the link"
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
            "the receiver drained and the read gate opened again — the write is still stuck"
        );
        endpoint.stall_writes(false);

        let items = drain_until_quiet(&mut h).await;
        assert_eq!(
            truncations(&items),
            Vec::<u64>::new(),
            "★a hold that was never seen is subtracted from the window deadline too★"
        );
        assert_eq!(chunk_seqs(&items), vec![2, 3], "★the replay fills the gap★");
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
        sup.streams
            .next_resume_deadline()
            .expect("the window opened")
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
            "★pushes both sections★ — the 1 s open in between is not pushed"
        );
        assert_eq!(
            sup.last_rx,
            clock.now(),
            "the release of an unseen hold is a release too"
        );
    }

    // ★An unseen span straddling the ordering area's hold counts as held from that first span★ — looking only at when the ordering area started
    //   would lose the 1 s closed before, and pushing the two separately would push the overlapping 1 s twice.
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
            "from when the read gate closed (1 s) to when the release was noticed (5 s)"
        );
    }

    // ★An unseen span that ended before the current hold is pushed separately★ — pulling the hold's start back to that span would push the open time in between.
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
            "the unseen 2 s + the current hold (closed at 4 s, noticed at 6 s) 2 s — the 1 s open in between is not pushed"
        );
    }

    // ★A span closed and opened while we knew it was held is not pushed again even if the count hands it over separately★ — it is all after the hold
    //   started, so already inside the time pushed at release.
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
            "only once, from 1 s when the hold started to 5 s when the release was noticed"
        );
    }

    // ★Even under a cut-on-decode-failure policy, salvaging hands over what follows★ — during operation what follows that frame is handed over by
    //   salvaging; stopping mid-salvage would hand over or drop the same read-ahead queue depending on where it was cut.
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
        settle_until("first connection", || {
            h.peer.peer_state() == PeerState::Live
        })
        .await;
        let endpoint = h.net.accept().unwrap();
        endpoint.push(TestWire::encode_in(&TestIn::Notice("n0".into())));
        endpoint.push(Frame::Text("garbage".into()));
        endpoint.push(TestWire::encode_in(&TestIn::Notice("n1".into())));
        settle().await;
        assert_eq!(
            endpoint.unread(),
            0,
            "n0 in the ordering area, the rest in the read-ahead queue"
        );
        h.peer.reconnect_now();
        settle_until("new link", || h.net.dials() == 2).await;

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
        settle_until("reattached", || h.net.dials() == 2).await;
        settle_until("operating", || h.peer.peer_state() == PeerState::Live).await;
        let second = h.net.accept().unwrap();
        settle().await;
        assert_eq!(
            second.frames(),
            vec![Frame::Text("res|1|1".into())],
            "the window survived and requested again on the new link"
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
        assert_eq!(
            truncations(&items),
            vec![3],
            "the window closed on the second link"
        );
        assert_eq!(
            held,
            vec![(3, 1)],
            "★it is the generation of the connection that carried it★ — stamping the generation of the connection where the window closed (2) breaks the `Arrival::generation` contract"
        );
    }

    // ★Teardown folds the open window before dropping the ledger★ — else held pieces vanish without delivery or truncation report.
    #[tokio::test]
    async fn closing_hands_over_a_held_chunk_as_a_truncation_before_closed() {
        let (mut h, endpoint) = streaming(BYTE_BUDGET, 4096).await;
        endpoint.push(TestWire::chunk(1, 1, 3));
        settle().await;
        h.peer.close();
        settle_until("folded", || h.peer.peer_state() == PeerState::Closed).await;

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
        assert_eq!(
            truncations(&items),
            vec![3],
            "the first held number is the lower bound"
        );
        assert_eq!(h.peer.held_bytes(), 0);
    }

    // ── 3-0 U3: pre-close flush ──
    //
    // ★Put the close code outside the crate's reserved range★ — equal to the default close (`GOING_AWAY` · "closing"), 「loaded the close we were given」 cannot be measured.

    const BYE: CloseCode = CloseCode(CloseCode::CONSUMER_BASE);

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
                .expect("outbound queue");
        }
    }

    #[tokio::test]
    async fn a_flush_writes_every_notify_accepted_before_it_then_the_given_close() {
        let (mut h, endpoint) = live().await;
        notify_all(&h.peer, 5);
        let done = h.peer.close_after_flush(BYE, "bye");
        settle_until("the supervisor ended", || h.task.is_finished()).await;

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
            "★the given close goes out after the five accepted earlier have all gone out★"
        );
        assert_eq!(
            poll_immediate(done).await,
            Some(()),
            "the finished future resolved"
        );
        assert_eq!(
            labels(&drain_until_quiet(&mut h).await),
            vec!["Disconnected:1:Explicit", "Closed:1:Explicit"],
            "everything went out, so there is no loss report"
        );
        assert!(h.exited_cleanly().await);
    }

    // ★Advance the clock to 1 ms before the deadline and look★ — separates a regression folding at the write deadline (5 s) or the teardown limit.
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
        assert!(endpoint.drain().is_empty(), "the first write is blocked");

        let flush = Policy::default().flush_timeout;
        h.clock.advance(flush - Duration::from_millis(1));
        settle().await;
        assert_eq!(
            h.peer.peer_state(),
            PeerState::Live,
            "does not fold before the deadline"
        );
        assert_eq!(poll_immediate(done.as_mut()).await, None);

        h.clock.advance(Duration::from_millis(1));
        settle_until(
            "the supervisor ended at the deadline — the blocked write was not released",
            || h.task.is_finished(),
        )
        .await;
        assert_eq!(poll_immediate(done.as_mut()).await, Some(()));
        assert!(
            endpoint.drain().is_empty(),
            "★the close frame is within the same deadline too★ — the remaining share is 0, so it cannot go out on a blocked link"
        );
        let items = drain_until_quiet(&mut h).await;
        assert_eq!(
            labels(&items),
            vec!["Disconnected:1:Explicit", "Dropped", "Closed:1:Explicit"]
        );
        assert_eq!(
            dropped_out(&items),
            3,
            "counts the one cut mid-write + the two not written"
        );
        assert!(h.exited_cleanly().await);
    }

    // ★Even with the receiving line backed up (ordering area holding), the drain's deadline runs★ — and the end waits until the receiver drains (teardown loads what it holds).
    #[tokio::test]
    async fn a_held_inbound_line_neither_holds_back_the_flush_deadline_nor_loses_what_was_held() {
        let (mut h, endpoint) = live_on_a_full_line().await;
        endpoint.push(TestWire::encode_in(&TestIn::Notice("held".into())));
        settle().await;
        assert!(h.peer.held_bytes() > 0, "the ordering area is holding");
        endpoint.stall_writes(true);
        notify_all(&h.peer, 1);
        let done = h.peer.close_after_flush(BYE, "bye");
        tokio::pin!(done);
        settle().await;

        h.clock.advance(Policy::default().flush_timeout);
        settle_until("folded at the deadline", || {
            h.peer.peer_state() == PeerState::Closed
        })
        .await;
        settle().await;
        assert_eq!(
            poll_immediate(done.as_mut()).await,
            None,
            "the cleanup waits to deliver the held items to the receiver"
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
        settle_until("once the receiver drained, the supervisor ended", || {
            h.task.is_finished()
        })
        .await;
        assert_eq!(poll_immediate(done.as_mut()).await, Some(()));
    }

    // ★Deadline and write end ready in the same turn★ — it must close whichever `select!` picks. If the write wins, the turn start sees the past deadline as a
    //   timer (if it is in `wake_at` but `on_timer` does not look, the progress check blows up). The pick is random, so many turns.
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
            settle_until(&format!("round {round}: the supervisor ended"), || {
                h.task.is_finished()
            })
            .await;
            let sent = wire_labels(&endpoint.drain());
            assert!(
                sent == ["not|n0", "close:4100:bye"] || sent == ["close:4100:bye"],
                "round {round}: {sent:?}"
            );
            assert_eq!(poll_immediate(done).await, Some(()));
            assert!(
                h.exited_cleanly().await,
                "round {round}: the supervisor panicked (progress check)"
            );
        }
    }

    // ★If the deadline passes while draining the queue back to back (`Turn::Outbound`), the next write does not start★ (start of `Supervisor::guard`)
    //   — that batch does not return to the turn start so the timer cannot cut in. Stand on a keepalive, fill the queue, release, so the next turn drains
    //   in a batch and stands on the first write (as `a_consumer_leaving_mid_batch_…`). First write and deadline ready in the same turn, random pick: many turns.
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
            settle_until("connection", || h.peer.peer_state() == PeerState::Live).await;
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
                "round {round}: the supervisor took n0 out as a batch and stalled writing it"
            );

            endpoint.stall_writes(false);
            h.clock.advance(Policy::default().flush_timeout);
            settle_until(&format!("round {round}: the supervisor ended"), || {
                h.task.is_finished()
            })
            .await;
            let sent = wire_labels(&endpoint.drain());
            assert!(
                sent == ["not|n0", "close:4100:bye"] || sent == ["close:4100:bye"],
                "round {round}: ★no write started after the deadline★ — {sent:?}"
            );
            assert_eq!(poll_immediate(done).await, Some(()));
            assert!(h.exited_cleanly().await);
        }
    }

    // ★Measure while still operating★ — keep the supervisor on a write before the marker; after folding it cannot be told from rejected-because-outside-operation.
    #[tokio::test]
    async fn every_send_after_the_marker_is_refused_on_the_spot_and_not_counted() {
        let (mut h, endpoint) = live().await;
        endpoint.stall_writes(true);
        notify_all(&h.peer, 1);
        settle().await;
        assert!(
            h.queue_drained(),
            "the supervisor took n0 out and stalled writing it"
        );
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
        assert!(
            asked.is_finished(),
            "the request did not enter the queue and ended right away"
        );
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
            "nothing went in after the marker"
        );
        assert_eq!(
            h.peer.refused_count(),
            0,
            "not counted since it is after the app said it would close"
        );

        endpoint.stall_writes(false);
        settle_until("the supervisor ended", || h.task.is_finished()).await;
        assert_eq!(
            wire_labels(&endpoint.drain()),
            vec!["not|n0", "close:4100:bye"],
            "★the redial did not go in either★ — what was before the marker went out and it ended with the given close"
        );
        assert_eq!(h.net.dials(), 1);
        assert_eq!(
            labels(&drain_until_quiet(&mut h).await),
            vec!["Disconnected:1:Explicit", "Closed:1:Explicit"]
        );
        assert_eq!(poll_immediate(done).await, Some(()));
    }

    // ★Even if it closes at once, that close frame is within the same deadline★ — left blocked it ends at `flush_timeout` (not the 5 s write deadline); released,
    //   the close we were given goes out. Control and the drain-start wake are ready in the same turn and which is caught first is random, so the blocked branch runs many turns.
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
            settle_until("connection", || h.peer.peer_state() == PeerState::Live).await;
            let endpoint = h.net.accept().unwrap();
            h.drain();
            endpoint.stall_writes(true);
            notify_all(&h.peer, 1);
            settle().await;
            h.peer.notify(TestOut::Notice("n1".into())).unwrap();
            h.peer.notify(TestOut::Notice("n2".into())).unwrap();
            assert_eq!(h.peer.inner.data_tx.capacity(), 0, "the queue is full");

            let done = h.peer.close_after_flush(BYE, "bye");
            settle_until("folded right away without advancing the clock", || {
                h.peer.peer_state() == PeerState::Closed
            })
            .await;
            if unstall {
                // The close that drops the channel is caught by the write block — a place we deliberately do not listen at; release it (`drop_link`).
                endpoint.stall_writes(false);
            } else {
                h.clock.advance(Policy::default().flush_timeout);
            }
            settle_until(&format!("round {round}: the supervisor ended"), || {
                h.task.is_finished()
            })
            .await;
            let expected: &[&str] = if unstall { &["close:4100:bye"] } else { &[] };
            assert_eq!(
                wire_labels(&endpoint.drain()),
                expected,
                "round {round}: what is in the queue is not written, and the close is the given one or was cut by the deadline"
            );
            let items = drain_until_quiet(&mut h).await;
            assert_eq!(
                labels(&items),
                vec!["Disconnected:1:Explicit", "Dropped", "Closed:1:Explicit"]
            );
            assert_eq!(
                dropped_out(&items),
                3,
                "the marker is not counted as a loss"
            );
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
            .expect("the finished future did not resolve");

        assert_eq!(
            wire_labels(&endpoint.drain()),
            vec!["not|n0", "not|n1", "not|n2", "close:4100:bye"]
        );
        assert_eq!(
            consumer.await.expect("draining task"),
            vec![
                "Connected:1",
                "msg:held",
                "Disconnected:1:Explicit",
                "Closed:1:Explicit",
            ],
            "drained to the end (`None`)"
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
        settle_until("folded without advancing the clock", || {
            h.peer.peer_state() == PeerState::Closed
        })
        .await;
        endpoint.stall_writes(false);
        settle_until("the supervisor ended", || h.task.is_finished()).await;
        assert_eq!(
            wire_labels(&endpoint.drain()),
            vec!["close:4100:bye"],
            "★the three accepted were dropped★ (crate header 「Receiver-side invariants」 5)"
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
        settle_until("folded without waiting for the backoff", || {
            h.peer.peer_state() == PeerState::Closed
        })
        .await;
        h.clock.advance(Policy::default().reconnect.cap);
        settle().await;
        assert_eq!(h.net.dials(), 1, "did not redial");
        settle_until("the supervisor ended", || h.task.is_finished()).await;
        let items = drain_until_quiet(&mut h).await;
        assert_eq!(
            labels(&items),
            vec!["Disconnected:1:WriteFailed", "Dropped", "Closed:1:Explicit"],
            "the disconnect carries its reason as is"
        );
        assert_eq!(dropped_out(&items), 3);
        assert_eq!(poll_immediate(done).await, Some(()));
    }

    #[tokio::test]
    async fn a_flush_asked_mid_handshake_abandons_the_attempt() {
        let mut h = build(Policy::default(), Arc::new(ListeningHandshake));
        settle_until("during handshake", || {
            h.peer.peer_state() == PeerState::Handshaking
        })
        .await;
        let done = h.peer.close_after_flush(BYE, "bye");
        settle_until("ended without waiting for the handshake", || {
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
        settle_until("give up", || h.peer.peer_state().is_terminal()).await;
        h.drain();
        let done = h.peer.close_after_flush(BYE, "bye");
        settle_until("ended right away from waiting to give up", || {
            h.task.is_finished()
        })
        .await;
        assert_eq!(h.kinds(), vec!["Closed"]);
        assert_eq!(poll_immediate(done).await, Some(()));
    }

    // ★The marker is not bound to a generation★ — a marker stamped with an old generation, crossing a re-attach, also closes. Created directly (in the
    //   handle's stamping order it needs a narrow race). The clock is not advanced — the drain's deadline would close even without that decision.
    #[tokio::test]
    async fn a_marker_stamped_with_an_older_generation_still_closes() {
        let (mut h, endpoint) = live().await;
        assert_eq!(h.peer.generation(), Generation(1));
        h.peer
            .inner
            .goodbye
            .set(Close::new(BYE, "bye"))
            .expect("writes the first");
        h.peer
            .inner
            .data_tx
            .try_send(PeerCmd {
                generation: Generation(0),
                op: PeerOp::Close {
                    _keep_ctl: h.peer.inner.ctl_tx.clone(),
                },
            })
            .unwrap_or_else(|_| panic!("the outbound queue must have room"));
        settle_until("closed by the marker of the old generation", || {
            h.task.is_finished()
        })
        .await;
        assert_eq!(wire_labels(&endpoint.drain()), vec!["close:4100:bye"]);
        assert_eq!(
            labels(&drain_until_quiet(&mut h).await),
            vec!["Disconnected:1:Explicit", "Closed:1:Explicit"]
        );
    }

    // ★Call before the supervisor runs★ — the dial phase's `select!` picks at random, so without a look-ahead check the dial could be polled once. Many turns.
    #[tokio::test]
    async fn a_flush_asked_before_the_supervisor_runs_never_dials() {
        for round in 0..16 {
            let mut h = build(Policy::default(), Arc::new(ImmediateHandshake));
            let done = h.peer.close_after_flush(BYE, "bye");
            settle_until(&format!("round {round}: the supervisor ended"), || {
                h.task.is_finished()
            })
            .await;
            assert_eq!(h.net.dials(), 0, "round {round}");
            assert_eq!(h.kinds(), vec!["Closed"]);
            assert_eq!(poll_immediate(done).await, Some(()));
        }
    }

    // ★The drain continues even if all handles are released★ — releasing all closes the control channel, which the supervisor reads as 「close」; the marker holds it open.
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
        settle_until("the supervisor ended", || task.is_finished()).await;
        assert_eq!(
            wire_labels(&endpoint.drain()),
            vec!["not|n0", "not|n1", "not|n2", "close:4100:bye"]
        );
        assert_eq!(poll_immediate(done).await, Some(()));
    }

    // ★A bare close does not wait for the marker (the control channel overtakes)★ — the close loaded is the one written earlier.
    #[tokio::test]
    async fn a_plain_close_during_a_flush_jumps_the_queue_and_speaks_the_flush_goodbye() {
        let (h, endpoint) = live().await;
        endpoint.stall_writes(true);
        notify_all(&h.peer, 2);
        let done = h.peer.close_after_flush(BYE, "bye");
        settle().await;
        h.peer.close();
        settle_until("folded right away", || {
            h.peer.peer_state() == PeerState::Closed
        })
        .await;
        endpoint.stall_writes(false);
        settle_until("the supervisor ended", || h.task.is_finished()).await;
        assert_eq!(wire_labels(&endpoint.drain()), vec!["close:4100:bye"]);
        assert_eq!(poll_immediate(done).await, Some(()));
    }

    #[tokio::test]
    async fn the_end_future_resolves_at_once_when_the_supervisor_is_already_gone() {
        let mut h = build(Policy::default(), Arc::new(ImmediateHandshake));
        settle_until("connection", || h.peer.peer_state() == PeerState::Live).await;
        drop_the_consumer(&mut h);
        settle_until("the supervisor ended", || h.task.is_finished()).await;
        assert_eq!(
            poll_immediate(h.peer.close_after_flush(BYE, "bye")).await,
            Some(())
        );
    }

    // ★Call the drain while the close that drops the channel to redial is blocked★ — it was caught by the write deadline (5 s) before the drain, but the
    //   drain's deadline (2 s) covers it too (`drop_link` listens for the drain start). Advancing to 1 ms before the deadline separates it from a regression ending at the write deadline.
    #[tokio::test]
    async fn a_flush_asked_while_a_dropped_link_is_still_closing_ends_at_the_flush_deadline() {
        let (mut h, endpoint) = live().await;
        endpoint.stall_writes(true);
        h.peer.reconnect_now();
        settle().await;
        assert_eq!(
            h.net.dials(),
            1,
            "stuck on the blocked close, so it has not redialed yet"
        );
        assert!(endpoint.drain().is_empty(), "the close is blocked");
        let done = h.peer.close_after_flush(BYE, "bye");
        tokio::pin!(done);
        settle().await;

        let flush = Policy::default().flush_timeout;
        assert!(flush < Policy::default().write_deadline);
        h.clock.advance(flush - Duration::from_millis(1));
        settle().await;
        assert_eq!(
            poll_immediate(done.as_mut()).await,
            None,
            "before the deadline"
        );
        h.clock.advance(Duration::from_millis(1));
        settle_until("the supervisor ended at the drain's deadline", || {
            h.task.is_finished()
        })
        .await;
        assert_eq!(poll_immediate(done.as_mut()).await, Some(()));
        assert_eq!(h.net.dials(), 1, "did not redial");
        assert!(
            endpoint.drain().is_empty(),
            "the blocked close was cut at the deadline"
        );
        assert_eq!(
            labels(&drain_until_quiet(&mut h).await),
            vec!["Disconnected:1:Explicit", "Closed:1:Explicit"]
        );
        assert!(h.exited_cleanly().await);
    }

    // ★Call the reconnect right before the drain★ — the handle refuses only calls after the drain, so that one stays in the control queue; wherever the
    //   supervisor reads it, it must drop it so the drain ends with the close we were given. Three arrangements split the read places: a call on a blocked write
    //   (mid-write · deterministic) · a call without a block (mostly the operating wait · sometimes mid-write) · a call on a blocked write released at once (write end and
    //   control ready in the same turn, so the turn-start control collection is about half). The pick is random so each runs 16 turns (the chance of never hitting the
    //   rarest branch is under 1e-4).
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
                    "{round}: the supervisor stalled writing n0"
                );
            }
            h.peer.reconnect_now();
            let done = h.peer.close_after_flush(BYE, "bye");
            if unstall_at_once {
                endpoint.stall_writes(false);
            }
            settle().await;
            endpoint.stall_writes(false);
            settle_until(&format!("round {round}: the supervisor ended"), || {
                h.task.is_finished()
            })
            .await;
            assert_eq!(
                wire_labels(&endpoint.drain()),
                vec!["not|n0", "not|n1", "close:4100:bye"],
                "round {round}: ★the redial did not cut the drain★"
            );
            assert_eq!(h.net.dials(), 1, "round {round}");
            assert_eq!(
                labels(&drain_until_quiet(&mut h).await),
                vec!["Disconnected:1:Explicit", "Closed:1:Explicit"],
                "round {round}: everything went out, so there is no loss"
            );
            assert_eq!(poll_immediate(done).await, Some(()));
            assert!(h.exited_cleanly().await);
        }
    }

    // ★If the connection is lost mid-drain, the reconnect machine's give-up · reconnect is not shown★ — the shell shows a reconnect popup on `GaveUp`
    //   (ADR-0180). With one budget that disconnect is `GaveUp`, with three `Backoff` — both are measured.
    // ★Make the disconnect from the peer's close★ — while the write is blocked the peer closes (into the read-ahead queue) and the write deadline cuts that write;
    //   the cause is the peer's close read first (`earlier_end`), and the close that drops the channel is still blocked, so we look at the state published in
    //   between. Set the drain's deadline longer than the write deadline — if it closed first, no disconnect would occur.
    // ★The close frame carries the code of the disconnect cause, not the close we were given★ (`drop_link`) — the write works once released, so that code really goes out.
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
            settle_until("connection", || h.peer.peer_state() == PeerState::Live).await;
            let endpoint = h.net.accept().unwrap();
            h.drain();
            endpoint.stall_writes(true);
            notify_all(&h.peer, 2);
            let done = h.peer.close_after_flush(BYE, "bye");
            settle().await;
            endpoint.close();
            settle().await;
            h.clock.advance(write_deadline);
            settle_until("went to abandon the link", || {
                h.peer.peer_state() != PeerState::Live
            })
            .await;
            settle().await;
            assert_eq!(
                h.peer.peer_state(),
                PeerState::Closed,
                "budget {max_attempts}: ★did not go through Backoff · GaveUp★"
            );
            assert!(
                !h.task.is_finished() && endpoint.drain().is_empty(),
                "budget {max_attempts}: the close that abandons the link is still blocked — the state was observed in between"
            );

            endpoint.stall_writes(false);
            settle_until("the supervisor ended", || h.task.is_finished()).await;
            assert_eq!(
                wire_labels(&endpoint.drain()),
                vec![format!("close:{}:closing", CloseCode::GOING_AWAY.0)],
                "budget {max_attempts}: it is the code of the disconnect reason, not the given close"
            );
            let items = drain_until_quiet(&mut h).await;
            assert_eq!(
                labels(&items),
                vec!["Disconnected:1:PeerClosed", "Dropped", "Closed:1:Explicit"],
                "budget {max_attempts}: no GaveUp · ConnectFailed"
            );
            assert_eq!(
                dropped_out(&items),
                2,
                "one cut mid-write + one not written"
            );
            assert_eq!(h.net.dials(), 1);
            assert_eq!(poll_immediate(done).await, Some(()));
            assert!(h.exited_cleanly().await);
        }
    }

    // ★If a drain is called after deciding to give up while the close that drops that channel is blocked, `GaveUp` does not go up★ — the machine's
    //   `[.., DropLink, Emit(GaveUp)]` precedes the drain so `fold_into_close` cannot collect it (`apply` collects it when emitting). Cut by the write
    //   deadline with one budget so it gives up, then call the drain while the close is blocked.
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
        settle_until("connection", || h.peer.peer_state() == PeerState::Live).await;
        let endpoint = h.net.accept().unwrap();
        h.drain();
        endpoint.stall_writes(true);
        notify_all(&h.peer, 1);
        settle().await;
        h.clock.advance(Policy::default().write_deadline);
        settle_until("decided to give up", || {
            h.peer.peer_state() == PeerState::GaveUp(crate::machine::GaveUpReason::BudgetExhausted)
        })
        .await;
        settle().await;
        assert!(
            !h.task.is_finished() && endpoint.drain().is_empty(),
            "the close that abandons the link is blocked"
        );

        let done = h.peer.close_after_flush(BYE, "bye");
        settle().await;
        endpoint.stall_writes(false);
        settle_until("the supervisor ended", || h.task.is_finished()).await;
        assert_eq!(
            wire_labels(&endpoint.drain()),
            vec![format!("close:{}:closing", CloseCode::WRITE_DEADLINE.0)],
            "it is the code of the disconnect reason"
        );
        let items = drain_until_quiet(&mut h).await;
        assert_eq!(
            labels(&items),
            vec![
                "Disconnected:1:WriteDeadline",
                "Dropped",
                "Closed:1:Explicit"
            ],
            "★no GaveUp★"
        );
        assert_eq!(dropped_out(&items), 1);
        assert_eq!(h.net.dials(), 1, "did not redial");
        assert_eq!(poll_immediate(done).await, Some(()));
        assert!(h.exited_cleanly().await);
    }

    // ★The close frame of a non-drain close waits up to the write deadline★ — separates it from a regression shortening it to the drain's deadline.
    #[tokio::test]
    async fn a_plain_close_waits_for_its_closing_frame_up_to_the_write_deadline() {
        let (h, endpoint) = live().await;
        endpoint.stall_writes(true);
        h.peer.close();
        settle_until("folded", || h.peer.peer_state() == PeerState::Closed).await;
        let flush = Policy::default().flush_timeout;
        let write = Policy::default().write_deadline;
        assert!(flush < write);
        h.clock.advance(flush);
        settle().await;
        assert!(!h.task.is_finished(), "not cut by the drain's deadline");
        h.clock.advance(write - flush - Duration::from_millis(1));
        settle().await;
        assert!(!h.task.is_finished(), "before the write deadline");
        h.clock.advance(Duration::from_millis(1));
        settle_until("the close was cut at the write deadline", || {
            h.task.is_finished()
        })
        .await;
        assert!(endpoint.drain().is_empty());
        assert!(h.exited_cleanly().await);
    }

    // ★A request before the marker is written and its reply is not waited for★ — it closes at the marker and wakes as a sent disconnect (`Explicit`);
    //   the clock is not advanced — separates it from a regression waking on the request deadline.
    #[tokio::test]
    async fn requests_ahead_of_the_marker_are_written_then_woken_as_sent_without_their_reply() {
        let (mut h, endpoint) = live().await;
        endpoint.stall_writes(true);
        let first = spawn_request(&h.peer, 7);
        let second = spawn_request(&h.peer, 8);
        settle().await;
        let done = h.peer.close_after_flush(BYE, "bye");
        endpoint.stall_writes(false);
        settle_until("the supervisor ended", || h.task.is_finished()).await;
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
            "the second marker did not go in"
        );
        tokio::pin!(first);
        tokio::pin!(second);
        settle().await;
        assert_eq!(
            poll_immediate(second.as_mut()).await,
            None,
            "the second does not resolve right away either and waits for the same end"
        );

        endpoint.stall_writes(false);
        settle_until("the supervisor ended", || h.task.is_finished()).await;
        assert_eq!(
            wire_labels(&endpoint.drain()),
            vec!["not|n0", "close:4100:bye"],
            "the first close stays"
        );
        assert_eq!(poll_immediate(first.as_mut()).await, Some(()));
        assert_eq!(poll_immediate(second.as_mut()).await, Some(()));
        assert_eq!(
            labels(&drain_until_quiet(&mut h).await),
            vec!["Disconnected:1:Explicit", "Closed:1:Explicit"]
        );
    }
}
