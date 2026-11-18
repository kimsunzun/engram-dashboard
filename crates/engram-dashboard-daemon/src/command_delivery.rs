//! Command delivery — finds the owner in the roster, **writes the envelope to that connection's frame outlet**, and carries the returned outcome
//! **to the connection that originally asked** (ADR-0154 · TRD §3-5·§3-8).
//!
//! The delivery rules themselves are not here — the tool crate's 3 stages ([`route`]) are common to every hop (ADR-0155 decision 3). This module adds
//! only the **four daemon-side pieces** those stages need:
//! ① the link from an owner token to a connection ([`OwnerLink`] — [`CommandRoster::sink_of`]) ·
//! ② the request correlation table ([`CommandDeliveries`] — `request_id` → origin connection · deadline) ·
//! ③ stage 1, which the daemon answers by itself ([`LocalCommands`]) — **only that stage runs outside `route`** (reason = [`deliver`]) ·
//! ④ the agent-reference field conversion before the relay ([`resolve_agent_args`] — ADR-0308): the daemon turns only the names in fields the owner
//!    marked into ids with its own resolver, without knowing screen content.
//!
//! ## ★Exactly one reply per received query★ (TRD §4-⑤)
//!
//! ★It is **not** "one frame per `request_id`" — that sentence is false for this code★: when several queries arrive with the same number
//! ([`Seat::Conflict`]·[`Seat::Taken`]·[`Seat::LocalStillRunning`]·[`Seat::LocalAlreadyRan`]·[`Seat::Closed`]) each gets **its own reply**.
//! What this module keeps is **one per query**; coalescing ([`Seat::Coalesced`]) is the branch where one in-flight reply answers two queries together.
//!
//! What keeps it is the **seat of the correlation table**, and the rules are two lines:
//! 1. **Open the seat first** ([`CommandDeliveries::open`]) and only then do anything — [`deliver`] opens it **before** `route`, so whichever
//!    stage makes the answer, that answer sits on one seat.
//! 2. **Release the seat only after the frame has gone out** ([`CommandDeliveries::release`]) — the end of the seat is the moment the frame **went out**, not the
//!    moment the outcome attaches; releasing in between lets a re-query of the same id see an empty table and send the envelope once more.
//!
//! ★Do not make a reply without going through the seat★ — **every** frame leaving this module rests on one decision of the table: a reply that won a seat, or a
//! refusal carrying the very decision that no seat was won. A late outcome gets nothing in [`CommandDeliveries::complete`] and is discarded.
//! (Stage 1 (own table) and stage 3 (owner absent) of `route` once bypassed the table and emitted a second frame under the key of a round trip in progress.)
//!
//! ## ★Stage 1 with the same number runs only once — but only **while it is remembered**★
//!
//! A command the daemon answers by itself (stage 1) has side effects and no verb to undo it (`agent.new` — ADR-0122), so a re-query of the same number is
//! stopped from applying it twice **by one seat**: while the body runs ([`Seat::LocalStillRunning`]) and after it ends ([`SeatState::Retained`]) that seat
//! keeps holding the number, and the point where the state changes is one critical section.
//! ★The owner of the number is **one seat table** — do not make a second table that collects numbers separately★: two authorities over one namespace
//! always diverge.
//! ★Only **what was applied** is held and the hold is **bounded**★ — decision = [`retains_the_id`], window = the one deadline, and a re-query outside
//! it cannot be told apart from a new request. An unbounded guarantee needs a real dedup store that replays completed work (TRD §4-⑥ · see [`no_retry`]) — this hold is **not** that store.
//!
//! ## ★Do not hold a handed-out copy beyond a round trip★ (ADR-0154)
//!
//! The roster cannot take back a frame outlet copy it handed out. So [`OwnerLink::send`] writes with the copy on the spot and drops it, and when giving the
//! reply it **looks up the origin connection again**. The correlation table holds only the connection **identifier** — a copy kept here would outlive `on_disconnect` and leak a live connection.
//!
//! ## ★The correlation table and the roster are different tables — they are not merged★ (TRD §4-④)
//!
//! The lifetime units differ: a correlation entry hangs on **one round trip** and also vanishes at the deadline, a roster registration hangs on **one connection**
//! and vanishes only at its end. Merged, one deadline would erase the vocabulary.
//!
//! ★This module calls [`CommandRoster::detach`] in **one place** — the destructor of [`CallerSeat`]★: it only reaps the one fake caller connection it set up (no name is
//! registered, so there is no roster entry to erase), and its number space does not overlap with the disconnect cleanup of a real connection (carve-out canonical in the `detach` doc).
//! ★That destructor reaps **both tables together**★ (`detach` + `drop_origin`, the same pair as the real disconnect hook) — a fake connection has no such hook, so reaping only
//! one side would leave the other for the whole process lifetime.
//! ★For "together" to hold **the first line must not panic**★: the roster's `lock_for_cleanup` inside [`CommandRoster::detach`] and
//! [`CommandDeliveries::lock_for_cleanup`] each reclaim a poisoned lock (base `sync::lock`). Reverting either to a panicking lock kills the destructor on that line
//! so the later line does not run, and during unwinding it is a double panic that aborts the process. ★The production process has no poisoning★ (a panic holding the two tables is outside the
//! containment marker and the hook ends the process first — ADR-0292); the discipline pays off in test builds without the hook (range = the doc of the two `lock_for_cleanup`).
//! ★Cleaning the correlation table does not touch the roster★ — deadline · completion · disconnect cleanup all finish inside the seat table.
//!
//! ## ★Two entrances, one machine★ (ADR-0160)
//!
//! [`deliver`] is called from two places — socket dispatch and the control route ([`CommandBus::invoke`]) — and both use the same seat table · deadline · duplicate prevention.
//! ★Do not make a different device per entrance★: with two wait tickets one entrance cannot see a seat the other opened, and a re-query of the same number applies the same operation twice.
//! ★But **of the 3 stages, stage 1 is ridden only by the socket entrance**★ — [`CommandBus::invoke`] refuses the daemon's own names and the decision part
//! (`control::catalog::handle_call`) answers them in its own place. The reason is cancellation: the stage 1 body is `spawn_blocking` and is not cancelled while an HTTP handler
//! future is, so the seat · cap would be left without cleanup (full text = the comment at that refusal). So "two entrances" concerns **stages 2·3**, and the entrance label of stage 1
//! ([`LocalCommands::run`]'s `entrance`) has one value today.
//!
//! ## Entry points
//!
//! [`deliver`] runs a whole round trip (it must run **outside** the connection task — see its doc) · [`CommandBus::invoke`] puts a connectionless caller (the control route) on it ·
//! [`CommandDeliveries::complete`] attaches the owner's outcome · [`CommandDeliveries::expire`] and [`CommandDeliveries::drop_origin`] reap the other two.
// ADR-0154
// ADR-0160

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::conn_port::{ConnId, Frame, FrameError, FrameSink};
use engram_dashboard_agent::commands::{resolve_in, AgentRosterRow};
use engram_dashboard_base::sync;
use engram_dashboard_base::time::{Clock, SystemClock};
use engram_dashboard_command::{
    route, CommandDecl, CommandEnvelope, CommandError, CommandLink, CommandReply, CommandTable,
    Effect, ErrorCode, OwnerToken, RequestId, RetryMode,
};
use engram_dashboard_protocol::AgentEvent;
use tokio::sync::{oneshot, watch};

use crate::command_roster::CommandRoster;
use crate::connection_core::{
    envelope_to_wire, event_json, reply_from_wire, reply_to_wire, sanitize_for_log, sanitize_within,
};

/// The wording when the connection that asked has already left — this reply has nowhere to go either (that connection vanished).
const CALLER_ALREADY_GONE: &str = "the calling connection went away before the envelope was sent";

/// The door that called delivery — the value of the log field `entrance` and the argument of [`deliver`].
///
/// ★Do not write the literal at each place★: the point is to tell the incidents of the two entrances apart by this field, and if the spelling
/// diverges per place that filter quietly catches only half. `"cli"` belongs to the **direct execution** of the control route that does not ride delivery
/// so it is not here (that place = the call site of `control::commands::call_daemon_command`).
pub const ENTRANCE_SOCKET: &str = "bus";
/// A relay the control route ran with another's name.
///
/// ★Two places emit this value and they are **the before and after of the same call**★: the line that writes who ordered it when the relay starts
/// (`control::mcp_server::control_call_handler`) and the line that writes its outcome ([`CommandBus::invoke`]). If only one side
/// uses this value "who ordered it" and "how it ended" split into different labels and the filter catches only half.
pub const ENTRANCE_CONTROL: &str = "control";

/// **Every** failure answer this path builds — ★the retry instruction is `Never` without exception★.
///
/// ## Why not use an instruction derived from the code
///
/// The ruler of ADR-0159 is "safety × usefulness", and the **safety** leg of `same-request-id` is "asking again with the same number does not
/// apply the same operation twice". That premise holds **only when the first receiving daemon caches completed work and replays it**
/// (TRD §4-⑥ "completed-work replay"), but that dedup store exists nowhere in this repo (measured). Without a store a re-query is not a re-query but **a
/// re-execution**, and a command with side effects is applied twice. If the safety leg is false the decision is "end" regardless of usefulness — so everything is `Never`.
///
/// ★The code vocabulary is left as it is★: `OUTCOME_UNKNOWN` is a **statement of certainty** so it stays as fact (it is not known whether it was applied),
/// and what changes is **only the instruction**. This is why the contract has two separate fields. No new error code is made.
///
/// ★Condition to resume★ — when a dedup store (completed-work replay + TTL) stands, lift this function and revive the instruction derived from the code.
/// Reviving it before then would make "it is safe" a lie.
///
/// ★The only way this file builds an error is here★ — the place guarded by machine is
/// `tests::this_module_can_only_build_do_not_retry_failures`.
// ADR-0159
fn no_retry(code: ErrorCode, detail: impl Into<String>) -> CommandError {
    CommandError::with_retry(code, detail, RetryMode::Never)
}

/// Does the deadline fit within the caller's **silence limit** — ★the inequality exists in this one function only★ (ADR-0161 decision 3).
///
/// Left as prose, an edit fixing either side quietly flips the relation — it really was flipped, and the symptom was a
/// **false report** of "could not reach the daemon" (full basis = [`CommandDeliveries::DEFAULT_DEADLINE`]).
/// ★But measuring only the constant cannot bite the **running value**★: the deadline production sees is a field of [`CommandDeliveries`]
/// and that value is decided by [`CommandDeliveries::with_clock`]. So this predicate is hung in two places —
/// the default by the `const` assertion below (at compile time) and the injected value by that constructor (at construction time), judged by the same function.
/// ★Compare in milliseconds — comparing in seconds makes this decision **looser than its own prose**★: `as_secs()` is a truncation that discards, so
/// a deadline of 8.5 s + a period of 1.4 s passes as `8 + 1 < 10` while the real worst case is 9.9 s. The product below is `u128` arithmetic so
/// it cannot overflow.
const fn fits_caller_silence_window(deadline: Duration) -> bool {
    deadline.as_millis()
        + CommandDeliveries::SWEEP_INTERVAL.as_millis()
        + CommandDeliveries::CALLER_MARGIN.as_millis()
        <= (engram_dashboard_agent::types::CLI_CONTROL_READ_TIMEOUT_SECS as u128) * 1_000
}

const _: () = assert!(
    fits_caller_silence_window(CommandDeliveries::DEFAULT_DEADLINE),
    "command deadline + reap period + margin exceeds the CLI's silence limit — the caller cuts off before the reply goes out"
);

/// One in-flight round trip — **the origin connection**, **the deadline**, and the place to take the reply.
///
/// ★Do not put a frame outlet copy here★ — this entry can outlive the connection, and one surviving copy keeps the disconnected connection's writer
/// from ending itself (module header · ADR-0154).
struct Pending {
    /// The connection to return the reply to. **It has no generation number — so it relies on connection ids not being reused.**
    ///
    /// ★The safety lies not in the delivery code but in the **allocator** (measured 2026-08-18)★: the connection id is drawn by `ConnRegistry::alloc_id` of the network layer
    /// `link/server.rs` as `fetch_add(1)` from an `AtomicU64` (initial value 1), and release
    /// (`unregister`) only erases it from the map and never puts that value back. The registry is made once in the daemon assembly so
    /// that counter increases monotonically for the whole process lifetime, and exhausting u64 is unreachable.
    /// ★If that property changes this breaks★ — if ids become reused (pool return · hash · client-supplied value), a seat left before the
    /// deadline would hand another's reply to **another peer that newly got the same number**. What is needed then is attaching a generation number to this
    /// field, and the reason it is not attached ahead of time is that it is a cost for an unreachable risk.
    origin: ConnId,
    deadline: Instant,
    /// ★The seat's ID card — **not the key, this is what points at that round trip**★.
    ///
    /// Without it cleanup picks a seat by `request_id` alone, and in between the earlier-opened seat may be gone and **a new seat with the same key**
    /// may be standing (the case where after the deadline or a close reaped it the caller re-queried with the same id per the contract —
    /// the contract does not forbid a re-query of the same number so this code must tolerate it). If picked by key a late task would
    /// reap **another's seat** and send out its own reply so there would be two frames. So [`CommandDeliveries::release`]
    /// compares the key and this table **together**.
    token: SeatToken,
    /// The **identity** of the request that opened this seat — this is what tells whether seeing the same id again is a re-query or a different request.
    ///
    /// ★It lives bearing a resident cost★: it keeps one copy of the envelope's name · arguments in the seat for the round trip. Without it
    /// there is no way to distinguish "same id + a different payload" (TRD §4-⑥ third leg) and another request would receive another's answer.
    /// There is not yet a resident cap for the whole table — the record of that open item is the [`CommandDeliveries`] header.
    name: String,
    args: serde_json::Value,
    state: SeatState,
    /// ★This round trip is **answered by the daemon itself**★ — meaning the envelope went to no one, and this one field
    /// decides three things:
    ///
    /// 1. **It does not accept an outcome sent by another** ([`CommandDeliveries::complete`]). Since no envelope was sent the `CommandOutcome` arriving with this
    ///    number is **by definition not a legitimate reply** — accepting it would push out the handler's real result with that payload as the reply
    ///    (and the real result would be quietly discarded).
    /// 2. **It does not release the seat even after the deadline** ([`Seat::LocalStillRunning`]) — the body cannot be cancelled and
    ///    keeps running, and releasing the seat lets a re-query of the same number run **the same Write verb once more**.
    /// 3. **It stays through disconnected-caller cleanup too** ([`CommandDeliveries::drop_origin`]) — the same reason.
    ///
    /// ★Decided **when the seat is born**★: turning it on later lets the outcome of item 1 arriving in between pass (a narrow window
    /// is not an absent one — connection tasks run separately so they really overlap).
    local: bool,
}

/// A monotonic number from the order seats are opened — [`Pending::token`].
type SeatToken = u64;

/// The standing of a seat — ★it splits by whether the reply is **not yet decided / carries a result / is empty-handed**★.
///
/// The three are distinguished because how to treat "a re-query with the same id" diverges: if a reply carrying a result is going out
/// a re-query can share that answer, but a seat folded up empty-handed **must not absorb the next attempt**
/// (absorbing it makes the retry a quiet no-op and the caller gets an already-settled answer before asking again).
enum SeatState {
    /// Waiting for the answer — this seat is that round trip's reply ticket.
    Awaiting(oneshot::Sender<CommandReply>),
    /// The outcome has attached — the frame is going out and it is still before [`CommandDeliveries::release`].
    Fulfilled,
    /// Folded **without** an outcome (deadline exceeded). The frame to go out has no result so a re-query must try anew.
    ///
    /// ★The residue that choice leaves — a caller who reuses a number can receive another's outcome★
    ///
    /// If this seat is overturned and a request of **different content** with the same number opens a new seat, the **late outcome of the old attempt**
    /// arriving afterwards settles into that new seat — what the outcome frame carries is only the request number so this table has no means
    /// to tell which attempt's reply it is (the same reason [`CommandDeliveries::complete`] does not even look at the sending connection). To tell apart the outcome
    /// would also have to carry a ticket, and that means growing the wire contract.
    /// ★The basis for tolerating this today★: this path lowers every retry instruction to `Never` ([`no_retry`]) so it **does not recommend
    /// reusing numbers.** Even so it still happens to a caller who breaks the convention — it is not a risk removed but
    /// **a risk narrowed by advice**.
    /// ★But **a stage 1 seat is not overturned**★ — it has side effects and no verb to undo, so it cannot bear the residue above.
    /// That seat is blocked by [`Seat::LocalStillRunning`] while the body runs and by [`SeatState::Retained`] after it ends.
    Void,
    /// **Applied and holding that number** — the reply has already gone out.
    ///
    /// ★The reason it stays in this standing without releasing the seat★: the most common shape of a re-query after the deadline is "deadline → `TIMEOUT` →
    /// body completes → re-query", and releasing the seat the moment it completes lets that re-query see an empty table and run **the same Write verb again**
    /// (one more agent comes up and there is no verb to delete it — ADR-0122).
    /// ★The point is that the place holding the number is **one seat only**★ — with a second table doing the same job (a memory collecting only numbers)
    /// two authorities diverge over one namespace: a seat that did not run remains in the memory, or unrelated read
    /// traffic pushes out the memory, or the two lifetimes drift apart. The seat is the owner of that number.
    /// ★The hold period is one deadline window★ (`until`) — once the window passes the collector releases it ([`CommandDeliveries::expire`]).
    /// ★A disconnected caller's too **remains** until the window passes★: [`CommandDeliveries::drop_origin`] does not
    /// touch stage 1 seats (the reason for that filter is in that function — a disconnect is exactly the event where that hold is most needed).
    /// The only place that releases is the collector.
    Retained { until: Instant },
}

/// The result of trying to attach an outcome the owner sent — ★the caller **must treat the three differently**★.
///
/// Folding into a single bool of "attached / not attached" would give [`OutcomeLanding::NotDelegated`] the same face as a late
/// outcome — the former is an **impossible frame** (that number has no owner) and the latter a common normal race.
// ADR-0154
#[derive(Debug, PartialEq, Eq)]
pub enum OutcomeLanding {
    /// Attached to that round trip — the delivery task wakes and gives the reply.
    Attached,
    /// No place to attach — it was reaped after the deadline, or nobody asked with that number.
    NoSeat,
    /// ★The round trip of that number was **never delegated to anyone**★ — a place the daemon answers by itself.
    NotDelegated,
}

/// The result of trying to open a seat — ★the branches split by **how many frames they emit** and **what instruction they carry**★.
/// The table has one seat per `request_id`. Answering without a decision when that key is already taken would give the same key two frames (a TRD §4-⑤ violation),
/// so "could not get a seat" is split by type rather than lumped into one value.
/// ★A refusal branch emits a frame too — that is **the only answer to that query**★: the decision itself was made under one lock of the table, so one answer per query still holds.
enum Seat {
    Opened {
        rx: oneshot::Receiver<CommandReply>,
        token: SeatToken,
    },
    /// ★**The same connection** asked **the same request** again — no second reply is emitted★.
    /// This is the **normal path**, not an error: the one reply of the in-flight round trip answers both requests (TRD §4-⑥ "in-flight duplicate = coalesce into the same pending").
    /// ★Do not emit a second reply here★ — the same key would get two frames and the caller's pending would resolve on the first and receive the other as an orphan. Coalescing is how to **not make** that
    /// second frame and also how not to send the envelope twice (prevents applying the same operation twice).
    /// ★Coalescing and "it is fine to ask again" are separate★ — this path does not instruct a retry ([`no_retry`]); coalescing is about how to treat **a duplicate query that has already arrived**.
    Coalesced,
    /// **The same id but a different request** (name · arguments differ) — the third leg of TRD §4-⑥.
    /// ★Why it must not be coalesced★: coalesced, this request would **never be delivered** and never get its reply, and the one frame going out under that key would carry
    /// **another request's result** — the caller resolves its promise with the wrong payload. So it is refused with `REQUEST_ID_CONFLICT`, a code that is **the plain fact**:
    /// nothing was executed (certainty = certain) and the only cause is that the number is already bound to another request. What the caller should do is **use a new id**.
    Conflict,
    /// **Another connection** holds the same request on that id.
    /// This side gets a reply — otherwise this caller would hang with no answer (that connection has no seat so deadline reaping cannot touch it). That reply goes to a **different connection** than
    /// the reply of the round trip opened first, so "one reply" per caller still holds.
    /// ★Which code to carry depends on **whether that seat's owner is still attached** — [`deliver`] decides★ (it cannot be decided here: the correlation table does not know the roster).
    Taken { holder: ConnId },
    /// **The deadline has passed but the stage 1 body of that number is still running** ([`Pending::local`]).
    /// ★The point is not to run it again here★: this seat already answered `TIMEOUT` but the body cannot be cancelled (blocking pool) and keeps running. Overturning it and running anew
    /// would apply the same Write verb twice.
    /// ★It comes **before the payload comparison**★ — the same holds even if different arguments arrive with the same number. What this daemon is doing with that number right now is the dominant fact, and until it ends no branch may open a new execution.
    LocalStillRunning,
    /// **Stage 1 of that number has already been applied** — the seat still holds the number ([`SeatState::Retained`]).
    /// ★Without this branch a re-query after the deadline applies the same Write verb a second time★ (the most common shape: deadline → `TIMEOUT` → body completes → re-query).
    LocalAlreadyRan,
    /// The table is closed — the daemon is going down so it takes no new round trips ([`CommandDeliveries::drain`]).
    Closed,
}

/// The request correlation table — all connections see the same one (an `Arc` clone sees the same table).
///
/// ## Residency — **only one axis is closed**
///
/// - **In-flight seats**: this table itself has no cap. Seats that can be opened are one per round trip and the deadline ([`CommandDeliveries::DEFAULT_DEADLINE`])
///   reaps them so it does not grow without bound, but that is **not the same as having a cap** — neither the seat count nor the size of the name · arguments a seat holds
///   ([`Pending::name`]·[`Pending::args`]) is measured. ★The two entrances differ★: on the control route that door carries its own cap ([`MAX_RELAYED_IN_FLIGHT`] —
///   global 64). The socket side has none, and what tolerates that is the **connection lifetime** — that entrance's seats hang on one connection and disconnect
///   cleanup reaps them ([`CommandDeliveries::drop_origin`]). ★Do not read this as "inside the auth boundary so one trusted client"★: both entrances are inside the auth boundary
///   (ADR-0153 — the socket by the daemon token, the control route by an issued agent credential), but the latter keys are **one per agent, several, and do not know each other**,
///   so that door's cap is carried by that constant, not by trust. If a cap is put on this table the precedent is the roster's
///   (`engram_dashboard_command::Roster::MAX_NAMES`).
/// - **Held seats**: closed. These live **after** the round trip ends for one more window, so residency becomes **arrival rate × window**, not "concurrency" — so they are bounded by both time (the window) and count
///   ([`MAX_RETAINED_LOCAL`]), and the payload is discarded when crossing it ([`CommandDeliveries::settle_local`]). ★Do not read this axis as "the deadline reaps it so it is fine"★ — held seats are not reaped by the deadline.
// ADR-0154
#[derive(Clone)]
pub struct CommandDeliveries {
    inner: Arc<Mutex<Seats>>,
    /// ★Why injected★: the deadline overrun is decided by one thing, **has the time passed**, and reading `Instant::now()` directly would make a test measuring that branch use real
    /// waiting (slow, and it flips on a loaded runner). Plugged in from outside the test **pushes the time by hand** and decides deterministically (ADR-0012).
    clock: Arc<dyn Clock>,
    deadline: Duration,
    max_local: usize,
    max_resolving: usize,
}

/// The inside of the table — seats + ticket number + closed marker + the number of stage 1 running now.
#[derive(Default)]
struct Seats {
    open: HashMap<RequestId, Pending>,
    next_token: SeatToken,
    /// ★A closed table **refuses but still answers**★ — basis = [`CommandDeliveries::drain`].
    closed: bool,
    /// The number of stage 1 bodies currently out on the blocking pool — material for the cap decision ([`MAX_LOCAL_IN_FLIGHT`]).
    /// ★Do not substitute by counting seats★: seats also count round trips being relayed so traffic arriving under another's name would eat our execution cap. This number
    /// counts **only what this daemon is actually running**.
    local_running: usize,
    /// **When the cap event was last written** — if absent no line has been written yet.
    /// ★Do not reopen by the count (that would be the opposite)★: in saturation one refusal is followed by one completion, so "re-arm when the count drops below the cap" becomes
    /// **one line per command** and "the other side decides the log size", which this set out to prevent, happens as is. Conversely in a truly bitten state (the count never drops)
    /// re-arming never happens so there is **one line for the whole process lifetime** — the loudest situation becomes the quietest. Opening by time releases both:
    /// at most one line per [`CEILING_WARN_INTERVAL`], continuing even while bitten.
    ceiling_warned_at: Option<Instant>,
    /// The number of agent-reference field resolutions (roster reads) currently running on the blocking pool — material for the cap decision ([`MAX_RESOLVING_IN_FLIGHT`]).
    resolving: usize,
    /// When the resolution cap event was last written — the discipline is the same as [`Seats::ceiling_warned_at`].
    resolve_warned_at: Option<Instant>,
    /// ★Has a sweeper **already been attached** to this table★ — [`CommandDeliveries::spawn_sweeper`] refuses an assembly that attaches a second one by this value.
    /// ★Visibility alone cannot prevent it (that is the basis of this field)★: the table is shared via [`CommandBus::deliveries`], `CommandDeliveries` is `Clone` and [`CommandBus::new`] is `pub`, so **even outside the module**
    /// `CommandBus::new(.., bus.deliveries().clone(), ..)` could set up a second sweeper. The moment that marker falls, [`CommandDeliveries::drain`] on its way out **closes the live daemon's shared table**
    /// and every later round trip becomes [`Seat::Closed`] (the daemon is fine yet every command is "shutting down").
    /// ★So this field makes **the table itself** remember its sweeper★ — an assembly trying to make that state panics on the spot instead of quietly succeeding (precedent = the duplicate-declaration panic of
    /// `CommandTable::new` — an assembly accident should blow up earlier than the assembly).
    swept: bool,
}

impl Seats {
    /// Brings the number of held seats back below [`MAX_RETAINED_LOCAL`] — releases **the oldest first**.
    /// ★The time boundary alone does not close the table★: the number of requests that can enter the window has no cap (arrival rate × window) so the count must be bounded too for residency to be bounded. One client running
    /// `agent.rename` fast really makes that product.
    /// ★An evicted number loses its protection★ — a re-query of that number becomes a new request and runs again. Even so the oldest go first because re-queries cluster at the **front** of the window (they come right after the deadline).
    /// ★Scan cost★: held seats number at most this cap and this function runs only when one more is held.
    fn evict_oldest_retained(&mut self) {
        let mut retained: Vec<(RequestId, Instant)> = self
            .open
            .iter()
            .filter_map(|(id, pending)| match pending.state {
                SeatState::Retained { until } => Some((*id, until)),
                _ => None,
            })
            .collect();
        let Some(excess) = retained.len().checked_sub(MAX_RETAINED_LOCAL) else {
            return;
        };
        if excess == 0 {
            return;
        }
        retained.sort_unstable_by_key(|(_, until)| *until);
        for (id, _) in retained.into_iter().take(excess) {
            self.open.remove(&id);
        }
    }
}

impl CommandDeliveries {
    /// The default deadline (TRD §4-⑥).
    /// ★By contract the deadline is **set by the caller** — today that field is not on the wire so the daemon uses a default★: `CommandEnvelope` has no deadline field so there is no way yet for the caller to carry a value. When the
    /// field appears (additive) this becomes its fallback.
    /// ★This value must be inside the caller's **silence limit** — [`fits_caller_silence_window`] bites it★: the caller of the control route (ADR-0160) is the `engram` CLI and its socket cap is
    /// [`CLI_CONTROL_READ_TIMEOUT_SECS`]. Exceeding it makes **the client cut off before the reply goes out**, the user sees "could not reach the daemon" (false — it reached and may have applied) and the daemon loses a place to send the reply.
    /// The old 10 s was exactly that state: both sides at 10 s, but the CLI's clock starts first (connection · parsing · blocking pool scheduling) and this side's reply goes out only after
    /// [`CommandDeliveries::SWEEP_INTERVAL`], so the client **deterministically** cut off first.
    /// ★That cap is **not a total budget — it is `set_read_timeout` so it is a "silence limit"**★ (measured — CLI `post_json`): the condition for the client to give up is "after sending the request **one read** receives no byte
    /// within that time". It measures **a run of consecutive silence**, not total elapsed time. Since this route sends nothing before the answer the two are effectively the same today, but do not write a sentence that leans on that property — if it changes to a shape
    /// that streams bytes midway (chunks · progress notices) the client does not cut off even when total elapsed exceeds this value, and a test that reads this value as a "budget" and measures total elapsed measures the wrong thing from that day.
    /// ★The arithmetic of 7 s★: limit 10 − reap period 1 − margin 2 ([`CommandDeliveries::CALLER_MARGIN`]) = 7. It buys **observability**, not throughput, so it is set toward the margin side.
    /// ★The socket entrance uses the same value — that is the point★: the deadline is a property of one seat table and if it diverges per entrance it becomes two machines (module header). The shell side has no constant tied to this value (measured — `HANDSHAKE_TIMEOUT` is for the handshake only).
    pub const DEFAULT_DEADLINE: Duration = Duration::from_secs(7);

    /// The expiry reap period — a deadline overrun can be observed this much later.
    /// ★The point is **not to ride the reaping on traffic**★: a shape that sweeps when the next frame comes means on a quiet daemon the deadline never passes, and the caller hangs with neither answer nor error (the unbounded wait ADR-0154 pointed at).
    const SWEEP_INTERVAL: Duration = Duration::from_secs(1);

    /// The part of time that flows **outside** the round trip — connection · request parsing · `spawn_blocking` wait · response serialization.
    /// ★The inequality must have this term for the prose to be true★: without it even a deadline of 8.9 s passes, and that value exceeds the client's silence limit if the above four spend just 0.1 s. When "2 s margin" was
    /// written only as a sentence it really was in that state.
    const CALLER_MARGIN: Duration = Duration::from_secs(2);

    pub fn new() -> Self {
        Self::with_clock(Arc::new(SystemClock), Self::DEFAULT_DEADLINE)
    }

    /// ★This is the only place the deadline is **injected**, so the inequality is bitten again here★ (ADR-0161 decision 3): an assertion measuring only the constant cannot see this argument — a test harness really ran the whole suite once with a value violating that relation.
    /// ★That it is `pub(crate)` is part of this★ — opening this door outside the crate would let the deadline be set outside the places that know the inequality.
    pub(crate) fn with_clock(clock: Arc<dyn Clock>, deadline: Duration) -> Self {
        assert!(
            fits_caller_silence_window(deadline),
            "the injected command deadline exceeds the caller's silence limit — the caller cuts off before the reply goes out so TIMEOUT becomes unreachable (CommandDeliveries::DEFAULT_DEADLINE doc)"
        );
        Self {
            inner: Arc::new(Mutex::new(Seats::default())),
            clock,
            deadline,
            max_local: MAX_LOCAL_IN_FLIGHT,
            max_resolving: MAX_RESOLVING_IN_FLIGHT,
        }
    }

    /// Sets the stage 1 concurrency cap lower — **only for tests measuring the cap branch**.
    /// To run that branch with the production value 64 parked bodies would have to be launched, and then what the test measures is the thread pool, not the decision.
    #[cfg(test)]
    pub(crate) fn with_local_limit(mut self, max_local: usize) -> Self {
        self.max_local = max_local;
        self
    }

    /// Sets the agent-reference field resolution cap lower — **test only** for the same reason as the sibling [`CommandDeliveries::with_local_limit`].
    #[cfg(test)]
    pub(crate) fn with_resolve_limit(mut self, max_resolving: usize) -> Self {
        self.max_resolving = max_resolving;
        self
    }

    /// The number of resolutions running now — a test measures "the marker is released only after the read ends".
    #[cfg(test)]
    pub(crate) fn resolving(&self) -> usize {
        self.lock().resolving
    }

    /// Launches the expiry sweeper task — on the daemon shutdown signal it **answers every remaining seat** and stops.
    /// ★Without the sweeper "unknown" becomes "an answer that never comes"★ (TRD §4-④). Even if this one line is missing every normal round trip still runs so **the symptom shows only as one unanswered request** — an easy place to miss in assembly.
    /// ★Calling [`CommandDeliveries::drain`] on the way out is what makes shutdown bounded★: a delivery task waits for its reply seat and **the sender side of that seat is inside this table**. Ending without emptying leaves those
    /// tasks with no occasion to wake until the deadline ([`CommandDeliveries::DEFAULT_DEADLINE`]), and meanwhile the table and tasks outlive the moment they are reported "ended". Emptying releases all of them **at once**.
    /// ★Do not discard the returned handle★ — the assembly must wait on it for "the sweeper stopped" to be observed.
    ///
    /// ★A table has **only one sweeper, and what carries that is the table's marker, not visibility**★
    /// That this function is not `pub` was only a necessary condition: the table is shared via [`CommandBus::deliveries`], `CommandDeliveries` is `Clone` and [`CommandBus::new`] is `pub`, so one line
    /// outside the module, `CommandBus::new(.., bus.deliveries().clone(), ..)`, could attach a second sweeper **to the live daemon's shared table**. When that marker falls, [`CommandDeliveries::drain`] on the way out closes
    /// that table and every later round trip is refused as [`Seat::Closed`].
    /// ★So the table remembers its sweeper ([`Seats::swept`]) and cuts a second request **by panic**★ — it is an assembly accident so blowing up on the spot fits the value better than quietly succeeding. The precedent is the duplicate-declaration
    /// panic of `CommandTable::new`, and ★the point is that that precedent is not an exception to **no panic in request-path handlers** (TRD §4-⑨) but is **outside that axis**★: that clause binds handlers processing requests others send, and this is an assembly-time invariant so no request exists
    /// then (only the code assembling the daemon can violate it, and that mistake cannot be reached by traffic). In a normal assembly the only caller of this place is [`CommandBus::new`].
    /// ★The assertion is **outside** the lock — otherwise it creates the very state it guards★: unwinding drops the guard and poisons the lock of [`Seats`], and then every later command of the live daemon dies at
    /// [`CommandDeliveries::lock`] (only cleanup paths pass quietly — the same "shared table is broken" state the marker was meant to prevent). Reading and setting finish at once in the critical section so measuring from outside keeps atomicity.
    /// Regression net = `tests::a_refused_second_sweeper_leaves_the_shared_table_usable`.
    #[must_use = "the assembly must wait on this handle for the shutdown to be bounded"]
    fn spawn_sweeper(&self, mut shutdown: watch::Receiver<bool>) -> tokio::task::JoinHandle<()> {
        let already_swept = {
            let mut table = self.lock();
            std::mem::replace(&mut table.swept, true)
        };
        assert!(
            !already_swept,
            "this seat table already has a sweeper attached — a second sweeper closes the shared table on the way out and refuses every later command as \"shutting down\" (check whether another CommandDeliveries was passed to CommandBus::new)"
        );
        let deliveries = self.clone();
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(Self::SWEEP_INTERVAL);
            // Delayed ticks are not burst — the sweep sees only "what has expired now" so there is no debt to catch up.
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tokio::select! {
                    _ = ticker.tick() => {
                        let expired = deliveries.expire();
                        if expired > 0 {
                            tracing::debug!(expired, "reaped command round trips past the deadline and answered TIMEOUT");
                        }
                    }
                    changed = shutdown.changed() => {
                        // The sender side vanishing is also shutdown (Err) — no signal can come after that.
                        if changed.is_err() || *shutdown.borrow() {
                            break;
                        }
                    }
                }
            }
            let drained = deliveries.drain();
            if drained > 0 {
                tracing::info!(
                    drained,
                    "daemon shutdown — answered and reaped every command round trip that was in flight"
                );
            }
        })
    }

    /// Reaps and answers **every** remaining seat (the shutdown path). Return = the number reaped.
    /// ★Why `OUTCOME_UNKNOWN`★: the envelope may already have gone to the owner so whether it was applied is **unknown**. That is the **certainty** axis of "safety × usefulness" (ADR-0159). Folding to `INTERNAL` would make the caller read an
    /// operation that may have been applied as a **certain failure** — not the fact. (The instruction is `Never` regardless of the code — [`no_retry`].)
    /// ★Unlike the deadline it does not look at the clock★ — shutdown is an event, not a time.
    /// ★It does not stop at emptying but **closes the table**★: emptying alone lets a `Command` that arrives afterward open a seat and wait for an answer, but by then the sweeper has stopped and there are no eyes anywhere to watch the deadline — that round trip would get zero replies
    /// until the process dies. This window is not theory: even after the accept loop is cut the connection tasks stay alive (detached spawns with no shutdown receiver) and the daemon spends several more seconds on session cleanup · flush.
    /// A query after closing is **refused at once** as [`Seat::Closed`] so "shutdown is bounded" holds in that window too. ★After closing, the seats are erased wholesale★ — the reason to prevent reuse (a new round trip of the same key) has already vanished with the close.
    ///
    /// ★Not `pub`★ — it is a verb that closes the table so the callable place must be the one shutdown path (full reason = [`CommandDeliveries::spawn_sweeper`]). The only caller is that sweeper.
    fn drain(&self) -> usize {
        // Answer outside the lock — the basis is the same as [`CommandDeliveries::expire`].
        let outstanding: Vec<(RequestId, Pending)> = {
            let mut table = self.lock();
            table.closed = true;
            table.open.drain().collect()
        };
        let mut count = 0;
        for (request_id, pending) in outstanding {
            // A seat that already has an answer (`Fulfilled`·`Void`) is left out of the count — its frame is already going out.
            if let SeatState::Awaiting(waiter) = pending.state {
                count += 1;
                let _ = waiter.send(CommandReply::err(
                    request_id,
                    no_retry(
                        ErrorCode::OutcomeUnknown,
                        "the daemon shut down while this command was in flight",
                    ),
                ));
            }
        }
        count
    }

    /// Opens one round trip — ★[`deliver`] calls it **first of all**★. The reply comes to the seat returned.
    /// ★Open before doing anything★ — opening after writing the envelope lets a quick owner's outcome arrive before the seat and get nothing, and if another stage of `route` makes the answer that answer goes out without going through the table at all (module header rule 1).
    /// ★Do not overwrite an already taken key★: overwriting quietly erases the earlier round trip's seat and that caller hangs until the deadline. The meaning of each branch is [`Seat`].
    /// `local` = **does the daemon answer this name by itself** ([`LocalCommands::owns`]). The reason it must be decided when the seat is born is [`Pending::local`].
    fn open(
        &self,
        request_id: RequestId,
        origin: ConnId,
        name: &str,
        args: &serde_json::Value,
        local: bool,
    ) -> Seat {
        // ★Settle expired seats **here first**★ (before locking — `expire` locks by itself).
        // The sweeper runs every second but the retry comes right after the deadline. If a re-query arriving in between were coalesced into "a corpse not yet reaped", **that attempt would never be delivered** and the caller would get the
        // `TIMEOUT` of the earlier attempt, settled before it asked again — a retry becomes a quiet no-op. Settling first makes that seat a `Void` with no outcome and it is overturned below so this attempt gets its own round trip.
        // ★The price is that the **order of two frames flips**★: the old round trip woken here has only been scheduled to write its `TIMEOUT`, so the actual write happens **after** this retry got its seat and even sent the envelope —
        // **the old answer can reach the same connection later than the new answer**. Both frames are normal (one per attempt) and the correlation key is the same, but a caller assuming "the last to arrive is the last answer" has that assumption broken here.
        self.expire();

        let (waiter, rx) = oneshot::channel();
        let now = self.clock.now();
        let deadline = now + self.deadline;
        let mut table = self.lock();
        // A closed table opens no seat — opening one has no eyes to wake it ([`CommandDeliveries::drain`]).
        if table.closed {
            return Seat::Closed;
        }

        // ★The decision must be **inside this critical section**★ — if changed to look at "is it already there" first outside and then enter, two delivery tasks of the same connection issued back to back would both see an empty table and each open a seat.
        enum Occupancy {
            Free,
            /// A corpse folded with no outcome — it must not absorb the next attempt so it is overturned.
            Stale,
            Held(Seat),
        }
        let occupancy = match table.open.get(&request_id) {
            None => Occupancy::Free,
            // ★This number has **already been applied**★ — before the hold window passes no branch can open a new execution (basis = [`SeatState::Retained`]). It comes before the payload comparison: even if different arguments arrive with the same number
            // what that number has already done is the dominant fact.
            Some(held) if matches!(held.state, SeatState::Retained { .. }) => {
                Occupancy::Held(Seat::LocalAlreadyRan)
            }
            // ★A seat whose body is running **cannot be overturned by anyone**★ — even if the deadline passed and the reply already went out, that work cannot be cancelled (blocking pool) and still runs. Overturning here would apply
            // the same Write verb twice (basis · range = [`Seat::LocalStillRunning`] · [`Pending::local`]).
            Some(held) if held.local && matches!(held.state, SeatState::Void) => {
                Occupancy::Held(Seat::LocalStillRunning)
            }
            Some(held) if matches!(held.state, SeatState::Void) => Occupancy::Stale,
            // ★Look at the payload **first**★: even with the same id, a different request cannot be coalesced whoever asked (TRD §4-⑥ third leg). The reason to look at the arguments and not only the name is that different operations under the same name are common
            // (`tab.create` twice with the window changed — coalesced, it opens only once).
            Some(held) if held.name != name || &held.args != args => {
                Occupancy::Held(Seat::Conflict)
            }
            Some(held) if held.origin == origin => Occupancy::Held(Seat::Coalesced),
            Some(held) => Occupancy::Held(Seat::Taken {
                holder: held.origin,
            }),
        };
        match occupancy {
            Occupancy::Held(seat) => return seat,
            Occupancy::Stale => {
                // The corpse's ticket becomes invalid here — a late `release` compares against the table and returns empty-handed.
                //
                // ★Unlike its siblings it is dropped **inside** the lock — there is one premise that makes that allowed★: `Void` is attached by [`CommandDeliveries::expire`] only **after taking out the sender**, so the value
                // dropped here has no one to wake. The reason `expire` · `drop_origin` · `drain` drop outside the lock (the destructor wakes a waiting task) does not apply to this line.
                // ★If code appears that attaches `Void` while leaving the sender, this line immediately becomes a violation★ — there is no test then to catch it, so an edit changing that premise must move this line too.
                table.open.remove(&request_id);
            }
            Occupancy::Free => {}
        }

        let token = table.next_token;
        table.next_token += 1;
        table.open.insert(
            request_id,
            Pending {
                origin,
                deadline,
                token,
                name: name.to_string(),
                args: args.clone(),
                state: SeatState::Awaiting(waiter),
                local,
            },
        );
        Seat::Opened { rx, token }
    }

    /// How long [`SeatState::Retained`] holds a number — **one deadline window**.
    /// The window to close is "until a caller that received `TIMEOUT` at the deadline asks again with the same number", and the natural length of that window is the deadline. There is no reason to take it longer — a re-query after that belongs to a caller that
    /// broke the convention (this path instructs no retry at all — [`no_retry`]) and seat residency grows by that much.
    fn retention(&self) -> Duration {
        self.deadline
    }

    /// May a stage 1 body start — within the cap it raises the count and issues **a marker keeper that undoes it**.
    /// ★Why the cap is looked at here★: the seat is already open (that is how the reply sits on one seat) and what must be blocked is the **execution**. On overflow nothing is run and it is refused so certainty is "certainly failed" — the code is `CONFLICT`
    /// not because the packet is wrong but because this daemon's **state** is so (the same vocabulary as the roster's cap refusal).
    /// ★The seat's `local` marker is not turned on here★ — it is decided when the seat is **born** ([`Pending::local`]). What this function counts is a **resource** (running bodies) and what the marker says is **identity** (who answers).
    /// ★The `Err` returned is logged before it comes★ — hitting the global cap is an operational event and without a log the only witness is **the refused caller**
    /// (logging convention "no silent swallow"). After one hit it stays quiet until the count drops below the cap — logging per refusal would let the other side decide the log size.
    fn reserve_local_slot(
        &self,
        name: &str,
        entrance: &'static str,
    ) -> Result<LocalGuard, CommandError> {
        let now = self.clock.now();
        let mut table = self.lock();
        if table.local_running >= self.max_local {
            let running = table.local_running;
            let due = table
                .ceiling_warned_at
                .is_none_or(|at| now.duration_since(at) >= CEILING_WARN_INTERVAL);
            if due {
                table.ceiling_warned_at = Some(now);
                tracing::warn!(
                    entrance,
                    command = name,
                    running,
                    ceiling = self.max_local,
                    "the daemon's own commands hit the concurrent-execution cap — refusing all of them until it drops below the cap (it does not clear while profile saving is blocked)"
                );
            }
            return Err(no_retry(
                ErrorCode::Conflict,
                format!(
                    "this daemon is already running {running} of its own commands and will not queue more, so nothing was run — send this again with a new request_id once they finish; if it never clears, this daemon's profile store is stuck and it has to be restarted"
                ),
            ));
        }
        table.local_running += 1;
        Ok(LocalGuard {
            deliveries: self.clone(),
            settled: false,
        })
    }

    /// A stage 1 body ended — undoes the count. Called only by the destructor of [`LocalGuard`].
    /// ★No panic on a lock failure here★: this function is on the **destructor path** so a panic during unwinding aborts the process at once — that is not a value this one subtraction can bear. Moreover, if it cannot subtract the cap loses one
    /// permit at a time permanently and eventually the whole family is blocked, so even a poisoned lock has its **contents taken out and the subtraction is always done** (whatever state the table is in at that moment, better than the count freezing larger than reality).
    fn release_local_slot(&self) {
        let mut table = self.lock_for_cleanup();
        // The keeper calls exactly once so this subtraction is once too — it is still saturating because a count wrapping negative and leaving the cap open forever is worse than leaking one permit.
        table.local_running = table.local_running.saturating_sub(1);
    }

    /// May an agent-reference field resolution (roster read) start — within the cap it raises the count and issues **a marker that undoes it** (ADR-0308).
    /// On overflow nothing is read and it is refused so certainty is "certainly failed" (the envelope never went to the owner), and the code `CONFLICT` has the same meaning as the sibling [`CommandDeliveries::reserve_local_slot`]
    /// (this daemon's **state**). The logging discipline is the same.
    fn reserve_resolve_slot(
        &self,
        name: &str,
        entrance: &'static str,
    ) -> Result<ResolveGuard, CommandError> {
        let now = self.clock.now();
        let mut table = self.lock();
        if table.resolving >= self.max_resolving {
            let running = table.resolving;
            let due = table
                .resolve_warned_at
                .is_none_or(|at| now.duration_since(at) >= CEILING_WARN_INTERVAL);
            if due {
                table.resolve_warned_at = Some(now);
                tracing::warn!(
                    entrance,
                    command = name,
                    running,
                    ceiling = self.max_resolving,
                    "agent-reference resolution hit the concurrent cap — refusing every marked command until roster reads finish and the count drops below it (it clears late if a dormant agent's working folder does not respond)"
                );
            }
            return Err(no_retry(
                ErrorCode::Conflict,
                format!(
                    "this daemon is already reading its agent roster for {running} commands and will not queue more, so this command was not sent to its owner and nothing was run — send it again once they finish; if it keeps happening, an agent's working folder is not answering (for example a disconnected network share)"
                ),
            ));
        }
        table.resolving += 1;
        Ok(ResolveGuard {
            deliveries: self.clone(),
        })
    }

    /// A roster read ended — undoes the count. Called only by the destructor of [`ResolveGuard`] (no panic since it is a destructor path —
    /// the basis is the sibling [`CommandDeliveries::release_local_slot`]).
    fn release_resolve_slot(&self) {
        let mut table = self.lock_for_cleanup();
        table.resolving = table.resolving.saturating_sub(1);
    }

    /// Hands a stage 1 seat whose application ended over **into a held state** — it does not release the seat and keeps holding that number.
    /// ★This and the sibling that releases a seat ([`CommandDeliveries::release`]) are **called split by outcome**★: if even a round trip where nothing was applied (caller left · cap refusal · rejection) were held, a caller who fixed a typo and resent with the same number
    /// would get back "already applied" for a **command that never ran**. The decision is [`retains_the_id`].
    /// ★Picks by ticket★ — if this call is late and a different round trip sits on the same key it must not be touched.
    fn settle_local(&self, request_id: RequestId, token: SeatToken, retain: bool) {
        let until = self.clock.now() + self.retention();
        let mut table = self.lock();
        // The count return is **inside the same lock** so there is no window where a permit looks free in between.
        table.local_running = table.local_running.saturating_sub(1);
        let Some(pending) = table.open.get_mut(&request_id) else {
            return;
        };
        // Picked by ticket — if this call is late and a different round trip sits on the same key it must not be touched.
        if pending.token != token {
            return;
        }
        if !retain {
            table.open.remove(&request_id);
            return;
        }
        pending.state = SeatState::Retained { until };
        // ★A held seat answers **by number only — there is no reason to hold the payload**★: the standing decision comes before the arguments comparison ([`Seat::LocalAlreadyRan`]) so these two are never read again, but left as is the JSON the caller gave
        // would stay resident for the whole window. Erasing it fixes the size of one held entry.
        pending.name.clear();
        pending.args = serde_json::Value::Null;
        table.evict_oldest_retained();
    }

    /// That round trip ended — ★releases the seat **after the frame goes out**★ (module header rule 2).
    /// ★Picks by ticket, not by key★: if this call is late a **different round trip** may sit on the same key (a normal re-query after deadline · shutdown reaped), and erasing by key makes that round trip lose its answer for no reason.
    fn release(&self, request_id: RequestId, token: SeatToken) {
        let mut table = self.lock();
        if table
            .open
            .get(&request_id)
            .is_some_and(|p| p.token == token)
        {
            table.open.remove(&request_id);
        }
    }

    /// Attaches the outcome the owner returned to that round trip. **False = there was no place to attach** (it came late, or nobody asked).
    /// ★The seat is **not erased** — the outcome attaches and it is only left as `Fulfilled`★: erasing would, in the instant before the frame goes out, let the same caller's re-query see an empty table and **send the envelope once more** (the same operation is applied twice and the frames of the same key
    /// become two). The seat is released by [`CommandDeliveries::release`] after the frame has gone out. A re-query that comes in between is folded as [`Seat::Coalesced`] and **shares that one frame going out**.
    /// ★It does not look at which connection sent it★: the door this outcome arrives at is **one socket only** (the production caller = the dispatch of `connection_core`) and that door is already inside the auth boundary by the daemon token, so knowing the correlation key
    /// string is enough to trust it (ADR-0153). Checking the sending connection too would need the table to hold the owner connection as well, and then a round trip where the owner reconnected could not attach its own answer.
    /// ★Do not read it as "safe because it is an unguessable random number" — that leg is gone now★: the key can be carried by the caller (ADR-0161 decision 1) and the daemon requires only **UUID syntax** (`parse_str` in `control::catalog`)
    /// so nil or a non-v4 value passes too. That is, a predictable key can sit in this table. What makes that not a problem today is **the single fact that the door reaching this function is not the control route** — that route's caller has no means to send an outcome.
    /// If even one such door is added, look at this place again.
    /// ★But **a round trip whose envelope went to no one** is the exception★ — that seat has no reason to receive an outcome so it is refused ([`OutcomeLanding::NotDelegated`] · basis = [`Pending::local`]).
    pub fn complete(&self, reply: CommandReply) -> OutcomeLanding {
        // Answer outside the lock (ADR-0006) — only the ticket is drawn inside.
        let waiter = {
            let mut table = self.lock();
            match table.open.get_mut(&reply.request_id) {
                // ★This seat is answered by the daemon itself — an outcome another sent is **by definition not a legitimate reply**★: since the envelope went to no one there is no owner to answer. Accepting it would let that payload push out the handler's real
                // result and go out as the reply (the caller reads a value another made up as its own command's result), and the real result would be quietly discarded.
                Some(pending) if pending.local => return OutcomeLanding::NotDelegated,
                Some(pending) => {
                    match std::mem::replace(&mut pending.state, SeatState::Fulfilled) {
                        SeatState::Awaiting(waiter) => Some(waiter),
                        // A seat that already has an answer — put the standing back and go out empty-handed.
                        settled => {
                            pending.state = settled;
                            None
                        }
                    }
                }
                None => None,
            }
        };
        let Some(waiter) = waiter else {
            return OutcomeLanding::NoSeat;
        };
        // If the receiving side has already vanished (the delivery task ended) there is nowhere to send — that too is the result of keeping to one reply.
        let _ = waiter.send(reply);
        OutcomeLanding::Attached
    }

    /// Reaps every round trip past the deadline and answers `TIMEOUT`. Return = the number reaped.
    /// ★`TIMEOUT` is the certainty "unknown"★ — the owner may answer late so whether it was applied is not known (TRD §4-④·§4-⑥). That does not lead to "ask again with the same number" — for that instruction to be safe there would need to be a dedup store replaying completed work, and there is none ([`no_retry`]).
    /// ★The seat is not erased but folded into `Void`★ — erasing is only [`CommandDeliveries::release`] (the basis is the same as [`CommandDeliveries::complete`]: if the key empties before the frame goes out a re-query sends the envelope once more). The basis for a folded seat not swallowing the next attempt is [`SeatState::Void`].
    /// ★**A seat past the hold window is released here**★ — that seat is not waiting for an answer but holding only the number (`SeatState::Retained`) so it is erased with no reply and **not counted in the number reaped** (that number is the count of "round trips whose answer could not come").
    /// The same 1-second sweeper does both jobs because the period and the material for the decision (the clock) are the same.
    pub fn expire(&self) -> usize {
        let now = self.clock.now();
        // ★Do not answer inside the lock★ (ADR-0006): wake after taking the tickets out and releasing — the woken task may lock this table again right away, and one delivery waiting for a reply must not stall registration · cleanup · other deliveries.
        let expired: Vec<(RequestId, oneshot::Sender<CommandReply>)> = {
            let mut table = self.lock();
            table
                .open
                .retain(|_, pending| !matches!(pending.state, SeatState::Retained { until } if until <= now));
            table
                .open
                .iter_mut()
                // A held seat is not waiting for an answer — we looked at the window above so there is no reason to touch it again here, and touching it would be a pointless pulling out and putting back of the standing every tick.
                .filter(|(_, pending)| {
                    pending.deadline <= now && !matches!(pending.state, SeatState::Retained { .. })
                })
                .filter_map(|(id, pending)| {
                    match std::mem::replace(&mut pending.state, SeatState::Void) {
                        SeatState::Awaiting(waiter) => Some((*id, waiter)),
                        // A seat already folded or with an outcome attached is not counted twice.
                        settled => {
                            pending.state = settled;
                            None
                        }
                    }
                })
                .collect()
        };
        let count = expired.len();
        for (request_id, waiter) in expired {
            let _ = waiter.send(CommandReply::err(
                request_id,
                no_retry(
                    ErrorCode::Timeout,
                    "the owner did not answer before the deadline",
                ),
            ));
        }
        count
    }

    /// Reaps the round trips that connection **issued** — the place to take the reply is gone. Return = the number reaped.
    /// ★But a seat whose stage 1 body is running is left★ — the reason is the filter comment below ([`Pending::local`]).
    /// ★It makes no reply and only releases the seat★: there is no connection to receive so whatever outcome it invented has nowhere to go. Releasing the seat frees the waiting delivery as "the one to wait for vanished" and the task ends.
    /// ★**Unimplemented — diverges from TRD §4-④**★ (out of this round's scope · escalated)
    /// ① **The owner side disconnecting is not caught here.** This table remembers only the origin connection (the entry shape ADR-0154 fixed), so if the owner dies mid round trip that round trip waits to the deadline and ends in `TIMEOUT` — TRD §4-④ wrote `OUTCOME_UNKNOWN` for that situation.
    /// That the code differs is itself non-compliance with the contract. What makes it tolerable today is that what the caller **does** does not split (both are certainty "unknown" with the same instruction), not that it is right.
    /// ② **TRD §4-④'s "on cleanup of either connection sweep by that ConnId key" is also missing** — this function does only the origin-connection half. Doing the other half needs an owner-connection index and the entry shape ADR-0154 pinned excludes that field. So this is not
    /// a hole to fill with code but **one to resolve by decision**.
    pub fn drop_origin(&self, conn_id: ConnId) -> usize {
        // Drop the reaped items **outside the lock** — the destructor wakes a waiting task (the same reason as `expire` above).
        let dropped: Vec<Pending> = {
            // ★This function is called from a destructor too ([`CallerSeat`])★ — so it does not panic on a lock failure.
            let mut table = self.lock_for_cleanup();
            let mine: Vec<RequestId> = table
                .open
                .iter()
                // ★Stage 1 seats are **not reaped here whether running or held**★: holding is tied to `request_id`, not to the connection. A disconnect is rather the event where that holding is most needed —
                //   the caller is cut off not knowing whether the command arrived, reconnects and **asks again with the same number** (the sibling [`Seat::Taken`] branch already wrote that scenario). With no seat then the same Write
                //   verb would be applied twice and there is no verb to delete it (ADR-0122). The cost is only that a seat lives **at most one more window**, and that residency is inside the two caps of window and count
                //   ([`MAX_RETAINED_LOCAL`]). The sweeper releases the seat when the window passes.
                .filter(|(_, pending)| pending.origin == conn_id && !pending.local)
                .map(|(id, _)| *id)
                .collect();
            mine.into_iter()
                .filter_map(|id| table.open.remove(&id))
                .collect()
        };
        dropped.len()
    }

    /// The number of seats now sitting in the table — an observation surface for whether the table leaks.
    ///
    /// A seat that has an answer attached but whose frame has not yet gone out is counted too — that seat still holds the key.
    pub fn in_flight(&self) -> usize {
        self.lock().open.len()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Seats> {
        self.inner.lock().expect("command deliveries poisoned")
    }

    /// An acquire that does not panic on a lock failure — ★**for the destructor path only**★.
    /// The sibling [`CommandDeliveries::lock`] panics to stop use of a poisoned table, but panicking inside a destructor (an immediate abort during unwinding) costs more than this one cleanup can bear. And skipping the
    /// cleanup is itself a loss for this table — a seat not released is never released by anyone.
    /// ★This discipline must hold **per table**★: the destructor of [`CallerSeat`] reaps the roster first, so if that line used a panicking lock this function would never be called. That is why the roster side has the same pair
    /// (`lock_for_cleanup` inside [`CommandRoster::detach`]) — with only one of the two, "reaps both tables together" becomes false.
    /// ★The production process cannot reach this branch★ — a panic holding this table is outside the containment marker so the hook ends the process before unwinding and no poisoning arises (ADR-0292 — so what this guards is test builds without the hook).
    /// Do not lift it on that basis: in that build a leaking table ruins the observations of the regression tests as is.
    fn lock_for_cleanup(&self) -> std::sync::MutexGuard<'_, Seats> {
        sync::lock(&self.inner)
    }
}

impl Default for CommandDeliveries {
    fn default() -> Self {
        Self::new()
    }
}

/// The transport stage 2 of the 3-stage delivery uses — moves an owner token to the frame outlet of the connection that owner is attached to.
/// ★The future does not borrow this struct★: the roster lookup and envelope write finish **synchronously** inside `send`, and only one reply seat goes outside. That is how a handed-out outlet copy does not outlive the round trip (module header).
struct OwnerLink<'a> {
    roster: &'a CommandRoster,
    origin: ConnId,
    /// The receiving end of the seat **opened beforehand** by [`deliver`] — `send` takes it out once.
    /// ★The seat is not opened here★: if opening and closing were in here each failure branch would need an undo, and that undo picks the seat by key and erases **another's round trip** (a seat can change between preemptions even without `.await`).
    /// Lifting the seat's whole lifetime into [`deliver`] removes that undo itself.
    waiter: Mutex<Option<oneshot::Receiver<CommandReply>>>,
}

/// A seat that emits an already-decided answer as is — fits the return shape `route` requires.
fn settled(reply: CommandReply) -> Pin<Box<dyn Future<Output = CommandReply> + Send>> {
    Box::pin(std::future::ready(reply))
}

impl CommandLink for OwnerLink<'_> {
    fn send(&self, env: CommandEnvelope) -> Pin<Box<dyn Future<Output = CommandReply> + Send>> {
        let request_id = env.request_id;
        // The name to put in the wording of the error branch — the envelope is moved whole below.
        // ★No length cap on the caller string sent back★ (ADR-0152) — the reader is an LLM and cutting the tail removes its next action. Only the **log** side truncates.
        let name = env.name.clone();

        let taken = self
            .waiter
            .lock()
            .expect("owner link waiter poisoned")
            .take();
        let Some(rx) = taken else {
            // `route` is called only once per hop — a second call is a defect of this module and waiting does not change it.
            return settled(CommandReply::err(
                request_id,
                no_retry(
                    ErrorCode::Internal,
                    format!("this daemon tried to hand '{name}' over twice for one round trip"),
                ),
            ));
        };

        // ★Just **before** handing over the envelope check whether the asking connection still exists — if not, do not send★
        // This task was detached by `dispatch` so **its first poll can be later than the cleanup**: in that order `drop_origin` sees an empty table and passes, and if we wake late and send the envelope
        // a command with side effects runs for a caller that has already left.
        //
        // ★Written split into what this check closes and what it does not★ — it is not "closes completely":
        // ① **Closes — orphan seats.** `on_disconnect` removes from the roster first (`detach`) and then reaps the seat (`drop_origin`), so if cleanup came before us this lookup is certainly empty-handed, and if we came first
        //    our seat is in the table and cleanup reaps it. **Reversing the order of those two lines disables this check.**
        // ② **Does not close — side effects.** This lookup is **a sample of that instant** and the envelope goes out below. A connection that drops in between is not seen here
        //    (the inherent residue of check-then-act that cannot be removed in this place — removing it would need carrying the roster lock to the envelope write, and that is the lock `route` deliberately released).
        //    ★So do not read this check as "a command of a caller who has now left is never executed"★.
        //
        // ★The reply of this branch goes **nowhere**★ — the lookup was just empty-handed and connection ids are not reused ([`Pending::origin`]) so that connection's outlet has no way to come back. So the code choice
        // cannot give the caller any instruction (unobservable) — `CONFLICT` is a test marker to catch "a reply that must not go out leaked" in one wording ([`CALLER_ALREADY_GONE`]).
        if self.roster.sink_for_conn(self.origin).is_none() {
            return settled(CommandReply::err(
                request_id,
                no_retry(ErrorCode::Conflict, CALLER_ALREADY_GONE),
            ));
        }

        let outcome = match self.roster.sink_of(&env.owner) {
            // ★Torn window★ — the roster lookup answered `Available` and in between the owner disconnected. `route` **deliberately** releases the roster lock between lookup and
            //   delivery so this race is a designed residue. The answer is `OUTCOME_UNKNOWN`: the owner may reattach so a re-query of the same id is **both safe and useful**
            //   (the ruler of ADR-0159). ★Do not fold into `UNKNOWN_COMMAND`·`OWNER_UNAVAILABLE` — ADR-0154 pinned that and no new code is made either★.
            None => Err(no_retry(
                ErrorCode::OutcomeUnknown,
                format!("the owner of '{name}' went away while the envelope was being handed over"),
            )),
            Some(sink) => {
                // ADR-0272: re-wrap into the raw field — the arguments the relay fixed (ADR-0308) are carried in these bytes.
                let packet = envelope_to_wire(&env)
                    .ok()
                    .and_then(|envelope| event_json(&AgentEvent::CommandRequest { envelope }));
                let written = match packet {
                    // Queue saturated · closed — the envelope could not go out but the queue may drain so a re-query of the same id is useful.
                    Some(text) => sink.try_send(Frame::Text(text)).map_err(|_| {
                        no_retry(
                            ErrorCode::OutcomeUnknown,
                            format!("the owner of '{name}' could not take the envelope right now"),
                        )
                    }),
                    // A serialization failure is a defect on our side and waiting does not change it — the usefulness term is false (ADR-0159).
                    None => Err(no_retry(
                        ErrorCode::Internal,
                        format!("this daemon could not encode the envelope for '{name}'"),
                    )),
                };
                // The copy drops here — taken, written on the spot and discarded (ADR-0154).
                drop(sink);
                written
            }
        };

        if let Err(error) = outcome {
            // ★Do not undo the seat here★ — the seat belongs to [`deliver`], which after sending this reply compares the ticket and releases it. Picking by key here would erase **another round trip** that sat on the same key in between.
            return settled(CommandReply::err(request_id, error));
        }

        settled_later(rx, request_id)
    }
}

/// Waits for the reply seat. If the seat **vanishes with no answer** (the origin connection disappeared) that too is folded into one outcome.
fn settled_later(
    rx: oneshot::Receiver<CommandReply>,
    request_id: RequestId,
) -> Pin<Box<dyn Future<Output = CommandReply> + Send>> {
    Box::pin(async move {
        rx.await.unwrap_or_else(|_| {
            CommandReply::err(
                request_id,
                no_retry(
                    ErrorCode::OutcomeUnknown,
                    "the caller's connection went away before the owner answered",
                ),
            )
        })
    })
}

/// **Stage 1** of the 3-stage delivery — commands this daemon answers by itself (`agent.*`).
/// ★Why injected (ADR-0012)★: the real thing is a table holding `AgentManager` so real processes · disk come with it. The harness measuring the delivery rules cannot drag that in so it is cut by a port — so this file's tests measure the stage 1 branch without a table
/// (or with a fake table).
/// ★The implementation must read the table **at call time**★: the table exists only after the manager is assembled but this port is loaded into the connection factory before that (the daemon `lib.rs` assembly order). Catching the assembly-time value would freeze it
/// forever as "no commands of mine", and the symptom is `agent.*` coming back as an **unknown command** with neither error nor log.
// ADR-0155
pub trait LocalCommands: Send + Sync {
    /// Do I hold this name — if so, also answers that command's **effect**.
    /// ★This answer is the name's **only routing authority**★: with `Some` this round trip is delegated to no one (the seat carries that fact — [`Pending::local`]) and always ends here. It used to be that [`LocalCommands::run`]
    /// could answer empty-handed and fall through to the next stage, but then the seat claimed "gave it to no one" while the envelope went to the owner, so ① the owner's legitimate outcome is refused ([`OutcomeLanding::NotDelegated`]) and
    /// ② disconnect cleanup does not clear that seat. Keeping one authority makes that mismatch **unrepresentable**.
    /// ★It only decides★ — no execution, and none of the locks · disk that execution takes. With `None` [`deliver`] does not stop by the blocking pool and goes straight to the next stage (carrying another's name is this daemon's constant path).
    /// ★Why the effect is received here★: only an applied Write holds the number ([`SeatState::Retained`]) and a read does not. Deciding that after the outcome would mean searching the declaration by name again, a value that may have
    /// changed by then — using the value born with the seat cannot diverge.
    fn claim(&self, name: &str) -> Option<Effect>;

    /// Checks and executes to produce the outcome.
    /// ★`None` is a **contract violation**★ — only names [`LocalCommands::claim`] answered as mine come here so it cannot be empty-handed. It is left in the return type because an implementation could make that state, and [`deliver`] does not let it flow to the
    /// next stage but **surfaces** it as `INTERNAL` (letting it flow would revive the mismatch in the doc above).
    /// ★This is blocking★ — [`deliver`] calls it on the blocking pool. Do not await async here.
    /// ★The entrance check (ADR-0157) must be **inside** this★: plugging in an implementation that calls the table without a check makes this path a second entrance running with no check — the failure where one typo field quietly runs as a different action.
    /// ★The `Ok` payload is **put in the log**★ (completion after the deadline — [`log_late_local`]). So do not put secrets such as credentials · tokens in a success payload leaving through this port. ★Today's real ones (`agent.*`·`usage.*`) are not
    /// only id · name · status★ — the payload of `agent.listQueuedInputs` carries text the user typed and the rows of `usage.*` carry the upstream raw text (`upstream`). The former does not sit in the log because that record carries a payload only for the **success of a held Write**
    /// (a read leaves only the event — [`log_late_local`]), and the latter is a held Write (`usage.refresh`) so it is carried, but before that record carries it **it walks every key named `upstream` all the way down regardless of the command** ([`strip_upstream`]).
    /// `caller` = the calling connection (material for the input lease decision — ADR-0231). `None` = an entrance with no connection (`/control/call`) and it is not counted as a lease holder.
    fn run(
        &self,
        name: &str,
        args: &mut serde_json::Value,
        entrance: &'static str,
        caller: Option<ConnId>,
    ) -> Option<Result<serde_json::Value, CommandError>>;

    /// Every name this daemon answers by itself — discovery (`ListCommands`) emits it **merged** with the roster.
    fn decls(&self) -> Vec<CommandDecl>;

    /// The roster that resolves the agent-reference fields of another's command (ADR-0308) — it must be **the very roster** the stage 1 verbs use to resolve targeting
    /// (one resolver — ADR-0132). If the two diverge the same name resolves to a different agent per entrance.
    /// ★This is blocking★ — path normalization of a dormant profile can take long (agent `AgentManager::roster`). [`deliver`] calls it on the blocking pool.
    /// ★No default body is provided★: an implementation returning an empty roster refuses every marked field as `NOT_FOUND` and blocks those commands wholesale. With a default, an assembly that has a roster could omit this method and still compile and the symptom would be only a runtime refusal —
    /// an assembly that holds no agents writes an empty roster **by itself**.
    // ADR-0308
    fn agent_roster(&self) -> Vec<AgentRosterRow>;
}

/// An assembly with no commands of its own — stage 1 is always a miss (smoke bins · harnesses measuring only the delivery rules).
pub struct NoLocalCommands;

impl LocalCommands for NoLocalCommands {
    fn claim(&self, _name: &str) -> Option<Effect> {
        None
    }

    fn run(
        &self,
        _name: &str,
        _args: &mut serde_json::Value,
        _entrance: &'static str,
        _caller: Option<ConnId>,
    ) -> Option<Result<serde_json::Value, CommandError>> {
        None
    }

    fn decls(&self) -> Vec<CommandDecl> {
        Vec::new()
    }

    // An assembly that holds no agents — every marked field is refused as `NOT_FOUND` and not relayed.
    fn agent_roster(&self) -> Vec<AgentRosterRow> {
        Vec::new()
    }
}

/// The number of stage 1 bodies that can be out on the blocking pool at the same time.
/// ★Without a cap both seats and the pool queue grow without bound★: if profile-save I/O stops the body does not end, and the deadline **cannot reap** that seat (it holds the seat while the body runs — [`Pending::local`]). Then for every command
/// arriving under a different number one seat + one pool job piles up and the two swell together until the I/O returns.
/// ★The basis of the value — **burst absorption**, not throughput★: every write verb of this family queues on one profile lock (the lock discipline of agent `ProfileRegistry`) so raising concurrency does not raise throughput. So this number is not "how fast to handle at once"
/// but "how many to hold before refusing loudly", and on overflow it refuses instead of quietly growing the queue (the same argument as the roster capping its own residency — `Roster::MAX_NAMES`). It also leaves ample room in tokio's default blocking pool (512 threads).
/// ★This entrance is inside the auth boundary (ADR-0153)★ — it is not a surface anyone can push into so the value need not be tight.
///
/// ## ★Two things this cap **cannot do** — left knowingly★
///
/// ① **It may not come back (a one-way ratchet).** If the situation it blocks (profile save blocked and bodies not ending) really happens those 64 **never release**: the deadline cannot reap the seat ([`Pending::local`]), disconnect cleanup skips it too
///    ([`CommandDeliveries::drop_origin`]), and the body itself has no time limit — `spawn_blocking` cannot be cancelled so there is no means at this layer to impose one. After that the bus's `agent.*` is `CONFLICT` for
///    the whole process lifetime. So the refusal wording does not end with "wait and retry" but goes as far as **the daemon must be restarted** (waiting in that state is a false instruction). To truly close it the limit must be placed where one can be imposed,
///    that is on **the profile save I/O itself** — not this layer's decision.
/// ② **It does not apply to the control route (`/control/agent`).** That too drives the same handlers by `spawn_blocking` (`mcp_server::control_agent_handler`) but does not look at this count. Moving it into the place both entrances share
///    (`control::commands::call_daemon_command`) would make the decision **inside the pool job**, refusing after already taking the resource it meant to guard — admission control must come **before** taking the resource so that seam cannot be this policy's home.
///    Sharing it properly means passing an admission-control object made at assembly time to both `start_mcp_server_with_help` and `run_accept_loop`, and that is out of this scope. ★So the pool load of the HTTP surface is still
///    unbounded★ — do not read this constant as blocking that too.
const MAX_LOCAL_IN_FLIGHT: usize = 64;

/// The number of agent-reference field resolutions (roster reads) that can run on the blocking pool at the same time (ADR-0308).
/// ★Without a cap blocked reads pile up without bound★: a read is `spawn_blocking` so it is not cancelled — it runs to the end even if that round trip's seat is settled first (deadline · shutdown · caller left) or a control route call is cancelled and the waiting side leaves.
/// If a dormant agent's cwd is a dead network share one read blocks for tens of seconds (agent `AgentManager::roster`), so for every marked command with a different number one more blocked thread appears. So the count's marker ([`ResolveGuard`]) moves **into** the read closure and is released only when the read ends.
/// The basis of the value: blocked reads all wait on the same share so concurrency does not raise throughput; it opens only enough (16) to take one burst of batch commands and, even summed with the stage 1 cap, leaves ample room in tokio's default blocking pool (512).
const MAX_RESOLVING_IN_FLIGHT: usize = 16;

/// The minimum interval before a cap event is written again.
/// ★One value opens both directions★: while saturation continues it **keeps the other side from deciding the line count**, and while truly bitten with the count not dropping it **keeps the signal coming** (that situation should be the loudest — basis =
/// [`Seats::ceiling_warned_at`]). The value is a sense of the period at which a person skims logs.
const CEILING_WARN_INTERVAL: Duration = Duration::from_secs(60);

/// The cap on the number of seats that can hold a number ([`Seats::evict_oldest_retained`]).
/// ★The arithmetic of the value★: one held entry is only standing · time · ticket · connection id (the payload is discarded when handing over to holding — [`CommandDeliveries::settle_local`]). 256 entries is within tens of KiB and more than the number of write round trips that
/// end inside one window (one deadline) in normal use — those round trips queue on one profile lock so per-window throughput is not that large. Exceeding it is not normal use but a **runaway caller**, and what must be protected then is the table's boundedness.
const MAX_RETAINED_LOCAL: usize = 256;

/// A marker representing one stage 1 running — ★`Drop` **always** undoes the count★.
/// The count must not leak by any exit — early return · panic · cancellation — so there is no state where the cap fills once and never reopens.
struct LocalGuard {
    deliveries: CommandDeliveries,
    /// Already settled — keeps `Drop` from subtracting the count **twice**.
    settled: bool,
}

impl LocalGuard {
    /// Finishes returning the count and disposing the seat **in one critical section**.
    /// ★Splitting makes the gap a window★: in the instant the reply has already gone out but the seat is still `Awaiting`, if the same request comes again it is folded as [`Seat::Coalesced`] and **receives not one frame** (the reply to coalesce has already gone). If settlement is
    /// one lock that window narrows to the width of one lock. ★Still it is not 0★ — the order that settlement happens **after** the frame goes out (seat rule 2) stands, so a re-query in between is still coalesced. Removing it completely would need the held branch alone to settle
    /// **before** the frame, but then the order diverges from the releasing branch and the rules become two.
    fn settle(mut self, request_id: RequestId, token: SeatToken, retain: bool) {
        self.deliveries.settle_local(request_id, token, retain);
        self.settled = true;
    }
}

impl Drop for LocalGuard {
    fn drop(&mut self) {
        if !self.settled {
            self.deliveries.release_local_slot();
        }
    }
}

/// A marker representing one roster read running — ★dropped inside the read closure★ ([`MAX_RESOLVING_IN_FLIGHT`]).
/// If the waiting side (the delivery task) held it, when that side leaves first (seat settlement · cancellation) the count would be released and a read still blocked would leak beyond the cap. Being `Drop` the count comes back even if the read panics.
struct ResolveGuard {
    deliveries: CommandDeliveries,
}

impl Drop for ResolveGuard {
    fn drop(&mut self) {
        self.deliveries.release_resolve_slot();
    }
}

/// What the blocking body returns — the envelope and the table's outcome (`None` = the table is empty-handed).
type LocalJoin = Result<
    (
        CommandEnvelope,
        Option<Result<serde_json::Value, CommandError>>,
    ),
    tokio::task::JoinError,
>;

/// Is this outcome **worth holding that number** — decides whether the seat is handed to holding or released.
/// ★Why split★: if even a round trip where nothing was applied were held, a caller who fixed a typo and resent with the same number would get back "already applied" for a **command that never ran** (wrong a field name to `agent.new` → refused →
/// fixed and resent). Conversely not holding what was applied would apply the same Write verb twice. The decision axis is one: "what was actually applied".
///
/// ★The decision is made by **the list to release** — not the list of "may have been applied"★
/// Because the costs of the two directions are asymmetric: holding too much **blocks one retry in the window** (the caller uses a new number), holding too little **leaves an undeletable duplicate permanently** (ADR-0122 — there is no delete verb). So only "certainly left nothing" is released and
/// everything else is held. This shape errs **toward the safe side** even when verbs grow or a code's meaning changes.
///
/// - **`Read` is released** — running it again does not change state, and holding it would let a common lookup block that number.
/// - **The failures to release are three = `INVALID_ARGUMENT`·`NOT_FOUND`·`CONFLICT`.** Today across the five verbs these three are all rejections **before touching** the roster (empty value · targeting failure · structural refusal · the store's decision refusal — agent side
///   `a_rejection_from_the_mutating_verbs_leaves_the_roster_untouched` pins that fact). Releasing these three keeps a caller who fixed a typo and resent with the same number from getting `ALREADY_APPLIED` for a "command that never ran".
///   ★One exception — a decision refusal of waking (`target` of `agent.spawn`)★: a registration rewriting that entry with the same value can be refused after it was saved (ADR-0291 §3-9 partial success) — a retry gives the same result so releasing is fine.
///   A launch refusal of create-and-launch leaves the created agent so it comes out as `INTERNAL`, not `CONFLICT` (agent `create_and_start`).
/// - **Everything else is held** — `INTERNAL`·`OUTCOME_UNKNOWN` fall here. The latter especially is a code that **states in its wording** "part may already have been applied" (`control::commands::drive_to_completion`).
/// - **Empty-handed is released** — it is certain the table ran nothing ([`fold_local`] folds it into an `INTERNAL` reply but the decision looks at the table's outcome, not the reply).
/// - **Death is held** — it is not known how far it got.
///
/// ★One known over-hold — `INTERNAL` of a store write failure★: a verb that changes the roster commits only if the save succeeds so a failure the store could not write is in fact a failure **before touching**, yet the code is `INTERNAL`
/// (agent `store_command_error` — only a decision refusal is `CONFLICT`). But this decision holds by code alone, so a caller whose save failed and resends within the window gets `ALREADY_APPLIED` for a **state that did not change**. Accepted under
/// the asymmetry above — resending with a new number works. ★The real fix is not this function★: the port has no field saying "what was applied" (the outcome is only `Ok`/`Err`) so there is no means to see the per-verb truth. When [`LocalCommands::run`] carries that signal this approximation disappears.
fn retains_the_id(effect: Effect, joined: &LocalJoin) -> bool {
    if effect == Effect::Read {
        return false;
    }
    match joined {
        Ok((_, Some(Err(failed)))) => !matches!(
            failed.code(),
            ErrorCode::InvalidArgument | ErrorCode::NotFound | ErrorCode::Conflict
        ),
        Ok((_, None)) => false,
        Ok((_, Some(Ok(_)))) | Err(_) => true,
    }
}

/// Sends the stage 1 body **out to the blocking pool** — the basis of that contract is [`LocalCommands::run`].
/// ★Why `spawn_blocking`★: this table's handlers write disk holding the profile lock and poll resume early-exit for about 3 seconds (agent `make_table` doc). Polled as is in an async task, a runtime worker is blocked that whole time
/// and the frame handling of other connections on the same worker stalls with it.
/// ★The cost — **it cannot be cancelled**★: a body once sent out cannot be recalled so it keeps running past the deadline. That fact is the whole reason [`Pending::local`] and [`Seat::LocalStillRunning`] exist.
fn spawn_local(
    locals: Arc<dyn LocalCommands>,
    roster: CommandRoster,
    origin: ConnId,
    envelope: CommandEnvelope,
    entrance: &'static str,
) -> tokio::task::JoinHandle<(
    CommandEnvelope,
    Option<Result<serde_json::Value, CommandError>>,
)> {
    tokio::task::spawn_blocking(move || {
        let mut envelope = envelope;
        // ★Look at the caller again **just before** the side effect★ — the same check stage 2 does just before handing over the envelope (`OwnerLink::send`), preventing the same thing: a command with side effects running for a caller who has already left.
        //   ★In stage 1 this **late** check is the substance★ — this closure may have queued in the pool so the sample seen when the task started goes stale, hence two looks (before launching the task · here).
        // ★The check-then-act residue still remains★: a connection that drops between this lookup and the execution below is not seen here either. Removing it would mean holding the roster lock through the whole execution and then one slow command stalls
        //   registration · cleanup · other deliveries. **Do not read it as "a command of a caller who has left is never executed".**
        // ★Use a predicate that **does not take an outlet copy**★ (ADR-0154): taking a copy here with `sink_for_conn` would keep that strong reference alive through a body of several seconds and the disconnected connection's writer could not end itself.
        //   The copy for giving the reply is looked up again then and used and dropped (`send_reply`).
        // ★Why comparing only the `ConnId` is enough★: connection ids are not reused (monotonic — the canonical measurement is [`Pending::origin`]). If reuse appears this predicate would answer "alive" for **another peer**, and then this place too must look at a generation number.
        if !roster.is_attached(origin) {
            return (
                envelope,
                Some(Err(no_retry(ErrorCode::Conflict, CALLER_ALREADY_GONE))),
            );
        }
        let outcome = locals.run(&envelope.name, &mut envelope.args, entrance, Some(origin));
        (envelope, outcome)
    })
}

/// The join result of the body → the outcome.
/// ★If the task ends with no answer it is `OUTCOME_UNKNOWN`★ — the body runs to the end on its first poll so it cannot be known where it died and part of the operation may already have been applied (the certainty axis of ADR-0159). Folding to `INTERNAL` would be a false report "certainly failed here".
/// ★The instrumentation is load-bearing★: this branch appears only as one reply so without a log there is no trace on the server side at all (logging convention no silent swallow). The CLI twin leaves an error with `entrance = "cli"` for the same incident
/// (`mcp_server::control_agent_handler`) so the same shape is left here — the incidents of the two surfaces are split by one field.
/// ★How it gets here (a known path)★: the panic net of `blocking_handler` covers **only the handler body**. The correction inside `CommandTable::call`, where the table fits arguments to the declared schema, is **outside** that net, so if it blows up there this task dies whole and comes here.
/// ★No third net on this path — for no name (ADR-0293)★: the execution span of a bus command is not contained. To fold a caught panic into `INTERNAL` ("certainly failed") the body must have changed nothing, and without that check this branch (`OUTCOME_UNKNOWN`) is right
/// (the condition for containing again = the `control::commands::call_daemon_command` doc).
/// ★In the production process a panic on the above path does not reach this branch★ — it is outside the containment marker so the hook leaves a reason line and ends the daemon on the spot (ADR-0292). Reaching here by the above path happens only in test builds without the hook.
fn fold_local(
    joined: LocalJoin,
    request_id: RequestId,
    name: &str,
    entrance: &'static str,
) -> CommandReply {
    match joined {
        Ok((_, Some(outcome))) => CommandReply {
            request_id,
            outcome,
        },
        // ★Do not let a contract violation **flow to the next stage**★: since `claim` answered mine, this place is claiming "delegated to no one" ([`Pending::local`]). Sending the envelope to the owner with that claim on would refuse the owner's legitimate outcome and
        //   disconnect cleanup would not clear this seat either — surface it instead of a quiet mismatch.
        Ok((envelope, None)) => {
            tracing::error!(
                entrance,
                command = name,
                %request_id,
                "the stage-1 table claimed the name as its own and came up empty-handed — the routing authority (`claim`) and execution diverged"
            );
            CommandReply::err(
                request_id,
                no_retry(
                    ErrorCode::Internal,
                    format!(
                        "this daemon claims '{}' but its own table produced no answer for it",
                        envelope.name
                    ),
                ),
            )
        }
        Err(joined) => {
            log_local_death(&joined, request_id, name, entrance);
            CommandReply::err(
                request_id,
                no_retry(
                    ErrorCode::OutcomeUnknown,
                    format!("this daemon's own handler for '{name}' died while running it"),
                ),
            )
        }
    }
}

/// The event where the body ended with no answer — ★the two branches (before and after the deadline) **use the same one line**★.
/// If the field shape were built separately per branch only one side would get fixed and the other would quietly remain under a different name.
fn log_local_death(
    joined: &tokio::task::JoinError,
    request_id: RequestId,
    name: &str,
    entrance: &'static str,
) {
    tracing::error!(
        entrance,
        command = name,
        panicked = joined.is_panic(),
        %request_id,
        "the daemon's own command ended with no answer — the task panicked or the runtime went down (part of the operation may have been applied)"
    );
}

/// The width when putting a post-deadline success payload in the log.
/// ★Using the default name width (64) as is **cuts off the reason this log exists**★: this line's value is `{"agent_id":"<uuid>",…}` and that one field alone is 49 characters — the moment a field is added before it or the id notation gets longer it quietly disappears
/// (and that is exactly when this log is needed). So the width is taken **separately for this place**. ★Widening does not mean anything may be carried★ — that this path writes the success payload whole is stated in the port contract ([`LocalCommands::run`]) and keeping secrets out is that implementation's duty.
const MAX_LATE_OUTCOME_CHARS: usize = 512;

/// Records the outcome of a body that ended **after** the deadline — the reply already went out as `TIMEOUT`.
/// ★Discard but **not silently**★: it tells the caller to "check whether it was applied" while this side holds that answer and throws it away, so without a server record nobody knows what really happened. With `agent.new` **one agent the caller does not know of stays registered**
/// (there is no delete verb — ADR-0122). What that reconciliation needs is the `agent_id` in the payload so it is carried in the wording as is.
/// ★Why the level is `warn`★: the data is not broken but **the state the caller knows and the real state diverged**. Only death (`JoinError`) is split to `error` and uses the same line as the sibling branch ([`log_local_death`]).
/// ★The payload carried is a copy with the `upstream` key stripped ([`strip_upstream`])★ — the upstream raw text goes only to the UI and LLM rows (TRD S21 usage-limit-slot §1-4 amendment 4).
fn log_late_local(
    joined: LocalJoin,
    retained: bool,
    request_id: RequestId,
    name: &str,
    entrance: &'static str,
) {
    match joined {
        // ★The payload is carried only for **what was held**★: the reason to carry it is "a clue to recover what remained unknown to the caller" and an outcome not held (a read · a rejection) has nothing to recover. And a read's payload is the whole roster
        //   so the agents' **absolute cwd paths** would sit whole in the log — not the value this line was meant to buy.
        Ok((_, Some(Ok(mut payload)))) if retained => {
            strip_upstream(&mut payload);
            tracing::warn!(
                entrance,
                command = name,
                %request_id,
                outcome = %sanitize_within(&payload.to_string(), MAX_LATE_OUTCOME_CHARS),
                "a daemon-own command that finished after the deadline **succeeded** — the caller received TIMEOUT so it does not know this result"
            )
        }
        // A success with nothing to recover (a read) — the event is left but the content is not carried.
        Ok((_, Some(Ok(_)))) => tracing::debug!(
            entrance,
            command = name,
            %request_id,
            "a daemon-own command that finished after the deadline succeeded — an outcome not held so there is nothing to recover"
        ),
        Ok((_, Some(Err(failed)))) => tracing::warn!(
            entrance,
            command = name,
            %request_id,
            code = %failed.code(),
            "a daemon-own command that finished after the deadline failed — the caller received TIMEOUT so it does not know this reason: {}",
            sanitize_for_log(failed.message())
        ),
        // The name is ours but the table was empty-handed (it should have gone to stage 2·3) — there is no side effect so it is written quietly.
        Ok((_, None)) => tracing::debug!(
            entrance,
            command = name,
            %request_id,
            "stage 1 that finished after the deadline was empty-handed — the chance to pass it to the delivery stage has already passed"
        ),
        Err(joined) => log_local_death(&joined, request_id, name, entrance),
    }
}

/// Strips **every object key named `upstream` regardless of depth** from the payload before it goes in the log.
/// ★It does not look at the command name★ — splitting by name (`usage.*`) is breached when another command carries the same field. It descends into objects inside arrays. ★The key name is the contract★ — if the name of the upstream raw field
///   (agent `UsageVendorRow.upstream`) changes, this and the pin test must change together.
fn strip_upstream(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            map.remove("upstream");
            map.values_mut().for_each(strip_upstream);
        }
        serde_json::Value::Array(items) => items.iter_mut().for_each(strip_upstream),
        _ => {}
    }
}

/// Runs a whole round trip — delivers, and carries the reply to the origin connection.
/// ★It must run **outside** the connection task★ (TRD §3-6): the frames of one connection are handled by that connection's read loop one at a time to the end, so waiting for this round trip inside that loop means **when the owner is that connection itself it cannot
/// take its own answer out** (the path where the shell puts its own name on and calls it itself — self-deadlock). That one reason is why the caller spawns this future in `dispatch`.
/// ★The reply goes out by **looking up the origin connection again**★ — because no outlet copy is held beyond the round trip (module header). If the origin connection dropped in between there is nowhere to send, and that is the normal end.
///
/// ## ★Stage 1 is **outside** [`route`] — this function is that place★
///
/// The daemon's `agent.*` table now answers through this path ([`LocalCommands`] · `control::commands` are the real ones). The reasons **not** to plug that stage 1 in as the table handed to `route` are two, and both are things `route` cannot express:
/// ① **blocking** — `route` waits the handler future in its own task, but this table's body is blocking that runs to the end on its first poll (profile lock + disk + 3-second polling) and blocks a runtime worker. Moving it to the pool
///    must wrap `table.call` and that call is `route`'s ([`spawn_local`]).
/// ② **entrance check** — `route` deliberately does not call `CommandTable::check_args` (without tolerance between hops additive evolution dies — that function's doc · TRD §4-③). The envelope arriving here, by contrast, is what a person · the LLM just typed
///    so our own names must get a check (ADR-0157). So the check is applied **only to names in our table** (another's name has no declaration to compare against and passes as is) and that check is done by **the same function** as the control route —
///    two entrances with one check is the point.
/// So the table handed to `route` stays empty, and the two stages after it (roster → error) are hop-common as is.
///
/// ★For another's name, the agent-reference fields are turned into ids before handing to `route`★ ([`resolve_agent_args`] · ADR-0308)
/// — both entrances pass this function so the conversion place is one here. If unresolved the envelope is not handed over and it answers with that error.
pub async fn deliver(
    roster: CommandRoster,
    deliveries: CommandDeliveries,
    locals: Arc<dyn LocalCommands>,
    origin: ConnId,
    envelope: CommandEnvelope,
    entrance: &'static str,
) {
    let request_id = envelope.request_id;
    let name = sanitize_for_log(&envelope.name);

    // ★Open the seat before anything★ (module header rule 1) — so that **whichever of the 3 stages makes the answer** (stage 1 is `spawn_local` below) that answer sits on one seat it must be here. It used to be that only stage 2 (delivery) opened a seat,
    //   so the replies of stage 1 (own table) and stage 3 (owner absent) went out without going through the table at all — a deterministic path emitting a second frame under the key of a round trip in progress (it happens even without a race).
    // ★Decide whether this name is mine **before opening the seat**★: that fact must be born with the seat so there is no gap for an outcome another sent to slip in ([`Pending::local`]). This question is pure — no execution, no lock.
    let claim = locals.claim(&envelope.name);
    let (rx, token) = match deliveries.open(
        request_id,
        origin,
        &envelope.name,
        &envelope.args,
        claim.is_some(),
    ) {
        Seat::Opened { rx, token } => (rx, token),
        // The reply is given by the round trip opened first — giving it here would make two under the same key (basis = [`Seat::Coalesced`]).
        Seat::Coalesced => {
            tracing::debug!(
                conn = origin,
                %request_id,
                %name,
                "same request from the same connection — coalesced into the round trip in progress (that one gives the reply)"
            );
            return;
        }
        // The three below failed to get a seat but **still answer** — that decision is this query's only answer (basis = [`Seat`]).
        Seat::Conflict => {
            send_reply(
                &roster,
                origin,
                &name,
                CommandReply::err(
                    request_id,
                    no_retry(
                        ErrorCode::RequestIdConflict,
                        format!(
                            "request_id {request_id} is already in flight for a different command"
                        ),
                    ),
                ),
            );
            return;
        }
        Seat::Taken { holder } => {
            // ★It splits by whether the owner is still attached — what splits is **only the fact the code states**★. The instruction is `Never` for both branches ([`no_retry`]) so what is chosen here is not "what should the caller do" but
            //   "what to say happened" (the certainty axis of ADR-0159).
            //   · attached = the key overlapped with a live round trip of another. We **ran nothing** and that is certain → `REQUEST_ID_CONFLICT`. The cause is exactly that one line.
            //   · absent = that seat's owner has already left. Most likely **the same client dropped and reattached** and is asking again with that number (the client has no identity that lives across reconnects
            //     so by connection id it looks like a stranger), and the envelope of that earlier round trip **may already have gone to the owner**. Whether it was applied is unknown so `OUTCOME_UNKNOWN` is the fact.
            //   ★Do not lump the two into `REQUEST_ID_CONFLICT`★ — that would write an operation that may have been applied as "nothing happened", and that is not a diagnosis but a false report.
            let reply = if roster.sink_for_conn(holder).is_some() {
                CommandReply::err(
                    request_id,
                    no_retry(
                        ErrorCode::RequestIdConflict,
                        format!(
                            "request_id {request_id} is already in flight for another connection"
                        ),
                    ),
                )
            } else {
                CommandReply::err(
                    request_id,
                    no_retry(
                        ErrorCode::OutcomeUnknown,
                        format!(
                            "request_id {request_id} is still held by a round trip whose caller is gone"
                        ),
                    ),
                )
            };
            send_reply(&roster, origin, &name, reply);
            return;
        }
        // ★Asked again with the same number but **that work is still running**★ — it is not run again (the same Write verb would be applied twice). The certainty is "unknown": the earlier attempt already answered `TIMEOUT` but its body is still running now
        //   and may be applied or may fail. So the code is `OUTCOME_UNKNOWN` as the fact, and what the caller should do is not **ask again with a new number** but **look up the result to confirm**.
        Seat::LocalStillRunning => {
            send_reply(
                &roster,
                origin,
                &name,
                CommandReply::err(
                    request_id,
                    no_retry(
                        ErrorCode::OutcomeUnknown,
                        format!(
                            "request_id {request_id} passed its deadline but this daemon is still running that command, so it was not run again — check whether it took effect before sending anything under a new request_id"
                        ),
                    ),
                ),
            );
            return;
        }
        // ★A number already run — a second application is not made★. The certainty is **applied** (that body ran to the end). The result itself is not held (that is the job of completed-work replay = a real dedup store) so `ALREADY_APPLIED`,
        //   which confirms only the fact, is exactly the vocabulary of this place.
        Seat::LocalAlreadyRan => {
            send_reply(
                &roster,
                origin,
                &name,
                CommandReply::err(
                    request_id,
                    no_retry(
                        ErrorCode::AlreadyApplied,
                        format!(
                            "request_id {request_id} already ran on this daemon and is not run again — this daemon does not keep the result, so read the current state instead of resending"
                        ),
                    ),
                ),
            );
            return;
        }
        Seat::Closed => {
            send_reply(
                &roster,
                origin,
                &name,
                CommandReply::err(
                    request_id,
                    no_retry(
                        ErrorCode::OutcomeUnknown,
                        "the daemon is shutting down and no longer takes commands",
                    ),
                ),
            );
            return;
        }
    };

    // ── stage 1 — my own table ("stage 1 is outside route" in the doc above) ──────────────────────────
    if let Some(effect) = claim {
        // ★Do not **start work** for a caller who left★ — the same as the check stage 2 does just before handing over the envelope (`OwnerLink::send`), and this one is the **early** side: it does not put work that cannot pass into the pool and uses not one cap permit.
        //   The **late** check actually just before the side effect is inside the body ([`spawn_local`]) — that closure can queue in the pool so the sample here goes stale. With only one of the two: with only the early one an exit during queue wait is missed, and with only the late one
        //   a seat · pool job · one cap permit is spent for a caller who has already left.
        // ★Here too no outlet copy is taken★ — the reason is the same check inside `spawn_local`.
        if !roster.is_attached(origin) {
            send_reply(
                &roster,
                origin,
                &name,
                CommandReply::err(
                    request_id,
                    no_retry(ErrorCode::Conflict, CALLER_ALREADY_GONE),
                ),
            );
            deliveries.release(request_id, token);
            return;
        }
        // The concurrent-execution cap — on overflow it refuses **running nothing** (and leaves that event on the server).
        // ★The seat is released (not held)★: nothing ran so holding that number would block a normal retry with `ALREADY_APPLIED` — the decision rule is [`retains_the_id`].
        let guard = match deliveries.reserve_local_slot(&name, entrance) {
            Ok(guard) => guard,
            Err(refused) => {
                send_reply(
                    &roster,
                    origin,
                    &name,
                    CommandReply::err(request_id, refused),
                );
                deliveries.release(request_id, token);
                return;
            }
        };
        let mut job = spawn_local(
            Arc::clone(&locals),
            roster.clone(),
            origin,
            envelope,
            entrance,
        );

        // ★Wait for the body and **the seat** together★ — without looking at the seat no frame goes out even past the deadline and the caller hangs indefinitely while only the sweeper log writes "answered with TIMEOUT" (writing what was not done).
        //   Do not revert to a shape that waits only for the body.
        // ★Why `biased`★: if both are ready pick the **real outcome**. Picking at random opens a window to send `TIMEOUT` for work that has already ended. In that instant the sweeper log still writes "answered with TIMEOUT"
        //   but a log one tick inaccurate is better than losing the answer.
        let mut rx = rx;
        let (reply, retain) = tokio::select! {
            biased;
            joined = &mut job => {
                // ★Whether to hold is decided by **the outcome the table gave** — not a reply we made up★: folding empty-handed into an `INTERNAL` reply and then looking at that reply would read as "applied" and hold a number that never ran.
                let retain = retains_the_id(effect, &joined);
                (fold_local(joined, request_id, &name, entrance), retain)
            }
            // ★There are **only two** ways a seat settles first — the deadline ([`CommandDeliveries::expire`]) and daemon shutdown ([`CommandDeliveries::drain`])★. What might look like a third (an outcome sent by the owner) cannot reach this seat:
            //   this round trip was delegated to no one and the table refuses by that fact ([`OutcomeLanding::NotDelegated`]). Without that refusal a payload another made up would go out here **as this command's answer** and the real result would be discarded below.
            settled = &mut rx => {
                // That reply is **this round trip's only frame**.
                send_reply(&roster, origin, &name, settled.unwrap_or_else(|_| CommandReply::err(
                    request_id,
                    no_retry(
                        ErrorCode::OutcomeUnknown,
                        "the caller's connection went away while this daemon was running the command",
                    ),
                )));
                // ★After the frame goes out it **still holds the seat until the body ends**★ — releasing here would let a re-query of the same number see an empty seat and run **the same Write verb once more** (two agents appear).
                //   The body cannot be cancelled ([`spawn_local`]) so "wait then release" is the only means. A re-query in between is blocked by [`Seat::LocalStillRunning`].
                // ★The outcome is **discarded but recorded**★: the reply already went out so there is no place to carry this result. Discarded silently, the caller knows only TIMEOUT and nobody knows what was actually made (e.g. one agent)
                //   ([`log_late_local`]).
                let joined = (&mut job).await;
                let retain = retains_the_id(effect, &joined);
                log_late_local(joined, retain, request_id, &name, entrance);
                guard.settle(request_id, token, retain);
                return;
            }
        };
        send_reply(&roster, origin, &name, reply);
        // ★The seat is disposed **after the frame goes out**★ (seat rule 2): releasing before that lets a re-query overturn a seat that the deadline just brushed into `Void` and run the same verb once more.
        guard.settle(request_id, token, retain);
        return;
    }

    let mut envelope = envelope;
    let mut rx = rx;
    if let Err(reply) = resolve_agent_args(
        &roster,
        &deliveries,
        &locals,
        origin,
        &mut envelope,
        &mut rx,
        entrance,
    )
    .await
    {
        send_reply(&roster, origin, &name, reply);
        deliveries.release(request_id, token);
        return;
    }

    let link = OwnerLink {
        roster: &roster,
        origin,
        waiter: Mutex::new(Some(rx)),
    };
    // The remaining two stages are the same per hop — no special case is made (ADR-0155 decision 3).
    let reply = route(&CommandTable::new(&[]), &roster, &link, envelope).await;

    send_reply(&roster, origin, &name, reply);
    // ★Released **after the frame goes out**★ (module header rule 2) — releasing before that makes a re-query send the envelope once more. It picks by ticket so if this seat has already gone to another round trip this call does nothing.
    deliveries.release(request_id, token);
}

/// The agent-reference conversion before the relay (ADR-0308) — turns the **string values** of the fields the owner marked at registration into agent ids with this daemon's resolver
/// ([`resolve_in`] — ADR-0132) and writes them back into the envelope.
/// All the daemon knows is "this field points at my agent" — what that field means on the owner side it does not know and it does not open `help` either. Fields are resolved in the marked order, and absent · `null` · non-string values are left untouched (the owner
/// refuses with that value).
/// `Err` = **this round trip's answer** and the envelope does not go out — the caller already left · the resolve cap ([`MAX_RESOLVING_IN_FLIGHT`]) · resolution failure (the code is the resolver's as is · `<field>: ` before the wording) ·
/// the seat settled first during resolution (deadline · shutdown · caller left — the code of that decision is the answer and the wording is corrected to "did not go to the owner") · death of the resolution job.
/// ★If the seat settled first it does not relay★: handed over as is the owner would run the command while the caller receives the `TIMEOUT` already given.
/// ★The remaining window — the marking is read here and `route` looks the owner up again★: if in between an owner that re-registered the same name has different markings, that field is changed less or more this round trip. If the name remains the owner
/// refuses, and since the resolver looks at an id before a name a more-changed field is the same agent.
// ADR-0308
async fn resolve_agent_args(
    roster: &CommandRoster,
    deliveries: &CommandDeliveries,
    locals: &Arc<dyn LocalCommands>,
    origin: ConnId,
    envelope: &mut CommandEnvelope,
    rx: &mut oneshot::Receiver<CommandReply>,
    entrance: &'static str,
) -> Result<(), CommandReply> {
    let request_id = envelope.request_id;
    let command = sanitize_for_log(&envelope.name);
    let refs: Vec<(String, String)> = roster
        .agent_args_of(&envelope.name)
        .into_iter()
        .filter_map(|arg| {
            let token = envelope.args.get(&arg)?.as_str()?.to_string();
            Some((arg, token))
        })
        .collect();
    if refs.is_empty() {
        return Ok(());
    }
    // Do not start a read for a caller who left — the same as the early check of stage 1 (that branch of `deliver`; the reason it is a predicate that takes no outlet copy is the same).
    if !roster.is_attached(origin) {
        return Err(CommandReply::err(
            request_id,
            no_retry(ErrorCode::Conflict, CALLER_ALREADY_GONE),
        ));
    }
    let slot = deliveries
        .reserve_resolve_slot(&command, entrance)
        .map_err(|refused| CommandReply::err(request_id, refused))?;

    let locals = Arc::clone(locals);
    let started = Instant::now();
    let mut job = tokio::task::spawn_blocking(move || {
        // The marker drops when the read ends — even if the waiting side leaves first a blocked read stays within the cap ([`ResolveGuard`]).
        let _slot = slot;
        let agents = locals.agent_roster();
        refs.into_iter()
            .map(|(arg, token)| match resolve_in(&agents, &token) {
                Ok(agent) => Ok((arg, agent.id.to_string())),
                Err(refused) => Err(no_retry(
                    refused.code(),
                    format!("{arg}: {}", refused.message()),
                )),
            })
            .collect::<Result<Vec<_>, CommandError>>()
    });

    // ★Look at the seat first with `biased`★ — if both are ready and the resolution result were picked, a round trip that already answered would be relayed.
    let joined = tokio::select! {
        biased;
        settled = &mut *rx => {
            let reply = match settled {
                Ok(reply) => never_sent(reply),
                Err(_) => CommandReply::err(
                    request_id,
                    no_retry(ErrorCode::Conflict, CALLER_ALREADY_GONE),
                ),
            };
            tracing::warn!(
                entrance,
                %command,
                %request_id,
                elapsed = ?started.elapsed(),
                code = %reply.outcome.as_ref().err().map_or("ok", |e| e.code().as_str()),
                "while resolving the agent-reference fields the round trip's seat settled first — not relayed"
            );
            return Err(reply);
        }
        joined = &mut job => joined,
    };
    match joined {
        Ok(Ok(resolved)) => {
            for (arg, id) in resolved {
                envelope.args[arg.as_str()] = serde_json::Value::String(id);
            }
            Ok(())
        }
        Ok(Err(refused)) => Err(CommandReply::err(request_id, refused)),
        Err(died) => {
            tracing::error!(
                entrance,
                %command,
                %request_id,
                panicked = died.is_panic(),
                "the job resolving the agent-reference fields ended with no answer — the envelope was not relayed"
            );
            Err(CommandReply::err(
                request_id,
                no_retry(
                    ErrorCode::Internal,
                    "this daemon could not read its agent roster to resolve the agent arguments, so the command was not sent",
                ),
            ))
        }
    }
}

/// The answer that settled the seat while resolving agent-reference fields — the wording of deadline ([`CommandDeliveries::expire`]) · shutdown ([`CommandDeliveries::drain`]) describes the circumstances of a round trip that sent the envelope
/// ("the owner did not answer" · "it went down while running") so here it is false. The envelope never went to the owner so the certainty is "nothing ran" (the certainty axis of ADR-0159), and the wording
/// is corrected to that fact. ★The code stays as the settling side decided★. Other answers are left alone.
// ADR-0308
fn never_sent(reply: CommandReply) -> CommandReply {
    let Err(settled) = &reply.outcome else {
        return reply;
    };
    let code = settled.code();
    let message = match code {
        ErrorCode::Timeout => "the deadline passed while this daemon was still resolving the agent arguments, so the command was never sent to its owner and nothing was run",
        ErrorCode::OutcomeUnknown => "the daemon shut down while it was still resolving the agent arguments, so the command was never sent to its owner and nothing was run",
        _ => return reply,
    };
    CommandReply::err(reply.request_id, no_retry(code, message))
}

/// Sends out one reply by **looking up the origin connection again** — because no outlet copy is held beyond a round trip (module header).
/// ★At every call place this is "this query's only answer"★ — whichever branch of [`deliver`] calls it, that branch ends right away. Do not make a branch that calls it twice.
/// ★And this place is **the only outlet that issues the retry instruction**★ — the basis is the line below.
fn send_reply(roster: &CommandRoster, origin: ConnId, name: &str, mut reply: CommandReply) {
    let request_id = reply.request_id;

    // ★Issued **once here** regardless of origin★: every failure answer this daemon sends the caller — one it built itself ([`no_retry`]), one built by `route`'s panic net, or **one relaying what the owner sent** — goes out as `Never`.
    //   ★A relay hop issuing the instruction is behavior the contract already assumed★ — that is why the code field and the `retry` field are separate: the certainty statement (what happened) belongs to the side that originally answered, and the instruction
    //   (so do this) belongs to the hop that has the device to bear that instruction. Until a dedup that replays completed work stands, that responsibility is on this hop — if not issued here "ask again with the same number" would cause **re-execution**
    //   (full reason · condition to resume = [`no_retry`]). ★The code vocabulary is left alone★ — only the instruction is issued and the certainty statement goes as the original. That is why it is `set_retry_for_relay` and not `set_retry`:
    //   the former **throws away the whole raw copy** and erases even the unknown code and out-of-contract fields the owner sent (a relay hop erasing another's vocabulary = exactly what the `received` doc forbids). The latter overwrites only the `retry` field of the copy
    //   so the instruction really goes down and the vocabulary survives. ★Do not add places for this★: issued in two places a later session fixes only one and the other quietly leaks.
    if let Err(error) = &mut reply.outcome {
        error.set_retry_for_relay(RetryMode::Never);
    }
    let Some(sink) = roster.sink_for_conn(origin) else {
        tracing::debug!(
            conn = origin,
            %request_id,
            %name,
            "no place to send the command reply — the asking connection has already dropped"
        );
        return;
    };
    // ADR-0272: wrapped into the raw field — the retry instruction issued above is carried in these bytes.
    let packet = match reply_to_wire(&reply) {
        Ok(reply) => event_json(&AgentEvent::CommandReply { reply }),
        Err(e) => {
            tracing::error!(conn = origin, %request_id, %name, "could not wrap the command reply into the raw field: {e}");
            None
        }
    };
    let Some(text) = packet else {
        // Both failures already left an error! (`event_json` · the branch above) — one more line here would write the same incident twice.
        return;
    };
    // ★Do not quietly swallow this failure★: this is the **only** reply of that `request_id`, and the outcome already took the seat away so there is no way to undo — swallowed, the caller would never get an answer with not even a deadline set, and the server side
    //   would have no trace of the incident anywhere (logging convention "instrumentation duty" · antipattern "silent swallow"). The level is `warn!` because there is no data risk (safe) but it is **not normal** — if the queue is saturated or closed the network layer closes that
    //   connection soon so the caller is cut off anyway.
    if sink.try_send(Frame::Text(text)).is_err() {
        tracing::warn!(
            conn = origin,
            %request_id,
            %name,
            "could not put the command reply into that connection — this round trip's only reply is lost (queue saturated · closed)"
        );
    }
}

// ── the second entrance — a connectionless caller (ADR-0160) ────────────────────────────────────

/// The number of control-route calls that can hold a relay at the same time.
///
/// ★What this constant binds is **concurrency only** — not resident bytes★
///
/// A cap is needed because this door has no connection lifetime: a round trip on the socket entrance hangs on **one connection** so when it drops the seat is cleaned ([`CommandDeliveries::drop_origin`]), but this door's caller stands up anew per request. One relay round trip holds
/// **a task + a seat + a roster entry** for the deadline ([`CommandDeliveries::DEFAULT_DEADLINE`]) and even when it leaves by cancellation all three come back (the count is [`RelaySlot`], the other two are [`CallerSeat`]) — so what this number counts is really
/// "the round trips held at the same time now".
/// ★There is a fourth — the roster read (blocking pool) of a command that marked agent-reference fields★: it does not come back on cancellation and remains until the read ends (`spawn_blocking`). So this number cannot bind it and [`MAX_RESOLVING_IN_FLIGHT`] binds it separately.
///
/// ★Do not claim a byte cap here★: what a seat holds is not the request body but **a parsed `serde_json::Value` copy** ([`Pending::args`]). The body is closed at 1 MiB (`MAX_BODY_BYTES` of `mcp_server`) but the ratio at which those bytes swell into a `Value`
/// is **decided by shape** — one array full of small elements makes a `Value` per element. So "1 MiB × this number = 64 MiB" is not true, and to bind bytes a separate axis measuring the size of what a seat holds must stand (there is none now).
///
/// ★The arithmetic of the value★: set to the same order as the sibling cap [`MAX_LOCAL_IN_FLIGHT`] so the two together speak of the scale of "work this daemon holds at once". It is not a throughput governor — on overflow it **refuses loudly** instead of growing the queue
/// (the same argument as the roster capping its own residency — `Roster::MAX_NAMES`).
/// ★A **different axis** from the stage 1 cap★: that one counts **its own bodies** out on the blocking pool so it does not apply to relays (that constant's doc ②). Merging the two would let traffic carrying another's name eat its own execution cap.
/// ★The axis is **global — all credentials share this 64**★: so one agent holding all of it makes every other refused (its own share is not guaranteed). Whether to split per credential is **open**, and that it is global now is not because that judgment was made but
/// because this round did not decide.
const MAX_RELAYED_IN_FLIGHT: usize = 64;

/// One copy of the command bus — ★both entrances hold **this one value**★ (socket dispatch · the control route).
///
/// The reason to bind the four into one value is **to block a wiring accident by type**: the seat table · roster · stage 1 table must be a pair from the same assembly, but if passed separately an assembly where only one entrance gets a different copy is representable. The symptom then is, with neither error
/// nor log, "discovery works but only the call forever does not know the name", or one entrance cannot see a seat the other opened (a re-query of the same number applies the same operation twice).
#[derive(Clone)]
pub struct CommandBus {
    roster: CommandRoster,
    deliveries: CommandDeliveries,
    locals: Arc<dyn LocalCommands>,
    /// The next number to give a connectionless caller — [`CommandBus::open_caller`].
    callers: Arc<AtomicU64>,
    /// The number of control-route calls holding a relay now — material for the cap decision ([`MAX_RELAYED_IN_FLIGHT`]).
    relayed: Arc<AtomicUsize>,
}

impl CommandBus {
    /// Returns the bus **together with that bus's only sweeper**.
    /// ★Why this is not a function that launches the sweeper separately★: that shape makes **representable** two sweepers on one bus, and the symptom of that state is nasty — the marker that drops first calls [`CommandDeliveries::drain`] on its way out and **closes the shared table**,
    /// and every later round trip is refused as [`Seat::Closed`] (the daemon is perfectly alive yet every command becomes "shutting down"). Returned as a pair that state cannot be made.
    /// ★Returning as a pair and hiding the two verbs **are not enough alone**★: the seat table is shared via [`CommandBus::deliveries`] and is `Clone`, so with this constructor left `pub` and **accepting another's table** one line
    /// `CommandBus::new(.., bus.deliveries().clone(), ..)` outside could build the above state as is (regardless of `spawn_sweeper`·`drain` being invisible outside this module). What really closes that hole is
    /// **the table remembering its own sweeper** ([`Seats::swept`]) and the second request panics inside this constructor. So when writing "such a state is unrepresentable" here the basis to cite is that marker, not visibility.
    /// ★This form also forces launching **at the very place the bus is made**★: leaving it to the accept loop creates a window — the control server stands before the accept loop (the fail-closed order of daemon `run()`) and a control route call arriving in between has no eyes to
    /// watch the deadline and hangs with neither answer nor error. In the days with only the socket entrance the two moments were the same so this window did not exist.
    /// ★It must be called in an async context★ — the sweeper is a tokio task.
    #[must_use = "the assembly must hold this marker and stop it at shutdown for shutdown to be bounded"]
    pub fn new(
        roster: CommandRoster,
        deliveries: CommandDeliveries,
        locals: Arc<dyn LocalCommands>,
    ) -> (Self, BusSweeper) {
        let bus = Self {
            roster,
            deliveries,
            locals,
            callers: Arc::new(AtomicU64::new(u64::MAX)),
            relayed: Arc::new(AtomicUsize::new(0)),
        };
        // ★Why the stop signal is **a separate signal this marker owns**, not the daemon shutdown watch★: there are three shutdown branches (StopDaemon · Ctrl-C · sender gone) and Ctrl-C of them does not touch that watch, so subscribing it would
        //   mean on that branch the sweeper never wakes and the shutdown wait never ends.
        let (stop, stopped) = watch::channel(false);
        let sweeper = BusSweeper {
            task: bus.deliveries.spawn_sweeper(stopped),
            stop,
        };
        (bus, sweeper)
    }

    /// A bus with neither its own table nor attached owners — ★only for assemblies measuring **other parts** of the control plane★ (smoke bins · mail gate · auth tests).
    /// ★Do not use in a production assembly★: it does not ask the stage 1 table slot so `agent.*` quietly becomes an "unknown command" and since the roster is empty no name is relayed. Production is [`CommandBus::new`].
    /// ★The sweeper is made here too★ — the reason this is separate from "has no table of its own" is that [`CommandBus::invoke`] of a bus without eyes on the deadline **never wakes** the moment one seat is opened. Not making that state is cheaper than writing that fact in prose.
    #[must_use = "dropping the sweeper marker closes the seat table on the spot"]
    pub fn without_commands() -> (Self, BusSweeper) {
        Self::new(
            CommandRoster::new(),
            CommandDeliveries::new(),
            Arc::new(NoLocalCommands),
        )
    }

    pub fn roster(&self) -> &CommandRoster {
        &self.roster
    }

    /// The seat table this bus holds — the place where connection cleanup (`agent_conn`) and assembly · tests hold the same one copy.
    /// ★This accessor **cannot touch the table's lifetime**★: the verb launching the sweeper and the verb closing the table are invisible outside this module (reason = [`CommandDeliveries::spawn_sweeper`]), and the path of feeding a copy obtained here back to
    /// [`CommandBus::new`] to stand a second sweeper is blocked by the table's marker ([`Seats::swept`]) — that assembly does not stand and panics on the spot. If either is lifted this sentence becomes false.
    pub fn deliveries(&self) -> &CommandDeliveries {
        &self.deliveries
    }

    pub fn locals(&self) -> &Arc<dyn LocalCommands> {
        &self.locals
    }

    /// Test only — aims at the cap branch **without holding 64 resources**.
    /// To run that branch with the production value 64 would have to be hung on an owner that does not answer and then what the test measures is not the decision but the runtime scheduler (the same reason as the sibling [`CommandDeliveries::with_local_limit`]).
    #[cfg(test)]
    pub(crate) fn relayed_counter(&self) -> &Arc<AtomicUsize> {
        &self.relayed
    }

    /// The **connection number** given to a connectionless caller — one round trip lives and [`CallerSeat`] reaps it.
    /// ★It must not overlap the network layer's numbers★: if it did, [`CommandRoster::attach`] would overwrite the live connection's frame outlet (the premise that function pinned) so an envelope bound for that connection would leak into this HTTP request while this side's reply would vanish.
    /// The network layer **goes up** from 1 (`ConnRegistry::alloc_id` of `link/server.rs` — release does not give it back) and here it **goes down** from `u64::MAX`. For the two sequences to meet 2^63 allocations are needed so it is unreachable.
    /// ★There must be one of this issuer per process★ — that is why [`CommandBus`] is handed out by clone (making a new issuer per copy would make two copies emit the same number).
    fn open_caller(&self) -> ConnId {
        self.callers.fetch_sub(1, Ordering::Relaxed)
    }

    /// One command of a connectionless caller — ★the delivery machine rides **the same one** as the socket entrance★ (ADR-0160 decision 1).
    /// ★It takes `request_id` as an argument — it is not made here★: whether to ask again with the same number is **the caller's** decision, and that re-query being caught by the seat table's duplicate prevention ([`SeatState::Retained`]) is why this door does not make a second wait
    /// ticket (ADR-0161 decisions 1·2). On the control route that value is one the request body carried (a retry sends back the same value) or, if absent, one the daemon issued with [`RequestId::new`] — **that the latter gets no duplicate prevention
    /// is the contract** (the impact section of the same decision).
    ///
    /// ★Sets up one fake connection and reaps it★: the delivery machine holds "the connection that asked" by number (seat owner · aliveness check · reply destination) and all three go through the roster. A connection that stands only during the round trip is **the lifetime of the HTTP
    /// request itself** so it is a fact, not a pretense, and that is why [`deliver`] can be ridden without changing one line.
    /// ★What reaps is the destructor of [`CallerSeat`] — do not revert to straight-line code★: this future **is cancelled**. axum/hyper drops the handler future when the client drops (the sibling route wrote the same fact — the at-least-once section of
    /// `mcp_server::eg_send`), and then the cleanup of straight-line code **does not run a single line**. ★No name is registered★ — this connection cannot become a command owner (the control door is call-only). With no registration path here no name can stand under this number so
    /// another's envelope cannot come here.
    ///
    /// ★It waits right there and does not spawn `deliver`★: the reason spawn is needed is **the self-deadlock where a connection task cannot take its own answer out** (that function's doc), and here it is a per-request task, not a connection task,
    /// so that causality is absent. Rather, spawning here would need yet another place to take the reply.
    ///
    /// ★A deadline overrun comes as a `TIMEOUT` reply — what makes it is the sweeper★ ([`CommandDeliveries::expire`]). That sweeper is brought out together by [`CommandBus::new`].
    pub async fn invoke(
        &self,
        request_id: RequestId,
        name: String,
        args: serde_json::Value,
    ) -> CommandReply {
        // ★**The daemon's own names do not come through this door — that contract is enforced here**★
        //   The decision part runs its own table first (`control::catalog::handle_call`) so today this branch does not come. ★For that "first" to stand it must be **the same table**, and today what carries that is the type★: the decision part takes the stage 1 table as `&dyn LocalCommands`
        //   and what the adapter passes is the one [`CommandBus::locals`], so that decision and the check below ask **the same question of the same value** (the basis is that paragraph of `handle_call`). Reverting to a shape that takes the table
        //   separately would let the two decisions see different copies, and then this refusal would either block a legitimate relay (one side answers it is its own) or become a quiet no-op.
        //   Even so the reason this check is not lifted is that [`deliver`]'s stage 1 branch **leans on that fact**: if that branch rode this door a single cancellation would leave a seat permanently. An HTTP handler future is cancelled
        //   ([`CallerSeat`]) while the stage 1 body is `spawn_blocking` and cannot be cancelled so its cleanup (`LocalGuard::settle`) runs not a single line, and the leftover seat, because of the `local` marker, can be touched by neither disconnect cleanup
        //   ([`CommandDeliveries::drop_origin`]) nor deadline reaping ([`CommandDeliveries::expire`] releases only `Retained`) — one seat holding copies of the name · arguments and one number frozen as "still running" remain for the whole process lifetime, and
        //   that number is only ever answered with [`Seat::LocalStillRunning`]. Besides, the cap count ([`MAX_LOCAL_IN_FLIGHT`]) is released while the body still runs so that axis cannot bind a live body.
        //   ★So it is not left as prose but **refused at the place that leans on it**★ — then that branch becomes unreachable at this door and the above leak cannot be expressed in code.
        // ★The vocabulary is `INTERNAL`★: it is a wiring defect and nothing ran (certainty = certain). And it turns away before taking any resource — it comes before the admission control below.
        // ★Why there is no `debug_assert`★: then the test measuring this branch would itself panic in debug, removing the regression net of the contract being enforced. The level (`error!`) carries that job.
        if self.locals.claim(&name).is_some() {
            tracing::error!(
                entrance = ENTRANCE_CONTROL,
                command = %sanitize_for_log(&name),
                %request_id,
                "the control route sent the daemon's own name as a relay — that name must be answered by the decision part in its own place (wiring defect, ADR-0160)"
            );
            return CommandReply::err(
                request_id,
                no_retry(
                    ErrorCode::Internal,
                    "this daemon answers that command itself, so the control route must not relay it — nothing was run; this is a wiring defect in the daemon, not something the caller can fix",
                ),
            );
        }

        // ★Admission control comes **before** taking resources★ — the seat, the task and the roster entry are all created after passing here.
        let Some(_admitted) = RelaySlot::admit(&self.relayed, &name, request_id) else {
            return CommandReply::err(
                request_id,
                no_retry(
                    ErrorCode::Conflict,
                    format!(
                        "this daemon is already relaying {MAX_RELAYED_IN_FLIGHT} commands for control-route callers and will not queue more, so nothing was run — retry once they finish"
                    ),
                ),
            );
        };

        let logged = sanitize_for_log(&name);
        let (answer, wait) = oneshot::channel();
        let caller = self.open_caller();
        let catcher: Arc<dyn FrameSink> = Arc::new(ReplyCatcher::new(answer, request_id));
        // The destructor reaps — even if cancellation leaves this function neither the roster entry nor the seat remains ([`CallerSeat`]).
        let seat = CallerSeat::attach(
            self.roster.clone(),
            self.deliveries.clone(),
            caller,
            catcher,
        );

        deliver(
            self.roster.clone(),
            self.deliveries.clone(),
            Arc::clone(&self.locals),
            caller,
            CommandEnvelope {
                name,
                request_id,
                // ★The destination owner is **overwritten** by `route` with the roster lookup result★ (that function) — a value made up here is read nowhere. This is not a place to carry a self-claimed identity (`CommandEnvelope::owner`).
                owner: OwnerToken::new(""),
                proto_ver: engram_dashboard_agent::commands::CATALOG_VERSION,
                args,
            },
            ENTRANCE_CONTROL,
        )
        .await;

        // ★Reap **after the reply goes out**★ — reaping before that leaves `send_reply` nowhere to send and this round trip's only reply vanishes. Left to the destructor the order is the same (end of scope), but releasing explicitly makes that order visible in edits.
        // ★This line releases **the seat too** — but in a normal round trip there is nothing to release★ ([`CallerSeat`]): the `deliver` above already disposed of its own seat after sending the frame ([`CommandDeliveries::release`]). So the only case where a seat is erased here is
        //   cancellation, and in that branch this line runs **even before reaching `wait.await` below**. ★The basis for that "only" is the refusal at the very top of this function★ — a stage 1 branch cannot release its seat when cancelled (the body is
        //   `spawn_blocking` so cleanup does not run) and since that branch is unreachable at this door, only seats of the delivery branch come here. Lifting that refusal makes this sentence false at once. That is, the reason this place must not be moved earlier is
        //   not the seat axis but the **roster axis** (the first line above).
        drop(seat);

        let reply = wait.await.unwrap_or_else(|_| {
            // The only way to get here is **the reply frame not reaching this outlet**. One is an encoding failure (`event_json` of `send_reply` came back empty — that side already left an error!), the other is this catcher
            //   rejecting that frame (a mismatched number · a second frame — each leaves its own at its own place). Coalescing ([`Seat::Coalesced`]) cannot arise at this door — the connection number is new per request so the same
            //   (number, connection) pair does not stand twice.
            tracing::error!(
                entrance = ENTRANCE_CONTROL,
                command = %logged,
                %request_id,
                "the control route command ended with no reply — the frame the delivery emitted did not reach this request"
            );
            CommandReply::err(
                request_id,
                no_retry(
                    ErrorCode::OutcomeUnknown,
                    "this daemon finished the round trip without producing a reply",
                ),
            )
        });
        // ★Success is recorded too★ — instrumentation that writes only failures cannot answer "what went through this door and how much", so when an incident happens even **whether that command passed through this daemon** cannot be told from the log (logging convention "instrumentation duty").
        //   The outcome payload is not carried — it is the return value of another's command so this layer does not know what is in it.
        // ★The level splits — only failures are `warn`, success is `info`★
        //   The axis that splits is **post-hoc recoverability**. The one line on this path that gets `warn` is one (the attribution line of `control::mcp_server`) and what it holds is **a fact that cannot be remade anywhere after the event passed**: **which credential**
        //   made a dashboard change visible to the user. Even raising the log level later does not tell the owner of an operation that already happened. Putting that one line at warn on this route is the judgment to treat "an agent moved a human's screen" as
        //   **an event to look at rather than everyday noise**, and the sibling gate (the mail refusal) sits in the same place. By contrast what the success line here adds is **only the one fact that it ended**, and that is recovered after the fact — the operation
        //   itself (a tab was created · a window was closed) is the evidence and a state lookup confirms it. So not being printed at the default level is not a loss. A failure is warn again: "it was rejected" leaves no trace on screen so the log is the only evidence.
        //   ★As a result one relay has one warn line★ — raising success would make two lines and then "an event to look at" and "normal completion" cannot be told apart in this channel. Do not raise it for that reason.
        // ★This is **an intentional departure from the logging convention's level table**★ (`docs/reference/logging-conventions.md` — normal lifecycle event = info). What its §15 forbids is raising the level of a normal notice on the ground that
        //   "the default filter hides it", and the reason above is not that but **a record of attribution that cannot be recovered**. Writing that exception into that document has not been done yet (open — out of this round's scope).
        match &reply.outcome {
            Ok(_) => tracing::info!(
                entrance = ENTRANCE_CONTROL,
                command = %logged,
                %request_id,
                "relayed the control route command and received the outcome"
            ),
            Err(failed) => tracing::warn!(
                entrance = ENTRANCE_CONTROL,
                command = %logged,
                %request_id,
                code = %failed.code(),
                "the control route command ended in failure"
            ),
        }
        reply
    }
}

/// The seat one relay holds — ★`Drop` **always** undoes the count★ (regardless of cancellation · early return · panic).
/// The same discipline as the sibling [`LocalGuard`] and the only difference is the axis counted (that one is its own body out on the blocking pool).
struct RelaySlot(Arc<AtomicUsize>);

impl RelaySlot {
    /// Within the cap it raises the count and issues a marker. `None` = hit the cap (nothing is run).
    /// ★Why `fetch_update`★: splitting "read → compare → raise" lets two requests read the same value and both pass. Counting **exactly** fits the reason this value lives better than leaking one over the cap.
    /// ★The refusal is logged★ — hitting the global cap is an operational event and without it the only witness is the refused caller (logging convention "no silent swallow"). Unlike the sibling there is no time suppression because
    /// this door's arrival rate is **a connection per request** and much lower — if it gets noisy bring the sibling's suppression then.
    fn admit(counter: &Arc<AtomicUsize>, name: &str, request_id: RequestId) -> Option<Self> {
        counter
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |held| {
                (held < MAX_RELAYED_IN_FLIGHT).then_some(held + 1)
            })
            .map(|_| Self(Arc::clone(counter)))
            .map_err(|held| {
                tracing::warn!(
                    entrance = ENTRANCE_CONTROL,
                    command = %sanitize_for_log(name),
                    held,
                    ceiling = MAX_RELAYED_IN_FLIGHT,
                    %request_id,
                    "the control route relay's concurrent cap — refusing all of them until it drops below this"
                );
            })
            .ok()
    }
}

impl Drop for RelaySlot {
    fn drop(&mut self) {
        // The reason for saturating is the same as the sibling — a count wrapping negative and leaving the cap open forever is worse.
        let _ = self
            .0
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |held| {
                Some(held.saturating_sub(1))
            });
    }
}

/// The life of one fake caller connection — ★`Drop` **always** reaps from **both tables**★ (the roster entry + the seat of the correlation table).
///
/// ★Do not revert to straight-line code★: there is no other path to reap these items. A real connection has one disconnect hook that cleans the two tables together (`on_disconnect` of `agent_conn` — the line after `detach` is `drop_origin`) but a fake connection does not
/// reach that hook. And panic is not the only way to skip this cleanup — an HTTP handler future is **cancelled** when the client drops.
///
/// ★Why **both axes** must be held together★: cancellation occurs in the middle of an `await` of [`deliver`] so that function's seat disposal ([`CommandDeliveries::release`]) runs not a single line. The leftover seat does not vanish by the deadline either —
/// [`CommandDeliveries::expire`] turns `Awaiting` into [`SeatState::Void`] and **leaves the item**. So if only the roster axis is reaped, the copy of the envelope's name · arguments ([`Pending::args`]) sits for the whole process lifetime, and one seat prices **every**
/// entrance of both ([`CommandDeliveries::open`] calls `expire` on every delivery). Besides, a seat cancelled after the owner already answered stays as `Fulfilled` and a re-query of that number permanently receives a [`Seat::Taken`]
/// that names the **dead fake connection** as owner.
struct CallerSeat {
    roster: CommandRoster,
    /// ★It sits in the same destructor as the roster — do not make a separate cleanup place★: split, a branch where only one side runs arises, and the symptom of that branch is the residency leak above (no signal at runtime).
    deliveries: CommandDeliveries,
    conn: ConnId,
}

impl CallerSeat {
    fn attach(
        roster: CommandRoster,
        deliveries: CommandDeliveries,
        conn: ConnId,
        frames: Arc<dyn FrameSink>,
    ) -> Self {
        roster.attach(conn, &frames);
        // The copy is held by the roster — one more is not held beyond a round trip (module header · ADR-0154).
        drop(frames);
        Self {
            roster,
            deliveries,
            conn,
        }
    }
}

impl Drop for CallerSeat {
    /// ★The order is the same as a real connection's disconnect cleanup★ (roster → correlation table) — so the two cleanups do not diverge when read, and what that order guards there (the state where a seat is released and tries to reply but the roster still has a dead outlet)
    /// **cannot occur here**: [`CommandBus::invoke`] waits for [`deliver`] right there so by the time this destructor runs that delivery has already ended or been dropped together.
    /// ★The locks do not overlap★ (ADR-0006) — the two lines each lock and release inside.
    /// ★**Both** lines are cleanup locks so there is no branch where the first line dies and the later one is skipped★ — the basis · range are in that paragraph of the module header (and the doc of the two `lock_for_cleanup`). Changing either to a panicking lock makes the "reaps together" written here false at once.
    fn drop(&mut self) {
        self.roster.detach(self.conn);
        // ★The return value is not counted★: in a normal round trip [`deliver`] already released the seat so the count here is 0, and the only way to 1 is cancellation — that event is observed not at this layer but at the HTTP layer (the caller dropped).
        self.deliveries.drop_origin(self.conn);
    }
}

/// The marker holding one expiry sweeper — ★the assembly must call [`BusSweeper::stop`] at shutdown for shutdown to be bounded★.
/// If not called the delivery tasks have no occasion to wake until the deadline and meanwhile live longer than the moment they are reported "ended" (basis = [`CommandDeliveries::drain`]).
/// ★Even simply dropping it empties and closes the table★ — when the sending end of the stop signal vanishes the sweeper reads that as shutdown ([`CommandDeliveries::spawn_sweeper`]). But since it **does not wait** there is no guarantee shutdown is bounded, and
/// every later round trip of that bus is refused as [`Seat::Closed`].
pub struct BusSweeper {
    stop: watch::Sender<bool>,
    task: tokio::task::JoinHandle<()>,
}

impl BusSweeper {
    pub async fn stop(self) {
        let _ = self.stop.send(true);
        if let Err(e) = self.task.await {
            tracing::warn!("failed to wait for the command round trip sweeper to stop: {e}");
        }
    }
}

/// The frame outlet that receives one reply and hands it to [`CommandBus::invoke`].
/// ★Why the frame is read back★: delivery ends with sending the reply out through **the frame outlet** (`send_reply`), and that outlet is the only place this layer can plug in. Adding one more typed shortcut here would make two paths for sending the reply, and
/// the day they diverge only one gets fixed.
struct ReplyCatcher {
    /// Receives only one — a second frame is returned as a [`FrameError`].
    seat: Mutex<Option<oneshot::Sender<CommandReply>>>,
    /// The number of the round trip this outlet waits for — a reply with another number is **not the answer to this request**.
    /// ★Why it compares★: this outlet sees only one request but **only this comparison guarantees it**. If an accident overwrites a roster entry (the same connection number twice — `CommandRoster::attach` writes an error! and overwrites) or the delivery
    /// mis-correlates, an outlet with no comparison would carry another's outcome in the HTTP response **as this call's result**.
    expected: RequestId,
}

impl ReplyCatcher {
    fn new(seat: oneshot::Sender<CommandReply>, expected: RequestId) -> Self {
        Self {
            seat: Mutex::new(Some(seat)),
            expected,
        }
    }
}

impl FrameSink for ReplyCatcher {
    /// ★The log is printed **outside** the lock★ — the file sink is a synchronous write (logging convention "infrastructure") so calling it inside the critical section would lock this outlet for that IO. The roster's attach/detach also took their logs outside for the same reason.
    fn try_send(&self, frame: Frame) -> Result<(), FrameError> {
        let Frame::Text(text) = frame else {
            tracing::error!(
                entrance = ENTRANCE_CONTROL,
                "a non-text frame came to the control route round trip"
            );
            return Err(FrameError);
        };
        let reply = match serde_json::from_str::<AgentEvent>(&text) {
            // The reply coming to this outlet is one this daemon wrapped with `send_reply` so it always unpacks — if it does not that is a defect on our side.
            Ok(AgentEvent::CommandReply { reply }) => match reply_from_wire(&reply) {
                Ok(reply) => reply,
                Err(e) => {
                    tracing::error!(
                        entrance = ENTRANCE_CONTROL,
                        "cannot unpack the control route round trip's reply — the reply this daemon wrapped is broken: {e}"
                    );
                    return Err(FrameError);
                }
            },
            _ => {
                tracing::error!(
                    entrance = ENTRANCE_CONTROL,
                    "an event that is not a reply came to the control route round trip — only that round trip's reply comes to this outlet"
                );
                return Err(FrameError);
            }
        };
        if reply.request_id != self.expected {
            tracing::error!(
                entrance = ENTRANCE_CONTROL,
                expected = %self.expected,
                got = %reply.request_id,
                "a reply with another's number came to the control route round trip — it is not this request's result"
            );
            return Err(FrameError);
        }
        let taken = self.seat.lock().expect("reply catcher poisoned").take();
        let Some(seat) = taken else {
            tracing::error!(
                entrance = ENTRANCE_CONTROL,
                request_id = %reply.request_id,
                "the reply came twice to the control route round trip — one reply per query is broken"
            );
            return Err(FrameError);
        };
        // The receiving side vanished = the request already ended. The caller must know there is nowhere to send.
        seat.send(reply).map_err(|_| FrameError)
    }

    fn send(&self, frame: Frame) -> futures_util::future::BoxFuture<'_, Result<(), FrameError>> {
        Box::pin(std::future::ready(self.try_send(frame)))
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::conn_port::FrameSink;
    use crate::test_doubles::FakeFrameSink;
    use engram_dashboard_base::time::ManualClock;
    use engram_dashboard_command::{CommandDecl, OwnerToken};
    use serde_json::json;
    use tokio::sync::mpsc;

    /// ★Use the production deadline as is — the harness does not pick its own value★: it used to put 10 seconds here, and that value was a relation `fits_caller_silence_window` forbids so **the whole suite ran with that relation violated**. The tests that push the time
    /// by hand must not be tied to how many seconds the deadline is (the `advance`s below derive from this value) so there is no reason to pick a value here either.
    pub(crate) fn deliveries_with(clock: Arc<ManualClock>) -> CommandDeliveries {
        CommandDeliveries::with_clock(clock, CommandDeliveries::DEFAULT_DEADLINE)
    }

    /// The default wiring measuring the delivery rules — **a daemon with no commands of its own** (stage 1 is always a miss).
    /// ★That it is a name that shadows [`super::deliver`] is intended★: most of the tests below measure stages 2·3 (delivery · owner absent) so if stage 1 gets in what is measured blurs. A test that rides stage 1 calls `super::deliver` directly.
    fn deliver(
        roster: CommandRoster,
        deliveries: CommandDeliveries,
        origin: ConnId,
        envelope: CommandEnvelope,
    ) -> impl Future<Output = ()> {
        super::deliver(
            roster,
            deliveries,
            Arc::new(NoLocalCommands),
            origin,
            envelope,
            ENTRANCE_SOCKET,
        )
    }

    fn sink_with_inbox() -> (Arc<dyn FrameSink>, mpsc::Receiver<Frame>) {
        sink_with_capacity(16)
    }

    /// An outlet made with a given capacity — set small it can make **saturation** (a state where nothing can be put in).
    fn sink_with_capacity(capacity: usize) -> (Arc<dyn FrameSink>, mpsc::Receiver<Frame>) {
        let (tx, rx) = mpsc::channel::<Frame>(capacity);
        (Arc::new(FakeFrameSink::new(tx)), rx)
    }

    use crate::log_capture::capture_loud_async as capture_loud;

    fn envelope(name: &str, owner: &OwnerToken) -> CommandEnvelope {
        CommandEnvelope {
            name: name.to_string(),
            request_id: RequestId::new(),
            owner: owner.clone(),
            proto_ver: 7,
            args: json!({ "window": "main" }),
        }
    }

    /// Reads one frame as an event.
    fn event_from(frame: Frame) -> AgentEvent {
        match frame {
            Frame::Text(text) => serde_json::from_str(&text).expect("event decode"),
            other => panic!("expected Text: {other:?}"),
        }
    }

    /// Waits for the end of a delivery task — the basis of the cap is the same as [`next_event`].
    async fn finished(handle: tokio::task::JoinHandle<()>) {
        tokio::time::timeout(Duration::from_secs(10), handle)
            .await
            .expect("the delivery task does not end")
            .expect("the delivery task");
    }

    /// ★Put a cap on the wait — if delivery breaks it must be **a failure, not a hang**★
    /// What these tests measure is "does that frame come to that connection", so in an implementation where the reply leaks to a wrong connection this never wakes. An unbounded wait looks **as if stuck** on the runner and does not say which assertion was wrong.
    /// ★The cap is not a criterion★ — the normal path wakes at once so this value is only a means of observing "it is broken" and is set generously so a slow runner gives no false positive.
    async fn next_event(rx: &mut mpsc::Receiver<Frame>) -> AgentEvent {
        let frame = tokio::time::timeout(Duration::from_secs(10), rx.recv())
            .await
            .expect("exceeded the cap waiting for a frame — the delivery does not come to this connection")
            .expect("one frame");
        event_from(frame)
    }

    /// A roster with one owner attached holding one name + the place that receives the frames sent to that owner.
    fn roster_with_owner(conn_id: ConnId, name: &str) -> (CommandRoster, mpsc::Receiver<Frame>) {
        let roster = CommandRoster::new();
        let (sink, inbox) = sink_with_inbox();
        roster.attach(conn_id, &sink);
        roster
            .register(
                conn_id,
                vec![CommandDecl {
                    name: name.to_string(),
                    help: "{}".to_string(),
                    agent_args: Vec::new(),
                }],
            )
            .expect("registration");
        (roster, inbox)
    }

    /// Puts the asking connection on the roster — the reply goes out through that connection's outlet.
    fn attach_caller(roster: &CommandRoster, conn_id: ConnId) -> mpsc::Receiver<Frame> {
        let (sink, inbox) = sink_with_inbox();
        roster.attach(conn_id, &sink);
        inbox
    }

    // ── branch ⓪ my own table (stage 1) ──────────────────────────────────────────

    /// A fake stage 1 table — ★with neither a manager nor a real table★ so this branch can be measured (the reason the port exists = [`LocalCommands`]).
    struct FakeLocal {
        /// The name it answers as held.
        name: &'static str,
        /// The outcome to give. `None` = "my name but empty-handed" (the branch that falls to the next stage).
        answer: Option<Result<serde_json::Value, CommandError>>,
        /// The thread the body ran on — measures whether it was moved to the blocking pool.
        ran_on: Mutex<Option<std::thread::ThreadId>>,
        /// The arguments the body received — to see whether the envelope arrives as is.
        saw: Mutex<Option<serde_json::Value>>,
        /// The **entrance label** the body received — the means to measure that delivery passes its own entrance as is.
        saw_entrance: Mutex<Option<&'static str>>,
        /// The caller the body received — to see whether delivery passes the connection that sent the envelope as material for the lease decision.
        saw_caller: Mutex<Option<Option<ConnId>>>,
        /// **How many times** the body ran — the means to observe "the same Write verb is not applied twice".
        runs: std::sync::atomic::AtomicUsize,
        /// A signal that the body has entered — lets a test aim deterministically at "while the body runs".
        entered: Mutex<Option<mpsc::UnboundedSender<()>>>,
        /// A door holding the body — it does not end until the test opens it.
        gate: Mutex<Option<std::sync::mpsc::Receiver<()>>>,
        /// Blows up **after** the door opens — makes "a body past the deadline dies with no answer" deterministic.
        panic_after_gate: bool,
        /// The effect of this name — separates whether it holds the number (Write) or not (Read) ([`retains_the_id`]).
        effect: Effect,
    }

    impl FakeLocal {
        fn answering(name: &'static str, answer: Result<serde_json::Value, CommandError>) -> Self {
            Self {
                name,
                answer: Some(answer),
                ran_on: Mutex::new(None),
                saw: Mutex::new(None),
                saw_entrance: Mutex::new(None),
                saw_caller: Mutex::new(None),
                runs: std::sync::atomic::AtomicUsize::new(0),
                entered: Mutex::new(None),
                gate: Mutex::new(None),
                panic_after_gate: false,
                effect: Effect::Write,
            }
        }

        fn empty_handed(name: &'static str) -> Self {
            Self {
                answer: None,
                ..Self::answering(name, Ok(serde_json::Value::Null))
            }
        }

        fn runs(&self) -> usize {
            self.runs.load(std::sync::atomic::Ordering::Relaxed)
        }

        fn entrance(&self) -> Option<&'static str> {
            *self.saw_entrance.lock().expect("fake local poisoned")
        }

        fn caller(&self) -> Option<Option<ConnId>> {
            *self.saw_caller.lock().expect("fake local poisoned")
        }
    }

    /// A stage 1 table with a door — the test holds the state where the body is **running** (the deadline · re-query · cap diverge only in that state).
    /// Returns: the table · the handle that opens the door · the signal that the body entered.
    fn parked_local(
        name: &'static str,
    ) -> (
        Arc<FakeLocal>,
        std::sync::mpsc::Sender<()>,
        mpsc::UnboundedReceiver<()>,
    ) {
        let (open, gate) = std::sync::mpsc::channel::<()>();
        let (entered_tx, entered_rx) = mpsc::unbounded_channel::<()>();
        let local = FakeLocal {
            entered: Mutex::new(Some(entered_tx)),
            gate: Mutex::new(Some(gate)),
            ..FakeLocal::answering(name, Ok(json!({ "ok": true })))
        };
        (Arc::new(local), open, entered_rx)
    }

    /// A stage 1 table that **dies** after the door opens — makes deterministic the branch where a body past the deadline ends with no answer.
    fn parked_dying_local(
        name: &'static str,
    ) -> (
        Arc<FakeLocal>,
        std::sync::mpsc::Sender<()>,
        mpsc::UnboundedReceiver<()>,
    ) {
        let (local, open, entered) = parked_local(name);
        let local = Arc::new(FakeLocal {
            entered: Mutex::new(local.entered.lock().unwrap().take()),
            gate: Mutex::new(local.gate.lock().unwrap().take()),
            panic_after_gate: true,
            ..FakeLocal::answering(name, Ok(json!({ "ok": true })))
        });
        (local, open, entered)
    }

    impl LocalCommands for Arc<FakeLocal> {
        fn claim(&self, name: &str) -> Option<Effect> {
            (name == self.name).then_some(self.effect)
        }

        fn run(
            &self,
            _name: &str,
            args: &mut serde_json::Value,
            entrance: &'static str,
            caller: Option<ConnId>,
        ) -> Option<Result<serde_json::Value, CommandError>> {
            *self.saw_entrance.lock().expect("fake local poisoned") = Some(entrance);
            *self.saw_caller.lock().expect("fake local poisoned") = Some(caller);
            self.runs.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            *self.ran_on.lock().expect("fake local poisoned") = Some(std::thread::current().id());
            *self.saw.lock().expect("fake local poisoned") = Some(args.clone());
            if let Some(entered) = self.entered.lock().expect("fake local poisoned").as_ref() {
                let _ = entered.send(());
            }
            // ★The door is waited on **outside** the lock★ — waiting inside would block the second body on the lock and "ran twice" would show up not as a failure but as a **hang**. Taking the door out lets the second simply pass and be caught in the count.
            let gate = self.gate.lock().expect("fake local poisoned").take();
            if let Some(gate) = gate {
                let _ = gate.recv();
            }
            assert!(!self.panic_after_gate, "the body died past its deadline");
            self.answer.clone()
        }

        fn decls(&self) -> Vec<CommandDecl> {
            Vec::new()
        }

        fn agent_roster(&self) -> Vec<AgentRosterRow> {
            Vec::new()
        }
    }

    /// ★If my own table has it, that name **ends here**★ — no envelope goes out even if an owner that put on the same name is attached. This priority follows the stage order of [`route`] (my table → roster) as is, and the discovery list must
    /// answer in the same order (that test in `connection_core`) — if the two diverge the caller fits arguments by another's help and gets refused by the daemon handler.
    #[tokio::test]
    async fn a_name_my_own_table_holds_is_answered_here_and_never_reaches_the_roster() {
        let (roster, mut owner_inbox) = roster_with_owner(1, "agent.list");
        let mut caller_inbox = attach_caller(&roster, 2);
        let deliveries = deliveries_with(Arc::new(ManualClock::new()));
        let local = Arc::new(FakeLocal::answering(
            "agent.list",
            Ok(json!({ "agents": [] })),
        ));
        let env = envelope("agent.list", &OwnerToken::new("whatever-the-caller-thinks"));
        let request_id = env.request_id;

        finished(tokio::spawn(super::deliver(
            roster.clone(),
            deliveries.clone(),
            Arc::new(Arc::clone(&local)),
            2,
            env,
            ENTRANCE_SOCKET,
        )))
        .await;

        match next_event(&mut caller_inbox).await {
            AgentEvent::CommandReply { reply } => {
                let reply = opened_reply(&reply);
                assert_eq!(reply.request_id, request_id);
                assert_eq!(reply.outcome, Ok(json!({ "agents": [] })));
            }
            other => panic!("a reply must come: {other:?}"),
        }
        assert!(
            owner_inbox.try_recv().is_err(),
            "if my table answered no envelope goes anywhere (even if an owner put on the same name)"
        );
        assert_eq!(
            *local.saw.lock().unwrap(),
            Some(json!({ "window": "main" })),
            "the envelope's arguments arrive as is"
        );
        // An applied Write **holds the seat without releasing** — if it comes again with that number it is not run ([`SeatState::Retained`]).
        assert_eq!(
            deliveries.in_flight(),
            1,
            "the seat keeps holding an applied number"
        );
    }

    /// ★The body runs **somewhere other than a runtime worker**★ — this table's handlers write disk holding the profile lock and poll early exit for seconds (agent `make_table` doc), so running as is on the delivery task blocks one worker that whole time
    /// and another connection on the same worker stalls with it.
    /// ★Being a current-thread runtime is what makes this comparison hold★: the delivery future runs on this test thread and the `spawn_blocking` body by definition runs on a blocking-pool thread — if the two ids are equal the offload is gone.
    #[tokio::test]
    async fn my_own_table_runs_off_the_delivery_task() {
        let roster = CommandRoster::new();
        let mut caller_inbox = attach_caller(&roster, 2);
        let local = Arc::new(FakeLocal::answering("agent.list", Ok(json!({}))));

        super::deliver(
            roster.clone(),
            deliveries_with(Arc::new(ManualClock::new())),
            Arc::new(Arc::clone(&local)),
            2,
            envelope("agent.list", &OwnerToken::new("x")),
            ENTRANCE_SOCKET,
        )
        .await;

        let _ = next_event(&mut caller_inbox).await;
        let ran_on = local
            .ran_on
            .lock()
            .unwrap()
            .expect("the body must have run");
        assert_ne!(
            ran_on,
            std::thread::current().id(),
            "running my table as is on the delivery task blocks that worker"
        );
    }

    /// ★Claiming it as its own name yet returning empty-handed is **surfaced** — it does not flow to the next stage★
    /// The routing authority is [`LocalCommands::claim`] alone. If that answers "mine" the seat is claiming "delegated to no one" ([`Pending::local`]), so sending the envelope to the owner in that state would ① have the owner's legitimate outcome refused with
    /// [`OutcomeLanding::NotDelegated`] and ② leave the seat uncleaned by disconnect cleanup. Instead of a quiet mismatch it answers `INTERNAL` and leaves it on the server.
    #[test]
    fn a_local_table_that_claims_a_name_but_answers_nothing_is_surfaced() {
        let (roster, mut owner_inbox) = roster_with_owner(1, "agent.list");
        let mut caller_inbox = attach_caller(&roster, 2);
        let deliveries = deliveries_with(Arc::new(ManualClock::new()));
        let empty_handed = Arc::new(FakeLocal::empty_handed("agent.list"));

        let logged = capture_loud(async {
            super::deliver(
                roster.clone(),
                deliveries.clone(),
                Arc::new(Arc::clone(&empty_handed)),
                2,
                envelope("agent.list", &OwnerToken::new("whatever")),
                ENTRANCE_SOCKET,
            )
            .await;
        });

        match event_from(caller_inbox.try_recv().expect("one reply")) {
            AgentEvent::CommandReply { reply } => {
                let reply = opened_reply(&reply);
                let err = reply.outcome.expect_err("contract violation");
                assert_eq!(err.code(), ErrorCode::Internal);
            }
            other => panic!("a reply must come: {other:?}"),
        }
        assert!(
            owner_inbox.try_recv().is_err(),
            "claiming mine then sending to another makes that seat refuse the owner's outcome"
        );
        assert!(
            logged.iter().any(|line| line.contains("agent.list")),
            "the incident where authority and execution diverged must also remain on the server: {logged:?}"
        );
        assert_eq!(deliveries.in_flight(), 0, "nothing ran so it is not held");
    }

    /// ★A failure of my own table is also **that round trip's one reply**★ — the code goes as the table built it and only the retry instruction is issued by this hop
    /// ([`no_retry`] · [`send_reply`]).
    #[tokio::test]
    async fn a_failure_from_my_own_table_travels_as_the_reply() {
        let roster = CommandRoster::new();
        let mut caller_inbox = attach_caller(&roster, 2);
        let local = Arc::new(FakeLocal::answering(
            "agent.rename",
            Err(CommandError::not_found("no agent called 'ghost'")),
        ));

        super::deliver(
            roster.clone(),
            deliveries_with(Arc::new(ManualClock::new())),
            Arc::new(Arc::clone(&local)),
            2,
            envelope("agent.rename", &OwnerToken::new("x")),
            ENTRANCE_SOCKET,
        )
        .await;

        match next_event(&mut caller_inbox).await {
            AgentEvent::CommandReply { reply } => {
                let reply = opened_reply(&reply);
                let err = reply.outcome.expect_err("failure reply");
                assert_eq!(err.code(), ErrorCode::NotFound);
                assert!(err.message().contains("ghost"), "{}", err.message());
                assert_eq!(
                    err.retry(),
                    RetryMode::Never,
                    "the instruction is issued by this hop"
                );
            }
            other => panic!("a reply must come: {other:?}"),
        }
    }

    /// ★The deadline must **stay alive** in stage 1 too★ — even if the body passes the deadline the caller gets a bounded answer.
    /// The regression that occurs without this test: if delivery waits only for the body and **does not look at the seat**, even when the sweeper puts `TIMEOUT` in the seat nobody takes it out and makes it a frame so **not one goes out**. Yet the sweeper log
    /// writes "answered TIMEOUT" so by the server record alone it looks as if it answered (a log writing what was not done).
    #[test]
    fn a_local_command_that_outlives_its_deadline_still_answers_the_caller() {
        // It hits the post-deadline success warn place so it runs inside capture (`log_capture` "Observation conditions" ①).
        let _logged = capture_loud(async {
            let roster = CommandRoster::new();
            let mut caller_inbox = attach_caller(&roster, 2);
            let clock = Arc::new(ManualClock::new());
            let deliveries = deliveries_with(clock.clone());
            let (local, open, mut entered) = parked_local("agent.new");

            let round_trip = tokio::spawn(super::deliver(
                roster.clone(),
                deliveries.clone(),
                Arc::new(Arc::clone(&local)),
                2,
                envelope("agent.new", &OwnerToken::new("whatever")),
                ENTRANCE_SOCKET,
            ));
            // Premise: the body must be **running** for this test to measure that state.
            entered.recv().await.expect("the body entered");

            clock.advance(Duration::from_secs(11));
            assert_eq!(deliveries.expire(), 1, "the deadline reaps that seat");

            match next_event(&mut caller_inbox).await {
                AgentEvent::CommandReply { reply } => {
                    let reply = opened_reply(&reply);
                    let err = reply.outcome.expect_err("the deadline is a failure reply");
                    assert_eq!(err.code(), ErrorCode::Timeout);
                    assert_eq!(err.retry(), RetryMode::Never);
                }
                other => panic!("the deadline must go out as one reply too: {other:?}"),
            }

            // Opening the door ends the body — an applied Write so the seat is not released but passes to **holding**.
            open.send(()).expect("open the door");
            finished(round_trip).await;
            assert_eq!(local.runs(), 1);
            assert_eq!(
                deliveries.in_flight(),
                1,
                "the seat keeps holding an applied number"
            );
            // When the hold window passes the sweeper releases it — if it did not that number would stay blocked forever.
            clock.advance(Duration::from_secs(11));
            assert_eq!(deliveries.expire(), 0);
            assert_eq!(
                deliveries.in_flight(),
                0,
                "when the window passes the seat is released"
            );
        });
    }

    /// ★An outcome another sent **cannot become the answer of a command I answer**★
    /// This round trip's envelope went to no one so a `CommandOutcome` arriving with that number is by definition not a legitimate answer. Accepting it would send a payload another made up out as the reply (the caller reads it as its own command's result) and quietly discard the
    /// handler's real result — **even inside the auth boundary one connection must not be able to decide another connection's command result**.
    #[tokio::test]
    async fn a_wire_outcome_cannot_answer_a_command_this_daemon_answers_itself() {
        let roster = CommandRoster::new();
        let mut caller_inbox = attach_caller(&roster, 2);
        let deliveries = deliveries_with(Arc::new(ManualClock::new()));
        let (local, open, mut entered) = parked_local("agent.new");
        let env = envelope("agent.new", &OwnerToken::new("whatever"));
        let request_id = env.request_id;

        let round_trip = tokio::spawn(super::deliver(
            roster.clone(),
            deliveries.clone(),
            Arc::new(Arc::clone(&local)),
            2,
            env,
            ENTRANCE_SOCKET,
        ));
        entered.recv().await.expect("the body entered");

        assert_eq!(
            deliveries.complete(CommandReply::ok(
                request_id,
                json!({ "agent_id": "forged", "state": "sleeping" })
            )),
            OutcomeLanding::NotDelegated,
            "the outcome of a round trip that was never delegated is refused"
        );

        open.send(()).expect("open the door");
        finished(round_trip).await;

        match next_event(&mut caller_inbox).await {
            AgentEvent::CommandReply { reply } => {
                let reply = opened_reply(&reply);
                assert_eq!(
                    reply.outcome,
                    Ok(json!({ "ok": true })),
                    "the handler's real result must go out"
                );
            }
            other => panic!("a reply must come: {other:?}"),
        }
        assert!(caller_inbox.try_recv().is_err(), "one frame per query");
    }

    /// ★Even if asked again with the same number after the deadline, **the same Write verb is not run twice**★
    /// The contract does not forbid a re-query of the same number ([`Pending::token`]) so this code must tolerate it. The body cannot be cancelled (blocking pool) and still runs after the deadline, and overturning the seat then would create two agents
    /// and send two frames under one number. Holding the seat is the means that blocks both together.
    #[test]
    fn a_resend_after_the_deadline_does_not_run_the_same_local_command_twice() {
        // It hits the post-deadline success warn place so it runs inside capture (`log_capture` "Observation conditions" ①).
        let _logged = capture_loud(async {
            let roster = CommandRoster::new();
            let mut caller_inbox = attach_caller(&roster, 2);
            let clock = Arc::new(ManualClock::new());
            let deliveries = deliveries_with(clock.clone());
            let (local, open, mut entered) = parked_local("agent.new");
            let env = envelope("agent.new", &OwnerToken::new("whatever"));
            let request_id = env.request_id;
            let args = env.args.clone();

            let round_trip = tokio::spawn(super::deliver(
                roster.clone(),
                deliveries.clone(),
                Arc::new(Arc::clone(&local)),
                2,
                env,
                ENTRANCE_SOCKET,
            ));
            entered.recv().await.expect("the body entered");
            clock.advance(Duration::from_secs(11));
            assert_eq!(deliveries.expire(), 1);
            let timed_out = next_event(&mut caller_inbox).await;
            assert!(
                matches!(timed_out, AgentEvent::CommandReply { .. }),
                "premise: the deadline reply went out first"
            );

            // Re-query with the same number per the convention — the body is still running.
            super::deliver(
                roster.clone(),
                deliveries.clone(),
                Arc::new(Arc::clone(&local)),
                2,
                CommandEnvelope {
                    name: "agent.new".to_string(),
                    request_id,
                    owner: OwnerToken::new("whatever"),
                    proto_ver: 7,
                    args,
                },
                ENTRANCE_SOCKET,
            )
            .await;

            match next_event(&mut caller_inbox).await {
                AgentEvent::CommandReply { reply } => {
                    let reply = opened_reply(&reply);
                    let err = reply.outcome.expect_err("it is not run again");
                    assert_eq!(err.code(), ErrorCode::OutcomeUnknown);
                    assert!(
                        err.message().contains("still running"),
                        "must say what is happening: {}",
                        err.message()
                    );
                }
                other => panic!("the re-query must get an answer too: {other:?}"),
            }
            assert_eq!(
                local.runs(),
                1,
                "if the re-query ran the body once more two agents would come up"
            );

            open.send(()).expect("open the door");
            finished(round_trip).await;
            assert_eq!(local.runs(), 1);
        });
    }

    /// ★A re-query that comes **after the body ended** is not run again either★ — because the seat keeps holding that number.
    /// An order that needs no race: deadline → `TIMEOUT` → body completes → re-query. Releasing the seat the moment it completes would let that re-query see an empty table and **make one more agent** (there is no delete verb — ADR-0122).
    /// ★That **only one seat** holds the number is the other half of this test★ — with a separate memory table the number would remain regardless of whether it ran or its effect, and every branch the neighboring tests below measure would go off.
    #[test]
    fn a_resend_after_the_body_finished_is_refused_by_the_retained_seat() {
        // It hits the post-deadline success warn place so it runs inside capture (`log_capture` "Observation conditions" ①).
        let _logged = capture_loud(async {
            let roster = CommandRoster::new();
            let mut caller_inbox = attach_caller(&roster, 2);
            let clock = Arc::new(ManualClock::new());
            let deliveries = deliveries_with(clock.clone());
            let (local, open, mut entered) = parked_local("agent.new");
            let env = envelope("agent.new", &OwnerToken::new("whatever"));
            let request_id = env.request_id;
            let args = env.args.clone();
            let again = || CommandEnvelope {
                name: "agent.new".to_string(),
                request_id,
                owner: OwnerToken::new("whatever"),
                proto_ver: 7,
                args: args.clone(),
            };

            let round_trip = tokio::spawn(super::deliver(
                roster.clone(),
                deliveries.clone(),
                Arc::new(Arc::clone(&local)),
                2,
                env,
                ENTRANCE_SOCKET,
            ));
            entered.recv().await.expect("the body entered");
            clock.advance(Duration::from_secs(11));
            assert_eq!(deliveries.expire(), 1);
            let _timed_out = next_event(&mut caller_inbox).await;

            // The body ends — the seat is **not released** and passes to holding.
            open.send(()).expect("open the door");
            finished(round_trip).await;
            assert_eq!(
                deliveries.in_flight(),
                1,
                "premise: an applied number is still held by the seat"
            );

            super::deliver(
                roster.clone(),
                deliveries.clone(),
                Arc::new(Arc::clone(&local)),
                2,
                again(),
                ENTRANCE_SOCKET,
            )
            .await;

            match next_event(&mut caller_inbox).await {
                AgentEvent::CommandReply { reply } => {
                    let reply = opened_reply(&reply);
                    let err = reply.outcome.expect_err("it is not run again");
                    assert_eq!(err.code(), ErrorCode::AlreadyApplied);
                    assert_eq!(err.retry(), RetryMode::Never);
                }
                other => panic!("the re-query must get an answer too: {other:?}"),
            }
            assert_eq!(local.runs(), 1, "there must be no second execution");

            // ★The hold is bounded — when the window passes the sweeper releases the seat★ (a known residue: a re-query after that is a new request).
            clock.advance(Duration::from_secs(11));
            assert_eq!(
                deliveries.expire(),
                0,
                "a hold expiry is not a 'round trip that got no answer'"
            );
            assert_eq!(
                deliveries.in_flight(),
                0,
                "when the window passes the seat is released"
            );
        });
    }

    /// ★A disconnect does **not end** the hold — it is rather the event where the hold is most needed★
    /// The very moment the socket drops is when the caller does not know "did my command arrive". Reattaching and asking again with the same number is the next step (the sibling [`Seat::Taken`] branch already wrote that scenario), and with no seat then the same Write verb is applied twice — there is no verb to delete it (ADR-0122).
    /// ★The life of the hold is tied to the **number**, not to the connection★ — so cleanup does not touch it, and when the window passes the sweeper releases it.
    #[tokio::test]
    async fn cleanup_leaves_a_retained_seat_for_the_reconnecting_caller() {
        let roster = CommandRoster::new();
        let mut caller_inbox = attach_caller(&roster, 2);
        let clock = Arc::new(ManualClock::new());
        let deliveries = deliveries_with(clock.clone());
        let local = Arc::new(FakeLocal::answering("agent.new", Ok(json!({ "ok": true }))));

        super::deliver(
            roster.clone(),
            deliveries.clone(),
            Arc::new(Arc::clone(&local)),
            2,
            envelope("agent.new", &OwnerToken::new("whatever")),
            ENTRANCE_SOCKET,
        )
        .await;
        let _ = next_event(&mut caller_inbox).await;
        assert_eq!(
            deliveries.in_flight(),
            1,
            "premise: the applied number is held"
        );

        // Exactly the production `on_disconnect` order.
        roster.detach(2);
        assert_eq!(
            deliveries.drop_origin(2),
            0,
            "if the held seat were reaped a reconnect re-query would run the same verb again"
        );
        assert_eq!(deliveries.in_flight(), 1);

        // It is released when the window passes — not permanent residency.
        clock.advance(Duration::from_secs(11));
        assert_eq!(deliveries.expire(), 0);
        assert_eq!(deliveries.in_flight(), 0);
    }

    /// ★The hold is closed **by count** too★ — by the window alone it grows to arrival rate × window (one fast-running client really makes that product). On overflow **the oldest are pushed out first** and a pushed-out number loses its protection.
    #[tokio::test]
    async fn retention_is_bounded_by_count_not_only_by_time() {
        let roster = CommandRoster::new();
        let mut caller_inbox = attach_caller(&roster, 2);
        let deliveries = deliveries_with(Arc::new(ManualClock::new()));
        let local = Arc::new(FakeLocal::answering(
            "agent.rename",
            Ok(json!({ "ok": true })),
        ));

        for _ in 0..(MAX_RETAINED_LOCAL + 8) {
            super::deliver(
                roster.clone(),
                deliveries.clone(),
                Arc::new(Arc::clone(&local)),
                2,
                envelope("agent.rename", &OwnerToken::new("whatever")),
                ENTRANCE_SOCKET,
            )
            .await;
            let _ = next_event(&mut caller_inbox).await;
        }

        assert_eq!(
            deliveries.in_flight(),
            MAX_RETAINED_LOCAL,
            "however many come inside the window the hold does not exceed the cap"
        );
    }

    /// ★A number that **did not run** is not held★ — this is the branch a separate memory table could not tell apart.
    /// The everyday order: a wrong field name gets rejected → fixed and resent **with the same number**. Remembering the rejection as "applied" would give that retry `ALREADY_APPLIED` and the caller would hear "read the current state" about a **command that never ran**.
    #[tokio::test]
    async fn a_rejected_local_command_does_not_hold_its_request_id() {
        let roster = CommandRoster::new();
        let mut caller_inbox = attach_caller(&roster, 2);
        let deliveries = deliveries_with(Arc::new(ManualClock::new()));
        let rejecting = Arc::new(FakeLocal::answering(
            "agent.new",
            Err(CommandError::invalid_argument("not an argument: 'cwdd'")),
        ));
        let env = envelope("agent.new", &OwnerToken::new("whatever"));
        let request_id = env.request_id;
        let args = env.args.clone();

        super::deliver(
            roster.clone(),
            deliveries.clone(),
            Arc::new(Arc::clone(&rejecting)),
            2,
            env,
            ENTRANCE_SOCKET,
        )
        .await;
        let refused = next_event(&mut caller_inbox).await;
        assert!(
            matches!(refused, AgentEvent::CommandReply { .. }),
            "premise: the rejection reply went out"
        );
        assert_eq!(
            deliveries.in_flight(),
            0,
            "a rejection does not hold the number"
        );

        // Fix the field and resend **with the same number** — this time it is a table that runs.
        let fixed = Arc::new(FakeLocal::answering("agent.new", Ok(json!({ "ok": true }))));
        super::deliver(
            roster.clone(),
            deliveries.clone(),
            Arc::new(Arc::clone(&fixed)),
            2,
            CommandEnvelope {
                name: "agent.new".to_string(),
                request_id,
                owner: OwnerToken::new("whatever"),
                proto_ver: 7,
                args,
            },
            ENTRANCE_SOCKET,
        )
        .await;

        match next_event(&mut caller_inbox).await {
            AgentEvent::CommandReply { reply } => {
                let reply = opened_reply(&reply);
                assert_eq!(
                    reply.outcome,
                    Ok(json!({ "ok": true })),
                    "the retry sent after fixing must actually run"
                );
            }
            other => panic!("a reply must come: {other:?}"),
        }
        assert_eq!(fixed.runs(), 1);
    }

    /// ★A failure that **says** "it may have been applied" is held★ — the substance of the decision standing as a list to release.
    /// `OUTCOME_UNKNOWN` is, in this repo, a code that states in its wording "part may already have been applied" (`control::commands::drive_to_completion`). Had the list to hold been counted by hand this code would have been missing from it and
    /// **leaked into re-execution** — only by inverting the list does such a code automatically stand on the safe side.
    #[tokio::test]
    async fn a_failure_that_may_have_applied_still_holds_its_request_id() {
        let roster = CommandRoster::new();
        let mut caller_inbox = attach_caller(&roster, 2);
        let deliveries = deliveries_with(Arc::new(ManualClock::new()));
        let unsure = Arc::new(FakeLocal::answering(
            "agent.new",
            Err(no_retry(
                ErrorCode::OutcomeUnknown,
                "did not finish on its first poll — part of it may already have been applied",
            )),
        ));

        super::deliver(
            roster.clone(),
            deliveries.clone(),
            Arc::new(Arc::clone(&unsure)),
            2,
            envelope("agent.new", &OwnerToken::new("whatever")),
            ENTRANCE_SOCKET,
        )
        .await;
        let _ = next_event(&mut caller_inbox).await;

        assert_eq!(
            deliveries.in_flight(),
            1,
            "releasing a number that may have been applied lets a re-query apply it once more"
        );
    }

    /// ★A read does not hold the number★ — holding it would let a common lookup block that number, and with a separate memory table it pushed out **even other numbers' holds** (traffic of lookups eating the count cap).
    #[tokio::test]
    async fn a_read_command_does_not_hold_its_request_id() {
        let roster = CommandRoster::new();
        let mut caller_inbox = attach_caller(&roster, 2);
        let deliveries = deliveries_with(Arc::new(ManualClock::new()));
        let reading = Arc::new(FakeLocal {
            effect: Effect::Read,
            ..FakeLocal::answering("agent.list", Ok(json!({ "agents": [] })))
        });

        for _ in 0..2 {
            super::deliver(
                roster.clone(),
                deliveries.clone(),
                Arc::new(Arc::clone(&reading)),
                2,
                envelope("agent.list", &OwnerToken::new("whatever")),
                ENTRANCE_SOCKET,
            )
            .await;
            let _ = next_event(&mut caller_inbox).await;
        }

        assert_eq!(deliveries.in_flight(), 0, "a read leaves no seat");
        assert_eq!(reading.runs(), 2, "a read runs every time");
    }

    /// ★An outcome that ended after the deadline is **not discarded silently**★
    /// The caller is sent `TIMEOUT` ("check whether it was applied") while this side holds that answer in hand and throws it away. Without a server record nobody can find the agent that was made — and there is no delete verb either (ADR-0122).
    #[test]
    fn a_body_that_finishes_after_its_deadline_is_recorded_not_dropped() {
        let roster = CommandRoster::new();
        let mut caller_inbox = attach_caller(&roster, 2);
        let clock = Arc::new(ManualClock::new());
        let deliveries = deliveries_with(clock.clone());
        let (local, open, mut entered) = parked_local("agent.new");

        let logged = capture_loud(async {
            let round_trip = tokio::spawn(super::deliver(
                roster.clone(),
                deliveries.clone(),
                Arc::new(Arc::clone(&local)),
                2,
                envelope("agent.new", &OwnerToken::new("whatever")),
                ENTRANCE_SOCKET,
            ));
            entered.recv().await.expect("the body entered");
            clock.advance(Duration::from_secs(11));
            assert_eq!(deliveries.expire(), 1);
            let _timed_out = next_event(&mut caller_inbox).await;
            open.send(()).expect("open the door");
            finished(round_trip).await;
        });

        assert!(
            logged.iter().any(|line| line.contains("\"ok\":true")),
            "the result produced after the deadline must remain on the server: {logged:?}"
        );
    }

    /// At the same place the record remains **even if the body dies** — had instrumentation been attached only to the before-deadline branch this side would go wholly silent.
    #[test]
    fn a_body_that_dies_after_its_deadline_is_logged_too() {
        let roster = CommandRoster::new();
        let mut caller_inbox = attach_caller(&roster, 2);
        let clock = Arc::new(ManualClock::new());
        let deliveries = deliveries_with(clock.clone());
        let (local, open, mut entered) = parked_dying_local("agent.new");

        let logged = engram_dashboard_command::testing::with_quiet_panic_hook(|| {
            capture_loud(async {
                let round_trip = tokio::spawn(super::deliver(
                    roster.clone(),
                    deliveries.clone(),
                    Arc::new(Arc::clone(&local)),
                    2,
                    envelope("agent.new", &OwnerToken::new("whatever")),
                    ENTRANCE_SOCKET,
                ));
                entered.recv().await.expect("the body entered");
                clock.advance(Duration::from_secs(11));
                assert_eq!(deliveries.expire(), 1);
                let _timed_out = next_event(&mut caller_inbox).await;
                open.send(()).expect("open the door");
                finished(round_trip).await;
            })
        });

        assert!(
            logged
                .iter()
                .any(|line| line.contains("agent.new") && line.contains("panicked")),
            "a body that died after the deadline must leave the same line as the sibling branch: {logged:?}"
        );
        // ★A dead body **does not know how far it got** — so it holds that number★ (if not held a re-query might apply
        //   an operation that may have been applied once more). The hold is released by the sweeper when the window passes.
        assert_eq!(
            deliveries.in_flight(),
            1,
            "if the certainty is unknown, holding is the safe side"
        );
    }

    /// ★Stage 1 **does not pile up without bound**★ — past the cap it refuses loudly instead of growing the queue.
    /// If storage I/O stops the body does not end and the deadline cannot reap that seat either ([`Pending::local`]), so without a cap seats and pool jobs grow together without bound.
    #[test]
    fn stage_one_refuses_to_queue_past_its_ceiling() {
        let roster = CommandRoster::new();
        let mut caller_inbox = attach_caller(&roster, 2);
        let deliveries = deliveries_with(Arc::new(ManualClock::new())).with_local_limit(1);
        let (local, open, mut entered) = parked_local("agent.new");

        let logged = capture_loud(async {
            let first = tokio::spawn(super::deliver(
                roster.clone(),
                deliveries.clone(),
                Arc::new(Arc::clone(&local)),
                2,
                envelope("agent.new", &OwnerToken::new("whatever")),
                ENTRANCE_SOCKET,
            ));
            entered.recv().await.expect("the body entered");

            // The second is blocked by the cap though it is a **different number** — the cap counts the number of running jobs, not numbers.
            super::deliver(
                roster.clone(),
                deliveries.clone(),
                Arc::new(Arc::clone(&local)),
                2,
                envelope("agent.new", &OwnerToken::new("whatever")),
                ENTRANCE_SOCKET,
            )
            .await;

            match next_event(&mut caller_inbox).await {
                AgentEvent::CommandReply { reply } => {
                    let reply = opened_reply(&reply);
                    let err = reply.outcome.expect_err("cap refusal");
                    assert_eq!(err.code(), ErrorCode::Conflict);
                    assert!(
                        err.message().contains("nothing was run"),
                        "the fact that nothing ran must be carried: {}",
                        err.message()
                    );
                    assert!(
                        err.message().contains("restarted"),
                        "it must even say the state may not clear: {}",
                        err.message()
                    );
                }
                other => panic!("a refusal is one reply too: {other:?}"),
            }
            assert_eq!(local.runs(), 1, "a refusal does not run the body");
            assert_eq!(
                deliveries.in_flight(),
                1,
                "the seat of a refused round trip is released — that number must not get blocked"
            );

            open.send(()).expect("open the door");
            finished(first).await;
        });

        // ★The refused caller must not be the only witness★ — the global cap is an operational event.
        assert!(
            logged.iter().any(|line| line.contains("ceiling")),
            "the cap event must remain on the server too: {logged:?}"
        );
    }

    /// A stage 1 that dies without an answer — stands in for a place that blows up **outside** the table's panic net (there is one such place in the real thing: the correction where the table fits arguments to
    /// the declared schema is outside the net of `blocking_handler`).
    struct PanickingLocal;

    impl LocalCommands for PanickingLocal {
        fn claim(&self, name: &str) -> Option<Effect> {
            (name == "agent.new").then_some(Effect::Write)
        }

        fn run(
            &self,
            _name: &str,
            _args: &mut serde_json::Value,
            _entrance: &'static str,
            _caller: Option<ConnId>,
        ) -> Option<Result<serde_json::Value, CommandError>> {
            panic!("the table blew up while coercing arguments")
        }

        fn decls(&self) -> Vec<CommandDecl> {
            Vec::new()
        }

        fn agent_roster(&self) -> Vec<AgentRosterRow> {
            Vec::new()
        }
    }

    /// ★When a stage 1 body **dies with no answer** a trace must remain on the server side too★
    /// The caller gets the single word "unknown" but that alone gives the operator no way to know what blew up (logging convention no silent swallow). The CLI twin leaves an error with `entrance = "cli"` for the same incident
    /// (`mcp_server::control_agent_handler`) so this surface leaves the same shape.
    #[test]
    fn a_local_body_that_dies_without_answering_is_logged_not_swallowed() {
        let roster = CommandRoster::new();
        let mut caller_inbox = attach_caller(&roster, 2);
        let deliveries = deliveries_with(Arc::new(ManualClock::new()));

        let logged = engram_dashboard_command::testing::with_quiet_panic_hook(|| {
            capture_loud(async {
                super::deliver(
                    roster.clone(),
                    deliveries.clone(),
                    Arc::new(PanickingLocal),
                    2,
                    envelope("agent.new", &OwnerToken::new("whatever")),
                    ENTRANCE_SOCKET,
                )
                .await;
            })
        });

        match event_from(caller_inbox.try_recv().expect("one reply")) {
            AgentEvent::CommandReply { reply } => {
                let reply = opened_reply(&reply);
                let err = reply.outcome.expect_err("a dead body");
                // Whether it was applied cannot be known — folding to `INTERNAL` would be a false report "certainly failed".
                assert_eq!(err.code(), ErrorCode::OutcomeUnknown);
            }
            other => panic!("a reply must come: {other:?}"),
        }
        assert!(
            logged.iter().any(|line| line.contains("agent.new")),
            "which command died must remain on the server too: {logged:?}"
        );
        // ★A dead body **does not know how far it got** — so it holds that number★ (if not held a re-query would run an operation that may have been applied
        //   once more). The hold is released by the sweeper when the window passes.
        assert_eq!(
            deliveries.in_flight(),
            1,
            "if the certainty is unknown, holding is the safe side"
        );
    }

    /// ★The cap signal must be bounded in **both directions**★ — writing per refusal lets the other side decide the log size, and reopening only when the count drops makes the signal **end at one line** in a truly bitten state (the count never drops).
    #[test]
    fn the_ceiling_signal_repeats_on_a_clock_not_on_every_refusal() {
        let roster = CommandRoster::new();
        let mut caller_inbox = attach_caller(&roster, 2);
        let clock = Arc::new(ManualClock::new());
        let deliveries = deliveries_with(clock.clone()).with_local_limit(1);
        let (local, open, mut entered) = parked_local("agent.new");

        let logged = capture_loud(async {
            let first = tokio::spawn(super::deliver(
                roster.clone(),
                deliveries.clone(),
                Arc::new(Arc::clone(&local)),
                2,
                envelope("agent.new", &OwnerToken::new("whatever")),
                ENTRANCE_SOCKET,
            ));
            entered.recv().await.expect("the body entered");

            // It hits the cap back to back — the line grows only once.
            for _ in 0..3 {
                super::deliver(
                    roster.clone(),
                    deliveries.clone(),
                    Arc::new(Arc::clone(&local)),
                    2,
                    envelope("agent.new", &OwnerToken::new("whatever")),
                    ENTRANCE_SOCKET,
                )
                .await;
                let _ = next_event(&mut caller_inbox).await;
            }
            // After the interval one more line — the signal does not stop even while bitten.
            clock.advance(CEILING_WARN_INTERVAL + Duration::from_secs(1));
            super::deliver(
                roster.clone(),
                deliveries.clone(),
                Arc::new(Arc::clone(&local)),
                2,
                envelope("agent.new", &OwnerToken::new("whatever")),
                ENTRANCE_SOCKET,
            )
            .await;
            let _ = next_event(&mut caller_inbox).await;

            open.send(()).expect("open the door");
            finished(first).await;
        });

        let ceiling_lines = logged
            .iter()
            .filter(|line| line.contains("ceiling"))
            .count();
        assert_eq!(
            ceiling_lines, 2,
            "four refusals must give two lines (one per interval): {logged:?}"
        );
    }

    /// ★The **reason a post-deadline success log exists must not be cut off**★ — that line's value is the only clue (`agent_id`) to recover what was made without the caller's knowledge. Using the default name width (64 characters) as is
    /// would quietly lose it as soon as the payload grew by even one field.
    #[test]
    fn the_late_success_log_keeps_the_datum_it_exists_for() {
        let roster = CommandRoster::new();
        let mut caller_inbox = attach_caller(&roster, 2);
        let clock = Arc::new(ManualClock::new());
        let deliveries = deliveries_with(clock.clone());
        let (local, open, mut entered) = parked_local("agent.new");
        // It must survive even if a field that sorts **before** `agent_id` by name is added.
        let payload = json!({
            "agent_id": "3f2504e0-4f89-11d3-9a0c-0305e82c3301",
            "a_long_field_that_comes_first": "x".repeat(80),
            "name": "from-bus",
            "state": "sleeping",
        });
        let local = Arc::new(FakeLocal {
            answer: Some(Ok(payload)),
            entered: Mutex::new(local.entered.lock().unwrap().take()),
            gate: Mutex::new(local.gate.lock().unwrap().take()),
            ..FakeLocal::answering("agent.new", Ok(json!({})))
        });

        let logged = capture_loud(async {
            let round_trip = tokio::spawn(super::deliver(
                roster.clone(),
                deliveries.clone(),
                Arc::new(Arc::clone(&local)),
                2,
                envelope("agent.new", &OwnerToken::new("whatever")),
                ENTRANCE_SOCKET,
            ));
            entered.recv().await.expect("the body entered");
            clock.advance(Duration::from_secs(11));
            assert_eq!(deliveries.expire(), 1);
            let _timed_out = next_event(&mut caller_inbox).await;
            open.send(()).expect("open the door");
            finished(round_trip).await;
        });

        assert!(
            logged
                .iter()
                .any(|line| line.contains("3f2504e0-4f89-11d3-9a0c-0305e82c3301")),
            "the clue to recover was cut off: {logged:?}"
        );
    }

    /// ★Does **not produce side effects** for a caller who has already left★ — the same as the check stage 2 does just before handing over the envelope (`OwnerLink::send`). `agent.new` has no way back (ADR-0122) so what remains is permanent.
    /// ★What is measured here is the **early** check★ — the **late** check just before the side effect is inside the body ([`spawn_local`]) and uses the same predicate. That one is not aimed at separately because this harness has no means to deterministically
    /// make "disconnect while queued in the pool" — instead every other test below must pass that check for the body to run, so if that check gave a false positive this whole file would turn red.
    #[tokio::test]
    async fn a_caller_that_already_left_does_not_get_its_local_command_run() {
        // Does not attach the caller to the roster = already left.
        let roster = CommandRoster::new();
        let local = Arc::new(FakeLocal::answering("agent.new", Ok(json!({ "ok": true }))));

        super::deliver(
            roster.clone(),
            deliveries_with(Arc::new(ManualClock::new())),
            Arc::new(Arc::clone(&local)),
            2,
            envelope("agent.new", &OwnerToken::new("whatever")),
            ENTRANCE_SOCKET,
        )
        .await;

        assert_eq!(
            local.runs(),
            0,
            "an irreversible operation must not be run for a caller who has left"
        );
    }

    // ── branch ① normal round trip ────────────────────────────────────────────

    /// ★The reply comes back to **the connection that originally asked**★ — leaking to another connection makes that side receive the result of something it did not order, and the asker hangs until the deadline.
    /// It also measures that the owner token on the outer envelope goes out **overwritten with the value the roster answered** (that field is the basis on which the middle hop splits in the 2-stage delivery of TRD §3-8).
    #[tokio::test]
    async fn a_command_reaches_its_owner_and_the_answer_returns_to_the_caller() {
        let (roster, mut owner_inbox) = roster_with_owner(1, "tab.create");
        let mut caller_inbox = attach_caller(&roster, 2);
        let mut bystander_inbox = attach_caller(&roster, 3);
        let deliveries = deliveries_with(Arc::new(ManualClock::new()));
        // ★The destination field is whatever the caller writes★ — targeting is the daemon's job (ADR-0154).
        let env = envelope("tab.create", &OwnerToken::new("whatever-the-caller-thinks"));
        let request_id = env.request_id;

        let round_trip = tokio::spawn(deliver(roster.clone(), deliveries.clone(), 2, env));

        match next_event(&mut owner_inbox).await {
            AgentEvent::CommandRequest { envelope } => {
                let envelope = opened(&envelope);
                assert_eq!(
                    envelope.request_id, request_id,
                    "the id is the same throughout"
                );
                assert_eq!(
                    envelope.owner,
                    CommandRoster::owner_of(1),
                    "the owner the roster answered is carried on the outer envelope"
                );
                assert_eq!(
                    envelope.args,
                    json!({ "window": "main" }),
                    "the inside passes through as is"
                );
            }
            other => panic!("the owner receives a CommandRequest: {other:?}"),
        }
        assert_eq!(
            deliveries.in_flight(),
            1,
            "it sits in the table while waiting for the answer"
        );

        assert_eq!(
            deliveries.complete(CommandReply::ok(request_id, json!({ "tab": 4 }))),
            OutcomeLanding::Attached,
            "it attaches to the round trip in progress"
        );
        finished(round_trip).await;

        match next_event(&mut caller_inbox).await {
            AgentEvent::CommandReply { reply } => {
                let reply = opened_reply(&reply);
                assert_eq!(reply.request_id, request_id);
                assert_eq!(
                    reply.outcome,
                    Ok(json!({ "tab": 4 })),
                    "the owner's outcome as is"
                );
            }
            other => panic!("the connection that asked receives a CommandReply: {other:?}"),
        }
        assert!(
            bystander_inbox.try_recv().is_err(),
            "another's round trip must not leak to another connection"
        );
        assert_eq!(
            deliveries.in_flight(),
            0,
            "a finished round trip leaves the table"
        );
    }

    // ── branch ② owner absent ────────────────────────────────────────────────

    /// A name not on the roster — gets **the same answer** as the name of a disconnected owner (the distinction loss ADR-0150 accepted).
    #[tokio::test]
    async fn a_command_nobody_owns_is_answered_unknown_command() {
        let roster = CommandRoster::new();
        let mut caller_inbox = attach_caller(&roster, 2);
        let deliveries = deliveries_with(Arc::new(ManualClock::new()));
        let env = envelope("theme.set", &OwnerToken::new("nobody"));
        let request_id = env.request_id;

        deliver(roster, deliveries.clone(), 2, env).await;

        match next_event(&mut caller_inbox).await {
            AgentEvent::CommandReply { reply } => {
                let reply = opened_reply(&reply);
                assert_eq!(reply.request_id, request_id);
                let err = reply.outcome.expect_err("there is no owner");
                assert_eq!(err.code(), ErrorCode::UnknownCommand);
            }
            other => panic!("expected a CommandReply: {other:?}"),
        }
        assert_eq!(
            deliveries.in_flight(),
            0,
            "a seat that was not opened must not remain"
        );
    }

    // ── branch ③ torn window ────────────────────────────────────────────────

    /// ★The lookup is `Available` but there is no way to reach★ — a designed residue that arises because `route` deliberately releases the roster lock between lookup and delivery (ADR-0154). To make that state deterministic only the owner token held in the table is swapped:
    /// the roster holds the name under the old token and no outlet comes out of that token now.
    /// ★The point is that it is not `UNKNOWN_COMMAND`★ — the lookup just answered that an owner exists, so that code is a false certainty, and that owner can reattach soon.
    #[tokio::test]
    async fn a_command_whose_owner_vanished_mid_handover_is_outcome_unknown() {
        let (roster, mut owner_inbox) = roster_with_owner(1, "tab.create");
        let mut caller_inbox = attach_caller(&roster, 2);
        roster.overwrite_stored_owner(1, OwnerToken::new("someone-else"));
        let deliveries = deliveries_with(Arc::new(ManualClock::new()));
        let env = envelope("tab.create", &OwnerToken::new("whatever"));
        let request_id = env.request_id;

        deliver(roster, deliveries.clone(), 2, env).await;

        match next_event(&mut caller_inbox).await {
            AgentEvent::CommandReply { reply } => {
                let reply = opened_reply(&reply);
                assert_eq!(reply.request_id, request_id);
                let err = reply.outcome.expect_err("there is no way to reach");
                assert_eq!(err.code(), ErrorCode::OutcomeUnknown);
                assert_eq!(
                    err.retry(),
                    engram_dashboard_command::RetryMode::Never,
                    "there is no dedup store so this path does not instruct a retry"
                );
            }
            other => panic!("expected a CommandReply: {other:?}"),
        }
        assert!(
            owner_inbox.try_recv().is_err(),
            "if there is no way to reach, nothing goes anywhere"
        );
        assert_eq!(
            deliveries.in_flight(),
            0,
            "the seat that was opened must be taken back"
        );
    }

    // ── branch ④ deadline exceeded ──────────────────────────────────────────

    /// ★After the deadline the reply goes out **exactly once**, and a late outcome cannot make a second reply★
    /// The place that discards a late outcome is the table's single ownership point — expiry reaped the seat so a later `complete` gets nothing.
    #[tokio::test]
    async fn a_deadline_answers_once_and_a_late_outcome_cannot_answer_again() {
        let clock = Arc::new(ManualClock::new());
        let (roster, mut owner_inbox) = roster_with_owner(1, "tab.create");
        let mut caller_inbox = attach_caller(&roster, 2);
        let deliveries = deliveries_with(clock.clone());
        let env = envelope("tab.create", &OwnerToken::new("whatever"));
        let request_id = env.request_id;

        let round_trip = tokio::spawn(deliver(roster.clone(), deliveries.clone(), 2, env));
        // From the moment the envelope is seen to have gone out that round trip is waiting for the answer.
        assert!(matches!(
            next_event(&mut owner_inbox).await,
            AgentEvent::CommandRequest { .. }
        ));

        // ★The boundary is **derived** from the deadline★ — hard-coding a number lets an edit fixing the deadline quietly move this test to the opposite branch ("before the deadline" becomes "after the deadline"
        //   and the first assertion may still be green).
        let deadline = CommandDeliveries::DEFAULT_DEADLINE;
        clock.advance(deadline - Duration::from_secs(1));
        assert_eq!(
            deliveries.expire(),
            0,
            "it is not reaped before the deadline"
        );
        clock.advance(Duration::from_secs(2));
        assert_eq!(
            deliveries.expire(),
            1,
            "it is reaped when the deadline passes"
        );
        finished(round_trip).await;

        match next_event(&mut caller_inbox).await {
            AgentEvent::CommandReply { reply } => {
                let reply = opened_reply(&reply);
                assert_eq!(reply.request_id, request_id);
                let err = reply.outcome.expect_err("deadline exceeded");
                assert_eq!(err.code(), ErrorCode::Timeout);
                assert_eq!(err.retry(), engram_dashboard_command::RetryMode::Never);
            }
            other => panic!("expected a CommandReply: {other:?}"),
        }

        assert_eq!(
            deliveries.complete(CommandReply::ok(request_id, json!({ "tab": 4 }))),
            OutcomeLanding::NoSeat,
            "there is no place to attach"
        );
        assert!(
            caller_inbox.try_recv().is_err(),
            "a late outcome must not become a second reply"
        );
    }

    // ── agent-reference conversion before the relay (ADR-0308) ────────────────────────────────

    /// A stage 1 table holding only an agent roster — it has none of my names (so every name goes by relay) and gives only the roster to resolve the reference fields.
    struct AgentsOnly {
        agents: Vec<(uuid::Uuid, &'static str)>,
        /// The thread that read the roster — measures whether it reads outside the delivery task (on the blocking pool).
        read_on: Mutex<Option<std::thread::ThreadId>>,
        /// A signal that the roster read started — the test aims deterministically at "while resolving".
        entered: Mutex<Option<mpsc::UnboundedSender<()>>>,
        /// A door holding the roster read — it does not end until the test opens it.
        gate: Mutex<Option<std::sync::mpsc::Receiver<()>>>,
        /// The number of times the roster was read.
        reads: AtomicUsize,
        /// If true the roster read panics (after passing the door) — the branch where the resolution job dies with no answer.
        panics: bool,
    }

    impl AgentsOnly {
        fn new(agents: &[(uuid::Uuid, &'static str)]) -> Arc<Self> {
            Arc::new(Self::build(agents, None, None, false))
        }

        /// A table that stops the first roster read at the door — when the read starts the third value rings, and the door must be opened with the second value for it to end.
        fn gated(
            agents: &[(uuid::Uuid, &'static str)],
        ) -> (
            Arc<Self>,
            std::sync::mpsc::Sender<()>,
            mpsc::UnboundedReceiver<()>,
        ) {
            let (open, gate) = std::sync::mpsc::channel::<()>();
            let (entered, entered_rx) = mpsc::unbounded_channel::<()>();
            (
                Arc::new(Self::build(agents, Some(entered), Some(gate), false)),
                open,
                entered_rx,
            )
        }

        fn panicking() -> Arc<Self> {
            Arc::new(Self::build(&[], None, None, true))
        }

        fn build(
            agents: &[(uuid::Uuid, &'static str)],
            entered: Option<mpsc::UnboundedSender<()>>,
            gate: Option<std::sync::mpsc::Receiver<()>>,
            panics: bool,
        ) -> Self {
            Self {
                agents: agents.to_vec(),
                read_on: Mutex::new(None),
                entered: Mutex::new(entered),
                gate: Mutex::new(gate),
                reads: AtomicUsize::new(0),
                panics,
            }
        }
    }

    impl LocalCommands for Arc<AgentsOnly> {
        fn claim(&self, _name: &str) -> Option<Effect> {
            None
        }

        fn run(
            &self,
            _name: &str,
            _args: &mut serde_json::Value,
            _entrance: &'static str,
            _caller: Option<ConnId>,
        ) -> Option<Result<serde_json::Value, CommandError>> {
            None
        }

        fn decls(&self) -> Vec<CommandDecl> {
            Vec::new()
        }

        fn agent_roster(&self) -> Vec<AgentRosterRow> {
            *self.read_on.lock().expect("agents poisoned") = Some(std::thread::current().id());
            self.reads.fetch_add(1, Ordering::SeqCst);
            if let Some(entered) = self.entered.lock().expect("agents poisoned").as_ref() {
                let _ = entered.send(());
            }
            let gate = self.gate.lock().expect("agents poisoned").take();
            if let Some(gate) = gate {
                let _ = gate.recv();
            }
            assert!(!self.panics, "the agent roster blew up");
            self.agents
                .iter()
                .map(|(id, name)| AgentRosterRow {
                    id: *id,
                    canonical_name: name.to_string(),
                    cwd: format!("C:/work/{name}"),
                    parent: None,
                    live: None,
                })
                .collect()
        }
    }

    /// An owner that put on one name marking agent-reference fields.
    fn roster_with_marked_owner(
        conn_id: ConnId,
        name: &str,
        agent_args: &[&str],
    ) -> (CommandRoster, mpsc::Receiver<Frame>) {
        let roster = CommandRoster::new();
        let (sink, inbox) = sink_with_inbox();
        roster.attach(conn_id, &sink);
        roster
            .register(
                conn_id,
                vec![CommandDecl {
                    name: name.to_string(),
                    help: "{}".to_string(),
                    agent_args: agent_args.iter().map(|arg| arg.to_string()).collect(),
                }],
            )
            .expect("registration");
        (roster, inbox)
    }

    fn marked_envelope(name: &str, args: serde_json::Value) -> CommandEnvelope {
        CommandEnvelope {
            args,
            ..envelope(name, &OwnerToken::new("whatever"))
        }
    }

    /// It was not relayed and an answer came — returns that answer's error.
    async fn refused_without_relay(
        caller_inbox: &mut mpsc::Receiver<Frame>,
        owner_inbox: &mut mpsc::Receiver<Frame>,
        deliveries: &CommandDeliveries,
        request_id: RequestId,
    ) -> CommandError {
        let err = match next_event(caller_inbox).await {
            AgentEvent::CommandReply { reply } => {
                let reply = opened_reply(&reply);
                assert_eq!(reply.request_id, request_id);
                reply.outcome.expect_err("it did not resolve")
            }
            other => panic!("expected a CommandReply: {other:?}"),
        };
        assert!(
            owner_inbox.try_recv().is_err(),
            "a command that did not resolve does not go to the owner"
        );
        assert_eq!(deliveries.in_flight(), 0, "the seat is released");
        err
    }

    /// ★The name in a marked field becomes an id, an id stays, and an unmarked field is passed untouched★ — the owner sees only ids in marked fields. The roster is read outside the delivery task (path normalization of a dormant profile can be long).
    #[tokio::test]
    async fn a_marked_name_is_relayed_as_the_agent_id() {
        let lead = uuid::Uuid::new_v4();
        let worker = uuid::Uuid::new_v4();
        let agents = AgentsOnly::new(&[(lead, "lead"), (worker, "worker")]);
        let (roster, mut owner_inbox) =
            roster_with_marked_owner(1, "slot.focus", &["agent", "after"]);
        let mut caller_inbox = attach_caller(&roster, 2);
        let deliveries = deliveries_with(Arc::new(ManualClock::new()));
        let env = marked_envelope(
            "slot.focus",
            json!({ "agent": "lead", "after": worker.to_string(), "label": "lead", "count": 3 }),
        );
        let request_id = env.request_id;

        let round_trip = tokio::spawn(super::deliver(
            roster.clone(),
            deliveries.clone(),
            Arc::new(Arc::clone(&agents)),
            2,
            env,
            ENTRANCE_SOCKET,
        ));

        match next_event(&mut owner_inbox).await {
            AgentEvent::CommandRequest { envelope } => {
                let envelope = opened(&envelope);
                assert_eq!(envelope.request_id, request_id);
                assert_eq!(
                    envelope.args,
                    json!({
                        "agent": lead.to_string(),
                        "after": worker.to_string(),
                        "label": "lead",
                        "count": 3
                    }),
                    "only marked fields become ids"
                );
            }
            other => panic!("the owner receives a CommandRequest: {other:?}"),
        }
        let read_on = agents
            .read_on
            .lock()
            .unwrap()
            .expect("the roster must have been read");
        assert_ne!(
            read_on,
            std::thread::current().id(),
            "the roster is read outside the delivery task"
        );

        deliveries.complete(CommandReply::ok(request_id, json!({})));
        finished(round_trip).await;
        assert!(matches!(
            next_event(&mut caller_inbox).await,
            AgentEvent::CommandReply { .. }
        ));
    }

    /// ★A nonexistent name is not relayed and the resolver's answer (with candidates) goes out with the field name in front★
    #[tokio::test]
    async fn an_unknown_marked_name_is_refused_with_the_arg_name_and_never_relayed() {
        let agents = AgentsOnly::new(&[
            (uuid::Uuid::new_v4(), "lead"),
            (uuid::Uuid::new_v4(), "leader"),
        ]);
        let (roster, mut owner_inbox) = roster_with_marked_owner(1, "slot.focus", &["agent"]);
        let mut caller_inbox = attach_caller(&roster, 2);
        let deliveries = deliveries_with(Arc::new(ManualClock::new()));
        let env = marked_envelope("slot.focus", json!({ "agent": "lea" }));
        let request_id = env.request_id;

        super::deliver(
            roster,
            deliveries.clone(),
            Arc::new(agents),
            2,
            env,
            ENTRANCE_SOCKET,
        )
        .await;

        let err =
            refused_without_relay(&mut caller_inbox, &mut owner_inbox, &deliveries, request_id)
                .await;
        assert_eq!(err.code(), ErrorCode::NotFound);
        assert!(err.message().starts_with("agent: "), "{}", err.message());
        assert!(
            err.message().contains("candidates: lead, leader"),
            "{}",
            err.message()
        );
    }

    /// ★If there are two of the same name none is picked★ — the resolver's `CONFLICT` as is and the candidates are the two ids.
    #[tokio::test]
    async fn a_duplicate_marked_name_is_a_conflict_and_never_relayed() {
        let twin_a = uuid::Uuid::new_v4();
        let twin_b = uuid::Uuid::new_v4();
        let agents = AgentsOnly::new(&[(twin_a, "twin"), (twin_b, "twin")]);
        let (roster, mut owner_inbox) = roster_with_marked_owner(1, "slot.focus", &["agent"]);
        let mut caller_inbox = attach_caller(&roster, 2);
        let deliveries = deliveries_with(Arc::new(ManualClock::new()));
        let env = marked_envelope("slot.focus", json!({ "agent": "twin" }));
        let request_id = env.request_id;

        super::deliver(
            roster,
            deliveries.clone(),
            Arc::new(agents),
            2,
            env,
            ENTRANCE_SOCKET,
        )
        .await;

        let err =
            refused_without_relay(&mut caller_inbox, &mut owner_inbox, &deliveries, request_id)
                .await;
        assert_eq!(err.code(), ErrorCode::Conflict);
        assert!(err.message().starts_with("agent: "), "{}", err.message());
        assert!(
            err.message().contains(&twin_a.to_string()),
            "{}",
            err.message()
        );
        assert!(
            err.message().contains(&twin_b.to_string()),
            "{}",
            err.message()
        );
    }

    /// ★Fields are resolved in marked order★ — if both fail to resolve the first-marked field becomes the answer (the same answer for the same request).
    #[tokio::test]
    async fn marked_args_are_resolved_in_the_order_they_were_marked() {
        let agents = AgentsOnly::new(&[]);
        let (roster, mut owner_inbox) =
            roster_with_marked_owner(1, "tab.move", &["after", "agent"]);
        let mut caller_inbox = attach_caller(&roster, 2);
        let deliveries = deliveries_with(Arc::new(ManualClock::new()));
        let env = marked_envelope("tab.move", json!({ "agent": "ghost", "after": "phantom" }));
        let request_id = env.request_id;

        super::deliver(
            roster,
            deliveries.clone(),
            Arc::new(agents),
            2,
            env,
            ENTRANCE_SOCKET,
        )
        .await;

        let err =
            refused_without_relay(&mut caller_inbox, &mut owner_inbox, &deliveries, request_id)
                .await;
        assert_eq!(err.code(), ErrorCode::NotFound);
        assert!(err.message().starts_with("after: "), "{}", err.message());
        assert!(
            err.message().contains("candidates: none"),
            "{}",
            err.message()
        );
    }

    /// ★Absent · `null` · non-string values are passed untouched★ — a refusal is the job of the owner that knows that field's contract. Against an empty roster, had it tried to resolve anything `NOT_FOUND` would have come.
    #[tokio::test]
    async fn marked_args_without_a_string_value_are_left_for_the_owner() {
        let agents = AgentsOnly::new(&[]);
        let (roster, mut owner_inbox) =
            roster_with_marked_owner(1, "slot.focus", &["agent", "after", "target"]);
        let _caller_inbox = attach_caller(&roster, 2);
        let deliveries = deliveries_with(Arc::new(ManualClock::new()));
        let args = json!({ "agent": null, "after": 7, "label": "ghost" });
        let env = marked_envelope("slot.focus", args.clone());
        let request_id = env.request_id;

        let round_trip = tokio::spawn(super::deliver(
            roster.clone(),
            deliveries.clone(),
            Arc::new(Arc::clone(&agents)),
            2,
            env,
            ENTRANCE_SOCKET,
        ));

        match next_event(&mut owner_inbox).await {
            AgentEvent::CommandRequest { envelope } => {
                let envelope = opened(&envelope);
                assert_eq!(envelope.args, args, "passes through as is");
            }
            other => panic!("the owner receives a CommandRequest: {other:?}"),
        }
        assert!(
            agents.read_on.lock().unwrap().is_none(),
            "with no field to resolve the roster is not read"
        );
        deliveries.complete(CommandReply::ok(request_id, json!({})));
        finished(round_trip).await;
    }

    /// ★If the deadline passes while resolving, that `TIMEOUT` is the answer and the envelope does not go out★ — passed on, the owner would run the command while the caller already received a failure.
    #[tokio::test]
    async fn a_deadline_while_resolving_answers_timeout_and_never_relays() {
        let clock = Arc::new(ManualClock::new());
        let (agents, open, mut entered_rx) = AgentsOnly::gated(&[(uuid::Uuid::new_v4(), "lead")]);
        let (roster, mut owner_inbox) = roster_with_marked_owner(1, "slot.focus", &["agent"]);
        let mut caller_inbox = attach_caller(&roster, 2);
        let deliveries = deliveries_with(clock.clone());
        let env = marked_envelope("slot.focus", json!({ "agent": "lead" }));
        let request_id = env.request_id;

        let round_trip = tokio::spawn(super::deliver(
            roster.clone(),
            deliveries.clone(),
            Arc::new(Arc::clone(&agents)),
            2,
            env,
            ENTRANCE_SOCKET,
        ));
        tokio::time::timeout(Duration::from_secs(10), entered_rx.recv())
            .await
            .expect("the roster read does not start")
            .expect("signal");

        clock.advance(CommandDeliveries::DEFAULT_DEADLINE + Duration::from_secs(1));
        assert_eq!(
            deliveries.expire(),
            1,
            "the deadline reaps the seat even while resolving"
        );
        finished(round_trip).await;

        match next_event(&mut caller_inbox).await {
            AgentEvent::CommandReply { reply } => {
                let reply = opened_reply(&reply);
                assert_eq!(reply.request_id, request_id);
                let err = reply.outcome.expect_err("deadline exceeded");
                assert_eq!(
                    err.code(),
                    ErrorCode::Timeout,
                    "the code is the settling side's as is"
                );
                assert!(
                    err.message().contains("never sent to its owner"),
                    "it is not that the owner did not answer but that it was not sent: {}",
                    err.message()
                );
            }
            other => panic!("expected a CommandReply: {other:?}"),
        }
        let _ = open.send(());
        assert!(
            owner_inbox.try_recv().is_err(),
            "the envelope of an answered round trip does not go out"
        );
    }

    /// Waits until the read ends and the resolve count becomes `n` — the count goes down only when the blocking-pool read ends.
    async fn resolving_settles_at(deliveries: &CommandDeliveries, n: usize) {
        tokio::time::timeout(Duration::from_secs(10), async {
            while deliveries.resolving() != n {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("the resolve count does not go down");
    }

    /// ★If the daemon goes down while resolving it stays `OUTCOME_UNKNOWN` but says "it was not sent"★ — the envelope never went to the owner.
    #[tokio::test]
    async fn a_shutdown_while_resolving_says_the_command_was_never_sent() {
        let (agents, open, mut entered_rx) = AgentsOnly::gated(&[(uuid::Uuid::new_v4(), "lead")]);
        let (roster, mut owner_inbox) = roster_with_marked_owner(1, "slot.focus", &["agent"]);
        let mut caller_inbox = attach_caller(&roster, 2);
        let deliveries = deliveries_with(Arc::new(ManualClock::new()));
        let env = marked_envelope("slot.focus", json!({ "agent": "lead" }));

        let round_trip = tokio::spawn(super::deliver(
            roster.clone(),
            deliveries.clone(),
            Arc::new(Arc::clone(&agents)),
            2,
            env,
            ENTRANCE_SOCKET,
        ));
        tokio::time::timeout(Duration::from_secs(10), entered_rx.recv())
            .await
            .expect("the roster read does not start")
            .expect("signal");
        assert_eq!(deliveries.drain(), 1);
        finished(round_trip).await;

        let err = match next_event(&mut caller_inbox).await {
            AgentEvent::CommandReply { reply } => {
                opened_reply(&reply).outcome.expect_err("shutdown")
            }
            other => panic!("expected a CommandReply: {other:?}"),
        };
        assert_eq!(err.code(), ErrorCode::OutcomeUnknown);
        assert!(
            err.message().contains("never sent to its owner"),
            "{}",
            err.message()
        );
        let _ = open.send(());
        resolving_settles_at(&deliveries, 0).await;
        assert!(
            owner_inbox.try_recv().is_err(),
            "the envelope does not go out"
        );
    }

    /// ★It does not read the roster for a caller who has already left★ — the same as the early check of stage 1. The answer has nowhere to go and only the seat is released.
    #[tokio::test]
    async fn a_caller_already_gone_starts_no_roster_read() {
        let agents = AgentsOnly::new(&[(uuid::Uuid::new_v4(), "lead")]);
        let (roster, mut owner_inbox) = roster_with_marked_owner(1, "slot.focus", &["agent"]);
        let deliveries = deliveries_with(Arc::new(ManualClock::new()));

        super::deliver(
            roster,
            deliveries.clone(),
            Arc::new(Arc::clone(&agents)),
            2,
            marked_envelope("slot.focus", json!({ "agent": "lead" })),
            ENTRANCE_SOCKET,
        )
        .await;

        assert_eq!(
            agents.reads.load(Ordering::SeqCst),
            0,
            "it does not start the read"
        );
        assert_eq!(
            deliveries.resolving(),
            0,
            "it does not use even one cap permit"
        );
        assert_eq!(deliveries.in_flight(), 0, "the seat is released");
        assert!(
            owner_inbox.try_recv().is_err(),
            "the envelope does not go out"
        );
    }

    /// ★If the caller leaves while resolving (the seat is reaped) that fact is the answer and the envelope does not go out★ — the read runs to the end and the count is released only when the read ends. To read the answer only the correlation table is reaped (in production it is removed from the roster first so this answer has nowhere to go).
    #[tokio::test]
    async fn a_caller_leaving_while_resolving_is_answered_gone_and_never_relayed() {
        let (agents, open, mut entered_rx) = AgentsOnly::gated(&[(uuid::Uuid::new_v4(), "lead")]);
        let (roster, mut owner_inbox) = roster_with_marked_owner(1, "slot.focus", &["agent"]);
        let mut caller_inbox = attach_caller(&roster, 2);
        let deliveries = deliveries_with(Arc::new(ManualClock::new()));
        let env = marked_envelope("slot.focus", json!({ "agent": "lead" }));
        let request_id = env.request_id;

        let round_trip = tokio::spawn(super::deliver(
            roster.clone(),
            deliveries.clone(),
            Arc::new(Arc::clone(&agents)),
            2,
            env,
            ENTRANCE_SOCKET,
        ));
        tokio::time::timeout(Duration::from_secs(10), entered_rx.recv())
            .await
            .expect("the roster read does not start")
            .expect("signal");
        assert_eq!(deliveries.drop_origin(2), 1);
        finished(round_trip).await;

        let err =
            refused_without_relay(&mut caller_inbox, &mut owner_inbox, &deliveries, request_id)
                .await;
        assert_eq!(err.code(), ErrorCode::Conflict);
        assert_eq!(err.message(), CALLER_ALREADY_GONE);
        assert_eq!(
            deliveries.resolving(),
            1,
            "before the read ends the count remains"
        );
        let _ = open.send(());
        resolving_settles_at(&deliveries, 0).await;
        assert!(
            owner_inbox.try_recv().is_err(),
            "even when the read ends the envelope does not go out"
        );
    }

    /// ★If the resolution job dies with no answer it is `INTERNAL` and does not relay★ — the count comes back even by panic.
    #[tokio::test]
    async fn a_resolver_that_dies_answers_internal_and_never_relays() {
        let agents = AgentsOnly::panicking();
        let (roster, mut owner_inbox) = roster_with_marked_owner(1, "slot.focus", &["agent"]);
        let mut caller_inbox = attach_caller(&roster, 2);
        let deliveries = deliveries_with(Arc::new(ManualClock::new()));
        let env = marked_envelope("slot.focus", json!({ "agent": "lead" }));
        let request_id = env.request_id;

        super::deliver(
            roster,
            deliveries.clone(),
            Arc::new(agents),
            2,
            env,
            ENTRANCE_SOCKET,
        )
        .await;

        let err =
            refused_without_relay(&mut caller_inbox, &mut owner_inbox, &deliveries, request_id)
                .await;
        assert_eq!(err.code(), ErrorCode::Internal);
        resolving_settles_at(&deliveries, 0).await;
    }

    /// ★Past the resolve cap it neither reads nor relays but refuses with `CONFLICT` — the count is released only when the blocked read ends★: even after the first round trip answered by the deadline and left, refusals continue while that read is blocked, and when the read ends the next command is relayed.
    #[tokio::test]
    async fn resolving_past_the_ceiling_is_refused_until_the_stuck_read_ends() {
        let lead = uuid::Uuid::new_v4();
        let clock = Arc::new(ManualClock::new());
        let (agents, open, mut entered_rx) = AgentsOnly::gated(&[(lead, "lead")]);
        let (roster, mut owner_inbox) = roster_with_marked_owner(1, "slot.focus", &["agent"]);
        let mut caller_inbox = attach_caller(&roster, 2);
        let deliveries = deliveries_with(clock.clone()).with_resolve_limit(1);
        let locals: Arc<dyn LocalCommands> = Arc::new(Arc::clone(&agents));
        let marked = || marked_envelope("slot.focus", json!({ "agent": "lead" }));
        let round_trip = |env: CommandEnvelope| {
            tokio::spawn(super::deliver(
                roster.clone(),
                deliveries.clone(),
                Arc::clone(&locals),
                2,
                env,
                ENTRANCE_SOCKET,
            ))
        };

        let stuck = round_trip(marked());
        tokio::time::timeout(Duration::from_secs(10), entered_rx.recv())
            .await
            .expect("the roster read does not start")
            .expect("signal");

        let env = marked();
        let request_id = env.request_id;
        finished(round_trip(env)).await;
        let err = match next_event(&mut caller_inbox).await {
            AgentEvent::CommandReply { reply } => {
                let reply = opened_reply(&reply);
                assert_eq!(reply.request_id, request_id);
                reply.outcome.expect_err("overflowed")
            }
            other => panic!("expected a CommandReply: {other:?}"),
        };
        assert!(
            owner_inbox.try_recv().is_err(),
            "an overflowing command is not relayed"
        );
        assert_eq!(
            deliveries.in_flight(),
            1,
            "an overflowing command's seat is released and only the blocked round trip's seat remains"
        );
        assert_eq!(err.code(), ErrorCode::Conflict);
        assert!(
            err.message().contains("not sent to its owner"),
            "{}",
            err.message()
        );
        assert_eq!(
            agents.reads.load(Ordering::SeqCst),
            1,
            "an overflowing command does not read the roster"
        );

        clock.advance(CommandDeliveries::DEFAULT_DEADLINE + Duration::from_secs(1));
        assert_eq!(deliveries.expire(), 1);
        finished(stuck).await;
        assert!(matches!(
            next_event(&mut caller_inbox).await,
            AgentEvent::CommandReply { .. }
        ));
        let env = marked();
        let request_id = env.request_id;
        finished(round_trip(env)).await;
        let err =
            refused_without_relay(&mut caller_inbox, &mut owner_inbox, &deliveries, request_id)
                .await;
        assert_eq!(
            err.code(),
            ErrorCode::Conflict,
            "if the answered round trip's read is still blocked the count remains too"
        );

        let _ = open.send(());
        resolving_settles_at(&deliveries, 0).await;
        let after = round_trip(marked());
        let envelope = match next_event(&mut owner_inbox).await {
            AgentEvent::CommandRequest { envelope } => opened(&envelope),
            other => panic!("the owner receives a CommandRequest: {other:?}"),
        };
        assert_eq!(envelope.args["agent"], json!(lead.to_string()));
        deliveries.complete(CommandReply::ok(envelope.request_id, json!({})));
        finished(after).await;
    }

    // ── disconnect cleanup ────────────────────────────────────────────────

    /// When the asking connection vanishes every item that connection held disappears, and the waiting delivery ends too.
    ///
    /// ★There must be no path where an item remains★ — if one remains a round trip nobody waits for sits in the table until the deadline.
    #[tokio::test]
    async fn a_caller_that_disconnects_takes_its_pending_round_trips_with_it() {
        let (roster, mut owner_inbox) = roster_with_owner(1, "tab.create");
        let _caller_inbox = attach_caller(&roster, 2);
        let deliveries = deliveries_with(Arc::new(ManualClock::new()));
        let other = envelope("tab.create", &OwnerToken::new("whatever"));

        let round_trip = tokio::spawn(deliver(roster.clone(), deliveries.clone(), 2, other));
        assert!(matches!(
            next_event(&mut owner_inbox).await,
            AgentEvent::CommandRequest { .. }
        ));

        // Exactly the production order — it leaves the roster first (that connection's outlet vanishes) and the correlation table follows.
        roster.detach(2);
        assert_eq!(
            deliveries.drop_origin(2),
            1,
            "that connection's round trip is reaped"
        );

        finished(round_trip).await;
        assert_eq!(deliveries.in_flight(), 0);
    }

    /// Another's connection dropping does not touch my round trip — otherwise a healthy caller loses its answer for no reason.
    #[tokio::test]
    async fn another_connections_cleanup_does_not_touch_my_round_trip() {
        let (roster, mut owner_inbox) = roster_with_owner(1, "tab.create");
        let mut caller_inbox = attach_caller(&roster, 2);
        let deliveries = deliveries_with(Arc::new(ManualClock::new()));
        let env = envelope("tab.create", &OwnerToken::new("whatever"));
        let request_id = env.request_id;

        let round_trip = tokio::spawn(deliver(roster.clone(), deliveries.clone(), 2, env));
        assert!(matches!(
            next_event(&mut owner_inbox).await,
            AgentEvent::CommandRequest { .. }
        ));

        assert_eq!(deliveries.drop_origin(7), 0, "another's connection cleanup");
        assert_eq!(deliveries.in_flight(), 1, "my round trip stays as is");

        assert_eq!(
            deliveries.complete(CommandReply::ok(request_id, json!({}))),
            OutcomeLanding::Attached
        );
        finished(round_trip).await;
        assert!(matches!(
            next_event(&mut caller_inbox).await,
            AgentEvent::CommandReply { .. }
        ));
    }

    // ── do not hold a handed-out copy beyond a round trip (ADR-0154) ──────────────────────────

    /// ★The owner's outlet copy must not stay alive during a round trip★
    /// An implementation that puts the copy in the wait ticket turns red here — after the envelope is written and the owner drops, that copy remains so the strong reference is not released (then the dropped connection's send queue outlives the roster and the connection leaks).
    #[tokio::test]
    async fn the_owners_sink_is_released_as_soon_as_the_envelope_is_written() {
        let roster = CommandRoster::new();
        let (owner_sink, mut owner_inbox) = sink_with_inbox();
        let released = Arc::downgrade(&owner_sink);
        roster.attach(1, &owner_sink);
        roster
            .register(
                1,
                vec![CommandDecl {
                    name: "tab.create".to_string(),
                    help: "{}".to_string(),
                    agent_args: Vec::new(),
                }],
            )
            .expect("registration");
        drop(owner_sink); // Now the roster is the only strong reference.
        let _caller_inbox = attach_caller(&roster, 2);
        let deliveries = deliveries_with(Arc::new(ManualClock::new()));
        let env = envelope("tab.create", &OwnerToken::new("whatever"));
        let request_id = env.request_id;

        let round_trip = tokio::spawn(deliver(roster.clone(), deliveries.clone(), 2, env));
        assert!(matches!(
            next_event(&mut owner_inbox).await,
            AgentEvent::CommandRequest { .. }
        ));

        // The envelope already went out and the reply has not — the owner drops in this state.
        roster.detach(1);
        drop(owner_inbox);

        assert!(
            released.upgrade().is_none(),
            "delivery is holding the owner's outlet copy beyond the round trip"
        );

        // The round trip is still reaped by the deadline — releasing the copy does not make the reply vanish.
        assert_eq!(
            deliveries.complete(CommandReply::ok(request_id, json!({}))),
            OutcomeLanding::Attached
        );
        finished(round_trip).await;
    }

    // ── when the same id overlaps (FIX 1) ────────────────────────────────────
    //
    // ★What is measured here is not the table but **the frames that went outside**★: looking only at the table's seat count completely misses the real violation "one seat yet two replies". Those two resolve the caller's pending with the one
    // that came first and make the rest an orphan (TRD §4-⑤).

    /// ★The same connection's same `request_id` = a re-query → **only one** reply★
    /// The second request coalesces into the round trip in progress, and the one owner outcome that arrives later answers both. If coalescing breaks the second request emits one error on the spot and then the real outcome goes out once more under the same key.
    #[tokio::test]
    async fn a_repeat_of_an_inflight_request_id_from_the_same_caller_yields_exactly_one_reply() {
        let (roster, mut owner_inbox) = roster_with_owner(1, "tab.create");
        let mut caller_inbox = attach_caller(&roster, 2);
        let deliveries = deliveries_with(Arc::new(ManualClock::new()));
        let first = envelope("tab.create", &OwnerToken::new("whatever"));
        let request_id = first.request_id;
        let again = CommandEnvelope {
            request_id,
            ..first.clone()
        };

        let a = tokio::spawn(deliver(roster.clone(), deliveries.clone(), 2, first));
        assert!(matches!(
            next_event(&mut owner_inbox).await,
            AgentEvent::CommandRequest { .. }
        ));
        // The same connection issues again with the same key — this task must end without emitting anything.
        let b = tokio::spawn(deliver(roster.clone(), deliveries.clone(), 2, again));
        finished(b).await;

        assert!(
            caller_inbox.try_recv().is_err(),
            "if a coalesced request gave its own reply separately the same key would get two replies"
        );
        assert_eq!(deliveries.in_flight(), 1, "the seat stays as the first one");
        assert!(
            owner_inbox.try_recv().is_err(),
            "the envelope must not go out twice either — the owner would do the same operation twice"
        );

        assert_eq!(
            deliveries.complete(CommandReply::ok(request_id, json!({ "tab": 4 }))),
            OutcomeLanding::Attached
        );
        finished(a).await;

        match next_event(&mut caller_inbox).await {
            AgentEvent::CommandReply { reply } => {
                let reply = opened_reply(&reply);
                assert_eq!(reply.request_id, request_id);
                assert_eq!(reply.outcome, Ok(json!({ "tab": 4 })));
            }
            other => panic!("expected a CommandReply: {other:?}"),
        }
        assert!(
            caller_inbox.try_recv().is_err(),
            "★this round trip gave exactly one reply★"
        );
    }

    /// If asked with an id that **another** connection is using, that side must get an answer — otherwise it hangs.
    /// That reply goes to a **different connection** than the reply of the round trip opened first, so "one reply" per caller is kept.
    /// (If coalescing ignored the connection this caller would wait forever with neither seat nor reply.)
    #[tokio::test]
    async fn another_caller_reusing_an_inflight_request_id_is_told_so_on_its_own_connection() {
        let (roster, mut owner_inbox) = roster_with_owner(1, "tab.create");
        let mut first_inbox = attach_caller(&roster, 2);
        let mut second_inbox = attach_caller(&roster, 3);
        let deliveries = deliveries_with(Arc::new(ManualClock::new()));
        let mine = envelope("tab.create", &OwnerToken::new("whatever"));
        let request_id = mine.request_id;
        let theirs = CommandEnvelope {
            request_id,
            ..mine.clone()
        };

        let a = tokio::spawn(deliver(roster.clone(), deliveries.clone(), 2, mine));
        assert!(matches!(
            next_event(&mut owner_inbox).await,
            AgentEvent::CommandRequest { .. }
        ));
        finished(tokio::spawn(deliver(
            roster.clone(),
            deliveries.clone(),
            3,
            theirs,
        )))
        .await;

        match next_event(&mut second_inbox).await {
            AgentEvent::CommandReply { reply } => {
                let reply = opened_reply(&reply);
                assert_eq!(reply.request_id, request_id);
                assert_eq!(
                    reply
                        .outcome
                        .expect_err("the key another's round trip holds")
                        .code(),
                    ErrorCode::RequestIdConflict
                );
            }
            other => panic!("the second caller must get an answer too: {other:?}"),
        }
        assert!(
            first_inbox.try_recv().is_err(),
            "another's conflict must not touch the reply seat of the round trip opened first"
        );

        assert_eq!(
            deliveries.complete(CommandReply::ok(request_id, json!({}))),
            OutcomeLanding::Attached
        );
        finished(a).await;
        assert!(matches!(
            next_event(&mut first_inbox).await,
            AgentEvent::CommandReply { .. }
        ));
        assert!(
            second_inbox.try_recv().is_err(),
            "the outcome of the round trip opened first must not leak to another's connection"
        );
    }

    /// ★A reply made by **another stage** of `route` must also go through the seat★ — it was a defect that arose without a race.
    /// Before, the seat was opened inside stage 2 (delivery) so the replies of stage 3 (owner absent → `UNKNOWN_COMMAND`) and stage 1 (my table) went out without looking at the table at all. So **when the owner dropped mid round trip and the same request was asked again** that
    /// re-query fell to stage 3 and sent `UNKNOWN_COMMAND` once under that key, then the real outcome went out once more. Now the seat is opened **before** `route` so that re-query coalesces into the round trip in progress and emits nothing.
    #[tokio::test]
    async fn a_reply_from_another_routing_stage_cannot_bypass_an_inflight_seat() {
        let (roster, mut owner_inbox) = roster_with_owner(1, "tab.create");
        let mut caller_inbox = attach_caller(&roster, 2);
        let deliveries = deliveries_with(Arc::new(ManualClock::new()));
        let first = envelope("tab.create", &OwnerToken::new("whatever"));
        let request_id = first.request_id;
        let again = CommandEnvelope {
            request_id,
            ..first.clone()
        };

        let a = tokio::spawn(deliver(roster.clone(), deliveries.clone(), 2, first));
        assert!(matches!(
            next_event(&mut owner_inbox).await,
            AgentEvent::CommandRequest { .. }
        ));

        // The owner drops mid round trip — now that name is not on the roster and the re-query is in a position to fall to stage 3.
        roster.detach(1);
        finished(tokio::spawn(deliver(
            roster.clone(),
            deliveries.clone(),
            2,
            again,
        )))
        .await;

        assert!(
            caller_inbox.try_recv().is_err(),
            "the UNKNOWN_COMMAND of stage 3 must not leak under the key of a round trip in progress"
        );
        assert_eq!(deliveries.in_flight(), 1, "the seat stays as the first one");

        assert_eq!(
            deliveries.complete(CommandReply::ok(request_id, json!({ "tab": 4 }))),
            OutcomeLanding::Attached
        );
        finished(a).await;
        match next_event(&mut caller_inbox).await {
            AgentEvent::CommandReply { reply } => {
                let reply = opened_reply(&reply);
                assert_eq!(
                    reply.outcome,
                    Ok(json!({ "tab": 4 })),
                    "the owner's outcome as is"
                )
            }
            other => panic!("expected a CommandReply: {other:?}"),
        }
        assert!(
            caller_inbox.try_recv().is_err(),
            "★this round trip gave exactly one reply★"
        );
    }

    /// ★A re-query that comes **right after the outcome attaches** must not send the envelope once more★
    /// In the structure where the outcome erased the seat and the frame went out after that, the gap in between was an empty table: the re-query opens a new seat and **makes the same command run again.** This aims at that window deterministically — it puts the first poll of
    /// the second delivery **before** the delivery task wakes after `complete` (a current-thread runtime so nobody else can slip in between).
    #[tokio::test]
    async fn a_repeat_arriving_before_the_reply_is_sent_does_not_forward_again() {
        let (roster, mut owner_inbox) = roster_with_owner(1, "tab.create");
        let mut caller_inbox = attach_caller(&roster, 2);
        let deliveries = deliveries_with(Arc::new(ManualClock::new()));
        let first = envelope("tab.create", &OwnerToken::new("whatever"));
        let request_id = first.request_id;
        let again = CommandEnvelope {
            request_id,
            ..first.clone()
        };

        let a = tokio::spawn(deliver(roster.clone(), deliveries.clone(), 2, first));
        assert!(matches!(
            next_event(&mut owner_inbox).await,
            AgentEvent::CommandRequest { .. }
        ));

        // The outcome has attached but the frame has not gone out — that task has not yet been able to wake.
        assert_eq!(
            deliveries.complete(CommandReply::ok(request_id, json!({ "tab": 4 }))),
            OutcomeLanding::Attached
        );
        // Put a cap — if this regression revives the second delivery opens its own seat and waits forever for an answer that will not come.
        tokio::time::timeout(
            Duration::from_secs(10),
            deliver(roster.clone(), deliveries.clone(), 2, again),
        )
        .await
        .expect("the re-query opened its own round trip and hung — it should have coalesced");

        assert!(
            owner_inbox.try_recv().is_err(),
            "a re-query that came between the outcome and the frame must not make the same operation run once more"
        );

        finished(a).await;
        assert!(matches!(
            next_event(&mut caller_inbox).await,
            AgentEvent::CommandReply { .. }
        ));
        assert!(
            caller_inbox.try_recv().is_err(),
            "★the frame is exactly one★"
        );
        assert_eq!(
            deliveries.in_flight(),
            0,
            "the seat is released after the frame goes out"
        );
    }

    /// ★If it is the same id but **a different request** it is not coalesced and is refused★ (the third leg of TRD §4-⑥)
    /// Coalesced, that request would **never be delivered** and never get its reply, and the one frame going out under that key would carry **the earlier request's result** — the caller resolves its promise with the wrong payload.
    #[tokio::test]
    async fn a_different_command_under_an_inflight_request_id_is_refused_not_folded() {
        let (roster, mut owner_inbox) = roster_with_owner(1, "tab.create");
        let mut caller_inbox = attach_caller(&roster, 2);
        let deliveries = deliveries_with(Arc::new(ManualClock::new()));
        let first = envelope("tab.create", &OwnerToken::new("whatever"));
        let request_id = first.request_id;
        // The same key, a different name — **a different operation** where the caller reused the id.
        let other = CommandEnvelope {
            request_id,
            name: "theme.set".to_string(),
            ..first.clone()
        };

        let a = tokio::spawn(deliver(roster.clone(), deliveries.clone(), 2, first));
        assert!(matches!(
            next_event(&mut owner_inbox).await,
            AgentEvent::CommandRequest { .. }
        ));
        finished(tokio::spawn(deliver(
            roster.clone(),
            deliveries.clone(),
            2,
            other,
        )))
        .await;

        match next_event(&mut caller_inbox).await {
            AgentEvent::CommandReply { reply } => {
                let reply = opened_reply(&reply);
                assert_eq!(
                    reply.outcome.expect_err("a different request used the same key").code(),
                    ErrorCode::RequestIdConflict,
                    "coalescing and handing over another's result is wrong — it is refused on the spot"
                );
            }
            other => panic!("the different request must get an answer too: {other:?}"),
        }
        assert!(
            owner_inbox.try_recv().is_err(),
            "a refused request must not be sent to the owner"
        );

        assert_eq!(
            deliveries.complete(CommandReply::ok(request_id, json!({ "tab": 4 }))),
            OutcomeLanding::Attached
        );
        finished(a).await;
        match next_event(&mut caller_inbox).await {
            AgentEvent::CommandReply { reply } => {
                let reply = opened_reply(&reply);
                assert_eq!(
                    reply.outcome,
                    Ok(json!({ "tab": 4 })),
                    "the round trip opened first receives its own result as is"
                )
            }
            other => panic!("expected a CommandReply: {other:?}"),
        }
    }

    /// ★A seat past the deadline but not yet reaped **must not absorb the next attempt**★
    /// The sweeper runs every second but the retry comes right after the deadline. If it absorbed, that attempt would never be delivered and the caller would receive the `TIMEOUT` of the earlier attempt, settled **before** it asked again — a retry becomes a quiet no-op.
    /// ★The sweeper is deliberately not started★: the point of this test is that what blocks absorption must be the query path itself, not the sweep period.
    ///
    /// ★What the below blesses — **two** frames and their **order**★: it is one per attempt so two is right, and this test asserts as is the order in which the earlier attempt's `TIMEOUT` is written to the same connection **after the retry was delivered to the owner**
    /// (reason = the inline settlement comment of `CommandDeliveries::open`). That is, two frames with the same correlation key can arrive in the order "old answer → new answer", and that is normal in this design.
    #[tokio::test]
    async fn a_retry_after_the_deadline_is_not_absorbed_by_the_unswept_seat() {
        let clock = Arc::new(ManualClock::new());
        let (roster, mut owner_inbox) = roster_with_owner(1, "tab.create");
        let mut caller_inbox = attach_caller(&roster, 2);
        let deliveries = deliveries_with(clock.clone());
        let first = envelope("tab.create", &OwnerToken::new("whatever"));
        let request_id = first.request_id;
        let retry = CommandEnvelope {
            request_id,
            ..first.clone()
        };

        let a = tokio::spawn(deliver(roster.clone(), deliveries.clone(), 2, first));
        assert!(matches!(
            next_event(&mut owner_inbox).await,
            AgentEvent::CommandRequest { .. }
        ));

        // The deadline has passed but nobody reaped — a re-query exactly as the contract instructs arrives in this state.
        clock.advance(Duration::from_secs(11));
        let b = tokio::spawn(deliver(roster.clone(), deliveries.clone(), 2, retry));

        assert!(
            matches!(
                next_event(&mut owner_inbox).await,
                AgentEvent::CommandRequest { .. }
            ),
            "the retry must get its own round trip and go to the owner again"
        );
        // The first attempt folded with no outcome — that one frame goes out first.
        finished(a).await;
        match next_event(&mut caller_inbox).await {
            AgentEvent::CommandReply { reply } => {
                let reply = opened_reply(&reply);
                assert_eq!(
                    reply
                        .outcome
                        .expect_err("the first attempt is the deadline")
                        .code(),
                    ErrorCode::Timeout
                );
            }
            other => panic!("expected a CommandReply: {other:?}"),
        }

        assert_eq!(
            deliveries.complete(CommandReply::ok(request_id, json!({ "tab": 4 }))),
            OutcomeLanding::Attached
        );
        finished(b).await;
        match next_event(&mut caller_inbox).await {
            AgentEvent::CommandReply { reply } => {
                let reply = opened_reply(&reply);
                assert_eq!(
                    reply.outcome,
                    Ok(json!({ "tab": 4 })),
                    "the retry gets its own result — it does not inherit the earlier attempt's TIMEOUT"
                )
            }
            other => panic!("expected a CommandReply: {other:?}"),
        }
        assert_eq!(deliveries.in_flight(), 0, "no seat may remain");
    }

    /// ★A reattached caller is not told "nothing happened"★ — that envelope may already have gone.
    /// The client has no identity that lives across reconnects, so a request that was half-dropped, reattached and asked again with the same id looks like a **stranger** by connection id. If the seat's owner has already left, whether that round trip was applied is **unknown**
    /// so both the certainty and the instruction answer so (ADR-0159).
    #[tokio::test]
    async fn a_reconnected_caller_retrying_its_id_is_not_told_nothing_happened() {
        let (roster, mut owner_inbox) = roster_with_owner(1, "tab.create");
        let _first_inbox = attach_caller(&roster, 2);
        let mut second_inbox = attach_caller(&roster, 3);
        let deliveries = deliveries_with(Arc::new(ManualClock::new()));
        let mine = envelope("tab.create", &OwnerToken::new("whatever"));
        let request_id = mine.request_id;
        let after_reconnect = CommandEnvelope {
            request_id,
            ..mine.clone()
        };

        let a = tokio::spawn(deliver(roster.clone(), deliveries.clone(), 2, mine));
        assert!(matches!(
            next_event(&mut owner_inbox).await,
            AgentEvent::CommandRequest { .. }
        ));

        // Only the disconnect was observed and cleanup (`drop_origin`) has not yet run — in between the reattached connection asks again with the same id.
        roster.detach(2);
        finished(tokio::spawn(deliver(
            roster.clone(),
            deliveries.clone(),
            3,
            after_reconnect,
        )))
        .await;

        match next_event(&mut second_inbox).await {
            AgentEvent::CommandReply { reply } => {
                let reply = opened_reply(&reply);
                let err = reply
                    .outcome
                    .expect_err("another's seat still holds that key");
                assert_eq!(err.code(), ErrorCode::OutcomeUnknown);
                assert_eq!(
                    err.retry(),
                    engram_dashboard_command::RetryMode::Never,
                    "there is no dedup store so this path does not instruct a retry"
                );
            }
            other => panic!("expected a CommandReply: {other:?}"),
        }

        assert_eq!(
            deliveries.complete(CommandReply::ok(request_id, json!({}))),
            OutcomeLanding::Attached
        );
        finished(a).await;
    }

    /// ★A command arriving in the shutdown window also **gets an answer**★ — otherwise it hangs until the process dies.
    /// If the table is only emptied and not closed, a later command opens a seat, and by then the sweeper has already stopped and there are no eyes to watch the deadline. A cap is put on the wait so that regression shows as a failure, not a hang.
    #[tokio::test]
    async fn a_command_arriving_after_shutdown_is_answered_instead_of_hanging() {
        let (roster, mut owner_inbox) = roster_with_owner(1, "tab.create");
        let mut caller_inbox = attach_caller(&roster, 2);
        let deliveries = deliveries_with(Arc::new(ManualClock::new()));
        let env = envelope("tab.create", &OwnerToken::new("whatever"));
        let request_id = env.request_id;

        assert_eq!(
            deliveries.drain(),
            0,
            "there is nothing to empty so only the table is closed"
        );

        tokio::time::timeout(
            Duration::from_secs(10),
            deliver(roster.clone(), deliveries.clone(), 2, env),
        )
        .await
        .expect("a command that arrived after shutdown hangs with no answer");

        match next_event(&mut caller_inbox).await {
            AgentEvent::CommandReply { reply } => {
                let reply = opened_reply(&reply);
                assert_eq!(reply.request_id, request_id);
                let err = reply.outcome.expect_err("the daemon is going down");
                assert_eq!(
                    err.code(),
                    ErrorCode::OutcomeUnknown,
                    "whether it was applied is unknown"
                );
                assert_eq!(err.retry(), engram_dashboard_command::RetryMode::Never);
            }
            other => panic!("expected a CommandReply: {other:?}"),
        }
        assert!(
            owner_inbox.try_recv().is_err(),
            "no envelope goes out on a closed table"
        );
        assert_eq!(
            deliveries.in_flight(),
            0,
            "a refused query does not sit in the table"
        );
    }

    // ── when the reply could not be put in (FIX 2) ──────────────────────────────────

    /// ★If the only reply could not be put in **it remains in the log at least**★
    /// The outcome already took the seat away so there is no way to undo. Swallowed, the caller would never get an answer with not even a deadline set and no trace at all would remain on the server (the logging convention antipattern "silent swallow").
    /// The queue has capacity 1 and is pre-filled to make saturation.
    #[test]
    fn a_reply_that_cannot_be_enqueued_is_reported_not_swallowed() {
        let (roster, mut owner_inbox) = roster_with_owner(1, "tab.create");
        // Capacity 1 — if one frame is pre-filled below there is no place for the reply to go.
        let (caller_sink, _caller_inbox) = sink_with_capacity(1);
        roster.attach(2, &caller_sink);
        caller_sink
            .try_send(Frame::Text("occupied".into()))
            .expect("the one place is free");
        let deliveries = deliveries_with(Arc::new(ManualClock::new()));
        let env = envelope("tab.create", &OwnerToken::new("whatever"));
        let request_id = env.request_id;

        let logged = capture_loud(async {
            let round_trip = tokio::spawn(deliver(roster.clone(), deliveries.clone(), 2, env));
            assert!(matches!(
                next_event(&mut owner_inbox).await,
                AgentEvent::CommandRequest { .. }
            ));
            assert_eq!(
                deliveries.complete(CommandReply::ok(request_id, json!({}))),
                OutcomeLanding::Attached
            );
            finished(round_trip).await;
        });

        assert!(
            logged
                .iter()
                .any(|line| line.contains(&request_id.to_string())),
            "which round trip's reply vanished must remain: {logged:?}"
        );
    }

    // ── a caller who already left (FIX 3) ────────────────────────────────────

    /// ★Even if cleanup runs **before the delivery task's first poll** the command must not be executed★
    /// `dispatch` detaches delivery so that task's first poll can be later than `on_disconnect`. In that order `drop_origin` sees an empty table and passes, and if the late-waking task opens a seat and sends the envelope ① **a command with side effects is executed**
    /// for a caller who has already left and ② nobody reaps that seat so it stays until the deadline.
    ///
    /// ★Make that bad order deterministic★: `deliver` is an `async fn` so it runs **when polled, not when called**. So building the future, running cleanup first and then awaiting reproduces exactly that interleave
    /// (with spawn the test cannot decide the first-poll moment so it cannot make this order).
    #[tokio::test]
    async fn a_command_from_a_caller_that_already_left_is_never_forwarded() {
        let (roster, mut owner_inbox) = roster_with_owner(1, "tab.create");
        let _caller_inbox = attach_caller(&roster, 2);
        let deliveries = deliveries_with(Arc::new(ManualClock::new()));
        let env = envelope("tab.create", &OwnerToken::new("whatever"));

        // A delivery that has never been polled.
        let pending_delivery = deliver(roster.clone(), deliveries.clone(), 2, env);

        // Exactly the production `on_disconnect` order — remove from the roster first and then reap the seat.
        roster.detach(2);
        assert_eq!(
            deliveries.drop_origin(2),
            0,
            "there is no seat yet — this empty-handedness is the starting point of this regression"
        );

        // ★Put a cap on the wait★ — if this regression revives the envelope goes out and delivery waits **forever** for the owner's answer (nobody reaps that seat and this test has no sweeper to push the deadline). Without the cap that regression shows not as a failure
        //   but as a hang and does not even say which assertion was wrong (the same convention as [`next_event`] — the normal path ends in one poll so this value is not a criterion).
        tokio::time::timeout(Duration::from_secs(10), pending_delivery)
            .await
            .expect(
                "the departed caller's delivery does not end — the envelope went out and it is waiting for an answer that will not come",
            );

        assert!(
            owner_inbox.try_recv().is_err(),
            "a departed caller's command must not be delivered to the owner — the side effect happens as is"
        );
        assert_eq!(
            deliveries.in_flight(),
            0,
            "leaving a seat nobody will reap would sit in the table until the deadline"
        );
    }

    // ── shutdown (FIX 4) ──────────────────────────────────────────────────

    /// ★Shutdown is bounded — it answers every remaining seat and those answers free the delivery tasks at once★
    /// If not emptied, a delivery task waits for its reply seat and **the sender side of that seat is inside this table**, so nothing wakes it until the deadline ([`CommandDeliveries::DEFAULT_DEADLINE`]). Meanwhile both the table and the task live longer than the moment
    /// they are reported "ended". The sweeper is **waited on** to observe that fact — dropping the handle leaves this assertion nowhere to stand.
    #[tokio::test]
    async fn shutdown_answers_every_outstanding_round_trip_and_stops_the_sweeper() {
        let (roster, mut owner_inbox) = roster_with_owner(1, "tab.create");
        let mut caller_inbox = attach_caller(&roster, 2);
        let deliveries = deliveries_with(Arc::new(ManualClock::new()));
        let (stop, stopped) = watch::channel(false);
        let sweeper = deliveries.spawn_sweeper(stopped);
        let env = envelope("tab.create", &OwnerToken::new("whatever"));
        let request_id = env.request_id;

        let round_trip = tokio::spawn(deliver(roster.clone(), deliveries.clone(), 2, env));
        assert!(matches!(
            next_event(&mut owner_inbox).await,
            AgentEvent::CommandRequest { .. }
        ));
        assert_eq!(deliveries.in_flight(), 1);

        stop.send(true).expect("the sweeper is alive");
        tokio::time::timeout(Duration::from_secs(10), sweeper)
            .await
            .expect("the sweeper does not stop on the shutdown signal")
            .expect("the sweeper task");

        assert_eq!(deliveries.in_flight(), 0, "no seat may remain");
        // ★The clock was not pushed even one notch★ — it was shutdown, not the deadline, that released it.
        finished(round_trip).await;
        match next_event(&mut caller_inbox).await {
            AgentEvent::CommandReply { reply } => {
                let reply = opened_reply(&reply);
                assert_eq!(reply.request_id, request_id);
                let err = reply.outcome.expect_err("shutdown");
                assert_eq!(
                    err.code(),
                    ErrorCode::OutcomeUnknown,
                    "whether it was applied is unknown"
                );
                assert_eq!(err.retry(), engram_dashboard_command::RetryMode::Never);
            }
            other => panic!("expected a CommandReply: {other:?}"),
        }
    }

    /// If there is nothing to empty it is quiet, and emptying twice does not send a reply twice.
    #[test]
    fn draining_twice_answers_each_seat_once() {
        let deliveries = deliveries_with(Arc::new(ManualClock::new()));
        let request_id = RequestId::new();
        let Seat::Opened { rx: _rx, .. } =
            deliveries.open(request_id, 1, "tab.create", &json!({}), false)
        else {
            panic!("must get a seat");
        };

        assert_eq!(deliveries.drain(), 1);
        assert_eq!(
            deliveries.drain(),
            0,
            "the second emptying has nothing to answer"
        );
        assert_eq!(deliveries.in_flight(), 0);
    }

    /// ★An emptied table **closes** — a later query gets no seat and is refused at once★ (the basis of shutdown being bounded).
    /// Without closing, a command arriving in the shutdown window opens a seat and waits for an answer, but by then the sweeper has already stopped and there are no eyes to watch the deadline — that round trip hangs with zero replies until the process dies.
    #[test]
    fn a_drained_table_refuses_new_round_trips() {
        let deliveries = deliveries_with(Arc::new(ManualClock::new()));
        assert_eq!(deliveries.drain(), 0);

        assert!(
            matches!(
                deliveries.open(RequestId::new(), 1, "tab.create", &json!({}), false),
                Seat::Closed
            ),
            "a closed table gives no seat"
        );
        assert_eq!(
            deliveries.in_flight(),
            0,
            "a refused query does not sit in the table"
        );
    }

    // ── the correlation table itself ─────────────────────────────────────────

    /// It does not overwrite an in-progress key and splits in the order **what was asked again → who asked again**.
    /// Overwriting makes the round trip opened first quietly lose its seat. And if the branches are lumped they break one by one — giving an error to the same request makes two replies under one key, giving nothing to another connection makes that side hang, and
    /// **coalescing a different request makes that request receive another's result with neither execution nor reply** (basis = [`Seat`]).
    #[test]
    fn a_second_open_of_the_same_request_id_splits_on_payload_then_on_who_asked() {
        let deliveries = deliveries_with(Arc::new(ManualClock::new()));
        let request_id = RequestId::new();
        let args = json!({ "window": "main" });
        let Seat::Opened { rx: _first, .. } =
            deliveries.open(request_id, 1, "tab.create", &args, false)
        else {
            panic!("the first round trip gets a seat");
        };

        assert!(
            matches!(
                deliveries.open(request_id, 1, "tab.create", &args, false),
                Seat::Coalesced
            ),
            "the same request from the same connection is coalesced"
        );
        assert!(
            matches!(
                deliveries.open(request_id, 1, "theme.set", &args, false),
                Seat::Conflict
            ),
            "a different name is a different request — it must not be coalesced"
        );
        assert!(
            matches!(
                deliveries.open(
                    request_id,
                    1,
                    "tab.create",
                    &json!({ "window": "other" }),
                    false
                ),
                Seat::Conflict
            ),
            "different arguments are a different request — looking only at the name leaves a hole here"
        );
        assert!(
            matches!(
                deliveries.open(request_id, 2, "tab.create", &args, false),
                Seat::Taken { holder: 1 }
            ),
            "if it is another connection that side must be answered"
        );
        assert_eq!(
            deliveries.in_flight(),
            1,
            "the seat opened first stays as is"
        );
    }

    /// ★The seat is released **after the frame goes out**, not the moment the outcome attaches★ (module header rule 2).
    /// If the key empties as soon as the outcome attaches, a re-query that arrives before that frame goes out sees an empty table and sends **the envelope once more** — the same operation is applied twice and the frames of the same key become two.
    #[test]
    fn a_seat_stays_claimed_until_its_reply_has_been_sent() {
        let deliveries = deliveries_with(Arc::new(ManualClock::new()));
        let request_id = RequestId::new();
        let args = json!({});
        let Seat::Opened { rx: _rx, token } =
            deliveries.open(request_id, 1, "tab.create", &args, false)
        else {
            panic!("must get a seat");
        };

        assert_eq!(
            deliveries.complete(CommandReply::ok(request_id, json!({}))),
            OutcomeLanding::Attached
        );
        assert!(
            matches!(
                deliveries.open(request_id, 1, "tab.create", &args, false),
                Seat::Coalesced
            ),
            "even if the outcome has attached, before the frame the seat still belongs to that round trip"
        );

        deliveries.release(request_id, token);
        assert!(
            matches!(
                deliveries.open(request_id, 1, "tab.create", &args, false),
                Seat::Opened { .. }
            ),
            "after the frame goes out a new round trip can be opened with the same id"
        );
    }

    /// ★A late `release` does not erase **another round trip that sits on the same key**★ — that is why it picks by ticket.
    /// A caller asking again with the same id after the deadline · shutdown reaped it is a normal path the contract does not forbid. Erasing by key makes that new round trip lose its answer for no reason.
    #[test]
    fn a_late_release_cannot_evict_the_next_round_trip_under_the_same_id() {
        let clock = Arc::new(ManualClock::new());
        let deliveries = deliveries_with(clock.clone());
        let request_id = RequestId::new();
        let args = json!({});
        let Seat::Opened {
            rx: _first,
            token: stale,
        } = deliveries.open(request_id, 1, "tab.create", &args, false)
        else {
            panic!("the first round trip gets a seat");
        };

        // The deadline folds that round trip with no outcome — up to here is the end of the first attempt.
        clock.advance(Duration::from_secs(11));
        assert_eq!(deliveries.expire(), 1);

        // A re-query of the same number that the contract does not forbid — the new round trip must sit on the same key.
        let Seat::Opened { rx: _second, .. } =
            deliveries.open(request_id, 1, "tab.create", &args, false)
        else {
            panic!("the re-query gets a new seat");
        };

        // The first task only now sends out its frame and releases its own seat.
        deliveries.release(request_id, stale);
        assert_eq!(
            deliveries.in_flight(),
            1,
            "a late cleanup erasing another's seat makes that caller lose its answer"
        );
    }

    /// ★Issuing the instruction applies to **an answer relayed, not one made up** — and that issuing **does not erase the owner's vocabulary**★ (the half FIX 9's structural gate cannot see)
    /// The owner is another process and does not use this module's constructors — by its own judgment it can send `retry: same-request-id`, and the daemon **relays that answer as is.** It is a path alive in the production process too (the panic net is preceded by
    /// the hook in production — ADR-0292 · ADR-0293) so this matters more than the panic net.
    /// ★The point of this test is to put the owner's answer in **decoded from the wire**★: an error made up in-process has an empty raw copy ([`CommandError::received`]) so the question "is the vocabulary preserved" itself becomes vacuous. Here an **unknown code** (`RATE_LIMITED`)
    /// and an **out-of-contract field** (`hint`) are carried, and whether those two are alive in the outgoing frame with only the instruction lowered is measured **in raw JSON** (read back by type an unknown code would look like `INTERNAL` and could not be measured at all).
    #[tokio::test]
    async fn an_error_relayed_from_the_owner_keeps_its_vocabulary_and_loses_only_its_retry_hint() {
        let (roster, mut owner_inbox) = roster_with_owner(1, "tab.create");
        let mut caller_inbox = attach_caller(&roster, 2);
        let deliveries = deliveries_with(Arc::new(ManualClock::new()));
        let env = envelope("tab.create", &OwnerToken::new("whatever"));
        let request_id = env.request_id;

        let round_trip = tokio::spawn(deliver(roster.clone(), deliveries.clone(), 2, env));
        assert!(matches!(
            next_event(&mut owner_inbox).await,
            AgentEvent::CommandRequest { .. }
        ));

        // Exactly the bytes the owner sent — carrying a code this daemon does not know and a field it does not know together.
        let from_owner: CommandReply = serde_json::from_str(&format!(
            r#"{{"request_id":"{request_id}","outcome":{{"Err":{{"code":"RATE_LIMITED","message":"slow down","retry":"same-request-id","hint":"5s"}}}}}}"#
        ))
        .expect("decode the owner's reply");
        assert_eq!(deliveries.complete(from_owner), OutcomeLanding::Attached);
        finished(round_trip).await;

        let frame = tokio::time::timeout(Duration::from_secs(10), caller_inbox.recv())
            .await
            .expect("the reply does not come")
            .expect("one frame");
        let Frame::Text(text) = frame else {
            panic!("expected Text: {frame:?}")
        };
        let sent: serde_json::Value =
            serde_json::from_str(&text).expect("read the outgoing frame as raw");
        let error = &sent["CommandReply"]["reply"]["outcome"]["Err"];

        assert_eq!(
            error["retry"], "never",
            "the relay hop issues the dangerous instruction — until dedup stands that responsibility is here"
        );
        assert_eq!(
            error["code"], "RATE_LIMITED",
            "★the owner's vocabulary is not erased★ — throwing away the copy to issue the instruction would change this to INTERNAL"
        );
        assert_eq!(
            error["hint"], "5s",
            "an out-of-contract field also reaches the final caller (an additive extension must not die in the relay)"
        );
        assert_eq!(
            error["message"], "slow down",
            "the wording is the owner's as is too"
        );
    }

    // ── retry instruction policy (FIX 9) ────────────────────────────────────

    /// ★If this path emits "ask again with the same number" again, it falls here★
    /// The basis is in [`no_retry`] (summary: there is no dedup store replaying completed work so a re-query becomes a **re-execution**).
    /// A behavior test can guard only the branches touched at the time, and when the next branch is added only that place quietly returns to the old instruction — so what is measured from the source is **whether the way to build an error is itself only one**.
    /// ★The needed strings are joined with `concat!`★ — written as is, this test would trip over its own pattern.
    ///
    /// ★Do not read this gate as the real guarantee★ — it is blind in two layers:
    /// ① **Origin**: it measures only answers this module *built*. Answers coming in from outside that this module **passes through** (`route`'s panic net · an error the owner sent) do not trip it.
    /// ② **Shape**: it only counts the **spelling** of the two constructors, so building an error through a thin wrapper around them (like `CommandError::internal` · `not_found`) passes as is.
    /// ★The real guarantee of this path is the one line of exit lowering in [`send_reply`], and the place that measures it is
    /// [`tests::an_error_relayed_from_the_owner_keeps_its_vocabulary_and_loses_only_its_retry_hint`]★ — this gate is an **early warning** laid on top: when a new place that carries the instruction directly appears
    /// it trips before reaching the exit.
    #[test]
    fn this_module_can_only_build_do_not_retry_failures() {
        let src = include_str!("command_delivery.rs");
        let derived = concat!("CommandError", "::of(");
        let typed = concat!("CommandError", "::with_retry(");

        assert_eq!(
            src.matches(derived).count(),
            0,
            "a constructor that derives the instruction from the code must not be used — errors are built only with no_retry"
        );
        assert_eq!(
            src.matches(typed).count(),
            1,
            "the constructor that carries the instruction directly must be only that one line inside no_retry"
        );
        assert_eq!(
            no_retry(ErrorCode::OutcomeUnknown, "x").retry(),
            RetryMode::Never,
            "the instruction that one line carries"
        );
    }

    /// An outcome nobody asked for has no place to attach.
    #[test]
    fn an_outcome_for_an_unknown_request_id_finds_no_seat() {
        let deliveries = deliveries_with(Arc::new(ManualClock::new()));

        assert_eq!(
            deliveries.complete(CommandReply::ok(RequestId::new(), json!({}))),
            OutcomeLanding::NoSeat
        );
    }

    // ── the second entrance — a connectionless caller (ADR-0160) ────────────────────────────

    /// ★The first fake caller's number is `u64::MAX`★ — the issuer goes down from there (`CommandBus::open_caller`) so the first call on a fresh bus uses this number. That is the basis for the leak test below getting that number in hand.
    const FIRST_CONTROL_CALLER: ConnId = u64::MAX;

    /// ★It returns the sweeper too — the receiver must hold it or the seat table closes ([`BusSweeper`]).★
    /// These tests push the deadline **by hand** so the sweeper runs but reaps nothing (a manual clock does not advance by itself).
    fn bus_with(roster: CommandRoster, clock: Arc<ManualClock>) -> (CommandBus, BusSweeper) {
        CommandBus::new(roster, deliveries_with(clock), Arc::new(NoLocalCommands))
    }

    /// Launches one control-route call — the answer is received with [`answered`].
    fn invoking(
        bus: &CommandBus,
        request_id: RequestId,
        name: &str,
    ) -> tokio::task::JoinHandle<CommandReply> {
        let bus = bus.clone();
        let name = name.to_string();
        tokio::spawn(async move {
            bus.invoke(request_id, name, json!({ "window": "main" }))
                .await
        })
    }

    /// Receives the call's answer **within a cap** — the basis is the same as [`next_event`] (breaking must be a failure, not a hang). ★Do not simply `.await` a call on this door★: the reply comes through the seat table so in a regression where the two entrances
    /// see different tables it never comes, and then the test stops without even saying which assertion was wrong.
    async fn answered(call: tokio::task::JoinHandle<CommandReply>) -> CommandReply {
        tokio::time::timeout(Duration::from_secs(10), call)
            .await
            .expect("the control route call hung with no answer")
            .expect("the call task")
    }

    /// Unpacks the raw envelope field of the packet that went out to the owner — an envelope the daemon re-wrapped always unpacks (ADR-0272).
    fn opened(raw: &serde_json::value::RawValue) -> CommandEnvelope {
        crate::connection_core::envelope_from_wire(raw)
            .expect("the envelope the daemon re-wrapped unpacks")
    }

    /// Unpacks the raw field of the reply packet that went out to the caller — a reply the daemon wrapped always unpacks (ADR-0272).
    fn opened_reply(raw: &serde_json::value::RawValue) -> CommandReply {
        reply_from_wire(raw).expect("the reply the daemon wrapped unpacks")
    }

    async fn envelope_from(inbox: &mut mpsc::Receiver<Frame>) -> CommandEnvelope {
        match next_event(inbox).await {
            AgentEvent::CommandRequest { envelope } => opened(&envelope),
            other => panic!("an envelope must come: {other:?}"),
        }
    }

    /// ★The control route's round trip runs end to end★ — the envelope goes out to the owner and its outcome comes back as **this call's return** (completion condition 1). And the fake connection it set up is reaped — if not reaped the roster's liveness table grows one entry per request
    /// permanently (that leak gives no signal at runtime).
    #[tokio::test]
    async fn the_control_door_hands_the_envelope_over_and_brings_the_answer_back() {
        let (roster, mut owner_inbox) = roster_with_owner(1, "tab.create");
        let (bus, _sweeper) = bus_with(roster, Arc::new(ManualClock::new()));
        let request_id = RequestId::new();

        let call = invoking(&bus, request_id, "tab.create");
        let envelope = envelope_from(&mut owner_inbox).await;
        assert_eq!(envelope.request_id, request_id);
        assert_eq!(envelope.name, "tab.create");
        assert_eq!(
            envelope.args["window"], "main",
            "the arguments cross as they are"
        );
        assert_eq!(
            envelope.owner,
            OwnerToken::new("conn-1"),
            "the destination owner is overwritten by the roster lookup result"
        );
        assert_eq!(
            bus.deliveries()
                .complete(CommandReply::ok(request_id, json!({ "tab": "t-1" }))),
            OutcomeLanding::Attached
        );

        let reply = answered(call).await;
        assert_eq!(reply.request_id, request_id);
        assert_eq!(reply.outcome.expect("the owner's success")["tab"], "t-1");
        assert_eq!(bus.deliveries().in_flight(), 0, "the seat is released");
        assert!(
            !bus.roster().is_attached(FIRST_CONTROL_CALLER),
            "once the round trip ends that fake connection must be reaped too"
        );
    }

    /// ★The control route too turns the names in marked fields into ids before relaying★ — the conversion is in the one place `deliver`, which both entrances pass (ADR-0308).
    #[tokio::test]
    async fn the_control_door_relays_a_marked_name_as_the_agent_id() {
        let lead = uuid::Uuid::new_v4();
        let (roster, mut owner_inbox) = roster_with_marked_owner(1, "slot.focus", &["slot_of"]);
        let (bus, _sweeper) = CommandBus::new(
            roster,
            deliveries_with(Arc::new(ManualClock::new())),
            Arc::new(AgentsOnly::new(&[(lead, "lead")])),
        );
        let request_id = RequestId::new();

        let call = {
            let bus = bus.clone();
            tokio::spawn(async move {
                bus.invoke(
                    request_id,
                    "slot.focus".to_string(),
                    json!({ "slot_of": "lead", "window": "main" }),
                )
                .await
            })
        };
        let envelope = envelope_from(&mut owner_inbox).await;
        assert_eq!(
            envelope.args,
            json!({ "slot_of": lead.to_string(), "window": "main" }),
            "only marked fields become ids"
        );
        bus.deliveries()
            .complete(CommandReply::ok(request_id, json!({ "ok": true })));

        let reply = answered(call).await;
        assert_eq!(reply.outcome.expect("the owner's success")["ok"], true);
        assert_eq!(bus.deliveries().in_flight(), 0, "the seat is released");
        assert_eq!(
            bus.deliveries().resolving(),
            0,
            "the resolve count comes back too"
        );
    }

    /// If nobody declared that name delivery answers "unknown command" — this door does not swallow that branch (the delivery-side half of completion condition 2. The side where the control route filters first by roster lookup is `control::catalog`).
    #[tokio::test]
    async fn a_name_with_no_owner_comes_back_as_an_unknown_command() {
        let (bus, _sweeper) = bus_with(CommandRoster::new(), Arc::new(ManualClock::new()));

        let reply = answered(invoking(&bus, RequestId::new(), "nope.nope")).await;

        assert_eq!(
            reply.outcome.expect_err("refusal").code(),
            ErrorCode::UnknownCommand
        );
        assert_eq!(bus.deliveries().in_flight(), 0);
    }

    /// ★An owner that does not answer is **released by the deadline**★ (completion condition 3) — meaning this door does not set its own time limit but uses the same deadline as the socket door. The clock is pushed by hand to ride only that branch deterministically.
    #[tokio::test]
    async fn a_control_call_that_passes_its_deadline_answers_timeout() {
        let clock = Arc::new(ManualClock::new());
        let (roster, mut owner_inbox) = roster_with_owner(1, "tab.create");
        let (bus, _sweeper) = bus_with(roster, clock.clone());

        let call = invoking(&bus, RequestId::new(), "tab.create");
        let _ = envelope_from(&mut owner_inbox).await;

        clock.advance(Duration::from_secs(11));
        assert_eq!(bus.deliveries().expire(), 1, "the deadline reaps that seat");

        let reply = answered(call).await;
        assert_eq!(
            reply.outcome.expect_err("refusal").code(),
            ErrorCode::Timeout
        );
    }

    /// ★A second query with the same number **does not send one more envelope**★ (completion condition 4 — the delivery branch).
    /// This door uses a new connection number per request so the two queries look like strangers, and so [`Seat::Taken`] of the seat table decides. Without that decision the same operation goes to the owner twice.
    #[tokio::test]
    async fn a_repeat_of_the_same_request_id_does_not_send_a_second_envelope() {
        let (roster, mut owner_inbox) = roster_with_owner(1, "tab.create");
        let (bus, _sweeper) = bus_with(roster, Arc::new(ManualClock::new()));
        let request_id = RequestId::new();

        let first = invoking(&bus, request_id, "tab.create");
        let _ = envelope_from(&mut owner_inbox).await;

        let repeat = answered(invoking(&bus, request_id, "tab.create")).await;
        assert_eq!(
            repeat.outcome.expect_err("refusal").code(),
            ErrorCode::RequestIdConflict
        );
        assert!(
            owner_inbox.try_recv().is_err(),
            "the envelope must be one — the second query sent the same operation again"
        );

        bus.deliveries()
            .complete(CommandReply::ok(request_id, json!({ "tab": "t-1" })));
        assert!(
            answered(first).await.outcome.is_ok(),
            "the round trip opened first gets its own answer"
        );
    }

    /// ★This door **does not ride the daemon's own names — this door bites that contract itself**★
    /// Today the basis for those names not coming here was in **another module** (`control::catalog::handle_call` runs its own table first). But the stage 1 branch of [`deliver`] leans on that basis, and if that branch rode this door a single cancellation would leave
    /// **one seat and one number for the whole process lifetime** (full reason = the refusal comment at the very top of [`CommandBus::invoke`]). So it refuses at the place that leans on it, and this test pins that refusal.
    /// ★The axis measured is not the "refusal code" but **whether anything was taken**★ — all four of body · seat · roster entry · cap count.
    /// ★To remove this branch and revive stage 1 at this door, cancellation-completeness must be built first★ (what is needed then = separating the `spawn_blocking` cleanup from cancellation). Lifting only the refusal before that brings the above leak straight back.
    #[tokio::test]
    async fn the_control_door_refuses_a_name_this_daemon_answers_itself() {
        let local = Arc::new(FakeLocal::answering(
            "agent.new",
            Ok(json!({ "agent_id": "a-1" })),
        ));
        let (bus, _sweeper) = CommandBus::new(
            CommandRoster::new(),
            deliveries_with(Arc::new(ManualClock::new())),
            Arc::new(Arc::clone(&local)),
        );

        let refused = answered(invoking(&bus, RequestId::new(), "agent.new")).await;

        assert_eq!(
            refused.outcome.expect_err("refusal").code(),
            ErrorCode::Internal,
            "a wiring defect, not something the caller can fix"
        );
        assert_eq!(local.runs(), 0, "the body is not run");
        assert_eq!(bus.deliveries().in_flight(), 0, "no seat is opened");
        assert!(
            !bus.roster().is_attached(FIRST_CONTROL_CALLER),
            "no roster entry is made"
        );
        assert_eq!(
            bus.relayed_counter().load(Ordering::SeqCst),
            0,
            "it turns away before even taking the cap count"
        );
    }

    /// ★The entrance label that arrives at stage 1 is **that door's own**★ — the point is to tell the incidents of two doors apart by this field, and if one side wears another's name tag that filter quietly catches only half (a relayed control call
    /// was once written as if it came from the socket).
    /// ★Today the only door that rides stage 1 is the socket★ (the refusal in the sibling test) — so this test measures two axes together: that the socket's label arrives as is, and that the control route **does not ride stage 1 at all**.
    /// ★The adapter's forwarding cannot be measured here★ — this double just records the label it received so a regression where `DaemonLocalCommands` hard-codes a constant is not caught here. That axis is measured by `the_entrance_label_is_forwarded_not_hardcoded` in `control::commands`
    /// riding that adapter directly.
    #[tokio::test]
    async fn stage_one_carries_the_label_of_the_door_that_drove_it() {
        let local = Arc::new(FakeLocal::answering("agent.new", Ok(json!({}))));

        let (roster, _inbox) = roster_with_owner(9, "someone.else");
        super::deliver(
            roster,
            CommandDeliveries::new(),
            Arc::new(Arc::clone(&local)),
            9,
            envelope("agent.new", &OwnerToken::new("")),
            ENTRANCE_SOCKET,
        )
        .await;
        assert_eq!(local.entrance(), Some(ENTRANCE_SOCKET));
        // ADR-0231: the connection that sent the envelope arrives as the caller of the lease decision — with `None` a holder's bus cancel would be refused.
        assert_eq!(
            local.caller(),
            Some(Some(9)),
            "delivery passes the connection that called"
        );
        assert_eq!(local.runs(), 1, "premise: the socket door rode stage 1");

        let (bus, _sweeper) = CommandBus::new(
            CommandRoster::new(),
            deliveries_with(Arc::new(ManualClock::new())),
            Arc::new(Arc::clone(&local)),
        );
        let _ = answered(invoking(&bus, RequestId::new(), "agent.new")).await;
        assert_eq!(
            local.entrance(),
            Some(ENTRANCE_SOCKET),
            "the control route rode stage 1 — that door must not be able to reach that branch"
        );
        assert_eq!(local.runs(), 1, "the body ran once more");
    }

    /// ★An assembly that attaches **two** sweepers to one seat table does not stand★
    /// Because the symptom when it stands is nasty: the marker that drops first calls [`CommandDeliveries::drain`] on its way out and **closes the live daemon's shared table**, and every later round trip is refused as [`Seat::Closed`] (the daemon is fine yet every command is
    /// "shutting down").
    /// ★What this test aims at is **a path visibility cannot block**★ — `CommandBus::new` is `pub` and the seat table is shared via [`CommandBus::deliveries`] and is `Clone`, so outside the module these two lines could be written. What blocks it is the table's own marker
    /// ([`Seats::swept`]).
    #[tokio::test]
    #[should_panic(expected = "already has a sweeper attached")]
    async fn a_second_sweeper_over_a_live_seat_table_cannot_be_assembled() {
        let (bus, _sweeper) = CommandBus::new(
            CommandRoster::new(),
            deliveries_with(Arc::new(ManualClock::new())),
            Arc::new(NoLocalCommands),
        );

        let (_second, _second_sweeper) = CommandBus::new(
            CommandRoster::new(),
            bus.deliveries().clone(),
            Arc::new(NoLocalCommands),
        );
    }

    /// ★The refusal **must not break that table**★ — this is the place where the assertion itself used to make the state it meant to guard.
    /// If that assertion is kept **while holding** the table's lock, unwinding drops that guard and the lock of [`Seats`] is poisoned. The state the live daemon then gets is exactly what this marker meant to prevent: afterward [`CommandDeliveries::open`]
    /// · [`CommandDeliveries::complete`] · [`CommandDeliveries::in_flight`] use a lock that panics on poisoning (`lock`) so every command dies, and only the destructor path (`lock_for_cleanup`) passes quietly.
    /// ★The production process cannot reach this branch★ — the panic of that assertion is outside the containment marker so the hook ends the process before unwinding and there is no poisoning (ADR-0292). So what this test guards is test builds without the hook, and the reason that is enough is that in that build
    /// there **really are** callers that catch a panic and keep running (the test harness is that shape).
    /// ★Even after moving the assertion outside the lock atomicity is unchanged★ — reading and setting finish at once inside the critical section, and what goes out is one local value, "was it already there".
    /// It is normal for the unwinding output (the panic backtrace) to print once on stderr — swapping the global panic hook would swallow other tests' output in the same binary so it is not touched.
    #[tokio::test]
    async fn a_refused_second_sweeper_leaves_the_shared_table_usable() {
        let (bus, _sweeper) = CommandBus::new(
            CommandRoster::new(),
            deliveries_with(Arc::new(ManualClock::new())),
            Arc::new(NoLocalCommands),
        );
        let shared = bus.deliveries().clone();

        let refused = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            CommandBus::new(
                CommandRoster::new(),
                shared.clone(),
                Arc::new(NoLocalCommands),
            )
        }));

        assert!(
            refused.is_err(),
            "premise: the second sweeper request is refused"
        );
        assert!(
            !shared.inner.is_poisoned(),
            "the refusal poisoned the live daemon's shared table — every later command panics at the lock"
        );
        // If poisoned `lock` panics and dies on this line — the property is bitten once more as a function.
        assert_eq!(shared.in_flight(), 0, "the table must stay usable");
    }

    /// ★A relay that hit the cap is refused **taking nothing**★ — before the seat, the task or the roster entry exists.
    /// This door has no connection lifetime (it stands anew per request) so it lacks the natural cap the socket entrance has. Without a cap what one round trip holds for the deadline (a task + a seat + a roster entry + an argument copy) piles up without bound.
    #[tokio::test]
    async fn a_relayed_call_past_the_ceiling_is_refused_without_taking_anything() {
        let (roster, mut owner_inbox) = roster_with_owner(1, "tab.create");
        let (bus, _sweeper) = bus_with(roster, Arc::new(ManualClock::new()));
        bus.relayed_counter()
            .store(MAX_RELAYED_IN_FLIGHT, Ordering::SeqCst);

        let reply = answered(invoking(&bus, RequestId::new(), "tab.create")).await;

        assert_eq!(
            reply.outcome.expect_err("refusal").code(),
            ErrorCode::Conflict
        );
        assert!(
            owner_inbox.try_recv().is_err(),
            "the refusal must come before the envelope goes out"
        );
        assert_eq!(bus.deliveries().in_flight(), 0, "no seat is opened");
        assert!(
            !bus.roster().is_attached(FIRST_CONTROL_CALLER),
            "no roster entry is made"
        );
    }

    /// ★The cap **comes back when the round trip ends**★ — otherwise a daemon that filled once refuses relays for the whole process lifetime.
    /// What gives it back is the destructor of [`RelaySlot`], so it is the same however it leaves — cancellation · early return · panic.
    #[tokio::test]
    async fn the_relay_ceiling_is_given_back_when_the_round_trip_ends() {
        let (roster, mut owner_inbox) = roster_with_owner(1, "tab.create");
        let (bus, _sweeper) = bus_with(roster, Arc::new(ManualClock::new()));
        let request_id = RequestId::new();

        let call = invoking(&bus, request_id, "tab.create");
        let _ = envelope_from(&mut owner_inbox).await;
        assert_eq!(
            bus.relayed_counter().load(Ordering::SeqCst),
            1,
            "during a round trip it holds one permit"
        );

        bus.deliveries()
            .complete(CommandReply::ok(request_id, json!({ "tab": "t-1" })));
        assert!(answered(call).await.outcome.is_ok());

        assert_eq!(
            bus.relayed_counter().load(Ordering::SeqCst),
            0,
            "when the round trip ends the count must come back"
        );
    }

    /// ★Even leaving by cancellation **all three axes must be empty**★ — roster entry · cap count · **seat**. An HTTP handler future is dropped when the client drops.
    /// Reverting to straight-line code (`attach` → `await` → `detach`) turns this test red: the lines after a dropped future do not run a single line so that entry stays for the whole process lifetime, and [`CommandRoster::sink_of`] walks that table linearly
    /// holding it locked on **every delivery**.
    /// ★Do not remove the seat axis here — that axis is the most expensive★: if a cancelled seat is not released it holds a copy of the envelope's name · arguments **for the whole process lifetime** ([`Pending::args`]). Moreover [`CommandDeliveries::open`]
    /// calls [`CommandDeliveries::expire`] on every delivery and that function walks the table twice, so a leftover seat prices every command at **both** entrances. The version without this axis (looking only at the roster · count) was green while carrying that leak.
    #[tokio::test]
    async fn a_cancelled_control_call_leaves_no_seat_or_pseudo_connection_behind() {
        let (roster, mut owner_inbox) = roster_with_owner(1, "tab.create");
        let (bus, _sweeper) = bus_with(roster, Arc::new(ManualClock::new()));

        // Dropping that future while it hangs on an owner that does not answer = the same causality as the caller dropping.
        let call = invoking(&bus, RequestId::new(), "tab.create");
        let _ = envelope_from(&mut owner_inbox).await;
        assert_eq!(
            bus.deliveries().in_flight(),
            1,
            "premise: one seat is open at the moment of cancellation"
        );
        call.abort();
        let _ = call.await;

        // The abort may not be observed at once — look at whether it vanishes within a cap (if it does not vanish it never will).
        for _ in 0..200 {
            if !bus.roster().is_attached(FIRST_CONTROL_CALLER)
                && bus.relayed_counter().load(Ordering::SeqCst) == 0
                && bus.deliveries().in_flight() == 0
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!(
            "what the cancelled control route call left behind: roster={} count={} seat={}",
            bus.roster().is_attached(FIRST_CONTROL_CALLER),
            bus.relayed_counter().load(Ordering::SeqCst),
            bus.deliveries().in_flight()
        );
    }

    // ── stripping the raw text of the post-deadline result log (TRD S21 usage-limit-slot §1-4 bus amendment 4) ───────────────
    //
    // ★Calls the post-deadline branch directly★ — what is measured is "what that branch carries", and the delivery path that passes the deadline is already measured above.
    //   ★All three hit that callsite only inside capture★ (`log_capture` "Observation conditions" ①).

    const UPSTREAM_TEXT: &str = "upstream said: quota blob 7f3a";

    /// A `usage.*` answer built with the real row type — if the field name (`upstream`) changes compilation stops here.
    fn usage_payload() -> serde_json::Value {
        use engram_dashboard_agent::commands::{
            AgentBackend, UsageServedWord, UsageStateWord, UsageVendorRow, UsageWindowRow,
            UsageWindowWord,
        };
        serde_json::to_value(UsageVendorRow {
            backend: AgentBackend::Codex,
            account_key: "default".to_owned(),
            plan: None,
            windows: vec![UsageWindowRow {
                window: UsageWindowWord::FiveHour,
                label: None,
                used_pct: Some(40.0),
                left_pct: Some(60),
                resets_at: Some(1_900_000_000),
                age_secs: 3,
                expired: false,
            }],
            in_flight: false,
            served: UsageServedWord::Cached,
            state: UsageStateWord::Failed,
            next_attempt_in_secs: Some(70),
            retry_in_secs: None,
            detail_kind: Some("rpc_error".to_owned()),
            detail_code: Some(-32_603),
            upstream: Some(UPSTREAM_TEXT.to_owned()),
        })
        .expect("row serialization")
    }

    fn late_success(name: &str, payload: serde_json::Value) -> LocalJoin {
        Ok((
            envelope(name, &OwnerToken::new("whatever")),
            Some(Ok(payload)),
        ))
    }

    /// ★The post-deadline success of a held Write (`usage.refresh`) carries the payload but with the upstream raw text removed★ — the other fields remain
    /// (the reason that line exists = a clue to recover the result the caller does not know).
    #[test]
    fn a_late_usage_row_is_logged_without_its_upstream_text() {
        let payload = usage_payload();
        assert!(
            payload.to_string().contains(UPSTREAM_TEXT),
            "premise: the row carries the raw text"
        );
        let request_id = RequestId::new();

        let ((), logged) = crate::log_capture::capture_loud(|| {
            log_late_local(
                late_success("usage.refresh", payload),
                true,
                request_id,
                "usage.refresh",
                ENTRANCE_SOCKET,
            )
        });

        let [line] = logged.as_slice() else {
            panic!("must be one line: {logged:?}");
        };
        assert!(
            !line.contains(UPSTREAM_TEXT),
            "the raw text sat in the log: {line}"
        );
        assert!(
            !line.contains("\"upstream\""),
            "stripped key and all: {line}"
        );
        for kept in [
            "\"state\":\"Failed\"",
            "\"detail_kind\":\"rpc_error\"",
            "\"detail_code\":-32603",
            "\"left_pct\":60",
        ] {
            assert!(line.contains(kept), "{kept} must remain: {line}");
        }
    }

    /// ★Stripped regardless of command name and regardless of depth★ — splitting by name (only those starting with `usage.`) leaves a hole when another command carries the field, and looking only at the top leaks raw text inside arrays and nested objects.
    #[test]
    fn the_upstream_scrub_ignores_the_command_name_and_reaches_nested_keys() {
        let payload = json!({
            "agent_id": "a1",
            "upstream": "TOP-LEVEL-ORIGINAL",
            "rows": [{ "keep": 7, "detail": { "upstream": "NESTED-ORIGINAL" } }],
        });

        let ((), logged) = crate::log_capture::capture_loud(|| {
            log_late_local(
                late_success("agent.new", payload),
                true,
                RequestId::new(),
                "agent.new",
                ENTRANCE_SOCKET,
            )
        });

        let [line] = logged.as_slice() else {
            panic!("must be one line: {logged:?}");
        };
        assert!(!line.contains("TOP-LEVEL-ORIGINAL"), "{line}");
        assert!(!line.contains("NESTED-ORIGINAL"), "{line}");
        assert!(!line.contains("upstream"), "{line}");
        assert!(line.contains("\"agent_id\":\"a1\""), "{line}");
        assert!(line.contains("\"keep\":7"), "{line}");
    }

    /// The post-deadline success of a read (`usage.get`) does not hold a number so it is **one debug line with no payload** — the existing rule that splits not by name comparison but by `retained`.
    #[test]
    fn a_late_usage_get_is_a_payloadless_debug_line() {
        let logged = crate::log_capture::capture_levels(|| {
            log_late_local(
                late_success("usage.get", usage_payload()),
                false,
                RequestId::new(),
                "usage.get",
                ENTRANCE_SOCKET,
            )
        });

        let [(level, line)] = logged.as_slice() else {
            panic!("must be one line: {logged:?}");
        };
        assert_eq!(*level, tracing::Level::DEBUG, "{line}");
        assert!(
            !line.contains("outcome="),
            "the payload is not carried: {line}"
        );
        assert!(!line.contains(UPSTREAM_TEXT), "{line}");
        assert!(line.contains("usage.get"), "{line}");
    }
}
