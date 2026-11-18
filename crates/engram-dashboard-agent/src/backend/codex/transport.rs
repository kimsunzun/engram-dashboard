//! CodexAppServerTransport — the [`AgentTransport`] implementation for the `codex app-server --stdio` child process.
//!
//! Owns the child + Job Object · the single stdin writer · the reader (pump) · the waiting table and id counter · thread id and readiness ·
//! the gated input queue · the tool-end steer trigger ([`State::steer_due`] — ADR-0235). Id · method · order of outgoing envelopes are
//! decided here, so [`AgentTransport::send_input`] receives the message body, not a wire frame
//! ([`crate::backend::InputEncoder::TransportFramed`]).
//!
//! ★Three threads★:
//!   - **Reader** (pump, launched by [`AgentTransport::start`]) — owns the decoder as `&mut` and [`RefusedItems`], no lock (ADR-0241).
//!     ★It never takes the stdin lock and never waits on the waiting table★: a line needing an answer goes to the outbox. Otherwise
//!     reading stops, the peer's queue fills, the peer keeps its stdout open, and **EOF never comes** — alive, no signal.
//!   - **Writer** — the only holder of the stdin lock across a blocking write and the only drainer of the control queue; the handshake
//!     runs here. ★It keeps draining control lines while it waits★, else a queued refusal cannot go out and both sides wait for each other.
//!   - **stderr drain** — keeps the child from blocking on a full pipe.
//!
//! ★Lock order = state → waiting table★ (ADR-0006), never reverse; nothing takes the stdin lock while holding the state lock.
//!
//! ★Teardown (ADR-0001 2 verbs)★: no wait inside [`AgentTransport::shutdown`], and **on that path** stdin is not closed before kill
//! (`transport/stdio.rs`; guarded by [`tests::shutdown_completes_even_if_a_write_blocks_on_a_full_pipe`]). [`writer_loop`] does close stdin
//! **without kill** after a failed handshake — the reasons differ; neither is grounds to change the other. Nobody joins the writer; it ends
//! on `State::closed`, set by `shutdown()` and by the `Drop` of [`ReaderExit`]. ★Unsent queued input disappears silently★.
//!
//! ★Known limits — remove an item only together with its fix★:
//!   - **The turn has no deadline** (only the `turn/start` response does): a turn ending with only an `error` notification never opens the
//!     gate, and [`AgentTransport::send_input`] then reports "the queue is full" for the wrong reason.
//!   - **Non-Windows cannot reap grandchildren** (no Job Object, no process groups); a peer that closes only stdout and lives leaves an
//!     orphan there (Windows: `KILL_ON_JOB_CLOSE` on link drop).
//!   - **Job Object enrolment is after spawn** — a grandchild born in between escapes (`pty.rs` · `stdio.rs` the same). Precedent fix:
//!     `usage::process` + `spawn::prepare_tree_root`.
//!   - [`AgentTransport::interrupt`] ignores the closed marker and can return `Ok` for one line dropped at closing.
//!   - **History restore is best-effort** ([`hydrate_history`]): how much was restored is only in the log.
//!   - ★**Restored history and live lines can mix**★ — prevented by measurement, not structure: ADR-0203 measured `item/*` after resume as
//!     **0** (0.154.0), and a turn opens only after [`Link::Ready`]; codex self-updates and has no version field. Ring order is closed
//!     ([`OutputCore::emit_batch_without_turn_observation`]); delivery order to an attached subscriber is open, and the frontend
//!     **discards** `chunk.seq <= lastSeq` (`RichSlot`), so history after a cut-in vanishes. Closing it needs bounded emit-holding
//!     (ADR-0205: not to be built; holds the reopening trigger).
//!   - If the connection cannot stand, queued input disappears; the count is in the failure `detail` ([`HandshakeFailure::headline`]).
//!   - The pump is launched with a panicking `thread::spawn` (the other two absorb failures into the log).
//!   - **One blocking write can hang forever** (peer not reading stdin → pipe backpressure); only `shutdown()`'s kill gets out. The whole
//!     file, not only the handshake.
//!   - **[`writer_loop`] is not contained (ADR-0292)**: production ends the process on a writer panic; without the hook the only
//!     containment is [`record_session_id`]'s `catch_unwind`, and a panic elsewhere leaves the link `Connecting`/`Ready` with `closed`
//!     unset — `send_input` keeps returning `Ok` and reports "the queue is full".
//!   - **[`EARLY_COMPLETION_SLOTS`]**: more ends than slots before one response drop the oldest; if it was our turn's, nothing ends that
//!     turn. Emptied at every return to `Idle` ([`end_turn_if`]).
//!   - **If the handshake fails [`writer_loop`] only lets go of stdin** — it kills nothing (ADR-0199 forbids the writer's `child.kill()` ·
//!     `TerminateJobObject`). Recovery = the peer exits on EOF, else the manager's teardown via [`Link::Down`]
//!     ([`AgentTransport::link_state`]); [unverified on the refusal branch]. Where that decision is **not hooked** (codex launched as
//!     Fresh) nobody looks at `Down`: no owner. Fixing it first needs a decision on who carries the bounded wait.
//!   - **The session id record port runs synchronously on the writer thread** — it takes the registry lock and rewrites `agents.json`
//!     (`manager.rs`), so the handshake end can stall for someone else's disk write, lengthening [`OUTBOX_LIMIT`] refusal dropping.
//!   - **An outgoing request's real cap is `budget + SWEEP_INTERVAL`**, started *after* the first write.
//!   - **During the handshake control lines go out one per slice**; faster server requests reach [`OUTBOX_LIMIT`] and refusals drop.
//!   - `turn/completed` attribution leans on **the turn id string the peer gave**; an unattributed one cannot emit a boundary
//!     ([`Reader::note_turn`]) — the turn stays open here and in the fact layer with no deadline backstop once the response came. Both
//!     axes must get **the same** decision, progress included ([`Reader::claims_our_turn`]).

use std::collections::{HashMap, VecDeque};
use std::io::{Read, Write};
use std::process::{Child, ChildStderr, ChildStdin, ChildStdout};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{mpsc, Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

use engram_dashboard_base::logging::mask_secrets;
use engram_dashboard_base::panic::{contain, uncontained, Caught};
use engram_dashboard_base::sync;
use engram_dashboard_platform::group::GroupOwner;
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;

use super::decoder::{
    history_turn_boundary, is_tool_item, is_user_message, user_bubble, CodexAppServerDecoder,
    MAX_ID_BYTES,
};
use super::protocol::{
    self, method, ClientInfo, Inbound, InitializeParams, InitializeResponse, RequestId,
    SortDirection, Thread, ThreadItemsListParams, ThreadItemsListResponse, ThreadOpen,
    ThreadResumeResponse, ThreadStartResponse, TurnInterruptParams, TurnStartParams,
    TurnStartResponse, TurnSteerParams, UserInput, CLIENT_NAME, METHOD_NOT_FOUND,
};
use crate::backend::{FirstTurnSink, SessionIdSink};
use crate::output_core::{estimate_cost_bytes, OutputCore, REPLAY_MAX_BYTES, REPLAY_MAX_EVENTS};
use crate::transport::spawn::{drain_stderr, spawn_piped, PipedChild};
use crate::transport::{AgentTransport, LinkResolution, LinkSink, OutputDecoder};
use crate::types::{
    AgentId, CommandSpec, ControlCaps, DeliveryAck, DropCause, InputCaps, InputEvent, InputOrigin,
    OutputCaps, OutputEvent, PtyError, QueuedInputEvent, TerminalReason, ToolOutcome,
    TransportCaps, TurnInput, TurnOutcome, Withdraw,
};

// ── Constants ──────────────────────────────────────────────────────────────────────
//
// ★The values below are all invented — this axis has never been measured★. Each item carries its own grounds.

/// The cap on how long one outgoing request waits for an answer.
///
/// ★Applied to every request★ (measured 0.154.0): the server gives **no answer at all to an undecodable envelope, even with an `id`**, so
/// the waiting table would hang forever. Observed `initialize` ≈130ms · `turn/interrupt` ≈18ms; 30 s covers a slow `thread/start` that
/// launches MCP children. Unrelated to turn length — the `turn/start` response comes when the turn opens.
const REQUEST_DEADLINE: Duration = Duration::from_secs(30);

/// The cap on **the whole thing until the connection stands** — two round trips + the record call + history paging; ★one section, not one
/// request★.
///
/// ★History paging runs **inside** this budget (ADR-0203)★: a separate one could delay `Ready` by two budgets past the backstop and decide a
/// successful resume a failure (started once in [`writer_loop`]; [`tests::the_handshake_and_the_history_share_one_budget`]). A slow
/// handshake shortens the restored screen — intended.
/// It only has to **give up on a silent peer** (a refusal comes immediately); ★it must stay **smaller** than
/// [`crate::manager::LINK_RESOLUTION_BACKSTOP`]★, which gives up without a reason.
/// ★It bounds waiting for a response, not writes★: a blocked `write_all` ([`write_line`]) is never evaluated against it; the backstop and
/// teardown cut that. Turn requests use [`REQUEST_DEADLINE`].
const HANDSHAKE_BUDGET: Duration = Duration::from_secs(10);

/// The period at which the writer **looks around**: the deadline sweep, and sending control lines **one at a time** while the handshake
/// waits ([`request_blocking`]). A blocking write can still hang (module header). 1/60 of [`REQUEST_DEADLINE`].
const SWEEP_INTERVAL: Duration = Duration::from_millis(500);

/// The queue cap of control lines that go out without the gate (server request refusals · `turn/interrupt`).
///
/// Grounds: they go out regardless of turn state, so they pile up only if the peer **stopped reading**; this cap turns that silent stop
/// into an observable refusal. 64 exposes it quickly and still absorbs a rush of approval requests.
const OUTBOX_LIMIT: usize = 64;

/// The number of items requested per history page.
///
/// ★Does **not** bound line size★ (measured): the peer decides bytes per item (one `limit:50` page 1,823,509B; `limit:1` up to 437,196B), so
/// [`MAX_LINE_BYTES`] can be exceeded at any value; 25 trades that chance against round trips. An over-cap line gets no answer until the
/// deadline and the restore ends there ([`hydrate_history`]).
const HISTORY_PAGE_LIMIT: u32 = 25;

/// The maximum number of round trips history paging runs — **a runaway backstop, not a policy**.
///
/// Normal ends: cursor exhausted · ring filled · handshake budget exhausted. This fires only if the peer keeps giving a cursor that
/// never ends, and even then the budget usually cuts first. Times [`HISTORY_PAGE_LIMIT`] it is 6400 items, over the ring's count
/// ceiling (4096), so it never prevents carrying what could be carried.
const MAX_HISTORY_PAGES: usize = 256;

/// The cap on the input the link holds (`State::pending`). Over it, that call returns `Err` (ADR-0190 — it does not drop).
///
/// Grounds: one entry = one complete user turn; the queue stands only during the handshake (≈130ms) and one ongoing turn, so 32 full
/// means a person pushed 32 turns without seeing an answer — refusing is more honest than taking more. Everything is counted
/// regardless of stage (a handed-over item is still one list entry until its echo).
// ADR-0231
const INPUT_QUEUE_LIMIT: usize = 32;

const READ_BUF_BYTES: usize = 4096;

/// The maximum bytes one line can use. A line over it is dropped **alone** and recovery starts from the next newline.
///
/// ★Not derived from observation★ — the observed maximum (a 4KB-range error body) only says what the floor must not be. This measures
/// **the memory a peer that sends no newline can take with this one buffer**: 4MiB is unnoticeable as an amount one session can hold
/// by mistake and far too small to exhaust memory. Too large → that line is caught late; too small → normal lines are dropped and the
/// screen goes empty — so it leans to a thousand times the observed maximum.
const MAX_LINE_BYTES: usize = 4 * 1024 * 1024;

/// The cap (in characters) on peer strings carried into logs and the screen.
///
/// Grounds: the 4KB error body above would cover a whole log line uncut; 512 keeps the method name and the first sentence of the reason.
const LOG_STRING_LIMIT: usize = 512;

/// The number of slots holding the turn id of an end notification that came before the response.
///
/// Grounds: only one turn is open, so **one slot suffices in normal operation**; the rest is room for a peer streaming ends of turns we
/// never opened. Over the cap the oldest are dropped — the later one came, the likelier it belongs to the turn awaited now.
const EARLY_COMPLETION_SLOTS: usize = 8;

/// The number of slots remembering the item ids of approval requests we refused ([`RefusedItems`]).
///
/// The peer emits that item's end within a few ms of our refusal (fixture `refuse_u2a`), so 0–1 slots are normally occupied; the rest are
/// items whose end never comes. Over the cap the oldest drop and that item's late end shows as "error"/`Declined` instead of refused.
// ADR-0241
const REFUSED_ITEM_SLOTS: usize = 16;

/// The maximum bytes of a turn id to hold. ★**Filtered, not truncated**★ — it is a token compared for equality later, and truncating
/// could make two different long ids read as the same. The observed id is a UUIDv7 string (36 bytes); 128 is room beyond that.
const MAX_TURN_ID_BYTES: usize = 128;

/// The lowest codex edition that uses the list · receipt notifications — `clientUserMessageId` of `turn/start` · `turn/steer` came in
/// with it (tag comparison — same as ADR-0198's floor). Below it, today's behaviour ([`Floor::Below`]).
// ADR-0231
const CLIENT_MESSAGE_ID_FLOOR: (u64, u64, u64) = (0, 140, 0);

// ★No branching on error codes, hence no retry backoff constant★: an error response wakes its waiter whatever its code. A retry is unsafe
//   for all three requests (`initialize` works once; `thread/start` creates two threads; `thread/resume`/`turn/start` repeat), and
//   [unverified — whether an overload code comes in this envelope; observe first].

/// The notification that a turn ended — ★the state machine input of this link★.
///
/// The translator also carries this line as a `TurnEnd` boundary; **whether it belongs to our turn** is decided only here (the `boundary`
/// argument of [`Reader::note_turn`]), else someone else's end closes our turn and the fact layer plugs mail mid-turn.
/// ★`turn/started` is deliberately not read — do not revive it★: it carries no identifier we issued, and guessing by circumstance lets a
/// late notification write its id into the new turn's slot. Only **the `turn/start` response paired by our request id** decides the turn id.
const TURN_COMPLETED: &str = method::TURN_COMPLETED;

// ── State machine ─────────────────────────────────────────────────────────────────

/// The connection axis. ★Once cut, it does not come back in this incarnation (ADR-0192)★ — no reconnect, no re-`initialize`.
#[derive(Debug)]
enum Link {
    /// Before or during the handshake. Input is not refused but **queues**.
    Connecting,
    /// The second handshake response (`thread/start` or `thread/resume`) was received and the record call returned — `turn/start` is
    /// allowed from here on.
    Ready,
    /// Cut, or the handshake failed. The reason is quoted as is when refusing new input.
    Down(String),
}

/// The turn axis.
///
/// ★Only the `turn/start` **response** fills `turn_id`★ — it is the only one paired by our request id (why not notifications:
/// [`TURN_COMPLETED`] doc). `seq` is a marker raised each time we open a turn; ★comparison is match/mismatch only★ and **prevents
/// something late from touching the next turn opened in between**. Without it turns overlap on one thread, which the input queue
/// exists to prevent (ADR-0193).
#[derive(Debug)]
enum TurnState {
    Idle,
    Active { seq: u64, turn_id: Option<String> },
}

/// The floor decision ([`CLIENT_MESSAGE_ID_FLOOR`]) — decided once by the handshake, never reverted for that incarnation.
///
/// ★User items that came in during `Pending` are held without announcing★ — the side the decision lands on decides them all at once
/// ([`settle_floor`]); announcing earlier would create list items that meet a below-floor decision (TRD §5-5).
// ADR-0231
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Floor {
    Pending,
    /// The list · synthetic bubble · `clientUserMessageId` · echo comparison stand.
    Above,
    /// Today's path — items are held without identifiers and leave at the moment the turn opens.
    Below,
}

/// Things living under one lock. ★The "can it send" check and the "transition to in-progress" must be **the same critical section**★
/// (ADR-0193): [`AgentTransport::send_input`] is `&self`, so concurrent calls are possible and a split would let two callers both see
/// idle and each open a turn. That pairing exists only in [`take_turn_locked`].
struct State {
    link: Link,
    turn: TurnState,
    thread_id: Option<String>,
    /// Control lines that do not go through the turn gate. The writer picks them **before** input — a refusal waiting behind a queued
    /// user turn would never reach the peer.
    outbox: VecDeque<String>,
    /// The input the link holds, in arrival order. A turn opens with the oldest `Held`. Items without an identifier leave at the opening;
    /// items carrying one stay `InFlight` until echo or the turn-end disposal ([`take_turn_locked`] · [`State::close_turn_items`]).
    // ADR-0231
    pending: VecDeque<PendingItem>,
    // ADR-0231
    floor: Floor,
    /// The marker of the next turn to open. Only increases, never rewound.
    next_turn_seq: u64,
    /// ★An end notification that came before the response★ — it cannot be attributed yet, so it is held and compared when the
    /// `turn/start` response gives **our turn's id authoritatively**. What is held is that **id**, not "there was an end", which is why
    /// this is not a misattribution; no turn ends merely by being here.
    early_completions: VecDeque<EarlyCompletion>,
    /// This link has nothing more to send — the writer sees this and ends its loop.
    ///
    /// ★Set in two places★: [`AgentTransport::shutdown`] (we killed it) and the `Drop` of [`ReaderExit`] (peer ended or the reader
    /// panicked — `Drop` survives unwinding). ★The second matters: without it the writer thread stays forever for every naturally
    /// ended agent★, holding the `Arcs` of core · stdin · waiting table (one session's memory), and `shutdown()` is not called on the
    /// reaper path, so nobody wakes it.
    closed: bool,
    /// The tool-end trigger — a tool item of our current turn ended ([`Reader::note_tool_item`]) · or user text held for the turn whose
    /// turn id just came remains ([`Reader::resolve`]). While it stands the writer hands held user texts over as steers one at a time in
    /// typing order and lowers it when none is left ([`take_steer_locked`]) — one tool end hands over all held text (ADR-0235). Also
    /// lowered when the turn closes ([`State::close_turn_items`]). ★The setter does `notify_all` in the same lock section★.
    // ADR-0231
    steer_due: bool,
    /// Halt after error — our turn ended abnormally ([`TurnClose`]). ★While it stands Idle opens no turn★ ([`State::idle_opening`]); neither
    ///   held user text nor mail goes out on its own (TRD §5-5 「halt after error」 · user decision N11).
    /// Set in [`State::close_turn_items`]; lifted when user text comes in ([`State::push_pending`]) or the link closes ([`ReaderExit`]) —
    ///   not by the list emptying through ✕ or time. An interruption does not set it; it does not stand on a below-floor incarnation. It
    ///   blocks only turns the link opens itself (mail enters through the kernel's `last_end_failed`). ★A field outside [`TurnState`]★ — as
    ///   `Active` it would make the attribution gate count a late echo as our turn (TRD §5-5 「echo the vendor sent」 condition ①).
    // ADR-0231
    // ADR-0235: lifted when user text comes in.
    halted: bool,
    /// The 「steer refused」 mark — the marker (`seq` of [`TurnState::Active`]) of a turn whose steer got an error response or could not
    ///   be written. No held item is handed over in that turn ([`take_steer_locked`]): the vendor closed its window (for a write
    ///   failure the same write would fail again). The next turn has a different marker.
    // ADR-0231
    steer_refused: Option<u64>,
}

/// What one disposal will emit — gathered under the state lock; the caller runs [`Disposal::emit`] after releasing it (ADR-0006; logs
/// too — reason [`TurnNoteLog`]).
// ADR-0231
#[derive(Debug, Default)]
struct Disposal {
    events: Vec<QueuedInputEvent>,
    /// Trace lines of items returned to the held place (`held_again` — [`STEER_TRACE`]).
    traces: Vec<SteerTrace>,
    transitions: Vec<Transition>,
    /// Ids of list items returned to the held place — their handed-over mark is re-emitted ([`publish_hand_over`]).
    handed_back: Vec<String>,
}

impl Disposal {
    /// Order: transition → trace lines → list events → handed-over marks. ★Call it before the turn boundary★ — the consumer the boundary
    /// wakes must see a tidied list ([`end_turn_if`]).
    fn emit(self, core: &OutputCore, state: &SharedState) {
        write_transitions(core.id(), self.transitions);
        for t in &self.traces {
            t.write(core.id());
        }
        for op in self.events {
            core.emit(OutputEvent::QueuedInput(op));
        }
        for id in &self.handed_back {
            publish_hand_over(core, state, id);
        }
    }
}

/// Emits the handed-over mark (`HandedOver`) — ★the value is read from the item's current stage★ (`InFlight` = handed over · `Held` =
/// held). Only list items that finished announcing emit it (so it never precedes `Queued`).
///
/// ★It re-reads after emitting and re-emits if changed★: the handing-over side (writer) and the returning side (reader · turn end) emit on
/// different threads after releasing the state lock, so a fixed value could leave a stale last mark in the ring. A duplicate is harmless.
// ADR-0231
// ADR-0235
fn publish_hand_over(core: &OutputCore, state: &SharedState, id: &str) {
    let mut published = None;
    loop {
        let sent = {
            let (lock, _) = &**state;
            let s = sync::lock(lock);
            s.pending
                .iter()
                .find(|p| {
                    p.listed && p.announced && p.death.is_none() && p.id.as_deref() == Some(id)
                })
                .map(|p| matches!(p.stage, Stage::InFlight { .. }))
        };
        match sent {
            Some(sent) if published != Some(sent) => {
                core.emit(OutputEvent::QueuedInput(QueuedInputEvent::HandedOver {
                    id: id.to_owned(),
                    sent,
                }));
                published = Some(sent);
            }
            _ => return,
        }
    }
}

/// One transition of the halt after error — gathered under the state lock, written by [`write_transitions`] after releasing it
/// ([`TurnNoteLog`]). ★User text is not carried★, only the turn marker.
/// Levels: set and lift are info; discard is debug (cleanup on link close).
// ADR-0231
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Transition {
    /// Turn `turn` ended with `close` and halted ([`State::halted`]).
    Halted { turn: u64, close: TurnClose },
    /// User text came in and the halt was lifted ([`State::push_pending`] — the writer = [`AgentTransport::send_turn`]).
    HaltLifted,
    /// The link closed and discarded the standing halt ([`ReaderExit`]).
    Discarded,
}

impl Transition {
    fn write(self, agent: AgentId) {
        match self {
            Transition::Halted { turn, close } => tracing::info!(
                agent = %agent,
                turn,
                ?close,
                "codex app-server: halt after error — no turn is opened until the next user text"
            ),
            Transition::HaltLifted => tracing::info!(
                agent = %agent,
                "codex app-server: halt after error lifted — user text came in"
            ),
            Transition::Discarded => tracing::debug!(
                agent = %agent,
                "codex app-server: the link closed, discarding the halt after error"
            ),
        }
    }
}

/// Writes the gathered transitions — ★call it after releasing the state lock★ ([`TurnNoteLog`]).
fn write_transitions(agent: AgentId, transitions: Vec<Transition>) {
    for t in transitions {
        t.write(agent);
    }
}

/// One `turn/completed` that came before the response — holds the id together with **the turn boundary that line made**.
///
/// ★Why the boundary is held too★: raising it to the screen now would let an unattributed end close a live turn ([`Boundary`]), but
/// dropping it would leave nothing to raise once the comparison holds and **that conversation's waiting indicator would spin
/// forever**. So it is only deferred.
struct EarlyCompletion {
    /// The comparison token. ★Held unmasked on purpose★ — transforming it would break the comparison; it is filtered by length
    /// ([`MAX_TURN_ID_BYTES`]) and passes [`sanitize`] when going to the log · screen.
    turn_id: String,
    /// The boundary the translator emitted from that line. `None` = it could not emit one (malformed line · no translator).
    boundary: Option<OutputEvent>,
    /// That line's status — where the comparison holds, that turn's items are disposed (TRD §5-5 「an early end is a turn end too」). Not
    /// read back from the boundary because disposal must stand even when no boundary could be emitted.
    // ADR-0231
    close: TurnClose,
}

/// The shape in which our turn ended — splits the disposal of that turn's items ([`State::close_turn_items`]).
// ADR-0231
// ADR-0234
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TurnClose {
    Completed,
    /// A `turn/completed` status of unknown meaning · no status (incl. `inProgress`). Disposal same as `Completed` but ★it does not set
    ///   the halt★ — the kernel classifier folds the same line into `TurnOutcome::Unknown` → `TurnEndKind::Other` and does not set
    ///   `last_end_failed`.
    Unknown,
    /// `failed` of `turn/completed`. With `Rejected` · `Unanswered` it is a failure end: it returns the user text that turn carried without
    /// an echo to the held place and sets the halt ([`State::close_turn_items`] · [`State::halted`]).
    Failed,
    /// `interrupted` of `turn/completed` — the vendor drops what was handed over with neither echo nor history (M6). Not a failure end:
    /// it neither sets the halt nor erases text; held and returned text go out at the next Idle (user decision 2026-09-26).
    Interrupted,
    /// `turn/start` did not reach the vendor or was refused — write failure · error response.
    Rejected,
    /// Unknown whether the vendor received `turn/start` — response deadline · decode failure · turn id over the cap. A failure end, disposal
    ///   same as `Failed` (if it was received, an earlier echo already closed it as received).
    Unanswered,
}

impl TurnClose {
    fn of_completion(params: Option<&Value>) -> Self {
        match params
            .and_then(|p| p.get("turn"))
            .and_then(|t| t.get("status"))
            .and_then(|v| v.as_str())
        {
            Some(protocol::turn_status::COMPLETED) => TurnClose::Completed,
            Some(protocol::turn_status::INTERRUPTED) => TurnClose::Interrupted,
            Some(protocol::turn_status::FAILED) => TurnClose::Failed,
            _ => TurnClose::Unknown,
        }
    }
}

// ── Mid-turn handover trace (ADR-0235) ─────────────────────────────

/// Target of the mid-turn handover trace lines — `RUST_LOG=engram::codex_steer=debug`; when off nothing is gathered
/// ([`SteerTrace::enabled`]). ★Field names are a contract★ — QA reads the log by them: every line has `agent` · `phase` · `item_id`
/// (`turn_id` when known); `phase` ([`SteerPhase`]) ∈ `tool_start` · `tool_end` (+`held`) · `steer` (+`request`) · `write_failed` ·
/// `refused` · `echo` · `held_again` (+`close` = [`TurnClose`] name). ★Order is not guaranteed across threads★ (reader vs writer lines) —
/// read timestamps. Written after releasing the state lock ([`TurnNoteLog`]).
// ADR-0231
const STEER_TRACE: &str = "engram::codex_steer";

/// The kinds of [`STEER_TRACE`] lines — the `phase` field value ([`SteerPhase::name`], a QA contract: do not change) and a message.
// ADR-0235
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SteerPhase {
    ToolStart,
    ToolEnd,
    Steer,
    WriteFailed,
    Refused,
    Echo,
    HeldAgain,
}

impl SteerPhase {
    fn name(self) -> &'static str {
        match self {
            SteerPhase::ToolStart => "tool_start",
            SteerPhase::ToolEnd => "tool_end",
            SteerPhase::Steer => "steer",
            SteerPhase::WriteFailed => "write_failed",
            SteerPhase::Refused => "refused",
            SteerPhase::Echo => "echo",
            SteerPhase::HeldAgain => "held_again",
        }
    }

    fn message(self) -> &'static str {
        match self {
            SteerPhase::ToolStart => "codex steer: tool started",
            SteerPhase::ToolEnd => "codex steer: tool ended — the trigger to hand over held user text",
            SteerPhase::Steer => "codex steer: handed held text over with turn/steer",
            SteerPhase::WriteFailed => {
                "codex steer: could not write turn/steer — it did not reach the vendor"
            }
            SteerPhase::Refused => "codex steer: turn/steer refused — the vendor did not take that text",
            SteerPhase::Echo => "codex steer: echo — text in the held list became received",
            SteerPhase::HeldAgain => {
                "codex steer: the turn ended without an echo, returning the handed-over text to the held place"
            }
        }
    }
}

/// One [`STEER_TRACE`] line — material gathered inside the state lock, written outside it.
// ADR-0231
#[derive(Debug)]
struct SteerTrace {
    phase: SteerPhase,
    turn_id: Option<String>,
    item_id: String,
    /// `tool_end` only — user items held at that moment.
    held: Option<usize>,
    /// `steer` only — the request id of that `turn/steer`.
    request: Option<i64>,
    /// `held_again` only — the shape in which that turn ended.
    close: Option<TurnClose>,
}

impl SteerTrace {
    /// If the line is off, the material (id copy) is not built.
    fn enabled() -> bool {
        tracing::enabled!(target: STEER_TRACE, tracing::Level::DEBUG)
    }

    fn new(phase: SteerPhase, turn_id: Option<&str>, item_id: &str) -> Self {
        SteerTrace {
            phase,
            turn_id: turn_id.map(str::to_string),
            item_id: item_id.to_string(),
            held: None,
            request: None,
            close: None,
        }
    }

    /// The turn id and tool item id are vendor strings, so they go through [`sanitize`]; our item id goes through the same door.
    fn write(&self, agent: AgentId) {
        let turn_id = self
            .turn_id
            .as_deref()
            .map(|t| sanitize(t, LOG_STRING_LIMIT));
        tracing::debug!(
            target: STEER_TRACE,
            agent = %agent,
            phase = self.phase.name(),
            turn_id = turn_id.as_deref(),
            item_id = %sanitize(&self.item_id, LOG_STRING_LIMIT),
            held = self.held,
            request = self.request,
            close = self.close.map(tracing::field::debug),
            "{}",
            self.phase.message()
        );
    }
}

/// One steer the writer picked — the item was already moved to `InFlight` in the picking lock section ([`take_steer_locked`]).
// ADR-0231
#[derive(Debug)]
struct Steer {
    request: i64,
    /// The id of the handed-over item — pairs the response with it ([`Waiter::Steer`]).
    item: String,
    /// The marker of the turn the item went into = its `InFlight` generation.
    gen: u64,
    line: String,
    turn_id: String,
}

impl Steer {
    /// The trace line of this steer — `steer` (after the write · [`writer_loop`]) · `write_failed` ([`steer_write_failed`]). `None` if off.
    fn trace(&self, phase: SteerPhase) -> Option<SteerTrace> {
        SteerTrace::enabled().then(|| SteerTrace {
            request: Some(self.request),
            ..SteerTrace::new(phase, Some(&self.turn_id), &self.item)
        })
    }
}

impl State {
    fn new() -> Self {
        State {
            link: Link::Connecting,
            turn: TurnState::Idle,
            thread_id: None,
            outbox: VecDeque::new(),
            pending: VecDeque::new(),
            floor: Floor::Pending,
            next_turn_seq: 0,
            early_completions: VecDeque::new(),
            closed: false,
            steer_due: false,
            halted: false,
            steer_refused: None,
        }
    }

    /// The held item the next Idle turn opens — TRD §5-5 「the order of opening turns = user first」 (PRD §3-9 · user decision N6).
    /// ★Decided by state alone★; whether a turn can open (`Ready` · `Idle`) is the caller's check ([`take_turn_locked`]).
    ///   1. The oldest user `Held` (incl. Direct · text returned at turn end). Even before announcing it is the head — the opening side
    ///      waits for the announcement (typing order).
    ///   2. Otherwise the oldest mail (incl. items without an identifier).
    /// ★During a halt ([`State::halted`]) nothing is opened★ — once the next user text lifts it, order is as above.
    /// ★If the floor decision is not `Above`★ it is today's order (oldest `Held` · any source) and the halt does not stand either.
    // ADR-0231
    fn idle_opening(&self) -> Option<usize> {
        let held = |p: &PendingItem| matches!(p.stage, Stage::Held);
        if self.floor != Floor::Above {
            return self.pending.iter().position(held);
        }
        if self.halted {
            return None;
        }
        self.pending
            .iter()
            .position(|p| held(p) && p.origin == InputOrigin::User)
            .or_else(|| self.pending.iter().position(held))
    }

    /// Puts one item at the end as `Held` and **decides what to announce**. A user item lifts the halt ([`State::halted`]); the returned
    /// value = it was lifted this time (the caller writes that transition log outside the lock).
    ///
    /// ★Classification uses the state **before** putting, in the same critical section★: `Ready ∧ Idle ∧ pending empty` → Direct
    ///   (synthetic bubble · outside the list); other user items are Queued (list). ★Direct is also put into pending★ — otherwise the
    ///   second input of the same idle moment also sees "empty" and becomes Direct, leaving a held item outside the list that blocks new
    ///   text and mail (TRD §5-5 「Direct items are kept in pending too」).
    /// Items with something to announce are held `announced=false`; [`announce`] emits outside the lock. Mail · items without an
    /// identifier emit nothing. After a below-floor decision the identifier is dropped.
    // ADR-0231
    fn push_pending(&mut self, id: Option<String>, body: Vec<u8>, origin: InputOrigin) -> bool {
        let id = if self.floor == Floor::Below { None } else { id };
        let (listed, announced) = match (&id, origin, self.floor) {
            (None, _, _) | (Some(_), InputOrigin::Mail, _) => (false, true),
            // Before the decision — the side the decision lands on decides ([`settle_floor`]).
            (Some(_), InputOrigin::User, Floor::Pending) => (false, false),
            (Some(_), InputOrigin::User, _) => {
                let direct = matches!(self.link, Link::Ready)
                    && matches!(self.turn, TurnState::Idle)
                    && self.pending.is_empty();
                (!direct, false)
            }
        };
        self.pending.push_back(PendingItem {
            id,
            body,
            origin,
            stage: Stage::Held,
            listed,
            announced,
            cancel_asked: false,
            death: None,
        });
        // ADR-0235: the next user text lifts the halt — even if that text later leaves by ✕, it does not stand again.
        origin == InputOrigin::User && std::mem::take(&mut self.halted)
    }

    /// The vendor received that text (`clientId` echo) — removed from pending regardless of stage. ★Receipt wins★: an echo at any stage
    ///   means the text is in the history (TRD §5-5 「consumption echo」), so a held item does not go out again, and for a handed-over item
    ///   the late response becomes a no-op in the [`State::turn_start_carrier`] comparison. Returns the removed item (`None` = id unknown
    ///   to the link — mail · already resolved · second echo).
    // ADR-0231
    fn take_delivered(&mut self, id: &str) -> Option<PendingItem> {
        let pos = self
            .pending
            .iter()
            .position(|p| p.id.as_deref() == Some(id))?;
        self.pending.remove(pos)
    }

    /// Among the items the `turn/start` of marker `gen` carried, the one still waiting for a response — the `InFlight` of that turn with
    /// unknown turn id.
    ///
    /// ★There is only one★: a `turn/start` carries one item (TRD §5-5) and a steer goes out only after the turn id is known. If the
    /// stage moved on (echo · disposed), `None` — the late response is a no-op (generation masking).
    // ADR-0231
    fn turn_start_carrier(&mut self, gen: u64) -> Option<&mut PendingItem> {
        self.pending.iter_mut().find(|p| {
            matches!(
                p.stage,
                Stage::InFlight { gen: g, turn_id: None } if g == gen
            )
        })
    }

    /// Disposes one list item (or a refused Direct) for this cause — returns the `Dropped` to emit outside the lock.
    ///
    /// ★Before announce only the death mark is attached and `None` is returned★ — `Queued` may not be in the ring yet, so emitting
    ///   `Dropped` could precede it. [`announce`] follows **right after** `Queued` and removes the item (TRD §5-5 「when a disposal meets
    ///   an item before announce」).
    // ADR-0231
    fn drop_listed(&mut self, pos: usize, cause: DropCause) -> Option<QueuedInputEvent> {
        let item = self.pending.get_mut(pos)?;
        if !item.announced {
            item.death = Some(cause);
            return None;
        }
        let item = self.pending.remove(pos)?;
        item.id.map(|id| QueuedInputEvent::Dropped { id, cause })
    }

    /// The turn of marker `gen` ended with `close` — disposes that turn's items and returns what to emit (the caller runs
    /// [`Disposal::emit`] outside the lock).
    ///
    /// ★Call it in **the same critical section** that returns the turn to `Idle`★ — else new text in the gap sees an outcome-less `InFlight`;
    /// a missed ending path leaves it in pending forever (TRD §5-5 「five exits」).
    ///
    /// | Item | Any end (`Completed` · `Unknown` · `Interrupted` · `Failed` · `Rejected` · `Unanswered`) |
    /// |---|---|
    /// | User item handed over in that turn — no echo | Returned to the held place (no outcome event; only the handed-over mark re-emits) |
    /// | … ✕ reached it after the handover | `Dropped{Withdrawn}` |
    /// | Mail handed over in that turn | Removed without an event |
    /// | Held item | As is |
    ///
    /// Only an echo removes handed-over text as received; a late echo makes it received while held ([`State::take_delivered`]). ★A failure
    /// end does not erase either★ (user decision 2026-09-26 · ADR-0235): returned and held text wait behind the halt ([`State::halted`]).
    /// ★No waiting for steer responses★ — an accepted steer's echo precedes `turn/completed` (TRD §3-2 · M10); the late response is a no-op
    /// by generation ([`State::hold_refused_steer`]).
    /// ★The halt is set here★: `Failed` · `Rejected` · `Unanswered` (floor-meeting incarnations only); the others do not.
    /// ★Write each `close` branch separately, no `_`★ — a new end must stop compilation here.
    // ADR-0231
    // ADR-0235
    fn close_turn_items(&mut self, gen: u64, close: TurnClose) -> Disposal {
        enum Fate {
            Keep,
            Held,
            Drop(DropCause),
            Remove,
        }
        // ADR-0235: the tool-end trigger belongs to that turn, so it is lowered with it — otherwise text typed after the next turn id came
        //   would go out as a steer without a tool end. For text held in the next turn, [`Reader::resolve`] sets it again.
        self.steer_due = false;
        let tracing_on = SteerTrace::enabled();
        let mut out = Disposal::default();
        let mut pos = 0;
        while let Some(item) = self.pending.get(pos) {
            let carried = item.death.is_none()
                && matches!(item.stage, Stage::InFlight { gen: g, .. } if g == gen);
            let fate = if !carried {
                // Held items · items of the previous turn · already disposed pre-announce items (death mark — its cause is not overwritten).
                Fate::Keep
            } else if item.origin == InputOrigin::Mail {
                Fate::Remove
            } else if item.cancel_asked {
                Fate::Drop(DropCause::Withdrawn)
            } else {
                // ★Returned to the held place whatever the end shape★ (user decision 2026-09-26 · ADR-0235): a failure end sets the halt below, so
                //   the returned text waits for the next user text.
                Fate::Held
            };
            match fate {
                Fate::Keep => pos += 1,
                Fate::Remove => {
                    self.pending.remove(pos);
                }
                Fate::Drop(cause) => {
                    // A handed-over item finished announcing, so it leaves right away ([`State::drop_listed`]).
                    out.events.extend(self.drop_listed(pos, cause));
                    if self.pending.get(pos).is_some_and(|p| p.death.is_some()) {
                        pos += 1;
                    }
                }
                Fate::Held => {
                    let item = &mut self.pending[pos];
                    if item.listed {
                        out.handed_back.extend(item.id.clone());
                    }
                    if tracing_on {
                        if let (Stage::InFlight { turn_id, .. }, Some(id)) = (&item.stage, &item.id)
                        {
                            out.traces.push(SteerTrace {
                                close: Some(close),
                                ..SteerTrace::new(SteerPhase::HeldAgain, turn_id.as_deref(), id)
                            });
                        }
                    }
                    item.stage = Stage::Held;
                    pos += 1;
                }
            }
        }
        // ★Halt after error (TRD §5-5)★ — Idle opens no turn until the next user text. An interruption does not set it, nor does an end of
        //   unknown meaning ([`TurnClose::Unknown`] — the kernel classifier folds it into "the rest").
        match close {
            TurnClose::Failed | TurnClose::Rejected | TurnClose::Unanswered => {
                if self.floor == Floor::Above {
                    self.halted = true;
                    out.transitions
                        .push(Transition::Halted { turn: gen, close });
                }
            }
            TurnClose::Completed | TurnClose::Interrupted | TurnClose::Unknown => {}
        }
        out
    }

    /// A steer got an error response or could not be written — the vendor does not have that text. Returns what the caller emits outside the
    /// lock (`None` = no-op).
    ///
    /// ★It moves only when (id, generation) **and the current stage** match★ (generation masking — TRD §5-5): an item already removed by echo or
    /// disposed by the turn end is not moved twice, and no 「steer refused」 mark is attached. On a match the item returns to `Held` (typing
    /// order), the turn gets the mark (no resend loop), and only the handed-over mark re-emits ([`Disposal::handed_back`]). ★A ✕-reached item
    /// is withdrawn instead★ (`Dropped{Withdrawn}`).
    // ADR-0231
    fn hold_refused_steer(&mut self, id: &str, gen: u64) -> Option<Disposal> {
        let pos = self.pending.iter().position(|p| {
            p.id.as_deref() == Some(id)
                && matches!(p.stage, Stage::InFlight { gen: g, turn_id: Some(_) } if g == gen)
        })?;
        let mut out = Disposal::default();
        // The mark is independent of ✕ — the vendor refused to put it into this turn either way.
        self.steer_refused = Some(gen);
        if self.pending[pos].cancel_asked {
            out.events
                .extend(self.drop_listed(pos, DropCause::Withdrawn));
        } else {
            let item = &mut self.pending[pos];
            item.stage = Stage::Held;
            if item.listed {
                out.handed_back.extend(item.id.clone());
            }
        }
        Some(out)
    }
}

/// One input the link holds. ★The link does not read the registry; it decides by these fields★ (ADR-0193).
// ADR-0231
struct PendingItem {
    /// The identifier for list events and the echo comparison (the `msg_uuid` of the session write). `None` = an item from
    /// [`AgentTransport::send_input`] — it can be neither withdrawn nor compared by echo and does not go into the list.
    id: Option<String>,
    body: Vec<u8>,
    origin: InputOrigin,
    stage: Stage,
    /// A list item (emitted or will emit `Queued`)? Direct · mail · items without an identifier are `false`.
    listed: bool,
    /// The announcement (Direct synthetic echo · `Queued`) finished or there is nothing to emit. ★The writer does not hand over an item
    /// with this `false`★ — a handover outrunning the announcement would drop the receipt (`Delivered`) for an id the registry does not
    /// know. ★`listed=false ∧ announced=false` = "not decided yet" before the decision, Direct after it (`Above`)★ — pre-decision items
    /// are all decided again by [`settle_floor`] under the same lock.
    announced: bool,
    /// ✕ came after the handover ([`withdraw_item`] sets it) — the vendor decides the outcome; if it did not take the item (steer error
    /// response · turn end without echo) the link withdraws it (`Dropped{Withdrawn}`) instead of returning it. An overlapping second ✕ is
    /// `TooLate` without an event.
    cancel_asked: bool,
    /// Cause of a disposal that arrived before announce — the announcing side follows `Queued` **right after** with a `Dropped` of this
    /// cause (so `Dropped` never precedes `Queued` in the ring).
    death: Option<DropCause>,
}

/// The stage of a [`PendingItem`].
// ADR-0231
enum Stage {
    Held,
    /// `turn/start` or a steer was written; waits for the echo.
    ///
    /// `gen` = ★the marker of the turn that request went into (`seq` of [`TurnState::Active`])★ — it rises with every turn the link
    ///   opens, and a turn id comes only from that turn's `turn/start` response, so it equals "the number that rises with each new active
    ///   turn id"; no separate counter exists, so the two cannot diverge. A late response · echo moves the item only when (id, gen) and
    ///   **the current stage** match. `turn_id` = the turn that request aimed at (`None` = `turn/start` carried it and its response has
    ///   not given the id yet — a steer goes out only after the turn id is known).
    InFlight {
        gen: u64,
        turn_id: Option<String>,
    },
}

/// State + the condition variable waiting on it. The writer sleeps here.
///
/// ★`notify` reduces **latency**, it does not carry correctness★: the writer wakes by `wait_timeout` of [`SWEEP_INTERVAL`] and
/// re-checks the same condition, so without any `notify_all` the behaviour is the same, only the queue opens later. Do not read it as
/// "without a notification it stops" — that quotes a guarantee that does not exist.
type SharedState = Arc<(Mutex<State>, Condvar)>;

// ── List announcement · floor decision ─────────────────────────────────────────────

/// The queue of announcing sides and whether receipt notifications are available — one per link (shared with the writer by `Arc`).
///
/// `order` — ★announcing must be outside the state lock (ADR-0006) and there are two announcing sides★ (input thread · writer); the holder
///   emits **all pre-announce items** in arrival order so the list keeps typing order. ★Lock order = `order` → state; emitting under
///   `order` is an intended ADR-0006 exception (`OutputSink::send` does not block)★. The reader and `withdraw` do not take it.
/// `ack` — filled by the floor decision; the session carries **the same** `Arc` ([`CodexAppServerTransport::delivery_ack`]).
// ADR-0231
// ADR-0234
struct Announcer {
    order: Mutex<()>,
    ack: Arc<DeliveryAck>,
}

/// Emits pre-announce items in arrival order — list items = `Queued`, Direct = synthetic bubble; nothing before the decision (`Above`).
///
/// Gather under the lock → release and emit → retake for `announced=true` + waking the writer; an item death-marked in between is removed
/// here with `Dropped` **right after** `Queued`.
/// ★The bubble uses `emit_without_turn_observation`, not the ordinary door★: the classifier would count it as progress and a refused
/// `turn/start` would leave a never-opened turn "in progress", blocking mail until the 30-minute fail-open (ADR-0127).
// ADR-0231
// ADR-0234
fn announce(core: &OutputCore, state: &SharedState, announcer: &Announcer) {
    let _order = sync::lock(&announcer.order);
    let batch: Vec<(String, String, bool)> = {
        let (lock, _) = &**state;
        let s = sync::lock(lock);
        if s.floor != Floor::Above {
            return;
        }
        s.pending
            .iter()
            .filter(|p| !p.announced)
            .filter_map(|p| {
                let id = p.id.clone()?;
                Some((id, String::from_utf8_lossy(&p.body).into_owned(), p.listed))
            })
            .collect()
    };
    if batch.is_empty() {
        return;
    }
    let ids: Vec<String> = batch.iter().map(|(id, _, _)| id.clone()).collect();
    for (id, text, listed) in batch {
        if listed {
            core.emit(OutputEvent::QueuedInput(QueuedInputEvent::Queued {
                id,
                text,
            }));
        } else {
            core.emit_without_turn_observation(user_bubble(&text, Some(&id)));
        }
    }
    let dead = {
        let (lock, cv) = &**state;
        let mut s = sync::lock(lock);
        let mut dead = Vec::new();
        for id in ids {
            let Some(pos) = s.pending.iter().position(|p| p.id.as_deref() == Some(&id)) else {
                continue;
            };
            s.pending[pos].announced = true;
            if let Some(cause) = s.pending[pos].death {
                s.pending.remove(pos);
                dead.push(QueuedInputEvent::Dropped { id, cause });
            }
        }
        cv.notify_all();
        dead
    };
    for op in dead {
        core.emit(OutputEvent::QueuedInput(op));
    }
}

/// Writes the floor decision and decides the path of items held before it — the writer calls it **once** after the handshake stands.
/// `Above`: pre-decision user items become list items (`Queued`). `Below`: every item drops its identifier and takes today's path (user
/// decision N1 (a)).
/// ★Call it **after** the session id record returned★ — earlier, the record failure branch ([`writer_loop`]) would erase list items
/// without an event.
// ADR-0231
fn settle_floor(core: &OutputCore, state: &SharedState, announcer: &Announcer, floor: Floor) {
    debug_assert_ne!(floor, Floor::Pending, "the decision is one of the two");
    {
        let (lock, cv) = &**state;
        let mut s = sync::lock(lock);
        if floor == Floor::Above {
            s.floor = Floor::Above;
            announcer.ack.set_available();
            for p in s.pending.iter_mut() {
                if !p.announced && p.origin == InputOrigin::User && p.id.is_some() {
                    p.listed = true;
                }
            }
        } else {
            s.floor = Floor::Below;
            announcer.ack.try_set_unavailable();
            for p in s.pending.iter_mut() {
                p.id = None;
                p.listed = false;
                p.announced = true;
            }
        }
        cv.notify_all();
    }
    announce(core, state, announcer);
}

/// The floor decision (M8) — `thread.cliVersion`, else the first edition in the `initialize` `userAgent`, else below the floor.
///
/// ★`userAgent` reads only the edition right after the first `/`★ — the trailing parenthesis holds **our** edition (0.156.1:
/// `engram-dashboard/0.156.1 (Windows …) unknown (engram-dashboard; 0.1.0)`). ★Unknown → below the floor★ (an unknown edition quietly
/// breaks the echo comparison); a floor prerelease (`0.140.0-alpha`) is below too.
// ADR-0231
fn floor_verdict(cli_version: Option<&str>, user_agent: &str) -> Floor {
    let from_agent = || {
        let (name, rest) = user_agent.split_once('/')?;
        if name.is_empty() {
            return None;
        }
        leading_semver(rest)
    };
    match cli_version.and_then(leading_semver).or_else(from_agent) {
        Some((v, prerelease))
            if v > CLIENT_MESSAGE_ID_FLOOR || (v == CLIENT_MESSAGE_ID_FLOOR && !prerelease) =>
        {
            Floor::Above
        }
        _ => Floor::Below,
    }
}

fn leading_semver(s: &str) -> Option<((u64, u64, u64), bool)> {
    let mut parts = [0u64; 3];
    let mut rest = s;
    for (i, slot) in parts.iter_mut().enumerate() {
        let end = rest
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(rest.len());
        if end == 0 {
            return None;
        }
        *slot = rest[..end].parse().ok()?;
        rest = &rest[end..];
        if i < 2 {
            rest = rest.strip_prefix('.')?;
        }
    }
    Some(((parts[0], parts[1], parts[2]), rest.starts_with('-')))
}

// ── Waiting table ────────────────────────────────────────────────────────────────────

enum Waiter {
    /// The sending thread waits on a channel (handshake only — that thread is not the reader).
    Handshake(mpsc::Sender<Result<Value, String>>),
    /// No thread waits and **the response gives a turn id** — the reader applies it where it receives it.
    ///
    /// ★`seq` is the marker of the turn this request opened★ — otherwise a late answer would write its id into the slot of **another
    /// turn opened in between** and the later `interrupt` would aim at the wrong turn.
    TurnStart { seq: u64 },
    /// `turn/steer` of item `id` handed to the running turn — only a refusal reaches that item ([`State::hold_refused_steer`]).
    /// `gen` = that item's `InFlight` generation (= the turn marker) — a late answer cannot move an item that moved to another stage.
    // ADR-0231
    Steer { id: String, gen: u64 },
    /// Only failures are looked at, in the log.
    Fire,
}

struct PendingEntry {
    waiter: Waiter,
    deadline: Instant,
    method: &'static str,
}

/// The waiting table of requests we issued. ★An id the server issued never enters here★ (TRD §4-4): the two id spaces overlap (a server
/// id must be returned as is), so **the envelope shape** tells them apart, not the id value. Lookups happen only on `result`/`error`
/// envelopes.
#[derive(Default)]
struct Pending {
    entries: Mutex<HashMap<i64, PendingEntry>>,
    /// No more answers can come — no new entries are taken. ★Without this, an entry registered **after** the link closed would hold its
    /// thread until its deadline★ (the handshake issues two requests back to back, so that window exists).
    closed: AtomicBool,
}

impl Pending {
    /// `false` = already closed, could not register.
    fn register(&self, id: i64, waiter: Waiter, method: &'static str) -> bool {
        self.register_at(id, waiter, method, Instant::now() + REQUEST_DEADLINE)
    }

    /// The deadline-as-argument branch — ★a seam so tests measure expiry without sleeping for [`REQUEST_DEADLINE`]★ (ADR-0012). The
    /// production caller is only [`Pending::register`].
    fn register_at(
        &self,
        id: i64,
        waiter: Waiter,
        method: &'static str,
        deadline: Instant,
    ) -> bool {
        let mut g = sync::lock(&self.entries);
        // ★Read while holding the lock★ — reading outside would let an entry into a table closed in between.
        if self.closed.load(Ordering::Acquire) {
            return false;
        }
        g.insert(
            id,
            PendingEntry {
                waiter,
                deadline,
                method,
            },
        );
        true
    }

    /// Closes and returns everything that remains — after this [`Pending::register`] always fails.
    fn close(&self) -> Vec<PendingEntry> {
        let mut g = sync::lock(&self.entries);
        self.closed.store(true, Ordering::Release);
        g.drain().map(|(_, e)| e).collect()
    }

    /// ★A string id cannot be ours★ (our counter emits only i64): it becomes `None` and the caller drops it as an "unknown id".
    fn take(&self, id: &RequestId) -> Option<PendingEntry> {
        let key = match id {
            RequestId::Num(n) => *n,
            RequestId::Str(_) => return None,
        };
        let mut g = sync::lock(&self.entries);
        g.remove(&key)
    }

    fn forget(&self, id: i64) {
        let mut g = sync::lock(&self.entries);
        g.remove(&id);
    }

    fn expired(&self, now: Instant) -> Vec<PendingEntry> {
        let mut g = sync::lock(&self.entries);
        let ids: Vec<i64> = g
            .iter()
            .filter(|(_, e)| e.deadline <= now)
            .map(|(k, _)| *k)
            .collect();
        ids.into_iter().filter_map(|k| g.remove(&k)).collect()
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.entries.lock().unwrap_or_else(|p| p.into_inner()).len()
    }
}

// ── Link ──────────────────────────────────────────────────────────────────────

pub(crate) struct CodexAppServerTransport {
    /// Shared by the pump (try_wait) and shutdown (kill+wait). std `Child` caches the status after wait, so a double wait is harmless.
    child: Arc<Mutex<Child>>,
    /// ★Only the writer thread takes it★ — if the reader took this lock, the module header's deadlock would stand. Only the last step of
    /// [`AgentTransport::shutdown`] touches it, with `try_lock` (no blocking).
    stdin: Arc<Mutex<Option<ChildStdin>>>,
    /// Taken in `start()` and moved to the reader. `None` = already started.
    stdout: Mutex<Option<ChildStdout>>,
    stderr: Mutex<Option<ChildStderr>>,
    decoder: Mutex<Option<Box<dyn OutputDecoder>>>,
    /// Taken in `start()` and moved to the writer — the second handshake request goes out with this value as is.
    open_params: Mutex<Option<ThreadOpen>>,
    shutdown: Arc<AtomicBool>,
    state: SharedState,
    pending: Arc<Pending>,
    // ADR-0231
    announcer: Arc<Announcer>,
    /// Filled by `start()` — used by the input thread's announcement ([`announce`]). ★While empty there is nothing to announce★: it
    ///   stands only after the floor decision, made by the writer `start()` launched.
    // ADR-0231
    core: OnceLock<Arc<OutputCore>>,
    /// Our request id counter. ★Server ids do not enter★ — the two spaces are separate.
    next_id: Arc<AtomicI64>,
    sid_sink: Option<SessionIdSink>,
    /// The port called when the peer first echoes a user message ([`Reader::witness_first_turn`]). `None` = nowhere to tell.
    // ADR-0226
    // ADR-0231
    first_turn_sink: Option<FirstTurnSink>,
    /// The port that delivers the connection outcome **once** (`None` = the assembly point gave none = this axis does not exist).
    /// ★Taken in `start()` and moved to the writer — delivery happens only on that thread, exactly once★.
    link_sink: Mutex<Option<LinkSink>>,
    /// The writer thread handle. ★Nobody joins it — waiting inside `shutdown()` is a contract violation★. It is held so that **whether
    /// the thread actually ends is observable** from outside; if it does not end, one session's memory remains and nothing else shows it.
    writer_handle: Mutex<Option<std::thread::JoinHandle<()>>>,
    /// Whether the output this link carries is a structured stream. ★An injected value (ADR-0044/0030/0191)★ — the link does not know
    /// what it carries; the backend that picked this mode does.
    structured: bool,
    /// The owner of the group of processes this link launched — only one, no weak handle handed out. ★Kept as the last field★: fields
    /// drop in declaration order, so this place decides when the group closes as the link is dropped. Whether it can move earlier was not
    /// measured and no test guards this placement.
    group: GroupOwner,
}

impl CodexAppServerTransport {
    /// **The pump is not launched yet** (in `start`). Returns `child_pid` together.
    ///
    /// `open_params` = the value that goes out as the second handshake request ([`ThreadOpen`]). ★The link hard-codes neither the
    ///   policies nor whether to resume★ — which folder to trust, which sandbox · approval policy, and whether to resume a stored thread
    ///   are backend knowledge, so they are injected.
    /// `sid_sink` = where to record the thread id codex issued. `None` = nowhere to record.
    pub(crate) fn open(
        spec: &CommandSpec,
        structured: bool,
        decoder: Option<Box<dyn OutputDecoder>>,
        open_params: ThreadOpen,
        sid_sink: Option<SessionIdSink>,
        link_sink: Option<LinkSink>,
    ) -> Result<(CodexAppServerTransport, Option<u32>), PtyError> {
        let PipedChild {
            child,
            stdin,
            stdout,
            stderr,
            group,
        } = spawn_piped(spec, "codex app-server")?;
        let child_pid = Some(child.id());

        let transport = CodexAppServerTransport {
            child: Arc::new(Mutex::new(child)),
            stdin: Arc::new(Mutex::new(stdin)),
            stdout: Mutex::new(stdout),
            stderr: Mutex::new(stderr),
            decoder: Mutex::new(decoder),
            open_params: Mutex::new(Some(open_params)),
            shutdown: Arc::new(AtomicBool::new(false)),
            state: Arc::new((Mutex::new(State::new()), Condvar::new())),
            pending: Arc::new(Pending::default()),
            announcer: Arc::new(Announcer {
                order: Mutex::new(()),
                ack: Arc::new(DeliveryAck::new()),
            }),
            core: OnceLock::new(),
            next_id: Arc::new(AtomicI64::new(0)),
            sid_sink,
            first_turn_sink: None,
            link_sink: Mutex::new(link_sink),
            writer_handle: Mutex::new(None),
            structured,
            group,
        };

        Ok((transport, child_pid))
    }

    /// Whether this incarnation's receipt notifications are available — **the very** `Arc` the floor decision fills. The assembly point
    /// carries it in `SpawnParts::delivery_ack` (a separately made one would give session and link different values).
    // ADR-0231
    pub(crate) fn delivery_ack(&self) -> Arc<DeliveryAck> {
        self.announcer.ack.clone()
    }

    /// Plugs in the first-turn port — ★call it before `start()`★ (the reader takes it then). This link's session does not count
    /// submissions (`MidTurnPolicy::TransportOwned`), so if missed this incarnation's thread id is not persisted.
    // ADR-0226
    // ADR-0231
    pub(super) fn with_first_turn(mut self, sink: Option<FirstTurnSink>) -> Self {
        self.first_turn_sink = sink;
        self
    }
}

/// If the pump thread panics anywhere, that agent stops silently forever, so it is made visible as Failed.
fn resolve_pump_reason(result: Result<TerminalReason, Caught>) -> TerminalReason {
    match result {
        Ok(reason) => reason,
        Err(caught) => TerminalReason::Error(format!("pump panicked: {caught}")),
    }
}

/// Cuts keeping character boundaries. If cut, leaves that fact.
///
/// ★Do not call this instead of [`sanitize`]★ — the peer string would skip masking and go to the log · screen. Two doors → one gets forgotten.
fn clip(s: &str, limit: usize) -> String {
    if s.chars().count() <= limit {
        return s.to_string();
    }
    let head: String = s.chars().take(limit).collect();
    format!("{head}…(truncated)")
}

/// The door external process strings pass through before reaching the log · screen — ★masking comes before truncation★: the
/// `mask_secrets` patterns need a certain length after the prefix, so cutting first could cut a credential across the boundary below
/// that quantifier and let it escape masking.
fn sanitize(s: &str, limit: usize) -> String {
    clip(&mask_secrets(s), limit)
}

// ── Writer side ─────────────────────────────────────────────────────────────────

fn write_line(stdin: &Mutex<Option<ChildStdin>>, line: &str) -> Result<(), PtyError> {
    let mut guard = sync::lock(stdin);
    let w = guard
        .as_mut()
        .ok_or_else(|| PtyError::WriteFailed("stdin closed".into()))?;
    w.write_all(line.as_bytes())
        .map_err(|e| PtyError::WriteFailed(e.to_string()))?;
    w.flush().map_err(|e| PtyError::WriteFailed(e.to_string()))
}

/// Sends **only one line** queued in the control queue. Returns whether there was anything to send.
///
/// ★One line at a time is the contract★: [`request_blocking`] calls this and must re-measure the deadline in between; draining
/// everything would make one call unbounded and the deadline **would not be evaluated**.
/// ★It does not write while holding the lock★ — takes the line, releases, then writes (ADR-0006).
fn drain_one_control_line(stdin: &Mutex<Option<ChildStdin>>, state: &SharedState) -> bool {
    let line = {
        let (lock, _) = &**state;
        let mut s = sync::lock(lock);
        s.outbox.pop_front()
    };
    match line {
        Some(l) => {
            if let Err(e) = write_line(stdin, &l) {
                tracing::debug!("codex app-server control line write failed: {e}");
            }
            true
        }
        None => false,
    }
}

/// Issues one request and **waits** for the answer. ★Never on the reader thread★ — reading stops while waiting.
///
/// ★It sends control lines while waiting★ — the writer is the only drainer; parking would deadlock with a peer awaiting our refusal until
/// the deadline (ADR-0192). [unverified — whether 0.154.0 puts `thread/start` after an approval request].
/// ★Order is the contract — **measure the deadline first, then write one line**★: the write blocks, so writing first leaves the deadline
/// unevaluated while one write hangs. One blocking write can still hang forever (module header).
/// `budget` = this request's deadline (production: [`REQUEST_DEADLINE`]); an argument as a seam for tests ([`Pending::register_at`], ADR-0012).
fn request_blocking<P: Serialize, R: DeserializeOwned>(
    stdin: &Mutex<Option<ChildStdin>>,
    state: &SharedState,
    pending: &Pending,
    next_id: &AtomicI64,
    method_name: &'static str,
    params: &P,
    budget: Duration,
) -> Result<R, String> {
    let id = next_id.fetch_add(1, Ordering::Relaxed);
    let (tx, rx) = mpsc::channel();
    if !pending.register(id, Waiter::Handshake(tx), method_name) {
        return Err(format!("{method_name}: the link closed"));
    }

    let line = match protocol::request_line(&RequestId::Num(id), method_name, params) {
        Ok(l) => l,
        Err(e) => {
            pending.forget(id);
            return Err(format!("{method_name} serialization failed: {e}"));
        }
    };
    if let Err(e) = write_line(stdin, &line) {
        pending.forget(id);
        return Err(format!("{method_name} write failed: {e}"));
    }

    let deadline = Instant::now() + budget;
    loop {
        match rx.recv_timeout(SWEEP_INTERVAL) {
            Ok(Ok(v)) => {
                return serde_json::from_value(v).map_err(|e| {
                    format!(
                        "{method_name} response decode failed: {}",
                        sanitize(&e.to_string(), LOG_STRING_LIMIT)
                    )
                })
            }
            Ok(Err(msg)) => return Err(msg),
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                pending.forget(id);
                return Err(format!("{method_name}: the waiting table closed"));
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                // ★Measure before writing★ — the order contract of the doc above.
                if Instant::now() >= deadline {
                    pending.forget(id);
                    return Err(format!(
                        "{method_name}: no answer within {} seconds",
                        budget.as_secs()
                    ));
                }
                drain_one_control_line(stdin, state);
            }
        }
    }
}

/// The handshake. Order is the contract — `initialize` → (`initialized`) → **the second request `open` picked** (`thread/start` or
/// `thread/resume`).
///
/// ★`initialize` works only once (measured 0.154.0)★, so this runs once per incarnation, never again even on failure (ADR-0192).
/// ★A refused second request is `Err` as is — no fallback to a new thread (ADR-0082)★: `-32600` is not specific to "unknown thread"
/// (unknown method · second `initialize` · config error all use it — [`tests::a_peer_error_message_is_masked_before_it_leaves_this_module`]),
/// so falling back would overwrite a sound handle on an unrelated failure.
fn handshake(
    stdin: &Mutex<Option<ChildStdin>>,
    state: &SharedState,
    pending: &Pending,
    next_id: &AtomicI64,
    open: &ThreadOpen,
    deadline: Instant,
) -> Result<Opened, String> {
    // ★The deadline covers **the whole section**, not each request★ — per request the round trips sum to a multiple of the cap and
    //   can exceed the manager's backstop, so this link's failure could not take part in the decision ([`HANDSHAKE_BUDGET`]).
    // ★So the deadline is **received from outside**★: the history paging right after ([`hydrate_history`]) shares it. A fresh
    //   `Instant::now()` here would give each 10 s and a resume that succeeded while fetching history would be decided a failure.
    // ★With 0 remaining, `request_blocking` returns at once with a deadline expiry★ — the only deciding place, so it is not
    //   pre-split here.
    let remaining = || deadline.saturating_duration_since(Instant::now());

    let init: InitializeResponse = request_blocking(
        stdin,
        state,
        pending,
        next_id,
        method::INITIALIZE,
        &InitializeParams {
            client_info: ClientInfo {
                name: CLIENT_NAME.to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
            },
        },
        remaining(),
    )?;
    // ★The only record that tells upstream drift apart after the fact★ — the protocol has no version field (`protocolVersion` appears 0
    //   times in the schema) and the CLI self-updates.
    tracing::info!(
        user_agent = %sanitize(&init.user_agent, LOG_STRING_LIMIT),
        platform = %sanitize(&init.platform_os, LOG_STRING_LIMIT),
        "codex app-server connected"
    );

    // Not an obligation (measured 0.154.0 — `thread/start` succeeds without it); harmless, kept to match the handshake shape.
    write_line(stdin, &protocol::notification_line(method::INITIALIZED))
        .map_err(|e| format!("initialized write failed: {e}"))?;

    // ★Both branches return the same `thread`, so one return★ — on resume too **the id the peer gave is raised as is**: if it differs
    //   from what we sent, that value is the thread this incarnation actually talks in, and the record verb must receive it so the next
    //   resume opens the right place.
    let (thread, items_backwards_cursor): (Thread, Option<String>) = match open {
        ThreadOpen::Start(params) => {
            let started: ThreadStartResponse = request_blocking(
                stdin,
                state,
                pending,
                next_id,
                method::THREAD_START,
                params,
                remaining(),
            )?;
            // ★A new conversation has no history — the response does not even have that field★ (schema 0.154.0).
            (started.thread, None)
        }
        ThreadOpen::Resume(params) => {
            let resumed: ThreadResumeResponse = request_blocking(
                stdin,
                state,
                pending,
                next_id,
                method::THREAD_RESUME,
                params,
                remaining(),
            )?;
            (resumed.thread, resumed.items_backwards_cursor)
        }
    };
    let floor = floor_verdict(thread.cli_version.as_deref(), &init.user_agent);
    tracing::info!(
        cli_version = ?thread.cli_version.as_deref().map(|v| sanitize(v, LOG_STRING_LIMIT)),
        resumed = matches!(open, ThreadOpen::Resume(_)),
        history_cursor = items_backwards_cursor.is_some(),
        above_floor = floor == Floor::Above,
        "codex thread opened"
    );
    Ok(Opened {
        thread_id: thread.id,
        items_backwards_cursor,
        floor,
    })
}

/// What the handshake set up — the thread this incarnation talks in, and the door into its past screen.
///
/// ★The two fields are of **different grades**★: without `thread_id` nothing works (handshake failure); without
///   `items_backwards_cursor` nothing breaks — nothing to restore or a new conversation, starting with an empty screen (ADR-0203).
struct Opened {
    thread_id: String,
    items_backwards_cursor: Option<String>,
    /// The floor decision — applied after the session id record returns ([`settle_floor`]).
    // ADR-0231
    floor: Floor,
}

/// Fetches the resumed thread's past screen **from the end** and carries it into neutral events (ADR-0203).
///
/// ★Failure does not create failure★: **it emits `Err` on no branch** — no cursor, refusal, over-cap line, budget out all end as "the events
/// up to there"; unfetched history is **an empty screen**, not a failed activation (ADR-0079).
/// ★It **shares** [`HANDSHAKE_BUDGET`] — no new clock★ (a second one could exceed [`crate::manager::LINK_RESOLUTION_BACKSTOP`]).
/// Stops: ① cursor exhausted ② ring filled ([`REPLAY_MAX_BYTES`] via the ring's [`estimate_cost_bytes`]) ③ budget ④ [`MAX_HISTORY_PAGES`].
/// Direction: `sortDirection: desc` + the resume's backward cursor; pages and items are newest first, reversed while gathering and flipped
/// once at the end.
/// ★A translator is made **for this call only**★ — the pump's `Box<dyn OutputDecoder>` is out of reach; history yields each item once, so
/// only the diagnostic count maps split.
// ADR-0203
fn hydrate_history(
    stdin: &Mutex<Option<ChildStdin>>,
    state: &SharedState,
    pending: &Pending,
    next_id: &AtomicI64,
    opened: &Opened,
    deadline: Instant,
) -> Vec<OutputEvent> {
    let Some(cursor) = opened.items_backwards_cursor.clone() else {
        // A new conversation, or the peer gave no entry point — empty screen.
        return Vec::new();
    };
    collect_history(&opened.thread_id, cursor, deadline, |params, budget| {
        request_blocking(
            stdin,
            state,
            pending,
            next_id,
            method::THREAD_ITEMS_LIST,
            params,
            budget,
        )
    })
}

/// The **round-trip-free core** of the function above — how pages are walked, where it stops, in what order it hands them out.
///
/// ★`fetch` is injected so this rule can be measured alone, without a child process★ (ADR-0012); production passes
/// [`request_blocking`]. ★An `Err` from `fetch` is **non-fatal**★ — what was gathered is emitted as is.
fn collect_history(
    thread_id: &str,
    entry_cursor: String,
    deadline: Instant,
    mut fetch: impl FnMut(&ThreadItemsListParams, Duration) -> Result<ThreadItemsListResponse, String>,
) -> Vec<OutputEvent> {
    let mut cursor = entry_cursor;
    let mut decoder = CodexAppServerDecoder::new();
    // Piled newest → oldest. One slot = the events one item emitted + the **group number** it belongs to. ★An item that carried nothing
    //   takes no slot★ — boundaries stand only **between turns with content**.
    // ★★The key is a **number**, not the original id — a memory cap★★: `turnId` has no wire cap (the line cap bounds one page, not this
    //   accumulation), so long ids would pile up hundreds of MiB; folding all over-cap ids into one `None` slot would **render two
    //   conversations as one turn**. So: **distinction** = comparison with the previous item's original id only (`last_raw`, O(1)); **display**
    //   = one id per group, **filtered, not truncated** ([`MAX_TURN_ID_BYTES`], `groups`; truncation could make different turns equal —
    //   as `MAX_ID_BYTES` of `decoder`). ★Price: **no id is carried** on the boundary of an over-cap turn★.
    let mut newest_first: Vec<(u32, Vec<OutputEvent>)> = Vec::new();
    let mut groups: Vec<Option<String>> = Vec::new();
    let mut last_raw: Option<String> = None;
    let mut cost = 0usize;
    let mut count = 0usize;
    let mut pages = 0usize;
    let mut hit_ceiling = false;

    while pages < MAX_HISTORY_PAGES {
        let budget = deadline.saturating_duration_since(Instant::now());
        if budget.is_zero() {
            tracing::info!(
                "codex history restore: used up the connection budget — carrying only up to here"
            );
            break;
        }
        pages += 1;
        let params = ThreadItemsListParams {
            thread_id: thread_id.to_string(),
            cursor: Some(cursor.clone()),
            limit: Some(HISTORY_PAGE_LIMIT),
            sort_direction: Some(SortDirection::Desc),
        };
        let page = match fetch(&params, budget) {
            Ok(page) => page,
            Err(reason) => {
                // ★`info`, not `warn`★ — "restored only up to here" is a fact, not a defect, and the connection stands. The reason is still logged
                //   (first signal of upstream drift).
                // ★Kept **knowing this branch swallows "the connection died"**★: reader EOF wakes the waiting entry with an error and
                //   lands here, indistinguishable by string. That decision is made by [`open_gate`], which looks at the state directly.
                tracing::info!(
                    page = pages,
                    "codex history restore stops here — only the screen gets shorter, the connection stays as is: {}",
                    sanitize(&reason, LOG_STRING_LIMIT)
                );
                break;
            }
        };
        let next = page.next_cursor;
        for entry in page.data {
            let translated = decoder.history_item(&entry.turn_id, &entry.item);
            if translated.is_empty() {
                continue;
            }
            let new_run = last_raw.as_deref() != Some(entry.turn_id.as_str());
            let shown =
                (entry.turn_id.len() <= MAX_TURN_ID_BYTES).then_some(entry.turn_id.as_str());
            // ★Decide the ceiling **before putting**, **counting the boundaries to be synthesized too**★: ① measuring after putting always
            //   exceeds by one entry ② without boundaries, a history with many turns would hold up to twice the counted events (every item
            //   splitting a turn) and the ring would drop half on landing — the mismatch tying the ceiling to the ring constant removes.
            // ★One boundary per new run★ — in time order there is one closing boundary per run, so counting each run change while walking
            //   in reverse gives the total.
            let boundary = new_run.then(|| history_turn_boundary(shown));
            let add_events = translated.len() + usize::from(boundary.is_some());
            let add_cost = translated.iter().map(estimate_cost_bytes).sum::<usize>()
                + boundary.as_ref().map(estimate_cost_bytes).unwrap_or(0);
            // ★The first item is always taken★ — even one over the ceiling beats an empty screen, and the ring keeps the newest by the same
            //   rule (the `len() > 1` guard of `Ring::push`).
            if !newest_first.is_empty()
                && (cost + add_cost > REPLAY_MAX_BYTES || count + add_events > REPLAY_MAX_EVENTS)
            {
                hit_ceiling = true;
                break;
            }
            count += add_events;
            cost += add_cost;
            if new_run {
                groups.push(shown.map(String::from));
                // ★Only **the previous** original is held★ — this assignment drops the old value, so nothing accumulates.
                last_raw = Some(entry.turn_id);
            }
            newest_first.push((groups.len() as u32 - 1, translated));
        }
        if hit_ceiling {
            break;
        }
        match next {
            // ★The same cursor again stops the walk★ — otherwise the same page is refetched up to the page cap, carrying **several copies of the
            //   same conversation** that push the real history out of the ring; the cap bounds the loop but not this contamination.
            //   ★Only **cycles of length 1** (A→A) are caught; **all of length ≥ 2 leak** (A→B→A, …)★ — those are stopped by the page cap ·
            //   ring ceiling · budget, meanwhile duplicates are visible and activation is delayed by the budget. Not "capped, so harmless".
            //   ★Not fixed because of **cost**, not value★: catching longer cycles means remembering seen cursors, strings whose length the
            //   peer decides — the same accumulated cost as `turnId` above — and there are no grounds for how many to remember.
            Some(next) if next == cursor => {
                tracing::warn!(
                    page = pages,
                    "codex history restore: the peer returned the same cursor — stopping without carrying duplicates"
                );
                break;
            }
            Some(next) => cursor = next,
            None => break,
        }
    }

    // ★Flipped once here back to time order★ — in the ring, insertion order is screen order.
    newest_first.reverse();
    let items = newest_first.len();
    let mut events: Vec<OutputEvent> = Vec::with_capacity(count);
    let mut open_group: Option<u32> = None;
    for (group, translated) in newest_first {
        if open_group != Some(group) {
            if let Some(previous) = open_group {
                events.push(history_turn_boundary(groups[previous as usize].as_deref()));
            }
            open_group = Some(group);
        }
        events.extend(translated);
    }
    // ★The last turn must be closed too★ — else the restored slot's waiting indicator spins forever ([`history_turn_boundary`]).
    if let Some(last) = open_group {
        events.push(history_turn_boundary(groups[last as usize].as_deref()));
    }

    debug_assert_eq!(
        events.len(),
        count,
        "the ceiling accounting diverged from the actual event count — a regression of not counting boundaries"
    );
    tracing::info!(
        pages,
        items,
        events = events.len(),
        bytes = cost,
        ring_full = hit_ceiling,
        "codex history restored"
    );
    events
}

/// Opens the gate — ★**only while this incarnation's connection is still ours**★.
///
/// Else a dead link stands as `Ready`: if the peer closes stdout during history, [`ReaderExit`] sets `closed`/[`Link::Down`] and wakes
/// waiters with an error that history treats as **partial success**; the next line would overwrite `Down` with `Ready` and **deliver
/// something dead as a successful activation.** The split axis is "is the connection standing now" (state read directly), not "what failed".
/// The `Err` wording is the state's reason as is (a JSON-RPC error on stdout or stream end — in neither console nor stderr tail).
// ADR-0203
// ADR-0201
fn open_gate(state: &SharedState, thread_id: String) -> Result<(), String> {
    let (lock, cv) = &**state;
    let mut s = sync::lock(lock);
    if let Link::Down(reason) = &s.link {
        return Err(reason.clone());
    }
    if s.closed {
        return Err("the connection closed".to_string());
    }
    s.thread_id = Some(thread_id);
    s.link = Link::Ready;
    cv.notify_all();
    Ok(())
}

enum Job {
    /// A control line that does not go through the gate.
    Line(String),
    /// A user turn taken out of the queue — the waiting entry is registered before writing. `seq` = the marker of the turn this Job
    /// opens. `item` = the id of the list item it carried (handed-over mark emitted after writing — [`publish_hand_over`]); `None` for
    /// Direct · mail · items without an identifier.
    Turn {
        id: i64,
        seq: u64,
        line: String,
        item: Option<String>,
    },
    /// This wake-up is only the deadline sweep.
    Sweep,
    /// Hands one held item released by the tool-end trigger to the running turn — waiting entry registered before writing ([`issue_steer`]).
    // ADR-0231
    Steer(Steer),
}

/// ★The only place where "can it send" and "transition to in-progress" are joined under one lock★ (ADR-0193).
///
/// What it opens with = the Idle order ([`State::idle_opening`]). A pre-announce head waits (typing order). An item with an identifier
/// (floor met) carries `clientUserMessageId` and stays `InFlight` for the echo. ★An item without one leaves at opening — it has no
/// comparison key, so the turn-end disposal could never close it and it would block mail★. One `turn/start` = one item (PRD §3-8).
/// ★It does not open while Active★ — a `turn/start` during a turn is handled by the vendor as a steer (TRD §3-2); the first-line
/// `Idle` check prevents it.
// ADR-0231
fn take_turn_locked(s: &mut State, next_id: &AtomicI64) -> Option<Job> {
    if !matches!(s.link, Link::Ready) || !matches!(s.turn, TurnState::Idle) {
        return None;
    }
    let thread_id = s.thread_id.clone()?;
    let pos = s.idle_opening()?;
    let head = &s.pending[pos];
    if !head.announced {
        return None;
    }
    let tracked = s.floor == Floor::Above && head.id.is_some();
    let params = TurnStartParams {
        thread_id,
        input: vec![UserInput::Text {
            text: String::from_utf8_lossy(&head.body).into_owned(),
        }],
        client_user_message_id: if tracked { head.id.clone() } else { None },
    };
    let id = next_id.fetch_add(1, Ordering::Relaxed);
    match protocol::request_line(&RequestId::Num(id), method::TURN_START, &params) {
        Ok(line) => {
            let seq = s.next_turn_seq;
            s.next_turn_seq += 1;
            let item = if tracked {
                let head = &mut s.pending[pos];
                head.stage = Stage::InFlight {
                    gen: seq,
                    turn_id: None,
                };
                head.id.clone().filter(|_| head.listed)
            } else {
                s.pending.remove(pos);
                None
            };
            s.turn = TurnState::Active { seq, turn_id: None };
            Some(Job::Turn {
                id,
                seq,
                line,
                item,
            })
        }
        Err(e) => {
            // Our types cannot fail to serialize, but by contract it is open; dropping the body would be a silent loss, so it stays queued and
            //   is retried at the next wake-up.
            tracing::warn!("turn/start serialization failed — this body stays in the queue: {e}");
            None
        }
    }
}

/// If the tool-end trigger ([`State::steer_due`]) stands, picks one held item for the running turn, moves it to `InFlight` and builds its
/// `turn/steer` (ADR-0235). ★It does not open a turn★ — only [`take_turn_locked`] does (ADR-0193).
///
/// Candidate = the oldest `Held` **user** item; mail is skipped (outside the list, waits for Idle). One request = one item (PRD §3-8);
/// the writer calls again until all held text is handed over, each with its own `clientUserMessageId`.
/// ★Nothing to hand over → the trigger is lowered★: no held user text · no running turn with a known id (`expectedTurnId`) · the turn's
/// 「steer refused」 mark ([`State::steer_refused`]) · candidate not a list item (Direct goes only by `turn/start`) · below the floor.
/// ★A pre-announce candidate waits with the trigger standing★ ([`announce`] wakes the writer). ★Split from ✕ under the same lock★ — a ✕
/// after the move to `InFlight` is `TooLate`.
// ADR-0231
// ADR-0235
fn take_steer_locked(s: &mut State, next_id: &AtomicI64) -> Option<Steer> {
    if !s.steer_due {
        return None;
    }
    let target = match (&s.turn, &s.thread_id) {
        (
            TurnState::Active {
                seq,
                turn_id: Some(turn_id),
            },
            Some(thread_id),
        ) if s.floor == Floor::Above
            && matches!(s.link, Link::Ready)
            && s.steer_refused != Some(*seq) =>
        {
            Some((*seq, turn_id.clone(), thread_id.clone()))
        }
        _ => None,
    };
    let pos = s
        .pending
        .iter()
        .position(|p| matches!(p.stage, Stage::Held) && p.origin == InputOrigin::User);
    let (Some((gen, turn_id, thread_id)), Some(pos)) = (target, pos) else {
        s.steer_due = false;
        return None;
    };
    let head = &s.pending[pos];
    if !head.announced {
        return None;
    }
    let Some(item) = head.id.clone().filter(|_| head.listed) else {
        s.steer_due = false;
        return None;
    };
    let params = TurnSteerParams {
        thread_id,
        expected_turn_id: turn_id.clone(),
        client_user_message_id: item.clone(),
        input: vec![UserInput::Text {
            text: String::from_utf8_lossy(&head.body).into_owned(),
        }],
    };
    let request = next_id.fetch_add(1, Ordering::Relaxed);
    match protocol::request_line(&RequestId::Num(request), method::TURN_STEER, &params) {
        Ok(line) => {
            s.pending[pos].stage = Stage::InFlight {
                gen,
                turn_id: Some(turn_id.clone()),
            };
            Some(Steer {
                request,
                item,
                gen,
                line,
                turn_id,
            })
        }
        Err(e) => {
            // Cannot fail to serialize but by contract it is open — the item stays held and goes out at the turn end; the trigger is lowered so
            //   the failure is not repeated on every wake-up.
            tracing::warn!("turn/steer serialization failed — this item stays held: {e}");
            s.steer_due = false;
            None
        }
    }
}

fn next_job(state: &SharedState, next_id: &AtomicI64) -> Option<Job> {
    let (lock, cv) = &**state;
    let mut s = sync::lock(lock);
    loop {
        if s.closed {
            return None;
        }
        if let Some(line) = s.outbox.pop_front() {
            return Some(Job::Line(line));
        }
        if let Some(job) = take_turn_locked(&mut s, next_id) {
            return Some(job);
        }
        // While the tool-end trigger stands, held user text one at a time; the function lowers the trigger when nothing is left.
        // ADR-0231
        if let Some(steer) = take_steer_locked(&mut s, next_id) {
            return Some(Job::Steer(steer));
        }
        let (guard, timeout) = sync::wait_timeout(cv, s, SWEEP_INTERVAL);
        s = guard;
        if timeout.timed_out() {
            return Some(Job::Sweep);
        }
    }
}

/// Ends the turn only when `seq` is **exactly the turn in progress now**, disposes its items with `close`, releases the queue and **emits
/// the boundary** (disposal events first). ★Without the marker check a late signal would close the next turn★. Returns whether it ended.
///
/// ★Emitting the boundary is part of this function★ (ADR-0127): on these paths `turn/completed` **does not come** (write failure · decode
/// failure · error response · deadline · turn id over the cap), and only a boundary returns the fact layer to "not mid-turn" — else the
/// incarnation is **observed mid-turn although idle**, blocking mail until the 30-minute fail-open. An [`OutputEvent::Error`] line cannot
/// replace it (not a turn signal); the reason rides **inside** the boundary (no extra ordering contract).
/// The outcome is always `Failed`: `close` = `Rejected` (write failure · error response) or `Unanswered` (the rest).
fn end_turn_if(
    state: &SharedState,
    core: &OutputCore,
    seq: u64,
    detail: String,
    close: TurnClose,
) -> bool {
    let ended = {
        let (lock, cv) = &**state;
        let mut s = sync::lock(lock);
        match &s.turn {
            TurnState::Active { seq: cur, turn_id } if *cur == seq => {
                let turn_id = turn_id.clone();
                s.turn = TurnState::Idle;
                // ★The held end is dropped with this turn★: that slot waits only for the id of **the turn that was open**; left behind, the moment
                //   the next turn reuses that id [`Reader::resolve`] would close the fresh turn and raise a stale boundary. A turn opens only
                //   from `Idle` ([`take_turn_locked`]), so emptying at every idle spot keeps any slot from carrying over.
                s.early_completions.clear();
                // ADR-0231
                let drops = s.close_turn_items(seq, close);
                cv.notify_all();
                Some((turn_id, drops))
            }
            _ => None,
        }
    };
    match ended {
        // ★Emit after releasing the lock★ (ADR-0006) — subscribers run arbitrary code inside this call.
        Some((turn_id, drops)) => {
            drops.emit(core, state);
            core.emit(OutputEvent::TurnEnd {
                turn_id,
                outcome: TurnOutcome::Failed {
                    detail: Some(detail),
                },
            });
            true
        }
        None => false,
    }
}

/// The `turn/start` of the turn the writer opened could not be written — withdraws the waiting entry and closes that turn as `Rejected`
/// (the text did not reach the vendor — it returns to the held place behind the halt · [`State::close_turn_items`]).
///
/// ★This failure has no way back to the caller★ — the call already returned `Ok`; all that remains is closing the turn as a failure.
// ADR-0231
fn turn_write_failed(
    state: &SharedState,
    pending: &Pending,
    core: &OutputCore,
    id: i64,
    seq: u64,
    e: &PtyError,
) {
    pending.forget(id);
    end_turn_if(
        state,
        core,
        seq,
        format!("codex app-server input send failed: {e}"),
        TurnClose::Rejected,
    );
}

/// The outcome of [`issue_steer`].
// ADR-0231
enum Issued {
    /// Fully written — the handed-over mark was emitted.
    Written,
    /// Could not write — the item was returned to the held place ([`steer_write_failed`]).
    Failed,
    /// The waiting table closed = the link closed, so the writer ends.
    Closed,
}

/// Emits one steer the writer picked. ★The waiting entry is registered **before** the write★ — after it, a fast response would arrive
/// first and be dropped as an "unknown id", leaving the item waiting. `write` = writing one line to the peer's stdin (production =
/// [`write_line`]; tests swap it).
/// ★Known remainder★: on `Closed` the item just moved stays `InFlight` — the link already closed and the core's end synthesis closes it.
// ADR-0231
fn issue_steer(
    state: &SharedState,
    pending: &Pending,
    core: &OutputCore,
    steer: Steer,
    write: impl FnOnce(&str) -> Result<(), PtyError>,
) -> Issued {
    let waiter = Waiter::Steer {
        id: steer.item.clone(),
        gen: steer.gen,
    };
    if !pending.register(steer.request, waiter, method::TURN_STEER) {
        return Issued::Closed;
    }
    match write(&steer.line) {
        Ok(()) => {
            // The handed-over mark — emitted **after** writing (value read from the current stage — [`publish_hand_over`]).
            publish_hand_over(core, state, &steer.item);
            Issued::Written
        }
        Err(e) => {
            steer_write_failed(state, pending, core, &steer, &e);
            Issued::Failed
        }
    }
}

/// A steer the writer picked could not be written — withdraws the waiting entry and returns the item to the held place (it did not
/// reach the vendor · user decision 2026-09-26 B — not erased). Disposal = a steer refusal ([`State::hold_refused_steer`]): the
/// 「steer refused」 mark stops the next tool end of the same turn from rewriting the same text; the next Idle opens it; a ✕-reached item
/// is withdrawn.
/// ★The turn is left as is★ — the steer did not open a turn, it keeps running at the vendor, and it is no halt trigger (TRD §5-5 five
/// exits). Generation masking as in refusal — an item already disposed by the turn end is untouched and gets no mark.
// ADR-0231
fn steer_write_failed(
    state: &SharedState,
    pending: &Pending,
    core: &OutputCore,
    steer: &Steer,
    e: &PtyError,
) {
    pending.forget(steer.request);
    let held = {
        let (lock, cv) = &**state;
        let mut s = sync::lock(lock);
        let held = s.hold_refused_steer(&steer.item, steer.gen);
        cv.notify_all();
        held
    };
    tracing::warn!(
        "codex app-server turn/steer send failed — that text returns to the held place: {e}"
    );
    // ADR-0235: also left as a trace line so it is visible to QA reading only the trace.
    if let Some(t) = steer.trace(SteerPhase::WriteFailed) {
        t.write(core.id());
    }
    if let Some(out) = held {
        out.emit(core, state);
    }
}

fn sweep_deadlines(state: &SharedState, pending: &Pending, core: &OutputCore) {
    let now = Instant::now();
    for entry in pending.expired(now) {
        let reason = format!(
            "{}: no answer within {} seconds",
            entry.method,
            REQUEST_DEADLINE.as_secs()
        );
        match entry.waiter {
            // ★This channel is also woken by `recv_timeout`★ — if both stand the first wins; a closed channel makes send fail quietly.
            Waiter::Handshake(tx) => {
                let _ = tx.send(Err(reason));
            }
            Waiter::TurnStart { seq } => {
                tracing::warn!("{reason}");
                if !end_turn_if(
                    state,
                    core,
                    seq,
                    format!("codex app-server: {reason}"),
                    TurnClose::Unanswered,
                ) {
                    tracing::debug!("the turn opened by a turn/start past its deadline has already ended — leaving it as is");
                }
            }
            // ★Same as a success response — the item is not moved★ (reason = the steer response spot of [`Reader::resolve`]): it waits for the
            //   echo, and a turn ending without one disposes it ([`State::close_turn_items`]). An expired steer processed late at the vendor
            //   cannot double-deliver — `expectedTurnId` ties it to that turn and it is refused after the turn ends.
            // ADR-0231
            Waiter::Steer { .. } => tracing::warn!("{reason}"),
            Waiter::Fire => tracing::warn!("{reason}"),
        }
    }
}

/// Hands the received thread id to the assembly point's record verb. `Err` = this incarnation's connection must not be set up.
///
/// Not persisted here — persistence happens at the peer's first user message echo ([`Reader::witness_first_turn`] · ADR-0226). ★The gate's
/// condition is "the record call returned", not "on disk"★ (the port swallows failures into the log). `None` port = nowhere to record.
/// ★Called before [`Link::Ready`]★ — a turn must not go out before the session is recorded.
/// ★The closed fence only narrows the window★: it blocks an end **already observed**. The rest is dominated by registry lock contention
/// (held **through the whole `agents.json` rewrite** — ADR-0071), so `shutdown()` · `join_pump` · the reaper can fit inside and a late write
/// records the sid into an already reaped session's profile and pushes the old sid into history; the incarnation marker does not help (no end
/// path touches `epoch`). Closing it needs one critical section on the registry side.
/// `catch_unwind` only keeps the codex writer from dying on top of an already broken (poisoned) registry — the graph below has no panic
/// source of our code. ★Not reached in production★ (the writer is outside the containment marking: the hook ends the process — ADR-0292);
/// the `Err` outcome is seen only in a hook-less build, where an unwind would leave the link `Connecting` with `send_input` returning `Ok`
/// while the queue fills (ADR-0190).
fn record_session_id(
    state: &SharedState,
    shutdown: &AtomicBool,
    sid_sink: Option<&SessionIdSink>,
    thread_id: &str,
) -> Result<(), String> {
    let Some(sink) = sid_sink else {
        tracing::debug!(
            "received the codex thread id but there is nowhere to record it — the assembly point gave no record port"
        );
        return Ok(());
    };

    // ★Look at both markers together★ — [`AgentTransport::shutdown`] sets this atomic **first**, then takes the state lock to set
    //   `closed`; the gap is not empty (the lock may be held by `send_input` · [`next_job`] · the reader's refusal path). One marker leaks
    //   the span; OR-ing both closes it.
    let already_ended = shutdown.load(Ordering::Acquire) || {
        let (lock, _) = &**state;
        let s = sync::lock(lock);
        s.closed
    };
    if already_ended {
        // ★A normal end, so no failure outcome★ — `Err` would stand one more error boundary on a session that already ended.
        tracing::debug!("not recording the codex thread id — this incarnation has already ended");
        return Ok(());
    }

    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| sink(thread_id))).map_err(|_| {
        tracing::error!(
            "the codex session id record panicked — taking down this incarnation's connection"
        );
        "the session id record panicked".to_string()
    })
}

/// One reason the connection could not stand. ★The two fields answer **different questions**★ — `reason` = what went wrong,
/// `while_recording` = **at which stage**; the latter only decides what to tell a person (headline · `detail`).
///
/// ★**The stdin disposal no longer splits by this field**★ — on failure it is closed on every branch ([`writer_loop`] has the reason).
/// The field once meant "is the peer alive" and doubled as that decision, which let **round-trip failure but peer alive** (codex's
/// `thread/resume` refusal) leak entirely. Do not bring that meaning back.
struct HandshakeFailure {
    reason: String,
    /// Did the two round trips succeed and it fell over at the record stage? Only splits the sentence given to a person.
    while_recording: bool,
}

impl HandshakeFailure {
    /// The headline for screen · log. ★The two branches get different sentences★ — writing a record failure as "handshake failed" would
    /// make one suspect the protocol round trip (it succeeded).
    fn headline(&self) -> &'static str {
        if self.while_recording {
            "codex app-server session id record failed (the protocol round trip succeeded)"
        } else {
            "codex app-server handshake failed"
        }
    }
}

/// Delivers the connection outcome — ★**at most once**★; not delivered if the user turned it off.
///
/// A handshake released by kill is not "the peer answered": delivering it would **record the user's cancellation as a resume failure** and
/// clean up twice (defect ③). The `shutdown` atomic is a latch set **first** on the kill path.
/// ★"At most once" holds by **the call-site shape**, not this function★ (it delivers every call): the production sites are the two branches
/// of one `match` in [`writer_loop`], and only the writer holds the port (`take` in [`AgentTransport::start`] prevents a second thread).
/// ★Only the message is suppressed — the channel still disappears★: the receiver wakes with `Disconnected`, so it checks the end-intent latch
/// first ([`crate::manager::AgentManager::link_activation_verdict`]).
fn deliver_link(shutdown: &AtomicBool, sink: &Option<LinkSink>, resolution: LinkResolution) {
    if shutdown.load(Ordering::Acquire) {
        tracing::debug!(
            "not delivering the connection outcome — the user turned this incarnation off"
        );
        return;
    }
    if let Some(sink) = sink {
        sink(resolution);
    }
}

#[allow(clippy::too_many_arguments)]
fn writer_loop(
    stdin: Arc<Mutex<Option<ChildStdin>>>,
    state: SharedState,
    pending: Arc<Pending>,
    next_id: Arc<AtomicI64>,
    shutdown: Arc<AtomicBool>,
    core: Arc<OutputCore>,
    open_params: ThreadOpen,
    sid_sink: Option<SessionIdSink>,
    link_sink: Option<LinkSink>,
    announcer: Arc<Announcer>,
) {
    // ★The two failures are carried **separately**★: the "handshake" runs **up to the record call returning** ([`record_session_id`]), so
    //   both block `Ready` and ride the cleanup below. Difference: after a record failure the round trip succeeded, so **the peer is
    //   alive and reading stdin**; after a round-trip failure no certainty — ★no certainty is not "dead"★ (a `thread/resume` refusal
    //   comes from a live peer). ★This splits **only what to tell a person**★; the stdin disposal below is the same on every branch.
    // ★★The budget clock starts **here, once** — handshake and history share it★★ ([`hydrate_history`]).
    let link_deadline = Instant::now() + HANDSHAKE_BUDGET;
    let outcome = match handshake(
        &stdin,
        &state,
        &pending,
        &next_id,
        &open_params,
        link_deadline,
    ) {
        Err(reason) => Err(HandshakeFailure {
            reason,
            while_recording: false,
        }),
        Ok(opened) => {
            match record_session_id(&state, &shutdown, sid_sink.as_ref(), &opened.thread_id) {
                Ok(()) => Ok(opened),
                Err(reason) => Err(HandshakeFailure {
                    reason,
                    while_recording: true,
                }),
            }
        }
    };

    match outcome {
        Ok(opened) => {
            // ★Write the floor decision **after** the record returns and **before** the history★ ([`settle_floor`]) — the window in which
            //   pre-decision list items appear late is bound to the handshake round trip (TRD §5-5). It is before the gate, so they cannot
            //   go out yet.
            // ADR-0231
            settle_floor(&core, &state, &announcer, opened.floor);
            // ★Raise the past screen **before opening the gate** — do not flip the order★ (ADR-0203): gate first, and queued user turns could go
            //   out and their response enter the ring **before** the unraised history, attaching the restored conversation after the new
            //   answer. (This replaces ADR-0079's seed-before-publish: here history comes **from a peer that exists only after the
            //   handshake**, so instead ⓐ it is before the gate — no turn of ours yet — and ⓑ the door below **fans out too**.)
            // ★`emit_without_turn_observation` is ⓑ★: `seed` only fills the ring without fanout, so a subscriber attached in between
            //   would **never see** the past screen. ★It touches no observation · state · finalize★ (ADR-0005 · ADR-0113 · ADR-0127) —
            //   past records as progress would leave that turn's end never coming. ★Raised **as one chunk**★ — item by item, live lines
            //   would take seqs in the gaps ([`OutputCore::emit_batch_without_turn_observation`]).
            core.emit_batch_without_turn_observation(hydrate_history(
                &stdin,
                &state,
                &pending,
                &next_id,
                &opened,
                link_deadline,
            ));
            // ★The gate opens **only while the connection still stands**★ — if the stream ended while fetching history, setting `Ready` would
            //   deliver something dead as a successful activation ([`open_gate`]).
            match open_gate(&state, opened.thread_id) {
                // ★Delivered **right after** opening the gate, not under the lock★ (ADR-0006) — this port calls assembly point code that touches channels.
                Ok(()) => deliver_link(&shutdown, &link_sink, LinkResolution::Ready),
                Err(reason) => {
                    // ★No error · boundary is added to the screen here★ — the stream ending means the pump will soon emit the end transition
                    //   (ADR-0005 sole actor), which already closes the screen; a user kill makes `deliver_link` suppress this.
                    tracing::warn!(
                        "codex app-server: the connection ended while fetching history — not opening the gate: {}",
                        sanitize(&reason, LOG_STRING_LIMIT)
                    );
                    deliver_link(&shutdown, &link_sink, LinkResolution::Failed { reason });
                }
            }
        }
        Err(failure) => {
            let HandshakeFailure {
                reason,
                while_recording,
            } = &failure;
            tracing::warn!("{}: {reason}", failure.headline());
            let dropped = {
                let (lock, _) = &*state;
                let mut s = sync::lock(lock);
                s.link = Link::Down(reason.clone());
                let n = s.pending.len();
                s.pending.clear();
                n
            };
            if dropped > 0 {
                tracing::warn!("{dropped} pending input(s) were lost because the handshake failed");
            }
            // ★The reason is **carried** in the delivery★ — this wording is a JSON-RPC error on stdout, in neither the console nor the stderr
            //   diagnostic tail; without carrying it the classification cannot reach it.
            deliver_link(
                &shutdown,
                &link_sink,
                LinkResolution::Failed {
                    reason: reason.clone(),
                },
            );
            // ★This boundary has no link-side turn★: [`take_turn_locked`] requires [`Link::Ready`], so `dropped` inputs are **queued bodies, not
            //   turns**, and the fact layer never saw this incarnation "mid-turn" (ADR-0193).
            // ★Still, **whether or not input was dropped**, one failure outcome is emitted★ — the only thing closing the screen: the frontend
            //   does not read `Error` as a turn end ([`OutputEvent::Error`]), so a slot holding only an error would spin **forever**. No
            //   open turn, so no `turn_id`; one boundary however many bodies were dropped.
            // ★Reason sentences differ on purpose★ — the dropping side says bodies a person sent were destroyed, the other only that the
            //   connection could not stand (claiming loss there would report something that did not happen). ★The reason rides both the error
            //   line and the boundary★ so a boundary-only consumer knows too (as [`end_turn_if`]).
            for entry in pending.close() {
                if let Waiter::Handshake(tx) = entry.waiter {
                    let _ = tx.send(Err(reason.clone()));
                }
            }
            core.emit(OutputEvent::Error(format!(
                "{}: {}",
                failure.headline(),
                sanitize(reason, LOG_STRING_LIMIT)
            )));
            let detail = if dropped > 0 {
                format!(
                    "{} — {dropped} sent input(s) were lost: {}",
                    failure.headline(),
                    sanitize(reason, LOG_STRING_LIMIT)
                )
            } else if *while_recording {
                // ★Here "the connection could not stand" is false★ — the round trip stood and we decided not to use it.
                format!(
                    "{} — this incarnation is not used: {}",
                    failure.headline(),
                    sanitize(reason, LOG_STRING_LIMIT)
                )
            } else {
                format!(
                    "{} — the connection could not stand: {}",
                    failure.headline(),
                    sanitize(reason, LOG_STRING_LIMIT)
                )
            };
            core.emit(OutputEvent::TurnEnd {
                turn_id: None,
                outcome: TurnOutcome::Failed {
                    detail: Some(detail),
                },
            });

            // ★Our stdin is closed **on both branches**★ — without it the loop below spins on `Job::Sweep`, the reader never sees EOF,
            //   [`OutputCore::finish`] never runs and **neither the end transition nor reaping happens** (screen Running, all input refused).
            //   A live peer after a failed round trip exists — ★codex **refusing** `thread/resume`★, **reached in production** (the record
            //   failure branch is ended by the hook — ADR-0292) — and a boot restore stands up one per resumable codex profile. A dead peer: a
            //   no-op; a live one: EOF; so survival certainty is not a premise of closing.
            // ★No kill here★ (ADR-0001's 2 verbs belong to the kill handler): letting go of the write end drives peer exit → reader EOF →
            //   `Drop` of [`ReaderExit`] (`closed`+`Link::Down`) → pump end transition (ADR-0005) → reaper. [unverified on the refusal branch].
            // ★Taking the stdin lock here is safe★: the writer itself takes it, before the loop, not inside `write_all` (`shutdown`'s risk is
            //   **another thread** waiting on a hung writer). Range = **the production call graph**: only [`write_line`] and this line take it
            //   blocking, and only the writer calls [`write_line`]; the count is guarded by
            //   [`tests::the_production_blocking_stdin_locks_are_counted`]. ★The test section holds it blocking outside that graph★
            //   ([`tests::shutdown_completes_even_if_a_write_blocks_on_a_full_pipe`]; harmless as it never calls [`AgentTransport::start`]).
            let had_stdin = sync::lock(&stdin).take().is_some();
            tracing::warn!(
                had_stdin,
                while_recording = *while_recording,
                "the connection could not stand, closing the codex-side stdin — the session is reaped once the peer sees EOF and ends"
            );

            // ★This link kills nothing★ — disposal is **the manager's job**: [`Link::Down`] is visible via [`AgentTransport::link_state`] and the
            //   activation decision runs the existing teardown (ADR-0001 2 verbs), whose kill also releases a hanging reader. Do not bring the
            //   writer's `child.kill()` / `TerminateJobObject` back here — ADR-0199 forbids it.
            // Both activation entrances (`AgentManager::activate_profile` Fresh · `restore_one` non-resume) run that decision
            //   (`spawn_fresh_settled`). ★One ownerless branch, not reached today★: ad-hoc `AgentManager::spawn_agent` (by-cwd) drops the
            //   delivery box; harmless only because it gives codex the **terminal** output format — if that default becomes app-server it opens.
        }
    }

    // ★The loop runs even if the handshake failed★ — reader EOF sets `closed`, [`next_job`] returns `None`, one exit. Stdin is already let go,
    //   so every [`write_line`] fails; ★"the peer never stalls" is unmeasured★ — with unanswered requests ([`OUTBOX_LIMIT`] drops)
    //   [unverified] whether it ends (else the module header's recovery item).
    loop {
        if shutdown.load(Ordering::Acquire) {
            break;
        }
        sweep_deadlines(&state, &pending, &core);
        let Some(job) = next_job(&state, &next_id) else {
            break;
        };
        let steer = match job {
            Job::Sweep => None,
            Job::Steer(steer) => Some(steer),
            Job::Line(line) => {
                if let Err(e) = write_line(&stdin, &line) {
                    tracing::debug!("codex app-server control line write failed: {e}");
                }
                None
            }
            Job::Turn {
                id,
                seq,
                line,
                item,
            } => {
                // ★The waiting entry is registered **before** writing★ — after it, a fast response would arrive first and be dropped as an "unknown id".
                // ★Known remainder★: leaving here leaves the turn just opened as Active. The waiting table having closed means the link
                //   already closed, so nothing reads that state, but "the turn is tidied on every way out" does not hold.
                if !pending.register(id, Waiter::TurnStart { seq }, method::TURN_START) {
                    break;
                }
                match write_line(&stdin, &line) {
                    Err(e) => turn_write_failed(&state, &pending, &core, id, seq, &e),
                    // ADR-0231: the handed-over mark of the carried list item — emitted **after** writing.
                    Ok(()) => {
                        if let Some(item) = &item {
                            publish_hand_over(&core, &state, item);
                        }
                    }
                }
                None
            }
        };
        // ADR-0231
        let Some(steer) = steer else {
            continue;
        };
        let trace = steer.trace(SteerPhase::Steer);
        match issue_steer(&state, &pending, &core, steer, |line| {
            write_line(&stdin, line)
        }) {
            Issued::Closed => break,
            Issued::Written => {
                if let Some(t) = trace {
                    t.write(core.id());
                }
            }
            Issued::Failed => {}
        }
    }
}

// ── Reader side ───────────────────────────────────────────────────────────────────

/// The cap on looking again **until the exit code is caught** after the reader sees EOF.
///
/// ★One `try_wait` right after EOF cannot get the code★ — stdout closing and reaping are not the same moment, so it usually falls to
/// `Ok(None)` and the code is **lost**, and `Exited { code: None }` becomes indistinguishable from "still alive".
/// Grounds: stdout close to reaping is milliseconds; 2 s is a generous margin, and beyond it the peer "has not ended" — carried by the
/// manager's teardown, not this link.
const CHILD_REAP_GRACE: Duration = Duration::from_secs(2);

/// Cuts arbitrary chunks into lines. ★It does not read as UTF-8 before a complete line is settled★ — the pump passes chunks with no
/// character boundaries, and a newline (0x0A) cannot appear as a UTF-8 continuation byte, so a byte-level search is safe.
struct LineSplitter {
    buf: Vec<u8>,
    /// In the middle of dropping the tail of an over-cap line up to the next newline? ★Emptying only the buffer would parse the rest of
    /// that line as a "new line" and make a fake envelope★.
    discarding: bool,
}

impl LineSplitter {
    fn new() -> Self {
        LineSplitter {
            buf: Vec::new(),
            discarding: false,
        }
    }

    fn feed(&mut self, chunk: &[u8], mut on_line: impl FnMut(&[u8])) {
        let mut rest = chunk;
        if self.discarding {
            match rest.iter().position(|&b| b == b'\n') {
                Some(nl) => {
                    self.discarding = false;
                    rest = &rest[nl + 1..];
                }
                None => return,
            }
        }
        while let Some(nl) = rest.iter().position(|&b| b == b'\n') {
            let head = &rest[..nl];
            rest = &rest[nl + 1..];
            if self.buf.is_empty() {
                if head.len() > MAX_LINE_BYTES {
                    tracing::warn!(
                        bytes = head.len(),
                        "codex app-server: line cap exceeded — dropping it"
                    );
                    continue;
                }
                on_line(head);
            } else {
                let mut line = std::mem::take(&mut self.buf);
                if line.len() + head.len() > MAX_LINE_BYTES {
                    tracing::warn!(
                        bytes = line.len() + head.len(),
                        "codex app-server: line cap exceeded — dropping it"
                    );
                    continue;
                }
                line.extend_from_slice(head);
                on_line(&line);
            }
        }
        if !rest.is_empty() {
            if self.buf.len() + rest.len() > MAX_LINE_BYTES {
                tracing::warn!(
                    bytes = self.buf.len() + rest.len(),
                    "codex app-server: line cap exceeded — dropping up to the next newline"
                );
                self.buf = Vec::new();
                self.discarding = true;
            } else {
                self.buf.extend_from_slice(rest);
            }
        }
    }
}

/// Closes the link **however the reader ends** — normal EOF or a panic inside `handle_line`. ★It must be `Drop`★: `contain` wraps the loop
/// from outside, so cleanup at the loop tail is skipped by unwind (a subscriber panic in `core.emit`, a poisoned `OutputCore` mutex), the
/// closed marker never stands and the writer spins forever. It only closes — decoder flush and the end reason stay in the loop.
struct ReaderExit {
    state: SharedState,
    pending: Arc<Pending>,
    /// The agent the transition log of a discarded halt carries ([`Transition::Discarded`]).
    agent: AgentId,
}

impl Drop for ReaderExit {
    /// ★Order is the contract — **write the state first, then wake**★. Waking first lets the writer return through a window the death itself
    /// opened, read `Connecting`, pass [`open_gate`] and **deliver `Ready` to an ended link**. The current order closes it **structurally**
    /// (state write → `send` → writer `recv` → state read chain by happens-before). Do not remove the waking (permanent hang). The waiting
    /// table closes **after** releasing the state lock (ADR-0006).
    // ADR-0203
    // ADR-0201
    fn drop(&mut self) {
        let discarded = {
            let (lock, cv) = &*self.state;
            let mut s = sync::lock(lock);
            // The turn in progress does not know its outcome — ★one log line is all★. As state it would need a third caller to erase it
            //   (ADR-0127 decision 5), and the next incarnation does not take over this link (ADR-0192), so nothing would read it.
            if matches!(s.turn, TurnState::Active { .. }) {
                tracing::warn!(
                    "codex app-server: the stream ended without knowing the turn outcome"
                );
            }
            s.link = Link::Down("the stream ended".to_string());
            s.turn = TurnState::Idle;
            // The held ends are dropped too ([`end_turn_if`]).
            s.early_completions.clear();
            // The halt is discarded too — after closing no turn opens, and waiting items are closed by the core's end synthesis.
            // ADR-0231
            let halted = std::mem::take(&mut s.halted);
            s.closed = true;
            cv.notify_all();
            halted.then_some(Transition::Discarded)
        };
        // Written outside the lock ([`TurnNoteLog`]).
        if let Some(t) = discarded {
            t.write(self.agent);
        }
        // ★Wake all pending RPCs with an error★ — leaving them is a permanent hang with no signal.
        for entry in self.pending.close() {
            match entry.waiter {
                Waiter::Handshake(tx) => {
                    let _ = tx.send(Err(format!("{}: the stream ended", entry.method)));
                }
                // ★The steer item is not touched★ — the stream end ends this incarnation and the core's end synthesis closes remaining items as
                //   `Dropped{AgentEnded}` (one owner of end disposal — TRD §5-5 「deadline and close」).
                Waiter::TurnStart { .. } | Waiter::Steer { .. } | Waiter::Fire => {
                    tracing::debug!(
                        "{}: the stream ended before receiving the answer",
                        entry.method
                    )
                }
            }
        }
    }
}

/// The one line [`Reader::note_turn`] leaves **after releasing the lock**.
///
/// ★Not written under the state lock★: tracing's file sink writes synchronously per event, which would line up
///   [`AgentTransport::send_input`] and the writer behind a disk write. [`Self::Unattributable`] is an **everyday** path (the real
///   `turn/completed` of an abandoned turn arriving late), so one chatty peer would make that many disk writes under the lock.
// ADR-0006
enum TurnNoteLog<'a> {
    ForeignThread(&'a str),
    OversizedTurnId(usize),
    Held(&'a str),
    Unattributable(Option<&'a str>),
}

impl TurnNoteLog<'_> {
    fn write(self) {
        match self {
            TurnNoteLog::ForeignThread(thread) => tracing::warn!(
                thread = %sanitize(thread, LOG_STRING_LIMIT),
                "codex app-server: turn/completed of another thread — blocking the boundary"
            ),
            TurnNoteLog::OversizedTurnId(bytes) => tracing::warn!(
                bytes,
                "codex app-server: the turn id exceeds the cap, so it is not held — blocking the boundary"
            ),
            TurnNoteLog::Held(turn) => tracing::debug!(
                turn = %sanitize(turn, LOG_STRING_LIMIT),
                "codex app-server: an end that came before the response — waiting for the comparison"
            ),
            TurnNoteLog::Unattributable(turn) => tracing::warn!(
                // Left empty, "no id" and "the id was an empty string" would look the same in the log.
                turn = %turn
                    .map(|t| sanitize(t, LOG_STRING_LIMIT))
                    .unwrap_or_else(|| "(no field)".to_string()),
                "codex app-server: an unattributable turn/completed — blocking the boundary"
            ),
        }
    }
}

/// The item ids of approval requests we refused — remembered to attribute that item's live end to our refusal ([`ToolOutcome::Refused`]).
///
/// ★The vendor end alone cannot tell our refusal apart★ (measured fixture `refuse_u2a`): a refused command closes as `failed`, a refused
///   file change as `declined`, shaped like a real failure · the vendor's own denial. So this layer holds the id and rewrites it at the end;
///   the translator stays pure.
/// ★Reader-only and lock-free★: writer ([`Reader::refuse`]) and consumer (the end in [`Reader::handle_line`]) are both the reader thread,
///   and the peer emits the end after receiving our answer. ★Emptied only by that item's end line, not at the turn end★: the end can come
///   after the turn end (fixture `steer_m6` last line); emptying earlier would show it as "error".
// ADR-0241
#[derive(Debug, Default)]
struct RefusedItems {
    ids: VecDeque<String>,
}

impl RefusedItems {
    /// Writes one refused item. If full, the oldest is dropped ([`REFUSED_ITEM_SLOTS`]). ★An already remembered id is not put in again★ —
    /// the peer alternates request → end per approval, but even if that changes, slots do not leak.
    fn remember(&mut self, item_id: &str) {
        if self.ids.iter().any(|r| r == item_id) {
            return;
        }
        if self.ids.len() >= REFUSED_ITEM_SLOTS {
            if let Some(evicted) = self.ids.pop_front() {
                tracing::debug!(
                    cap = REFUSED_ITEM_SLOTS,
                    evicted = %sanitize(&evicted, LOG_STRING_LIMIT),
                    "codex app-server: the memory of refused approval items reached its cap — dropping the oldest"
                );
            }
        }
        self.ids.push_back(item_id.to_string());
    }

    /// Attributes the translation result of one notification line to our refusal. On `item/completed` of a remembered item the memory is
    /// **taken out** (whatever the end status) and the translator's `Failed` · `Declined` results with that id become `Refused`; with
    /// no result emitted (`completed` etc.) only the memory is erased. ★It does not split by vendor end status★ — `failed` or `declined`,
    /// the reason is our refusal.
    fn attribute(&mut self, method_name: &str, params: Option<&Value>, events: &mut [OutputEvent]) {
        if method_name != method::ITEM_COMPLETED || self.ids.is_empty() {
            return;
        }
        let Some(item_id) = params
            .and_then(|p| p.get("item"))
            .and_then(|i| i.get("id"))
            .and_then(Value::as_str)
        else {
            return;
        };
        let Some(at) = self.ids.iter().position(|r| r == item_id) else {
            return;
        };
        self.ids.remove(at);
        for ev in events.iter_mut() {
            if let OutputEvent::ToolResult { id, outcome } = ev {
                if id == item_id && matches!(outcome, ToolOutcome::Failed | ToolOutcome::Declined) {
                    *outcome = ToolOutcome::Refused;
                }
            }
        }
    }
}

/// The id if a refused server request points to a tool item to attribute to our refusal — otherwise `None` (just refusing).
///
/// ★Only command approval and file change approval point★: `itemId` is a tool item started earlier and the peer closes it right after our
///   refusal (fixture `refuse_u2a`). Others (`item/tool/call` · `item/permissions/requestApproval`; unknown whether that item is a tool
///   row we draw · old-name approval has `callId`) do not.
/// ★A command approval with `approvalId` or `networkApprovalContext` does not point (only absent or `null`)★: the former is a
///   sub-command approval (the parent's end is the whole script's result), the latter a network approval whose item end is not emitted
///   (upstream `rust-v0.156.1` — likely; measured approvals had neither).
/// ★The id is filtered by the translator's cap ([`MAX_ID_BYTES`])★ — over it the translator emits no result, so the memory would only take
///   a slot; it also caps held bytes (else up to [`MAX_LINE_BYTES`] per slot × [`REFUSED_ITEM_SLOTS`]).
// ADR-0241
fn refused_item_id<'a>(method_name: &str, params: Option<&'a Value>) -> Option<&'a str> {
    let params = params?;
    match method_name {
        method::ITEM_COMMAND_EXECUTION_REQUEST_APPROVAL => {
            let carried = |key: &str| params.get(key).is_some_and(|v| !v.is_null());
            if carried("approvalId") || carried("networkApprovalContext") {
                return None;
            }
        }
        method::ITEM_FILE_CHANGE_REQUEST_APPROVAL => {}
        _ => return None,
    }
    params
        .get("itemId")
        .and_then(Value::as_str)
        .filter(|id| id.len() <= MAX_ID_BYTES)
}

struct Reader {
    core: Arc<OutputCore>,
    decoder: Option<Box<dyn OutputDecoder>>,
    state: SharedState,
    pending: Arc<Pending>,
    /// The first-turn port — emptied as it is called (once per incarnation). `None` = already called or nowhere to tell.
    // ADR-0226
    first_turn: Option<FirstTurnSink>,
    // ADR-0241
    refused: RefusedItems,
}

impl Reader {
    /// Refuses a server request. ★Unanswered, that agent stops forever★, so even an unknown request is always answered. ★It does not
    /// disguise success★: a success response to an approval request is an automatic approval.
    /// ★A refused approval request pointing to a tool item has its id remembered ([`RefusedItems`] · [`refused_item_id`])★ — only when the
    /// refusal line actually entered the control queue (if closed or overflowing the peer gets no answer and no end).
    // ADR-0241
    fn refuse(&mut self, id: &RequestId, method_name: &str, params: Option<&Value>) {
        let shown = sanitize(method_name, LOG_STRING_LIMIT);
        let line = protocol::error_response_line(
            id,
            METHOD_NOT_FOUND,
            &format!("engram-dashboard does not handle `{shown}`"),
        );
        let dropped = {
            let (lock, cv) = &*self.state;
            let mut s = sync::lock(lock);
            if s.closed {
                return;
            }
            if s.outbox.len() >= OUTBOX_LIMIT {
                true
            } else {
                s.outbox.push_back(line);
                cv.notify_all();
                false
            }
        };
        if dropped {
            // ★No caller to return an error to — the reader made this line itself★. Logged only, the event **is visible nowhere**: the peer waits
            //   forever with no signal on screen. The sibling event (turn/start deadline) shows on screen, so this is raised to that grade.
            tracing::warn!(
                cap = OUTBOX_LIMIT,
                "codex app-server: the control queue is full, cannot send the request refusal"
            );
            self.core.emit(OutputEvent::Error(format!(
                "codex app-server: the control queue is full, could not answer the `{shown}` request — that side keeps waiting for the answer"
            )));
        } else if let Some(item_id) = refused_item_id(method_name, params) {
            self.refused.remember(item_id);
        }
    }

    /// Feeds one turn-ended notification into this state machine and ★decides **whether to raise the translator's boundary from that line**★.
    /// Returns the boundary to raise now (`None` if none); if attributed to our turn's end, its item disposal events are emitted here (before
    /// the boundary — [`State::close_turn_items`]).
    ///
    /// ★Only this layer knows attribution★ (our thread · turn id): without the gate someone else's end closes our live turn and plugs mail
    /// mid-turn. A blocked line is logged as warn.
    /// ★Ends only when both match★: our thread id (if known) and **the turn id we know**.
    /// ★Unknown turn id → nothing ends★, no stall: that turn's `turn/start` has no answer yet and has [`REQUEST_DEADLINE`] ([`end_turn_if`]).
    /// ★An end **before** the response★: its id **and boundary** are held in [`State::early_completions`] and compared where the response
    /// decides our turn's id ([`Reader::resolve`]) — else nothing would end the turn once the waiting entry is removed. ★What is held is that
    /// **id**, never "there was an end"★; the ending decision is only the id match on every path — do not widen it.
    // ADR-0127
    fn note_turn(
        &self,
        method_name: &str,
        params: Option<&Value>,
        boundary: Option<OutputEvent>,
    ) -> Option<OutputEvent> {
        if method_name != TURN_COMPLETED {
            // ★The translator emits no turn boundary outside this name★ — still what was received is returned as is: if the premise breaks, a
            //   visible boundary beats a silent one.
            return boundary;
        }
        let incoming_thread = params
            .and_then(|p| p.get("threadId"))
            .and_then(|v| v.as_str());
        let incoming_turn = params
            .and_then(|p| p.get("turn"))
            .and_then(|t| t.get("id"))
            .and_then(|v| v.as_str());

        let close = TurnClose::of_completion(params);
        let mut drops = Disposal::default();
        let (result, log) = {
            let (lock, cv) = &*self.state;
            let mut s = sync::lock(lock);
            // ★An unknown threadId is not an error★ — measured: a notification arrives before the response that tells that thread's id. While
            //   our id is unknown it is not filtered on that axis.
            let foreign_thread = match (s.thread_id.as_deref(), incoming_thread) {
                (Some(mine), Some(theirs)) if mine != theirs => Some(theirs),
                _ => None,
            };
            if let Some(theirs) = foreign_thread {
                (None, Some(TurnNoteLog::ForeignThread(theirs)))
            } else {
                match (&s.turn, incoming_turn) {
                    (
                        TurnState::Active {
                            seq,
                            turn_id: Some(mine),
                        },
                        Some(theirs),
                    ) if mine == theirs => {
                        let seq = *seq;
                        s.turn = TurnState::Idle;
                        s.early_completions.clear();
                        // ADR-0231
                        drops = s.close_turn_items(seq, close);
                        cv.notify_all();
                        (boundary, None)
                    }
                    // ★An end while the current turn's id is unknown★: it cannot be attributed, so it **neither ends nor raises the boundary**; its id
                    //   and boundary are held and compared when the following `turn/start` response gives our turn's id ([`Reader::resolve`]).
                    //   Without that the turn would have nothing to end it — the response removes the waiting entry and the deadline backstop.
                    (TurnState::Active { turn_id: None, .. }, Some(theirs)) => {
                        if theirs.len() > MAX_TURN_ID_BYTES {
                            (None, Some(TurnNoteLog::OversizedTurnId(theirs.len())))
                        } else if s.early_completions.iter().any(|k| k.turn_id == theirs) {
                            (None, None)
                        } else {
                            if s.early_completions.len() >= EARLY_COMPLETION_SLOTS {
                                s.early_completions.pop_front();
                            }
                            s.early_completions.push_back(EarlyCompletion {
                                turn_id: theirs.to_string(),
                                boundary,
                                close,
                            });
                            (None, Some(TurnNoteLog::Held(theirs)))
                        }
                    }
                    // No open turn, or it differs from the id we know, or the notification has no turn id.
                    _ => (None, Some(TurnNoteLog::Unattributable(incoming_turn))),
                }
            }
        };
        if let Some(entry) = log {
            entry.write();
        }
        // ★Emitted before the boundary, outside the lock★ ([`end_turn_if`] · ADR-0006) — the caller emits the boundary.
        drops.emit(&self.core, &self.state);
        result
    }

    /// Can this notification **be counted as belonging to our open turn** — ★whether to write a progress signal into the fact layer★, not
    /// whether to raise it to the screen (ADR-0113).
    ///
    /// Split from the boundary decision because blocking a body would lose the agent's words; unsplit, one `item/*` line of a turn we never
    /// opened would turn the fact layer on with its end blocked by [`Reader::note_turn`] — **nobody could turn it off** until the 30-minute
    /// fail-open (ADR-0127's mechanism). ★"Does not contradict", not "matches"★: `item/*` before the `turn/start` response is normal
    /// ([`State::early_completions`]); with an open turn and no contradiction it counts, with no open turn it never does.
    /// ★No lock between this decision and the emit (known remainder)★ — one line may count toward the other side (ADR-0006 forbids holding it).
    // ADR-0113
    fn claims_our_turn(&self, params: Option<&Value>) -> bool {
        let (lock, _) = &*self.state;
        let s = sync::lock(lock);
        if let (Some(mine), Some(theirs)) = (
            s.thread_id.as_deref(),
            params
                .and_then(|p| p.get("threadId"))
                .and_then(|v| v.as_str()),
        ) {
            if mine != theirs {
                return false;
            }
        }
        match &s.turn {
            TurnState::Idle => false,
            TurnState::Active { turn_id: None, .. } => true,
            TurnState::Active {
                turn_id: Some(mine),
                ..
            } => match params
                .and_then(|p| p.get("turnId"))
                .and_then(|v| v.as_str())
            {
                Some(theirs) => mine == theirs,
                None => true,
            },
        }
    }

    fn resolve(&self, id: &RequestId, outcome: Result<Value, String>) {
        let entry = match self.pending.take(id) {
            Some(e) => e,
            None => {
                // An id we never issued — a server id-space value, or one already withdrawn by the deadline.
                tracing::debug!(
                    ?id,
                    "codex app-server: response with an unknown id — dropping it"
                );
                return;
            }
        };
        match entry.waiter {
            Waiter::Handshake(tx) => {
                let _ = tx.send(outcome);
            }
            Waiter::TurnStart { seq } => match outcome {
                Ok(v) => match serde_json::from_value::<TurnStartResponse>(v) {
                    // ★The id the response decides is length-capped too★ — it goes as is into `turn/interrupt` params and [`OutputEvent::TurnEnd`], so
                    //   uncapped, one response could make a boundary over the replay ring's single-event cap and empty the ring.
                    //   ★A comparison token, so filtered, not truncated★ ([`MAX_TURN_ID_BYTES`]) — the turn then has no attribution material
                    //   and is closed here **at the same grade** as the error branch below.
                    Ok(r) if r.turn.id.len() > MAX_TURN_ID_BYTES => {
                        tracing::warn!(
                            bytes = r.turn.id.len(),
                            "the turn id of the turn/start response exceeded the cap"
                        );
                        end_turn_if(
                            &self.state,
                            &self.core,
                            seq,
                            format!(
                                "codex app-server: the turn id of the turn/start response exceeded the cap ({MAX_TURN_ID_BYTES}B)"
                            ),
                            TurnClose::Unanswered,
                        );
                    }
                    Ok(r) => {
                        let released = {
                            let (lock, cv) = &*self.state;
                            let mut s = sync::lock(lock);
                            // ★A mismatched marker means the turn this answer opened already ended★ — writing its id into the current turn's slot would make
                            //   a later interrupt aim at the wrong turn.
                            match &mut s.turn {
                                TurnState::Active { seq: cur, turn_id } if *cur == seq => {
                                    let newly = turn_id.is_none();
                                    if newly {
                                        *turn_id = Some(r.turn.id);
                                    }
                                    // ★This response decides our turn's id authoritatively★ — if an end with that id already passed, it ends here and **the held
                                    //   boundary is raised now**. Without it the turn would stay open forever (waiting entry just removed) and `send_input`
                                    //   would report that stall as "the queue is full" — wrong reason.
                                    // ★The ending decision is still only the id match★ — no turn ends merely by being in the held list.
                                    let ours = turn_id.clone();
                                    // ★Writes the turn id onto the item this request carried — only when (marker, stage) match★ (generation masking: a no-op if
                                    //   already removed by echo). A success response is acceptance, not receipt — the item keeps waiting for the echo (TRD §5-5).
                                    // ADR-0231
                                    if let Some(item) = s.turn_start_carrier(seq) {
                                        item.stage = Stage::InFlight {
                                            gen: seq,
                                            turn_id: ours.clone(),
                                        };
                                    }
                                    let hit = match ours.as_deref() {
                                        Some(ours) => s
                                            .early_completions
                                            .iter()
                                            .position(|k| k.turn_id == ours),
                                        None => None,
                                    };
                                    match hit.and_then(|pos| s.early_completions.remove(pos)) {
                                        Some(held) => {
                                            s.turn = TurnState::Idle;
                                            // The rest held in the same window are dropped with this turn ([`end_turn_if`]).
                                            s.early_completions.clear();
                                            // ★An early end is a turn end too★ — undisposed, that turn's `InFlight` stays without an outcome (TRD §5-5 turn-end disposal ②).
                                            // ADR-0231
                                            let drops = s.close_turn_items(seq, held.close);
                                            cv.notify_all();
                                            (drops, held.boundary)
                                        }
                                        // ★The turn id just came★ — user text held while this turn opened (left at a turn end · typed before the response) goes into this turn
                                        //   at once without waiting for a tool end, so text left at the turn end gathers into one turn (ADR-0235).
                                        // ADR-0231
                                        None => {
                                            if newly
                                                && s.pending.iter().any(|p| {
                                                    matches!(p.stage, Stage::Held)
                                                        && p.origin == InputOrigin::User
                                                })
                                            {
                                                s.steer_due = true;
                                                cv.notify_all();
                                            }
                                            (Disposal::default(), None)
                                        }
                                    }
                                }
                                _ => {
                                    tracing::debug!(
                                        "codex app-server: turn/start response of an already ended turn — dropping it"
                                    );
                                    (Disposal::default(), None)
                                }
                            }
                        };
                        // ★Raised after releasing the lock★ (ADR-0006) — subscribers run arbitrary code inside emit. Disposal events come before the
                        //   boundary ([`end_turn_if`]).
                        let (drops, boundary) = released;
                        drops.emit(&self.core, &self.state);
                        if let Some(ev) = boundary {
                            self.core.emit(ev);
                        }
                    }
                    // ★Must be **the same grade** as an error response★: the waiting entry is already removed, so a turn left open has no `turn_id` and
                    //   no entry to expire — `note_turn` can never attribute, the gate does not open and `send_input` reports "the queue is
                    //   full". An undecodable success is as fatal as an explicit error.
                    Err(e) => {
                        let masked = sanitize(&e.to_string(), LOG_STRING_LIMIT);
                        tracing::warn!("turn/start response decode failed: {masked}");
                        end_turn_if(
                            &self.state,
                            &self.core,
                            seq,
                            format!("codex app-server: could not read the turn/start response: {masked}"),
                            TurnClose::Unanswered,
                        );
                    }
                },
                // An error response of `turn/start` is a refusal (`Rejected`) — the disposal returns the carried user text to the held place and the
                //   halt stands ([`State::close_turn_items`]).
                // ADR-0231
                Err(msg) => {
                    let masked = sanitize(&msg, LOG_STRING_LIMIT);
                    tracing::warn!("turn/start failed: {masked}");
                    end_turn_if(
                        &self.state,
                        &self.core,
                        seq,
                        format!("codex app-server: {masked}"),
                        TurnClose::Rejected,
                    );
                }
            },
            // ★The response reaches only that item — it does not end the turn★: a steer did not open the turn and even on an error response it keeps
            //   running at the vendor (`turn/completed` ends it; the error sometimes comes first — M10, 0.1 ms ahead — no order is assumed).
            // ★A success response (and the deadline) does not move the item — receipt is only the echo★: an accepted steer's echo precedes
            //   `turn/completed` (TRD §3-2 · M10), so text without an echo by the turn end was not accepted and may be sent again; a late
            //   response is a no-op by generation ([`State::hold_refused_steer`]).
            // ADR-0231
            Waiter::Steer { id: item, gen } => {
                let Err(msg) = &outcome else {
                    return;
                };
                let (held, trace) = {
                    let (lock, cv) = &*self.state;
                    let mut s = sync::lock(lock);
                    // ADR-0231: the refusal trace line — carries that turn id if the turn is still running.
                    let trace = SteerTrace::enabled().then(|| {
                        let turn = match &s.turn {
                            TurnState::Active { seq, turn_id } if *seq == gen => turn_id.as_deref(),
                            _ => None,
                        };
                        SteerTrace::new(SteerPhase::Refused, turn, &item)
                    });
                    let held = s.hold_refused_steer(&item, gen);
                    cv.notify_all();
                    (held, trace)
                };
                // Written and emitted outside the lock ([`TurnNoteLog`] · ADR-0006).
                tracing::warn!("turn/steer refused: {}", sanitize(msg, LOG_STRING_LIMIT));
                if let Some(t) = trace {
                    t.write(self.core.id());
                }
                match held {
                    None => tracing::debug!(
                        "codex app-server: turn/steer refusal of an item already accepted or disposed by the turn end — dropping it"
                    ),
                    Some(out) => out.emit(&self.core, &self.state),
                }
            }
            Waiter::Fire => {
                if let Err(msg) = outcome {
                    tracing::warn!(
                        "{} failed: {}",
                        entry.method,
                        sanitize(&msg, LOG_STRING_LIMIT)
                    );
                }
            }
        }
    }

    /// For every receipt (`Delivered`) the translator emitted, removes that id from pending regardless of stage ([`State::take_delivered`]).
    /// Returns whether a turn boundary must be newly set after the receipt.
    ///
    /// ★Removed **before** emitting to the screen★ — the text is in the vendor history and must not go out again. ★A late receipt★ (text
    /// returned to the held place) also closes as received (TRD §5-5 「when a receipt comes after the turn end」); removal wakes the writer.
    /// ★A receipt with no open turn sets a new boundary (`true`)★ — only a boundary turns off the frontend waiting indicator (unknown ids
    /// likewise); with an open turn, that turn's end is the boundary.
    // ADR-0231
    fn note_delivered(&self, events: &[OutputEvent]) -> bool {
        let ids: Vec<&str> = events
            .iter()
            .filter_map(|e| match e {
                OutputEvent::QueuedInput(QueuedInputEvent::Delivered { id }) => Some(id.as_str()),
                _ => None,
            })
            .collect();
        if ids.is_empty() {
            return false;
        }
        let tracing_on = SteerTrace::enabled();
        let mut traces = Vec::new();
        let idle = {
            let (lock, cv) = &*self.state;
            let mut s = sync::lock(lock);
            let idle = matches!(s.turn, TurnState::Idle);
            for id in ids {
                let Some(item) = s.take_delivered(id) else {
                    continue;
                };
                // ADR-0231: the receipt trace line — carries the id of the turn that item went into.
                if tracing_on {
                    let turn = match &item.stage {
                        Stage::InFlight { turn_id, .. } => turn_id.as_deref(),
                        Stage::Held => None,
                    };
                    traces.push(SteerTrace::new(SteerPhase::Echo, turn, id));
                }
            }
            cv.notify_all();
            idle
        };
        for t in &traces {
            t.write(self.core.id());
        }
        idle
    }

    /// The tool-end trigger (ADR-0235 — t3code shape): a tool item ([`is_tool_item`]) of our current turn ending (`item/completed`) sets
    /// [`State::steer_due`] and wakes the writer — that tool end hands over all held user text. Returns the tool item's trace lines.
    ///
    /// ★Only lines of **the current turn id** count★ — a late completion of an old turn id (M6) and lines before the turn id came
    /// (`Active{turn_id: None}`; [`Reader::resolve`] hands over there) are no trigger. Parallel tools trigger at every end. ★Called **before**
    /// translation★ — the trigger's arrival is the reaction latency.
    // ADR-0231
    // ADR-0235
    fn note_tool_item(&self, method_name: &str, params: Option<&Value>) -> Option<SteerTrace> {
        let started = method_name == method::ITEM_STARTED;
        let completed = method_name == method::ITEM_COMPLETED;
        if !started && !completed {
            return None;
        }
        let params = params?;
        let item = params.get("item");
        let kind = item
            .and_then(|i| i.get("type"))
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        if !is_tool_item(kind) {
            return None;
        }
        let incoming_turn = params.get("turnId").and_then(|v| v.as_str())?;
        let incoming_thread = params.get("threadId").and_then(|v| v.as_str());
        let item_id = item.and_then(|i| i.get("id")).and_then(|v| v.as_str());

        let (lock, cv) = &*self.state;
        let mut s = sync::lock(lock);
        if let (Some(mine), Some(theirs)) = (s.thread_id.as_deref(), incoming_thread) {
            if mine != theirs {
                return None;
            }
        }
        match &s.turn {
            TurnState::Active {
                turn_id: Some(current),
                ..
            } if current == incoming_turn => {}
            _ => return None,
        }
        // Start · end trace lines of tool items — tools whose id cannot be compared (absent · overflowing) are not written (the trigger still stands).
        let trace = match item_id {
            Some(id) if id.len() <= MAX_TURN_ID_BYTES && SteerTrace::enabled() => {
                let phase = if started {
                    SteerPhase::ToolStart
                } else {
                    SteerPhase::ToolEnd
                };
                let held = completed.then(|| {
                    s.pending
                        .iter()
                        .filter(|p| matches!(p.stage, Stage::Held) && p.origin == InputOrigin::User)
                        .count()
                });
                Some(SteerTrace {
                    held,
                    ..SteerTrace::new(phase, Some(incoming_turn), id)
                })
            }
            _ => None,
        };
        if completed {
            s.steer_due = true;
            cv.notify_all();
        }
        trace
    }

    /// On the peer's **first** echo of a user message item, calls the first-turn port. ★Any live `userMessage` counts★ — with or without
    /// `clientId`, regardless of [`Reader::claims_our_turn`]; only lines **declaring** another thread are excluded; history restore does not
    /// pass here ([`hydrate_history`]). ★Called without any lock, after emitting to the screen★ (the latch commit is a disk write).
    // ADR-0226 · ADR-0231 · ADR-0233: persisted when the id became real (the peer's first user message echo).
    fn witness_first_turn(&mut self, method_name: &str, params: Option<&Value>) {
        if self.first_turn.is_none()
            || (method_name != method::ITEM_STARTED && method_name != method::ITEM_COMPLETED)
        {
            return;
        }
        let Some(params) = params else {
            return;
        };
        let kind = params
            .get("item")
            .and_then(|i| i.get("type"))
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        if !is_user_message(kind) {
            return;
        }
        let incoming_thread = params.get("threadId").and_then(|v| v.as_str());
        let foreign = {
            let (lock, _) = &*self.state;
            let s = sync::lock(lock);
            matches!(
                (s.thread_id.as_deref(), incoming_thread),
                (Some(mine), Some(theirs)) if mine != theirs
            )
        };
        if foreign {
            return;
        }
        if let Some(sink) = self.first_turn.take() {
            // ADR-0292 decision 3: the latch commit changes the profile table several agents write — even inside the pump's containment, the marking is turned off.
            uncontained(|| sink());
        }
    }

    fn handle_line(&mut self, line: &[u8]) {
        let text = match std::str::from_utf8(line) {
            Ok(t) => t,
            Err(_) => {
                tracing::debug!(
                    bytes = line.len(),
                    "codex app-server: a line that is not UTF-8 — dropping it"
                );
                return;
            }
        };
        if text.trim().is_empty() {
            return;
        }
        match protocol::classify(text) {
            Ok(Inbound::Request { id, method, params }) => {
                self.refuse(&id, &method, params.as_ref())
            }
            Ok(Inbound::Notification { method, params }) => {
                // ADR-0231
                // Trace lines are written right after releasing the lock — so they come before the steer line (writer) the trigger released.
                if let Some(t) = self.note_tool_item(&method, params.as_ref()) {
                    t.write(self.core.id());
                }
                // ★The translator gets **the original line bytes** as is★ — a second entrance would move that side's line reassembly · cap · masking
                //   discipline out of the delivery path (`decoder.rs` header). It splits lines by newline, so the terminator is given too.
                let mut events = match self.decoder.as_mut() {
                    Some(dec) => {
                        let mut events = dec.decode(line);
                        events.extend(dec.decode(b"\n"));
                        // Usage observation is not an event — handed straight to the status sink regardless of the attribution gate.
                        self.core.report_usage(dec.take_usage());
                        events
                    }
                    None => Vec::new(),
                };
                // ADR-0241: the translator does not know our refusals — the end of a refused item is rewritten here before emit (no lock).
                self.refused
                    .attribute(&method, params.as_ref(), &mut events);
                // ★Only the turn boundary is pulled out and passes the **attribution gate** — the rest flows as is★: the translator cannot answer
                //   "is this outcome our turn's" ([`TURN_COMPLETED`]).
                let boundary = events
                    .iter()
                    .position(|e| matches!(e, OutputEvent::TurnEnd { .. }))
                    .map(|i| events.remove(i));
                let boundary_owed = self.note_delivered(&events);
                // ★The body always goes to the screen, but whether the line counts as grounds for "mid-turn" is split★ ([`Reader::claims_our_turn`]);
                //   unsplit, an unattributed line would turn only the fact layer on and its end would be blocked by the gate below.
                // A line yielding only one boundary (most `turn/completed`) does not need this decision — the lock is saved.
                let ours = !events.is_empty() && self.claims_our_turn(params.as_ref());
                for ev in events {
                    // ★End vocabulary is not sent through that door★ — it also skips the doorbell (`StatusSink::turn_ended`), so consumers waiting for the
                    //   end would not wake. By the translator contract one line emits one boundary, already pulled out above; guarding here is
                    //   cheaper than silently losing it if that breaks.
                    if ours || matches!(ev, OutputEvent::TurnEnd { .. }) {
                        self.core.emit(ev);
                    } else {
                        self.core.emit_without_turn_observation(ev);
                    }
                }
                // ★The boundary after a receipt outside the open turn ([`Reader::note_delivered`]) goes through the observation-free door★: the turn
                //   table is already idle (the receipt also went without observation), so no consumer needs waking, and the ordinary door would
                //   let the classifier fold it into a clean end and erase the preceding error end (`last_end_failed`) — mail that should stay
                //   halted would flow. If a new turn opened in between and the receipt counted as its progress (`ours`), that turn's end is the boundary.
                // ADR-0231
                if boundary_owed && !ours {
                    self.core
                        .emit_without_turn_observation(OutputEvent::TurnEnd {
                            turn_id: None,
                            outcome: TurnOutcome::Completed,
                        });
                }
                if let Some(ev) = self.note_turn(&method, params.as_ref(), boundary) {
                    self.core.emit(ev);
                }
                // ADR-0226: after emitting and tracing, outside the lock.
                self.witness_first_turn(&method, params.as_ref());
            }
            Ok(Inbound::Response { id, result }) => self.resolve(&id, Ok(result)),
            Ok(Inbound::Error { id, error }) => {
                // ★Mask **here**, not at the boundary★ — this string goes to the log, the screen and back to the caller; filtering at every outgoing door
                //   means one gets forgotten.
                let msg = format!(
                    "[{}] {}",
                    error.code,
                    sanitize(&error.message, LOG_STRING_LIMIT)
                );
                self.resolve(&id, Err(msg))
            }
            Err(e) => {
                // ★A decode failure is not made fatal★ — cutting the stream for one line would kill the agent.
                tracing::debug!(
                    "codex app-server: could not read the envelope ({e}) — {}",
                    sanitize(text, LOG_STRING_LIMIT)
                );
            }
        }
    }
}

fn reader_loop(
    stdout: ChildStdout,
    shutdown: Arc<AtomicBool>,
    mut reader: Reader,
    child: Arc<Mutex<Child>>,
) -> TerminalReason {
    // ★This guard stands at the very top★ — wherever unwind comes from below, `Drop` runs.
    let _exit = ReaderExit {
        state: reader.state.clone(),
        pending: reader.pending.clone(),
        agent: reader.core.id(),
    };

    let mut source = stdout;
    let mut buf = [0u8; READ_BUF_BYTES];
    let mut splitter = LineSplitter::new();

    loop {
        let n = match source.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        if shutdown.load(Ordering::Relaxed) {
            break;
        }
        let mut lines: Vec<Vec<u8>> = Vec::new();
        splitter.feed(&buf[..n], |line| lines.push(line.to_vec()));
        for line in lines {
            reader.handle_line(&line);
        }
    }

    // ★Closing is carried by `ReaderExit` above — not repeated here★ (unwind must pass the same place).
    // ★No flush on the kill path★ — that tail is a fragment cut because we killed the process.
    if !shutdown.load(Ordering::Acquire) {
        if let Some(dec) = reader.decoder.as_mut() {
            for ev in dec.flush() {
                reader.core.emit(ev);
            }
            reader.core.report_usage(dec.take_usage());
        }
    }

    // ★One `try_wait` right after EOF cannot get the exit code★ — stdout closing and reaping are not the same moment, so it usually falls
    //   to `Ok(None)` and the code is **lost**, and `Exited { code: None }` becomes indistinguishable from "still alive" for tests.
    // ★So it retries, bounded★ — the lock is released each attempt (sleeping under it would block `shutdown`'s kill that long).
    // ★The kill path runs no round of this loop★ — `shutdown()` already did `wait()`, so the first attempt gives the value.
    // ★`None` can still remain, honestly★: the peer closed only stdout and did not end — carried by [`wait_for_close_or_tear_down`].
    let code = {
        let deadline = Instant::now() + CHILD_REAP_GRACE;
        loop {
            let reaped = {
                let mut c = sync::lock(&child);
                match c.try_wait() {
                    Ok(Some(status)) => Some(status.code()),
                    _ => None,
                }
            };
            if let Some(code) = reaped {
                break code;
            }
            if Instant::now() >= deadline {
                break None;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    };
    if shutdown.load(Ordering::Acquire) {
        TerminalReason::Killed
    } else {
        TerminalReason::Exited { code }
    }
}

/// The common receipt of `send_input` · `send_turn`; `Err` conditions: [`AgentTransport::send_input`]. `Ok(true)` = this item lifted the
/// halt after error ([`State::push_pending`] — the caller writes that log outside the lock).
// ADR-0231
fn accept_input(
    state: &SharedState,
    id: Option<String>,
    body: Vec<u8>,
    origin: InputOrigin,
) -> Result<bool, PtyError> {
    let (lock, cv) = &**state;
    let mut s = sync::lock(lock);
    if s.closed {
        return Err(PtyError::WriteFailed(
            "codex app-server: the link closed".into(),
        ));
    }
    if let Link::Down(reason) = &s.link {
        return Err(PtyError::WriteFailed(format!(
            "codex app-server connection ended: {}",
            sanitize(reason, LOG_STRING_LIMIT)
        )));
    }
    if s.pending.len() >= INPUT_QUEUE_LIMIT {
        return Err(PtyError::WriteFailed(format!(
            "codex app-server: pending input reached the cap ({INPUT_QUEUE_LIMIT})"
        )));
    }
    let lifted = s.push_pending(id, body, origin);
    cv.notify_all();
    Ok(lifted)
}

/// The body of [`AgentTransport::withdraw`] (contract there). `core` = where to emit (`None` before `start()` — then no list item has
/// finished announcing).
// ADR-0231
fn withdraw_item(state: &SharedState, core: Option<&OutputCore>, id: &str) -> Withdraw {
    let (answer, ops) = {
        let (lock, cv) = &**state;
        let mut s = sync::lock(lock);
        let Some(pos) = s.pending.iter().position(|p| p.id.as_deref() == Some(id)) else {
            return Withdraw::NotHeld;
        };
        let item = &mut s.pending[pos];
        if !item.listed || item.death.is_some() {
            return Withdraw::NotHeld;
        }
        match item.stage {
            Stage::Held => {
                let dropped = s.drop_listed(pos, DropCause::Withdrawn);
                cv.notify_all();
                (Withdraw::Withdrawn, dropped.into_iter().collect())
            }
            // ★Already handed over — cannot be withdrawn · the row stays in the list★ (the frontend draws it as a handed-over row without ✕): the
            //   vendor decides the outcome (echo = received · steer refusal or turn end without echo = withdrawn), and the cancellation
            //   receipt and "could not remove it from the vendor queue" follow immediately (codex has no verb to withdraw handed-over text).
            //   ★An overlapping second ✕ (window + LLM — not through the input lock) is `TooLate` without an event★ — the registry's
            //   `Cancelling`+`CancelRequested` no-op row is the second defence.
            // ADR-0231
            Stage::InFlight { .. } if item.cancel_asked => (Withdraw::TooLate, Vec::new()),
            Stage::InFlight { .. } => {
                item.cancel_asked = true;
                let id = id.to_owned();
                (
                    Withdraw::TooLate,
                    vec![
                        QueuedInputEvent::CancelRequested { id: id.clone() },
                        QueuedInputEvent::CancelAnswered { id, removed: false },
                    ],
                )
            }
        }
    };
    // ★Emitted after releasing the lock★ (ADR-0006).
    if let Some(core) = core {
        for op in ops {
            core.emit(OutputEvent::QueuedInput(op));
        }
    }
    answer
}

// ── AgentTransport ────────────────────────────────────────────────────────────

impl AgentTransport for CodexAppServerTransport {
    /// If stdout was already taken (a repeated call), it does nothing (idempotence defence).
    fn start(&self, core: Arc<OutputCore>) {
        let agent_id = core.id();

        let stdout = match sync::lock(&self.stdout).take() {
            Some(s) => s,
            None => return,
        };
        // ★Filled before the writer★ — the input thread announces after the writer's floor decision, so it must stand first.
        let _ = self.core.set(core.clone());

        // ── stderr drain ──
        // The peer's diagnostic text comes on this stream (measured 0.154.0 — errors went here, not stdout), the only evidence of an
        //   activation failure.
        if let Some(stderr) = sync::lock(&self.stderr).take() {
            drain_stderr(stderr, &core, "engram-codex-stderr", "codex");
        }

        // ── Writer ──
        // ★Nobody joins it★ — `core` holds one pump handle (widening it is a core change) and waiting inside `shutdown()` is a contract
        //   violation. Kill breaks the pipe, releasing the blocked write; the thread sees the closed marker and ends on its own.
        if let Some(params) = sync::lock(&self.open_params).take() {
            let stdin = self.stdin.clone();
            let state = self.state.clone();
            let pending = self.pending.clone();
            let next_id = self.next_id.clone();
            let shutdown = self.shutdown.clone();
            let writer_core = core.clone();
            let announcer = self.announcer.clone();
            let sink = self.sid_sink.clone();
            let link = sync::lock(&self.link_sink).take();
            let spawn_result = std::thread::Builder::new()
                .name("engram-codex-writer".into())
                .spawn(move || {
                    writer_loop(
                        stdin,
                        state,
                        pending,
                        next_id,
                        shutdown,
                        writer_core,
                        params,
                        sink,
                        link,
                        announcer,
                    )
                });
            match spawn_result {
                Ok(handle) => {
                    *sync::lock(&self.writer_handle) = Some(handle);
                }
                Err(e) => {
                    // Without a writer neither handshake nor input goes out — not left silent.
                    tracing::warn!(agent = %agent_id, "codex writer thread failed to start: {e}");
                    let (lock, cv) = &*self.state;
                    let mut s = sync::lock(lock);
                    s.link = Link::Down(format!("writer thread failed to start: {e}"));
                    cv.notify_all();
                }
            }
        }

        // ── Reader (pump) ──
        let (done_tx, done_rx) = mpsc::channel();
        let reader = Reader {
            core: core.clone(),
            decoder: sync::lock(&self.decoder).take(),
            state: self.state.clone(),
            pending: self.pending.clone(),
            first_turn: self.first_turn_sink.clone(),
            refused: RefusedItems::default(),
        };
        let pump_core = core.clone();
        let child = self.child.clone();
        let shutdown = self.shutdown.clone();
        let handle = std::thread::spawn(move || {
            // ADR-0292: finish is outside the marking — do not move it into the closure (same spot in `transport/pty.rs`).
            let normal_reason = contain(std::panic::AssertUnwindSafe(|| {
                reader_loop(stdout, shutdown, reader, child)
            }));
            pump_core.finish(resolve_pump_reason(normal_reason));
            let _ = done_tx.send(());
        });

        core.attach_pump(handle, done_rx);
    }

    /// The bytes are **a complete user turn body** — the envelope is built by this link.
    ///
    /// ★`Ok` = all received with order settled, `Err` = not received; no short `Ok`★. `Err` is decided only **before** "we will take it":
    /// ① queue cap exceeded ② connection already ended. ★Known limit★: a write that fails after accepting has no way back here; the
    /// failure shows as a **failure turn boundary** on the output ([`end_turn_if`]; [`writer_loop`] for the dropped-queue branch).
    fn send_input(&self, input: InputEvent) -> Result<(), PtyError> {
        let InputEvent::Raw(bytes) = input;
        // From mail, so it does not lift the halt.
        accept_input(&self.state, None, bytes, InputOrigin::Mail).map(|_| ())
    }

    /// The same receipt as [`AgentTransport::send_input`] — carries identifier and source on the item, and after the floor decision finishes
    /// the announcement (synthetic bubble · `Queued`) before returning ([`announce`]).
    // ADR-0231
    fn send_turn(&self, turn: TurnInput) -> Result<(), PtyError> {
        let lifted = accept_input(&self.state, Some(turn.id), turn.body, turn.origin)?;
        if let Some(core) = self.core.get() {
            // ADR-0231: written outside the lock ([`TurnNoteLog`]).
            if lifted {
                Transition::HaltLifted.write(core.id());
            }
            announce(core, &self.state, &self.announcer);
        }
        Ok(())
    }

    /// ✕ — a held list item is withdrawn (`Dropped{Withdrawn}` · no vendor call); a handed-over list item is `TooLate` (`CancelRequested` then
    /// `CancelAnswered{removed:false}`; withdrawn later if its turn ends without an echo); outside the list (Direct · mail · no identifier ·
    /// below the floor) and unknown ids are `NotHeld`.
    ///
    /// ★Ordered under the same state lock as the handover★. ★A pre-announce item gets only a death mark and `Withdrawn` is answered★
    /// ([`State::drop_listed`]); a death mark already there = the first ✕ removed it: `NotHeld`.
    // ADR-0231
    fn withdraw(&self, id: &str) -> Withdraw {
        withdraw_item(&self.state, self.core.get().map(|c| &**c), id)
    }

    fn resize(&self, _cols: u16, _rows: u16) -> Result<(), PtyError> {
        Err(PtyError::Unsupported(
            "CodexAppServerTransport::resize (a pipe has no terminal size)".into(),
        ))
    }

    /// Interrupts one turn in progress with `turn/interrupt` (≠kill — the process stays alive).
    ///
    /// ★Unknown turn id → `Err`★: the peer requires that field and an envelope without it gets no answer at all (measured 0.154.0).
    fn interrupt(&self) -> Result<(), PtyError> {
        let (lock, cv) = &*self.state;
        let mut s = sync::lock(lock);
        let (thread_id, turn_id) = match (&s.thread_id, &s.turn) {
            (
                Some(t),
                TurnState::Active {
                    turn_id: Some(turn),
                    ..
                },
            ) => (t.clone(), turn.clone()),
            _ => {
                return Err(PtyError::Unsupported(
                    "CodexAppServerTransport::interrupt (no turn to interrupt)".into(),
                ))
            }
        };
        if s.outbox.len() >= OUTBOX_LIMIT {
            return Err(PtyError::WriteFailed(format!(
                "codex app-server: the control queue reached its cap ({OUTBOX_LIMIT})"
            )));
        }
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let line = protocol::request_line(
            &RequestId::Num(id),
            method::TURN_INTERRUPT,
            &TurnInterruptParams { thread_id, turn_id },
        )
        .map_err(|e| PtyError::WriteFailed(format!("turn/interrupt serialization failed: {e}")))?;
        if !self
            .pending
            .register(id, Waiter::Fire, method::TURN_INTERRUPT)
        {
            return Err(PtyError::WriteFailed(
                "codex app-server: the link closed".into(),
            ));
        }
        s.outbox.push_back(line);
        cv.notify_all();
        Ok(())
    }

    /// The app-server version of the ADR-0001 2 verbs. ★Nothing is waited for inside★ (`core.join_pump` waits).
    ///
    /// ★Order invariant — stdin close must never come before kill★: the writer holds the stdin lock across a blocking `write_all`, so with a
    /// peer not reading stdin, taking `stdin.lock()` first never reaches kill and `join_pump` hangs. Killing first breaks the pipe and
    /// releases the lock. (Closing stdin alone also makes the peer exit — 41–51ms — but that does not change the order.)
    // ADR-0001
    fn shutdown(&self) {
        self.shutdown.store(true, Ordering::Release);

        // ★Only the state lock is taken here★ — the thread in a blocking write does not hold it (released before writing), so this cannot
        //   hang. The queue not yet sent disappears here.
        {
            let (lock, cv) = &*self.state;
            let mut s = sync::lock(lock);
            s.closed = true;
            cv.notify_all();
        }

        {
            let mut child = sync::lock(&self.child);
            let _ = child.kill();
            let _ = child.wait();
        }

        // Grandchildren (codex under the cmd shim, MCP children launched at thread/start) end together too; a no-op on an OS with no group.
        let _ = self.group.terminate(1);

        // If try_lock fails (write_all not yet released), skip — the OS reclaims an uncleaned `ChildStdin` on drop. ★No blocking lock★.
        if let Ok(mut guard) = self.stdin.try_lock() {
            let _ = guard.take();
        }

        // Pending requests are woken here too — so none remain even when the reader ends without seeing EOF.
        for entry in self.pending.close() {
            if let Waiter::Handshake(tx) = entry.waiter {
                let _ = tx.send(Err(format!("{}: the link closed", entry.method)));
            }
        }
    }

    fn capabilities(&self) -> TransportCaps {
        TransportCaps {
            input: InputCaps {
                // ★`raw` is false because this channel does not carry keystrokes★ — received bytes are complete message bodies; streaming them one
                //   character at a time would open a turn per character.
                raw: false,
                message: true,
                attachment: false,
            },
            output: OutputCaps {
                terminal_bytes: false,
                structured: self.structured,
                markdown: false,
                tool_events: false,
                usage: false,
            },
            control: ControlCaps {
                resize: false,
                interrupt: true,
                cancel: false,
                graceful_shutdown: false,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::protocol::{ThreadItemEntry, ThreadStartParams};
    use super::*;
    use crate::output_core::TurnWiring;
    use crate::turn::TurnObservations;
    use crate::types::{
        AgentId, AgentStatus, DeliveryAckState, OutputFrame, OutputPayload, OutputSink, SinkError,
        SinkId, StatusSink,
    };

    // ── Harness ──────────────────────────────────────────────────────────────

    struct NoopStatus;
    impl StatusSink for NoopStatus {
        fn status_changed(&self, _id: AgentId, _status: AgentStatus, _epoch: u32) {}
        fn agent_list_updated(&self, _agents: Vec<crate::types::AgentInfo>) {}
    }

    /// Collects emitted events as is — `snapshot()` returns only `TerminalBytes`, so structured events are visible only by subscribing.
    struct EventSink {
        id: SinkId,
        seen: Arc<Mutex<Vec<OutputEvent>>>,
    }
    impl OutputSink for EventSink {
        fn send(&self, frame: OutputFrame<'_>) -> Result<(), SinkError> {
            if let OutputPayload::Event(e) = frame.payload {
                self.seen.lock().unwrap().push(e.clone());
            }
            Ok(())
        }
        fn sink_id(&self) -> SinkId {
            self.id
        }
    }

    /// A subscriber that panics inside `core.emit` — one of the two unwind paths (the other is `OutputCore` mutex poison). `emit` does not
    /// catch a sink panic.
    struct PanickingSink(SinkId);
    impl OutputSink for PanickingSink {
        fn send(&self, _frame: OutputFrame<'_>) -> Result<(), SinkError> {
            panic!("subscriber blew up inside emit");
        }
        fn sink_id(&self) -> SinkId {
            self.0
        }
    }

    /// On one notification line, emits an event — passing through the subscriber above, so the panic unwinds the reader **inside the
    /// read loop**. ★It must be `decode`, not flush★: the leaking spot is the read loop (the old tail-of-loop close ran **before** flush).
    struct EmittingDecoder;
    impl OutputDecoder for EmittingDecoder {
        fn decode(&mut self, _chunk: &[u8]) -> Vec<OutputEvent> {
            vec![OutputEvent::Error("mid-stream".into())]
        }
        fn flush(&mut self) -> Vec<OutputEvent> {
            Vec::new()
        }
    }

    /// A probe that streams one notification line to stdout. ★Not built with `echo`★ — the shell changes the quotes and the envelope is
    /// not classified as a notification.
    #[cfg(windows)]
    struct NotificationFile(std::path::PathBuf);

    #[cfg(windows)]
    impl NotificationFile {
        fn new(tag: &str) -> Self {
            let path = std::env::temp_dir()
                .join(format!("engram-codex-{tag}-{}.ndjson", std::process::id()));
            std::fs::write(&path, "{\"method\":\"turn/completed\",\"params\":{}}\n")
                .expect("temp write");
            NotificationFile(path)
        }
        fn args(&self) -> Vec<String> {
            vec![
                "/c".to_string(),
                "type".to_string(),
                self.0.to_string_lossy().into_owned(),
            ]
        }
    }

    #[cfg(windows)]
    impl Drop for NotificationFile {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    #[cfg(windows)]
    fn await_writer_end(t: &CodexAppServerTransport, why: &str) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let finished = t
                .writer_handle
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .as_ref()
                .map(|h| h.is_finished())
                .unwrap_or(false);
            if finished {
                return;
            }
            assert!(Instant::now() < deadline, "{why}");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn core_with_sink() -> (Arc<OutputCore>, Arc<Mutex<Vec<OutputEvent>>>) {
        let core = Arc::new(OutputCore::new(
            AgentId::new_v4(),
            1,
            Arc::new(NoopStatus),
            TurnWiring::detached(),
        ));
        let seen = Arc::new(Mutex::new(Vec::new()));
        core.subscribe(Arc::new(EventSink {
            id: SinkId::new_v4(),
            seen: seen.clone(),
        }));
        (core, seen)
    }

    fn shared() -> SharedState {
        Arc::new((Mutex::new(State::new()), Condvar::new()))
    }

    fn announcer() -> Arc<Announcer> {
        Arc::new(Announcer {
            order: Mutex::new(()),
            ack: Arc::new(DeliveryAck::new()),
        })
    }

    fn with_state<R>(state: &SharedState, f: impl FnOnce(&mut State) -> R) -> R {
        let mut g = state.0.lock().unwrap_or_else(|p| p.into_inner());
        f(&mut g)
    }

    impl State {
        fn push_legacy(&mut self, body: &[u8]) {
            self.push_pending(None, body.to_vec(), InputOrigin::Mail);
        }
    }

    /// Opens one turn with marker `seq`. ★The test bench picks the marker directly★ — which turn a late signal belongs to is told only by
    /// that marker.
    fn activate(state: &SharedState, seq: u64, turn_id: Option<&str>) {
        with_state(state, |s| {
            s.turn = TurnState::Active {
                seq,
                turn_id: turn_id.map(|t| t.to_string()),
            };
            s.next_turn_seq = seq + 1;
        });
    }

    fn turn_seq(state: &SharedState) -> Option<u64> {
        with_state(state, |s| match s.turn {
            TurnState::Active { seq, .. } => Some(seq),
            TurnState::Idle => None,
        })
    }

    fn turn_id_of(state: &SharedState) -> Option<String> {
        with_state(state, |s| match &s.turn {
            TurnState::Active { turn_id, .. } => turn_id.clone(),
            TurnState::Idle => None,
        })
    }

    fn make_ready(state: &SharedState, thread_id: &str) {
        with_state(state, |s| {
            s.link = Link::Ready;
            s.thread_id = Some(thread_id.to_string());
        });
    }

    struct RecordingDecoder {
        chunks: Arc<Mutex<Vec<u8>>>,
    }
    impl OutputDecoder for RecordingDecoder {
        fn decode(&mut self, chunk: &[u8]) -> Vec<OutputEvent> {
            self.chunks.lock().unwrap().extend_from_slice(chunk);
            Vec::new()
        }
        fn flush(&mut self) -> Vec<OutputEvent> {
            Vec::new()
        }
    }

    struct Harness {
        reader: Reader,
        state: SharedState,
        pending: Arc<Pending>,
        next_id: Arc<AtomicI64>,
        seen: Arc<Mutex<Vec<OutputEvent>>>,
        decoded: Arc<Mutex<Vec<u8>>>,
    }

    fn harness() -> Harness {
        let (core, seen) = core_with_sink();
        let state = shared();
        let pending = Arc::new(Pending::default());
        let decoded = Arc::new(Mutex::new(Vec::new()));
        let reader = Reader {
            core,
            decoder: Some(Box::new(RecordingDecoder {
                chunks: decoded.clone(),
            })),
            state: state.clone(),
            pending: pending.clone(),
            first_turn: None,
            refused: RefusedItems::default(),
        };
        Harness {
            reader,
            state,
            pending,
            next_id: Arc::new(AtomicI64::new(0)),
            seen,
            decoded,
        }
    }

    /// The bench with the real translator. ★Attribution-gate tests must use this one★ — [`RecordingDecoder`] emits no events, so "blocked
    /// the boundary" would pass because there was none to begin with.
    fn harness_with_real_decoder() -> Harness {
        let mut h = harness();
        h.reader.decoder = Some(Box::new(super::super::decoder::CodexAppServerDecoder::new()));
        h
    }

    /// The bench with the real translator **and** the real fact layer — measures in one place that "the screen boundary" and "is this
    /// incarnation mid-turn" get the same decision. The incarnation marker is fixed at 1.
    fn harness_with_facts() -> (Harness, Arc<TurnObservations>, AgentId) {
        let id = AgentId::new_v4();
        let turns = Arc::new(TurnObservations::new());
        let core = Arc::new(OutputCore::new(
            id,
            1,
            Arc::new(NoopStatus),
            TurnWiring::new(turns.clone(), super::super::classify_turn),
        ));
        turns.register(id, 1);
        let seen = Arc::new(Mutex::new(Vec::new()));
        core.subscribe(Arc::new(EventSink {
            id: SinkId::new_v4(),
            seen: seen.clone(),
        }));
        let state = shared();
        let pending = Arc::new(Pending::default());
        let decoded = Arc::new(Mutex::new(Vec::new()));
        let reader = Reader {
            core,
            decoder: Some(Box::new(super::super::decoder::CodexAppServerDecoder::new())),
            state: state.clone(),
            pending: pending.clone(),
            first_turn: None,
            refused: RefusedItems::default(),
        };
        (
            Harness {
                reader,
                state,
                pending,
                next_id: Arc::new(AtomicI64::new(0)),
                seen,
                decoded,
            },
            turns,
            id,
        )
    }

    fn boundaries(seen: &Arc<Mutex<Vec<OutputEvent>>>) -> Vec<(Option<String>, TurnOutcome)> {
        seen.lock()
            .unwrap()
            .iter()
            .filter_map(|e| match e {
                OutputEvent::TurnEnd { turn_id, outcome } => {
                    Some((turn_id.clone(), outcome.clone()))
                }
                _ => None,
            })
            .collect()
    }

    fn failure_boundaries(seen: &Arc<Mutex<Vec<OutputEvent>>>) -> Vec<String> {
        boundaries(seen)
            .into_iter()
            .filter_map(|(_, outcome)| match outcome {
                TurnOutcome::Failed { detail } => Some(detail.unwrap_or_default()),
                _ => None,
            })
            .collect()
    }

    fn completed_line(thread_id: &str, turn_id: &str, status: &str) -> String {
        serde_json::json!({
            "method": "turn/completed",
            "params": {"threadId": thread_id,
                       "turn": {"id": turn_id, "items": [], "status": status}},
        })
        .to_string()
    }

    /// One notification line **in the middle** of a turn — a progress signal, not a boundary.
    fn progress_line(thread_id: &str, turn_id: &str) -> String {
        serde_json::json!({
            "method": "item/agentMessage/delta",
            "params": {"threadId": thread_id, "turnId": turn_id, "itemId": "i-1", "delta": "hi"},
        })
        .to_string()
    }

    // ── Carrying usage observations (`take_usage` → `StatusSink::usage_observed`) ──

    struct UsageStatus(Arc<Mutex<Vec<crate::usage::UsageObservation>>>);
    impl StatusSink for UsageStatus {
        fn status_changed(&self, _id: AgentId, _status: AgentStatus, _epoch: u32) {}
        fn agent_list_updated(&self, _agents: Vec<crate::types::AgentInfo>) {}
        fn usage_observed(&self, obs: crate::usage::UsageObservation) {
            self.0.lock().unwrap().push(obs);
        }
    }

    type SeenUsage = Arc<Mutex<Vec<crate::usage::UsageObservation>>>;

    fn usage_reader(
        decoder: Box<dyn OutputDecoder>,
    ) -> (Reader, Arc<Mutex<Vec<OutputEvent>>>, SeenUsage) {
        let observed: SeenUsage = Arc::new(Mutex::new(Vec::new()));
        let core = Arc::new(OutputCore::new(
            AgentId::new_v4(),
            1,
            Arc::new(UsageStatus(observed.clone())),
            TurnWiring::detached(),
        ));
        let seen = Arc::new(Mutex::new(Vec::new()));
        core.subscribe(Arc::new(EventSink {
            id: SinkId::new_v4(),
            seen: seen.clone(),
        }));
        let reader = Reader {
            core,
            decoder: Some(decoder),
            state: shared(),
            pending: Arc::new(Pending::default()),
            first_turn: None,
            refused: RefusedItems::default(),
        };
        (reader, seen, observed)
    }

    fn rate_limits_line() -> String {
        serde_json::json!({
            "method": "account/rateLimits/updated",
            "params": {"rateLimits": {"limitId": "codex",
                "primary": {"usedPercent": 37, "windowDurationMins": 300, "resetsAt": 1_790_424_706i64}}},
        })
        .to_string()
    }

    /// ★The observation passes the read loop and reaches the status sink★ — if the translator only gathered it and nobody emptied it, it
    /// would vanish without an error. Nothing goes to the screen.
    #[test]
    fn a_rate_limits_update_reaches_the_status_sink_not_the_screen() {
        let (mut reader, seen, observed) = usage_reader(Box::new(CodexAppServerDecoder::new()));
        reader.handle_line(rate_limits_line().as_bytes());
        let observed = observed.lock().unwrap();
        assert_eq!(observed.len(), 1);
        assert_eq!(observed[0].five_hour.and_then(|w| w.used_pct), Some(37.0));
        assert!(
            seen.lock().unwrap().is_empty(),
            "usage leaked into a screen event"
        );
    }

    /// A translator blocked by the wrapper (non-default profile) hands nothing to the sink — that decision is measured by this folder's `usage` tests.
    #[test]
    fn a_gated_decoder_reports_no_usage() {
        let gated = crate::usage::UsageGate::blocking(Box::new(CodexAppServerDecoder::new()));
        let (mut reader, seen, observed) = usage_reader(Box::new(gated));
        reader.handle_line(rate_limits_line().as_bytes());
        assert!(observed.lock().unwrap().is_empty());
        assert!(seen.lock().unwrap().is_empty());
    }

    /// The bench with table + codex classifier + real translator — ★only for items that measure the fact layer★. The default [`harness`] is
    /// `TurnWiring::detached()` with a silent table, so "did not turn mid-turn on" would pass because the table is off.
    struct FactHarness {
        reader: Reader,
        state: SharedState,
        table: Arc<TurnObservations>,
        id: AgentId,
        seen: Arc<Mutex<Vec<OutputEvent>>>,
    }

    fn fact_harness() -> FactHarness {
        let id = AgentId::new_v4();
        let table = Arc::new(TurnObservations::new());
        table.register(id, 1);
        let core = Arc::new(OutputCore::new(
            id,
            1,
            Arc::new(NoopStatus),
            TurnWiring::new(table.clone(), super::super::classify_turn),
        ));
        let seen = Arc::new(Mutex::new(Vec::new()));
        core.subscribe(Arc::new(EventSink {
            id: SinkId::new_v4(),
            seen: seen.clone(),
        }));
        let state = shared();
        let reader = Reader {
            core,
            decoder: Some(Box::new(super::super::decoder::CodexAppServerDecoder::new())),
            state: state.clone(),
            pending: Arc::new(Pending::default()),
            first_turn: None,
            refused: RefusedItems::default(),
        };
        FactHarness {
            reader,
            state,
            table,
            id,
            seen,
        }
    }

    /// ★A line while no turn is open goes to the screen but does not turn "mid-turn" on★ — otherwise that turn's end would be blocked by
    /// the attribution gate and nobody could turn it off (no mail for 30 minutes; link Idle but only the fact layer busy).
    #[test]
    fn a_line_that_arrives_with_no_open_turn_is_shown_but_never_marks_the_agent_busy() {
        let mut h = fact_harness();
        make_ready(&h.state, "T");

        h.reader
            .handle_line(progress_line("T", "U-OTHER").as_bytes());
        assert!(
            !h.table.is_in_turn(h.id, 1),
            "an unattributed line turned the fact layer on"
        );
        // ★Not passed by dropping it★ — the line goes to the screen as is; only the fact layer was split off.
        assert!(
            h.seen
                .lock()
                .unwrap()
                .iter()
                .any(|e| matches!(e, OutputEvent::TextDelta { .. })),
            "the body disappeared from the screen"
        );

        // That end cannot be attributed and is blocked — still, nothing is left in the table to turn off.
        h.reader
            .handle_line(completed_line("T", "U-OTHER", "completed").as_bytes());
        assert!(!h.table.is_in_turn(h.id, 1));
        assert!(
            boundaries(&h.seen).is_empty(),
            "an unattributed boundary showed on screen"
        );
    }

    #[test]
    fn a_line_inside_our_open_turn_marks_the_agent_busy_and_its_completion_clears_it() {
        let mut h = fact_harness();
        make_ready(&h.state, "T");
        activate(&h.state, 0, Some("U-MINE"));

        h.reader
            .handle_line(progress_line("T", "U-MINE").as_bytes());
        assert!(h.table.is_in_turn(h.id, 1));
        h.reader
            .handle_line(completed_line("T", "U-MINE", "completed").as_bytes());
        assert!(!h.table.is_in_turn(h.id, 1));
    }

    /// ★Item notifications before the response are normal★ — blocking by id match would push our turn's streaming out of the fact layer;
    /// the decision is "does not contradict", not "matches".
    #[test]
    fn a_line_that_arrives_before_the_turn_id_is_known_still_counts_as_ours() {
        let mut h = fact_harness();
        make_ready(&h.state, "T");
        activate(&h.state, 0, None);

        h.reader
            .handle_line(progress_line("T", "U-MINE").as_bytes());
        assert!(h.table.is_in_turn(h.id, 1));
    }

    /// ★A held end is dropped with that turn★ — otherwise when the peer reuses the id [`Reader::resolve`] closes **the freshly opened
    /// turn** and raises a stale boundary.
    #[test]
    fn a_held_completion_does_not_survive_the_turn_that_was_open_when_it_arrived() {
        let mut h = harness_with_real_decoder();
        make_ready(&h.state, "T");
        activate(&h.state, 0, None);

        h.reader
            .handle_line(completed_line("T", "U-1", "completed").as_bytes());
        assert_eq!(
            with_state(&h.state, |s| s.early_completions.len()),
            1,
            "did not hold the end that came before the response"
        );

        // That turn ends by another path (deadline · write failure · error response — all via end_turn_if).
        let core = h.reader.core.clone();
        assert!(end_turn_if(
            &h.state,
            &core,
            0,
            "giving up".into(),
            TurnClose::Unanswered
        ));
        assert_eq!(
            with_state(&h.state, |s| s.early_completions.len()),
            0,
            "the held end lived longer than the turn"
        );

        // Even if the next turn receives the same id, it stays open.
        activate(&h.state, 1, None);
        assert!(h
            .pending
            .register(5, Waiter::TurnStart { seq: 1 }, method::TURN_START));
        h.reader
            .handle_line(br#"{"id":5,"result":{"turn":{"id":"U-1"}}}"#);
        assert_eq!(
            turn_seq(&h.state),
            Some(1),
            "the freshly opened turn was closed by a stale slot"
        );
        assert_eq!(
            boundaries(&h.seen).len(),
            1,
            "the stale boundary showed on screen once more: {:?}",
            boundaries(&h.seen)
        );
    }

    /// ★The turn id the response decides is capped too★ — uncapped it goes into the boundary field and alone exceeds the replay ring's
    /// single-event cap. Filtered, not truncated, so the turn has no attribution material and closes at the same grade as an error response.
    #[test]
    fn an_oversized_turn_id_in_the_response_closes_the_turn_instead_of_becoming_a_boundary_field() {
        let h = harness();
        make_ready(&h.state, "T");
        activate(&h.state, 0, None);
        assert!(h
            .pending
            .register(5, Waiter::TurnStart { seq: 0 }, method::TURN_START));

        let huge = "u".repeat(MAX_TURN_ID_BYTES + 1);
        h.reader.resolve(
            &RequestId::Num(5),
            Ok(serde_json::json!({"turn": {"id": huge}})),
        );

        assert_eq!(turn_seq(&h.state), None, "the turn remained open");
        let seen = boundaries(&h.seen);
        assert_eq!(seen.len(), 1, "{seen:?}");
        assert!(matches!(seen[0].1, TurnOutcome::Failed { .. }), "{seen:?}");
        assert!(
            seen[0].0.is_none(),
            "an id over the cap was carried on the boundary"
        );
    }

    #[test]
    fn a_line_from_another_turn_or_thread_does_not_count_even_while_ours_is_open() {
        let mut h = fact_harness();
        make_ready(&h.state, "T");
        activate(&h.state, 0, Some("U-MINE"));

        h.reader
            .handle_line(progress_line("T", "U-OTHER").as_bytes());
        assert!(
            !h.table.is_in_turn(h.id, 1),
            "someone else's turn turned it on"
        );
        h.reader
            .handle_line(progress_line("OTHER", "U-MINE").as_bytes());
        assert!(
            !h.table.is_in_turn(h.id, 1),
            "someone else's thread turned it on"
        );
    }

    fn outbox_lines(state: &SharedState) -> Vec<Value> {
        with_state(state, |s| {
            s.outbox
                .iter()
                .map(|l| serde_json::from_str(l).expect("outgoing lines are JSON"))
                .collect()
        })
    }

    /// A short wait — used only where "it did **not** wake" is measured.
    const NOT_WOKEN: Duration = Duration::from_millis(50);

    // ── Envelope routing: server requests ────────────────────────────────────────────

    #[test]
    fn a_server_request_is_refused_with_its_own_id_and_never_a_success() {
        let mut h = harness();
        h.reader
            .handle_line(br#"{"id":7,"method":"item/approval/request","params":{}}"#);

        let lines = outbox_lines(&h.state);
        assert_eq!(lines.len(), 1, "one answer per request: {lines:?}");
        assert_eq!(lines[0]["id"], 7, "the received id must be returned as is");
        assert_eq!(lines[0]["error"]["code"], METHOD_NOT_FOUND);
        assert!(
            lines[0].get("result").is_none(),
            "★disguising success is an automatic approval★: {:?}",
            lines[0]
        );
    }

    #[test]
    fn a_server_request_id_is_echoed_verbatim_when_it_is_a_string() {
        let mut h = harness();
        h.reader.handle_line(br#"{"id":"srv-1","method":"x"}"#);
        let lines = outbox_lines(&h.state);
        assert_eq!(
            lines[0]["id"], "srv-1",
            "normalizing a string id to a number leaves that request unresolved forever"
        );
    }

    /// ★The core of TRD §4-4★ — server ids and our ids overlap (server ids are returned as is); **the envelope shape** tells them apart,
    /// not the value.
    #[test]
    fn a_server_request_whose_id_collides_with_ours_never_touches_the_pending_map() {
        let mut h = harness();
        let (tx, rx) = mpsc::channel();
        let _ = h
            .pending
            .register(0, Waiter::Handshake(tx), method::THREAD_START);

        h.reader
            .handle_line(br#"{"id":0,"method":"item/approval/request"}"#);

        assert_eq!(
            h.pending.len(),
            1,
            "a request envelope touched the waiting table"
        );
        assert!(
            rx.recv_timeout(NOT_WOKEN).is_err(),
            "an approval request was delivered to our thread/start waiter"
        );
        assert_eq!(
            outbox_lines(&h.state).len(),
            1,
            "an answer must still go out"
        );
    }

    #[test]
    fn a_notification_is_never_looked_up_in_the_pending_map() {
        let mut h = harness();
        let (tx, _rx) = mpsc::channel();
        let _ = h
            .pending
            .register(0, Waiter::Handshake(tx), method::INITIALIZE);
        h.reader
            .handle_line(br#"{"method":"turn/completed","params":{"threadId":"t"}}"#);
        assert_eq!(h.pending.len(), 1);
    }

    // ── Envelope routing: responses ─────────────────────────────────────────────────

    #[test]
    fn two_waiters_do_not_cross() {
        let mut h = harness();
        let (tx0, rx0) = mpsc::channel();
        let (tx1, rx1) = mpsc::channel();
        let _ = h
            .pending
            .register(0, Waiter::Handshake(tx0), method::INITIALIZE);
        let _ = h
            .pending
            .register(1, Waiter::Handshake(tx1), method::THREAD_START);

        h.reader
            .handle_line(br#"{"id":1,"result":{"thread":{"id":"T-1"}}}"#);

        let got = rx1.recv_timeout(NOT_WOKEN).expect("waiter 1 must wake");
        let parsed: ThreadStartResponse =
            serde_json::from_value(got.expect("success response")).expect("decode");
        assert_eq!(parsed.thread.id, "T-1");
        assert!(
            rx0.recv_timeout(NOT_WOKEN).is_err(),
            "waiter 0 received someone else's answer"
        );
        assert_eq!(
            h.pending.len(),
            1,
            "only the woken waiting entry is taken away"
        );
    }

    #[test]
    fn an_error_envelope_wakes_the_waiter_as_a_failure() {
        let mut h = harness();
        let (tx, rx) = mpsc::channel();
        let _ = h
            .pending
            .register(3, Waiter::Handshake(tx), method::INITIALIZE);
        h.reader
            .handle_line(br#"{"id":3,"error":{"code":-32600,"message":"Already initialized"}}"#);
        let got = rx.recv_timeout(NOT_WOKEN).expect("must wake");
        let msg = got.expect_err("an error envelope is a failure");
        assert!(msg.contains("Already initialized"), "{msg}");
    }

    /// ★The peer's error body goes to three places — log · screen · caller — so masking is done **where it is made**★, not at the three
    /// doors (one would be forgotten).
    #[test]
    fn a_peer_error_message_is_masked_before_it_leaves_this_module() {
        let mut h = harness();
        let (tx, rx) = mpsc::channel();
        let _ = h
            .pending
            .register(4, Waiter::Handshake(tx), method::INITIALIZE);
        let secret = "sk-proj-ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
        h.reader.handle_line(
            format!(
                r#"{{"id":4,"error":{{"code":-32600,"message":"bad config token={secret}"}}}}"#
            )
            .as_bytes(),
        );
        let msg = rx
            .recv_timeout(NOT_WOKEN)
            .expect("must wake")
            .expect_err("an error");
        assert!(
            !msg.contains(secret),
            "the credential leaked out as is: {msg}"
        );
        assert!(msg.contains("***"), "{msg}");
    }

    /// ★[`clip`] must be called only inside [`sanitize`]★ — elsewhere the string skips masking and leaks **quietly**. This item measures
    /// that property **in the source**; one credential item per leaking path is not maintainable.
    #[test]
    fn clip_is_only_reachable_through_the_masking_door() {
        let src = include_str!("transport.rs");
        // ★Not split by `#[cfg(test)]`★ — that attribute is also inside the production section (test-bench accessors); cutting there would scan only the front part.
        // ★No marker containing a newline★ — the repository checks out with CRLF, so `\n` would not match.
        let production = src.split("mod tests {").next().expect("production section");
        let offenders: Vec<&str> = production
            .lines()
            .map(|l| l.trim())
            .filter(|l| l.contains("clip("))
            .filter(|l| !l.starts_with("//"))
            .filter(|l| !l.starts_with("fn clip(") && !l.starts_with("clip(&mask_secrets("))
            .collect();
        assert!(
            offenders.is_empty(),
            "there is a truncation call that skips masking: {offenders:?}"
        );
    }

    #[test]
    fn a_response_for_an_unknown_id_is_discarded_without_disturbing_anything() {
        let mut h = harness();
        let (tx, rx) = mpsc::channel();
        let _ = h
            .pending
            .register(0, Waiter::Handshake(tx), method::INITIALIZE);

        h.reader.handle_line(br#"{"id":9999,"result":{}}"#);
        h.reader.handle_line(br#"{"id":"server-side","result":{}}"#);

        assert_eq!(
            h.pending.len(),
            1,
            "an unknown id took away someone else's waiting entry"
        );
        assert!(rx.recv_timeout(NOT_WOKEN).is_err());
        assert!(
            outbox_lines(&h.state).is_empty(),
            "responses are not answered"
        );
        assert!(
            h.seen.lock().unwrap().is_empty(),
            "nothing is raised to the screen"
        );
    }

    /// If the stream were cut by an unreadable line the agent would die — it drops the line and keeps going.
    #[test]
    fn unreadable_lines_are_dropped_and_the_next_line_still_routes() {
        let mut h = harness();
        h.reader.handle_line(b"not json at all");
        h.reader.handle_line(br#"{"no":"envelope"}"#);
        h.reader.handle_line(br#"{"id":1,"method":"x"}"#);
        assert_eq!(outbox_lines(&h.state).len(), 1);
    }

    // ── Notification → translator ─────────────────────────────────────────────────────

    #[test]
    fn a_notification_reaches_the_decoder_as_the_original_line_with_its_newline() {
        let mut h = harness();
        let line =
            br#"{"method":"item/agentMessage/delta","params":{"turnId":"T","itemId":"I","delta":"hi"}}"#;
        h.reader.handle_line(line);
        let got = h.decoded.lock().unwrap().clone();
        let mut expected = line.to_vec();
        expected.push(b'\n');
        assert_eq!(
            got, expected,
            "the translator must receive the original line as is"
        );
    }

    #[test]
    fn responses_and_requests_never_reach_the_decoder() {
        let mut h = harness();
        h.reader.handle_line(br#"{"id":1,"result":{}}"#);
        h.reader.handle_line(br#"{"id":2,"method":"x"}"#);
        assert!(
            h.decoded.lock().unwrap().is_empty(),
            "the translator receives only notifications"
        );
    }

    // ── Turn state machine ──────────────────────────────────────────────────────

    /// ★`turn/started` takes no part in identity — do not revive it★. It has no identifier we issued; telling it apart by "is the current
    /// turn waiting for an answer" lets a late notification of the previous turn pass **while the new turn waits for its own request** and
    /// write its id into the new turn's slot.
    #[test]
    fn a_turn_started_notification_never_sets_the_turn_id() {
        let mut h = harness();
        make_ready(&h.state, "T");
        activate(&h.state, 0, None);
        // The current turn's request is waiting for an answer — exactly the state the old circumstantial gate let through.
        let _ = h
            .pending
            .register(9, Waiter::TurnStart { seq: 0 }, method::TURN_START);
        h.reader.handle_line(
            br#"{"method":"turn/started","params":{"threadId":"T","turn":{"id":"U-FROM-NOTIFY"}}}"#,
        );
        assert_eq!(
            turn_id_of(&h.state),
            None,
            "a notification decided the turn id — at that spot, a late notification of the previous turn comes in by the same path"
        );
    }

    #[test]
    fn the_turn_id_can_come_from_the_response() {
        let mut h = harness();
        make_ready(&h.state, "T");
        activate(&h.state, 0, None);
        let _ = h
            .pending
            .register(5, Waiter::TurnStart { seq: 0 }, method::TURN_START);
        h.reader
            .handle_line(br#"{"id":5,"result":{"turn":{"id":"U-2"}}}"#);
        assert_eq!(turn_id_of(&h.state).as_deref(), Some("U-2"));
    }

    /// ★A late response of the previous turn must not write its id into the current turn's slot★ — a later `interrupt` would aim at the
    /// wrong turn. Only the marker tells them apart; without it both look like "the turn id is empty".
    #[test]
    fn a_late_response_from_a_finished_turn_never_lands_on_the_next_turn() {
        let mut h = harness();
        make_ready(&h.state, "T");
        activate(&h.state, 0, None);
        let _ = h
            .pending
            .register(5, Waiter::TurnStart { seq: 0 }, method::TURN_START);
        activate(&h.state, 1, None);
        h.reader
            .handle_line(br#"{"id":5,"result":{"turn":{"id":"U-OLD"}}}"#);
        assert_eq!(
            turn_id_of(&h.state),
            None,
            "the answer of an ended turn filled the current turn's slot"
        );
        assert_eq!(
            turn_seq(&h.state),
            Some(1),
            "the turn itself must stay as is"
        );
    }

    /// ★An unknown threadId is not an error★ — but a **known, different** threadId is not our turn.
    #[test]
    fn a_turn_notification_for_another_thread_does_not_end_our_turn() {
        let mut h = harness();
        make_ready(&h.state, "MINE");
        activate(&h.state, 0, Some("U"));
        h.reader.handle_line(
            br#"{"method":"turn/completed","params":{"threadId":"OTHER","turn":{"id":"U"}}}"#,
        );
        assert!(
            turn_seq(&h.state).is_some(),
            "another thread closed our turn"
        );
    }

    /// ★An end of **another turn** of the same thread must not close ours★ — else the next turn opens and two run at once on one thread,
    /// which the input queue exists to prevent.
    #[test]
    fn a_turn_completed_for_a_different_turn_on_our_thread_does_not_end_ours() {
        let mut h = harness();
        make_ready(&h.state, "T");
        activate(&h.state, 0, Some("U-MINE"));
        with_state(&h.state, |s| s.push_legacy(b"queued"));
        h.reader.handle_line(
            br#"{"method":"turn/completed","params":{"threadId":"T","turn":{"id":"U-OTHER"}}}"#,
        );
        assert!(
            turn_seq(&h.state).is_some(),
            "someone else's turn end closed our turn"
        );
        assert!(
            with_state(&h.state, |s| take_turn_locked(s, &h.next_id)).is_none(),
            "because of that the queue opened and turns overlapped"
        );
    }

    /// ★An end while the turn id is unknown ends nothing★ — closing a live turn on an unattributable signal opens the queue and overlaps turns.
    #[test]
    fn a_turn_completed_we_cannot_attribute_ends_nothing() {
        let mut h = harness();
        make_ready(&h.state, "T");
        activate(&h.state, 0, None);
        with_state(&h.state, |s| s.push_legacy(b"queued"));
        h.reader.handle_line(
            br#"{"method":"turn/completed","params":{"threadId":"T","turn":{"id":"U-?"}}}"#,
        );
        assert_eq!(
            turn_seq(&h.state),
            Some(0),
            "an unattributable end closed the turn"
        );
        assert!(
            with_state(&h.state, |s| take_turn_locked(s, &h.next_id)).is_none(),
            "because of that the queue opened and turns overlapped"
        );
    }

    /// ★Measures exactly the order that used to get stuck★: the end first (unattributable → ignored), and **no sweep in between**, then
    /// the response decides our turn's id. That response removes the waiting entry, so if the turn is not ended here nothing ever ends it.
    /// ★A sweep would stop measuring this order★ — the deadline would end it and the response-side comparison would never run.
    #[test]
    fn an_early_completion_is_reconciled_when_the_response_names_it() {
        let mut h = harness();
        make_ready(&h.state, "T");
        activate(&h.state, 0, None);
        with_state(&h.state, |s| s.push_legacy(b"queued"));
        let _ = h
            .pending
            .register(5, Waiter::TurnStart { seq: 0 }, method::TURN_START);

        h.reader.handle_line(
            br#"{"method":"turn/completed","params":{"threadId":"T","turn":{"id":"U-EARLY"}}}"#,
        );
        assert_eq!(
            turn_seq(&h.state),
            Some(0),
            "an unattributable end closed the turn on the spot"
        );

        h.reader
            .handle_line(br#"{"id":5,"result":{"turn":{"id":"U-EARLY"}}}"#);

        assert_eq!(
            turn_seq(&h.state),
            None,
            "the response decided the id but did not compare it with the end that came first — this turn has nothing to end it forever"
        );
        assert_eq!(
            h.pending.len(),
            0,
            "the response took away the waiting entry (no deadline backstop)"
        );
        assert!(
            with_state(&h.state, |s| take_turn_locked(s, &h.next_id)).is_some(),
            "the queue did not open"
        );
    }

    /// ★The comparison is still only **the id match**★ — no turn ends merely because a held end exists.
    #[test]
    fn an_early_completion_for_another_turn_does_not_end_ours() {
        let mut h = harness();
        make_ready(&h.state, "T");
        activate(&h.state, 0, None);
        let _ = h
            .pending
            .register(5, Waiter::TurnStart { seq: 0 }, method::TURN_START);

        h.reader.handle_line(
            br#"{"method":"turn/completed","params":{"threadId":"T","turn":{"id":"U-SOMEONE-ELSE"}}}"#,
        );
        h.reader
            .handle_line(br#"{"id":5,"result":{"turn":{"id":"U-MINE"}}}"#);

        assert_eq!(
            turn_seq(&h.state),
            Some(0),
            "our turn was closed because someone else's end is held"
        );
        assert_eq!(turn_id_of(&h.state).as_deref(), Some("U-MINE"));
    }

    // ── Turn boundary attribution gate ─────────────────────────────────────────────────
    //
    // The items above measure that **this link's state machine** is not fooled by someone else's end; the items below measure that
    // **the screen and the fact layer** get the same decision.

    /// ★Exactly the named input★: our turn is `U-MINE` but an end of `U-OTHER` of the same thread comes. The link ignores it, but the
    /// translator does not know attribution and emits a boundary — flowing as is, classifier and frontend (neither reads the id) would
    /// **stamp the live `U-MINE` as ended.**
    #[test]
    fn a_foreign_turn_completed_never_ends_the_live_turn() {
        let (mut h, turns, id) = harness_with_facts();
        make_ready(&h.state, "MINE");
        activate(&h.state, 0, Some("U-MINE"));
        h.reader.core.emit(OutputEvent::Structured {
            kind: "user".into(),
            json: "{}".into(),
        });
        assert!(turns.is_in_turn(id, 1), "premise: observed as mid-turn");

        h.reader
            .handle_line(completed_line("MINE", "U-OTHER", "completed").as_bytes());

        assert!(
            boundaries(&h.seen).is_empty(),
            "an unattributable end emitted a screen boundary"
        );
        assert!(
            turns.is_in_turn(id, 1),
            "someone else's turn end flipped our turn to idle — mail gets plugged in mid-turn"
        );
        assert_eq!(
            turn_seq(&h.state),
            Some(0),
            "even the state machine was shaken"
        );
    }

    #[test]
    fn a_turn_completed_from_another_thread_makes_no_boundary() {
        let mut h = harness_with_real_decoder();
        make_ready(&h.state, "MINE");
        activate(&h.state, 0, Some("U-MINE"));
        h.reader
            .handle_line(completed_line("OTHER", "U-MINE", "completed").as_bytes());
        assert!(boundaries(&h.seen).is_empty(), "{:?}", boundaries(&h.seen));
    }

    /// ★If attributed, the boundary flows **as is** — including the outcome★; a gate that only blocks would leave the waiting indicator
    /// spinning forever.
    #[test]
    fn an_attributed_turn_completed_makes_exactly_one_boundary_with_its_outcome() {
        let mut h = harness_with_real_decoder();
        make_ready(&h.state, "T");
        activate(&h.state, 0, Some("U-1"));
        h.reader
            .handle_line(completed_line("T", "U-1", "interrupted").as_bytes());
        assert_eq!(
            boundaries(&h.seen),
            vec![(Some("U-1".to_string()), TurnOutcome::Interrupted)]
        );
        assert_eq!(
            turn_seq(&h.state),
            None,
            "the state machine must close together too"
        );
    }

    /// ★The boundary of an end before the response is **deferred**, neither dropped nor let through★ — letting it through would let an
    /// unattributed end close a live turn; dropping it would leave nothing to raise after the comparison holds, and the waiting indicator
    /// would spin forever.
    #[test]
    fn an_early_completion_boundary_is_held_until_the_response_names_it() {
        let mut h = harness_with_real_decoder();
        make_ready(&h.state, "T");
        activate(&h.state, 0, None);
        let _ = h
            .pending
            .register(5, Waiter::TurnStart { seq: 0 }, method::TURN_START);

        h.reader
            .handle_line(completed_line("T", "U-EARLY", "interrupted").as_bytes());
        assert!(
            boundaries(&h.seen).is_empty(),
            "the boundary flowed before attribution — a live turn closes on screen"
        );

        h.reader
            .handle_line(br#"{"id":5,"result":{"turn":{"id":"U-EARLY"}}}"#);
        assert_eq!(
            boundaries(&h.seen),
            vec![(Some("U-EARLY".to_string()), TurnOutcome::Interrupted)],
            "the held outcome did not come up even after the comparison"
        );
    }

    #[test]
    fn an_early_completion_for_another_turn_never_releases_its_boundary() {
        let mut h = harness_with_real_decoder();
        make_ready(&h.state, "T");
        activate(&h.state, 0, None);
        let _ = h
            .pending
            .register(5, Waiter::TurnStart { seq: 0 }, method::TURN_START);

        h.reader
            .handle_line(completed_line("T", "U-SOMEONE-ELSE", "completed").as_bytes());
        h.reader
            .handle_line(br#"{"id":5,"result":{"turn":{"id":"U-MINE"}}}"#);
        assert!(boundaries(&h.seen).is_empty(), "{:?}", boundaries(&h.seen));
    }

    // ── A turn that dies without an outcome (ADR-0127) ─────────────────────────────────────────

    /// ★The only function that folds a turn **always** emits a boundary★ — the only way this link's four give-up paths (write failure ·
    /// response decode failure · error response · deadline) return the fact layer. On a marker mismatch it ends nothing and **emits no
    /// boundary either** — a late signal must not invent the next turn's boundary.
    #[test]
    fn ending_a_turn_always_makes_a_failure_boundary_and_a_stale_marker_makes_none() {
        let (core, seen) = core_with_sink();
        let state = shared();
        activate(&state, 3, Some("U-3"));

        assert!(
            !end_turn_if(
                &state,
                &core,
                2,
                "a late signal".into(),
                TurnClose::Unanswered
            ),
            "ended although the marker differs"
        );
        assert!(
            boundaries(&seen).is_empty(),
            "emitted a boundary although it did not end"
        );

        assert!(end_turn_if(
            &state,
            &core,
            3,
            "giving up".into(),
            TurnClose::Unanswered
        ));
        assert_eq!(
            boundaries(&seen),
            vec![(
                Some("U-3".to_string()),
                TurnOutcome::Failed {
                    detail: Some("giving up".to_string())
                }
            )],
            "the reason is carried **inside** the boundary — no separate error line goes in front"
        );
    }

    /// ★The very outcome ADR-0127 prevents★: a link that gave up mid-turn without returning the fact layer leaves the incarnation observed
    /// "mid-turn" although idle, mail blocked until the 30-minute fail-open. ★One [`OutputEvent::Error`] line cannot do this★ — the
    /// classifier does not count that vocabulary as a turn signal.
    #[test]
    fn a_turn_abandoned_without_a_completion_clears_the_fact_layer() {
        let (h, turns, id) = harness_with_facts();
        make_ready(&h.state, "T");
        activate(&h.state, 0, Some("U-1"));
        h.reader.core.emit(OutputEvent::Structured {
            kind: "user".into(),
            json: "{}".into(),
        });
        assert!(turns.is_in_turn(id, 1), "premise: observed as mid-turn");

        assert!(end_turn_if(
            &h.state,
            &h.reader.core,
            0,
            "codex app-server input send failed".into(),
            TurnClose::Rejected
        ));
        assert!(
            !turns.is_in_turn(id, 1),
            "folded the turn but the fact layer is still mid-turn"
        );
    }

    #[test]
    fn an_error_response_to_turn_start_ends_the_turn_with_its_reason_inside_the_boundary() {
        let mut h = harness();
        make_ready(&h.state, "T");
        activate(&h.state, 0, None);
        with_state(&h.state, |s| s.push_legacy(b"queued"));
        let _ = h
            .pending
            .register(5, Waiter::TurnStart { seq: 0 }, method::TURN_START);

        h.reader
            .handle_line(br#"{"id":5,"error":{"code":-32000,"message":"turn refused"}}"#);

        assert_eq!(
            turn_seq(&h.state),
            None,
            "the error response left the turn open"
        );
        let failures = failure_boundaries(&h.seen);
        assert_eq!(failures.len(), 1, "{failures:?}");
        assert!(failures[0].contains("turn refused"), "{failures:?}");
        assert!(
            with_state(&h.state, |s| take_turn_locked(s, &h.next_id)).is_some(),
            "the queue did not open"
        );
    }

    /// ★Even if the real end comes late to a turn closed by giving up, there is **one** boundary★ — two would close the same turn twice on
    /// screen and pile up an unpaired end in the fact layer.
    #[test]
    fn an_abandoned_turn_and_its_late_completion_make_exactly_one_boundary() {
        let mut h = harness_with_real_decoder();
        make_ready(&h.state, "T");
        activate(&h.state, 0, Some("U-1"));
        let _ = h.pending.register_at(
            9,
            Waiter::TurnStart { seq: 0 },
            method::TURN_START,
            Instant::now(),
        );

        sweep_deadlines(&h.state, &h.pending, &h.reader.core);
        h.reader
            .handle_line(completed_line("T", "U-1", "completed").as_bytes());

        let seen = boundaries(&h.seen);
        assert_eq!(seen.len(), 1, "the same turn was closed twice: {seen:?}");
        assert!(
            matches!(seen[0].1, TurnOutcome::Failed { .. }),
            "the outcome that stood first was overturned: {seen:?}"
        );
    }

    /// Runs the writer once without stdin to make the handshake fail. `queued` = bodies that had already received `Ok` in that window.
    fn run_failed_handshake(queued: &[&str]) -> (SharedState, Arc<Mutex<Vec<OutputEvent>>>) {
        let (core, seen) = core_with_sink();
        let state = shared();
        with_state(&state, |s| {
            for body in queued {
                s.push_legacy(body.as_bytes());
            }
        });

        writer_loop(
            Arc::new(Mutex::new(None)),
            state.clone(),
            Arc::new(Pending::default()),
            Arc::new(AtomicI64::new(0)),
            Arc::new(AtomicBool::new(true)),
            core,
            ThreadOpen::Start(ThreadStartParams {
                cwd: None,
                approval_policy: None,
                sandbox: None,
                developer_instructions: None,
            }),
            None,
            None,
            announcer(),
        );

        (state, seen)
    }

    /// Pulls the errors · boundaries raised to the screen **in arrival order** — this branch must measure "does the reason come before the
    /// boundary", not just the count.
    fn error_and_boundary_order(seen: &Arc<Mutex<Vec<OutputEvent>>>) -> Vec<&'static str> {
        seen.lock()
            .unwrap()
            .iter()
            .filter_map(|e| match e {
                OutputEvent::Error(_) => Some("error"),
                OutputEvent::TurnEnd { .. } => Some("boundary"),
                _ => None,
            })
            .collect()
    }

    /// ★Even with no dropped input, **one** failure outcome goes out★ — a turn opens only from [`Link::Ready`], so this incarnation never
    /// had one, but the frontend does not read `Error` as a turn end, so a slot with only an error spins its waiting indicator **forever**
    /// (the link is `Down`). ★That reason does not claim input loss★ — nothing was dropped.
    #[test]
    fn a_failed_handshake_with_nothing_queued_still_ends_the_turn_exactly_once() {
        let (state, seen) = run_failed_handshake(&[]);

        assert_eq!(
            error_and_boundary_order(&seen),
            vec!["error", "boundary"],
            "not one reason line followed by one boundary: {:?}",
            error_and_boundary_order(&seen)
        );
        assert_eq!(
            boundaries(&seen)[0].0,
            None,
            "invented the id of a turn that never opened: {:?}",
            boundaries(&seen)
        );
        let failures = failure_boundaries(&seen);
        assert_eq!(failures.len(), 1, "not a failure outcome: {failures:?}");
        assert!(
            !failures[0].contains("were lost"),
            "reports a loss although no input was dropped: {failures:?}"
        );
        assert!(with_state(&state, |s| matches!(s.link, Link::Down(_))));
    }

    /// ★With queued bodies destroyed, **one** failure outcome goes out★ — `dropped` counts bodies, not turns. ★Without this boundary★ the
    /// frontend does not read `Error` as a turn end (retried stream errors use that vocabulary), so a slot that received only an error spins
    /// **forever**.
    #[test]
    fn a_failed_handshake_that_destroyed_queued_input_ends_the_turn_exactly_once() {
        let (state, seen) = run_failed_handshake(&["queued one", "queued two"]);

        assert_eq!(
            error_and_boundary_order(&seen),
            vec!["error", "boundary"],
            "not one reason line followed by one boundary: {:?}",
            error_and_boundary_order(&seen)
        );
        assert_eq!(
            boundaries(&seen)[0].0,
            None,
            "invented the id of a turn that never opened: {:?}",
            boundaries(&seen)
        );
        let failures = failure_boundaries(&seen);
        assert_eq!(failures.len(), 1, "not a failure outcome: {failures:?}");
        assert!(failures[0].contains("2 sent input(s)"), "{failures:?}");
        assert!(with_state(&state, |s| s.pending.is_empty()));
        assert!(with_state(&state, |s| matches!(s.link, Link::Down(_))));
    }

    /// ★If the places that return a turn to idle grow, this goes red★ — each must answer "who emits the boundary" (ADR-0127). Today:
    /// [`end_turn_if`] · [`Reader::note_turn`] · [`Reader::resolve`] · [`ReaderExit::drop`].
    #[test]
    fn every_place_that_idles_a_turn_is_accounted_for() {
        let src = include_str!("transport.rs");
        let production = src.split("mod tests {").next().expect("production section");
        let sites = production
            .lines()
            .map(|l| l.trim())
            .filter(|l| l.starts_with("s.turn = TurnState::Idle"))
            .count();
        assert_eq!(
            sites, 4,
            "the places that return a turn to idle are not four — a new place must emit its own turn boundary too"
        );
        // ★Every such place must also drop the held ends★ — else that slot survives to the next turn and when the peer reuses the id the fresh
        //   turn is closed by a stale boundary ([`end_turn_if`]).
        let drops = production
            .lines()
            .map(|l| l.trim())
            .filter(|l| l.starts_with("s.early_completions.clear()"))
            .count();
        assert_eq!(
            drops, sites,
            "the number of idle places differs from the number of places dropping held ends"
        );
        // ★Every such place must also dispose that turn's items (ADR-0231)★ — a missed place leaves the ended turn's `InFlight` without an
        //   outcome and turns all new text into waiting. Only exception: the stream closing (`OutputCore::finish` closes list items as
        //   `Dropped{AgentEnded}`; the link is no longer used).
        let disposals = production
            .lines()
            .map(|l| l.trim())
            .filter(|l| !l.starts_with("//"))
            .filter(|l| l.contains(".close_turn_items("))
            .count();
        assert_eq!(
            disposals,
            sites - 1,
            "every place that returns a turn to idle (except the stream closing) must dispose that turn's items"
        );
    }

    /// ★Even if an unattributed end is dropped, that turn's **deadline must remain**★ — removing the waiting entry while ignoring it would
    /// leave nothing to end the turn. This measures only that, **not** "an unattributable turn always has a deadline" (false — an end before
    /// the response is carried by the module header's known limits).
    #[test]
    fn an_ignored_completion_leaves_the_turn_s_deadline_intact() {
        let mut h = harness();
        make_ready(&h.state, "T");
        activate(&h.state, 0, None);
        with_state(&h.state, |s| s.push_legacy(b"queued"));
        let _ = h.pending.register_at(
            7,
            Waiter::TurnStart { seq: 0 },
            method::TURN_START,
            Instant::now(),
        );

        // An unattributable end passes first — it must end nothing and take nothing away.
        h.reader.handle_line(
            br#"{"method":"turn/completed","params":{"threadId":"T","turn":{"id":"U-?"}}}"#,
        );
        assert_eq!(
            turn_seq(&h.state),
            Some(0),
            "an unattributable end closed the turn"
        );
        assert_eq!(
            h.pending.len(),
            1,
            "took away the waiting entry while ignoring it — then there is nothing to end it"
        );

        sweep_deadlines(&h.state, &h.pending, &h.reader.core);
        assert_eq!(
            turn_seq(&h.state),
            None,
            "the remaining deadline did not end that turn"
        );
        assert!(
            with_state(&h.state, |s| take_turn_locked(s, &h.next_id)).is_some(),
            "the queue did not open"
        );
    }

    /// ★An undecodable success response is **the same grade** as an explicit error★ — the waiting entry is already gone, so a turn left open
    /// would stay open forever with neither `turn_id` nor a deadline (`send_input` then reports "the queue is full").
    #[test]
    fn a_success_response_we_cannot_read_ends_the_turn_like_an_error_would() {
        let mut h = harness();
        make_ready(&h.state, "T");
        activate(&h.state, 0, None);
        with_state(&h.state, |s| s.push_legacy(b"queued"));
        let _ = h
            .pending
            .register(5, Waiter::TurnStart { seq: 0 }, method::TURN_START);

        // A success response without the `turn` field — the envelope is sound, the payload does not read as our type.
        h.reader
            .handle_line(br#"{"id":5,"result":{"not-a-turn":true}}"#);

        assert_eq!(
            turn_seq(&h.state),
            None,
            "an undecodable success left the turn open"
        );
        assert_eq!(
            h.pending.len(),
            0,
            "the waiting entry was already taken away — there is no deadline backstop"
        );
        assert_eq!(
            failure_boundaries(&h.seen).len(),
            1,
            "folding it quietly leaves not a single signal on the screen or in the fact layer"
        );
        assert!(
            with_state(&h.state, |s| take_turn_locked(s, &h.next_id)).is_some(),
            "the queue did not open"
        );
    }

    /// ★The deadline of an already ended turn must not close the turn running now★ — the same overlap opens on the deadline side too.
    #[test]
    fn an_expired_deadline_from_a_finished_turn_does_not_end_the_current_turn() {
        let h = harness();
        make_ready(&h.state, "T");
        activate(&h.state, 1, None);
        let _ = h.pending.register_at(
            5,
            Waiter::TurnStart { seq: 0 },
            method::TURN_START,
            Instant::now(),
        );
        sweep_deadlines(&h.state, &h.pending, &h.reader.core);
        assert_eq!(
            turn_seq(&h.state),
            Some(1),
            "the deadline of an ended turn closed the current turn"
        );
    }

    // ── Input queue ───────────────────────────────────────────────────────────

    #[test]
    fn queued_input_flushes_in_order_one_turn_at_a_time() {
        let h = harness();
        make_ready(&h.state, "T");
        with_state(&h.state, |s| {
            s.push_legacy(b"first");
            s.push_legacy(b"second");
            s.push_legacy(b"third");
        });

        let mut bodies = Vec::new();
        for _ in 0..3 {
            let job = with_state(&h.state, |s| take_turn_locked(s, &h.next_id))
                .expect("when idle, one goes out");
            let Job::Turn { line, .. } = job else {
                panic!("must be a turn job")
            };
            let v: Value = serde_json::from_str(&line).unwrap();
            assert_eq!(v["params"]["threadId"], "T");
            bodies.push(
                v["params"]["input"][0]["text"]
                    .as_str()
                    .unwrap()
                    .to_string(),
            );

            assert!(
                with_state(&h.state, |s| take_turn_locked(s, &h.next_id)).is_none(),
                "a second turn/start went out while a turn is in progress"
            );
            with_state(&h.state, |s| s.turn = TurnState::Idle);
        }
        assert_eq!(bodies, vec!["first", "second", "third"]);
    }

    #[test]
    fn accepted_inputs_carry_their_id_and_origin_in_arrival_order() {
        let state = shared();
        accept_input(
            &state,
            Some("u-1".into()),
            b"one".to_vec(),
            InputOrigin::User,
        )
        .unwrap();
        accept_input(&state, None, b"two".to_vec(), InputOrigin::Mail).unwrap();
        accept_input(
            &state,
            Some("m-3".into()),
            b"three".to_vec(),
            InputOrigin::Mail,
        )
        .unwrap();
        with_state(&state, |s| {
            let got: Vec<_> = s.pending.iter().map(|p| (p.id.clone(), p.origin)).collect();
            assert_eq!(
                got,
                vec![
                    (Some("u-1".to_string()), InputOrigin::User),
                    (None, InputOrigin::Mail),
                    (Some("m-3".to_string()), InputOrigin::Mail),
                ]
            );
            for p in &s.pending {
                assert!(matches!(p.stage, Stage::Held));
                assert!(!p.listed, "before the decision, list items are not decided");
                assert!(!p.cancel_asked);
                assert!(p.death.is_none());
            }
            let announced: Vec<bool> = s.pending.iter().map(|p| p.announced).collect();
            assert_eq!(
                announced,
                vec![false, true, true],
                "only pre-decision user items wait for the announcement — mail · items without an identifier have nothing to emit"
            );
        });
    }

    #[test]
    fn the_turn_opens_from_the_head_whatever_its_origin() {
        let h = harness();
        make_ready(&h.state, "T");
        // Below the floor — today's path, so both leave at the moment of opening.
        with_state(&h.state, |s| s.floor = Floor::Below);
        accept_input(&h.state, None, b"legacy".to_vec(), InputOrigin::Mail).unwrap();
        accept_input(
            &h.state,
            Some("u".into()),
            b"user".to_vec(),
            InputOrigin::User,
        )
        .unwrap();

        let mut bodies = Vec::new();
        for _ in 0..2 {
            let Some(Job::Turn { line, .. }) =
                with_state(&h.state, |s| take_turn_locked(s, &h.next_id))
            else {
                panic!("when idle, the head becomes the turn")
            };
            let v: Value = serde_json::from_str(&line).unwrap();
            bodies.push(
                v["params"]["input"][0]["text"]
                    .as_str()
                    .unwrap()
                    .to_string(),
            );
            with_state(&h.state, |s| s.turn = TurnState::Idle);
        }
        assert_eq!(bodies, vec!["legacy", "user"]);
        assert!(with_state(&h.state, |s| s.pending.is_empty()));
    }

    /// A handed-over item is also one entry — visible in the list until receipt (echo).
    #[test]
    fn the_bound_counts_every_pending_item_whatever_its_stage() {
        let state = shared();
        with_state(&state, |s| {
            for i in 0..INPUT_QUEUE_LIMIT - 1 {
                s.push_pending(Some(format!("u-{i}")), b"x".to_vec(), InputOrigin::User);
            }
            s.pending[0].stage = Stage::InFlight {
                gen: 0,
                turn_id: None,
            };
        });
        accept_input(&state, None, b"last".to_vec(), InputOrigin::Mail).expect("the last entry");
        let over = accept_input(
            &state,
            Some("over".into()),
            b"y".to_vec(),
            InputOrigin::User,
        );
        assert!(matches!(over, Err(PtyError::WriteFailed(_))), "{over:?}");
        assert_eq!(
            with_state(&state, |s| s.pending.len()),
            INPUT_QUEUE_LIMIT,
            "a refused item went in"
        );
    }

    #[test]
    fn turn_completed_releases_the_queue() {
        let mut h = harness();
        make_ready(&h.state, "T");
        activate(&h.state, 0, Some("U"));
        with_state(&h.state, |s| s.push_legacy(b"queued"));
        assert!(
            with_state(&h.state, |s| take_turn_locked(s, &h.next_id)).is_none(),
            "it does not go out mid-turn"
        );
        h.reader.handle_line(
            br#"{"method":"turn/completed","params":{"threadId":"T","turn":{"id":"U"}}}"#,
        );
        assert!(
            with_state(&h.state, |s| take_turn_locked(s, &h.next_id)).is_some(),
            "the turn ended but the queue did not open"
        );
    }

    /// ★This item **cannot measure** atomicity — the signature already carries it★: [`take_turn_locked`] takes `&mut State` (exclusivity is
    /// a compiler guarantee; the lock is the harness's `with_state`), so practically no mutation turns this red. It is kept for **whether the
    /// accounting is right** (with eight rushing in, only one is consumed) — all it measures.
    #[test]
    fn two_concurrent_takers_open_exactly_one_turn() {
        let h = harness();
        make_ready(&h.state, "T");
        with_state(&h.state, |s| {
            s.push_legacy(b"a");
            s.push_legacy(b"b");
        });
        let barrier = Arc::new(std::sync::Barrier::new(8));
        let opened = Arc::new(AtomicI64::new(0));
        let mut handles = Vec::new();
        for _ in 0..8 {
            let state = h.state.clone();
            let next_id = h.next_id.clone();
            let barrier = barrier.clone();
            let opened = opened.clone();
            handles.push(std::thread::spawn(move || {
                barrier.wait();
                if with_state(&state, |s| take_turn_locked(s, &next_id)).is_some() {
                    opened.fetch_add(1, Ordering::Relaxed);
                }
            }));
        }
        for handle in handles {
            handle.join().unwrap();
        }
        assert_eq!(
            opened.load(Ordering::Relaxed),
            1,
            "more than one turn opened"
        );
        assert_eq!(
            with_state(&h.state, |s| s.pending.len()),
            1,
            "only one must be consumed"
        );
    }

    /// Measures **only the `Ready` transition** — not "that value is on disk" (the record port swallows failures inside itself).
    /// ★The point is filling only the thread id and not `Ready`★ — filling both lets the id's absence block it instead, so **it stays green
    /// even if the gate is removed** (measured by mutation).
    #[test]
    fn nothing_is_sent_before_the_link_is_ready_even_when_the_thread_id_is_known() {
        let h = harness();
        with_state(&h.state, |s| {
            s.thread_id = Some("T".into());
            s.push_legacy(b"early");
        });
        assert!(
            with_state(&h.state, |s| take_turn_locked(s, &h.next_id)).is_none(),
            "turn/start went out before the record call returned"
        );
        with_state(&h.state, |s| s.link = Link::Ready);
        assert!(
            with_state(&h.state, |s| take_turn_locked(s, &h.next_id)).is_some(),
            "ready but the queue did not open"
        );
    }

    // ── The two exceptional branches of the record call ──────────────────────────────────────────────

    /// ★An already ended incarnation is not recorded★ — else a dead session's profile gets someone else's conversation and the old sid enters
    /// history. Measures only an end **already observed**; ★both markers★ are needed (`shutdown()` sets the atomic first, then `closed`).
    #[test]
    fn a_session_that_already_ended_is_not_recorded() {
        for (closed, shutting_down) in [(true, false), (false, true), (true, true)] {
            let state = shared();
            with_state(&state, |s| s.closed = closed);
            let calls = Arc::new(AtomicI64::new(0));
            let sink: SessionIdSink = {
                let calls = calls.clone();
                Arc::new(move |_| {
                    calls.fetch_add(1, Ordering::Relaxed);
                })
            };

            assert!(
                record_session_id(&state, &AtomicBool::new(shutting_down), Some(&sink), "T")
                    .is_ok(),
                "raised a normal end as a failure outcome (closed={closed}, shutdown={shutting_down})"
            );
            assert_eq!(
                calls.load(Ordering::Relaxed),
                0,
                "recorded into an ended session (closed={closed}, shutdown={shutting_down})"
            );
        }
    }

    /// The pair direction — if neither stands, it records. Without it the item above would be green even for "always skip".
    #[test]
    fn a_live_session_is_recorded() {
        let state = shared();
        let calls = Arc::new(AtomicI64::new(0));
        let sink: SessionIdSink = {
            let calls = calls.clone();
            Arc::new(move |_| {
                calls.fetch_add(1, Ordering::Relaxed);
            })
        };

        assert!(record_session_id(&state, &AtomicBool::new(false), Some(&sink), "T").is_ok());
        assert_eq!(calls.load(Ordering::Relaxed), 1);
    }

    /// ★Even if the record panics, it does not take the writer with it★ — an unwind would leave the link `Connecting` with `send_input`
    /// returning `Ok` (ADR-0190); `Err` sends the writer to the failure branch. Hook-less build only; production ends the process (ADR-0292).
    #[test]
    fn a_panicking_sink_becomes_a_failure_instead_of_unwinding() {
        let state = shared();
        let sink: SessionIdSink = Arc::new(|_| panic!("the record port blew up"));

        // ★The hook is not swapped bare-handed★ — it is process-global, so concurrent items would lose their own panic output or an overlapping
        //   span would leave the quiet hook forever; a helper that prevents both already exists.
        let out = engram_dashboard_command::testing::with_quiet_panic_hook(|| {
            record_session_id(&state, &AtomicBool::new(false), Some(&sink), "T")
        });

        assert!(
            out.is_err(),
            "the panic did not turn into a failure: {out:?}"
        );
        assert!(
            with_state(&state, |s| matches!(s.link, Link::Connecting)),
            "this function moved the link directly — the one that moves it is the caller (the writer's failure branch)"
        );
    }

    /// The pair in the opposite direction — `Ready` but without a thread id, the envelope cannot be built.
    #[test]
    fn nothing_is_sent_without_a_thread_id() {
        let h = harness();
        with_state(&h.state, |s| {
            s.link = Link::Ready;
            s.push_legacy(b"early");
        });
        assert!(
            with_state(&h.state, |s| take_turn_locked(s, &h.next_id)).is_none(),
            "turn/start went out without a thread id"
        );
    }

    /// The writer picks control lines **before** user turns — a refusal waiting behind the queue would stop the peer forever.
    #[test]
    fn the_writer_takes_control_lines_before_queued_input() {
        let h = harness();
        make_ready(&h.state, "T");
        with_state(&h.state, |s| {
            s.push_legacy(b"user turn");
            s.outbox.push_back("{\"id\":1,\"error\":{}}\n".to_string());
        });
        match next_job(&h.state, &h.next_id).expect("there is work") {
            Job::Line(l) => assert!(l.contains("error")),
            other => panic!(
                "the control line must come first: {}",
                matches!(other, Job::Sweep)
            ),
        }
    }

    // ── Deadline ──────────────────────────────────────────────────────────────

    #[test]
    fn a_request_with_no_answer_wakes_its_waiter_at_the_deadline() {
        let h = harness();
        let (tx, rx) = mpsc::channel();
        let _ = h
            .pending
            .register_at(0, Waiter::Handshake(tx), method::INITIALIZE, Instant::now());
        sweep_deadlines(&h.state, &h.pending, &h.reader.core);
        let msg = rx
            .recv_timeout(NOT_WOKEN)
            .expect("the deadline must wake the waiter")
            .expect_err("a deadline is a failure");
        assert!(msg.contains(method::INITIALIZE), "{msg}");
        assert_eq!(h.pending.len(), 0, "an expired waiting entry is taken away");
    }

    #[test]
    fn a_turn_start_deadline_ends_the_turn_and_releases_the_queue() {
        let h = harness();
        make_ready(&h.state, "T");
        activate(&h.state, 0, None);
        with_state(&h.state, |s| s.push_legacy(b"next"));
        let _ = h.pending.register_at(
            0,
            Waiter::TurnStart { seq: 0 },
            method::TURN_START,
            Instant::now(),
        );
        sweep_deadlines(&h.state, &h.pending, &h.reader.core);

        assert!(
            with_state(&h.state, |s| matches!(s.turn, TurnState::Idle)),
            "the deadline passed but the turn is in progress forever"
        );
        assert_eq!(
            failure_boundaries(&h.seen).len(),
            1,
            "folding it quietly leaves not a single signal on the screen or in the fact layer"
        );
        assert!(with_state(&h.state, |s| take_turn_locked(s, &h.next_id)).is_some());
    }

    #[test]
    fn a_deadline_that_has_not_passed_does_not_fire() {
        let h = harness();
        let (tx, rx) = mpsc::channel();
        let _ = h
            .pending
            .register(0, Waiter::Handshake(tx), method::INITIALIZE);
        sweep_deadlines(&h.state, &h.pending, &h.reader.core);
        assert_eq!(h.pending.len(), 1);
        assert!(rx.recv_timeout(NOT_WOKEN).is_err());
    }

    // ── Line reassembly ───────────────────────────────────────────────────────

    // hangul-ok-begin: multi-byte characters are the fixture of these two splitter tests
    #[test]
    fn a_line_split_across_chunks_is_reassembled() {
        let mut sp = LineSplitter::new();
        let mut lines: Vec<Vec<u8>> = Vec::new();
        sp.feed(br#"{"met"#, |l| lines.push(l.to_vec()));
        assert!(
            lines.is_empty(),
            "the line is not complete before the newline"
        );
        sp.feed("hod\":\"한글\"}\n{\"a\":1}\n".as_bytes(), |l| {
            lines.push(l.to_vec())
        });
        assert_eq!(lines.len(), 2);
        assert_eq!(
            String::from_utf8(lines[0].clone()).unwrap(),
            "{\"method\":\"한글\"}"
        );
    }

    #[test]
    fn a_multibyte_char_split_at_a_chunk_boundary_survives() {
        let mut sp = LineSplitter::new();
        let mut lines: Vec<Vec<u8>> = Vec::new();
        let payload = "가".as_bytes();
        sp.feed(&payload[..1], |l| lines.push(l.to_vec()));
        sp.feed(&payload[1..], |l| lines.push(l.to_vec()));
        sp.feed(b"\n", |l| lines.push(l.to_vec()));
        assert_eq!(lines.len(), 1);
        assert_eq!(String::from_utf8(lines[0].clone()).unwrap(), "가");
    }
    // hangul-ok-end

    #[test]
    fn an_oversize_line_is_dropped_and_the_next_line_survives() {
        let mut sp = LineSplitter::new();
        let mut lines: Vec<Vec<u8>> = Vec::new();
        let huge = vec![b'x'; MAX_LINE_BYTES + 1];
        sp.feed(&huge, |l| lines.push(l.to_vec()));
        assert!(lines.is_empty());
        // ★The tail of that line must not be parsed as a new line★ — everything up to the next newline is dropped.
        sp.feed(b"tail-of-the-huge-line\n{\"a\":1}\n", |l| {
            lines.push(l.to_vec())
        });
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert_eq!(String::from_utf8(lines[0].clone()).unwrap(), "{\"a\":1}");
    }

    // ── Real process: receipt meaning · teardown · child leaks ─────────────────────

    #[cfg(windows)]
    fn probe_spec(args: &[&str]) -> CommandSpec {
        CommandSpec {
            program: "cmd.exe".into(),
            args: args.iter().map(|s| s.to_string()).collect(),
            env: vec![],
            cwd: std::path::PathBuf::from("."),
        }
    }

    #[cfg(windows)]
    fn open_probe_owned(args: Vec<String>) -> (CodexAppServerTransport, Option<u32>) {
        let spec = CommandSpec {
            program: "cmd.exe".into(),
            args,
            env: vec![],
            cwd: std::path::PathBuf::from("."),
        };
        CodexAppServerTransport::open(
            &spec,
            true,
            None,
            ThreadOpen::Start(ThreadStartParams::default()),
            None,
            None,
        )
        .expect("open")
    }

    #[cfg(windows)]
    fn open_probe(args: &[&str]) -> (CodexAppServerTransport, Option<u32>) {
        CodexAppServerTransport::open(
            &probe_spec(args),
            true,
            None,
            ThreadOpen::Start(ThreadStartParams::default()),
            None,
            None,
        )
        .expect("open")
    }

    #[cfg(windows)]
    #[test]
    fn capabilities_reflect_the_injected_structured_flag_and_the_real_control_axis() {
        let (json, _) = open_probe(&["/c", "echo caps-probe"]);
        let caps = json.capabilities();
        assert!(caps.output.structured, "reports the injected value as is");
        assert!(!caps.output.terminal_bytes);
        assert!(!caps.control.resize);
        assert!(
            caps.control.interrupt,
            "this link actually emits turn/interrupt"
        );
        assert!(!caps.input.raw, "not a keystroke channel");
        assert!(caps.input.message);
        json.shutdown();

        let (plain, _) = CodexAppServerTransport::open(
            &probe_spec(&["/c", "echo caps-probe"]),
            false,
            None,
            ThreadOpen::Start(ThreadStartParams::default()),
            None,
            None,
        )
        .expect("open");
        assert!(
            !plain.capabilities().output.structured,
            "★no hard-coding★ — the link does not know what it carries"
        );
        plain.shutdown();
    }

    #[cfg(windows)]
    #[test]
    fn input_is_queued_before_ready_and_refused_over_the_bound() {
        let (t, _) = open_probe(&["/c", "ping", "-n", "30", "127.0.0.1"]);
        for i in 0..INPUT_QUEUE_LIMIT {
            assert!(
                t.send_input(InputEvent::Raw(format!("turn {i}").into_bytes()))
                    .is_ok(),
                "input in the handshake window is taken by the queue (item {i})"
            );
        }
        let over = t.send_input(InputEvent::Raw(b"one too many".to_vec()));
        assert!(
            matches!(over, Err(PtyError::WriteFailed(_))),
            "★over the cap is Err — it does not report a reduction with a short Ok★: {over:?}"
        );
        assert_eq!(
            with_state(&t.state, |s| s.pending.len()),
            INPUT_QUEUE_LIMIT,
            "a refused item went into the queue"
        );
        t.shutdown();
    }

    #[cfg(windows)]
    #[test]
    fn send_input_and_send_turn_share_one_queue() {
        let (t, _) = open_probe(&["/c", "ping", "-n", "30", "127.0.0.1"]);
        t.send_input(InputEvent::Raw(b"legacy".to_vec()))
            .expect("input in the handshake window is taken by the queue");
        t.send_turn(TurnInput {
            id: "u-1".into(),
            body: b"user".to_vec(),
            origin: InputOrigin::User,
        })
        .expect("the same queue takes it");
        let got = with_state(&t.state, |s| {
            s.pending
                .iter()
                .map(|p| (p.id.clone(), p.origin, p.body.clone()))
                .collect::<Vec<_>>()
        });
        assert_eq!(
            got,
            vec![
                (None, InputOrigin::Mail, b"legacy".to_vec()),
                (Some("u-1".to_string()), InputOrigin::User, b"user".to_vec()),
            ],
            "send_input comes in as a mail shape without an identifier"
        );
        t.shutdown();
    }

    #[cfg(windows)]
    #[test]
    fn input_after_the_link_is_down_is_an_error() {
        let (t, _) = open_probe(&["/c", "ping", "-n", "30", "127.0.0.1"]);
        with_state(&t.state, |s| s.link = Link::Down("handshake failed".into()));
        let out = t.send_input(InputEvent::Raw(b"hi".to_vec()));
        assert!(matches!(out, Err(PtyError::WriteFailed(_))), "{out:?}");
        t.shutdown();
    }

    #[cfg(windows)]
    #[test]
    fn interrupt_needs_a_turn_and_carries_both_ids() {
        let (t, _) = open_probe(&["/c", "ping", "-n", "30", "127.0.0.1"]);
        assert!(
            matches!(t.interrupt(), Err(PtyError::Unsupported(_))),
            "no envelope is built if there is no turn to interrupt"
        );
        make_ready(&t.state, "T-9");
        activate(&t.state, 0, Some("U-9"));
        t.interrupt().expect("it goes out if there is a turn");
        let lines = outbox_lines(&t.state);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0]["method"], method::TURN_INTERRUPT);
        assert_eq!(lines[0]["params"]["threadId"], "T-9");
        assert_eq!(lines[0]["params"]["turnId"], "U-9");
        t.shutdown();
    }

    #[cfg(windows)]
    #[test]
    fn shutdown_is_idempotent_and_leaves_no_child_behind() {
        let (t, pid) = open_probe(&["/c", "ping", "-n", "30", "127.0.0.1"]);
        let pid = pid.expect("pid");
        assert!(engram_dashboard_platform::process::pid_alive(pid));
        t.shutdown();
        t.shutdown();
        t.shutdown();
        assert!(
            !engram_dashboard_platform::process::pid_alive(pid),
            "the child is alive even after shutdown"
        );
        assert!(
            t.send_input(InputEvent::Raw(b"x".to_vec())).is_err(),
            "a closed link accepted input"
        );
    }

    /// ★A waiting entry registered after the link closed holds its thread until its deadline★ — the handshake issues two requests back to
    /// back, so a closing window really exists. If the table itself closes, that wait ends immediately.
    /// ★The **time** assertion below cannot measure the table closing★ — `shutdown()` already took stdin, so the write fails at once even
    /// without the closed marker. Only the two assertions above (new entry refused · `interrupt` refused) measure the marker.
    #[cfg(windows)]
    #[test]
    fn after_shutdown_a_request_fails_at_once_instead_of_waiting_out_the_deadline() {
        let (t, _) = open_probe(&["/c", "ping", "-n", "30", "127.0.0.1"]);
        make_ready(&t.state, "T-1");
        activate(&t.state, 0, Some("U-1"));
        t.shutdown();

        assert!(
            !t.pending.register(77, Waiter::Fire, method::TURN_INTERRUPT),
            "a closed waiting table took a new entry"
        );
        assert!(t.interrupt().is_err(), "a closed link built an envelope");

        let start = Instant::now();
        let out: Result<Value, String> = request_blocking(
            &t.stdin,
            &t.state,
            &t.pending,
            &t.next_id,
            method::TURN_INTERRUPT,
            &TurnInterruptParams {
                thread_id: "T-1".into(),
                turn_id: "U-1".into(),
            },
            REQUEST_DEADLINE,
        );
        assert!(out.is_err());
        assert!(
            start.elapsed() < Duration::from_secs(1),
            "★it waited out the whole {}-second deadline★ — a thread nobody joins hangs that long",
            REQUEST_DEADLINE.as_secs()
        );
    }

    /// ★Even if the peer ends on its own, the writer thread must end★ — else it stays holding the `Arc`s of core · stdin · the waiting
    /// table (one session's memory); `shutdown()` is not called on this path (the reaper only removes the session from the registry).
    /// ★The only item that actually runs **both** loops★ — the other link items call `handle_line` · `next_job` · `sweep_deadlines` directly.
    #[cfg(windows)]
    #[test]
    fn the_writer_thread_ends_when_the_peer_exits_on_its_own() {
        let (t, _) = open_probe(&["/c", "echo", "{}"]);
        let (core, _seen) = core_with_sink();
        t.start(core.clone());

        core.join_pump(Duration::from_secs(10));

        await_writer_end(
            &t,
            "the peer ended on its own but the writer thread did not end — one thread per agent remains forever",
        );
        assert!(
            with_state(&t.state, |s| s.closed),
            "EOF handling did not set the closed marker"
        );
    }

    /// ★The same marker must survive a reader **panic** too★ — `contain` wraps the loop from outside, so tail cleanup is skipped by unwind;
    /// without the marker the writer spins forever holding core · stdin · the waiting table (the half the EOF item cannot see). The unwind is
    /// produced by **a subscriber panicking inside `core.emit`** — the only way this file reaches it without the real codex.
    #[cfg(windows)]
    #[test]
    fn the_closing_mark_survives_a_reader_panic() {
        let probe = NotificationFile::new("panic");
        let (t, _) = open_probe_owned(probe.args());
        *t.decoder.lock().unwrap_or_else(|p| p.into_inner()) = Some(Box::new(EmittingDecoder));

        let core = Arc::new(OutputCore::new(
            AgentId::new_v4(),
            1,
            Arc::new(NoopStatus),
            TurnWiring::detached(),
        ));
        core.subscribe(Arc::new(PanickingSink(SinkId::new_v4())));
        t.start(core.clone());
        core.join_pump(Duration::from_secs(10));

        assert!(
            with_state(&t.state, |s| s.closed),
            "the reader ended with a panic and the closed marker did not stand"
        );
        await_writer_end(&t, "the writer thread did not end after the reader panic");
        drop(probe);
    }

    /// A contained reader panic folds into `Failed` and `finish` runs outside the marking — exactly the production reader closure.
    // ADR-0292
    #[cfg(windows)]
    #[test]
    fn a_contained_reader_panic_fails_the_agent_and_finish_runs_unmarked() {
        use crate::transport::pump_marks::{self, Marks};

        let probe = NotificationFile::new("marks");
        let (t, _) = open_probe_owned(probe.args());
        *t.decoder.lock().unwrap_or_else(|p| p.into_inner()) = Some(Box::new(EmittingDecoder));
        let marks = Arc::new(Marks::default());
        let core = pump_marks::core(&marks);
        t.start(core.clone());
        core.join_pump(Duration::from_secs(10));

        pump_marks::assert_contained_body_and_unmarked_finish(&marks);
        t.shutdown();
        drop(probe);
    }

    /// ★The control queue must go out even while the handshake waits for an answer★ — the writer is the only drainer, so parking entirely
    /// would hold a refusal the reader queued until the deadline, and a peer waiting for it would deadlock with us.
    #[cfg(windows)]
    #[test]
    fn the_handshake_keeps_draining_the_control_queue_while_it_waits() {
        let (t, _) = open_probe(&["/c", "ping", "-n", "30", "127.0.0.1"]);
        let stdin = t.stdin.clone();
        let state = t.state.clone();
        let pending = t.pending.clone();
        let next_id = t.next_id.clone();

        let waiter = std::thread::spawn(move || -> Result<Value, String> {
            request_blocking(
                &stdin,
                &state,
                &pending,
                &next_id,
                method::INITIALIZE,
                &serde_json::json!({}),
                REQUEST_DEADLINE,
            )
        });

        // While waiting for the answer, the reader puts in one refusal.
        with_state(&t.state, |s| {
            s.outbox
                .push_back("{\"id\":1,\"error\":{\"code\":-32601}}\n".to_string())
        });

        let deadline = Instant::now() + Duration::from_secs(10);
        while with_state(&t.state, |s| !s.outbox.is_empty()) {
            assert!(
                Instant::now() < deadline,
                "★the handshake blocked the control queue★ — the peer does not receive that answer until the deadline"
            );
            std::thread::sleep(Duration::from_millis(20));
        }

        // Ends that thread by giving the answer (so the whole deadline is not waited out).
        let entry = t
            .pending
            .take(&RequestId::Num(0))
            .expect("the waiting entry of the request we issued");
        match entry.waiter {
            Waiter::Handshake(tx) => tx.send(Ok(serde_json::json!({}))).expect("send"),
            _ => panic!("must be the handshake waiting entry"),
        }
        waiter.join().expect("waiter thread").expect("response");
        t.shutdown();
    }

    /// ★The order of the sliced wait is the contract — measure the deadline **first**, then write one line★: in a slice whose deadline passed
    /// it returns without attempting a write (flipped, that write hangs on the stdin lock). ★Cannot measure★ a write that blocks in a live
    /// slice (only `shutdown()`'s kill gets out) — do not quote this as that guarantee.
    #[cfg(windows)]
    #[test]
    fn an_expired_slice_returns_without_attempting_another_write() {
        // The first request write must succeed (small line), so only the deadline is short; by the time one slice (500ms) runs it has passed.
        const BUDGET: Duration = Duration::from_millis(50);
        let (t, _) = open_probe(&["/c", "ping", "-n", "30", "127.0.0.1"]);

        // One control line is put up so that the flipped order actually attempts a write.
        with_state(&t.state, |s| {
            s.outbox
                .push_back("{\"id\":1,\"error\":{\"code\":-32601}}\n".to_string())
        });

        let stdin = t.stdin.clone();
        let state = t.state.clone();
        let pending = t.pending.clone();
        let next_id = t.next_id.clone();
        let waiter = std::thread::spawn(move || -> Result<Value, String> {
            request_blocking(
                &stdin,
                &state,
                &pending,
                &next_id,
                method::INITIALIZE,
                &serde_json::json!({}),
                BUDGET,
            )
        });

        // Hold stdin **after** the first request write ends — the peer does not read stdin, so this write hangs forever holding the lock.
        std::thread::sleep(Duration::from_millis(100));
        let filler = t.stdin.clone();
        let filling = std::thread::spawn(move || {
            let _ = write_line(&filler, &"x".repeat(8 * 1024 * 1024));
        });

        let hard_stop = Instant::now() + Duration::from_secs(15);
        while !waiter.is_finished() {
            assert!(
                Instant::now() < hard_stop,
                "★a slice past its deadline attempted the write first★ — it cannot get that lock, so the deadline is not evaluated"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
        let out = waiter.join().expect("waiter thread");
        assert!(
            out.is_err(),
            "ended as success although no answer ever came: {out:?}"
        );

        t.shutdown();
        let _ = filling.join();
    }

    /// ★Failing to send a refusal because the control queue is full must show on screen★ — logged only, the peer waits forever with no
    /// signal on this side. The sibling event (`turn/start` deadline) already shows on screen.
    #[test]
    fn a_full_control_queue_drops_a_protocol_obligation_visibly() {
        let mut h = harness();
        with_state(&h.state, |s| {
            for i in 0..OUTBOX_LIMIT {
                s.outbox.push_back(format!("{{\"filler\":{i}}}\n"));
            }
        });
        h.reader
            .handle_line(br#"{"id":42,"method":"item/approval/request"}"#);

        assert_eq!(
            with_state(&h.state, |s| s.outbox.len()),
            OUTBOX_LIMIT,
            "pushed in past the cap"
        );
        let seen = h.seen.lock().unwrap();
        assert!(
            seen.iter()
                .any(|e| matches!(e, OutputEvent::Error(m) if m.contains("control queue"))),
            "the fact that the refusal could not be sent did not show on screen: {seen:?}"
        );
    }

    /// ★The record call must come **before** the gate opens★ — why the one-verb port exists ([`crate::backend::SessionIdSink`]). Measured in
    /// the source: the port is `Fn(&str)` and cannot see link state. The other half is
    /// [`tests::nothing_is_sent_before_the_link_is_ready_even_when_the_thread_id_is_known`]. ★Pins only "**handed** to the port before the
    /// gate" (ADR-0226)★ — persistence-before-first-turn is covered by the latch tests and `manager`'s wiring test.
    #[test]
    fn the_session_id_is_recorded_before_the_gate_opens() {
        let body = writer_loop_code();

        let recorded = body.find("record_session_id(").expect(
            "there is no `record_session_id(` call in writer_loop — this item's premise is stale",
        );
        // ★The anchor moved from `Link::Ready` to `open_gate(`★ — the line that sets the gate moved into that function, so the `Link::Ready`
        //   left in this body became **only comments**; left as is the item would measure a comment position and pass quietly.
        let gate_open = body
            .find("open_gate(")
            .expect("there is no `open_gate(` call in writer_loop — this item's premise is stale");

        assert!(
            recorded < gate_open,
            "the record call stands after the gate (`Link::Ready`) — a turn can go out before it is recorded, and then the one-\
             verb port guarantees nothing (a second verb becomes necessary)"
        );
    }

    /// Only the code of the `writer_loop` body **with comments stripped** — the shared lens of the items measuring order · door choice in the source.
    ///
    /// ★Stripping comments is why this exists★: this file's comments quote the names they guard, so a raw scan lets **one comment line
    ///   pose as a real call** — it once did, two items looking for `Link::Ready` were green measuring the **comments** left in the body.
    fn writer_loop_code() -> String {
        let src = include_str!("transport.rs");
        let production = src.split("mod tests {").next().expect("production section");
        production
            .split("fn writer_loop(")
            .nth(1)
            .expect("writer_loop body")
            .lines()
            .filter(|l| !l.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join(
                "
",
            )
    }

    // ── History restore paging (ADR-0203) ─────────────────────────────────────────────

    fn entry(turn: &str, item: Value) -> ThreadItemEntry {
        ThreadItemEntry {
            turn_id: turn.to_string(),
            item,
        }
    }

    fn agent_message(id: &str, text: &str) -> Value {
        serde_json::json!({"type": "agentMessage", "id": id, "text": text})
    }

    fn page(data: Vec<ThreadItemEntry>, next: Option<&str>) -> ThreadItemsListResponse {
        ThreadItemsListResponse {
            data,
            next_cursor: next.map(String::from),
        }
    }

    fn far_deadline() -> Instant {
        Instant::now() + Duration::from_secs(60)
    }

    fn texts(events: &[OutputEvent]) -> Vec<String> {
        events
            .iter()
            .filter_map(|e| match e {
                OutputEvent::TextDelta { text, .. } => Some(text.clone()),
                _ => None,
            })
            .collect()
    }

    /// ★Pages come newest first and the screen needs **time order**★ — that flip is this path's only order contract (measured 2026-09-16,
    /// real 0.154.0: the first page's `data[0]` of `sortDirection: desc` is the last assistant response, the last page ends with the first
    /// `userMessage`). ★Also measures the request shape★ — a wrong direction · cursor continuation stands the screen upside down or refetches
    /// the same page forever.
    #[test]
    fn history_pages_backwards_and_lands_in_time_order() {
        let mut asked: Vec<(String, Option<String>)> = Vec::new();
        let events = collect_history("th-1", "c0".to_string(), far_deadline(), |params, _| {
            asked.push((params.thread_id.clone(), params.cursor.clone()));
            assert!(
                matches!(params.sort_direction, Some(SortDirection::Desc)),
                "without the backward direction, it cannot walk from the newest"
            );
            assert_eq!(params.limit, Some(HISTORY_PAGE_LIMIT));
            Ok(match params.cursor.as_deref() {
                Some("c0") => page(
                    vec![
                        entry("t1", agent_message("m3", "third")),
                        entry("t1", agent_message("m2", "second")),
                    ],
                    Some("c1"),
                ),
                _ => page(vec![entry("t1", agent_message("m1", "first"))], None),
            })
        });

        assert_eq!(
            asked,
            vec![
                ("th-1".to_string(), Some("c0".to_string())),
                ("th-1".to_string(), Some("c1".to_string())),
            ],
            "it must keep asking with the cursor the response gave, on the thread the handshake opened"
        );
        assert_eq!(texts(&events), vec!["first", "second", "third"]);
    }

    /// ★Exactly one close per turn★ — without closing, the restored slot's waiting indicator spins forever; closing twice piles up empty
    /// separators. Closing the last turn too is half of this item.
    #[test]
    fn every_history_turn_is_closed_exactly_once() {
        let events = collect_history("th", "c".to_string(), far_deadline(), |_, _| {
            Ok(page(
                vec![
                    entry("t2", agent_message("m2", "later")),
                    entry("t1", agent_message("m1", "earlier")),
                ],
                None,
            ))
        });

        let shape: Vec<&str> = events
            .iter()
            .map(|e| match e {
                OutputEvent::TextDelta { .. } => "text",
                OutputEvent::MessageDone { .. } => "close",
                _ => "other",
            })
            .collect();
        assert_eq!(shape, vec!["text", "close", "text", "close"]);
        let closed: Vec<Option<String>> = events
            .iter()
            .filter_map(|e| match e {
                OutputEvent::MessageDone { turn_id, .. } => Some(turn_id.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(
            closed,
            vec![Some("t1".to_string()), Some("t2".to_string())],
            "the boundary carries that turn's id and stands in time order"
        );
    }

    /// ★If there is nothing to carry over, **nothing is emitted**★ — a list of only boundaries would create separators in an empty slot.
    /// Variants live drops (reasoning etc.) get the same decision in history.
    #[test]
    fn a_history_of_nothing_translatable_produces_no_events() {
        let events = collect_history("th", "c".to_string(), far_deadline(), |_, _| {
            Ok(page(
                vec![
                    entry("t1", serde_json::json!({"type": "reasoning", "id": "r1"})),
                    entry(
                        "t1",
                        serde_json::json!({"type": "subAgentActivity", "id": "s1"}),
                    ),
                ],
                None,
            ))
        });
        assert!(
            events.is_empty(),
            "a list of only boundaries came out: {events:?}"
        );
    }

    /// ★★Even if one page fails, it is **not an activation failure**★★ — the screen received so far is carried and closed normally
    /// (ADR-0203).
    #[test]
    fn a_failed_page_keeps_everything_that_came_before_it() {
        let mut calls = 0;
        let events = collect_history("th", "c0".to_string(), far_deadline(), |_, _| {
            calls += 1;
            if calls == 1 {
                Ok(page(
                    vec![entry("t1", agent_message("m1", "kept"))],
                    Some("c1"),
                ))
            } else {
                Err("the peer refused".to_string())
            }
        });
        assert_eq!(calls, 2);
        assert_eq!(texts(&events), vec!["kept"]);
        assert!(
            matches!(events.last(), Some(OutputEvent::MessageDone { .. })),
            "even if cut midway, the last turn must be closed: {events:?}"
        );
    }

    /// ★★The ceiling is the ring★★ (ADR-0203) — receiving beyond 2MiB is pure cost (the ring drops from the front), and the decision must
    /// use the axis the ring counts ([`estimate_cost_bytes`]).
    #[test]
    fn history_stops_once_the_replay_ring_is_full() {
        let big = "x".repeat(64 * 1024);
        let mut calls = 0;
        let events = collect_history("th", "c".to_string(), far_deadline(), |_, _| {
            calls += 1;
            Ok(page(
                (0..HISTORY_PAGE_LIMIT)
                    .map(|i| entry("t1", agent_message(&format!("m{calls}-{i}"), &big)))
                    .collect(),
                // The cursor does not end — only the ceiling must stop it.
                Some("more"),
            ))
        });

        let weight: usize = events.iter().map(estimate_cost_bytes).sum();
        // ★Stops **before** exceeding — do not revert to "stop after exceeding"★: measuring after putting always exceeds by one entry, which
        //   the ring drops on landing (the mismatch tying the ceiling to the ring constant removes).
        assert!(
            weight <= REPLAY_MAX_BYTES,
            "received past the ceiling: {weight}B > {REPLAY_MAX_BYTES}B"
        );
        assert!(
            weight + 64 * 1024 >= REPLAY_MAX_BYTES,
            "stopped far short of filling the ring: {weight}B"
        );
        assert_eq!(
            calls, 2,
            "kept asking for pages after the ring filled (wasted round trips) — {calls} times"
        );
    }

    /// ★★The ceiling accounting must count **the synthesized boundaries too**★★ — uncounted, a history where every item splits a turn has
    /// twice the counted events and the ring (4096 entries) drops half on landing — the **oldest** half, cutting off the head of the
    /// restored conversation.
    #[test]
    fn the_ceiling_counts_the_boundaries_it_synthesizes() {
        let mut n = 0usize;
        let events = collect_history("th", "c".to_string(), far_deadline(), |_, _| {
            let data = (0..HISTORY_PAGE_LIMIT)
                .map(|_| {
                    n += 1;
                    entry(&format!("t{n}"), agent_message(&format!("m{n}"), "x"))
                })
                .collect();
            Ok(page(data, Some(&format!("c{n}"))))
        });

        assert!(
            events.len() <= REPLAY_MAX_EVENTS,
            "received past the ring count ceiling: {} > {REPLAY_MAX_EVENTS}",
            events.len()
        );
        let texts = events
            .iter()
            .filter(|e| matches!(e, OutputEvent::TextDelta { .. }))
            .count();
        let closes = events
            .iter()
            .filter(|e| matches!(e, OutputEvent::MessageDone { .. }))
            .count();
        assert_eq!(
            texts, closes,
            "one boundary per turn — the premise of the accounting broke"
        );
        assert!(
            events.len() > REPLAY_MAX_EVENTS - 2 * HISTORY_PAGE_LIMIT as usize,
            "stopped far short of filling the ceiling: {}",
            events.len()
        );
    }

    /// ★★`turnId` has no wire cap — it is filtered before holding★★: the line cap bounds **one** page, not the accumulation, so items with
    /// long ids on every page reach hundreds of MiB (25 × 160KiB fits 4MiB on one page, but up to the count ceiling it is ~650MiB).
    /// ★★**Different turns must stay two**★★ — folding all over-cap ids into one `None` slot catches the memory but **renders two
    /// conversations as one turn**. So distinction comes from the original comparison with the previous item (O(1)) and the filtered id is
    /// only **display**; this measures both axes — two boundaries, no id carried.
    #[test]
    fn an_oversized_turn_id_is_filtered_before_it_is_retained() {
        let huge_a = "a".repeat(MAX_TURN_ID_BYTES + 1);
        let huge_b = "b".repeat(MAX_TURN_ID_BYTES + 1);
        let events = collect_history("th", "c".to_string(), far_deadline(), |_, _| {
            Ok(page(
                vec![
                    entry(&huge_b, agent_message("m2", "later")),
                    entry(&huge_a, agent_message("m1", "earlier")),
                ],
                None,
            ))
        });

        let closes: Vec<Option<String>> = events
            .iter()
            .filter_map(|e| match e {
                OutputEvent::MessageDone { turn_id, .. } => Some(turn_id.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(
            closes,
            vec![None, None],
            "two turns over the cap folded into one turn — different conversations render as one chunk: {events:?}"
        );
        assert_eq!(
            texts(&events),
            vec!["earlier", "later"],
            "all the content remains"
        );
        let same = collect_history("th", "c".to_string(), far_deadline(), |_, _| {
            Ok(page(
                vec![
                    entry(&huge_a, agent_message("m2", "later")),
                    entry(&huge_a, agent_message("m1", "earlier")),
                ],
                None,
            ))
        });
        assert_eq!(
            same.iter()
                .filter(|e| matches!(e, OutputEvent::MessageDone { .. }))
                .count(),
            1,
            "split the same turn into two: {same:?}"
        );
    }

    /// ★The same page is not refetched from a peer that returns the same cursor★ — else the conversation is carried several times and the
    /// duplicates push the real history out of the ring; the page cap bounds the loop but not this contamination.
    #[test]
    fn a_repeating_cursor_stops_instead_of_duplicating_history() {
        let mut calls = 0usize;
        let events = collect_history("th", "c".to_string(), far_deadline(), |_, _| {
            calls += 1;
            Ok(page(
                vec![entry("t1", agent_message("m1", "once"))],
                Some("same"),
            ))
        });
        assert_eq!(
            calls, 2,
            "did not recognize the repeating cursor — {calls} times"
        );
        assert_eq!(
            texts(&events),
            vec!["once", "once"],
            "the first and second cursors differ, so up to two pages is normal: {events:?}"
        );
    }

    /// ★It does not hang forever on a cursor that never ends★ — the budget usually cuts first, but for a peer answering immediately only
    /// this backstop remains.
    #[test]
    fn an_unending_cursor_stops_at_the_page_cap() {
        let mut calls = 0usize;
        let events = collect_history("th", "c".to_string(), far_deadline(), |_, _| {
            calls += 1;
            Ok(page(
                vec![entry(
                    "t1",
                    serde_json::json!({"type": "reasoning", "id": "r"}),
                )],
                // ★A **different** cursor every time★ — the same value would make the repeat guard cut first and the item would measure that guard.
                Some(&format!("c{calls}")),
            ))
        });
        assert_eq!(calls, MAX_HISTORY_PAGES);
        assert!(events.is_empty());
    }

    /// ★★If the connection ended while fetching history, the gate is not opened★★ — history treats that error as "a page did not come"
    /// (right), so a gate that did not look at the state would let **an already dead link stand as `Ready` and be delivered as a successful
    /// activation**; the manager would carry a corpse as a live agent.
    #[test]
    fn a_gate_does_not_open_over_a_link_that_died_while_hydrating() {
        let state = shared();
        with_state(&state, |s| {
            s.link = Link::Down("the stream ended".to_string());
            s.closed = true;
        });
        let verdict = open_gate(&state, "th".to_string());
        assert_eq!(verdict, Err("the stream ended".to_string()));
        with_state(&state, |s| {
            assert!(
                matches!(s.link, Link::Down(_)),
                "overwrote the downed connection"
            );
            assert!(
                s.thread_id.is_none(),
                "planted a thread id into a dead link"
            );
        });
    }

    /// ★★The stream end **writes the state first, then wakes**★★ — the reverse lets the writer return through the death's own window and
    /// **deliver `Ready` to an ended link**. ★Why by **source order**★: a wake-first version run 200 times with an observer thread was **never
    /// caught** (the waker practically always wins the race), so that harness was green with the wrong order; for "the order of two statements"
    /// measuring it directly is honest. ★The share measured by running is the sibling below★ — "did both happen".
    // ADR-0203
    #[test]
    fn a_stream_end_writes_the_state_before_it_wakes_the_waiters() {
        let src = include_str!("transport.rs");
        let production = src.split("mod tests {").next().expect("production section");
        let body = production
            .split("impl Drop for ReaderExit {")
            .nth(1)
            .expect("ReaderExit::drop body")
            .lines()
            .filter(|l| !l.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");

        let wrote_down = body
            .find("s.closed = true;")
            .expect("`s.closed = true;` is not in ReaderExit::drop — this item's premise is stale");
        let woke = body.find("self.pending.close()").expect(
            "`self.pending.close()` is not in ReaderExit::drop — this item's premise is stale",
        );
        assert!(
            wrote_down < woke,
            "the waking stands before the state write — the woken writer reads the live state and delivers Ready \
             to a dead link"
        );
    }

    /// Measures by running **whether both happened** — removing either is caught here (no waking = permanent hang; no state write = a dead
    /// link stays alive).
    #[test]
    fn a_stream_end_both_wakes_the_waiters_and_marks_the_link_down() {
        let state = shared();
        let pending = Arc::new(Pending::default());
        let (tx, rx) = mpsc::channel();
        assert!(pending.register(1, Waiter::Handshake(tx), method::THREAD_ITEMS_LIST));

        drop(ReaderExit {
            state: state.clone(),
            pending: pending.clone(),
            agent: AgentId::new_v4(),
        });

        assert!(
            matches!(rx.try_recv(), Ok(Err(_))),
            "did not wake the waiting entries with an error — a permanent hang"
        );
        with_state(&state, |s| {
            assert!(
                matches!(s.link, Link::Down(_)),
                "did not take the connection down"
            );
            assert!(s.closed, "did not set the closed marker");
        });
    }

    #[test]
    fn a_gate_does_not_open_over_a_closed_link() {
        let state = shared();
        with_state(&state, |s| s.closed = true);
        assert!(open_gate(&state, "th".to_string()).is_err());
        with_state(&state, |s| {
            assert!(!matches!(s.link, Link::Ready));
        });
    }

    #[test]
    fn a_healthy_link_opens_the_gate_and_carries_the_thread_id() {
        let state = shared();
        assert_eq!(open_gate(&state, "th-9".to_string()), Ok(()));
        with_state(&state, |s| {
            assert!(matches!(s.link, Link::Ready));
            assert_eq!(s.thread_id.as_deref(), Some("th-9"));
        });
    }

    /// ★If one round trip uses up the budget, **the next is not asked**★ — where this loop enforces the shared budget; the overrun **inside**
    /// one request (one 500ms slice) it cannot block.
    ///
    /// ★The cap until `Ready` is 10s + 2×0.5s = 11s★: the unit counting overruns is the [`request_blocking`] **call** — the success branch
    /// does not re-check the deadline and even with budget 0 waits out the next slice, so [`handshake`]'s two round trips can each overrun by
    /// one slice. Paging's first check sees remainder 0 and cuts (no third overrun) — measured here. ★One more handshake round trip adds
    /// that much★. The 15s backstop stays above it.
    #[test]
    fn a_page_that_burns_the_budget_is_the_last_one() {
        let mut calls = 0usize;
        let deadline = Instant::now() + Duration::from_millis(80);
        let events = collect_history("th", "c0".to_string(), deadline, |_, _| {
            calls += 1;
            std::thread::sleep(Duration::from_millis(160));
            Ok(page(
                vec![entry("t1", agent_message("m1", "only"))],
                Some("c1"),
            ))
        });
        assert_eq!(
            calls, 1,
            "asked again after the budget ran out — {calls} times"
        );
        assert_eq!(
            texts(&events),
            vec!["only"],
            "what was received is carried as is"
        );
    }

    /// ★If the budget is already gone, **it never asks**★ — the handshake barely succeeded using it up; one more round trip would push
    /// `Ready` behind the manager's backstop.
    #[test]
    fn an_exhausted_budget_asks_for_nothing() {
        let mut calls = 0;
        let events = collect_history(
            "th",
            "c".to_string(),
            Instant::now() - Duration::from_secs(1),
            |_, _| {
                calls += 1;
                Ok(page(vec![entry("t1", agent_message("m", "never"))], None))
            },
        );
        assert_eq!(calls, 0);
        assert!(events.is_empty());
    }

    /// ★With nothing to resume there is no round trip at all★ — an incarnation from `thread/start` (new conversation) falls here.
    #[test]
    fn a_thread_without_a_history_cursor_never_pages() {
        let state = shared();
        let events = hydrate_history(
            &Mutex::new(None),
            &state,
            &Pending::default(),
            &AtomicI64::new(0),
            &Opened {
                thread_id: "th".to_string(),
                items_backwards_cursor: None,
                floor: Floor::Below,
            },
            far_deadline(),
        );
        assert!(events.is_empty());
    }

    /// ★★History must go into the ring **before** the gate opens — this path's order contract★★: gate first, and queued user turns go out
    /// and their responses enter the ring before the unraised history, attaching the restored conversation **after** the new answer. This
    /// replaces what ADR-0079's seed-before-publish kept (it cannot be pulled ahead of registry registration — the history exists only after
    /// the handshake).
    /// ★Measured in the source for the same reason as the sibling★ — running needs a child and a peer, and the harness would measure itself.
    /// Sibling = [`tests::the_session_id_is_recorded_before_the_gate_opens`].
    #[test]
    fn the_history_is_emitted_before_the_gate_opens() {
        let body = writer_loop_code();

        let hydrated = body.find("hydrate_history(").expect(
            "there is no `hydrate_history(` call in writer_loop — this item's premise is stale",
        );
        let gate_open = body
            .find("open_gate(")
            .expect("there is no `open_gate(` call in writer_loop — this item's premise is stale");
        assert!(
            hydrated < gate_open,
            "the history restore stands after the gate — the restored conversation attaches after the new answer"
        );

        // ★The entry door must **fan out too**★: the session is already in the registry, so with `OutputCore::seed` (ring only, no fanout) a
        //   subscriber attached in between has replayed the empty ring and **never sees** the past screen (`seed` sufficed for ADR-0079 because
        //   that was **before** registry registration).
        // ★It must be **the chunk door**★ — the item-by-item door keeps fanout but lets the reader's live lines take seqs between calls and split
        //   the restored conversation ([`crate::output_core::OutputCore::emit_batch_without_turn_observation`]).
        assert!(
            body.contains("emit_batch_without_turn_observation"),
            "history goes in through the item-by-item door — fanout works but the middle of the chunk is split"
        );
        assert!(
            !body.contains("core.seed("),
            "`seed` loses fanout at this spot (the doc above)"
        );
    }

    /// ★★The handshake and the history **share one budget**★★ — separate budgets could reach two budgets before the connection stands,
    /// exceed the manager's backstop and **decide a successful resume a failure**. Concretely: `writer_loop` starts the clock once and passes
    /// the same value to both.
    #[test]
    fn the_handshake_and_the_history_share_one_budget() {
        let code = writer_loop_code();

        // ★Measured by **the number of appearances of the name**, not call shape★ — `cargo fmt` decides argument-list line breaks, so whole-call
        //   comparison would break on a formatting change (it did).
        assert_eq!(
            code.matches("HANDSHAKE_BUDGET").count(),
            1,
            "writer_loop must start the budget clock only once — a second clock is a second budget"
        );
        assert_eq!(
            code.matches("link_deadline").count(),
            3,
            "that deadline must be bound once and handed to **two consumers** (handshake · history) each"
        );
    }

    /// ★If the user turned it off, **the connection outcome is not delivered**★ — the root of defect ③: kill wakes the handshake with `Err`,
    /// and delivering that would record the user's cancellation as a resume failure and run cleanup once more.
    #[test]
    fn a_shutdown_suppresses_the_link_delivery() {
        let seen: Arc<Mutex<Vec<LinkResolution>>> = Arc::new(Mutex::new(Vec::new()));
        let sink: Option<LinkSink> = {
            let seen = seen.clone();
            Some(Arc::new(move |r| {
                seen.lock().expect("seen poisoned").push(r)
            }))
        };

        let quiet = AtomicBool::new(false);
        deliver_link(&quiet, &sink, LinkResolution::Ready);
        assert_eq!(
            seen.lock().expect("seen poisoned").len(),
            1,
            "normally it must be delivered — if this item's control group dies, the assertion below becomes hollow"
        );

        let killed = AtomicBool::new(true);
        deliver_link(
            &killed,
            &sink,
            LinkResolution::Failed {
                reason: "the waiting table closed".into(),
            },
        );
        assert_eq!(
            seen.lock().expect("seen poisoned").len(),
            1,
            "a failure the user's kill made was delivered — then the user's cancellation is recorded as a resume failure and cleanup runs once more on top of that"
        );
    }

    /// ★The link's handshake cap must be **smaller** than the manager's liveness backstop★: the cap **makes a reason** (writes the peer's
    /// silence into `Down`); the backstop **gives up without one**. A smaller backstop would cover a reasoned outcome with a reasonless one.
    /// ★Two constants in two files — the compiler cannot see the relation★, so it is measured here.
    #[test]
    fn the_handshake_budget_expires_before_the_managers_backstop() {
        assert!(
            HANDSHAKE_BUDGET < crate::manager::LINK_RESOLUTION_BACKSTOP,
            "the handshake cap ({HANDSHAKE_BUDGET:?}) is at least the manager backstop ({:?}) — then a give-up without a reason covers a refusal with a reason, and the cause remains nowhere",
            crate::manager::LINK_RESOLUTION_BACKSTOP
        );
    }

    /// ★Pins the number of places in the production section that take the stdin lock **blocking**★.
    ///
    /// The safety of the handshake failure branch of [`writer_loop`] taking that lock rests on "only those two take it blocking in the
    /// production graph, and [`write_line`] is called only by the writer"; a quiet third would make that stale — and the result is **a
    /// deadlock**, not a compile error. ★Counts, not positions★; if it grew, fix the number **after deciding which thread calls the new
    /// place**. Rule: [`count_blocking_stdin_locks`]. Comment lines are not counted (trailing comments can over-count — red direction, fine).
    /// ★The test section is out of range★: it holds the lock on purpose in
    /// [`tests::shutdown_completes_even_if_a_write_blocks_on_a_full_pipe`], harmless because it never calls [`AgentTransport::start`].
    #[test]
    fn the_production_blocking_stdin_locks_are_counted() {
        let src = include_str!("transport.rs");
        let production = src.split("mod tests {").next().expect("production section");
        let code: String = production
            .lines()
            .filter(|l| !l.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");

        assert_eq!(
            count_blocking_stdin_locks(&code),
            2,
            "the number of blocking stdin lock acquisitions in the production section changed — `write_line` and **the handshake failure branch** of `writer_loop` (not only record failure but round-trip failure · refusal pass the same place) must be all of them. If it grew, first decide on which thread the new place is called: if a place appears that waits for that lock blocking while the writer hangs in `write_all`, it is a deadlock"
        );
    }

    /// Counts blocking stdin lock acquisition places — two forms together.
    ///
    /// ① `.lock()` after `stdin`, skipping whitespace (incl. newlines). ② `stdin` in the argument of a `lock(` whose preceding character is
    /// not an identifier character — `sync::lock(` · full paths · a bare `lock(` · `Mutex::lock(`; argument = up to the matching
    /// parenthesis, else that line and the next two. Today's two production places are form ② (ADR-0275).
    /// Not counted: `try_lock(` (does not wait — [`AgentTransport::shutdown`]) · `.lock()` with an empty argument (① covers it).
    /// ★Do not count only one form★ (a third place in the other would pass green); ★over-counting is intended★ (`stdin_buf`): red calls
    /// for a decision, under-counting lets a third pass. ★Uncatchable forms remain★: receivers not ending in `stdin`
    /// (`stdin.as_ref().lock()` · `(*stdin).lock()`).
    fn count_blocking_stdin_locks(src: &str) -> usize {
        let mut found = 0;
        let mut from = 0;
        while let Some(rel) = src[from..].find("stdin") {
            let after = from + rel + "stdin".len();
            if src[after..].trim_start().starts_with(".lock()") {
                found += 1;
            }
            from = after;
        }
        for (at, call) in src.match_indices("lock(") {
            let glued = src[..at]
                .chars()
                .next_back()
                .is_some_and(|c| c == '_' || c.is_alphanumeric());
            if !glued && lock_argument(&src[at + call.len()..]).contains("stdin") {
                found += 1;
            }
        }
        found
    }

    /// `rest` = right after `lock(`. Returns up to before the matching closing parenthesis, else that line and the next two.
    fn lock_argument(rest: &str) -> &str {
        let mut depth = 1usize;
        for (i, b) in rest.bytes().enumerate() {
            match b {
                b'(' => depth += 1,
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        return &rest[..i];
                    }
                }
                _ => {}
            }
        }
        let end = rest
            .match_indices('\n')
            .nth(2)
            .map_or(rest.len(), |(i, _)| i);
        &rest[..end]
    }

    #[test]
    fn blocking_stdin_lock_counter_catches_every_shape() {
        let cases: &[(&str, usize)] = &[
            // ── Today's two production places ──
            ("let mut guard = sync::lock(stdin);", 1),
            ("let had_stdin = sync::lock(&stdin).take().is_some();", 1),
            // ── Forms the old ② decision missed ──
            ("let g = sync::lock(stdin.as_ref());", 1),
            ("let g = sync::lock(\n    &self.stdin,\n);", 1),
            (
                "use engram_dashboard_base::sync::lock;\nlet g = lock(&self.stdin);",
                1,
            ),
            ("let g = Mutex::lock(&self.stdin);", 1),
            (
                "let g = engram_dashboard_base::sync::lock(&*self.stdin);",
                1,
            ),
            // ── Form ① — ② does not count it twice ──
            ("let g = stdin.lock();", 1),
            ("let g = stdin\n  .lock();", 1),
            // ── Forms not counted ──
            ("if let Ok(g) = stdin.try_lock() {}", 0),
            ("let g = sync::lock(&self.stdout);", 0),
        ];
        for (snippet, want) in cases {
            assert_eq!(
                count_blocking_stdin_locks(snippet),
                *want,
                "the count for this form is wrong: {snippet:?}"
            );
        }
    }

    // ── List announcement · floor decision · echo comparison · ✕ (ADR-0231) ──────────────────────

    /// List events (ring order). ★The handed-over mark (`HandedOver`) is excluded★ — a row mark, not an outcome; measured by [`handed`].
    fn list_ops(seen: &Arc<Mutex<Vec<OutputEvent>>>) -> Vec<QueuedInputEvent> {
        seen.lock()
            .unwrap()
            .iter()
            .filter_map(|e| match e {
                OutputEvent::QueuedInput(QueuedInputEvent::HandedOver { .. }) => None,
                OutputEvent::QueuedInput(op) => Some(op.clone()),
                _ => None,
            })
            .collect()
    }

    fn user_bubbles(seen: &Arc<Mutex<Vec<OutputEvent>>>) -> Vec<(Option<String>, String)> {
        seen.lock()
            .unwrap()
            .iter()
            .filter_map(|e| match e {
                OutputEvent::Structured { kind, json } if kind == "user" => {
                    let v: Value = serde_json::from_str(json).unwrap();
                    Some((
                        v["uuid"].as_str().map(str::to_string),
                        v["text"].as_str().unwrap_or_default().to_string(),
                    ))
                }
                _ => None,
            })
            .collect()
    }

    fn queued(id: &str, text: &str) -> QueuedInputEvent {
        QueuedInputEvent::Queued {
            id: id.to_string(),
            text: text.to_string(),
        }
    }

    fn withdrawn(id: &str) -> QueuedInputEvent {
        QueuedInputEvent::Dropped {
            id: id.to_string(),
            cause: DropCause::Withdrawn,
        }
    }

    fn take(h: &Harness) -> Option<Job> {
        with_state(&h.state, |s| take_turn_locked(s, &h.next_id))
    }

    fn turn_line(job: Option<Job>) -> String {
        match job {
            Some(Job::Turn { line, .. }) => line,
            _ => panic!("turn/start did not go out"),
        }
    }

    fn stage_of(state: &SharedState, id: &str) -> Option<String> {
        with_state(state, |s| {
            s.pending
                .iter()
                .find(|p| p.id.as_deref() == Some(id))
                .map(|p| match &p.stage {
                    Stage::Held => "held".to_string(),
                    Stage::InFlight { gen, turn_id } => {
                        format!("in-flight gen={gen} turn={turn_id:?}")
                    }
                })
        })
    }

    fn accept_user(state: &SharedState, id: &str, text: &str) {
        accept_input(
            state,
            Some(id.to_string()),
            text.as_bytes().to_vec(),
            InputOrigin::User,
        )
        .expect("accepts");
    }

    fn echo_line(turn: &str, client_id: &str, text: &str) -> String {
        serde_json::json!({
            "method": "item/started",
            "params": {"threadId": "T", "turnId": turn,
                       "item": {"type": "userMessage", "id": format!("codex-{client_id}"),
                                "clientId": client_id,
                                "content": [{"type": "text", "text": text}]}},
        })
        .to_string()
    }

    /// ★Our edition in the trailing parenthesis is not read · unknown means below the floor★ — `clientUserMessageId` on an unknown edition
    /// quietly breaks the echo comparison.
    #[test]
    fn floor_verdict_reads_cli_version_first_then_the_agent_s_own_version_and_falls_below_when_unsure(
    ) {
        let ua = |v: &str| {
            format!("engram-dashboard/{v} (Windows 10.0.26200; x86_64) unknown (engram-dashboard; 0.200.0)")
        };
        for (cli, agent, want, why) in [
            (
                Some("0.156.1"),
                String::new(),
                Floor::Above,
                "measured edition",
            ),
            (
                Some("0.140.0"),
                String::new(),
                Floor::Above,
                "the floor edition itself",
            ),
            (
                Some("1.0.0"),
                String::new(),
                Floor::Above,
                "a large edition",
            ),
            (
                Some("0.139.9"),
                ua("0.156.1"),
                Floor::Below,
                "cliVersion comes first",
            ),
            (
                Some("0.140.0-alpha.1"),
                String::new(),
                Floor::Below,
                "a prerelease of the floor is below the floor",
            ),
            (
                Some("0.141.0-alpha.1"),
                String::new(),
                Floor::Above,
                "a prerelease above the floor",
            ),
            (
                None,
                ua("0.156.1"),
                Floor::Above,
                "fallback = first edition of userAgent",
            ),
            (
                None,
                ua("0.139.0"),
                Floor::Below,
                "our edition in the trailing parenthesis (0.200.0) is not read",
            ),
            (
                Some("garbage"),
                ua("0.141.2"),
                Floor::Above,
                "an unreadable cliVersion goes to the fallback",
            ),
            (None, "no-version-here".into(), Floor::Below, "both fail"),
            (None, String::new(), Floor::Below, "empty value"),
            (
                None,
                "/0.156.1".into(),
                Floor::Below,
                "a head without a name",
            ),
        ] {
            assert_eq!(floor_verdict(cli, &agent), want, "{why}: {cli:?} / {agent}");
        }
    }

    /// ★Pre-decision items are not announced; the side the decision lands on emits them all at once★ — earlier, list items would meet a
    /// below-floor decision. Items while connecting are not synthetic bubbles (Direct stands only after `Ready`).
    #[test]
    fn user_input_before_the_verdict_is_listed_only_once_the_verdict_is_above() {
        let h = harness();
        let ann = announcer();
        accept_user(&h.state, "u-1", "one");
        accept_input(
            &h.state,
            Some("m-2".into()),
            b"two".to_vec(),
            InputOrigin::Mail,
        )
        .unwrap();
        accept_user(&h.state, "u-3", "three");

        announce(&h.reader.core, &h.state, &ann);
        assert!(
            h.seen.lock().unwrap().is_empty(),
            "announced before the decision"
        );
        assert_eq!(ann.ack.state(), DeliveryAckState::Unknown);

        settle_floor(&h.reader.core, &h.state, &ann, Floor::Above);
        assert_eq!(
            list_ops(&h.seen),
            vec![queued("u-1", "one"), queued("u-3", "three")],
            "once each, in typing order"
        );
        assert!(
            user_bubbles(&h.seen).is_empty(),
            "{:?}",
            user_bubbles(&h.seen)
        );
        assert_eq!(ann.ack.state(), DeliveryAckState::Available);
        with_state(&h.state, |s| {
            let got: Vec<_> = s
                .pending
                .iter()
                .map(|p| (p.id.clone(), p.listed, p.announced))
                .collect();
            assert_eq!(
                got,
                vec![
                    (Some("u-1".to_string()), true, true),
                    (Some("m-2".to_string()), false, true),
                    (Some("u-3".to_string()), true, true),
                ],
                "mail does not go into the list but carries the identifier"
            );
        });

        announce(&h.reader.core, &h.state, &ann);
        assert_eq!(list_ops(&h.seen).len(), 2, "0 re-announcements");
    }

    /// ★Below the floor = today's path★ (user decision N1 (a)) — 0 events · no identifier · `turn/start` bytes as before this field existed
    /// · it leaves at the moment of opening.
    #[test]
    fn a_below_verdict_is_today_s_path_with_no_events_no_ids_and_today_s_bytes() {
        let h = harness();
        let ann = announcer();
        accept_user(&h.state, "u-1", "a");
        settle_floor(&h.reader.core, &h.state, &ann, Floor::Below);
        accept_user(&h.state, "u-2", "b");
        announce(&h.reader.core, &h.state, &ann);

        assert!(
            h.seen.lock().unwrap().is_empty(),
            "an event happened on a below-floor decision"
        );
        assert_eq!(ann.ack.state(), DeliveryAckState::Unavailable);
        with_state(&h.state, |s| {
            for p in &s.pending {
                assert!(p.id.is_none() && p.announced && !p.listed);
            }
        });

        make_ready(&h.state, "T");
        let first = turn_line(take(&h));
        with_state(&h.state, |s| s.turn = TurnState::Idle);
        let second = turn_line(take(&h));
        assert_eq!(
            [first, second],
            [
                concat!(
                    r#"{"id":0,"method":"turn/start","params":{"input":[{"text":"a","type":"text"}],"threadId":"T"}}"#,
                    "\n"
                ),
                concat!(
                    r#"{"id":1,"method":"turn/start","params":{"input":[{"text":"b","type":"text"}],"threadId":"T"}}"#,
                    "\n"
                ),
            ]
        );
        assert!(with_state(&h.state, |s| s.pending.is_empty()));
    }

    #[test]
    fn turn_start_carries_the_client_message_id_only_for_id_bearing_items() {
        let h = harness();
        let ann = announcer();
        make_ready(&h.state, "T");
        settle_floor(&h.reader.core, &h.state, &ann, Floor::Above);
        with_state(&h.state, |s| s.push_legacy(b"a"));
        accept_user(&h.state, "u-1", "b");
        accept_input(
            &h.state,
            Some("m-1".into()),
            b"c".to_vec(),
            InputOrigin::Mail,
        )
        .unwrap();
        announce(&h.reader.core, &h.state, &ann);

        let mut lines = Vec::new();
        for _ in 0..3 {
            lines.push(turn_line(take(&h)));
            with_state(&h.state, |s| s.turn = TurnState::Idle);
        }
        assert_eq!(
            lines,
            [
                concat!(
                    r#"{"id":0,"method":"turn/start","params":{"clientUserMessageId":"u-1","input":[{"text":"b","type":"text"}],"threadId":"T"}}"#,
                    "\n"
                ),
                concat!(
                    r#"{"id":1,"method":"turn/start","params":{"input":[{"text":"a","type":"text"}],"threadId":"T"}}"#,
                    "\n"
                ),
                concat!(
                    r#"{"id":2,"method":"turn/start","params":{"clientUserMessageId":"m-1","input":[{"text":"c","type":"text"}],"threadId":"T"}}"#,
                    "\n"
                ),
            ]
        );
        assert_eq!(
            stage_of(&h.state, "u-1").as_deref(),
            Some("in-flight gen=0 turn=None")
        );
        assert_eq!(
            stage_of(&h.state, "m-1").as_deref(),
            Some("in-flight gen=2 turn=None")
        );
        assert_eq!(
            with_state(&h.state, |s| s.pending.len()),
            2,
            "only items without an identifier leave"
        );
        assert_eq!(
            list_ops(&h.seen),
            vec![queued("u-1", "b")],
            "mail is not announced"
        );
        assert!(user_bubbles(&h.seen).is_empty());
    }

    /// ★Two inputs at the same idle moment → first Direct · second Queued★ — Direct also goes into pending, so the second cannot see "empty".
    /// With the Direct id `Queued` goes out on no branch, and one `turn/start` is one item.
    #[test]
    fn an_idle_input_is_a_direct_bubble_and_the_next_one_in_the_same_moment_is_queued() {
        let h = harness();
        let ann = announcer();
        make_ready(&h.state, "T");
        settle_floor(&h.reader.core, &h.state, &ann, Floor::Above);
        accept_user(&h.state, "d-1", "hello");
        announce(&h.reader.core, &h.state, &ann);
        accept_user(&h.state, "q-2", "second");
        announce(&h.reader.core, &h.state, &ann);

        assert_eq!(
            user_bubbles(&h.seen),
            vec![(Some("d-1".to_string()), "hello".to_string())]
        );
        assert_eq!(list_ops(&h.seen), vec![queued("q-2", "second")]);

        let v: Value = serde_json::from_str(&turn_line(take(&h))).unwrap();
        assert_eq!(v["params"]["clientUserMessageId"], "d-1");
        assert_eq!(
            stage_of(&h.state, "d-1").as_deref(),
            Some("in-flight gen=0 turn=None")
        );
        assert!(take(&h).is_none(), "one item per turn/start");
        assert_eq!(stage_of(&h.state, "q-2").as_deref(), Some("held"));
    }

    #[test]
    fn a_busy_input_is_queued_and_never_bubbled() {
        let h = harness();
        let ann = announcer();
        make_ready(&h.state, "T");
        settle_floor(&h.reader.core, &h.state, &ann, Floor::Above);
        activate(&h.state, 0, Some("U"));
        accept_user(&h.state, "q-1", "x");
        announce(&h.reader.core, &h.state, &ann);
        assert_eq!(list_ops(&h.seen), vec![queued("q-1", "x")]);
        assert!(user_bubbles(&h.seen).is_empty());
        assert!(take(&h).is_none());
    }

    /// ★After the link's synthetic bubble the turn table stays idle★ (ADR-0127) — the ordinary door would count it as progress, and a
    /// refused `turn/start` would leave a never-opened turn "in progress", blocking mail.
    #[test]
    fn the_direct_bubble_leaves_the_turn_table_idle() {
        let h = fact_harness();
        let ann = announcer();
        make_ready(&h.state, "T");
        settle_floor(&h.reader.core, &h.state, &ann, Floor::Above);
        accept_user(&h.state, "d-1", "hello");
        announce(&h.reader.core, &h.state, &ann);
        assert_eq!(user_bubbles(&h.seen).len(), 1);
        assert!(
            !h.table.is_in_turn(h.id, 1),
            "the synthetic bubble turned 「mid-turn」 on"
        );
    }

    /// ★The writer does not skip a pre-announce head★ — a handover outrunning the announcement would drop the receipt for an id the
    /// registry does not know, and skipping would break typing order.
    #[test]
    fn the_writer_waits_for_an_unannounced_head_instead_of_skipping_it() {
        let h = harness();
        let ann = announcer();
        make_ready(&h.state, "T");
        settle_floor(&h.reader.core, &h.state, &ann, Floor::Above);
        accept_user(&h.state, "u-1", "first");
        with_state(&h.state, |s| s.push_legacy(b"mail"));
        assert!(
            take(&h).is_none(),
            "skipped the pre-announce head and a later item went out"
        );

        announce(&h.reader.core, &h.state, &ann);
        let v: Value = serde_json::from_str(&turn_line(take(&h))).unwrap();
        assert_eq!(v["params"]["clientUserMessageId"], "u-1");
        assert_eq!(v["params"]["input"][0]["text"], "first");
    }

    /// A success response writes the turn id onto its item and only releases the response wait (not receipt). ★Generation masking★ — a late
    /// response of a turn with a different marker does not touch it.
    #[test]
    fn a_turn_start_reply_marks_its_carrier_and_a_stale_reply_moves_nothing() {
        let mut h = harness();
        let ann = announcer();
        make_ready(&h.state, "T");
        settle_floor(&h.reader.core, &h.state, &ann, Floor::Above);
        accept_user(&h.state, "u-1", "hi");
        announce(&h.reader.core, &h.state, &ann);
        let Some(Job::Turn { id, seq, .. }) = take(&h) else {
            panic!("turn/start did not go out")
        };
        assert!(h
            .pending
            .register(id, Waiter::TurnStart { seq }, method::TURN_START));
        h.reader
            .handle_line(format!(r#"{{"id":{id},"result":{{"turn":{{"id":"U1"}}}}}}"#).as_bytes());
        assert_eq!(
            stage_of(&h.state, "u-1"),
            Some(format!("in-flight gen={seq} turn=Some(\"U1\")"))
        );

        with_state(&h.state, |s| {
            s.push_pending(Some("x".into()), b"x".to_vec(), InputOrigin::Mail);
            s.pending.back_mut().unwrap().stage = Stage::InFlight {
                gen: 7,
                turn_id: None,
            };
        });
        activate(&h.state, 8, None);
        assert!(h
            .pending
            .register(50, Waiter::TurnStart { seq: 8 }, method::TURN_START));
        h.reader
            .handle_line(br#"{"id":50,"result":{"turn":{"id":"U8"}}}"#);
        assert!(h
            .pending
            .register(51, Waiter::TurnStart { seq: 7 }, method::TURN_START));
        h.reader
            .handle_line(br#"{"id":51,"result":{"turn":{"id":"U7"}}}"#);
        assert_eq!(
            stage_of(&h.state, "x").as_deref(),
            Some("in-flight gen=7 turn=None"),
            "a response of a different marker moved the item"
        );
    }

    /// ★Receipt wins at any stage★ — a held item does not go out again; with no open turn a turn boundary stands after the receipt
    /// ([`Reader::note_delivered`]).
    #[test]
    fn our_echo_is_delivered_before_its_bubble_and_leaves_pending_whatever_the_stage() {
        for stage in ["held", "in-flight"] {
            let mut h = harness_with_real_decoder();
            make_ready(&h.state, "T");
            with_state(&h.state, |s| {
                s.floor = Floor::Above;
                s.push_pending(Some("ours-1".into()), b"hi".to_vec(), InputOrigin::Mail);
                if stage == "in-flight" {
                    s.pending[0].stage = Stage::InFlight {
                        gen: 0,
                        turn_id: Some("U".into()),
                    };
                }
            });
            h.reader
                .handle_line(echo_line("U", "ours-1", "hi").as_bytes());

            let seen = h.seen.lock().unwrap().clone();
            match seen.as_slice() {
                [OutputEvent::QueuedInput(QueuedInputEvent::Delivered { id }), OutputEvent::Structured { kind, json }, OutputEvent::TurnEnd {
                    turn_id: None,
                    outcome: TurnOutcome::Completed,
                }] => {
                    assert_eq!(id, "ours-1", "{stage}");
                    assert_eq!(kind, "user", "{stage}");
                    assert!(json.contains(r#""uuid":"ours-1""#), "{stage}: {json}");
                }
                other => panic!("{stage}: not receipt + bubble: {other:?}"),
            }
            assert!(
                with_state(&h.state, |s| s.pending.is_empty()),
                "{stage}: received text remained in pending"
            );
            if stage == "held" {
                assert!(take(&h).is_none(), "received text went out again");
            }
        }
    }

    #[test]
    fn an_echo_for_an_id_we_do_not_hold_leaves_pending_alone() {
        let mut h = harness_with_real_decoder();
        make_ready(&h.state, "T");
        with_state(&h.state, |s| {
            s.floor = Floor::Above;
            s.push_pending(Some("mine".into()), b"hi".to_vec(), InputOrigin::Mail);
        });
        h.reader
            .handle_line(echo_line("U", "someone-else", "yo").as_bytes());
        assert_eq!(
            list_ops(&h.seen),
            vec![QueuedInputEvent::Delivered {
                id: "someone-else".into()
            }]
        );
        assert_eq!(stage_of(&h.state, "mine").as_deref(), Some("held"));
    }

    #[test]
    fn withdrawing_a_held_listed_item_drops_it_and_sends_nothing() {
        let h = harness();
        let ann = announcer();
        make_ready(&h.state, "T");
        settle_floor(&h.reader.core, &h.state, &ann, Floor::Above);
        activate(&h.state, 0, Some("U"));
        accept_user(&h.state, "q-1", "x");
        announce(&h.reader.core, &h.state, &ann);

        assert_eq!(
            withdraw_item(&h.state, Some(&h.reader.core), "q-1"),
            Withdraw::Withdrawn
        );
        assert_eq!(
            list_ops(&h.seen),
            vec![queued("q-1", "x"), withdrawn("q-1")]
        );
        assert!(with_state(&h.state, |s| s.pending.is_empty()));
        with_state(&h.state, |s| s.turn = TurnState::Idle);
        assert!(take(&h).is_none(), "withdrawn text went out");
        assert!(outbox_lines(&h.state).is_empty());
        assert_eq!(
            withdraw_item(&h.state, Some(&h.reader.core), "q-1"),
            Withdraw::NotHeld,
            "overlapping second ✕"
        );
        assert_eq!(list_ops(&h.seen).len(), 2, "the second ✕ emitted an event");
    }

    /// ★✕ on a pre-announce item → ring order `Queued` → `Dropped`★ — only the death mark is attached; the announcing side follows `Queued`.
    #[test]
    fn withdrawing_before_the_announcement_lands_the_drop_right_after_the_queued_line() {
        let h = harness();
        let ann = announcer();
        make_ready(&h.state, "T");
        settle_floor(&h.reader.core, &h.state, &ann, Floor::Above);
        activate(&h.state, 0, Some("U"));
        accept_user(&h.state, "q-1", "x");

        assert_eq!(
            withdraw_item(&h.state, Some(&h.reader.core), "q-1"),
            Withdraw::Withdrawn
        );
        assert!(
            list_ops(&h.seen).is_empty(),
            "Dropped stood in the ring before the announcement"
        );
        assert_eq!(
            withdraw_item(&h.state, Some(&h.reader.core), "q-1"),
            Withdraw::NotHeld,
            "the first ✕ already removed it"
        );

        announce(&h.reader.core, &h.state, &ann);
        assert_eq!(
            list_ops(&h.seen),
            vec![queued("q-1", "x"), withdrawn("q-1")]
        );
        assert!(with_state(&h.state, |s| s.pending.is_empty()));
    }

    /// Outside the list (Direct · mail · pre-decision) and unknown ids are `NotHeld` with no event. A handed-over list item cannot be
    /// withdrawn — `TooLate` with the cancellation receipt (ADR-0231 · TRD §5-5 「✕」).
    #[test]
    fn only_a_listed_held_item_can_be_withdrawn() {
        let before = harness();
        accept_user(&before.state, "p-1", "pre");
        assert_eq!(
            withdraw_item(&before.state, Some(&before.reader.core), "p-1"),
            Withdraw::NotHeld,
            "pre-decision items are not in the list"
        );

        let h = harness();
        let ann = announcer();
        make_ready(&h.state, "T");
        settle_floor(&h.reader.core, &h.state, &ann, Floor::Above);
        accept_user(&h.state, "d-1", "direct");
        accept_input(
            &h.state,
            Some("m-1".into()),
            b"mail".to_vec(),
            InputOrigin::Mail,
        )
        .unwrap();
        accept_user(&h.state, "q-2", "queued");
        announce(&h.reader.core, &h.state, &ann);
        with_state(&h.state, |s| {
            let q = s
                .pending
                .iter_mut()
                .find(|p| p.id.as_deref() == Some("q-2"))
                .unwrap();
            q.stage = Stage::InFlight {
                gen: 0,
                turn_id: Some("U".into()),
            };
        });
        let mut expected = list_ops(&h.seen);
        for id in ["d-1", "m-1", "nope"] {
            assert_eq!(
                withdraw_item(&h.state, Some(&h.reader.core), id),
                Withdraw::NotHeld,
                "{id}"
            );
        }
        assert_eq!(list_ops(&h.seen), expected, "NotHeld emitted an event");
        assert_eq!(
            withdraw_item(&h.state, Some(&h.reader.core), "q-2"),
            Withdraw::TooLate,
            "handed-over item"
        );
        expected.extend([
            QueuedInputEvent::CancelRequested { id: "q-2".into() },
            QueuedInputEvent::CancelAnswered {
                id: "q-2".into(),
                removed: false,
            },
        ]);
        assert_eq!(list_ops(&h.seen), expected);
        assert_eq!(with_state(&h.state, |s| s.pending.len()), 3);
    }

    // ── Turn-end disposal v1 (ADR-0231 · TRD §5-5) ───────────────────────────────

    fn above(h: &Harness) -> Arc<Announcer> {
        make_ready(&h.state, "T");
        let ann = announcer();
        settle_floor(&h.reader.core, &h.state, &ann, Floor::Above);
        ann
    }

    fn open_turn(h: &Harness) -> (i64, u64) {
        let Some(Job::Turn { id, seq, .. }) = take(h) else {
            panic!("turn/start did not go out")
        };
        assert!(h
            .pending
            .register(id, Waiter::TurnStart { seq }, method::TURN_START));
        (id, seq)
    }

    fn reply(h: &mut Harness, request: i64, turn: &str) {
        h.reader.handle_line(
            format!(r#"{{"id":{request},"result":{{"turn":{{"id":"{turn}"}}}}}}"#).as_bytes(),
        );
    }

    fn in_flight_left(state: &SharedState) -> bool {
        with_state(state, |s| {
            s.pending
                .iter()
                .any(|p| matches!(p.stage, Stage::InFlight { .. }))
        })
    }

    /// List events · bubbles · turn boundaries in ring order — also checks disposal events come before the boundary. ★The handed-over mark
    /// is excluded★ (a row mark; measured by [`handed`]).
    fn trace(seen: &Arc<Mutex<Vec<OutputEvent>>>) -> Vec<String> {
        seen.lock()
            .unwrap()
            .iter()
            .filter_map(|e| match e {
                OutputEvent::QueuedInput(QueuedInputEvent::HandedOver { .. }) => None,
                OutputEvent::QueuedInput(QueuedInputEvent::Queued { id, .. }) => {
                    Some(format!("queued {id}"))
                }
                OutputEvent::QueuedInput(QueuedInputEvent::Dropped { id, cause }) => {
                    Some(format!("dropped {id} {cause:?}"))
                }
                OutputEvent::QueuedInput(QueuedInputEvent::Delivered { id }) => {
                    Some(format!("delivered {id}"))
                }
                OutputEvent::QueuedInput(other) => Some(format!("{other:?}")),
                OutputEvent::TurnEnd { outcome, .. } => Some(
                    match outcome {
                        TurnOutcome::Completed => "end completed",
                        TurnOutcome::Failed { .. } => "end failed",
                        TurnOutcome::Interrupted => "end interrupted",
                        TurnOutcome::Unknown => "end unknown",
                    }
                    .to_string(),
                ),
                OutputEvent::Structured { kind, json } if kind == "user" => {
                    let v: Value = serde_json::from_str(json).unwrap();
                    Some(format!("bubble {}", v["uuid"].as_str().unwrap_or("?")))
                }
                _ => None,
            })
            .collect()
    }

    fn handed(seen: &Arc<Mutex<Vec<OutputEvent>>>) -> Vec<String> {
        seen.lock()
            .unwrap()
            .iter()
            .filter_map(|e| match e {
                OutputEvent::QueuedInput(QueuedInputEvent::HandedOver { id, sent }) => {
                    Some(format!("{} {id}", if *sent { "sent" } else { "held" }))
                }
                _ => None,
            })
            .collect()
    }

    fn row_ids(h: &Harness) -> Vec<String> {
        h.reader
            .core
            .queued_inputs()
            .snapshot()
            .0
            .into_iter()
            .map(|r| r.id)
            .collect()
    }

    #[derive(Debug, Clone, Copy, PartialEq)]
    enum Whose {
        Listed,
        Direct,
        Mail,
    }

    #[derive(Debug, Clone, Copy)]
    enum End {
        Completed(&'static str),
        Early(&'static str),
        ErrorReply,
        WriteFailed,
        Deadline,
        Unreadable,
        LongTurnId,
    }

    /// Ends a turn that handed over one item with `end` and returns the traces after it. ★At every end the turn must be closed and pending
    /// must have no `InFlight`★ (TRD §5-5 invariant).
    fn end_a_turn(whose: Whose, end: End) -> (Harness, &'static str, Vec<String>) {
        let mut h = harness_with_real_decoder();
        let ann = above(&h);
        let id = match whose {
            Whose::Listed => {
                activate(&h.state, 0, Some("U0"));
                accept_user(&h.state, "q-1", "hi");
                with_state(&h.state, |s| s.turn = TurnState::Idle);
                "q-1"
            }
            Whose::Direct => {
                accept_user(&h.state, "d-1", "hi");
                "d-1"
            }
            Whose::Mail => {
                accept_input(
                    &h.state,
                    Some("m-1".into()),
                    b"hi".to_vec(),
                    InputOrigin::Mail,
                )
                .unwrap();
                "m-1"
            }
        };
        announce(&h.reader.core, &h.state, &ann);
        let (request, seq) = open_turn(&h);
        h.seen.lock().unwrap().clear();
        finish_turn(&mut h, request, seq, end);
        assert_eq!(
            turn_seq(&h.state),
            None,
            "{whose:?}/{end:?}: the turn did not end"
        );
        assert!(
            !in_flight_left(&h.state),
            "{whose:?}/{end:?}: the InFlight of the ended turn remained"
        );
        let t = trace(&h.seen);
        (h, id, t)
    }

    fn finish_turn(h: &mut Harness, request: i64, seq: u64, end: End) {
        match end {
            End::Completed(status) => {
                reply(h, request, "U1");
                h.reader
                    .handle_line(completed_line("T", "U1", status).as_bytes());
            }
            End::Early(status) => {
                h.reader
                    .handle_line(completed_line("T", "U1", status).as_bytes());
                assert_eq!(turn_seq(&h.state), Some(seq), "ended before the response");
                reply(h, request, "U1");
            }
            End::ErrorReply => h.reader.handle_line(
                format!(r#"{{"id":{request},"error":{{"code":-32600,"message":"rejected"}}}}"#)
                    .as_bytes(),
            ),
            End::WriteFailed => {
                let core = h.reader.core.clone();
                turn_write_failed(
                    &h.state,
                    &h.pending,
                    &core,
                    request,
                    seq,
                    &PtyError::WriteFailed("broken pipe".into()),
                );
                assert_eq!(
                    h.pending.len(),
                    0,
                    "the waiting entry of the request that could not be written remained"
                );
            }
            End::Deadline => {
                h.pending.forget(request);
                assert!(h.pending.register_at(
                    request,
                    Waiter::TurnStart { seq },
                    method::TURN_START,
                    Instant::now(),
                ));
                sweep_deadlines(&h.state, &h.pending, &h.reader.core);
            }
            End::Unreadable => h.reader.handle_line(
                format!(r#"{{"id":{request},"result":{{"not-a-turn":true}}}}"#).as_bytes(),
            ),
            End::LongTurnId => reply(h, request, &"U".repeat(MAX_TURN_ID_BYTES + 1)),
        }
    }

    fn assert_left_silently(h: Harness, whose: Whose, end: End, t: &[String], ending: &str) {
        assert_eq!(t, [ending], "{whose:?}/{end:?}");
        assert!(
            with_state(&h.state, |s| s.pending.is_empty()),
            "{whose:?}/{end:?}: an item outside the list remained in pending"
        );
        accept_user(&h.state, "next", "again");
        announce(&h.reader.core, &h.state, &announcer());
        assert_eq!(
            trace(&h.seen).last().map(String::as_str),
            Some("bubble next"),
            "{whose:?}/{end:?}: the following idle input is not Direct"
        );
    }

    fn assert_held_again(h: Harness, whose: Whose, end: End, id: &str, t: &[String], ending: &str) {
        let at = format!("{whose:?}/{end:?}");
        assert_eq!(t, [ending], "{at}: a list event happened");
        assert_eq!(stage_of(&h.state, id).as_deref(), Some("held"), "{at}");
        let rows: Vec<String> = match whose {
            Whose::Listed => vec![id.to_string()],
            _ => Vec::new(),
        };
        assert_eq!(row_ids(&h), rows, "{at}: the list changed");
        assert!(!with_state(&h.state, |s| s.halted), "{at}: a halt stood");
        let v: Value = serde_json::from_str(&turn_line(take(&h))).unwrap();
        assert_eq!(
            v["params"]["clientUserMessageId"], id,
            "{at}: the returned text did not ride the next turn"
        );
        let seq = turn_seq(&h.state).expect("a turn opened");
        assert_eq!(
            stage_of(&h.state, id),
            Some(format!("in-flight gen={seq} turn=None")),
            "{at}"
        );
        assert!(take(&h).is_none(), "{at}: went out twice");
    }

    /// The handed-over user text `id` returns after a failure end and waits behind the halt — 0 events · 0 turns while halted · when the
    /// next user text lifts the halt, the returned text rides the next turn **once**, before that text (what remains goes first).
    fn assert_held_behind_the_halt(h: Harness, whose: Whose, end: End, id: &str, t: &[String]) {
        let at = format!("{whose:?}/{end:?}");
        assert_eq!(t, ["end failed"], "{at}: a list event happened");
        assert_eq!(stage_of(&h.state, id).as_deref(), Some("held"), "{at}");
        let rows: Vec<String> = match whose {
            Whose::Listed => vec![id.to_string()],
            _ => Vec::new(),
        };
        assert_eq!(row_ids(&h), rows, "{at}: the list changed");
        assert!(
            with_state(&h.state, |s| s.halted),
            "{at}: the halt did not stand"
        );
        assert!(take(&h).is_none(), "{at}: a turn opened during the halt");
        accept_user(&h.state, "next", "again");
        announce(&h.reader.core, &h.state, &announcer());
        let v: Value = serde_json::from_str(&turn_line(take(&h))).unwrap();
        assert_eq!(
            v["params"]["clientUserMessageId"], id,
            "{at}: the returned text did not go out before the text that lifted the halt"
        );
        assert!(take(&h).is_none(), "{at}: went out twice");
    }

    /// ★User text of a turn that ended without an echo returns to the held place whatever the end shape★ (user decision 2026-09-26 ·
    /// ADR-0235 — replaces 「erase on failure」). Outcome-speaking ends (`completed` · unknown status) and interruption let it ride the
    /// next turn at once (no halt); failure ends (`failed` · `turn/start` write failure · error response · deadline · decode failure · long
    /// turn id) set the halt and it waits for the next user text. No events either way; before/after the response and early-end release are
    /// the same. Mail leaves without an event.
    #[test]
    fn a_turn_that_ends_without_our_echo_returns_its_user_item_to_held_whatever_the_close() {
        for (end, ending) in [
            (End::Completed("completed"), "end completed"),
            (End::Completed("inProgress"), "end unknown"),
            (End::Completed("interrupted"), "end interrupted"),
            (End::Early("completed"), "end completed"),
            (End::Early("interrupted"), "end interrupted"),
        ] {
            for whose in [Whose::Listed, Whose::Direct] {
                let (h, id, t) = end_a_turn(whose, end);
                assert_held_again(h, whose, end, id, &t, ending);
            }
            let (h, _, t) = end_a_turn(Whose::Mail, end);
            assert_left_silently(h, Whose::Mail, end, &t, ending);
        }
        for end in [
            End::Completed("failed"),
            End::Early("failed"),
            End::ErrorReply,
            End::WriteFailed,
            End::Deadline,
            End::Unreadable,
            End::LongTurnId,
        ] {
            for whose in [Whose::Listed, Whose::Direct] {
                let (h, id, t) = end_a_turn(whose, end);
                assert_held_behind_the_halt(h, whose, end, id, &t);
            }
            let (h, _, t) = end_a_turn(Whose::Mail, end);
            assert_left_silently(h, Whose::Mail, end, &t, "end failed");
        }
    }

    #[test]
    fn a_return_to_held_writes_one_held_again_line_with_the_close() {
        let captured = Arc::new(Mutex::new(Vec::new()));
        tracing::subscriber::with_default(TraceCapture(captured.clone()), || {
            end_a_turn(Whose::Listed, End::Completed("interrupted"));
            end_a_turn(Whose::Listed, End::ErrorReply);
            end_a_turn(Whose::Listed, End::Early("completed"));
        });
        let lines = captured.lock().unwrap();
        let got: Vec<(Vec<&str>, [&str; 3])> = lines
            .iter()
            .filter(|l| l.get("phase").map(String::as_str) == Some("held_again"))
            .map(|l| {
                let at = |k: &str| l.get(k).map_or("-", String::as_str);
                (
                    l.keys().map(String::as_str).collect(),
                    [at("turn_id"), at("item_id"), at("close")],
                )
            })
            .collect();
        assert_eq!(
            got,
            vec![
                (
                    vec!["agent", "close", "item_id", "message", "phase", "turn_id"],
                    ["U1", "q-1", "Interrupted"]
                ),
                (
                    vec!["agent", "close", "item_id", "message", "phase"],
                    ["-", "q-1", "Rejected"]
                ),
                (
                    vec!["agent", "close", "item_id", "message", "phase", "turn_id"],
                    ["U1", "q-1", "Completed"]
                ),
            ],
            "{lines:?}"
        );
    }

    /// ★An interruption returns that turn's handed-over list items to the held place and leaves held items as is★ (user decision
    /// 2026-09-26 — held text goes right away into a new turn) — pre-announce items only get `Queued`, not dropped. Held mail · items
    /// without an identifier stay, no halt, and the next Idle opens from the oldest user text in typing order.
    #[test]
    fn an_interrupt_returns_the_handed_over_item_to_held_and_keeps_held_items_for_the_next_idle() {
        let mut h = harness_with_real_decoder();
        let ann = above(&h);
        activate(&h.state, 0, Some("U0"));
        accept_user(&h.state, "q-1", "first");
        with_state(&h.state, |s| s.turn = TurnState::Idle);
        announce(&h.reader.core, &h.state, &ann);
        let (request, _) = open_turn(&h);
        reply(&mut h, request, "U1");
        accept_user(&h.state, "q-2", "held");
        accept_input(
            &h.state,
            Some("m-1".into()),
            b"mail".to_vec(),
            InputOrigin::Mail,
        )
        .unwrap();
        with_state(&h.state, |s| s.push_legacy(b"legacy"));
        announce(&h.reader.core, &h.state, &ann);
        accept_user(&h.state, "q-3", "not yet announced");
        h.seen.lock().unwrap().clear();

        h.reader
            .handle_line(completed_line("T", "U1", "interrupted").as_bytes());
        assert_eq!(trace(&h.seen), ["end interrupted"]);
        announce(&h.reader.core, &h.state, &ann);
        assert_eq!(
            trace(&h.seen)[1..],
            ["queued q-3"],
            "a held pre-announce item was dropped"
        );

        for id in ["q-1", "q-2", "q-3", "m-1"] {
            assert_eq!(stage_of(&h.state, id).as_deref(), Some("held"), "{id}");
        }
        assert_eq!(
            with_state(&h.state, |s| s.pending.len()),
            5,
            "the interruption removed an item"
        );
        assert_eq!(row_ids(&h), ["q-1", "q-2", "q-3"]);
        assert!(
            !with_state(&h.state, |s| s.halted),
            "the interruption set a halt"
        );
        let v: Value = serde_json::from_str(&turn_line(take(&h))).unwrap();
        assert_eq!(v["params"]["clientUserMessageId"], "q-1");
    }

    #[test]
    fn an_echo_before_the_turn_end_leaves_nothing_to_dispose() {
        let mut h = harness_with_real_decoder();
        let ann = above(&h);
        activate(&h.state, 0, Some("U0"));
        accept_user(&h.state, "q-1", "hi");
        with_state(&h.state, |s| s.turn = TurnState::Idle);
        announce(&h.reader.core, &h.state, &ann);
        let (request, _) = open_turn(&h);
        reply(&mut h, request, "U1");
        h.reader
            .handle_line(echo_line("U1", "q-1", "hi").as_bytes());
        h.reader
            .handle_line(completed_line("T", "U1", "interrupted").as_bytes());
        assert_eq!(
            trace(&h.seen),
            [
                "queued q-1",
                "delivered q-1",
                "bubble q-1",
                "end interrupted"
            ]
        );
        assert!(with_state(&h.state, |s| s.pending.is_empty()));
    }

    /// ★A late echo for text returned to the held place is receipt — it does not go out again★ (both outcome-speaking ends · interruption).
    /// No open turn, so a boundary stands after the receipt.
    #[test]
    fn a_late_echo_after_the_return_to_held_delivers_the_item_and_it_is_not_sent_again() {
        for end in [End::Completed("completed"), End::Completed("interrupted")] {
            let (mut h, _, _) = end_a_turn(Whose::Listed, end);
            assert_eq!(
                stage_of(&h.state, "q-1").as_deref(),
                Some("held"),
                "{end:?}"
            );
            h.reader
                .handle_line(echo_line("U1", "q-1", "hi").as_bytes());
            assert_eq!(
                trace(&h.seen)[1..],
                ["delivered q-1", "bubble q-1", "end completed"],
                "{end:?}: no boundary stood after a receipt outside the open turn"
            );
            assert!(with_state(&h.state, |s| s.pending.is_empty()), "{end:?}");
            assert!(
                row_ids(&h).is_empty(),
                "{end:?}: the late receipt did not close the list"
            );
            assert!(take(&h).is_none(), "{end:?}: received text went out again");
        }
    }

    #[test]
    fn a_late_echo_of_the_old_send_after_the_re_send_delivers_the_item_once() {
        let (mut h, _, _) = end_a_turn(Whose::Listed, End::Completed("completed"));
        let (request, seq) = open_turn(&h);
        assert_eq!(
            stage_of(&h.state, "q-1"),
            Some(format!("in-flight gen={seq} turn=None")),
            "the returned text did not ride the next turn"
        );
        reply(&mut h, request, "U2");
        h.reader
            .handle_line(echo_line("U1", "q-1", "hi").as_bytes());
        assert!(with_state(&h.state, |s| s.pending.is_empty()));
        assert!(
            row_ids(&h).is_empty(),
            "the old request's receipt did not close the list"
        );

        h.reader
            .handle_line(echo_line("U2", "q-1", "hi").as_bytes());
        h.reader
            .handle_line(completed_line("T", "U2", "completed").as_bytes());
        assert_eq!(turn_seq(&h.state), None);
        assert!(with_state(&h.state, |s| s.pending.is_empty()));
        assert!(row_ids(&h).is_empty());
        assert!(take(&h).is_none(), "received text went out once more");
    }

    #[test]
    fn an_item_returned_to_held_keeps_the_kernel_s_inputs_pending_true_until_it_leaves() {
        let id = AgentId::new_v4();
        let table = Arc::new(crate::inputs_pending::InputsPendingTable::new());
        table.register(id, 1);
        let core = Arc::new(
            OutputCore::new(id, 1, Arc::new(NoopStatus), TurnWiring::detached()).with_queued(
                crate::output_core::QueuedWiring {
                    registry: Arc::new(crate::queued_input::QueuedInputs::new()),
                    pending: table.clone(),
                },
            ),
        );
        let mut h = harness_with_real_decoder();
        h.reader.core = core;
        let ann = above(&h);
        activate(&h.state, 0, Some("U0"));
        accept_user(&h.state, "q-1", "hi");
        with_state(&h.state, |s| s.turn = TurnState::Idle);
        announce(&h.reader.core, &h.state, &ann);
        assert_eq!(table.get(id, 1), Some(true));

        let (request, _) = open_turn(&h);
        reply(&mut h, request, "U1");
        h.reader
            .handle_line(completed_line("T", "U1", "completed").as_bytes());
        assert_eq!(stage_of(&h.state, "q-1").as_deref(), Some("held"));
        assert_eq!(
            table.get(id, 1),
            Some(true),
            "it returned to the held place but the list was written as empty"
        );

        h.reader
            .handle_line(echo_line("U1", "q-1", "hi").as_bytes());
        assert_eq!(table.get(id, 1), Some(false));
    }

    #[test]
    fn a_stream_end_leaves_the_list_to_the_core_s_synthesis_exactly_once() {
        let (mut h, _, _) = end_a_turn(Whose::Listed, End::Completed("completed"));
        let ann = announcer();
        accept_user(&h.state, "q-2", "in flight");
        announce(&h.reader.core, &h.state, &ann);
        let (request, _) = open_turn(&h);
        reply(&mut h, request, "U2");
        accept_user(&h.state, "q-3", "held");
        announce(&h.reader.core, &h.state, &ann);
        h.seen.lock().unwrap().clear();

        drop(ReaderExit {
            state: h.state.clone(),
            pending: h.pending.clone(),
            agent: h.reader.core.id(),
        });
        assert!(
            trace(&h.seen).is_empty(),
            "the link emitted an event on stream close"
        );
        h.reader.core.finish(TerminalReason::StreamClosed);
        let mut got = trace(&h.seen);
        got.sort();
        assert_eq!(
            got,
            [
                "dropped q-1 AgentEnded",
                "dropped q-2 AgentEnded",
                "dropped q-3 AgentEnded"
            ]
        );
    }

    /// The floor decision is written **after** the record returns and **before** the history — earlier, the record failure branch would
    /// erase list items without an event; after the history, the late-list window would grow by the paging.
    #[test]
    fn the_floor_is_settled_after_the_session_id_is_recorded_and_before_the_history() {
        let body = writer_loop_code();
        let recorded = body
            .find("record_session_id(")
            .expect("there is no `record_session_id(` call in writer_loop");
        let settled = body
            .find("settle_floor(")
            .expect("there is no `settle_floor(` call in writer_loop");
        let hydrated = body
            .find("hydrate_history(")
            .expect("there is no `hydrate_history(` call in writer_loop");
        assert!(
            recorded < settled && settled < hydrated,
            "the floor decision place is not after the record · before the history"
        );
    }

    #[cfg(windows)]
    #[test]
    fn the_transport_hands_out_the_very_ack_its_verdict_fills() {
        let (t, _) = open_probe(&["/c", "ping", "-n", "30", "127.0.0.1"]);
        let ack = t.delivery_ack();
        assert!(Arc::ptr_eq(&ack, &t.announcer.ack));
        assert_eq!(ack.state(), DeliveryAckState::Unknown);
        let (core, _) = core_with_sink();
        settle_floor(&core, &t.state, &t.announcer, Floor::Above);
        assert_eq!(ack.state(), DeliveryAckState::Available);
        t.shutdown();
    }

    #[cfg(windows)]
    #[test]
    fn shutdown_completes_even_if_a_write_blocks_on_a_full_pipe() {
        let (t, _) = open_probe(&["/c", "ping", "-n", "30", "127.0.0.1"]);
        let t = Arc::new(t);

        let stdin = t.stdin.clone();
        let writer = std::thread::spawn(move || {
            let big = "x".repeat(8 * 1024 * 1024);
            let _ = write_line(&stdin, &big);
        });
        std::thread::sleep(Duration::from_millis(500));

        let killer = t.clone();
        let start = Instant::now();
        let shutdown_thread = std::thread::spawn(move || killer.shutdown());
        let deadline = start + Duration::from_secs(10);
        while !shutdown_thread.is_finished() {
            assert!(
                Instant::now() < deadline,
                "shutdown did not end within 10s — stdin lock deadlock regression"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        shutdown_thread.join().expect("shutdown thread");
        let _ = writer.join();
    }

    // ── Tool-end trigger · trace lines (ADR-0231 · ADR-0235) ─────────────

    fn item_line(method_name: &str, turn: &str, kind: &str, id: &str) -> String {
        serde_json::json!({
            "method": method_name,
            "params": {"threadId": "T", "turnId": turn, "item": {"type": kind, "id": id}},
            "emittedAtMs": 1_790_274_927_235i64,
        })
        .to_string()
    }

    fn item_started(h: &mut Harness, turn: &str, kind: &str, id: &str) {
        h.reader
            .handle_line(item_line(method::ITEM_STARTED, turn, kind, id).as_bytes());
    }

    fn item_completed(h: &mut Harness, turn: &str, kind: &str, id: &str) {
        h.reader
            .handle_line(item_line(method::ITEM_COMPLETED, turn, kind, id).as_bytes());
    }

    /// The steer the writer picks right where our turn's tool just ended — sets the trigger like [`Reader::note_tool_item`] and picks it
    /// (`None` = nothing to hand over). It does not go through [`next_job`]'s sweep wait.
    fn steer_at_tool_end(h: &Harness) -> Option<Steer> {
        with_state(&h.state, |s| {
            s.steer_due = true;
            take_steer_locked(s, &h.next_id)
        })
    }

    /// The steer the writer would pick now — does not set the trigger, uses only what stood (`None` = the writer sleeps).
    fn steer_next(h: &Harness) -> Option<Steer> {
        with_state(&h.state, |s| take_steer_locked(s, &h.next_id))
    }

    fn steer_due(state: &SharedState) -> bool {
        with_state(state, |s| s.steer_due)
    }

    /// The next job the writer picks must be a steer — with the trigger standing, [`next_job`] returns at once without waiting.
    fn next_steer_job(h: &Harness, why: &str) -> Steer {
        match next_job(&h.state, &h.next_id) {
            Some(Job::Steer(s)) => s,
            _ => panic!("{why}"),
        }
    }

    /// ★Only the end of a tool item of our current turn is a trigger★ — a non-tool item end (output · echo · known variant with no grounds
    /// to be a tool · unknown variant) · a tool start · another turn id (late completion of an interrupted turn's tool — M6) · another
    /// thread's tool end hand over nothing (held text stays).
    #[test]
    fn only_a_tool_item_ending_in_our_current_turn_arms_the_steer() {
        let mut h = harness();
        let ann = above(&h);
        running_turn(&mut h, &ann, "t1");
        accept_user(&h.state, "q-1", "x");
        announce(&h.reader.core, &h.state, &ann);
        for kind in [
            "agentMessage",
            "reasoning",
            "plan",
            "userMessage",
            "contextCompaction",
            "imageView",
            "someFutureVariant",
        ] {
            item_started(&mut h, "t1", kind, "o");
            item_completed(&mut h, "t1", kind, "o");
            assert!(
                !steer_due(&h.state),
                "{kind}: the end of a non-tool item set the trigger"
            );
        }
        item_started(&mut h, "t1", "commandExecution", "c");
        assert!(!steer_due(&h.state), "the start of a tool set the trigger");
        item_completed(&mut h, "t0", "commandExecution", "old");
        assert!(!steer_due(&h.state), "a tool end of another turn id");
        let foreign = serde_json::json!({
            "method": method::ITEM_COMPLETED,
            "params": {"threadId": "X", "turnId": "t1", "item": {"type": "commandExecution", "id": "c"}},
        })
        .to_string();
        h.reader.handle_line(foreign.as_bytes());
        assert!(!steer_due(&h.state), "a tool end of another thread");
        assert!(steer_next(&h).is_none(), "handed over without a trigger");
        assert_eq!(stage_of(&h.state, "q-1").as_deref(), Some("held"));

        item_completed(&mut h, "t1", "commandExecution", "c");
        assert!(
            steer_due(&h.state),
            "a tool end of our turn did not set the trigger"
        );
        let s = steer_next(&h).expect("the tool end did not release the held text");
        assert_eq!(steer_params(&s)["clientUserMessageId"], "q-1");
    }

    /// ★A tool end before the turn id is known is not a trigger★ — no `expectedTurnId`, nothing to hand over; held text is handed over where
    /// the response gives the turn id ([`Reader::resolve`]).
    #[test]
    fn a_tool_end_before_the_turn_id_is_known_arms_nothing() {
        let mut h = harness();
        let ann = above(&h);
        accept_user(&h.state, "head", "first");
        announce(&h.reader.core, &h.state, &ann);
        let (request, _) = open_turn(&h);
        accept_user(&h.state, "q-1", "x");
        announce(&h.reader.core, &h.state, &ann);
        item_started(&mut h, "t1", "commandExecution", "c");
        item_completed(&mut h, "t1", "commandExecution", "c");
        assert!(
            !steer_due(&h.state),
            "a tool end before the turn id set the trigger"
        );
        assert!(steer_next(&h).is_none());
        assert_eq!(stage_of(&h.state, "q-1").as_deref(), Some("held"));

        reply(&mut h, request, "t1");
        let s = next_steer_job(&h, "the turn id did not hand over the held text");
        assert_eq!(steer_params(&s)["clientUserMessageId"], "q-1");
        assert_eq!(steer_params(&s)["expectedTurnId"], "t1");
    }

    /// A tool end wakes the waiting writer right away — without the wake-up the writer sees the trigger only after [`SWEEP_INTERVAL`] (the
    /// tool end is fed after the writer fell asleep).
    #[test]
    fn a_tool_end_wakes_a_waiting_writer_at_once() {
        let mut h = harness();
        let ann = above(&h);
        running_turn(&mut h, &ann, "t1");
        accept_user(&h.state, "q-1", "x");
        announce(&h.reader.core, &h.state, &ann);
        let (tx, rx) = mpsc::channel();
        let writer = {
            let state = h.state.clone();
            let next_id = h.next_id.clone();
            std::thread::spawn(move || {
                while let Some(job) = next_job(&state, &next_id) {
                    if let Job::Steer(s) = job {
                        let _ = tx.send(s.item);
                    }
                }
            })
        };
        std::thread::sleep(Duration::from_millis(50));
        item_started(&mut h, "t1", "commandExecution", "c");
        let fed = Instant::now();
        item_completed(&mut h, "t1", "commandExecution", "c");
        let got = rx
            .recv_timeout(Duration::from_secs(5))
            .expect("a tool end but it did not hand over");
        assert_eq!(got, "q-1");
        assert!(
            fed.elapsed() < SWEEP_INTERVAL / 2,
            "handed over by the sweep without a wake-up ({:?})",
            fed.elapsed()
        );

        with_state(&h.state, |s| s.closed = true);
        h.state.1.notify_all();
        writer.join().expect("writer");
    }

    type TraceLine = std::collections::BTreeMap<String, String>;

    /// A subscriber collecting only this thread's trace lines — field names are the contract with QA, so it measures they go out as documented.
    struct TraceCapture(Arc<Mutex<Vec<TraceLine>>>);

    struct TraceFields<'a>(&'a mut TraceLine);

    impl tracing::field::Visit for TraceFields<'_> {
        fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
            self.0
                .insert(field.name().to_string(), format!("{value:?}"));
        }
        fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
            self.0.insert(field.name().to_string(), value.to_string());
        }
    }

    impl tracing::Subscriber for TraceCapture {
        fn enabled(&self, metadata: &tracing::Metadata<'_>) -> bool {
            metadata.target() == STEER_TRACE
        }
        fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
            tracing::span::Id::from_u64(1)
        }
        fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}
        fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
        fn event(&self, event: &tracing::Event<'_>) {
            let mut line = TraceLine::new();
            event.record(&mut TraceFields(&mut line));
            self.0.lock().unwrap().push(line);
        }
        fn enter(&self, _: &tracing::span::Id) {}
        fn exit(&self, _: &tracing::span::Id) {}
    }

    #[test]
    fn the_steer_trace_lines_carry_the_documented_fields_per_phase() {
        let captured = Arc::new(Mutex::new(Vec::new()));
        let (first, second, agent) =
            tracing::subscriber::with_default(TraceCapture(captured.clone()), || {
                let mut h = harness_with_real_decoder();
                let agent = h.reader.core.id();
                let ann = above(&h);
                running_turn(&mut h, &ann, "t1");
                item_started(&mut h, "t1", "commandExecution", "c");
                accept_user(&h.state, "q-1", "a");
                announce(&h.reader.core, &h.state, &ann);
                item_completed(&mut h, "t1", "commandExecution", "c");
                let s = steer_next(&h).expect("the tool end did not release the held text");
                let first = s.request;
                let trace = s
                    .trace(SteerPhase::Steer)
                    .expect("the line is on but there is no material");
                assert!(matches!(issue(&h, s), Issued::Written));
                trace.write(agent);
                h.reader.handle_line(echo_line("t1", "q-1", "a").as_bytes());

                accept_user(&h.state, "q-2", "b");
                announce(&h.reader.core, &h.state, &ann);
                item_started(&mut h, "t1", "commandExecution", "c2");
                item_completed(&mut h, "t1", "commandExecution", "c2");
                let s = steer_next(&h).expect("the second tool end did not release the held text");
                let second = s.request;
                issue(&h, s);
                steer_refused_reply(&mut h, second);
                (first, second, agent.to_string())
            });
        let lines = captured.lock().unwrap();
        let got: Vec<(Vec<&str>, [&str; 3])> = lines
            .iter()
            .map(|l| {
                let at = |k: &str| l.get(k).map_or("-", String::as_str);
                (
                    l.keys().map(String::as_str).collect(),
                    [at("phase"), at("turn_id"), at("item_id")],
                )
            })
            .collect();
        let base = ["agent", "item_id", "message", "phase", "turn_id"];
        assert_eq!(
            got,
            vec![
                (base.to_vec(), ["tool_start", "t1", "c"]),
                (
                    vec!["agent", "held", "item_id", "message", "phase", "turn_id"],
                    ["tool_end", "t1", "c"]
                ),
                (
                    vec!["agent", "item_id", "message", "phase", "request", "turn_id"],
                    ["steer", "t1", "q-1"]
                ),
                (base.to_vec(), ["echo", "t1", "q-1"]),
                (base.to_vec(), ["tool_start", "t1", "c2"]),
                (
                    vec!["agent", "held", "item_id", "message", "phase", "turn_id"],
                    ["tool_end", "t1", "c2"]
                ),
                (base.to_vec(), ["refused", "t1", "q-2"]),
            ],
            "{lines:?}"
        );
        assert_eq!(lines[1]["held"], "1", "number of held user items");
        assert_eq!(lines[5]["held"], "1", "number of held user items");
        assert_eq!(lines[2]["request"], first.to_string());
        assert_ne!(first, second);
        let phases = [
            SteerPhase::ToolStart,
            SteerPhase::ToolEnd,
            SteerPhase::Steer,
            SteerPhase::WriteFailed,
            SteerPhase::Refused,
            SteerPhase::Echo,
            SteerPhase::HeldAgain,
        ];
        for l in lines.iter() {
            assert_eq!(l["agent"], agent, "{l:?}");
            let phase = phases
                .iter()
                .find(|p| p.name() == l["phase"])
                .expect("unknown phase");
            assert_eq!(l["message"], phase.message(), "{l:?}");
        }
    }

    #[test]
    fn a_steer_that_cannot_be_written_writes_one_write_failed_trace_line() {
        let captured = Arc::new(Mutex::new(Vec::new()));
        let (request, agent) =
            tracing::subscriber::with_default(TraceCapture(captured.clone()), || {
                let mut h = harness();
                let ann = above(&h);
                running_turn(&mut h, &ann, "t1");
                accept_user(&h.state, "q-1", "x");
                announce(&h.reader.core, &h.state, &ann);
                let s = steer_at_tool_end(&h).expect("steer");
                let request = s.request;
                let issued = issue_steer(&h.state, &h.pending, &h.reader.core, s, |_| {
                    Err(PtyError::WriteFailed("broken pipe".into()))
                });
                assert!(matches!(issued, Issued::Failed));
                (request, h.reader.core.id().to_string())
            });
        let lines = captured.lock().unwrap();
        assert_eq!(lines.len(), 1, "{lines:?}");
        let l = &lines[0];
        assert_eq!(
            l.keys().map(String::as_str).collect::<Vec<_>>(),
            ["agent", "item_id", "message", "phase", "request", "turn_id"]
        );
        assert_eq!(l["phase"], "write_failed");
        assert_eq!(l["turn_id"], "t1");
        assert_eq!(l["item_id"], "q-1");
        assert_eq!(l["request"], request.to_string());
        assert_eq!(l["agent"], agent);
    }

    // ── steer — handing over to the running turn · response pairing · ✕ on a handed-over item (ADR-0231 · TRD §5-5) ──
    //
    // Measured with the [`above`] bench (decision `Above`), steering exactly as production; the trigger is the tool end
    // ([`steer_at_tool_end`]).

    fn steer_params(steer: &Steer) -> Value {
        let v: Value = serde_json::from_str(&steer.line).unwrap();
        assert_eq!(v["method"], "turn/steer");
        assert_eq!(v["id"], steer.request);
        v["params"].clone()
    }

    fn issue(h: &Harness, steer: Steer) -> Issued {
        issue_steer(&h.state, &h.pending, &h.reader.core, steer, |_| Ok(()))
    }

    fn steer_refused_reply(h: &mut Harness, request: i64) {
        h.reader.handle_line(
            format!(
                r#"{{"id":{request},"error":{{"code":-32600,"message":"no active turn to steer"}}}}"#
            )
            .as_bytes(),
        );
    }

    fn steer_accepted_reply(h: &mut Harness, request: i64, turn: &str) {
        h.reader.handle_line(
            format!(r#"{{"id":{request},"result":{{"turnId":"{turn}"}}}}"#).as_bytes(),
        );
    }

    /// A turn opened with `head`, text typed while idle, and that `turn/start` response gave turn id `turn` — returns that turn's marker.
    /// The turn id signal the response set is left unpicked. `head` is already received (the vendor echoes the `turn/start` text as the
    /// turn's first item) — unreceived, the turn end would return it to the held place as the next turn's head. [`RecordingDecoder`] cannot
    /// make a receipt from an echo line, so it calls the link's receipt verb directly.
    fn running_turn(h: &mut Harness, ann: &Announcer, turn: &str) -> u64 {
        accept_user(&h.state, "head", "first");
        announce(&h.reader.core, &h.state, ann);
        let (request, seq) = open_turn(h);
        reply(h, request, turn);
        with_state(&h.state, |s| {
            s.take_delivered("head");
        });
        seq
    }

    fn cancel_ops(id: &str) -> [QueuedInputEvent; 2] {
        [
            QueuedInputEvent::CancelRequested { id: id.into() },
            QueuedInputEvent::CancelAnswered {
                id: id.into(),
                removed: false,
            },
        ]
    }

    /// ★Nothing is handed over while waiting for the turn id — neither on a tool end nor by `turn/started`★. When the `turn/start` response
    /// gives the turn id, text held meanwhile is handed over one at a time from the head without waiting for a tool end (`expectedTurnId`
    /// · `clientUserMessageId`), and the writer hands over the rest in turn (ADR-0235). No list events.
    #[test]
    fn a_held_item_is_steered_only_once_the_turn_start_reply_names_the_turn() {
        let mut h = harness();
        let ann = above(&h);
        accept_user(&h.state, "head", "first");
        announce(&h.reader.core, &h.state, &ann);
        let (request, seq) = open_turn(&h);
        accept_user(&h.state, "q-1", "second");
        accept_user(&h.state, "q-2", "third");
        announce(&h.reader.core, &h.state, &ann);
        assert!(
            steer_at_tool_end(&h).is_none(),
            "a steer went out before the turn id"
        );
        h.reader.handle_line(
            br#"{"method":"turn/started","params":{"threadId":"T","turn":{"id":"t1"}}}"#,
        );
        assert!(
            steer_next(&h).is_none(),
            "caught the turn id from turn/started"
        );

        reply(&mut h, request, "t1");
        let first = next_steer_job(&h, "no steer released by the turn id");
        let p = steer_params(&first);
        assert_eq!(p["threadId"], "T");
        assert_eq!(p["expectedTurnId"], "t1");
        assert_eq!(p["clientUserMessageId"], "q-1");
        assert_eq!(p["input"][0]["text"], "second");
        assert_eq!(
            stage_of(&h.state, "q-1"),
            Some(format!("in-flight gen={seq} turn=Some(\"t1\")"))
        );

        let second = next_steer_job(&h, "the second item was not handed over next");
        assert_eq!(steer_params(&second)["clientUserMessageId"], "q-2");
        assert!(
            steer_next(&h).is_none(),
            "a steer went out with nothing to hand over"
        );
        assert!(
            !steer_due(&h.state),
            "everything was handed over but the trigger remained"
        );
        assert_eq!(
            list_ops(&h.seen),
            vec![queued("q-1", "second"), queued("q-2", "third")],
            "the handover emitted a list event"
        );
    }

    #[test]
    fn a_tool_end_waits_for_an_unannounced_head_and_never_steers_mail_or_a_direct_item() {
        let mut h = harness();
        let ann = above(&h);
        running_turn(&mut h, &ann, "t1");
        accept_input(
            &h.state,
            Some("m-1".into()),
            b"mail".to_vec(),
            InputOrigin::Mail,
        )
        .unwrap();
        accept_user(&h.state, "q-1", "typed");
        assert!(
            steer_at_tool_end(&h).is_none(),
            "skipped the pre-announce head"
        );
        assert!(
            steer_due(&h.state),
            "the trigger was lowered in front of the pre-announce head"
        );
        announce(&h.reader.core, &h.state, &ann);
        let s =
            steer_next(&h).expect("the announced text was not handed over by the standing trigger");
        assert_eq!(steer_params(&s)["clientUserMessageId"], "q-1");
        assert_eq!(stage_of(&h.state, "m-1").as_deref(), Some("held"));
        assert!(steer_next(&h).is_none(), "mail went out as a steer");
        assert!(
            !steer_due(&h.state),
            "nothing to hand over but the trigger remained"
        );

        // Direct is the head of an idle moment, so it is never held mid-turn — even if held, it does not go out as a steer and is not skipped.
        let h = harness();
        let ann = above(&h);
        accept_user(&h.state, "d-1", "direct");
        accept_user(&h.state, "q-2", "queued");
        announce(&h.reader.core, &h.state, &ann);
        activate(&h.state, 5, Some("t5"));
        assert!(
            steer_at_tool_end(&h).is_none(),
            "Direct went out as a steer or was skipped"
        );
    }

    /// A steer's success response and deadline expiry do not move the item — not receipt (the echo is); it stays in the list at the
    /// handed-over stage, no list event, no turn end.
    #[test]
    fn a_steer_reply_or_its_deadline_moves_nothing_and_the_row_stays_listed() {
        for deadline in [false, true] {
            let mut h = harness();
            let ann = above(&h);
            let seq = running_turn(&mut h, &ann, "t1");
            accept_user(&h.state, "q-1", "x");
            announce(&h.reader.core, &h.state, &ann);
            let s = steer_at_tool_end(&h).expect("steer");
            let request = s.request;
            assert!(matches!(issue(&h, s), Issued::Written));
            assert_eq!(h.pending.len(), 1, "the waiting entry was not registered");
            if deadline {
                h.pending.forget(request);
                assert!(h.pending.register_at(
                    request,
                    Waiter::Steer {
                        id: "q-1".into(),
                        gen: seq,
                    },
                    method::TURN_STEER,
                    Instant::now(),
                ));
                sweep_deadlines(&h.state, &h.pending, &h.reader.core);
            } else {
                steer_accepted_reply(&mut h, request, "t1");
            }
            assert_eq!(h.pending.len(), 0, "deadline={deadline}");
            assert_eq!(
                stage_of(&h.state, "q-1"),
                Some(format!("in-flight gen={seq} turn=Some(\"t1\")")),
                "deadline={deadline}"
            );
            assert_eq!(
                list_ops(&h.seen),
                vec![queued("q-1", "x")],
                "deadline={deadline}"
            );
            assert_eq!(row_ids(&h), ["q-1"], "deadline={deadline}: left the list");
            assert_eq!(
                turn_seq(&h.state),
                Some(seq),
                "deadline={deadline}: the turn ended"
            );
        }
    }

    /// Steer error response → the item returns to `Held` (0 list events) and the turn is 「steer refused」 — even on a tool end both returned
    /// and newly arrived text stay held. At turn end the returned text opens the next `turn/start` in typing order and is handed over again
    /// when that turn id comes.
    #[test]
    fn a_refused_steer_is_held_again_and_that_turn_steers_nothing_more() {
        let mut h = harness();
        let ann = above(&h);
        running_turn(&mut h, &ann, "t1");
        accept_user(&h.state, "q-1", "second");
        announce(&h.reader.core, &h.state, &ann);
        let s = steer_at_tool_end(&h).expect("steer");
        let request = s.request;
        issue(&h, s);
        steer_refused_reply(&mut h, request);

        assert_eq!(stage_of(&h.state, "q-1").as_deref(), Some("held"));
        assert_eq!(
            list_ops(&h.seen),
            vec![queued("q-1", "second")],
            "the returned text was announced again"
        );
        assert!(
            steer_at_tool_end(&h).is_none(),
            "the returned text went out again in a turn marked 「steer refused」"
        );
        accept_user(&h.state, "q-2", "third");
        announce(&h.reader.core, &h.state, &ann);
        assert!(
            steer_at_tool_end(&h).is_none(),
            "a steer went out again in the refused turn"
        );

        h.reader
            .handle_line(completed_line("T", "t1", "completed").as_bytes());
        let Some(Job::Turn { id, seq, line, .. }) = take(&h) else {
            panic!("the turn/start of the returned text did not go out")
        };
        let v: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(
            v["params"]["clientUserMessageId"], "q-1",
            "not typing order"
        );
        assert!(h
            .pending
            .register(id, Waiter::TurnStart { seq }, method::TURN_START));
        reply(&mut h, id, "t2");
        let s = steer_next(&h).expect("hands over again on the next turn id");
        let p = steer_params(&s);
        assert_eq!(p["clientUserMessageId"], "q-2");
        assert_eq!(p["expectedTurnId"], "t2");
    }

    /// If the steer of a handed-over item ✕ reached is refused, it is withdrawn (`Dropped{Withdrawn}`), not held again — the vendor never
    /// saw it and the user removed it. The turn's 「steer refused」 mark stands.
    #[test]
    fn a_refused_steer_the_user_already_cancelled_is_withdrawn_instead_of_held() {
        let mut h = harness();
        let ann = above(&h);
        running_turn(&mut h, &ann, "t1");
        accept_user(&h.state, "q-1", "x");
        announce(&h.reader.core, &h.state, &ann);
        let s = steer_at_tool_end(&h).expect("steer");
        let request = s.request;
        issue(&h, s);
        assert_eq!(
            withdraw_item(&h.state, Some(&h.reader.core), "q-1"),
            Withdraw::TooLate
        );
        steer_refused_reply(&mut h, request);

        let mut want = vec![queued("q-1", "x")];
        want.extend(cancel_ops("q-1"));
        want.push(withdrawn("q-1"));
        assert_eq!(list_ops(&h.seen), want);
        assert_eq!(
            stage_of(&h.state, "q-1"),
            None,
            "withdrawn text remained in pending"
        );
        assert!(row_ids(&h).is_empty(), "it remained in the registry");
        accept_user(&h.state, "q-2", "y");
        announce(&h.reader.core, &h.state, &ann);
        assert!(
            steer_at_tool_end(&h).is_none(),
            "a steer went out again in the refused turn"
        );
    }

    #[test]
    fn a_steer_the_user_cancelled_that_cannot_be_written_is_withdrawn_instead_of_held() {
        let mut h = harness();
        let ann = above(&h);
        running_turn(&mut h, &ann, "t1");
        accept_user(&h.state, "q-1", "x");
        announce(&h.reader.core, &h.state, &ann);
        let s = steer_at_tool_end(&h).expect("steer");
        assert_eq!(
            withdraw_item(&h.state, Some(&h.reader.core), "q-1"),
            Withdraw::TooLate
        );
        let issued = issue_steer(&h.state, &h.pending, &h.reader.core, s, |_| {
            Err(PtyError::WriteFailed("broken pipe".into()))
        });
        assert!(matches!(issued, Issued::Failed));

        let mut want = vec![queued("q-1", "x")];
        want.extend(cancel_ops("q-1"));
        want.push(withdrawn("q-1"));
        assert_eq!(list_ops(&h.seen), want);
        assert!(
            handed(&h.seen).is_empty(),
            "emitted a mark for a steer that could not be written"
        );
        assert_eq!(
            stage_of(&h.state, "q-1"),
            None,
            "withdrawn text remained in pending"
        );
        assert!(row_ids(&h).is_empty(), "it remained in the registry");
        assert_eq!(
            h.pending.len(),
            0,
            "the waiting entry of the unwritten steer remained"
        );
        accept_user(&h.state, "q-2", "y");
        announce(&h.reader.core, &h.state, &ann);
        assert!(
            steer_at_tool_end(&h).is_none(),
            "a steer went out again in the turn whose write failed"
        );
    }

    /// ★Generation masking★ — a late steer response for an item already removed by echo moves nothing (not held again · no mark). A
    /// response of a waiting entry with another generation cannot touch the item either.
    #[test]
    fn a_late_steer_reply_moves_only_the_item_and_stage_it_was_sent_for() {
        let mut h = harness_with_real_decoder();
        let ann = above(&h);
        let seq = running_turn(&mut h, &ann, "t1");
        accept_user(&h.state, "q-1", "x");
        announce(&h.reader.core, &h.state, &ann);
        let s = steer_at_tool_end(&h).expect("steer");
        let request = s.request;
        issue(&h, s);
        h.reader.handle_line(echo_line("t1", "q-1", "x").as_bytes());
        assert_eq!(
            stage_of(&h.state, "q-1"),
            None,
            "received text remained in pending"
        );
        let before = list_ops(&h.seen);
        steer_refused_reply(&mut h, request);
        assert_eq!(
            stage_of(&h.state, "q-1"),
            None,
            "a late refusal held the received text again"
        );
        assert_eq!(list_ops(&h.seen), before, "a late refusal emitted an event");

        accept_user(&h.state, "q-2", "y");
        announce(&h.reader.core, &h.state, &ann);
        let s = steer_at_tool_end(&h).expect("a late refusal attached the mark to that turn");
        issue(&h, s);
        assert!(h.pending.register(
            900,
            Waiter::Steer {
                id: "q-2".into(),
                gen: seq + 7,
            },
            method::TURN_STEER
        ));
        steer_refused_reply(&mut h, 900);
        assert_eq!(
            stage_of(&h.state, "q-2"),
            Some(format!("in-flight gen={seq} turn=Some(\"t1\")")),
            "a response of another generation moved the item"
        );
    }

    #[test]
    fn a_stream_end_with_a_steer_waiting_hangs_nothing_and_leaves_the_item_to_the_core() {
        let mut h = harness();
        let ann = above(&h);
        running_turn(&mut h, &ann, "t1");
        accept_user(&h.state, "q-1", "x");
        announce(&h.reader.core, &h.state, &ann);
        let s = steer_at_tool_end(&h).expect("steer");
        issue(&h, s);
        h.seen.lock().unwrap().clear();

        drop(ReaderExit {
            state: h.state.clone(),
            pending: h.pending.clone(),
            agent: h.reader.core.id(),
        });
        assert_eq!(h.pending.len(), 0, "the steer waiting entry remained");
        assert!(
            trace(&h.seen).is_empty(),
            "the link emitted an event on stream close"
        );
        assert!(stage_of(&h.state, "q-1").is_some_and(|s| s.starts_with("in-flight")));
        h.reader.core.finish(TerminalReason::StreamClosed);
        assert_eq!(trace(&h.seen), ["dropped q-1 AgentEnded"]);
    }

    #[test]
    fn cancelling_a_handed_over_item_is_too_late_says_so_once_and_its_echo_still_delivers_it() {
        let mut h = harness_with_real_decoder();
        let ann = above(&h);
        let seq = running_turn(&mut h, &ann, "t1");
        accept_user(&h.state, "q-1", "x");
        announce(&h.reader.core, &h.state, &ann);
        let s = steer_at_tool_end(&h).expect("steer");
        issue(&h, s);

        assert_eq!(
            withdraw_item(&h.state, Some(&h.reader.core), "q-1"),
            Withdraw::TooLate
        );
        assert_eq!(
            withdraw_item(&h.state, Some(&h.reader.core), "q-1"),
            Withdraw::TooLate,
            "overlapping second ✕"
        );
        let mut want = vec![queued("q-1", "x")];
        want.extend(cancel_ops("q-1"));
        assert_eq!(list_ops(&h.seen), want, "the events of ✕ are not one set");
        assert_eq!(
            stage_of(&h.state, "q-1"),
            Some(format!("in-flight gen={seq} turn=Some(\"t1\")")),
            "✕ withdrew the handed-over text"
        );
        assert!(steer_at_tool_end(&h).is_none(), "sent again");

        h.reader.handle_line(echo_line("t1", "q-1", "x").as_bytes());
        let t = trace(&h.seen);
        assert_eq!(t[t.len() - 2..], ["delivered q-1", "bubble q-1"]);
        assert!(
            row_ids(&h).is_empty(),
            "received text remained in the registry"
        );
        assert_eq!(
            withdraw_item(&h.state, Some(&h.reader.core), "q-1"),
            Withdraw::NotHeld
        );
    }

    // ── Handed-over mark — a handed-over row is 「sent」 (✕ hidden) · a returned row is 「waiting」 (ADR-0231 · ADR-0235) ──

    fn sent_marks(h: &Harness) -> Vec<(String, bool)> {
        h.reader
            .core
            .queued_inputs()
            .snapshot()
            .0
            .into_iter()
            .map(|r| (r.id, r.sent))
            .collect()
    }

    #[test]
    fn a_written_steer_marks_its_row_sent_and_a_refusal_hands_it_back() {
        let mut h = harness();
        let ann = above(&h);
        running_turn(&mut h, &ann, "t1");
        accept_user(&h.state, "q-1", "x");
        accept_user(&h.state, "q-2", "y");
        announce(&h.reader.core, &h.state, &ann);
        h.seen.lock().unwrap().clear();
        let s = steer_at_tool_end(&h).expect("steer");
        let request = s.request;
        assert!(matches!(issue(&h, s), Issued::Written));
        assert_eq!(handed(&h.seen), ["sent q-1"]);
        assert_eq!(
            sent_marks(&h),
            [("q-1".to_string(), true), ("q-2".to_string(), false)]
        );
        let (rows, _) = h.reader.core.queued_inputs().snapshot();
        let listed = crate::queued_input::overlay_unconfirmed(rows, &[]);
        assert_eq!(
            listed[0].state.as_str(),
            "sent",
            "the list query cannot carry the handed-over row"
        );
        assert_eq!(listed[1].state.as_str(), "queued");

        steer_refused_reply(&mut h, request);
        assert_eq!(stage_of(&h.state, "q-1").as_deref(), Some("held"));
        assert_eq!(handed(&h.seen), ["sent q-1", "held q-1"]);
        assert_eq!(
            sent_marks(&h),
            [("q-1".to_string(), false), ("q-2".to_string(), false)]
        );
        assert!(
            list_ops(&h.seen).is_empty(),
            "the return emitted an outcome event"
        );
    }

    #[test]
    fn a_turn_end_without_the_echo_hands_a_listed_row_back_at_every_close() {
        for end in [
            End::Completed("completed"),
            End::Completed("inProgress"),
            End::Completed("interrupted"),
            End::Completed("failed"),
            End::Early("completed"),
            End::ErrorReply,
            End::WriteFailed,
            End::Deadline,
            End::Unreadable,
            End::LongTurnId,
        ] {
            let (h, _, _) = end_a_turn(Whose::Listed, end);
            assert_eq!(handed(&h.seen), ["held q-1"], "{end:?}");
            assert_eq!(sent_marks(&h), [("q-1".to_string(), false)], "{end:?}");
            for whose in [Whose::Direct, Whose::Mail] {
                let (h, _, _) = end_a_turn(whose, end);
                assert!(handed(&h.seen).is_empty(), "{whose:?}/{end:?}");
            }
        }

        let mut h = harness_with_real_decoder();
        let ann = above(&h);
        activate(&h.state, 0, Some("U0"));
        accept_user(&h.state, "q-1", "hi");
        with_state(&h.state, |s| s.turn = TurnState::Idle);
        announce(&h.reader.core, &h.state, &ann);
        let Some(Job::Turn { id, seq, item, .. }) = take(&h) else {
            panic!("turn/start did not go out")
        };
        assert_eq!(
            item.as_deref(),
            Some("q-1"),
            "did not hold the list item it carried"
        );
        assert!(h
            .pending
            .register(id, Waiter::TurnStart { seq }, method::TURN_START));
        publish_hand_over(&h.reader.core, &h.state, "q-1");
        assert_eq!(sent_marks(&h), [("q-1".to_string(), true)]);
        reply(&mut h, id, "U1");
        h.reader
            .handle_line(completed_line("T", "U1", "completed").as_bytes());
        assert_eq!(handed(&h.seen), ["sent q-1", "held q-1"]);
        assert_eq!(sent_marks(&h), [("q-1".to_string(), false)]);
    }

    /// The mark target `turn/start` carries — list items only (`None` for Direct · mail · below-floor items without an identifier). The value
    /// is read from the current stage, not by the caller — nothing is emitted for gone items · items outside the list.
    #[test]
    fn only_a_listed_carrier_is_marked_and_the_mark_is_read_from_the_stage() {
        let item_of = |h: &Harness| match take(h) {
            Some(Job::Turn { item, .. }) => item,
            _ => panic!("turn/start did not go out"),
        };
        let h = harness();
        above(&h);
        accept_user(&h.state, "d-1", "direct");
        announce(&h.reader.core, &h.state, &announcer());
        assert_eq!(item_of(&h), None, "attached a mark to Direct");
        publish_hand_over(&h.reader.core, &h.state, "d-1");
        assert!(
            handed(&h.seen).is_empty(),
            "emitted a mark for an item outside the list"
        );

        let h = harness();
        above(&h);
        accept_input(
            &h.state,
            Some("m-1".into()),
            b"mail".to_vec(),
            InputOrigin::Mail,
        )
        .unwrap();
        assert_eq!(item_of(&h), None, "attached a mark to mail");

        let h = harness();
        make_ready(&h.state, "T");
        settle_floor(&h.reader.core, &h.state, &announcer(), Floor::Below);
        accept_user(&h.state, "q-1", "below");
        assert_eq!(item_of(&h), None, "attached a mark to a below-floor item");

        let h = harness();
        let ann = above(&h);
        activate(&h.state, 0, Some("U0"));
        accept_user(&h.state, "q-1", "listed");
        announce(&h.reader.core, &h.state, &ann);
        publish_hand_over(&h.reader.core, &h.state, "q-1");
        assert_eq!(
            handed(&h.seen),
            ["held q-1"],
            "the mark of a held item is held"
        );
        with_state(&h.state, |s| {
            s.take_delivered("q-1");
        });
        publish_hand_over(&h.reader.core, &h.state, "q-1");
        assert_eq!(
            handed(&h.seen),
            ["held q-1"],
            "emitted a mark for a removed item"
        );
    }

    #[test]
    fn a_cancel_racing_a_push_is_not_removed_and_the_row_flips_to_sent() {
        let mut h = harness_with_real_decoder();
        let ann = above(&h);
        running_turn(&mut h, &ann, "t1");
        accept_user(&h.state, "q-1", "x");
        announce(&h.reader.core, &h.state, &ann);
        let s = steer_at_tool_end(&h).expect("steer");
        assert_eq!(
            withdraw_item(&h.state, Some(&h.reader.core), "q-1"),
            Withdraw::TooLate
        );
        assert!(matches!(issue(&h, s), Issued::Written));
        let (rows, _) = h.reader.core.queued_inputs().snapshot();
        assert_eq!(rows.len(), 1, "✕ removed the handed-over row from the list");
        assert!(rows[0].sent, "the handed-over row did not become 「sent」");
        assert_eq!(
            rows[0].phase,
            crate::queued_input::RowPhase::Cancelling {
                answer: crate::queued_input::CancelAnswer::NotRemoved,
                vendor_closed: false,
            }
        );
        h.reader.handle_line(echo_line("t1", "q-1", "x").as_bytes());
        assert!(
            row_ids(&h).is_empty(),
            "received text remained in the registry"
        );
    }

    /// A subscriber that flips that item's stage once on receiving the handed-over mark `sent` — imitates another thread's transition (the
    /// reader's turn end · the writer picking it for the next turn) taking the state lock between [`publish_hand_over`]'s read and emit.
    /// The emitting side releases the state lock before emitting, so taking it here is fine.
    struct FlipOnMark {
        id: SinkId,
        state: SharedState,
        item: &'static str,
        sent: bool,
        to: fn() -> Stage,
        done: AtomicBool,
    }

    impl OutputSink for FlipOnMark {
        fn send(&self, frame: OutputFrame<'_>) -> Result<(), SinkError> {
            let OutputPayload::Event(OutputEvent::QueuedInput(QueuedInputEvent::HandedOver {
                id,
                sent,
            })) = frame.payload
            else {
                return Ok(());
            };
            if id == self.item && *sent == self.sent && !self.done.swap(true, Ordering::AcqRel) {
                with_state(&self.state, |s| {
                    if let Some(p) = s
                        .pending
                        .iter_mut()
                        .find(|p| p.id.as_deref() == Some(self.item))
                    {
                        p.stage = (self.to)();
                    }
                });
            }
            Ok(())
        }
        fn sink_id(&self) -> SinkId {
            self.id
        }
    }

    fn flip_on_mark(h: &Harness, item: &'static str, sent: bool, to: fn() -> Stage) {
        h.reader.core.subscribe(Arc::new(FlipOnMark {
            id: SinkId::new_v4(),
            state: h.state.clone(),
            item,
            sent,
            to,
            done: AtomicBool::new(false),
        }));
    }

    fn listed_state(h: &Harness, id: &str) -> String {
        let (rows, _) = h.reader.core.queued_inputs().snapshot();
        crate::queued_input::overlay_unconfirmed(rows, &[])
            .into_iter()
            .find(|r| r.id == id)
            .map(|r| r.state.as_str().to_string())
            .expect("not in the list")
    }

    /// ★If the turn closes between reading and emitting the mark and returns it to the held place, the last mark is `held`★ — the writer
    /// read `sent` after the steer write but before emitting, the reader returned the text at a turn end without an echo. Without re-reading,
    /// the stale `sent` stays the ring's last mark and held text is drawn 「sent」 without ✕.
    #[test]
    fn a_sent_mark_overtaken_by_a_return_to_held_is_followed_by_held() {
        let mut h = harness();
        let ann = above(&h);
        running_turn(&mut h, &ann, "t1");
        accept_user(&h.state, "q-1", "x");
        announce(&h.reader.core, &h.state, &ann);
        let s = steer_at_tool_end(&h).expect("steer");
        flip_on_mark(&h, "q-1", true, || Stage::Held);
        assert!(matches!(issue(&h, s), Issued::Written));
        assert_eq!(handed(&h.seen), ["sent q-1", "held q-1"]);
        assert_eq!(sent_marks(&h), [("q-1".to_string(), false)]);
        assert_eq!(listed_state(&h, "q-1"), "queued");
    }

    /// Mirror — ★if the writer opens the next turn with that text between reading and emitting, the last mark is `sent`★. The reader read
    /// `held` after returning it, then before emitting the writer picked it with `turn/start`; without re-reading, handed-over text would
    /// be drawn 「waiting」 with ✕ again.
    #[test]
    fn a_held_mark_overtaken_by_the_next_turn_start_is_followed_by_sent() {
        let mut h = harness();
        let ann = above(&h);
        running_turn(&mut h, &ann, "t1");
        accept_user(&h.state, "q-1", "x");
        announce(&h.reader.core, &h.state, &ann);
        let s = steer_at_tool_end(&h).expect("steer");
        assert!(matches!(issue(&h, s), Issued::Written));
        // The next turn's (marker 1) `turn/start` carried it — the mark does not look at the generation, so the value only matches the shape.
        flip_on_mark(&h, "q-1", false, || Stage::InFlight {
            gen: 1,
            turn_id: None,
        });
        h.reader
            .handle_line(completed_line("T", "t1", "completed").as_bytes());
        assert_eq!(handed(&h.seen), ["sent q-1", "held q-1", "sent q-1"]);
        assert_eq!(sent_marks(&h), [("q-1".to_string(), true)]);
        assert_eq!(listed_state(&h, "q-1"), "sent");
    }

    #[test]
    fn a_cancel_while_the_turn_id_is_pending_withdraws_and_nothing_is_steered() {
        let mut h = harness();
        let ann = above(&h);
        accept_user(&h.state, "head", "first");
        announce(&h.reader.core, &h.state, &ann);
        let (request, _) = open_turn(&h);
        accept_user(&h.state, "q-1", "x");
        announce(&h.reader.core, &h.state, &ann);
        assert_eq!(
            withdraw_item(&h.state, Some(&h.reader.core), "q-1"),
            Withdraw::Withdrawn
        );
        reply(&mut h, request, "t1");
        assert!(
            !steer_due(&h.state),
            "the turn id set the trigger although there is no held text"
        );
        assert!(
            steer_at_tool_end(&h).is_none(),
            "withdrawn text was handed over"
        );
        assert_eq!(
            list_ops(&h.seen),
            vec![queued("q-1", "x"), withdrawn("q-1")]
        );
    }

    // ── Handover at a tool end (ADR-0231 · ADR-0235) ──

    /// ★One tool end hands over all held user text in typing order★ — one `turn/steer` per text · its own `clientUserMessageId` · the current
    /// turn id. Mail neither hands over nor blocks (held until the turn end). Handed-over text stays in the list (receipt is only the echo —
    /// the only list event is `Queued`) and the trigger is lowered once all is handed over.
    #[test]
    fn a_current_turn_tool_end_steers_every_held_user_item_in_typed_order_and_never_mail() {
        let mut h = harness();
        let ann = above(&h);
        let seq = running_turn(&mut h, &ann, "t1");
        item_started(&mut h, "t1", "commandExecution", "c");
        accept_user(&h.state, "q-1", "a");
        accept_input(
            &h.state,
            Some("m-1".into()),
            b"mail".to_vec(),
            InputOrigin::Mail,
        )
        .unwrap();
        accept_user(&h.state, "q-2", "b");
        accept_user(&h.state, "q-3", "c");
        announce(&h.reader.core, &h.state, &ann);
        assert!(steer_next(&h).is_none(), "handed over during the tool");

        item_completed(&mut h, "t1", "commandExecution", "c");
        let mut requests = Vec::new();
        for (id, text) in [("q-1", "a"), ("q-2", "b"), ("q-3", "c")] {
            let s = next_steer_job(&h, "the tool end did not hand over all the held text");
            let p = steer_params(&s);
            assert_eq!(p["clientUserMessageId"], id);
            assert_eq!(p["expectedTurnId"], "t1", "{id}");
            assert_eq!(p["input"][0]["text"], text, "{id}");
            assert_eq!(stage_of(&h.state, id), steered_stage(seq, "t1"), "{id}");
            requests.push(s.request);
            assert!(matches!(issue(&h, s), Issued::Written));
        }
        assert!(
            requests.windows(2).all(|w| w[0] < w[1]),
            "the request ids are not separate per text: {requests:?}"
        );
        assert!(steer_next(&h).is_none(), "mail went out as a steer");
        assert!(
            !steer_due(&h.state),
            "everything was handed over but the trigger remained"
        );
        assert_eq!(stage_of(&h.state, "m-1").as_deref(), Some("held"));
        assert_eq!(
            row_ids(&h),
            ["q-1", "q-2", "q-3"],
            "handed-over text left the list"
        );
        assert_eq!(
            list_ops(&h.seen),
            vec![queued("q-1", "a"), queued("q-2", "b"), queued("q-3", "c")],
            "the handover emitted a list event"
        );
    }

    #[test]
    fn a_cancel_during_a_tool_withdraws_the_text_and_the_tool_end_steers_nothing() {
        let mut h = harness();
        let ann = above(&h);
        running_turn(&mut h, &ann, "t1");
        item_started(&mut h, "t1", "commandExecution", "c");
        accept_user(&h.state, "q-1", "x");
        announce(&h.reader.core, &h.state, &ann);
        assert_eq!(
            withdraw_item(&h.state, Some(&h.reader.core), "q-1"),
            Withdraw::Withdrawn
        );
        assert_eq!(
            withdraw_item(&h.state, Some(&h.reader.core), "q-1"),
            Withdraw::NotHeld
        );

        item_completed(&mut h, "t1", "commandExecution", "c");
        assert!(steer_next(&h).is_none(), "withdrawn text was handed over");
        assert!(
            !steer_due(&h.state),
            "nothing to hand over but the trigger remained"
        );
        assert_eq!(h.pending.len(), 0, "a vendor request went out");
        assert_eq!(
            list_ops(&h.seen),
            vec![queued("q-1", "x"), withdrawn("q-1")]
        );
    }

    /// ★Without a tool end it is held until the turn end★ — text typed **after** the tool ended is not handed over if no tool follows, and
    /// stays with 0 list events. At turn end the oldest text opens the next turn with `turn/start`, and when that turn id comes the rest go
    /// into it (no wait for a tool end — remaining text gathers into one turn).
    #[test]
    fn text_typed_with_no_tool_afterwards_waits_for_the_turn_end_and_the_rest_ride_the_next_turn_once_its_id_is_known(
    ) {
        let mut h = harness();
        let ann = above(&h);
        running_turn(&mut h, &ann, "t1");
        item_started(&mut h, "t1", "commandExecution", "c");
        item_completed(&mut h, "t1", "commandExecution", "c");
        assert!(steer_next(&h).is_none());
        assert!(!steer_due(&h.state));
        accept_user(&h.state, "q-1", "a");
        accept_user(&h.state, "q-2", "b");
        accept_user(&h.state, "q-3", "c");
        announce(&h.reader.core, &h.state, &ann);
        item_started(&mut h, "t1", "agentMessage", "m");
        item_completed(&mut h, "t1", "agentMessage", "m");
        assert!(steer_next(&h).is_none(), "handed over without a tool end");

        h.reader
            .handle_line(completed_line("T", "t1", "completed").as_bytes());
        assert_eq!(turn_seq(&h.state), None, "t1 did not end");
        for id in ["q-1", "q-2", "q-3"] {
            assert_eq!(stage_of(&h.state, id).as_deref(), Some("held"), "{id}");
        }
        assert_eq!(
            list_ops(&h.seen),
            vec![queued("q-1", "a"), queued("q-2", "b"), queued("q-3", "c")]
        );

        let (request, seq, p) = opens(&h).expect("the next turn did not open");
        assert_eq!(p["clientUserMessageId"], "q-1");
        assert!(steer_next(&h).is_none(), "handed over before the turn id");
        reply(&mut h, request, "t2");
        for id in ["q-2", "q-3"] {
            let s = next_steer_job(&h, "the turn id did not hand over the remaining text");
            let p = steer_params(&s);
            assert_eq!(p["clientUserMessageId"], id);
            assert_eq!(p["expectedTurnId"], "t2", "{id}");
            assert_eq!(stage_of(&h.state, id), steered_stage(seq, "t2"), "{id}");
        }
        assert!(steer_next(&h).is_none());
        assert!(
            !steer_due(&h.state),
            "everything was handed over but the trigger remained"
        );
        assert!(
            take(&h).is_none(),
            "turn/start went out again on the running turn"
        );
    }

    /// ★The tool-end trigger is lowered with the turn★ — if the turn ends before the writer sees a trigger set by a tool end with no held text
    /// (during a blocking write) and the writer opens the next turn, text typed after that turn id came waits for that turn's tool end — it is
    /// not handed over by the previous turn's trigger (it would go in without a tool end, recorded but never answered).
    #[test]
    fn a_tool_end_armed_just_before_its_turn_closes_does_not_steer_text_typed_into_the_next_turn() {
        let mut h = harness();
        let ann = above(&h);
        running_turn(&mut h, &ann, "t1");
        item_started(&mut h, "t1", "commandExecution", "c");
        item_completed(&mut h, "t1", "commandExecution", "c");
        assert!(
            steer_due(&h.state),
            "a tool end with no held text also sets the trigger"
        );
        h.reader
            .handle_line(completed_line("T", "t1", "completed").as_bytes());
        assert!(
            !steer_due(&h.state),
            "the trigger of the closed turn remained"
        );

        accept_user(&h.state, "d-1", "next");
        announce(&h.reader.core, &h.state, &ann);
        let (request, seq, p) = opens(&h).expect("the next turn did not open");
        assert_eq!(p["clientUserMessageId"], "d-1");
        reply(&mut h, request, "t2");
        accept_user(&h.state, "q-1", "typed");
        announce(&h.reader.core, &h.state, &ann);
        assert!(
            steer_next(&h).is_none(),
            "handed over without a tool end by the previous turn's trigger"
        );
        assert_eq!(stage_of(&h.state, "q-1").as_deref(), Some("held"));

        item_completed(&mut h, "t2", "commandExecution", "c2");
        let s = next_steer_job(&h, "that turn's tool end did not hand over the held text");
        assert_eq!(steer_params(&s)["expectedTurnId"], "t2");
        assert_eq!(stage_of(&h.state, "q-1"), steered_stage(seq, "t2"));
    }

    /// ★The tool-end fixture (M7 trial 1 — measured codex-cli 0.156.1)★: text typed while a tool runs is held and handed over at the tool's
    /// `item/completed` (request id 3 of the fixture); the vendor echoes it in the same turn and answers — receipt (`Delivered`) · 0 waiting
    /// entries. Request ids match the fixture's response ids (`fixtures/README.md`).
    #[test]
    fn the_tool_end_fixture_holds_text_typed_during_the_tool_and_delivers_it_in_that_turn() {
        const THREAD: &str = "01a0d4b3-6048-7b41-aa4b-eec199dd1c40";
        const TURN: &str = "01a0d4b3-60f6-78e1-80fb-bf1ac3ecd0c7";
        const CARRIER: &str = "e53bb68a-aea6-421e-b04f-45caea0fc6f1";
        const TYPED: &str = "d05ef2f8-fa23-4c56-b379-b5aba4dc39e1";
        let lines: Vec<&str> = include_str!("fixtures/tool_end_m7.jsonl").lines().collect();
        let feed = |h: &mut Harness, range: std::ops::RangeInclusive<usize>| {
            for n in range {
                h.reader.handle_line(lines[n - 1].as_bytes());
            }
        };
        let mut h = harness_with_real_decoder();
        make_ready(&h.state, THREAD);
        let ann = announcer();
        settle_floor(&h.reader.core, &h.state, &ann, Floor::Above);
        accept_user(&h.state, CARRIER, "run the command");
        announce(&h.reader.core, &h.state, &ann);
        h.next_id.store(2, Ordering::Relaxed);
        let (request, _, _) = opens(&h).expect("turn/start");
        assert_eq!(request, 2);
        feed(&mut h, 1..=6);
        assert_eq!(turn_id_of(&h.state).as_deref(), Some(TURN));

        accept_user(&h.state, TYPED, "Also reply with the word KIWI1.");
        announce(&h.reader.core, &h.state, &ann);
        assert!(steer_next(&h).is_none(), "handed over before the tool end");
        h.next_id.store(3, Ordering::Relaxed);
        feed(&mut h, 7..=7);
        let s = next_steer_job(&h, "the tool end did not release the held text");
        assert_eq!(s.request, 3);
        assert_eq!(steer_params(&s)["expectedTurnId"], TURN);
        issue(&h, s);

        feed(&mut h, 8..=19);
        assert_eq!(turn_seq(&h.state), None, "the turn did not end");
        let typed: Vec<_> = list_ops(&h.seen)
            .into_iter()
            .filter(|op| {
                matches!(op, QueuedInputEvent::Queued { id, .. } | QueuedInputEvent::Delivered { id } if id == TYPED)
            })
            .collect();
        assert_eq!(
            typed,
            vec![
                queued(TYPED, "Also reply with the word KIWI1."),
                QueuedInputEvent::Delivered { id: TYPED.into() }
            ]
        );
        assert!(
            row_ids(&h).is_empty(),
            "received text remained in the registry"
        );
        assert_eq!(h.pending.len(), 0, "a waiting entry remained");
        assert!(
            take(&h).is_none(),
            "turn/start went out after the ended turn"
        );
    }

    /// ★If the steer cannot be written, the text returns to the held place★ (user decision 2026-09-26 · ADR-0235 — not erased) · 0 list
    /// events · the waiting entry is removed and the turn keeps running (no halt trigger). The turn gets the 「steer refused」 mark, so the
    /// next tool end of the turn does not rewrite it (no retry loop), and after the turn ends it goes out **once** with `turn/start` at the
    /// next Idle.
    #[test]
    fn a_steer_that_cannot_be_written_is_held_not_retried_in_that_turn_and_sent_once_at_the_next_idle(
    ) {
        let mut h = harness();
        let ann = above(&h);
        let seq = running_turn(&mut h, &ann, "t1");
        accept_user(&h.state, "q-1", "x");
        announce(&h.reader.core, &h.state, &ann);
        let s = steer_at_tool_end(&h).expect("steer");
        let issued = issue_steer(&h.state, &h.pending, &h.reader.core, s, |_| {
            Err(PtyError::WriteFailed("broken pipe".into()))
        });
        assert!(matches!(issued, Issued::Failed));
        assert_eq!(
            h.pending.len(),
            0,
            "the waiting entry of the unwritten steer remained"
        );
        assert_eq!(
            list_ops(&h.seen),
            vec![queued("q-1", "x")],
            "the write failure emitted a list event"
        );
        assert_eq!(stage_of(&h.state, "q-1").as_deref(), Some("held"));
        assert_eq!(row_ids(&h), ["q-1"], "the write failure changed the list");
        assert_eq!(
            turn_seq(&h.state),
            Some(seq),
            "the steer write failure ended the turn"
        );
        assert!(!halted(&h.state), "the steer write failure set a halt");

        item_started(&mut h, "t1", "commandExecution", "c");
        item_completed(&mut h, "t1", "commandExecution", "c");
        assert!(
            steer_next(&h).is_none(),
            "a tool end of the same turn wrote the unwritten text again"
        );
        h.reader
            .handle_line(completed_line("T", "t1", "completed").as_bytes());
        let v: Value = serde_json::from_str(&turn_line(take(&h))).unwrap();
        assert_eq!(v["params"]["clientUserMessageId"], "q-1");
        assert!(take(&h).is_none(), "went out twice");
    }

    /// ★Production steers at the tool end★ — in a state built without test seams (decision `Above`), text typed in the running turn is held,
    /// and when our turn's tool-end line passes the reader the next job the writer picks ([`next_job`]) is that text's `turn/steer`. A
    /// below-floor incarnation is today's path — it stays held and goes with `turn/start` after the turn end.
    #[test]
    fn production_steers_a_held_item_through_the_writer_after_a_tool_end() {
        let mut h = harness();
        let ann = above(&h);
        let seq = running_turn(&mut h, &ann, "t1");
        accept_user(&h.state, "q-1", "x");
        announce(&h.reader.core, &h.state, &ann);
        item_started(&mut h, "t1", "commandExecution", "c");
        assert!(steer_next(&h).is_none(), "handed over before the tool end");
        item_completed(&mut h, "t1", "commandExecution", "c");
        let s = next_steer_job(&h, "the tool end did not release the held text");
        let p = steer_params(&s);
        assert_eq!(p["clientUserMessageId"], "q-1");
        assert_eq!(p["expectedTurnId"], "t1");
        assert_eq!(stage_of(&h.state, "q-1"), steered_stage(seq, "t1"));

        let mut h = harness();
        make_ready(&h.state, "T");
        settle_floor(&h.reader.core, &h.state, &announcer(), Floor::Below);
        accept_user(&h.state, "head", "first");
        let (request, _) = open_turn(&h);
        accept_user(&h.state, "q-1", "x");
        reply(&mut h, request, "t1");
        assert!(
            steer_next(&h).is_none(),
            "a below-floor incarnation steered on the turn id"
        );
        item_started(&mut h, "t1", "commandExecution", "c");
        item_completed(&mut h, "t1", "commandExecution", "c");
        assert!(
            steer_next(&h).is_none(),
            "a below-floor incarnation steered at the tool end"
        );
        h.reader
            .handle_line(completed_line("T", "t1", "completed").as_bytes());
        let v: Value = serde_json::from_str(&turn_line(take(&h))).unwrap();
        assert_eq!(v["params"]["input"][0]["text"], "x");
        assert!(v["params"].get("clientUserMessageId").is_none());
    }

    // ── First user message echo → session id latch (ADR-0226 amendment · user decision 2026-09-26) ──
    //
    // ★Plugged in as production★: the handshake's record port = latch receipt, the reader's first-turn port = the same latch. This mode's
    //   session does not count submissions, so commit happens only at the echo.

    /// Real translator bench + latch — after the handshake finished (thread id handed to the latch, ready). Returns the values the commit
    /// port received.
    fn harness_with_latch() -> (Harness, Arc<Mutex<Vec<String>>>) {
        let mut h = harness_with_real_decoder();
        let commits = Arc::new(Mutex::new(Vec::new()));
        let port: SessionIdSink = {
            let commits = commits.clone();
            Arc::new(move |raw: &str| commits.lock().unwrap().push(raw.to_owned()))
        };
        let latch = crate::session_id_latch::SessionIdLatch::new(AgentId::new_v4(), 1, port);
        record_session_id(
            &h.state,
            &AtomicBool::new(false),
            Some(&latch.offer_sink()),
            "T",
        )
        .expect("the handshake's record");
        make_ready(&h.state, "T");
        h.reader.first_turn = Some(latch.first_turn_sink());
        (h, commits)
    }

    fn anonymous_echo_line(method_name: &str, thread: &str, turn: &str, item_id: &str) -> String {
        serde_json::json!({
            "method": method_name,
            "params": {"threadId": thread, "turnId": turn,
                       "item": {"type": "userMessage", "id": item_id,
                                "content": [{"type": "text", "text": "letter"}]}},
        })
        .to_string()
    }

    #[test]
    fn the_thread_id_persists_at_the_first_user_message_echo_and_only_once() {
        let (mut h, commits) = harness_with_latch();
        assert!(
            commits.lock().unwrap().is_empty(),
            "persisted by the handshake alone — the 0-turn id of an incarnation turned off without input remains"
        );

        accept_user(&h.state, "u-1", "hi");
        activate(&h.state, 1, Some("U1"));
        h.reader.handle_line(progress_line("T", "U1").as_bytes());
        assert!(
            commits.lock().unwrap().is_empty(),
            "persisted before the echo — having sent is not enough for the conversation to have a turn"
        );

        h.reader
            .handle_line(echo_line("U1", "u-1", "hi").as_bytes());
        assert_eq!(commits.lock().unwrap().as_slice(), ["T"]);

        h.reader.handle_line(
            anonymous_echo_line(method::ITEM_COMPLETED, "T", "U1", "codex-u-1").as_bytes(),
        );
        h.reader
            .handle_line(echo_line("U2", "u-2", "again").as_bytes());
        assert_eq!(
            commits.lock().unwrap().as_slice(),
            ["T"],
            "the completion of the same item · the next turn's echo committed again"
        );
    }

    #[test]
    fn an_echo_without_our_client_id_persists_the_thread_id_too() {
        let (mut h, commits) = harness_with_latch();
        accept_input(&h.state, None, b"letter".to_vec(), InputOrigin::Mail).expect("accepts");
        activate(&h.state, 1, None);

        h.reader.handle_line(
            anonymous_echo_line(method::ITEM_STARTED, "T", "U1", "codex-1").as_bytes(),
        );

        assert_eq!(commits.lock().unwrap().as_slice(), ["T"]);
    }

    /// (d)·(f) — lines that are not a live user message echo open no persistence: an echo declaring another thread · an agent answer item · a
    ///   turn end · a **response** line carrying a user message (the door history restore rides).
    /// ★The last echo keeps that decision from being empty★ — the same reader does count a real echo.
    #[test]
    fn nothing_but_a_live_user_message_echo_of_our_thread_persists_the_thread_id() {
        let (mut h, commits) = harness_with_latch();
        accept_user(&h.state, "u-1", "hi");
        activate(&h.state, 1, Some("U1"));

        let agent_item = |method_name: &str| {
            serde_json::json!({
                "method": method_name,
                "params": {"threadId": "T", "turnId": "U1",
                           "item": {"type": "agentMessage", "id": "a-1", "text": "answer"}},
            })
            .to_string()
        };
        let history_page = serde_json::json!({
            "id": 99,
            "result": {"data": [{"turnId": "old", "item": {"type": "userMessage", "id": "h-1",
                                 "content": [{"type": "text", "text": "old question"}]}}]},
        })
        .to_string();
        for line in [
            anonymous_echo_line(method::ITEM_STARTED, "OTHER", "U9", "x-1"),
            agent_item(method::ITEM_STARTED),
            agent_item(method::ITEM_COMPLETED),
            history_page,
            completed_line("T", "U1", "completed"),
        ] {
            h.reader.handle_line(line.as_bytes());
        }
        assert!(
            commits.lock().unwrap().is_empty(),
            "a line that is not an echo opened persistence: {:?}",
            commits.lock().unwrap()
        );

        h.reader.handle_line(
            anonymous_echo_line(method::ITEM_STARTED, "T", "U2", "codex-2").as_bytes(),
        );
        assert_eq!(commits.lock().unwrap().as_slice(), ["T"]);
    }

    /// A reader without a first-turn port (none given by the assembly point) handles echoes as usual — there is just nowhere to call.
    #[test]
    fn a_reader_without_a_first_turn_port_handles_the_echo_as_usual() {
        let mut h = harness_with_real_decoder();
        make_ready(&h.state, "T");
        activate(&h.state, 1, None);

        h.reader.handle_line(
            anonymous_echo_line(method::ITEM_STARTED, "T", "U1", "codex-1").as_bytes(),
        );

        assert!(h.reader.first_turn.is_none());
        assert!(
            h.seen
                .lock()
                .unwrap()
                .iter()
                .any(|e| matches!(e, OutputEvent::Structured { .. })),
            "the echoed bubble did not go up"
        );
    }

    /// The first-turn port is called outside the marking — the reader runs inside the pump's containment, but the latch commit changes the
    /// shared profile table.
    // ADR-0292
    #[test]
    fn the_first_turn_port_runs_outside_the_contain_mark() {
        use engram_dashboard_base::panic::is_contained;

        let mut h = harness_with_real_decoder();
        make_ready(&h.state, "T");
        activate(&h.state, 1, None);
        let marks = Arc::new(Mutex::new(Vec::new()));
        let port: FirstTurnSink = {
            let marks = marks.clone();
            Arc::new(move || marks.lock().unwrap().push(is_contained()))
        };
        h.reader.first_turn = Some(port);

        let inside = contain(std::panic::AssertUnwindSafe(|| {
            h.reader.handle_line(
                anonymous_echo_line(method::ITEM_STARTED, "T", "U1", "codex-1").as_bytes(),
            );
            is_contained()
        }))
        .expect("no panic");

        assert!(
            inside,
            "test premise — the reader is inside the marking even after calling the port"
        );
        assert_eq!(*marks.lock().unwrap(), vec![false]);
    }

    // ── Returning to the held place at the turn end (ADR-0231 · TRD §5-5) ──────────────────────────
    //
    // The turn end does not wait for the steer response — the [`above`] bench steers exactly as production.

    fn steer_one(h: &Harness, ann: &Announcer, id: &str, text: &str) -> i64 {
        accept_user(&h.state, id, text);
        announce(&h.reader.core, &h.state, ann);
        let s = steer_at_tool_end(h).expect("the steer did not go out");
        let request = s.request;
        assert!(matches!(issue(h, s), Issued::Written));
        request
    }

    fn steered_stage(seq: u64, turn: &str) -> Option<String> {
        Some(format!("in-flight gen={seq} turn=Some(\"{turn}\")"))
    }

    /// Does `act` wake the side sleeping on the state condition variable? ★The sleeper signals readiness while holding the lock and releases
    /// it as it falls asleep★ — so when `act` gets the lock the sleeper is already asleep (no window to miss the wake-up).
    fn wakes_a_waiter(state: &SharedState, act: impl FnOnce()) -> bool {
        let (ready_tx, ready_rx) = mpsc::channel();
        let waiter = {
            let state = state.clone();
            std::thread::spawn(move || {
                let (lock, cv) = &*state;
                let guard = lock.lock().unwrap_or_else(|p| p.into_inner());
                ready_tx.send(()).unwrap();
                let (_guard, timeout) = cv
                    .wait_timeout(guard, Duration::from_secs(5))
                    .unwrap_or_else(|p| p.into_inner());
                !timeout.timed_out()
            })
        };
        ready_rx.recv().unwrap();
        act();
        waiter.join().unwrap()
    }

    /// ★The window between the turn end and the steer response★ — whether −32600 (no turn running) comes after or before `turn/completed`
    /// (M10 — no order assumed), the text remains `Held` and goes out **exactly once** with `turn/start` · 0 list events · the turn end does
    /// not wait for that response · a refusal of the old request after it went out again is a no-op (generation masking). `failed` also holds
    /// the same text — text returned by a mid-turn refusal stays, text before the refusal came is returned by the end (user decision
    /// 2026-09-26 · ADR-0235). That `turn/start` comes after the text typed after the halt (halt after error).
    #[test]
    fn a_refused_steer_is_held_and_sent_once_whether_its_error_comes_before_or_after_the_turn_end()
    {
        for (status, error_first) in [
            ("completed", false),
            ("completed", true),
            ("failed", false),
            ("failed", true),
        ] {
            let at = format!("{status} error_first={error_first}");
            let mut h = harness();
            let ann = above(&h);
            running_turn(&mut h, &ann, "t1");
            let request = steer_one(&h, &ann, "q-1", "second");
            if error_first {
                steer_refused_reply(&mut h, request);
            }
            h.reader
                .handle_line(completed_line("T", "t1", status).as_bytes());
            assert_eq!(
                stage_of(&h.state, "q-1").as_deref(),
                Some("held"),
                "{at}: did not return to the held place at the turn end"
            );
            let mut listed = vec![queued("q-1", "second")];
            if status == "failed" {
                // Halt after error — text held before the halt does not go out on its own and enters as the head of the turn of text typed after the halt (AC24).
                assert!(take(&h).is_none(), "{at}: a turn opened during the halt");
                accept_user(&h.state, "u-2", "later");
                announce(&h.reader.core, &h.state, &ann);
                listed.push(queued("u-2", "later"));
            }
            let v: Value = serde_json::from_str(&turn_line(take(&h))).unwrap();
            assert_eq!(v["params"]["clientUserMessageId"], "q-1", "{at}");
            let resent = stage_of(&h.state, "q-1");
            if !error_first {
                steer_refused_reply(&mut h, request);
                assert_eq!(
                    stage_of(&h.state, "q-1"),
                    resent,
                    "{at}: the old request's refusal moved the text handed over again"
                );
            }
            assert!(take(&h).is_none(), "{at}: turn/start went out twice");
            assert_eq!(list_ops(&h.seen), listed, "{at}: a list event happened");
        }
    }

    /// Text of a steer with no response until the turn end returns to the held place at the turn end (0 list events · stays in the list) and
    /// rides the next `turn/start` at once without waiting for that response. The old request's success response · deadline · refusal after
    /// it went out again do not move that text (generation masking).
    #[test]
    fn a_steer_answered_only_after_its_turn_ended_rides_the_next_turn_once() {
        for deadline in [false, true] {
            let at = format!("deadline={deadline}");
            let mut h = harness();
            let ann = above(&h);
            let seq = running_turn(&mut h, &ann, "t1");
            let request = steer_one(&h, &ann, "q-1", "x");
            h.reader
                .handle_line(completed_line("T", "t1", "completed").as_bytes());
            assert_eq!(stage_of(&h.state, "q-1").as_deref(), Some("held"), "{at}");
            let v: Value = serde_json::from_str(&turn_line(take(&h))).unwrap();
            assert_eq!(v["params"]["clientUserMessageId"], "q-1", "{at}");
            assert!(take(&h).is_none(), "{at}: went out twice");
            let resent = stage_of(&h.state, "q-1");
            if deadline {
                h.pending.forget(request);
                assert!(h.pending.register_at(
                    request,
                    Waiter::Steer {
                        id: "q-1".into(),
                        gen: seq,
                    },
                    method::TURN_STEER,
                    Instant::now(),
                ));
                sweep_deadlines(&h.state, &h.pending, &h.reader.core);
            } else {
                steer_accepted_reply(&mut h, request, "t1");
            }
            assert_eq!(
                stage_of(&h.state, "q-1"),
                resent,
                "{at}: the old request's response moved the text handed over again"
            );
            assert_eq!(list_ops(&h.seen), vec![queued("q-1", "x")], "{at}");
            assert_eq!(row_ids(&h), ["q-1"], "{at}: left the list");
            assert!(
                take(&h).is_none(),
                "{at}: went out again after the late response"
            );

            assert!(h.pending.register(
                900,
                Waiter::Steer {
                    id: "q-1".into(),
                    gen: seq,
                },
                method::TURN_STEER
            ));
            steer_refused_reply(&mut h, 900);
            assert_eq!(
                stage_of(&h.state, "q-1"),
                resent,
                "{at}: a late response of the old generation moved the item handed over again"
            );
            assert_eq!(list_ops(&h.seen).len(), 1, "{at}");
        }
    }

    /// Text handed over as a steer in an interrupted turn also returns to the held place without an echo and rides the next turn at once —
    /// interruption does not set the halt. Same whether the response was received; a late response of the old request does not move the
    /// re-handed-over text.
    #[test]
    fn a_steered_item_without_its_echo_returns_to_held_when_the_turn_is_interrupted() {
        for replied in [true, false] {
            let at = format!("replied={replied}");
            let mut h = harness();
            let ann = above(&h);
            running_turn(&mut h, &ann, "t1");
            let request = steer_one(&h, &ann, "q-1", "x");
            if replied {
                steer_accepted_reply(&mut h, request, "t1");
            }
            h.reader
                .handle_line(completed_line("T", "t1", "interrupted").as_bytes());
            assert_eq!(stage_of(&h.state, "q-1").as_deref(), Some("held"), "{at}");
            assert!(
                !with_state(&h.state, |s| s.halted),
                "{at}: the interruption set the halt"
            );
            assert_eq!(list_ops(&h.seen), vec![queued("q-1", "x")], "{at}");
            let v: Value = serde_json::from_str(&turn_line(take(&h))).unwrap();
            assert_eq!(v["params"]["clientUserMessageId"], "q-1", "{at}");
            assert!(take(&h).is_none(), "{at}: went out twice");
            let resent = stage_of(&h.state, "q-1");
            if !replied {
                steer_refused_reply(&mut h, request);
                assert_eq!(
                    stage_of(&h.state, "q-1"),
                    resent,
                    "{at}: the old request's refusal moved the text handed over again"
                );
            }
        }
    }

    #[test]
    fn a_cancelled_steer_whose_turn_ends_without_its_echo_is_withdrawn_at_the_end() {
        for how in ["refused", "accepted"] {
            let mut h = harness();
            let ann = above(&h);
            running_turn(&mut h, &ann, "t1");
            let request = steer_one(&h, &ann, "q-1", "x");
            assert_eq!(
                withdraw_item(&h.state, Some(&h.reader.core), "q-1"),
                Withdraw::TooLate
            );
            h.reader
                .handle_line(completed_line("T", "t1", "completed").as_bytes());
            let mut want = vec![queued("q-1", "x")];
            want.extend(cancel_ops("q-1"));
            want.push(withdrawn("q-1"));
            assert_eq!(
                list_ops(&h.seen),
                want,
                "{how}: not withdrawn at the turn end"
            );
            assert_eq!(stage_of(&h.state, "q-1"), None, "{how}");
            assert!(row_ids(&h).is_empty(), "{how}: it remained in the registry");
            match how {
                "refused" => steer_refused_reply(&mut h, request),
                _ => steer_accepted_reply(&mut h, request, "t1"),
            }
            assert_eq!(
                list_ops(&h.seen),
                want,
                "{how}: the response emitted an event"
            );
            assert!(take(&h).is_none(), "{how}: withdrawn text went out");
        }
    }

    #[test]
    fn steers_returned_at_the_close_go_in_typed_order_without_waiting_for_their_replies() {
        let mut h = harness();
        let ann = above(&h);
        running_turn(&mut h, &ann, "t1");
        let first = steer_one(&h, &ann, "q-1", "a");
        let second = steer_one(&h, &ann, "q-2", "b");
        h.reader
            .handle_line(completed_line("T", "t1", "completed").as_bytes());
        let (request, _, p) =
            opens(&h).expect("the turn did not open because it waited for responses");
        assert_eq!(p["clientUserMessageId"], "q-1");
        steer_refused_reply(&mut h, second);
        steer_refused_reply(&mut h, first);
        assert_eq!(
            stage_of(&h.state, "q-2").as_deref(),
            Some("held"),
            "an old refusal moved the held text"
        );
        reply(&mut h, request, "t2");
        let s = steer_next(&h).expect("the turn id did not release the later text");
        assert_eq!(steer_params(&s)["clientUserMessageId"], "q-2");
        assert_eq!(
            list_ops(&h.seen),
            vec![queued("q-1", "a"), queued("q-2", "b")]
        );
    }

    #[test]
    fn a_steer_write_failure_after_its_turn_ended_leaves_the_returned_item_alone() {
        let mut h = harness();
        let ann = above(&h);
        running_turn(&mut h, &ann, "t1");
        accept_user(&h.state, "q-1", "x");
        announce(&h.reader.core, &h.state, &ann);
        let s = steer_at_tool_end(&h).expect("steer");
        h.reader
            .handle_line(completed_line("T", "t1", "completed").as_bytes());
        assert_eq!(stage_of(&h.state, "q-1").as_deref(), Some("held"));
        let issued = issue_steer(&h.state, &h.pending, &h.reader.core, s, |_| {
            Err(PtyError::WriteFailed("broken pipe".into()))
        });
        assert!(matches!(issued, Issued::Failed));
        assert_eq!(stage_of(&h.state, "q-1").as_deref(), Some("held"));
        assert_eq!(list_ops(&h.seen), vec![queued("q-1", "x")]);
        assert_eq!(with_state(&h.state, |s| s.steer_refused), None);
        let v: Value = serde_json::from_str(&turn_line(take(&h))).unwrap();
        assert_eq!(v["params"]["clientUserMessageId"], "q-1");
    }

    /// ★A receipt after the turn end★ — if the echo of a steer with no response comes after `turn/completed` (the text has returned to the
    /// held place), it closes as received (leaves the list, and the vendor bubble) · 0 empty `turn/start`. No open turn, so **a turn
    /// boundary after the receipt** stands (turns off the frontend waiting indicator) — through the observation-free door, so the turn
    /// table stays as is. A late success response emits nothing.
    #[test]
    fn a_late_echo_after_the_close_delivers_the_returned_item_and_its_late_reply_changes_nothing() {
        let (mut h, turns, id) = harness_with_facts();
        let ann = above(&h);
        running_turn(&mut h, &ann, "t1");
        h.reader
            .handle_line(echo_line("t1", "head", "first").as_bytes());
        let request = steer_one(&h, &ann, "q-1", "second");
        h.reader
            .handle_line(completed_line("T", "t1", "completed").as_bytes());
        assert_eq!(stage_of(&h.state, "q-1").as_deref(), Some("held"));
        let facts = turns.get(id, 1);
        assert!(facts.is_some_and(|f| !f.in_turn));
        h.seen.lock().unwrap().clear();

        h.reader
            .handle_line(echo_line("t1", "q-1", "second").as_bytes());
        assert_eq!(
            trace(&h.seen),
            ["delivered q-1", "bubble q-1", "end completed"]
        );
        assert_eq!(
            turns.get(id, 1),
            facts,
            "the late receipt or the boundary after it touched the turn table"
        );
        assert_eq!(stage_of(&h.state, "q-1"), None);
        assert!(row_ids(&h).is_empty());
        assert!(
            take(&h).is_none(),
            "received text went out again, or an empty turn/start went out"
        );

        steer_accepted_reply(&mut h, request, "t1");
        assert_eq!(
            trace(&h.seen).len(),
            3,
            "the late success response emitted a list event or a boundary"
        );
        assert!(take(&h).is_none(), "an empty turn/start went out");
    }

    /// ★If the echo came before the steer response, the turn end does not wait for it★ — the receipt removes the item, so there is nothing to
    /// dispose at the turn end and the next user text's `turn/start` goes out at once before the response. A late success response emits nothing.
    #[test]
    fn an_echoed_steer_whose_reply_is_still_owed_does_not_hold_the_turn_end() {
        let mut h = harness_with_real_decoder();
        let ann = above(&h);
        answered_turn(&mut h, &ann, "t1");
        let request = steer_one(&h, &ann, "q-1", "second");
        h.reader
            .handle_line(echo_line("t1", "q-1", "second").as_bytes());
        assert_eq!(
            stage_of(&h.state, "q-1"),
            None,
            "the receipt did not remove the item"
        );
        item_started(&mut h, "t1", "agentMessage", "a-2");
        h.reader
            .handle_line(completed_line("T", "t1", "completed").as_bytes());

        accept_user(&h.state, "q-2", "typed after the turn");
        announce(&h.reader.core, &h.state, &ann);
        let (_, _, p) =
            opens(&h).expect("turn/start did not go out because it waited for the steer response");
        assert_eq!(p["clientUserMessageId"], "q-2");
        let ops = list_ops(&h.seen);
        steer_accepted_reply(&mut h, request, "t1");
        assert_eq!(
            list_ops(&h.seen),
            ops,
            "the late response emitted a list event"
        );
    }

    #[test]
    fn a_refusal_after_the_echo_never_resurrects_the_item() {
        for after_turn_end in [false, true] {
            let at = format!("after_turn_end={after_turn_end}");
            let mut h = harness_with_real_decoder();
            let ann = above(&h);
            answered_turn(&mut h, &ann, "t1");
            let request = steer_one(&h, &ann, "q-1", "second");
            h.reader
                .handle_line(echo_line("t1", "q-1", "second").as_bytes());
            item_started(&mut h, "t1", "agentMessage", "a-2");
            if after_turn_end {
                h.reader
                    .handle_line(completed_line("T", "t1", "completed").as_bytes());
            }
            let ops = list_ops(&h.seen);
            steer_refused_reply(&mut h, request);
            assert_eq!(stage_of(&h.state, "q-1"), None, "{at}: the item came back");
            assert_eq!(list_ops(&h.seen), ops, "{at}: a list event happened");
            assert_eq!(
                with_state(&h.state, |s| s.steer_refused),
                None,
                "{at}: a refusal of accepted text stopped that turn's steers"
            );
        }
    }

    #[test]
    fn a_late_echo_inside_a_running_turn_adds_no_boundary() {
        let (mut h, _, _) = end_a_turn(Whose::Listed, End::Completed("completed"));
        accept_user(&h.state, "q-2", "next");
        announce(&h.reader.core, &h.state, &announcer());
        let (request, _) = open_turn(&h);
        reply(&mut h, request, "U2");
        h.seen.lock().unwrap().clear();

        h.reader
            .handle_line(echo_line("U1", "q-1", "hi").as_bytes());
        assert_eq!(trace(&h.seen), ["delivered q-1", "bubble q-1"]);
        assert_eq!(stage_of(&h.state, "q-1"), None);
        assert_eq!(stage_of(&h.state, "q-2").as_deref(), Some("held"));
    }

    #[test]
    fn a_late_echo_of_an_item_returned_to_held_wakes_the_writer() {
        let (mut h, _, _) = end_a_turn(Whose::Listed, End::Completed("completed"));
        assert_eq!(stage_of(&h.state, "q-1").as_deref(), Some("held"));
        let state = h.state.clone();
        assert!(
            wakes_a_waiter(&state, || h
                .reader
                .handle_line(echo_line("U1", "q-1", "hi").as_bytes())),
            "did not wake the writer"
        );
        assert_eq!(stage_of(&h.state, "q-1"), None);
    }

    #[test]
    fn cancelling_an_item_returned_to_held_withdraws_it_at_once() {
        let (h, _, _) = end_a_turn(Whose::Listed, End::Completed("completed"));
        assert_eq!(stage_of(&h.state, "q-1").as_deref(), Some("held"));
        let mut answer = None;
        assert!(
            wakes_a_waiter(&h.state, || {
                answer = Some(withdraw_item(&h.state, Some(&h.reader.core), "q-1"))
            }),
            "did not wake the writer"
        );
        assert_eq!(answer, Some(Withdraw::Withdrawn));
        assert_eq!(list_ops(&h.seen), vec![withdrawn("q-1")]);
        assert_eq!(stage_of(&h.state, "q-1"), None);
        assert!(row_ids(&h).is_empty(), "it remained in the registry");
        assert_eq!(
            withdraw_item(&h.state, Some(&h.reader.core), "q-1"),
            Withdraw::NotHeld,
            "overlapping second ✕"
        );
        assert_eq!(list_ops(&h.seen).len(), 1, "the second ✕ emitted an event");
        accept_user(&h.state, "next", "typed after");
        announce(&h.reader.core, &h.state, &announcer());
        let v: Value = serde_json::from_str(&turn_line(take(&h))).unwrap();
        assert_eq!(
            v["params"]["clientUserMessageId"], "next",
            "withdrawn text went out"
        );
    }

    #[test]
    fn a_cancelled_handed_over_item_is_withdrawn_when_its_turn_ends_without_its_echo() {
        for end in [
            End::Completed("completed"),
            End::Completed("inProgress"),
            End::Completed("failed"),
            End::Completed("interrupted"),
            End::Early("completed"),
            End::ErrorReply,
            End::WriteFailed,
            End::Deadline,
            End::Unreadable,
            End::LongTurnId,
        ] {
            let mut h = harness_with_real_decoder();
            let ann = above(&h);
            activate(&h.state, 0, Some("U0"));
            accept_user(&h.state, "q-1", "hi");
            with_state(&h.state, |s| s.turn = TurnState::Idle);
            announce(&h.reader.core, &h.state, &ann);
            let (request, seq) = open_turn(&h);
            assert_eq!(
                withdraw_item(&h.state, Some(&h.reader.core), "q-1"),
                Withdraw::TooLate,
                "{end:?}"
            );
            h.seen.lock().unwrap().clear();
            finish_turn(&mut h, request, seq, end);
            assert_eq!(list_ops(&h.seen), vec![withdrawn("q-1")], "{end:?}");
            assert_eq!(stage_of(&h.state, "q-1"), None, "{end:?}");
            assert!(
                row_ids(&h).is_empty(),
                "{end:?}: it remained in the registry"
            );
            assert!(take(&h).is_none(), "{end:?}: withdrawn text went out");
        }
    }

    // ── Idle order (ADR-0231 · TRD §5-5 「the order of opening turns」) ─────────────────────────────────────────

    fn opens(h: &Harness) -> Option<(i64, u64, Value)> {
        let Some(Job::Turn { id, seq, line, .. }) = take(h) else {
            return None;
        };
        assert!(h
            .pending
            .register(id, Waiter::TurnStart { seq }, method::TURN_START));
        let v: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(v["method"], "turn/start");
        Some((id, seq, v["params"].clone()))
    }

    fn answered_turn(h: &mut Harness, ann: &Announcer, turn: &str) -> u64 {
        let seq = running_turn(h, ann, turn);
        h.reader
            .handle_line(echo_line(turn, "head", "first").as_bytes());
        item_started(h, turn, "agentMessage", "a-head");
        item_completed(h, turn, "agentMessage", "a-head");
        seq
    }

    fn steer_recorded(h: &mut Harness, ann: &Announcer, turn: &str, id: &str, text: &str) {
        let request = steer_one(h, ann, id, text);
        steer_accepted_reply(h, request, turn);
        h.reader.handle_line(echo_line(turn, id, text).as_bytes());
    }

    /// ★Idle order = user first → mail★ (PRD §3-9 · N6) — with held mail and user text together, the user turn comes before earlier-typed
    /// mail, and the mail opens after that turn ends. ★Even if an accepted steer ends without an answer, no empty `turn/start` goes out★
    /// (the follow-up turn debt was removed — ADR-0235).
    #[test]
    fn the_idle_order_is_user_then_mail() {
        for with_user in [true, false] {
            let at = format!("with_user={with_user}");
            let mut h = harness_with_real_decoder();
            let ann = above(&h);
            answered_turn(&mut h, &ann, "t1");
            steer_recorded(&mut h, &ann, "t1", "q-1", "second");
            accept_input(
                &h.state,
                Some("m-1".into()),
                b"mail".to_vec(),
                InputOrigin::Mail,
            )
            .unwrap();
            h.reader
                .handle_line(completed_line("T", "t1", "completed").as_bytes());
            if with_user {
                accept_user(&h.state, "u-1", "typed");
                announce(&h.reader.core, &h.state, &ann);
                let (request, _, p) = opens(&h).expect("the user turn did not open");
                assert_eq!(p["clientUserMessageId"], "u-1", "{at}");
                reply(&mut h, request, "t2");
                h.reader
                    .handle_line(echo_line("t2", "u-1", "typed").as_bytes());
                item_started(&mut h, "t2", "agentMessage", "a-2");
                h.reader
                    .handle_line(completed_line("T", "t2", "completed").as_bytes());
            }
            let (_, _, p) = opens(&h).expect("the mail turn/start did not go out");
            assert_eq!(p["clientUserMessageId"], "m-1", "{at}");
            assert_ne!(p["input"], serde_json::json!([]), "{at}: empty turn/start");
        }
    }

    /// At Idle `[U1, M, U2]` (M = mail) → U1's `turn/start` · U2 steered at once on the new turn id · M gets `turn/start` only at the Idle
    /// after the user `Held` are empty (TRD §7-1 · AC22 · N6).
    #[test]
    fn mail_between_user_items_waits_until_the_user_items_are_in() {
        let mut h = harness_with_real_decoder();
        let ann = above(&h);
        accept_user(&h.state, "u-1", "one");
        accept_input(
            &h.state,
            Some("m-1".into()),
            b"mail".to_vec(),
            InputOrigin::Mail,
        )
        .unwrap();
        accept_user(&h.state, "u-2", "two");
        announce(&h.reader.core, &h.state, &ann);
        let (request, _, p) = opens(&h).expect("U1's turn/start");
        assert_eq!(p["clientUserMessageId"], "u-1");
        reply(&mut h, request, "t1");
        let s = next_steer_job(&h, "U2 was not handed over on the new turn id");
        assert_eq!(steer_params(&s)["clientUserMessageId"], "u-2");
        let steer = s.request;
        assert!(matches!(issue(&h, s), Issued::Written));
        assert!(steer_next(&h).is_none(), "mail went out as a steer");
        steer_accepted_reply(&mut h, steer, "t1");
        h.reader
            .handle_line(echo_line("t1", "u-1", "one").as_bytes());
        h.reader
            .handle_line(echo_line("t1", "u-2", "two").as_bytes());
        item_started(&mut h, "t1", "agentMessage", "a-1");
        assert!(take(&h).is_none(), "mail went out on the running turn");
        h.reader
            .handle_line(completed_line("T", "t1", "completed").as_bytes());
        let (_, _, p) = opens(&h).expect("M's turn/start");
        assert_eq!(p["clientUserMessageId"], "m-1");
    }

    /// ★User text returned to the held place goes out before earlier-held mail★ (Idle order = user first — N6) — the mail opens at the Idle
    /// after that text is accepted.
    #[test]
    fn a_user_item_returned_to_held_opens_before_older_held_mail() {
        let mut h = harness_with_real_decoder();
        let ann = above(&h);
        activate(&h.state, 0, Some("U0"));
        accept_input(
            &h.state,
            Some("m-1".into()),
            b"mail".to_vec(),
            InputOrigin::Mail,
        )
        .unwrap();
        accept_user(&h.state, "q-1", "hi");
        with_state(&h.state, |s| s.turn = TurnState::Idle);
        announce(&h.reader.core, &h.state, &ann);
        let (request, _, p) = opens(&h).expect("the user turn did not open");
        assert_eq!(p["clientUserMessageId"], "q-1");
        reply(&mut h, request, "U1");
        h.reader
            .handle_line(completed_line("T", "U1", "completed").as_bytes());
        assert_eq!(stage_of(&h.state, "q-1").as_deref(), Some("held"));

        let (request, _, p) = opens(&h).expect("the turn of the returned text did not open");
        assert_eq!(
            p["clientUserMessageId"], "q-1",
            "mail overtook the returned text"
        );
        reply(&mut h, request, "U2");
        h.reader
            .handle_line(echo_line("U2", "q-1", "hi").as_bytes());
        h.reader
            .handle_line(completed_line("T", "U2", "completed").as_bytes());
        let (_, _, p) = opens(&h).expect("the mail turn/start did not go out");
        assert_eq!(p["clientUserMessageId"], "m-1");
    }

    // ── Halt after error (ADR-0231 · TRD §5-5 「halt after error」 · PRD AC24 · AC29) ──────────────────────────

    fn halted(state: &SharedState) -> bool {
        with_state(state, |s| s.halted)
    }

    /// The kind of turn that sets the halt — what its `turn/start` carried (TRD §7-1).
    #[derive(Debug, Clone, Copy)]
    enum Kind {
        Listed,
        Direct,
        Mail,
    }

    const HALTING_ENDS: [End; 7] = [
        End::Completed("failed"),
        End::Early("failed"),
        End::ErrorReply,
        End::WriteFailed,
        End::Deadline,
        End::Unreadable,
        End::LongTurnId,
    ];

    /// One turn of `kind` opened (incl. waiting entry) with user text `pre` and mail `m-pre` held during it — (request id, marker). `pre`
    /// comes in while waiting for the turn id and stays held (the bench does not run the writer).
    fn open_kind(h: &mut Harness, ann: &Announcer, kind: Kind) -> (i64, u64) {
        match kind {
            Kind::Listed => {
                activate(&h.state, 0, Some("U0"));
                accept_user(&h.state, "q-1", "hi");
                with_state(&h.state, |s| s.turn = TurnState::Idle);
            }
            Kind::Direct => accept_user(&h.state, "d-1", "hi"),
            Kind::Mail => {
                accept_input(
                    &h.state,
                    Some("m-1".into()),
                    b"hi".to_vec(),
                    InputOrigin::Mail,
                )
                .unwrap();
            }
        }
        announce(&h.reader.core, &h.state, ann);
        let (request, seq, _) = opens(h).expect("the turn did not open");
        accept_user(&h.state, "pre", "typed before");
        accept_input(
            &h.state,
            Some("m-pre".into()),
            b"mail".to_vec(),
            InputOrigin::Mail,
        )
        .unwrap();
        announce(&h.reader.core, &h.state, ann);
        (request, seq)
    }

    /// ★Six triggers × three kinds — the halt stands either way★ · the user text that turn carried is returned, not erased (user decision
    /// 2026-09-26 B — mail leaves as today) · 0 turns opened by Idle while halted (returned text · text held before the halt · held mail) ·
    /// when the next user text comes in the halt lifts and a `turn/start` with the remaining text as head opens, the following text entering
    /// that turn id as a steer — what remains goes first, in typing order (AC24).
    #[test]
    fn every_abnormal_end_halts_whatever_the_turn_kind_until_the_next_user_item() {
        for kind in [Kind::Listed, Kind::Direct, Kind::Mail] {
            let (head, next) = match kind {
                Kind::Listed => ("q-1", "pre"),
                Kind::Direct => ("d-1", "pre"),
                Kind::Mail => ("pre", "post"),
            };
            for end in HALTING_ENDS {
                let at = format!("{kind:?}/{end:?}");
                let mut h = harness_with_real_decoder();
                let ann = above(&h);
                let (request, seq) = open_kind(&mut h, &ann, kind);
                assert!(!halted(&h.state), "{at}: stood before the turn ended");
                finish_turn(&mut h, request, seq, end);
                assert_eq!(turn_seq(&h.state), None, "{at}: the turn did not end");
                assert!(halted(&h.state), "{at}");
                assert!(take(&h).is_none(), "{at}: a turn opened during the halt");
                assert_eq!(
                    stage_of(&h.state, head).as_deref(),
                    Some("held"),
                    "{at}: the text that should become the head is not in the held place"
                );

                accept_user(&h.state, "post", "typed after");
                assert!(!halted(&h.state), "{at}: user text did not lift the halt");
                announce(&h.reader.core, &h.state, &ann);
                let (request, _, p) =
                    opens(&h).expect("the turn of the text typed after the halt did not open");
                assert_eq!(
                    p["clientUserMessageId"], head,
                    "{at}: the remaining text is not the head"
                );
                reply(&mut h, request, "t9");
                let steer =
                    steer_next(&h).expect("the next text was not handed over on that turn id");
                assert_eq!(steer_params(&steer)["clientUserMessageId"], next, "{at}");
            }
        }
    }

    /// ★The halt is not lifted by mail — only the moment user text comes in★ · it stays even if all held text is removed by ✕ and the list
    /// empties (0 held mail). Idle order after lifting is as usual — when the user turn ends, held mail goes out.
    #[test]
    fn only_a_user_item_lifts_the_halt_and_the_idle_order_resumes() {
        let mut h = harness_with_real_decoder();
        let ann = above(&h);
        let (request, seq) = open_kind(&mut h, &ann, Kind::Mail);
        finish_turn(&mut h, request, seq, End::Completed("failed"));
        assert!(halted(&h.state));
        assert_eq!(
            withdraw_item(&h.state, Some(&h.reader.core), "pre"),
            Withdraw::Withdrawn
        );
        let lifted = accept_input(
            &h.state,
            Some("m-2".into()),
            b"more mail".to_vec(),
            InputOrigin::Mail,
        );
        assert!(matches!(lifted, Ok(false)), "{lifted:?}");
        assert!(halted(&h.state), "mail lifted the halt");
        assert!(take(&h).is_none(), "mail went out when ✕ emptied the list");

        // The fact that it lifted goes back to the caller — `send_turn` uses it to write the lift log outside the lock.
        let lifted = accept_input(
            &h.state,
            Some("u-1".into()),
            b"typed after".to_vec(),
            InputOrigin::User,
        );
        assert!(matches!(lifted, Ok(true)), "{lifted:?}");
        assert!(!halted(&h.state), "user text did not lift the halt");
        announce(&h.reader.core, &h.state, &ann);
        let (request, _, p) =
            opens(&h).expect("the turn of the text typed after the halt did not open");
        assert_eq!(p["clientUserMessageId"], "u-1");
        reply(&mut h, request, "t2");
        h.reader
            .handle_line(echo_line("t2", "u-1", "typed after").as_bytes());
        h.reader
            .handle_line(completed_line("T", "t2", "completed").as_bytes());
        assert_eq!(turn_seq(&h.state), None);
        let (_, _, p) = opens(&h).expect("the held mail did not go out");
        assert_eq!(p["clientUserMessageId"], "m-pre");
    }

    /// ★An interruption (`interrupted`) does not set the halt★ — the interrupted turn's returned user text and the text held during it are
    /// not dropped and go out at once with `turn/start` from the next Idle in typing order; held mail goes after those user turns end (user
    /// decision 2026-09-26).
    #[test]
    fn an_interrupted_turn_does_not_halt_and_held_items_go_at_the_next_idle() {
        for kind in [Kind::Listed, Kind::Direct, Kind::Mail] {
            let at = format!("{kind:?}");
            let users: &[&str] = match kind {
                Kind::Listed => &["q-1", "pre"],
                Kind::Direct => &["d-1", "pre"],
                Kind::Mail => &["pre"],
            };
            let mut h = harness_with_real_decoder();
            let ann = above(&h);
            let (request, seq) = open_kind(&mut h, &ann, kind);
            h.seen.lock().unwrap().clear();
            finish_turn(&mut h, request, seq, End::Completed("interrupted"));
            assert_eq!(turn_seq(&h.state), None, "{at}: the turn did not end");
            assert!(!halted(&h.state), "{at}: the interruption set the halt");
            assert!(
                !list_ops(&h.seen)
                    .iter()
                    .any(|op| matches!(op, QueuedInputEvent::Dropped { .. })),
                "{at}: user text was dropped"
            );
            for (n, id) in users.iter().enumerate() {
                assert_eq!(stage_of(&h.state, id).as_deref(), Some("held"), "{at}");
                let (request, _, p) = opens(&h).expect("the turn of the held text did not open");
                assert_eq!(p["clientUserMessageId"], *id, "{at}");
                let turn = format!("t{}", n + 2);
                reply(&mut h, request, &turn);
                h.reader.handle_line(echo_line(&turn, id, "hi").as_bytes());
                h.reader
                    .handle_line(completed_line("T", &turn, "completed").as_bytes());
            }
            let (_, _, p) = opens(&h).expect("the held mail did not go out");
            assert_eq!(p["clientUserMessageId"], "m-pre", "{at}");
        }
    }

    /// ★A failed turn holds until the next user text — then held text goes first★ (the turn's carried text returns as the head, text held
    /// during it follows in typing order) · text typed after the halt and then ✕-removed after lifting does not re-stand the halt — the
    /// remaining held text goes out at once (lifting is an arrival event).
    #[test]
    fn a_failed_turn_holds_until_the_next_user_item_then_the_older_held_items_go_first() {
        let mut h = harness_with_real_decoder();
        let ann = above(&h);
        let (request, seq) = open_kind(&mut h, &ann, Kind::Direct);
        accept_user(&h.state, "pre-2", "typed before too");
        announce(&h.reader.core, &h.state, &ann);
        finish_turn(&mut h, request, seq, End::Completed("failed"));
        assert!(halted(&h.state));
        assert!(take(&h).is_none(), "a turn opened during the halt");

        accept_user(&h.state, "post", "typed after");
        announce(&h.reader.core, &h.state, &ann);
        assert_eq!(
            withdraw_item(&h.state, Some(&h.reader.core), "post"),
            Withdraw::Withdrawn
        );
        assert!(!halted(&h.state), "✕ set the halt again");
        let (request, _, p) = opens(&h).expect("the turn of the held text did not open");
        assert_eq!(
            p["clientUserMessageId"], "d-1",
            "the text returned to the held place is not the head"
        );
        reply(&mut h, request, "t9");
        for id in ["pre", "pre-2"] {
            let steer =
                next_steer_job(&h, "the next held text was not handed over on that turn id");
            assert_eq!(steer_params(&steer)["clientUserMessageId"], id);
        }
    }

    /// ★Accepted consequence (1) — if the turn of the text that lifted the halt is interrupted, held text and mail that went into the link go
    /// out at the next Idle★ (ADR-0235 「accepted consequences」 first item): failure end sets the halt → lifts when user text comes in →
    /// even if the turn opened after is interrupted, the halt does not stand again → handed-over text returns and goes out again at once in
    /// typing order, and when that user turn ends, held mail goes out.
    #[test]
    fn an_interrupted_turn_after_the_halt_is_lifted_lets_held_items_and_mail_go_at_the_next_idle() {
        let mut h = harness_with_real_decoder();
        let ann = above(&h);
        let (request, seq) = open_kind(&mut h, &ann, Kind::Mail);
        finish_turn(&mut h, request, seq, End::Completed("failed"));
        assert!(halted(&h.state), "the halt did not stand");
        assert!(take(&h).is_none(), "a turn opened during the halt");

        accept_user(&h.state, "u-1", "typed after");
        assert!(!halted(&h.state), "user text did not lift the halt");
        announce(&h.reader.core, &h.state, &ann);
        let (request, _, p) = opens(&h).expect("the turn did not open after the halt lifted");
        assert_eq!(
            p["clientUserMessageId"], "pre",
            "the held text is not first"
        );
        reply(&mut h, request, "t2");
        let s = next_steer_job(
            &h,
            "the text that lifted the halt was not handed over on that turn id",
        );
        assert_eq!(steer_params(&s)["clientUserMessageId"], "u-1");
        assert!(matches!(issue(&h, s), Issued::Written));

        h.seen.lock().unwrap().clear();
        h.reader
            .handle_line(completed_line("T", "t2", "interrupted").as_bytes());
        assert_eq!(turn_seq(&h.state), None, "the interrupted turn did not end");
        assert!(!halted(&h.state), "the interruption set the halt again");
        assert!(
            !list_ops(&h.seen)
                .iter()
                .any(|op| matches!(op, QueuedInputEvent::Dropped { .. })),
            "the text of the interrupted turn was dropped"
        );
        for id in ["pre", "u-1"] {
            assert_eq!(stage_of(&h.state, id).as_deref(), Some("held"), "{id}");
        }

        let (request, _, p) =
            opens(&h).expect("the turn of the text held after the interruption did not open");
        assert_eq!(p["clientUserMessageId"], "pre");
        reply(&mut h, request, "t3");
        let s = next_steer_job(&h, "the held text was not handed over on the next turn id");
        assert_eq!(steer_params(&s)["clientUserMessageId"], "u-1");
        issue(&h, s);
        assert!(take(&h).is_none(), "held mail went out on the running turn");
        h.reader
            .handle_line(echo_line("t3", "pre", "typed before").as_bytes());
        h.reader
            .handle_line(echo_line("t3", "u-1", "typed after").as_bytes());
        h.reader
            .handle_line(completed_line("T", "t3", "completed").as_bytes());
        let (_, _, p) = opens(&h).expect("the held mail did not go out");
        assert_eq!(p["clientUserMessageId"], "m-pre");
    }

    /// ★The halt does not stand on a below-floor incarnation★ (N1 (a)) — even after `failed` · an exit, held input goes out at once with
    /// `turn/start` as today.
    #[test]
    fn below_the_floor_no_halt_stands_and_held_input_goes_at_once() {
        for end in [End::Completed("failed"), End::ErrorReply, End::Deadline] {
            let mut h = harness_with_real_decoder();
            make_ready(&h.state, "T");
            settle_floor(&h.reader.core, &h.state, &announcer(), Floor::Below);
            accept_user(&h.state, "a", "one");
            accept_user(&h.state, "b", "two");
            let (request, seq) = open_turn(&h);
            finish_turn(&mut h, request, seq, end);
            assert!(!halted(&h.state), "{end:?}");
            let v: Value = serde_json::from_str(&turn_line(take(&h))).unwrap();
            assert_eq!(v["params"]["input"][0]["text"], "two", "{end:?}");
            assert!(v["params"].get("clientUserMessageId").is_none(), "{end:?}");
        }
    }

    /// ★Steer text without an echo carried by a `failed` turn is not erased but held behind the halt★ (user decision 2026-09-26 · ADR-0235
    /// — replaces 「erase on failure」) · 0 list events · accepted text leaves and held mail stays · the halt stands. It does not wait for
    /// that steer's response — when the next user text lifts the halt the returned text goes out at once with `turn/start` before that text
    /// (before held mail); a late success response of the old request emits nothing.
    #[test]
    fn a_failed_turn_keeps_its_unechoed_steer_held_behind_the_halt_and_it_opens_first_without_waiting_for_the_reply(
    ) {
        let mut h = harness_with_real_decoder();
        let ann = above(&h);
        answered_turn(&mut h, &ann, "t1");
        steer_recorded(&mut h, &ann, "t1", "q-1", "recorded");
        let waiting = steer_one(&h, &ann, "q-2", "unechoed");
        accept_input(
            &h.state,
            Some("m-1".into()),
            b"mail".to_vec(),
            InputOrigin::Mail,
        )
        .unwrap();
        h.seen.lock().unwrap().clear();
        h.reader
            .handle_line(completed_line("T", "t1", "failed").as_bytes());
        assert_eq!(
            list_ops(&h.seen),
            Vec::new(),
            "the failure end emitted a list event"
        );
        assert_eq!(stage_of(&h.state, "q-1"), None, "accepted text remained");
        assert_eq!(
            stage_of(&h.state, "q-2").as_deref(),
            Some("held"),
            "the failure end did not hold the text without an echo"
        );
        assert_eq!(row_ids(&h), ["q-2"], "the registry changed");
        assert_eq!(stage_of(&h.state, "m-1").as_deref(), Some("held"));
        assert!(halted(&h.state), "the halt did not stand");
        assert!(take(&h).is_none(), "a turn opened during the halt");

        accept_user(&h.state, "after", "typed after");
        announce(&h.reader.core, &h.state, &ann);
        let (_, _, p) = opens(&h).expect("the turn of the text typed after the halt did not open");
        assert_eq!(
            p["clientUserMessageId"], "q-2",
            "the returned text did not go out before the text that lifted the halt"
        );
        let ops = list_ops(&h.seen);
        steer_accepted_reply(&mut h, waiting, "t1");
        assert_eq!(
            list_ops(&h.seen),
            ops,
            "the late response emitted a list event"
        );
    }

    /// ★A late receipt during the halt★ — if the echo of text a failure end returned comes late, receipt + vendor bubble + display-only
    /// boundary stand and the text leaves (not sent again), but the halt stays (0 mail turns) and that boundary does not pass the turn
    /// table, so the kernel's error end (`last_end_failed`) stays too (TRD §5-5 「when a receipt comes after the turn end」).
    #[test]
    fn a_late_echo_while_halted_keeps_the_halt_and_the_kernel_s_error_end() {
        let (mut h, turns, id) = harness_with_facts();
        let ann = above(&h);
        let (request, seq) = open_kind(&mut h, &ann, Kind::Listed);
        assert_eq!(
            withdraw_item(&h.state, Some(&h.reader.core), "pre"),
            Withdraw::Withdrawn
        );
        finish_turn(&mut h, request, seq, End::Completed("failed"));
        assert_eq!(
            stage_of(&h.state, "q-1").as_deref(),
            Some("held"),
            "the failure end did not return the text it carried to the held place"
        );
        assert!(halted(&h.state), "the halt did not stand");
        let failed = || turns.get(id, 1).map(|o| o.last_end_failed);
        assert_eq!(
            failed(),
            Some(true),
            "the classifier did not fold the error end"
        );

        h.seen.lock().unwrap().clear();
        h.reader
            .handle_line(echo_line("U1", "q-1", "hi").as_bytes());
        assert_eq!(
            trace(&h.seen),
            ["delivered q-1", "bubble q-1", "end completed"]
        );
        assert!(halted(&h.state), "the late receipt touched the halt");
        assert_eq!(stage_of(&h.state, "q-1"), None, "received text remained");
        assert!(
            take(&h).is_none(),
            "a mail turn opened after the late receipt"
        );
        assert_eq!(
            failed(),
            Some(true),
            "the display-only boundary erased the kernel's error end"
        );
        assert!(
            !turns.is_in_turn(id, 1),
            "the late receipt turned mid-turn on"
        );
    }

    /// ★The error end of the codex classifier (TRD §5-0)★ — an attributed `TurnEnd{Failed}` (translated `failed` · early-end release · each
    /// of the five exits) → turn table `last_end_failed` true · next `completed` → false (lifted) · interruption → unchanged.
    #[test]
    fn every_attributed_failed_end_sets_the_kernel_s_error_end_and_only_a_completed_end_lifts_it() {
        for end in HALTING_ENDS {
            let (mut h, turns, id) = harness_with_facts();
            let ann = above(&h);
            let failed = || turns.get(id, 1).map(|o| o.last_end_failed);
            let (request, seq) = open_kind(&mut h, &ann, Kind::Direct);
            finish_turn(&mut h, request, seq, end);
            assert_eq!(failed(), Some(true), "{end:?}");

            assert!(
                opens(&h).is_none(),
                "{end:?}: a turn opened during the halt"
            );
            accept_user(&h.state, "post", "typed after");
            announce(&h.reader.core, &h.state, &ann);
            let (request, seq, _) =
                opens(&h).expect("the turn of the text typed after the halt did not open");
            finish_turn(&mut h, request, seq, End::Completed("interrupted"));
            assert_eq!(
                failed(),
                Some(true),
                "{end:?}: the interruption erased the error end"
            );

            accept_user(&h.state, "again", "typed after the interrupt");
            announce(&h.reader.core, &h.state, &ann);
            let (request, seq, _) =
                opens(&h).expect("the turn of the text typed after the interruption did not open");
            finish_turn(&mut h, request, seq, End::Completed("completed"));
            assert_eq!(
                failed(),
                Some(false),
                "{end:?}: the success end did not lift it"
            );
        }
    }

    // ── Ends of unknown meaning (ADR-0231 · P2 review doc-aware F2) ─────────────────────────────────────────────

    fn statusless_completed_line(thread_id: &str, turn_id: &str) -> String {
        serde_json::json!({
            "method": "turn/completed",
            "params": {"threadId": thread_id, "turn": {"id": turn_id, "items": []}},
        })
        .to_string()
    }

    const UNKNOWN_STATUSES: [Option<&str>; 3] =
        [None, Some("inProgress"), Some("quantumCollapsed")];

    fn unknown_completed_line(turn_id: &str, status: Option<&str>) -> String {
        match status {
            Some(s) => completed_line("T", turn_id, s),
            None => statusless_completed_line("T", turn_id),
        }
    }

    #[test]
    fn an_unknown_status_is_its_own_close() {
        for status in UNKNOWN_STATUSES {
            let v: Value = serde_json::from_str(&unknown_completed_line("U1", status)).unwrap();
            assert_eq!(
                TurnClose::of_completion(v.get("params")),
                TurnClose::Unknown,
                "{status:?}"
            );
        }
        let v: Value = serde_json::from_str(&completed_line("T", "U1", "completed")).unwrap();
        assert_eq!(
            TurnClose::of_completion(v.get("params")),
            TurnClose::Completed
        );
    }

    /// ★An end of unknown meaning neither sets the halt nor lifts the kernel's error end★ — if the turn opened after user text lifted the halt
    /// after `failed` ends with an unknown status, the link's halt does not stand and the kernel's `last_end_failed` stays. Item disposal =
    /// `completed` — list items ended without an echo return to the held place.
    #[test]
    fn an_unknown_status_after_a_failed_turn_raises_no_halt_and_keeps_the_kernel_s_error_end() {
        for status in UNKNOWN_STATUSES {
            let at = format!("{status:?}");
            let (mut h, turns, id) = harness_with_facts();
            let ann = above(&h);
            let failed = || turns.get(id, 1).map(|o| o.last_end_failed);
            let (request, seq) = open_kind(&mut h, &ann, Kind::Mail);
            finish_turn(&mut h, request, seq, End::Completed("failed"));
            assert!(halted(&h.state), "{at}: the halt did not stand");
            assert_eq!(failed(), Some(true), "{at}");

            accept_user(&h.state, "u-1", "typed after");
            announce(&h.reader.core, &h.state, &ann);
            let (request, _, p) =
                opens(&h).expect("the turn of the text typed after the halt did not open");
            assert_eq!(p["clientUserMessageId"], "pre", "{at}");
            reply(&mut h, request, "U2");
            h.reader
                .handle_line(unknown_completed_line("U2", status).as_bytes());
            assert_eq!(turn_seq(&h.state), None, "{at}: the turn did not end");
            assert!(
                !halted(&h.state),
                "{at}: the end of unknown meaning set the halt"
            );
            assert_eq!(
                failed(),
                Some(true),
                "{at}: the kernel's error end was lifted"
            );
            assert_eq!(
                stage_of(&h.state, "pre").as_deref(),
                Some("held"),
                "{at}: the disposal differs from completed"
            );
        }
    }

    /// An end of unknown meaning also returns steer text without an echo to the held place like `completed` and lets it ride the next turn at
    /// once (no wait for the response · if nothing is held, no turn opens).
    #[test]
    fn an_unknown_status_returns_an_unechoed_steer_to_held_like_completed() {
        for status in UNKNOWN_STATUSES {
            for steered in [false, true] {
                let at = format!("{status:?} steered={steered}");
                let mut h = harness_with_real_decoder();
                let ann = above(&h);
                answered_turn(&mut h, &ann, "t1");
                steer_recorded(&mut h, &ann, "t1", "q-1", "second");
                if steered {
                    steer_one(&h, &ann, "q-2", "third");
                }
                h.reader
                    .handle_line(unknown_completed_line("t1", status).as_bytes());
                assert_eq!(turn_seq(&h.state), None, "{at}");
                if steered {
                    assert_eq!(stage_of(&h.state, "q-2").as_deref(), Some("held"), "{at}");
                    let v: Value = serde_json::from_str(&turn_line(take(&h))).unwrap();
                    assert_eq!(v["params"]["clientUserMessageId"], "q-2", "{at}");
                } else {
                    assert!(
                        take(&h).is_none(),
                        "{at}: turn/start went out with nothing held"
                    );
                }
            }
        }
    }

    // ── Transition logs (ADR-0231 · P2 review doc-aware F1 · logging-conventions 「instrumentation duty」) ────────────────

    type LogLine = (tracing::Level, TraceLine);

    struct LogCapture(Arc<Mutex<Vec<LogLine>>>);

    impl tracing::Subscriber for LogCapture {
        fn register_callsite(
            &self,
            _: &'static tracing::Metadata<'static>,
        ) -> tracing::subscriber::Interest {
            tracing::subscriber::Interest::sometimes()
        }
        fn enabled(&self, _: &tracing::Metadata<'_>) -> bool {
            true
        }
        fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
            tracing::span::Id::from_u64(1)
        }
        fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}
        fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
        fn event(&self, event: &tracing::Event<'_>) {
            let mut line = TraceLine::new();
            event.record(&mut TraceFields(&mut line));
            self.0
                .lock()
                .unwrap()
                .push((*event.metadata().level(), line));
        }
        fn enter(&self, _: &tracing::span::Id) {}
        fn exit(&self, _: &tracing::span::Id) {}
    }

    fn logged<'a>(lines: &'a [LogLine], needle: &str) -> Vec<&'a LogLine> {
        lines
            .iter()
            .filter(|(_, l)| l.get("message").is_some_and(|m| m.contains(needle)))
            .collect()
    }

    /// ★The halt's transitions remain in the log★ — set and lift = info (end kind · agent). User text is in no line. The lift line is
    /// written by `send_turn` outside the lock — the bench runs without a link, so it measures only that line's level · fields.
    #[test]
    fn halt_transitions_are_logged_at_their_levels() {
        let captured = Arc::new(Mutex::new(Vec::new()));
        let (agent, halted_at) =
            tracing::subscriber::with_default(LogCapture(captured.clone()), || {
                let mut h = harness_with_real_decoder();
                let agent = h.reader.core.id().to_string();
                let ann = above(&h);
                let (request, seq) = open_kind(&mut h, &ann, Kind::Mail);
                finish_turn(&mut h, request, seq, End::Completed("failed"));
                assert!(halted(&h.state), "the halt did not stand");
                accept_user(&h.state, "u-1", "secret words typed after");
                Transition::HaltLifted.write(h.reader.core.id());
                (agent, seq)
            });

        let lines = captured.lock().unwrap();
        let halt = logged(&lines, "halt after error —");
        assert_eq!(halt.len(), 1, "{lines:?}");
        let (level, fields) = halt[0];
        assert_eq!(*level, tracing::Level::INFO);
        assert_eq!(fields.get("turn"), Some(&halted_at.to_string()));
        assert_eq!(fields.get("close").map(String::as_str), Some("Failed"));
        assert_eq!(fields.get("agent"), Some(&agent));

        let lift = logged(&lines, "halt after error lifted");
        assert_eq!(lift.len(), 1, "{lines:?}");
        assert_eq!(lift[0].0, tracing::Level::INFO);
        assert_eq!(lift[0].1.get("agent"), Some(&agent));

        assert!(
            lines
                .iter()
                .all(|(_, l)| l.values().all(|v| !v.contains("secret words"))),
            "user text was carried in the log"
        );
    }

    #[test]
    fn a_halt_discarded_at_the_stream_end_is_logged_once() {
        let captured = Arc::new(Mutex::new(Vec::new()));
        let agent = tracing::subscriber::with_default(LogCapture(captured.clone()), || {
            let mut h = harness_with_real_decoder();
            let ann = above(&h);
            answered_turn(&mut h, &ann, "t1");
            h.reader
                .handle_line(completed_line("T", "t1", "completed").as_bytes());
            drop(ReaderExit {
                state: h.state.clone(),
                pending: h.pending.clone(),
                agent: h.reader.core.id(),
            });

            let mut h = harness_with_real_decoder();
            let ann = above(&h);
            answered_turn(&mut h, &ann, "t1");
            h.reader
                .handle_line(completed_line("T", "t1", "failed").as_bytes());
            assert!(halted(&h.state), "the halt did not stand");
            drop(ReaderExit {
                state: h.state.clone(),
                pending: h.pending.clone(),
                agent: h.reader.core.id(),
            });
            assert!(!halted(&h.state), "the close did not discard the halt");
            h.reader.core.id().to_string()
        });
        let lines = captured.lock().unwrap();
        let discarded = logged(&lines, "discarding the halt after error");
        assert_eq!(discarded.len(), 1, "{lines:?}");
        assert_eq!(discarded[0].0, tracing::Level::DEBUG);
        assert_eq!(discarded[0].1.get("agent"), Some(&agent));
    }

    // ── Attributing our refusal (ADR-0241 · TRD §4-7 ②-2 · ⑨ 「link — refusal attribution」 ①–⑫) ──────────────────────

    fn server_request(id: i64, method_name: &str, params: Value) -> String {
        serde_json::json!({"id": id, "method": method_name, "params": params}).to_string()
    }

    fn command_approval(id: i64, item_id: &str) -> String {
        server_request(
            id,
            method::ITEM_COMMAND_EXECUTION_REQUEST_APPROVAL,
            serde_json::json!({"kind": "command", "threadId": "T", "turnId": "u-1", "itemId": item_id}),
        )
    }

    fn file_change_approval(id: i64, item_id: &str) -> String {
        server_request(
            id,
            method::ITEM_FILE_CHANGE_REQUEST_APPROVAL,
            serde_json::json!({"threadId": "T", "turnId": "u-1", "itemId": item_id,
                               "reason": null, "grantRoot": null}),
        )
    }

    fn tool_end(kind: &str, item_id: &str, status: &str) -> String {
        let item = match kind {
            "commandExecution" => serde_json::json!({
                "type": kind, "id": item_id, "command": "x", "commandActions": [],
                "status": status, "exitCode": null, "aggregatedOutput": null,
            }),
            _ => serde_json::json!({"type": kind, "id": item_id, "changes": [], "status": status}),
        };
        serde_json::json!({
            "method": method::ITEM_COMPLETED,
            "params": {"threadId": "T", "turnId": "u-1", "item": item},
        })
        .to_string()
    }

    fn command_end(item_id: &str, status: &str) -> String {
        tool_end("commandExecution", item_id, status)
    }

    fn patch_end(item_id: &str, status: &str) -> String {
        tool_end("fileChange", item_id, status)
    }

    fn tool_results(seen: &Arc<Mutex<Vec<OutputEvent>>>) -> Vec<(String, ToolOutcome)> {
        seen.lock()
            .unwrap()
            .iter()
            .filter_map(|e| match e {
                OutputEvent::ToolResult { id, outcome } => Some((id.clone(), *outcome)),
                _ => None,
            })
            .collect()
    }

    fn refusals(state: &SharedState) -> Vec<Value> {
        outbox_lines(state)
            .into_iter()
            .filter(|l| l["error"]["code"] == METHOD_NOT_FOUND)
            .collect()
    }

    fn feed(h: &mut Harness, lines: &[String]) {
        for l in lines {
            h.reader.handle_line(l.as_bytes());
        }
    }

    fn result(id: &str, outcome: ToolOutcome) -> (String, ToolOutcome) {
        (id.to_string(), outcome)
    }

    #[test]
    fn refused_items_hold_one_slot_per_id_and_drop_the_oldest_past_the_cap() {
        let captured = Arc::new(Mutex::new(Vec::new()));
        let items = tracing::subscriber::with_default(LogCapture(captured.clone()), || {
            let mut items = RefusedItems::default();
            items.remember("a");
            items.remember("a");
            assert_eq!(items.ids.len(), 1, "the same id took two slots");
            for i in 0..REFUSED_ITEM_SLOTS {
                items.remember(&format!("n-{i}"));
            }
            items
        });
        assert_eq!(items.ids.len(), REFUSED_ITEM_SLOTS);
        assert!(
            !items.ids.iter().any(|r| r == "a"),
            "the oldest one remained"
        );
        assert_eq!(
            items.ids.back(),
            Some(&format!("n-{}", REFUSED_ITEM_SLOTS - 1))
        );
        let lines = captured.lock().unwrap();
        let evicted = logged(&lines, "reached its cap");
        assert_eq!(evicted.len(), 1, "{lines:?}");
        assert_eq!(evicted[0].0, tracing::Level::DEBUG);
        assert_eq!(evicted[0].1.get("evicted").map(String::as_str), Some("a"));
    }

    #[test]
    fn refused_items_rewrite_only_the_failed_or_declined_result_of_the_item_whose_end_this_is() {
        let end_of = |id: &str| serde_json::json!({"item": {"id": id}});
        let mut items = RefusedItems::default();
        items.remember("a");
        items.remember("b");

        let mut events = vec![OutputEvent::ToolResult {
            id: "a".into(),
            outcome: ToolOutcome::Failed,
        }];
        items.attribute(method::ITEM_STARTED, Some(&end_of("a")), &mut events);
        assert_eq!(
            items.ids.len(),
            2,
            "a line that is not an end took out the memory"
        );
        assert!(matches!(
            events[0],
            OutputEvent::ToolResult {
                outcome: ToolOutcome::Failed,
                ..
            }
        ));

        let mut events = vec![
            OutputEvent::ToolResult {
                id: "b".into(),
                outcome: ToolOutcome::Failed,
            },
            OutputEvent::ToolResult {
                id: "a".into(),
                outcome: ToolOutcome::Declined,
            },
            OutputEvent::ToolResult {
                id: "a".into(),
                outcome: ToolOutcome::Completed,
            },
        ];
        items.attribute(method::ITEM_COMPLETED, Some(&end_of("a")), &mut events);
        let outcomes: Vec<ToolOutcome> = events
            .iter()
            .map(|e| match e {
                OutputEvent::ToolResult { outcome, .. } => *outcome,
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(
            outcomes,
            vec![
                ToolOutcome::Failed,
                ToolOutcome::Refused,
                ToolOutcome::Completed
            ]
        );
        assert_eq!(items.ids, VecDeque::from(vec!["b".to_string()]));
    }

    #[test]
    fn a_refused_item_id_is_bounded_exactly_at_the_decoders_id_limit() {
        for method_name in [
            method::ITEM_COMMAND_EXECUTION_REQUEST_APPROVAL,
            method::ITEM_FILE_CHANGE_REQUEST_APPROVAL,
        ] {
            let at = "x".repeat(MAX_ID_BYTES);
            let past = "x".repeat(MAX_ID_BYTES + 1);
            let params = |id: &str| serde_json::json!({"itemId": id});
            assert_eq!(
                refused_item_id(method_name, Some(&params(&at))),
                Some(at.as_str()),
                "{method_name}: dropped an id within the cap"
            );
            assert_eq!(
                refused_item_id(method_name, Some(&params(&past))),
                None,
                "{method_name}: pointed to an id outside the cap"
            );
        }
    }

    #[test]
    fn a_refused_command_approval_turns_its_failed_end_into_refused() {
        let mut h = harness_with_real_decoder();
        feed(&mut h, &[command_approval(0, "exec-1")]);
        let lines = refusals(&h.state);
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert_eq!(lines[0]["id"], 0);

        feed(&mut h, &[command_end("exec-1", "failed")]);
        assert_eq!(
            tool_results(&h.seen),
            vec![result("exec-1", ToolOutcome::Refused)]
        );
        assert!(
            h.reader.refused.ids.is_empty(),
            "the end did not take out the memory"
        );
    }

    #[test]
    fn a_failed_end_we_never_refused_stays_failed() {
        let mut h = harness_with_real_decoder();
        feed(&mut h, &[command_end("exec-1", "failed")]);
        assert_eq!(
            tool_results(&h.seen),
            vec![result("exec-1", ToolOutcome::Failed)]
        );
    }

    #[test]
    fn in_one_reader_our_refusal_and_the_vendors_own_decline_keep_their_own_outcomes() {
        let mut h = harness_with_real_decoder();
        feed(
            &mut h,
            &[
                command_approval(0, "exec-1"),
                command_end("exec-2", "declined"),
                command_end("exec-1", "failed"),
            ],
        );
        assert_eq!(
            tool_results(&h.seen),
            vec![
                result("exec-2", ToolOutcome::Declined),
                result("exec-1", ToolOutcome::Refused)
            ]
        );
    }

    #[test]
    fn a_refused_file_change_approval_turns_its_end_into_refused_whether_declined_or_failed() {
        let mut h = harness_with_real_decoder();
        feed(
            &mut h,
            &[
                file_change_approval(0, "patch-1"),
                patch_end("patch-1", "declined"),
                file_change_approval(1, "patch-2"),
                patch_end("patch-2", "failed"),
            ],
        );
        assert_eq!(refusals(&h.state).len(), 2);
        assert_eq!(
            tool_results(&h.seen),
            vec![
                result("patch-1", ToolOutcome::Refused),
                result("patch-2", ToolOutcome::Refused)
            ]
        );
    }

    #[test]
    fn requests_that_do_not_point_at_a_drawn_tool_item_are_refused_but_not_remembered() {
        let with_item = |extra: Value| {
            let mut p = serde_json::json!({"threadId": "T", "turnId": "u-1", "itemId": "exec-1"});
            p.as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            p
        };
        for (name, request) in [
            (
                "item/tool/call",
                server_request(0, "item/tool/call", with_item(serde_json::json!({}))),
            ),
            (
                "item/permissions/requestApproval",
                server_request(
                    0,
                    "item/permissions/requestApproval",
                    with_item(serde_json::json!({})),
                ),
            ),
            (
                "sub-command approval",
                server_request(
                    0,
                    method::ITEM_COMMAND_EXECUTION_REQUEST_APPROVAL,
                    with_item(serde_json::json!({"approvalId": "sub-1"})),
                ),
            ),
            (
                "network approval",
                server_request(
                    0,
                    method::ITEM_COMMAND_EXECUTION_REQUEST_APPROVAL,
                    with_item(
                        serde_json::json!({"networkApprovalContext": {"host": "example.com"}}),
                    ),
                ),
            ),
            (
                "no itemId",
                server_request(
                    0,
                    method::ITEM_COMMAND_EXECUTION_REQUEST_APPROVAL,
                    serde_json::json!({"threadId": "T", "turnId": "u-1"}),
                ),
            ),
            (
                "approval under the old name (callId)",
                server_request(
                    0,
                    "execCommandApproval",
                    serde_json::json!({"conversationId": "T", "callId": "exec-1"}),
                ),
            ),
        ] {
            let mut h = harness_with_real_decoder();
            feed(&mut h, &[request, command_end("exec-1", "failed")]);
            assert_eq!(
                refusals(&h.state).len(),
                1,
                "{name}: not exactly one refusal"
            );
            assert!(h.reader.refused.ids.is_empty(), "{name}: remembered it");
            assert_eq!(
                tool_results(&h.seen),
                vec![result("exec-1", ToolOutcome::Failed)],
                "{name}"
            );
        }
    }

    /// ⑤ continued — an `itemId` over the cap (128 B) is not remembered and the translator emits no result for its end (no row to point to).
    /// ★Whether it is remembered is looked at **before** the end line★ — the end line takes out a remembered id in its original shape, so
    /// afterwards it would look empty even if the filtering were missing.
    #[test]
    fn an_item_id_past_the_id_bound_is_neither_remembered_nor_given_a_result() {
        let long = "x".repeat(MAX_ID_BYTES + 1);
        let mut h = harness_with_real_decoder();
        feed(&mut h, &[command_approval(0, &long)]);
        assert_eq!(refusals(&h.state).len(), 1);
        assert!(
            h.reader.refused.ids.is_empty(),
            "remembered an id outside the cap"
        );

        feed(&mut h, &[command_end(&long, "failed")]);
        assert!(
            tool_results(&h.seen).is_empty(),
            "{:?}",
            tool_results(&h.seen)
        );
    }

    #[test]
    fn a_command_approval_whose_sub_and_network_fields_are_null_is_still_remembered() {
        let mut h = harness_with_real_decoder();
        feed(
            &mut h,
            &[
                server_request(
                    0,
                    method::ITEM_COMMAND_EXECUTION_REQUEST_APPROVAL,
                    serde_json::json!({"threadId": "T", "turnId": "u-1", "itemId": "exec-1",
                                       "approvalId": null, "networkApprovalContext": null}),
                ),
                command_end("exec-1", "failed"),
            ],
        );
        assert_eq!(
            tool_results(&h.seen),
            vec![result("exec-1", ToolOutcome::Refused)]
        );
    }

    #[test]
    fn the_end_line_takes_the_memory_out_whatever_its_status() {
        let mut h = harness_with_real_decoder();
        feed(
            &mut h,
            &[
                command_approval(0, "exec-1"),
                command_end("exec-1", "failed"),
                command_end("exec-1", "failed"),
                file_change_approval(1, "patch-2"),
                patch_end("patch-2", "declined"),
                patch_end("patch-2", "declined"),
                command_approval(2, "exec-3"),
                command_end("exec-3", "completed"),
            ],
        );
        assert!(
            h.reader.refused.ids.is_empty(),
            "the `completed` end did not take out the memory"
        );
        feed(&mut h, &[command_end("exec-3", "failed")]);
        assert_eq!(
            tool_results(&h.seen),
            vec![
                result("exec-1", ToolOutcome::Refused),
                result("exec-1", ToolOutcome::Failed),
                result("patch-2", ToolOutcome::Refused),
                result("patch-2", ToolOutcome::Declined),
                result("exec-3", ToolOutcome::Failed),
            ]
        );
    }

    #[test]
    fn past_the_slot_cap_the_oldest_refusal_loses_its_attribution() {
        let mut h = harness_with_real_decoder();
        let requests: Vec<String> = (0..=REFUSED_ITEM_SLOTS)
            .map(|i| command_approval(i as i64, &format!("exec-{i}")))
            .collect();
        feed(&mut h, &requests);
        assert_eq!(refusals(&h.state).len(), REFUSED_ITEM_SLOTS + 1);
        assert_eq!(h.reader.refused.ids.len(), REFUSED_ITEM_SLOTS);

        feed(
            &mut h,
            &[
                command_end("exec-0", "failed"),
                command_end("exec-1", "failed"),
            ],
        );
        assert_eq!(
            tool_results(&h.seen),
            vec![
                result("exec-0", ToolOutcome::Failed),
                result("exec-1", ToolOutcome::Refused)
            ]
        );
    }

    #[test]
    fn a_turn_end_does_not_clear_the_memory() {
        let mut h = harness_with_real_decoder();
        make_ready(&h.state, "T");
        activate(&h.state, 0, Some("u-1"));
        feed(&mut h, &[command_approval(0, "exec-1")]);
        h.reader
            .handle_line(completed_line("T", "u-1", "completed").as_bytes());
        assert_eq!(turn_seq(&h.state), None, "premise: the turn closed");
        assert_eq!(boundaries(&h.seen).len(), 1, "premise: a boundary stood");

        feed(&mut h, &[command_end("exec-1", "failed")]);
        assert_eq!(
            tool_results(&h.seen),
            vec![result("exec-1", ToolOutcome::Refused)]
        );
    }

    #[test]
    fn a_refusal_that_never_reached_the_control_queue_is_not_remembered() {
        let mut full = harness_with_real_decoder();
        with_state(&full.state, |s| {
            for i in 0..OUTBOX_LIMIT {
                s.outbox.push_back(format!("{{\"filler\":{i}}}\n"));
            }
        });
        let mut closed = harness_with_real_decoder();
        with_state(&closed.state, |s| s.closed = true);

        for (name, h) in [("overflow", &mut full), ("closed", &mut closed)] {
            feed(
                h,
                &[
                    command_approval(0, "exec-1"),
                    command_end("exec-1", "failed"),
                ],
            );
            assert!(
                refusals(&h.state).is_empty(),
                "{name}: a refusal line went in"
            );
            assert!(h.reader.refused.ids.is_empty(), "{name}: remembered it");
            assert_eq!(
                tool_results(&h.seen),
                vec![result("exec-1", ToolOutcome::Failed)],
                "{name}"
            );
        }
    }

    /// ⑩ New reader = empty memory — one reader's refusals do not leak into another (= the next incarnation).
    #[test]
    fn a_new_reader_starts_with_no_memory() {
        let mut first = harness_with_real_decoder();
        feed(&mut first, &[command_approval(0, "exec-1")]);
        assert_eq!(
            first.reader.refused.ids.len(),
            1,
            "premise: the first reader remembered it"
        );

        let mut next = harness_with_real_decoder();
        assert!(next.reader.refused.ids.is_empty());
        feed(&mut next, &[command_end("exec-1", "failed")]);
        assert_eq!(
            tool_results(&next.seen),
            vec![result("exec-1", ToolOutcome::Failed)]
        );
    }

    /// ⑪ ★Measured (codex-cli 0.156.1 · fixture `refuse_u2a` — third B5 capture)★: after each call of the command turn · the file change
    /// turn there is exactly one `Refused` result of the same id. One refusal line goes out for each of the two server request lines (those
    /// answers are not in the fixture — the reader builds them), and the in-between `serverRequest/resolved` · `thread/status/changed` do not
    /// shake the attribution.
    #[test]
    fn the_refusal_fixture_yields_each_call_then_exactly_one_refused_result() {
        const COMMAND: &str = "exec-e59b2cc3-9cdc-49a5-954d-a9e9ec098636";
        const PATCH: &str = "exec-af747e06-e382-48e9-b906-8d02f706d9d9";
        let mut h = harness_with_real_decoder();
        make_ready(&h.state, "01a0e674-807f-7370-8321-9afa484350cd");
        for line in include_str!("fixtures/refuse_u2a.jsonl").lines() {
            h.reader.handle_line(line.as_bytes());
        }

        let lines = refusals(&h.state);
        assert_eq!(
            lines.iter().map(|l| l["id"].clone()).collect::<Vec<_>>(),
            vec![Value::from(0), Value::from(1)],
            "{lines:?}"
        );
        let seen = h.seen.lock().unwrap();
        let tools: Vec<(&str, Option<ToolOutcome>)> = seen
            .iter()
            .filter_map(|e| match e {
                OutputEvent::ToolCall { id: Some(id), .. } => Some((id.as_str(), None)),
                OutputEvent::ToolResult { id, outcome } => Some((id.as_str(), Some(*outcome))),
                _ => None,
            })
            .collect();
        assert_eq!(
            tools,
            vec![
                (COMMAND, None),
                (COMMAND, Some(ToolOutcome::Refused)),
                (PATCH, None),
                (PATCH, Some(ToolOutcome::Refused)),
            ]
        );
        assert!(
            !seen.iter().any(|e| matches!(e, OutputEvent::Error(_))),
            "the refusal path emitted an error: {seen:?}"
        );
        drop(seen);
        assert!(
            h.reader.refused.ids.is_empty(),
            "memory remained after the end"
        );
    }

    #[test]
    fn the_same_item_refused_twice_takes_one_slot() {
        let mut h = harness_with_real_decoder();
        feed(
            &mut h,
            &[command_approval(0, "exec-1"), command_approval(1, "exec-1")],
        );
        assert_eq!(
            refusals(&h.state).len(),
            2,
            "an answer goes out separately for every request"
        );
        assert_eq!(h.reader.refused.ids.len(), 1);

        feed(
            &mut h,
            &[
                command_end("exec-1", "failed"),
                command_end("exec-1", "failed"),
            ],
        );
        assert_eq!(
            tool_results(&h.seen),
            vec![
                result("exec-1", ToolOutcome::Refused),
                result("exec-1", ToolOutcome::Failed)
            ]
        );
    }
}
