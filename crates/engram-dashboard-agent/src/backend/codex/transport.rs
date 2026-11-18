//! CodexAppServerTransport — the [`AgentTransport`] implementation for the `codex app-server --stdio` child process.
//!
//! ★What this file owns★: the child process + Job Object · the single stdin writer · the reader (pump) · the waiting table of
//!   outgoing requests and the id counter · the thread id and readiness · the gated input queue · the trigger that hands held text at a tool end
//!   of the running turn ([`State::steer_due`] — ADR-0235). The id · method · order of outgoing envelopes are
//!   all held here — so the bytes [`AgentTransport::send_input`] receives are not a wire frame but
//!   **the message body** ([`crate::backend::InputEncoder::TransportFramed`]).
//!
//! ★The three threads and the walls between them★:
//!   - **Reader** (= pump, launched by [`AgentTransport::start`]) — owns the decoder exclusively as `&mut`. The memory of approval
//!     items we refused ([`RefusedItems`]) is also held by the reader alone — there is no lock (ADR-0241).
//!     ★It never takes the stdin lock and never waits on the waiting table★. When a line that needs an answer comes, it **puts it in the outbox and
//!     returns immediately.** Violate this and reading stops, and when reading stops the peer's queue fills, and the peer does not close
//!     its stdout write handle, so **EOF never comes** — the process is alive and there is not a single signal.
//!   - **Writer** (itself) — the **only** thread that holds the stdin lock throughout a blocking write, and also the **only**
//!     thread that drains the control queue. The handshake runs here too (meanwhile the queue holds user input). ★Even
//!     during that wait it keeps draining the control queue★ — if it parked entirely, a refusal the reader put in could not go out, and if the peer was
//!     waiting for that answer, both sides would wait for each other until the deadline.
//!   - **stderr drain** — keeps the child from stopping because the pipe filled.
//!
//! ★Lock order = state → waiting table★ (ADR-0006). There is no reverse: [`Pending`] does not call outside while holding its own
//!   lock. There is also no place that takes the stdin lock while holding the state lock.
//!
//! ★Teardown is today's causality as is (ADR-0001 2 verbs)★ — no wait is added inside [`AgentTransport::shutdown`],
//!   and ★**on that path** stdin is not closed before kill★ (source of truth for the reason = the same spot in `transport/stdio.rs`).
//!   ★**This is not a whole-file rule but a rule of the teardown path — do not misread it either way**★:
//!   ① on the branch where the handshake failed, [`writer_loop`] closes stdin **without kill** — ★not only on a record failure
//!   but **also on a round-trip failure (including refusal)**★ (the comment at that spot is the source of truth for why it is safe there). Do not use
//!   this sentence to revert its existence. ② Conversely, do not use that new spot as grounds to
//!   flip the order of `shutdown()` either — the reasons the two spots are safe **differ**, and that side's order is
//!   a real regression guarded by [`tests::shutdown_completes_even_if_a_write_blocks_on_a_full_pipe`].
//!   Nobody joins the writer thread, and ★it ends on its own on seeing the closed marker (`State::closed`) — there are **two** places that
//!   set that marker★: `shutdown()` (we killed it) and the `Drop` of [`ReaderExit`] (the peer ended on its own
//!   or the reader panicked — being `Drop`, unwinding passes it for sure). In the former case killing the child breaks the pipe,
//!   and a blocked write is released with an error. ★The price = unsent input left in the queue quietly disappears★.
//!
//! ★Known limits — do not shrink this list; remove items together with fixing them★:
//!   - **The turn itself has no deadline.** Only the `turn/start` **response** has a deadline, so if a turn
//!     ends leaving only an `error` notification without `turn/completed`, the gate never opens. After that,
//!     [`AgentTransport::send_input`] reports it as "the queue is full" — a report with the wrong reason.
//!   - **Non-Windows has no means of reaping grandchildren.** Neither Job Object nor process groups are used.
//!   - [`AgentTransport::interrupt`] does not look at the closed marker — there is a window where it returns `Ok` for one line
//!     that will be dropped at the moment of closing.
//!   - **History restore is best-effort as a whole** — whether refusal · deadline · line cap, the outcome is 「restore only
//!     up to there」 and the connection stands as is ([`hydrate_history`]). So **how much was restored remains only in the log
//!     and has no indication on screen** — a person reads the shortened screen as 「it was only this much」.
//!   - ★**Restored history and live lines can mix — what prevents it is not structure but measurement**★:
//!     the reader runs even during the handshake, so if the peer streams notifications in that window, those lines mix with the history.
//!     ★★There are **two** mixing axes and only one is closed — writing only 「splitting is closed」 is false★★:
//!     ① **the order inside the ring** is closed — [`OutputCore::emit_batch_without_turn_observation`] carries the whole history
//!        in one lock section. So a subscriber that **attaches later** always receives the intact history by replay.
//!     ② **the delivery order to an already attached subscriber is not closed** — that door sends after letting go of the ring lock,
//!        so a live emit in between can cut in as [batch seq N] → [live seq N+k] → [batch seq N+1].
//!        ★And the frontend handles that **by discarding, not reordering**★ — the subscription callback of `RichSlot`
//!        drops `chunk.seq <= lastSeq`, so the history frames after the cut-in **disappear from the screen**.
//!     That is, today both 「stands in front」 and 「splits during delivery」 are open, and what actually prevents them is
//!     only the measurement below. There is one way to close both axes together — **the reader holding emits bounded**
//!     until the gate opens. Deciding that queue's cap and the disposal on overflow (silent loss) comes first, so it is a separate matter.
//!     ★Write precisely what the containment is — that is the point of this item★: ADR-0203 measured `item/*`
//!     notifications after resume as **0** (0.154.0), and the place that opens a turn requires [`Link::Ready`], so there is
//!     no turn of ours yet either. ★That is, it is **an observation of vendor behaviour**, not a structural guarantee★ — codex updates itself and this
//!     protocol has no version field (another item of this header already says so), so that 0 can quietly
//!     change. If it changes, the symptom is 「one strange line above the restored conversation」, and closing it requires the reader to **hold emits
//!     bounded** until the gate — that queue's cap and the disposal on overflow (silent loss) must be decided first, so it is
//!     a separate matter.
//!     ★That separate matter was **closed as not to be built** (ADR-0205) — this limit stays open, and that ADR holds the reopening trigger too★.
//!   - If the connection cannot stand, the input queued disappears. **How many disappeared remains both in the log and on screen** —
//!     the `detail` of the failure outcome carries that count (that branch of [`writer_loop`]). ★What this spot once said, 「it remains only in the log
//!     (all that shows on screen is "handshake failed")」, is a stale description★ — the count is carried,
//!     and the headline is not one but two ([`HandshakeFailure::headline`] — it writes a protocol round-trip failure and a session id record
//!     failure separately).
//!   - The pump is launched with a panicking `thread::spawn` (the other two absorb failures into the log).
//!   - **One blocking write can hang forever** — if the peer does not read our stdin, `write_all`
//!     stops on pipe backpressure, and the only way out of it is `shutdown()`'s kill. Neither a deadline
//!     nor a condition variable reaches inside that one write. ★This is a limit over this whole file,
//!     not only the handshake★.
//!   - **The codex writer ([`writer_loop`]) is not contained (ADR-0292)** — in the production process, a writer panic
//!     anywhere has the hook end the process. Below is the shape that stands only in a test build without the hook: the containment is only the one place, the `catch_unwind` of
//!     [`record_session_id`] wrapping the session id record call, and **the rest of the writer code has none.** If the writer panics outside
//!     the containment, the state already standing at that moment remains: **before** the handshake the link is `Connecting`
//!     and the closed marker does not stand either, so [`AgentTransport::send_input`] keeps returning `Ok`, fills the queue up to the cap,
//!     and reports that stop as "the queue is full" (the same reason mismatch as the first item above). **After** the handshake the link
//!     is already `Ready`, so the same symptom happens on the turn path. ★Either way the closed marker does not stand, so neither the pump
//!     nor the reaper moves★.
//!   - **The slots that hold an end that came before the response are [`EARLY_COMPLETION_SLOTS`]** (each slot carries the turn id
//!     together with **the turn boundary that line made** — [`EarlyCompletion`]). If more ends than that stream in while waiting for one
//!     response, the oldest are dropped first, and if a dropped one was our turn's, that
//!     turn has nothing left to end it again. Only one turn is open, so one slot suffices in normal operation, but
//!     if the peer streams ends of turns we never opened, it is eaten into that much. ★Those slots are emptied entirely at every place
//!     where the turn returns to `Idle`★ — source of truth for the reason = that line of [`end_turn_if`].
//!   - **Job Object enrolment is **after** spawn, so a grandchild created in between is outside the Job.** Once enrolled,
//!     breakaway is blocked (neither `BREAKAWAY_OK` nor `SILENT_BREAKAWAY_OK` is turned on) and the tree goes down as a whole,
//!     but descendants born in that window are outside that guarantee. ★This window is not this link's alone★ — `pty.rs` · `stdio.rs` have
//!     the same shape. Fixing it is a separate matter touching all three links together, and the precedent is the usage query runner — it launched suspended,
//!     put it into the Job, then woke it, closing that window (`usage::process` + OS layer `spawn::prepare_tree_root` ·
//!     `spawn::TreeRoot`).
//!   - **If the handshake fails, [`writer_loop`] lets go of our stdin — regardless of the branch.**
//!     ★This spot once said 「the child · reader · writer remain and stay resident until the manager reaps them」. That
//!     was beyond stale, it was **false — nobody reaps that session**★: what opens the reaping is the one message the pump's
//!     [`OutputCore::finish`] emits, and the reaper is its sole consumer (ADR-0019), so if the reader does not see EOF,
//!     that message **is never created at all.** So that shape was not 「resident until reaped」 but
//!     **a permanent wedge** — the screen says `Running` but all input is refused, and the child + Job Object + three threads are
//!     held for the whole daemon lifetime. On a boot restore, as many stand at once as there are resumable codex profiles.
//!     ★Still, this link does not call kill★ — it only lets go of the write end, and the end transition is still the pump's
//!     alone (ADR-0005 as is; the 2 verbs of ADR-0001 belong to the `kill` handler). ★What actually cuts that wedge
//!     is the manager's teardown★ (the 「recovery」 item below).
//!   - ★**The case where the peer closes only stdout and keeps living has no owner yet**★ — the reader sees EOF and sets `closed`,
//!     and the session is reaped, but the child is alive. On Windows, when the link is dropped the Job handle closes and
//!     `KILL_ON_JOB_CLOSE` reaps the tree, but **non-Windows has no such backstop and an orphan remains**
//!     (the parent version of 「non-Windows has no means of reaping grandchildren」, which this header already writes).
//!     ★**Production reachability differs per branch — read it neither as 「all dead code」 nor as 「all running」**★:
//!     a session id **record** failure comes only from a panic caught by [`record_session_id`], and that spot is outside the containment
//!     marking, so in the production process the hook ends the process at that spot (ADR-0292) — so
//!     the 「session id record failed」 headline of [`HandshakeFailure::headline`] and the 「this incarnation is not used」 `detail`
//!     are observed only in a test build without the hook. **Closing stdin is different** — it is reached in production too by a round-trip failure (deadline · decode failure · and the peer's
//!     **refusal**). ★**Do not point to them by ordinal**★ — the branch order changes.
//!   - **The first link of recovery after a handshake failure is *the peer's* action, and if that does not come, the manager is what cuts it** —
//!     what actually ends the session after letting go of stdin is 「the peer sees EOF and exits on its own」 (measured
//!     41–51ms — ★that figure was measured on the **record failure branch**, and the refusal branch is unmeasured★), and only when that happens does
//!     reader EOF → pump → reaper follow.
//!     ★Even if that premise breaks, this link **kills nothing**★ — instead [`Link::Down`] is immediately
//!     visible outside via [`AgentTransport::link_state`], and the activation decision confirms it as a failure and then runs the
//!     existing teardown (ADR-0001 2 verbs). ★Do not bring that disposal back into this file — ADR-0199 explicitly forbids
//!     the writer's `child.kill()` · `TerminateJobObject`★ (it came in once and was removed).
//!     ★**The remaining hole**: in a spawn where that decision is **not hooked** (codex launched as Fresh), nobody looks at that `Down`
//!     — that branch still has no owner.★ codex updates itself and this
//!     protocol has no version field (another item of this header already says so), so that behaviour can quietly
//!     change. **Do not shrink this item** — to fix it, first decide who carries the bounded wait and the disposal after it.
//!   - **The session id record port is called synchronously on the writer thread** — if that callback blocks, that thread is stuck and
//!     neither input nor control lines go out. ★**This is real, not potential**★: the assembly point (`manager.rs`) passes the port to every
//!     spawn, and that implementation takes the profile registry lock and rewrites `agents.json` entirely.
//!     That is, this link can stop at the end of the handshake for **as long as someone else's disk write**. Meanwhile, multiplied with the item above where control lines go out
//!     only one per slice, it makes the [`OUTBOX_LIMIT`] refusal dropping **long**.
//!   - **The real cap of an outgoing request is `budget + SWEEP_INTERVAL`, and that clock starts *after* the first write.**
//!     The deadline is first read only after one slice of the first `recv_timeout`, and the request write before that is itself not bounded.
//!   - **During the handshake, control lines go out only one per slice.** If server requests pile up faster than that,
//!     they reach [`OUTBOX_LIMIT`] and refusals start being dropped — it is visible on screen and ends with the handshake deadline, but
//!     the missed obligations in the meantime are real.
//!   - The attribution of `turn/completed` leans on **the turn id string the peer gave**. If the peer sends back the id it is using now as is,
//!     the wrong turn closes — it is not a value we issued, so there is nothing more this layer can count.
//!   - **An unattributed `turn/completed` cannot emit a turn boundary** ([`Reader::note_turn`]). So in a combination where the peer sends only ends
//!     without a turn id, that turn remains open both in this link and in the fact layer — if the response already
//!     came, there is no deadline backstop either. ★Still, the point of this shape is that both axes get **the same** decision★: drawing only the screen
//!     as 「ended」 while this layer, which holds the queue, does not take the next input would be a worse mismatch.
//!     ★That 「same decision」 holds only if it **applies to progress too**★ — gating only the end and leaving progress open, one `item/*`
//!     line that came while no turn is open would turn on only the fact layer, and its end would be blocked per the sentence above with nobody able to turn it off
//!     (the link is `Idle` = queue open, but the fact layer is busy). [`Reader::claims_our_turn`] carries that split —
//!     the body always goes to the screen, but whether to count it as grounds for 「mid-turn」 is split by the same axis.
//!
//! tauri import 0.

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
/// ★Applied to every request without exception★ (measured 0.154.0): the server **gives no answer at all to an envelope it could not decode,
/// even if it carries an `id`** — an answer comes only for a parameter error after the envelope was read. So a waiting table without a deadline
/// hangs forever not "someday" but **during normal operation**.
/// ★Grounds for the value★: the observed round trips are `initialize` ≈130ms · `turn/interrupt` ≈18ms. 30 seconds is over 200× the slowest
/// of them, so there is no room to cut a normal round trip, and it also generously covers `thread/start` being slow because it launches
/// its own MCP children (measured — those children are our grandchildren, so the Job Object reaps them together). At the same time
/// the error shows up before a person in front of the screen decides "it froze".
/// ★Unrelated to the length of the turn★ — the **response** of `turn/start` comes when the turn opens, not when it ends.
const REQUEST_DEADLINE: Duration = Duration::from_secs(30);

/// The cap on **the whole thing until the connection stands** — two round trips + the record call + history paging. ★Applied not to one request but to one
/// section★.
///
/// ★★History paging running **inside** this budget is the contract (ADR-0203)★★: giving it a separate budget would delay
///   `Ready` by up to two budgets and break the backstop contract below, and then **a resume that succeeded but was busy fetching the screen
///   would be decided as a failure.** That the clock is started in one place, [`writer_loop`], is the concrete form of it, and
///   [`tests::the_handshake_and_the_history_share_one_budget`] measures it.
///   ★The price = if the handshake was slow, the restored screen gets shorter★ — leaning that way is intended.
///
/// ★This value now **does not need to fit any window**★ — the activation decision does not watch a window but receives the connection outcome
///   this link emits **on the spot** (user decision — [`AgentTransport::link_state`]). That is, the purpose of the cap narrowed from
///   「emit the outcome before the decision」 to **「give up on a peer that does not answer, eventually」**.
///   ★This spot once had 「smaller than the manager's decision window」 written as a contract — that window is gone, so it is stale★.
/// ★Still, the value is not raised★: reverting to [`REQUEST_DEADLINE`] (30 seconds) makes two round trips up to 60 seconds, and
///   there is nothing gained by waiting that long — the observed round trip is `initialize` ≈130ms and a refusal comes **immediately**
///   (measured — a child-thread `thread/resume` returned `-32600` right away). 10 seconds is over 70× that normal round trip,
///   covering even a slow startup (npm shim → node), and the outcome arrives before a person feels 「it froze」.
/// ★Only one contract remains: it must be **smaller** than the manager's liveness backstop★
///   ([`crate::manager::LINK_RESOLUTION_BACKSTOP`]). That one is not a decision but **giving up**, so if this value were larger,
///   a refusal with a reason would be covered by a give-up without a reason. That relation is inside the same crate, so the test bench measures it directly.
/// ★★**This cap applies only to 「waiting for a response」 — it does not apply to writes**★★: if the peer does not read our stdin,
///   `write_all` of [`write_line`] hangs forever on pipe backpressure, and meanwhile this budget is never
///   evaluated (the write of the `initialized` notification between handshake steps is the same spot). On that path the link
///   stops at `Connecting` forever, and **what cuts it is not this link but the backstop above and the teardown after it.**
///   That fact is the only reason the backstop still exists — to remove it, making writes bounded comes first.
/// ★Turn requests do not use this value★ — they use [`REQUEST_DEADLINE`] as is.
const HANDSHAKE_BUDGET: Duration = Duration::from_secs(10);

/// The period at which the writer **looks around**. Two places use this value — the deadline-expiry sweep, and sending control lines
/// **one line at a time** while the handshake waits for an answer ([`request_blocking`]).
///
/// ★The two differ in nature★: the sweep is one map scan, while draining control lines is **a blocking write**. So this value
/// means not 「one slice takes this long」 but 「**unless blocked**, decide again at this interval」.
/// The fact that one blocking write can hang forever is carried not by this constant but by the 「known limits」 of the module header.
/// ★Grounds for the value★: it is 1/60 of [`REQUEST_DEADLINE`], so the deadline error does not change a decision, and the delay until a server request
/// refusal goes out during the handshake is also bound to that much.
const SWEEP_INTERVAL: Duration = Duration::from_millis(500);

/// The queue cap of control lines that go out without the gate (server request refusals · `turn/interrupt`).
///
/// ★Grounds★: these lines go out right away regardless of the turn state, so they do not pile up as long as the peer reads stdin. If they pile up,
/// the peer **stopped reading**, and then this cap turns a "stop without a signal" into an observable refusal. 64 is
/// small enough to expose that stop within a short time, and large enough to absorb a moment when approval requests rush in.
const OUTBOX_LIMIT: usize = 64;

/// The number of items requested per history page.
///
/// ★★This value **does not bound the line size — do not read it that way**★★ (measured): what `limit` cuts is the count, and
///   the bytes of one item are decided by the peer. In the measured sample one `limit:50` page was 1,823,509B, and
///   even at `limit:1` the largest page was 437,196B (one `commandExecution.aggregatedOutput`).
///   That is, no `limit` guarantees staying under [`MAX_LINE_BYTES`].
/// ★So what this value weighs is **the trade-off between the probability of exceeding that cap and the number of round trips**★ — 25 is about half
///   the largest observed page (1.8MB@50), so there is 4× left to the line cap, and it fills the 2MiB ring within a few round trips.
/// ★If exceeded and the line is dropped, that request gets no answer until the deadline, and the restore ends **only up to there**★ —
///   quiet and harmless (the disposal of that branch is carried by [`hydrate_history`]).
const HISTORY_PAGE_LIMIT: u32 = 25;

/// The maximum number of round trips history paging runs — **a runaway backstop, not a policy**.
///
/// ★There are three normal ends★: cursor exhausted · ring filled · handshake budget exhausted. This value fires only when the peer
///   keeps giving a cursor that never ends, and even then the budget usually cuts first.
/// ★Grounds for the value★: multiplied by [`HISTORY_PAGE_LIMIT`] it is 6400 items, over the ring's count ceiling (4096) —
///   that is, this cap never prevents carrying what could be carried.
const MAX_HISTORY_PAGES: usize = 256;

/// The cap on the input the link holds (`State::pending`). Over it, that call returns `Err` (ADR-0190 — it does not drop).
///
/// ★Grounds★: one entry is one complete user turn. The windows in which the queue stands are only the handshake (≈130ms) and one ongoing turn,
/// so 32 entries being full means a person pushed in 32 turns without seeing a single answer — at that point
/// refusing is more honest than taking more.
/// ★Everything is counted regardless of stage★ — an item handed over is also one entry visible in the list until receipt (echo).
// ADR-0231
const INPUT_QUEUE_LIMIT: usize = 32;

/// The bytes the reader reads at a time.
const READ_BUF_BYTES: usize = 4096;

/// The maximum bytes one line can use. A line over it is dropped **alone** and recovery starts from the next newline.
///
/// ★This value is not derived from observation★ — the observed maximum (an error body in the 4KB range) only says **what the floor must not
/// be**, and does not set the cap. What it measures is not the size of a normal line but
/// **the memory a peer that sends no newline can take with this one buffer**, and on that axis 4MiB is the spot that is unnoticeable
/// as an amount one session can hold by mistake and far too small for the peer to exhaust memory.
/// ★Which way it is wrong if off★: too large and that one line is caught late, too small and normal lines are dropped and
/// the screen goes empty — so it leans toward a thousand times the observed maximum.
const MAX_LINE_BYTES: usize = 4 * 1024 * 1024;

/// The cap (in characters) on peer strings carried into logs and the screen.
///
/// ★Grounds★: the 4KB error body above, if not cut, covers a whole log line. With 512 characters, the method name and the first sentence of the reason
/// remain.
const LOG_STRING_LIMIT: usize = 512;

/// The number of slots holding the turn id of an end notification that came before the response.
///
/// ★Grounds★: only one turn is open at a time, so **one slot is needed in normal operation**. The rest is room for when the peer
/// streams ends of turns we never opened, and this value is the cap on that room. Over it, the oldest
/// are dropped first — the later one came, the more likely it belongs to the turn being waited on now.
const EARLY_COMPLETION_SLOTS: usize = 8;

/// The number of slots remembering the item ids of approval requests we refused ([`RefusedItems`]).
///
/// ★Grounds★: the peer emits that item's end right at the spot it receives our refusal (measured fixture `refuse_u2a` — request ·
/// refusal resolution · end come within a few ms). So **0–1 slots are occupied in normal operation**. What remains is only items whose end never
/// comes (a peer that died midway · a turn interrupted while waiting for approval), and this value is the cap on that remainder. Over it, the oldest are
/// dropped first — the late end of a pushed-out item loses the reason that it was our refusal, and a command shows as 「error」, a file change as `Declined` (reason unknown
/// — 「denied · not executed」).
// ADR-0241
const REFUSED_ITEM_SLOTS: usize = 16;

/// The maximum bytes of a turn id to hold. ★It is **filtered, not truncated**★ — this value is not an observation key people read but
/// a **token compared for equality** later, so storing it truncated could make two different long ids read as the same.
/// The observed id is a UUIDv7 string (36 bytes), and 128 is room beyond that.
const MAX_TURN_ID_BYTES: usize = 128;

/// The lowest codex edition that uses the list · receipt notifications — `clientUserMessageId` of `turn/start` · `turn/steer` came in
/// with this edition (tag comparison — the same as ADR-0198's floor). Below it, today's behaviour as is ([`Floor::Below`]).
// ADR-0231
const CLIENT_MESSAGE_ID_FLOOR: (u64, u64, u64) = (0, 140, 0);

// ★That there is no place that branches on error codes is intended — that is also why there is no retry backoff constant★.
//   An error response wakes the waiter of that request whatever its code, so it does not go quietly silent. The three reasons for not adding
//   「resend the same request on a particular code」 on top: ① it is unverified whether an overload code actually comes in this envelope
//   (the `-32xxx` range appears 0 times in the schema, and overload · rate limit come as `codexErrorInfo` of the `error`
//   **notification**) ② the three requests we emit are dangerous to retry — `initialize` can be sent only
//   once (measured: the second is an error), retrying `thread/start` creates two threads, retrying `thread/resume`
//   is also a request that opens the same thread twice, and retrying `turn/start`
//   opens once more a turn the peer already received ③ so to pick a value, seeing that code on a real server comes
//   first. ★What is needed to reopen it★ = an observation of which code comes for which request and when.

/// The notification that a turn ended — ★the state machine input of this link★. Queue release must know that the turn ended, so
/// this layer, which classifies envelopes, reads it as its own job.
///
/// ★The translator also reads the same line and carries it over as a turn boundary (`TurnEnd`) — the two readers do different work★: that one
/// translates 「what outcome this line speaks of」 (codex vocabulary → neutral vocabulary), and this one decides 「whether that outcome **belongs to our
/// turn**」. ★The translator has no material for that decision★ — our thread · turn id exist only in this
/// layer. So translation is always done, but **this layer decides whether to raise that output to the screen** (the `boundary` argument of
/// [`Reader::note_turn`]). Remove that gate, and someone else's turn end closes our turn, later output is split by a fake
/// boundary, and the fact layer observes idleness and plugs mail in mid-turn.
///
/// ★`turn/started` is **deliberately** not read — do not revive it★: that notification **carries not a single identifier we
/// issued**, so in principle it cannot be attributed to a turn. Trying to tell it apart instead by circumstances like 「is the current turn waiting
/// for an answer」, a late notification of the previous turn writes its id into the new turn's slot and does not recover after that
/// (an end notification with that id closes the live turn, the queue opens and turns overlap, and `interrupt` aims at a dead
/// turn). The only thing that decides the turn id is **the `turn/start` response paired by our request id**.
///
const TURN_COMPLETED: &str = method::TURN_COMPLETED;

// ── State machine ─────────────────────────────────────────────────────────────────

/// The connection axis. ★Once cut, it does not come back in this incarnation (ADR-0192)★ — no reconnect and no re-`initialize`.
#[derive(Debug)]
enum Link {
    /// Before or during the handshake. Input is not refused but **queues**.
    Connecting,
    /// The response to the second handshake request (`thread/start` or `thread/resume`) was received and the record call returned —
    /// `turn/start` is allowed from here on.
    Ready,
    /// Cut, or the handshake failed. The reason is quoted as is when refusing new input.
    Down(String),
}

/// The turn axis.
///
/// ★The only thing that fills `turn_id` is the `turn/start` **response**★ — only that response is paired by our request id and
/// attributed without ambiguity (why notifications are not used = [`TURN_COMPLETED`] doc).
/// `seq` = a marker raised every time we open a turn. ★Comparison is only match/mismatch★, and it does one thing — **it prevents something that arrived
/// late from touching the next turn opened in between.** Without this marker, turns overlap on one thread,
/// and the input queue exists to prevent that (ADR-0193).
#[derive(Debug)]
enum TurnState {
    Idle,
    Active { seq: u64, turn_id: Option<String> },
}

/// The floor decision ([`CLIENT_MESSAGE_ID_FLOOR`]) — the handshake decides it once and does not revert it for that incarnation.
///
/// ★User items that came in during `Pending` are held without announcing★ — the side the decision lands on decides them all at once
///   ([`settle_floor`]). Putting them in the list in advance creates list items that meet a below-floor decision (TRD §5-5).
// ADR-0231
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Floor {
    Pending,
    /// The list · synthetic bubble · `clientUserMessageId` · echo comparison stand.
    Above,
    /// Today's path — items are held without identifiers (no list · synthetic bubble · comparison) and leave at the moment the turn opens.
    Below,
}

/// Things living under one lock. ★The "can it send" check and the "transition to in-progress" must be **the same critical section**★
/// (ADR-0193): [`AgentTransport::send_input`] is `&self`, so concurrent calls are structurally possible, and if the two
/// were split, both callers would see idle and each open a turn. That pairing exists in one place only, [`take_turn_locked`].
struct State {
    link: Link,
    turn: TurnState,
    thread_id: Option<String>,
    /// Control lines that do not go through the turn gate. The writer picks them **before** input — if a refusal response waited behind a queued user turn,
    /// the peer would never get the answer to that request.
    outbox: VecDeque<String>,
    /// The input the link holds — in arrival order. A turn opens with the oldest `Held`. Items without an identifier leave at the opening
    /// spot, and items carrying an identifier remain as `InFlight` until receipt (echo) or the turn-end disposal ([`take_turn_locked`] ·
    /// [`State::close_turn_items`]).
    // ADR-0231
    pending: VecDeque<PendingItem>,
    // ADR-0231
    floor: Floor,
    /// The marker of the next turn to open. Only increases monotonically and is never rewound.
    next_turn_seq: u64,
    /// ★An end notification that came before the response★ — at that point it cannot be attributed, so it is held, and compared and ended when the
    /// `turn/start` response that follows tells **our turn's id authoritatively**.
    ///
    /// ★What is held is not 「there was an end」 but **that id**★ — that distinction is why this slot is not a misattribution.
    ///   The ending decision is still only the id match, and no turn ends merely by being here.
    early_completions: VecDeque<EarlyCompletion>,
    /// This link has nothing more to send — the writer sees this and ends its loop.
    ///
    /// ★There are two places that set it★: [`AgentTransport::shutdown`] (we killed it) and the `Drop` of [`ReaderExit`]
    /// (the peer ended on its own or **the reader panicked** — being `Drop`, unwinding passes it for sure). ★The second
    /// out, the writer thread remains forever for every agent that ended naturally★ — that thread holds the `Arc`s of core · stdin · the waiting table,
    /// so one session's worth of memory remains with it, and `shutdown()` is not called on the reaper path, so
    /// nobody wakes it.
    closed: bool,
    /// The tool-end trigger — a tool item of our current turn ended ([`Reader::note_tool_item`]) · or user text held for the turn whose turn id
    ///   just came remains ([`Reader::resolve`]). While it stands, the writer hands the held user texts over as steers one at a time in typing order, and lowers it
    ///   when there is nothing left to hand over ([`take_steer_locked`]) — one tool end hands over all held text (ADR-0235 — 「when one tool
    ///   ends, slip it in」). It is also lowered when the turn closes ([`State::close_turn_items`]). ★The side that sets it does `notify_all` in the same lock
    ///   section★.
    // ADR-0231
    steer_due: bool,
    /// Halt after error — our turn ended abnormally (`failed` · the `turn/start` exit — [`TurnClose`]).
    ///   ★While it stands, Idle opens no turn at all★ ([`State::idle_opening`]) — neither held user text nor mail goes out on its own
    ///   (TRD §5-5 「halt after error」 · user decision N11).
    /// Where it is set = [`State::close_turn_items`] (regardless of turn kind — user · Direct · mail). Where it is lifted = the moment user text comes in
    ///   ([`State::push_pending`]) · link closed ([`ReaderExit`]). Idle after lifting is the usual order, so the held text goes out before the new text
    ///   (what remains goes first — PRD AC24). ★An interruption (`interrupted`) does not set it★ — text held after an interruption goes out at the next Idle.
    ///   The list emptying by ✕ · time do not lift it. ★It does not stand on a below-floor incarnation★ — like this whole feature, that is today's path.
    /// ★It blocks only the turns the link opens on its own★ — the door through which mail enters the link is the kernel's `last_end_failed`, and the codex classifier
    ///   folds the `TurnEnd{Failed}` of the same trigger (`classify_turn`). ★It is a field outside [`TurnState`]★ — expressing the halt as `Active`
    ///   would make the attribution gate count a late echo as our turn and turn the turn table on again (TRD §5-5 「echo the vendor sent」 condition ①).
    // ADR-0231: the halt itself and where it is set.
    // ADR-0235: where it is lifted = the moment user text comes in (decision 4).
    halted: bool,
    /// The 「steer refused」 mark — the marker (`seq` of [`TurnState::Active`]) of a turn whose steer got an error response or could not be written. During that turn
    ///   no held item is handed over ([`take_steer_locked`]). The vendor closed that turn's window (for a write failure, the same
    ///   write would fail again), so no loop of sending again in the same turn is made. The next turn has a different marker and is released on its own.
    // ADR-0231
    steer_refused: Option<u64>,
}

/// What one disposal will emit — gathered under the state lock, and the caller does [`Disposal::emit`] after letting go of the lock (ADR-0006 · logs are also
/// written outside the lock — reason = [`TurnNoteLog`]).
// ADR-0231
#[derive(Debug, Default)]
struct Disposal {
    events: Vec<QueuedInputEvent>,
    /// The trace lines of items returned to the held place (`held_again` — [`STEER_TRACE`]).
    traces: Vec<SteerTrace>,
    /// The halt transition this disposal produced.
    transitions: Vec<Transition>,
    /// The ids of list items returned to the held place — their handed-over mark is read again ([`publish_hand_over`]).
    handed_back: Vec<String>,
}

impl Disposal {
    /// Order: transition → trace lines → list events → handed-over marks. ★Call it before the turn boundary★ — the consumer the boundary wakes sees a tidied list
    /// ([`end_turn_if`]).
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

/// Emits the handed-over mark (`HandedOver`) — ★the value is not decided by the caller but read from the item's current stage★ (`InFlight` = handed over ·
/// `Held` = held). Only list items that finished announcing emit it — Direct · mail · items without an identifier · items already gone · items with a death mark
/// emit nothing (so the mark never precedes that `Queued` in the ring).
///
/// ★After emitting, it reads again and emits again if it changed★ — the handing-over side (the writer — after finishing the write) and the returning side (reader · turn end) emit
///   on different threads after letting go of the state lock, so emitting a fixed value would leave the stale value of the later emitter as the last mark in the ring (handed-over text as
///   「waiting」 · held text as 「sent」 without ✕). The side that changed the stage always calls this, and this stops only when the value read again equals the last emitted
///   value, so the last mark in the ring = the last stage. The same value going out twice is harmless (the reducer just
///   swaps the mark).
/// The state lock is held only while reading (ADR-0006 — emit is outside the lock).
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

/// One transition of the halt after error — gathered under the state lock and written with [`write_transitions`] after letting go of the lock (reason =
/// [`TurnNoteLog`]). ★User text is not carried★ — only the marker (turn) is carried.
/// Levels: set and lift are info (operational events where the turns the link opens on its own halt and resume) · discard is debug (cleanup on link close).
// ADR-0231
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Transition {
    /// Turn `turn` ended with `close` and halted ([`State::halted`]).
    Halted { turn: u64, close: TurnClose },
    /// User text came in and the halt was lifted ([`State::push_pending`] — the writer = [`AgentTransport::send_turn`]).
    HaltLifted,
    /// The link closed and discarded the halt that was standing ([`ReaderExit`]).
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

/// Writes the gathered transitions — ★call it after letting go of the state lock★ (reason = [`TurnNoteLog`]).
fn write_transitions(agent: AgentId, transitions: Vec<Transition>) {
    for t in transitions {
        t.write(agent);
    }
}

/// One `turn/completed` that came before the response — holds the id together with **the turn boundary that line made**.
///
/// ★Why the boundary is held too★: raising that boundary to the screen at this point would make an end not yet attributed close a live
///   turn ([`Boundary`]). But dropping it would leave nothing to raise after the comparison holds, and **that conversation's waiting
///   indicator would spin forever** — even though the outcome was known. So it is only deferred.
struct EarlyCompletion {
    /// The comparison token. ★Holding it unmasked is intended★ — transforming it would break the comparison. Instead it is filtered by length
    /// ([`MAX_TURN_ID_BYTES`]), and passes [`sanitize`] when going out to the log · screen.
    turn_id: String,
    /// The turn boundary the translator emitted from that line. `None` = the translator could not emit a boundary (a malformed line · no translator).
    boundary: Option<OutputEvent>,
    /// That line's status — at the spot where the comparison holds, the items of that turn are disposed (TRD §5-5 「an early end is a turn end too」).
    ///   It is not read back from the boundary because disposal must stand even for a line with no translator or where no boundary could be emitted.
    // ADR-0231
    close: TurnClose,
}

/// The shape in which our turn ended — splits the disposal of that turn's items ([`State::close_turn_items`]).
// ADR-0231
// ADR-0234
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TurnClose {
    /// `completed` of `turn/completed`.
    Completed,
    /// A status of `turn/completed` with unknown meaning · no status (including `inProgress` — a value not carried into an outcome). The disposal is
    ///   the same as `Completed` but ★does not set the halt after error★ — the kernel classifier folds the same line into 「other
    ///   end」 (`TurnOutcome::Unknown` → `TurnEndKind::Other`) and does not set `last_end_failed` as an error end.
    Unknown,
    /// `failed` of `turn/completed`. This and `Rejected` · `Unanswered` are failure ends — they return the user text that turn carried without an echo
    ///   to the held place (the same as other ends) and set the halt after error ([`State::close_turn_items`] · [`State::halted`]).
    Failed,
    /// `interrupted` of `turn/completed` — the vendor drops what was handed over with neither echo nor history (M6). Not a failure end — it does not set the halt
    ///   and does not erase the carried text: held text and text returned to the held place go out at the next Idle (user decision 2026-09-26).
    Interrupted,
    /// `turn/start` did not reach the vendor or the vendor refused it — write failure · error response.
    Rejected,
    /// It is unknown whether the vendor received `turn/start` — response deadline · response decode failure · a turn id over the cap. A failure end, so the disposal is
    ///   the same as `Failed` (if it was received, an echo that came before already closed it as received).
    Unanswered,
}

impl TurnClose {
    /// `turn.status` of the `turn/completed` params.
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

/// The target of the mid-turn handover trace lines — turn on with `RUST_LOG=engram::codex_steer=debug`. When off, not even the material is gathered
/// ([`SteerTrace::enabled`] — at the default level warn, not a single line goes out). One line per event.
///
/// ★Field names are a contract★ — QA reads the log by these names. Every line carries `agent` · `phase` · `item_id`, and `turn_id` only
///   when that turn is known. `phase` is the kind of the line ([`SteerPhase`]):
///   - `tool_start` — `item/started` of a tool item of our current turn (`item_id` = that tool's id).
///   - `tool_end` — `item/completed` of the same tool item. `held` = the number of user items held at that moment.
///   - `steer` — the writer finished writing `turn/steer` for one held item (`item_id` = that item · `request` = request id).
///   - `write_failed` — that `turn/steer` could not be written (fields same as `steer`) — it did not reach the vendor.
///   - `refused` — that steer got an error response (`turn_id` only while that turn is still running).
///   - `echo` — an item of the held list became received by a `userMessage` echo (`turn_id` = the turn the item went into).
///   - `held_again` — a handed-over item returned to the held place because that turn ended without an echo (`turn_id` = the turn it was handed to · `close` = the shape of that end
///     — the [`TurnClose`] name).
/// What QA tells apart with these lines = whether text handed over after one tool ended went in before the next tool (does `tool_end` → `steer` → `echo` come before the next
///   `tool_start`). ★Line order is not guaranteed across threads★ — the reader (`tool_*` · `refused` · `echo`) and
///   the writer (`steer` · `write_failed`) write separately, so order is read from the lines' timestamps. Lines are written after letting go of the state lock (reason =
///   [`TurnNoteLog`]).
// ADR-0231
const STEER_TRACE: &str = "engram::codex_steer";

/// The kinds of [`STEER_TRACE`] lines — the value of the `phase` field ([`SteerPhase::name`] · a QA contract, so it is not changed) and a one-line message.
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

/// One [`STEER_TRACE`] line — the material is gathered inside the state lock and written outside the lock.
// ADR-0231
#[derive(Debug)]
struct SteerTrace {
    phase: SteerPhase,
    turn_id: Option<String>,
    item_id: String,
    /// `tool_end` only — the number of user items held at that moment.
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

    /// The turn id and the tool item id are strings the vendor gave, so they go out through [`sanitize`] — our item id goes through the same door too (the source
    /// is not told apart per field).
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
    /// Our request id — the waiting table key.
    request: i64,
    /// The id of the handed-over item — pairs the response with that item ([`Waiter::Steer`]).
    item: String,
    /// The marker of the turn that item went into = the `InFlight` generation of that item.
    gen: u64,
    line: String,
    /// The id of the turn it was handed to — material of the trace line ([`STEER_TRACE`]).
    turn_id: String,
}

impl Steer {
    /// The trace line of this steer — `steer` (after the writer finished writing · [`writer_loop`]) · `write_failed` ([`steer_write_failed`]).
    /// `None` if the line is off.
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

    /// The held item place of the turn Idle opens next — TRD §5-5 「the order of opening turns = user first」 (PRD §3-9 · user decision N6).
    /// ★Decided by state alone★. Whether a turn can open (`Ready` · `Idle`) is looked at by the caller ([`take_turn_locked`]).
    ///   1. The oldest user `Held` (including Direct · text returned to the held place at turn end). Even before announcing, it is the head — the opening
    ///      side waits for the announcement (it does not skip — typing order).
    ///   2. Otherwise the oldest mail (including items without an identifier).
    /// ★During a halt after error ([`State::halted`]) nothing is opened★ — once the next user text lifts the halt, it is as usual from 1 (the held
    ///   text goes before that new text — what remains goes first).
    /// ★If the floor decision is not `Above`, it is today's order as is★ (oldest `Held` · regardless of source) — a below-floor incarnation is today's behaviour together with
    ///   this whole feature, and the halt does not stand either.
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

    /// Puts one item at the end as `Held` and **decides what to announce**. If it is a user item, it lifts the halt after error ([`State::halted`]
    /// — the returned value = it was lifted this time. The caller writes that transition log outside the lock).
    ///
    /// ★Classification is done with the state **before** putting, in the same critical section as putting★: if `Ready ∧ Idle ∧ pending empty`,
    ///   Direct (synthetic bubble · outside the list); other user items are Queued (list). ★Direct is also put into pending★ —
    ///   if not, the second input of the same idle moment also sees 「empty」 and becomes Direct, creating a branch where an item not even in the list is held
    ///   and blocks new text and mail (TRD §5-5 「Direct items are kept in pending too」).
    /// An item with something to announce is held with `announced=false`, and the announcement is done by [`announce`] outside the lock.
    /// Mail · items without an identifier have nothing to emit. After a below-floor decision, the identifier is dropped (today's path).
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
        // ADR-0235: the halt after error is lifted by the next user text — even if that text leaves afterwards by ✕, it does not stand again.
        origin == InputOrigin::User && std::mem::take(&mut self.halted)
    }

    /// The vendor received that text (`clientId` echo) — removed from pending regardless of stage. ★Receipt wins★:
    ///   an echo, whatever stage it comes in, is the fact that the text went into the history (TRD §5-5 「consumption echo」), so a held item (including text
    ///   returned to the held place at turn end) does not go out again, and for a handed-over item the late response of that request
    ///   becomes a no-op in the [`State::turn_start_carrier`] comparison (generation masking). The returned value = the removed item (`None` = an id the link
    ///   does not know — mail · already resolved · the second echo of the same text).
    // ADR-0231
    fn take_delivered(&mut self, id: &str) -> Option<PendingItem> {
        let pos = self
            .pending
            .iter()
            .position(|p| p.id.as_deref() == Some(id))?;
        self.pending.remove(pos)
    }

    /// Among the items the `turn/start` of marker `gen` carried, the one still waiting for a response — the `InFlight` of that turn whose turn id is
    /// unknown.
    ///
    /// ★There is only one★: one `turn/start` carries only one item (TRD §5-5), and a steer goes out **only after** the turn id is known
    ///   and writes that id on its own item. If the stage already moved on (left by echo · disposed), `None` — the late
    ///   response is a no-op (generation masking).
    // ADR-0231
    fn turn_start_carrier(&mut self, gen: u64) -> Option<&mut PendingItem> {
        self.pending.iter_mut().find(|p| {
            matches!(
                p.stage,
                Stage::InFlight { gen: g, turn_id: None } if g == gen
            )
        })
    }

    /// Disposes one list item (or a refused Direct) for this cause — returns the `Dropped` to emit, and the caller emits it
    /// outside the lock.
    ///
    /// ★Before announce, only the death mark is attached and `None` is returned★ — that item's `Queued` may not be in the ring yet, so emitting
    ///   `Dropped` here could precede `Queued` in the ring. [`announce`] follows **right after** `Queued` and
    ///   removes the item (TRD §5-5 「when a disposal meets an item before announce」).
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

    /// The turn of marker `gen` ended with `close` — disposes that turn's items and returns what to emit (the caller does
    /// [`Disposal::emit`] outside the lock).
    ///
    /// ★Call it in **the same critical section** that returns the turn to `Idle`★ — if they come apart, new text arriving in that gap sees an `InFlight`
    ///   without an outcome and is classified as waiting, and if even one path by which a turn ends misses this, that item remains in pending
    ///   forever and all new text goes to waiting (TRD §5-5 「five exits」 invariant).
    ///
    /// | Item | Any end (`Completed` · `Unknown` · `Interrupted` · `Failed` · `Rejected` · `Unanswered`) |
    /// |---|---|
    /// | User item handed over in that turn (list · Direct) — no echo | Returned to the held place (no outcome event — the list row · drawn bubble stay as is · the list row only re-emits the handed-over mark) |
    /// | User item handed over in that turn — ✕ reached it after the handover | `Dropped{Withdrawn}` |
    /// | Mail handed over in that turn | Removed without an event |
    /// | Held item | As is |
    ///
    /// ★Only receipt (echo) removes handed-over text from the list as received★ — text of a turn that ended without an echo is held again and rides the next turn (Idle order —
    ///   [`State::idle_opening`]). It does not change place, so typing order stays as is. If a late echo comes, it becomes received while held
    ///   ([`State::take_delivered`] — it does not go out again).
    /// ★A failure end does not erase either★ (user decision 2026-09-26 · ADR-0235 — replaces 「erase on failure」) — the returned text and the held text are made
    ///   to wait until the next user text by the halt below ([`State::halted`]). The only thing that splits a failure end from the other ends is that halt.
    /// An item ✕ reached is not held because the user already removed it — returning it would send it in the next turn.
    /// Mail is not held again (today's disposal as is) — it is outside the list, so the kernel carries the outcome of that text.
    /// ★It does not wait for steer responses★ — the echo of an accepted steer comes before `turn/completed` (TRD §3-2 · M10), so at this end
    ///   text without an echo was not accepted. The late response of that request is made a no-op by the generation comparison ([`State::hold_refused_steer`]).
    /// ★The halt after error is also set here★ ([`State::halted`]) — every path by which a turn ends (`turn/completed` · early-end release ·
    ///   the five `turn/start` exits) passes this function, so there is no other place to hook the trigger. `Failed` · `Rejected` · `Unanswered`
    ///   set it (floor-meeting incarnations only), and `Completed` · `Interrupted` · `Unknown` do not touch it (the lifting side = [`State::push_pending`]).
    /// The halt transition goes into [`Disposal::transitions`], and the trace lines of the return (`held_again`) into [`Disposal::traces`]
    ///   (written outside the lock).
    /// ★Write each branch of `close` separately — do not group them with `_`★: when a new end appears, compilation must stop here so that the disposal of that end
    ///   gets decided.
    // ADR-0231
    // ADR-0235
    fn close_turn_items(&mut self, gen: u64, close: TurnClose) -> Disposal {
        enum Fate {
            Keep,
            Held,
            Drop(DropCause),
            Remove,
        }
        // ADR-0235: the tool-end trigger belongs to that turn, so it is lowered with the turn — if left, when the writer opened the next turn before seeing it,
        //   text typed after that turn id came would go out as a steer without a tool end. For text held in the next turn, the place where the turn id comes sets it again
        //   ([`Reader::resolve`]).
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
                // ★Returned to the held place regardless of the shape of the end★ (user decision 2026-09-26 · ADR-0235 — even on error it is not erased but halted):
                //   a failure end sets the halt below, so the returned text waits until the next user text.
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
        // ★Halt after error (TRD §5-5)★ — Idle does not open a turn until the next user text. An interruption does not set it (held text goes out at the next Idle
        //   — user decision 2026-09-26). An end of unknown meaning does not set it either — the kernel classifier folds the same end into 「the rest」
        //   ([`TurnClose::Unknown`]).
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

    /// A steer got an error response or could not be written — the vendor does not have that text (`-32600` 「no turn running」 · review/compaction turn ·
    /// id mismatch · write failure are all the same — [`Reader::resolve`] · [`steer_write_failed`]). The returned value = what the caller emits outside the lock
    /// (`None` = no-op).
    ///
    /// ★It moves only when (id, generation) and **the current stage** match★ (generation masking — TRD §5-5): an item already removed by echo or disposed by the turn end (returned
    ///   to the held place or erased) cannot be moved twice by a late refusal — the 「steer refused」 mark is not attached to that turn either (a refusal of accepted text is no
    ///   grounds that the turn's window closed — the generation masking test measures this).
    /// If they match, the item is returned to `Held` (same place — typing order) and the 「steer refused」 mark is attached to that turn — no loop of sending again in the same turn
    ///   is made, and the next Idle opens it. There is no outcome event (`Queued` is once per id) — only the handed-over mark is re-emitted
    ///   ([`Disposal::handed_back`]).
    ///   ★An item ✕ reached is withdrawn instead of returned★ (`Dropped{Withdrawn}` — the user already removed it, and the vendor never saw that text).
    // ADR-0231
    fn hold_refused_steer(&mut self, id: &str, gen: u64) -> Option<Disposal> {
        let pos = self.pending.iter().position(|p| {
            p.id.as_deref() == Some(id)
                && matches!(p.stage, Stage::InFlight { gen: g, turn_id: Some(_) } if g == gen)
        })?;
        let mut out = Disposal::default();
        // The mark is independent of ✕ — the fact that the vendor refused to put it into this turn is the same.
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

/// One input the link holds. ★The link does not read the registry and decides by these fields★ (ADR-0193).
// ADR-0231
struct PendingItem {
    /// The identifier the list events and the echo comparison use (the `msg_uuid` of the session write). `None` = an item that came in through
    /// [`AgentTransport::send_input`] — without an identifier it can neither be withdrawn nor compared by echo, and it does not go into the list.
    id: Option<String>,
    /// The body after the encoder — the envelope is built by this link.
    body: Vec<u8>,
    origin: InputOrigin,
    stage: Stage,
    /// Is it a list item (an item that emitted or will emit `Queued`)? Direct · mail · items without an identifier are `false`.
    listed: bool,
    /// The announcement (Direct synthetic echo · `Queued`) finished, or there is nothing to emit. ★The writer does not hand over an item for which this is `false`★
    /// — if the handover outran the announcement, the receipt (`Delivered`) would be dropped with an id the registry does not know.
    /// ★`listed=false ∧ announced=false` is 「not decided yet」 before the decision, and Direct after the decision (`Above`)★ —
    /// pre-decision items are all decided again by [`settle_floor`] under the same lock, and Direct stands only after the decision.
    announced: bool,
    /// ✕ came after the handover ([`withdraw_item`] sets it) — the vendor decides the outcome, and if it turns out the vendor did not take it
    /// (steer error response · turn end without an echo), the link withdraws it (`Dropped{Withdrawn}`) instead of returning it to the held place. If set, an overlapping second ✕ is `TooLate` without an event.
    cancel_asked: bool,
    /// The cause of a disposal that arrived before announce — the announcing side follows `Queued` **right after** with a `Dropped` of this cause
    /// (so that `Dropped` never stands before that `Queued` in the ring).
    death: Option<DropCause>,
}

/// The stage of a [`PendingItem`].
// ADR-0231
enum Stage {
    /// Not handed over yet.
    Held,
    /// `turn/start` or a steer was written, and it waits for the echo.
    ///
    /// `gen` = the generation of that request = ★the marker of the turn that request went into (`seq` of [`TurnState::Active`])★ — it rises every time the link
    ///   opens a turn, and a turn id comes only from that turn's `turn/start` response, so it equals 「the number that rises with every new active turn
    ///   id」. There is no separate counter because the two always rise together, so there is no place for them to diverge. A late
    ///   response · echo moves the item only when (id, gen) and **the current stage** match (so an item moved to another stage is not moved
    ///   twice). `turn_id` = the turn that request aimed at (`None` = `turn/start` carried it and its response has not given the turn id
    ///   yet — a steer goes out only after the turn id is known).
    InFlight { gen: u64, turn_id: Option<String> },
}

/// State + the condition variable waiting on it. The writer sleeps here.
///
/// ★The condition variable's `notify` reduces **latency**, it does not carry correctness★ — the writer wakes by a `wait_timeout` of [`SWEEP_INTERVAL`]
/// and looks at the same condition again, so even if every `notify_all` were removed the behaviour would be the same and only the queue
/// would open that much later. What carries correctness is that timeout. ★So do not read it as "without a notification it stops"★
/// — reading it that way leads to quoting a guarantee that does not exist.
type SharedState = Arc<(Mutex<State>, Condvar)>;

// ── List announcement · floor decision ─────────────────────────────────────────────────────

/// The queue of the announcing sides and whether this incarnation's receipt notifications are available — one per link (shared with the writer by `Arc`).
///
/// `order` — ★announcing must be outside the state lock (ADR-0006), and there are two announcing sides★ (the input thread that put the item ·
///   the writer that made the decision). Without this queue, in the gap where both let go of the lock, they would outrun each other's announcements and the list would
///   disagree with typing order. The side holding the queue emits not only its own item but **all pre-announce items** in arrival order.
///   ★Lock order = `order` → state★. ★It emits while holding `order` — an intended exception to ADR-0006 「send without holding a lock」,
///   backed by the contract that `OutputSink::send` does not block★. The reader and `withdraw` do not take this queue — both dispose only items
///   that finished announcing, and only attach a death mark to pre-announce items ([`State::drop_listed`]).
/// `ack` — filled by the floor decision (`Above` = `Available` · `Below` = `Unavailable`). The assembly point carries **the same**
///   `Arc` on the session ([`CodexAppServerTransport::delivery_ack`]).
// ADR-0231
// ADR-0234
struct Announcer {
    order: Mutex<()>,
    ack: Arc<DeliveryAck>,
}

/// Emits pre-announce items in arrival order — list items = `Queued`, Direct = synthetic bubble. Before the decision (`Above`)
/// nothing is emitted (the side the decision lands on calls it in [`settle_floor`]).
///
/// ★Two steps★: gather what to emit under the lock → let go of the lock and emit → take it again for `announced=true` + waking the writer.
///   An item that got a death mark in between is removed here, and `Dropped` follows **right after** `Queued`.
/// ★The synthetic bubble is emitted with `emit_without_turn_observation` — do not change it to the ordinary door★: the classifier would write it
///   as progress and turn 「mid-turn」 on, and if that text's `turn/start` were refused, a turn that never opened would remain 「in progress」 and
///   mail would be blocked until the 30-minute fail-open (ADR-0127). codex's idle decision is carried by the link state (ADR-0193).
/// ★uuid = item id★ — the vendor echo carries the same value as `clientId`, and the accumulator folds them into one copy.
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

/// Writes the floor decision and decides the path of items held before the decision — the writer calls it **once** after the handshake stands.
///
/// `Above` → pre-decision user items become list items and are announced right away as `Queued`. `Below` → every item
///   drops its identifier and goes on today's path (held until the turn end without list · synthetic bubble — user decision N1 (a)).
///   ★So an item already in the list never meets a below-floor decision★.
/// ★Call it **after** the session id record returned — do not bring it forward★: deciding before that would let the record failure branch
///   ([`writer_loop`]) erase items already in the list without an event.
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

/// The floor decision (M8) — `thread.cliVersion` → if absent or unreadable, the first edition of the `initialize` response `userAgent` → if both
/// fail, below the floor.
///
/// ★`userAgent` reads only the edition right after the first `/` — do not change it to 「the last semver」★: the head is our
///   `clientInfo.name`, and the trailing parenthesis holds **our** edition (measured 0.156.1: `engram-dashboard/0.156.1
///   (Windows …) unknown (engram-dashboard; 0.1.0)`).
/// ★If unknown, it falls below the floor★ — carrying `clientUserMessageId` on an unknown edition quietly breaks the echo comparison.
///   A prerelease of the floor edition (`0.140.0-alpha`) is also below the floor (semver order).
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

/// Does the string start with `MAJOR.MINOR.PATCH`, and does a prerelease mark (`-`) follow it?
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

/// Who receives the answer to one outgoing request, and how.
enum Waiter {
    /// The sending thread waits on a channel (handshake only — that thread is not the reader).
    Handshake(mpsc::Sender<Result<Value, String>>),
    /// No thread is waiting, and **the response gives a turn id** — the reader applies it where it receives it.
    ///
    /// ★`seq` is the marker of the turn this request opened★ — without it, a late answer would write its id into the slot of **another turn opened in between**,
    /// and the `interrupt` after that would aim at the wrong turn.
    TurnStart { seq: u64 },
    /// `turn/steer` of item `id` handed to the running turn — only a refusal reaches that item ([`State::hold_refused_steer`]).
    /// `gen` = the `InFlight` generation of that item (= the marker of the turn it was handed to) — so a late answer cannot move an item that moved to another stage.
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

/// The waiting table of requests we issued. ★An id the server issued never enters here★ (TRD §4-4) — the two id spaces
/// inevitably overlap (a server id must be returned as is), and what tells them apart is not the id value but **the envelope shape**.
/// Lookups happen only on `result`/`error` envelopes.
#[derive(Default)]
struct Pending {
    entries: Mutex<HashMap<i64, PendingEntry>>,
    /// No more answers can come — no new waiting entries are taken. ★Without this, one waiting entry registered **after** the link closed
    /// holds its thread until its deadline runs out★ (the handshake issues two requests back to back, so a window of closing
    /// in between really exists).
    closed: AtomicBool,
}

impl Pending {
    /// `false` = already closed, could not register.
    fn register(&self, id: i64, waiter: Waiter, method: &'static str) -> bool {
        self.register_at(id, waiter, method, Instant::now() + REQUEST_DEADLINE)
    }

    /// The branch that takes the deadline as an argument — ★a seam for the test bench to measure expiry without actually sleeping for [`REQUEST_DEADLINE`]★
    /// (ADR-0012). The production caller is only [`Pending::register`] above.
    fn register_at(
        &self,
        id: i64,
        waiter: Waiter,
        method: &'static str,
        deadline: Instant,
    ) -> bool {
        let mut g = sync::lock(&self.entries);
        // ★Read while holding the lock★ — reading outside would let a waiting entry into a table closed in between.
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

    /// ★A string id cannot be ours★ — our counter emits only i64. So it becomes `None` here and
    /// the caller drops it as an "unknown id".
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
    /// Shared by the pump (try_wait) and shutdown (kill+wait). std Child caches the status after wait, so a double wait
    /// is harmless.
    child: Arc<Mutex<Child>>,
    /// ★Only the writer thread takes it★ — if the reader took this lock, the deadlock of the module header would stand.
    /// Only the last step of [`AgentTransport::shutdown`] touches it, with `try_lock` (no blocking).
    stdin: Arc<Mutex<Option<ChildStdin>>>,
    /// Taken in `start()` and moved to the reader. If None, it already started.
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
    /// Filled by `start()` — used by the input thread's announcement ([`announce`]). ★While empty there is nothing to announce★:
    ///   announcement stands only after the floor decision, and the decision is made by the writer `start()` launched.
    // ADR-0231
    core: OnceLock<Arc<OutputCore>>,
    /// Our request id counter. ★Server ids do not enter here★ — the two spaces are separate.
    next_id: Arc<AtomicI64>,
    sid_sink: Option<SessionIdSink>,
    /// The port to call at the moment the peer first echoes a user message ([`Reader::witness_first_turn`]). `None` = nowhere to tell.
    // ADR-0226
    // ADR-0231
    first_turn_sink: Option<FirstTurnSink>,
    /// The port that delivers the connection outcome **once** (`None` = the assembly point did not give one = this axis does not exist).
    /// ★Taken in `start()` and moved to the writer — delivery happens only on that thread, exactly once★.
    link_sink: Mutex<Option<LinkSink>>,
    /// The writer thread handle. ★Nobody joins it — waiting inside `shutdown()` is a contract violation★.
    ///
    /// There is one reason to hold this handle: **it must be visible from outside whether that thread actually ends.** If it does not end,
    /// one session's worth of memory remains with it, and that fact shows up nowhere without a means of observing it.
    writer_handle: Mutex<Option<std::thread::JoinHandle<()>>>,
    /// Whether the output this link carries is a structured stream. ★An injected value (ADR-0044/0030/0191)★ — the link does not know what it
    /// carries, and the side that knows is the backend that picked this mode.
    structured: bool,
    /// The owner of the group of processes this link launched — only one, and no weak handle is handed out either. ★Kept as the last field★ — fields
    /// are dropped in declaration order, so this place decides when the group closes as the link is dropped. Whether it can be moved earlier was not
    /// measured, and no test guards this placement.
    group: GroupOwner,
}

impl CodexAppServerTransport {
    /// **The pump is not launched yet** (in `start`). Returns `child_pid` together.
    ///
    /// `open_params` = the value that goes out as is as the second handshake request ([`ThreadOpen`]). ★Neither the policies nor whether to
    ///   resume are hard-coded by the link★ — which folder to trust as the workspace and with which sandbox · approval
    ///   policy to run, and whether to resume a stored thread, are backend knowledge, so they are injected.
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
    /// carries it in `SpawnParts::delivery_ack` (a separately made one would have the session and the link see different values).
    // ADR-0231
    pub(crate) fn delivery_ack(&self) -> Arc<DeliveryAck> {
        self.announcer.ack.clone()
    }

    /// Plugs in the first-turn port — ★call it before `start()`★ (the reader takes it then). This link's session does not count submissions
    /// (`MidTurnPolicy::TransportOwned`), so if it is missed this incarnation's thread id is not persisted.
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
/// ★Do not call this directly instead of [`sanitize`]★ — the peer string would skip masking and go out to the log · screen.
/// If there are two doors, one of them is bound to be forgotten.
fn clip(s: &str, limit: usize) -> String {
    if s.chars().count() <= limit {
        return s.to_string();
    }
    let head: String = s.chars().take(limit).collect();
    format!("{head}…(truncated)")
}

/// The door external process strings pass through before being carried to the log · screen — ★masking comes before truncation★:
/// the patterns of `mask_secrets` require a certain length after the prefix, so cutting first would cut a credential spanning the boundary
/// below that quantifier and let it escape masking.
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

/// Sends **only one line** queued in the control queue. The returned value = was there anything to send.
///
/// ★One line at a time is the contract★ — [`request_blocking`], which calls this function, must measure the deadline again
/// in between. Letting it drain everything would make that one call indefinitely long, and the deadline **would not be evaluated**.
/// ★It does not write while holding the lock★ — it takes the line out, lets go, then writes (ADR-0006).
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

/// Issues one request and **waits** for the answer. ★Must not be called on the reader thread★ — reading stops while waiting.
///
/// ★It sends control lines even while waiting★: the one calling this function is the writer, and the writer is the **only** thread that drains that queue.
///   If it parked entirely, a server request refusal the reader put in could not go out until this wait ends,
///   and if the peer was waiting for that answer, both sides would wait for each other until the deadline (after that the connection does not
///   come back in this incarnation — ADR-0192). ★Whether 0.154.0 actually puts `thread/start` after an approval request is
///   unconfirmed★ — what is blocked is that structural window.
/// ★Order is the contract — **measure the deadline first, then write one line**★: the write is blocking, so writing first would leave the deadline
///   unevaluated while that one write hangs. Flip that order and this function stops being bounded,
///   and the handshake neither fails nor shows on screen while input keeps being accepted.
/// ★The limit that still remains★: one blocking write can still hang forever, and the only way out of it is
///   `shutdown()`'s kill (module header 「known limits」).
/// `budget` = this request's deadline. Production callers all pass [`REQUEST_DEADLINE`] — taking it as an argument is
///   ★a seam for the test bench to measure boundedness without actually sleeping for that cap★ (the same reason as
///   [`Pending::register_at`], ADR-0012).
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

/// The handshake. Order is the contract — `initialize` → (`initialized`) → **the second request `open` picked**
/// (`thread/start` or `thread/resume`).
///
/// ★`initialize` can be sent only once (measured 0.154.0)★ — the second comes back as an error. So this function
/// runs exactly once per incarnation and is not called again even on failure (ADR-0192).
/// ★If the second request is refused, it is `Err` as is — it does not fall back to a new thread (ADR-0082)★. There is one more codex-side
/// reason: `-32600` of this protocol does not have one meaning — an unknown method, a second `initialize`,
/// and a config error all come with that code (measured 0.154.0 — the source of truth for the first two is this folder's `protocol`, and the third is
/// the real response [`tests::a_peer_error_message_is_masked_before_it_leaves_this_module`] holds). So splitting off 「unknown thread」 by error code
/// and falling back to a new thread would overwrite a still sound handle on an unrelated failure.
fn handshake(
    stdin: &Mutex<Option<ChildStdin>>,
    state: &SharedState,
    pending: &Pending,
    next_id: &AtomicI64,
    open: &ThreadOpen,
    deadline: Instant,
) -> Result<Opened, String> {
    // ★The deadline is applied not per request but to **the whole section**★ — per request, the sum of round trips becomes a multiple of the cap, and
    //   the moment that sum exceeds the manager's backstop, this link's failure cannot take part in the decision
    //   (the doc of [`HANDSHAKE_BUDGET`] is the source of truth for that causality).
    // ★So the deadline is **received from outside**★ — the history paging that runs right after this section ([`hydrate_history`]) shares
    //   the same deadline. Taking `Instant::now()` again here would give each of the two 10 seconds, so the
    //   sum would exceed the backstop, and **a resume that succeeded while fetching history would be decided as a failure.**
    // ★If the remaining share is 0, `request_blocking` returns right away with a deadline expiry★ — that is the only deciding place, so
    //   it is not split here in advance to reach the same conclusion in two places.
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
    // ★This line is the only record that tells upstream drift apart after the fact★ — this protocol has no version field
    //   (`protocolVersion` appears 0 times in the whole schema), and the CLI updates itself.
    tracing::info!(
        user_agent = %sanitize(&init.user_agent, LOG_STRING_LIMIT),
        platform = %sanitize(&init.platform_os, LOG_STRING_LIMIT),
        "codex app-server connected"
    );

    // Sending it is not an obligation (measured 0.154.0 — `thread/start` succeeds even without it). It is harmless, so it is kept
    //   to match the handshake shape.
    write_line(stdin, &protocol::notification_line(method::INITIALIZED))
        .map_err(|e| format!("initialized write failed: {e}"))?;

    // ★Both branches return the same `thread`, so this function has one return★ — on resume too, **the id the peer
    //   gave is raised as is**. It is not rewritten on the assumption that it equals what we sent: if it differs, that different
    //   value is the thread this incarnation actually talks in, and the record verb must receive it so that the next resume opens the right place.
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
            // ★A new conversation has no history to go back to — the response does not even have that field★ (schema 0.154.0).
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

/// What the handshake set up — the thread this incarnation talks in, and the door into that thread's past screen.
///
/// ★The two fields are of **different grades**★: without `thread_id` this incarnation can do nothing (handshake failure).
///   Without `items_backwards_cursor` nothing breaks — it means there is nothing to restore or it is a new conversation, and
///   that absence is today's behaviour of starting with an empty screen as is (ADR-0203: history failure is quiet and harmless).
struct Opened {
    thread_id: String,
    items_backwards_cursor: Option<String>,
    /// The floor decision — applied after the session id record returns ([`settle_floor`]).
    // ADR-0231
    floor: Floor,
}

/// Fetches the resumed thread's past screen **from the end** and carries it over into neutral events (ADR-0203).
///
/// ★★Failure does not create failure★★: this function **emits `Err` on no branch.** Without a cursor,
///   if the peer refuses, if a line exceeds the cap and no answer comes, if the budget runs out — the outcome is always 「the events
///   up to there」. History that could not be fetched is **an empty screen**, not a failed activation (the same grade as the claude-side `.jsonl` read
///   returning an empty `Vec` when the file is absent — ADR-0079).
/// ★★So this call **shares** [`HANDSHAKE_BUDGET`] with the handshake — it does not start a new clock★★:
///   starting a separate one would delay `Ready` by up to two budgets, exceed the manager's
///   [`crate::manager::LINK_RESOLUTION_BACKSTOP`], and **a successful resume would be decided as 「the link could not reach an outcome」.**
///   That regression is of a different grade from the screen getting a little shorter.
/// ★Where it stops = four★: ① cursor exhausted (normal) ② the ring is filled ③ budget exhausted ④ [`MAX_HISTORY_PAGES`].
///   Why ② is this function's ceiling is carried by [`REPLAY_MAX_BYTES`] — fetching more than the ring will keep is pure
///   cost, and that decision axis must be the [`estimate_cost_bytes`] the ring actually uses (estimating on another axis
///   either fetches less and loses screen, or fetches more and only wastes round trips).
/// ★Direction★: it walks **from the newest** with `sortDirection: desc` + the backward cursor resume gave (schema 0.154.0
///   writes that combination directly in that field's description). The `data` inside a page is also newest first, so while gathering it is reversed, and
///   at the end it is flipped once back to time order (measured 2026-09-16: the first page's `data[0]` is the last assistant
///   response, and the end of the last page is the first `userMessage`).
/// ★A translator is made **for this call only**★ — what the pump holds is a `Box<dyn OutputDecoder>`, so it cannot reach doors outside
///   the trait. What splits are two things, the diagnostic count map and the 「first come」 memory of echoed user items, but
///   in history one item comes **exactly once**, so that memory has nothing to do, and the counts are diagnostic only.
///   ★The price = drift seen in history and drift seen live pile up in different maps★ (both go out
///   to the log — the observation key contains the method name, so which side is told apart).
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
        // A new conversation, or the peer gave no entry point — today's behaviour (empty screen) as is.
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

/// The **round-trip-free core** of the function above — how pages are walked, where it stops, and in what order it hands them out.
///
/// ★There is one reason `fetch` is injected: this rule must be measurable alone, without a child process★
/// (ADR-0012). In production what goes in that place is [`request_blocking`] alone, and the test bench builds the pages
/// by hand. ★An `Err` from `fetch` is **non-fatal**★ — what was gathered up to that point is emitted as is.
fn collect_history(
    thread_id: &str,
    entry_cursor: String,
    deadline: Instant,
    mut fetch: impl FnMut(&ThreadItemsListParams, Duration) -> Result<ThreadItemsListResponse, String>,
) -> Vec<OutputEvent> {
    let mut cursor = entry_cursor;
    let mut decoder = CodexAppServerDecoder::new();
    // Piled up newest → oldest. One slot = the events one item emitted + the **group number** that item belongs to.
    // ★An item that carried nothing over takes no slot★ — only then do turn boundaries stand **between turns with content**,
    //   and a history that was all dropped does not become a list of only boundaries.
    // ★★The key being a **number** instead of the original id is a memory cap — but filtering alone was not enough★★:
    //   `turnId` has no wire cap, so if items with long ids come on every page, the piled-up original strings alone
    //   reach hundreds of MiB (the line cap applies only to **one** page, not to this accumulation). Yet folding all ids
    //   over the cap into one `None` slot would **render two different conversations as one turn** — what is lost would rise in grade from
    //   「the comparison gets loose」 to 「the screen lies」.
    //   So the axis is split in two: **distinction** is carried by the number, and **display** by the filtered id.
    //   - Distinction: compared only with **one** original id, the previous item's (`last_raw` — when a new value comes in, the previous one is dropped,
    //     so it is always O(1)). So even ids over the cap become different groups if they differ.
    //   - Display: each group holds only one id, **filtered, not truncated**, by [`MAX_TURN_ID_BYTES`] (`groups`).
    //     If two truncated ids became equal, different turns would fold into one, so they are not truncated (the same decision as
    //     `MAX_ID_BYTES` of `decoder`). The number of groups is at most the number of items, so this side is bounded too.
    //   ★The remaining price = **no id is carried** on the boundary of a turn over the cap★ (the boundary itself stands).
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
                // ★It is `info`, not `warn`★ — this is not a defect but the fact 「restored only up to here」,
                //   and the connection stands normally after this. The reason is still left (the first signal of upstream drift is here).
                // ★★It is kept **knowing that this branch swallows 「the connection died」**★★ — when the reader sees EOF,
                //   the waiting entry wakes with an error and comes here, and the two cannot be told apart by string. So that
                //   decision is carried not here but by the place that opens the gate ([`open_gate`]) — there the state is looked at directly,
                //   so no string guessing is needed.
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
            // ★★The ceiling decision is made **before putting**, and **counting the boundaries to be synthesized too**★★:
            //   ① measuring after putting always exceeds by that one entry. ② without counting the boundaries, in a history with many turns the actual event
            //   count would be up to twice what is counted here (the extreme where every item splits a turn), and the ring would drop half
            //   right on landing — tying the ceiling to the ring constant was meant precisely to remove that mismatch.
            // ★There is **one boundary per new run**★ — turned back to time order, there is one closing boundary per run,
            //   so counting one each time the run changes while walking in reverse gives exactly the total.
            let boundary = new_run.then(|| history_turn_boundary(shown));
            let add_events = translated.len() + usize::from(boundary.is_some());
            let add_cost = translated.iter().map(estimate_cost_bytes).sum::<usize>()
                + boundary.as_ref().map(estimate_cost_bytes).unwrap_or(0);
            // ★The first item is always taken★ — even if one alone exceeds the ceiling, that one is better than an empty screen,
            //   and the ring keeps the newest one by the same rule (the `len() > 1` guard of `Ring::push`).
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
                // ★Only **the previous one** original is held★ — this assignment drops the previous value, so it does not accumulate.
                last_raw = Some(entry.turn_id);
            }
            newest_first.push((groups.len() as u32 - 1, translated));
        }
        if hit_ceiling {
            break;
        }
        match next {
            // ★★If the same cursor is given again, it stops★★ — otherwise it would fetch the same page again up to the page cap,
            //   carrying **several copies of the same conversation**, and those duplicates would push the real history out of the ring. The cap bounds the loop
            //   but cannot prevent this contamination.
            //   ★★What is blocked is **only one fixed point** — write precisely which cycles leak★★: only cycles of length 1
            //   (A→A) are caught, and **all of length 2 or more leak** (A→B→A, A→B→C→A, …). In that case what stops it is
            //   the page cap · ring ceiling · budget, and until then **several copies of the same conversation** go into the ring — duplicates
            //   are visible on screen and activation is delayed by the budget. Do not read it as 「there is a cap, so it is harmless」.
            //   ★The reason it is not fixed is not the value but **the axis of cost**★ — catching longer cycles requires remembering the cursors seen,
            //   and a cursor is a string whose length the peer decides, so that memory becomes the same accumulated cost as `turnId`
            //   above. There are no grounds for how many to remember, so no constant is invented.
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

    // ★It is flipped once here back to time order★ — in the ring, insertion order is screen order.
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
    // ★The last turn must be closed too★ — if not, the waiting indicator of the restored slot spins forever
    //   ([`history_turn_boundary`] is the source of truth for that causality).
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
/// ★★Without it, a dead link stands as `Ready` — that is why this function exists★★: if the peer closes stdout while
///   fetching history after the handshake succeeded, the `Drop` of [`ReaderExit`] sets `closed`/[`Link::Down`]
///   and wakes the waiting entries with an error. The history side is made to treat that error as **partial success** (that is
///   right — one page not coming and the connection being cut are different events), so left as is, the next line would overwrite the state
///   that just went down with `Ready` and **deliver something already dead as a successful activation.**
/// ★So the axis that splits is not 「what failed」 but **「is the connection standing now」**★ — the state is looked at directly, so
///   there is no need to interpret error strings. If only a page did not come, this check passes.
/// ★The wording of `Err` is the reason the state held, as is★ — that reason is a JSON-RPC error on stdout or the stream ending, so
///   it is in neither the console tail nor the stderr tail (the doc of `LinkResolution::Failed` is the source of truth for that causality).
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
    /// A user turn taken out of the queue — the waiting entry is registered before writing. `seq` = the marker of the turn this Job opened. `item` = the id of the list item
    /// it carried (the handed-over mark is emitted after writing — [`publish_hand_over`]) · `None` for Direct · mail · items without an identifier.
    Turn {
        id: i64,
        seq: u64,
        line: String,
        item: Option<String>,
    },
    /// This wake-up is only the deadline sweep.
    Sweep,
    /// Hands one held item released by the tool-end trigger to the running turn — the waiting entry is registered before writing ([`issue_steer`]).
    // ADR-0231
    Steer(Steer),
}

/// ★The only place where "can it send" and "transition to in-progress" are joined under one lock★ (ADR-0193).
///
/// What it opens with is decided by the Idle order ([`State::idle_opening`] — user `Held` → mail · nothing during a halt after error).
/// If the head is pre-announce, it waits (does not skip — typing order). An item carrying an identifier (floor met) carries `clientUserMessageId` and remains `InFlight`
/// waiting for the echo. ★An item without an identifier leaves at the moment of opening, as today — there is no key to compare, so if left, the turn-end disposal could never
/// close it and it would block mail★. One `turn/start` is one item (PRD §3-8). ★It does not open while Active★ — an in-progress `turn/start` is
/// handled by the vendor as a steer (TRD §3-2). The `Idle` check on the first line prevents it.
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
            // Our types cannot fail to serialize, but by contract it is open. Dropping the body here would be a silent
            //   loss, so it stays in the queue and is retried at the next wake-up.
            tracing::warn!("turn/start serialization failed — this body stays in the queue: {e}");
            None
        }
    }
}

/// If the tool-end trigger ([`State::steer_due`]) stands, picks one held item to hand to the running turn, moves it to `InFlight`, and builds its
/// `turn/steer` — that is all of mid-turn handover (ADR-0235). ★It does not open a turn★ — the opening place is still
/// [`take_turn_locked`] alone (ADR-0193).
///
/// Candidate = the oldest `Held` **user** item. ★Mail is skipped★ — it is outside the list, so it is not under the no-overtaking rule, and it is held until the turn
///   end and goes out at Idle (held mail does not block the steer of user items). One request is one item (PRD §3-8)
///   — while the trigger stands, the writer calls it again in turn and hands over all held text in typing order (each item with its own `clientUserMessageId`).
/// ★If there is nothing to hand over, the trigger is lowered★ — no held user text · no running turn whose turn id is known (`expectedTurnId` is the vendor's
///   precondition) · that turn's 「steer refused」 mark ([`State::steer_refused`]) · the candidate is not a list item (Direct
///   goes out only by `turn/start`) · below the floor (no identifier — today's path). Text typed after that waits for the next tool end or the turn end (Idle
///   order).
/// ★If the candidate is pre-announce, it waits with the trigger left standing★ — it does not skip (typing order), and the announcement wakes the writer ([`announce`]).
/// ★It splits from ✕ under the same lock★ — a ✕ that came after it moved to `InFlight` here is on the handed-over side (`TooLate`).
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
            // The type cannot fail to serialize but by contract it is open — the item stays held and goes out at the turn end. The trigger is lowered
            //   so the same failure is not repeated on every wake-up.
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
        // While the tool-end trigger stands, held user text one at a time — when there is nothing left to hand over, that function lowers the trigger.
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

/// Ends the turn only when `seq` is **exactly the turn in progress now**, disposes that turn's items with `close`, releases the queue, and
/// **emits the boundary** of that turn. The list events of the disposal are emitted **before** the boundary — the consumer the boundary wakes
/// sees a tidied list.
///
/// ★Ending without looking at the marker would let a late signal close the next turn★ — then two turns would be open at once, which is exactly the state
/// the input queue is meant to prevent. The returned value = whether it actually ended it.
///
/// ★The point is that emitting the boundary is not a choice but part of this function★ (ADR-0127): for turns ending on this path,
///   `turn/completed` **does not come** (write failure · response decode failure · error response · deadline expiry · turn id over the cap).
///   Item disposal is part of this function for the same reason — without it that `InFlight` remains forever. But the only input that
///   returns the fact layer to 「not mid-turn」 is the turn boundary signal, so returning only the state without a boundary would leave that
///   incarnation **observed as mid-turn although idle**, and mail is blocked until the 30-minute fail-open valve sweeps it.
///   ★So it is not replaced by one [`OutputEvent::Error`] line★ — that vocabulary means 「not an end」, so the classifier does not
///   count it as a turn signal (that was exactly the mechanism of the defect above).
/// ★The reason is carried **inside** the boundary, with no separate error line in front★ — splitting would divide 「the turn ended」 and 「how
///   it ended」 into two events and give consumers one more ordering contract (the translator's `turn/completed` spot already made
///   the same decision).
/// ★The outcome of this path is always `Failed`★ — all five callers mean 「we give up on this turn」, and none of them
///   is an outcome the peer told us. An outcome the peer told us passes the translator and is attributed by [`Reader::note_turn`].
///   `close` is `Rejected` (write failure · error response) or `Unanswered` (the rest).
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
                // ★The held end is dropped together with this turn★: that slot only waits for the id of **the turn that was open**,
                //   so if the turn ends by another path, it will never be compared. If left, the moment the next
                //   turn reuses that id, [`Reader::resolve`] would close the freshly opened turn on the spot
                //   and raise a stale boundary. A turn always opens only from `Idle` ([`take_turn_locked`]),
                //   so emptying it at every idle spot keeps any slot from carrying over to the next turn.
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
        // ★Emit after letting go of the lock★ (ADR-0006) — subscribers run arbitrary code inside this call.
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

/// The `turn/start` of the turn the writer opened could not be written — withdraws the waiting entry and closes that turn as refused (`Rejected`) (it did not reach
/// the vendor — that text returns to the held place and waits behind the halt · [`State::close_turn_items`]).
///
/// ★This failure has no way back to the caller★ — that call already received `Ok` and left. All that remains is closing this turn as a
///   failure (no new correction channel is made).
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
    /// Fully written — the handed-over mark of that item was emitted.
    Written,
    /// Could not write — that item was returned to the held place ([`steer_write_failed`]).
    Failed,
    /// The waiting table closed — it means the link closed, so the writer ends.
    Closed,
}

/// Emits one steer the writer picked. ★The waiting entry is registered **before** the write★ — registered after, a fast response would arrive first and be dropped as 「unknown
/// id」, and that item would remain waiting for a response. `write` = the action of writing one line to the peer's stdin (production =
/// [`write_line`] — the test bench swaps the write).
/// ★Known remainder★: on `Closed`, the item just moved remains `InFlight` — the link has already closed, and the core's end synthesis closes it.
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
            // The handed-over mark — emitted **after** writing (the value is read from the current stage — [`publish_hand_over`]).
            publish_hand_over(core, state, &steer.item);
            Issued::Written
        }
        Err(e) => {
            steer_write_failed(state, pending, core, &steer, &e);
            Issued::Failed
        }
    }
}

/// A steer the writer picked could not be written — withdraws the waiting entry and returns that item to the held place (it did not reach the vendor · user decision
/// 2026-09-26 B — not erased). The disposal is the same as a steer refusal ([`State::hold_refused_steer`]): the 「steer refused」 mark is attached to that turn
/// so no loop is made where the next tool end of the same turn immediately writes the same text again, and the next Idle opens it · if ✕ reached it, it is withdrawn.
/// ★The turn is left as is★ — the steer did not open a turn, and that turn keeps running at the vendor (it is not a trigger of the halt after error either — TRD §5-5
/// five exits). Generation masking is the same as refusal — an item already disposed because the turn ended before the write is not touched and no mark is attached.
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
    // ADR-0235: the same event is also left as a trace line so it is visible to QA reading only the trace.
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
            // ★This channel is already woken by `recv_timeout` too★ — if both stand, the one that comes first wins, and if the channel
            //   is closed, send fails quietly.
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
            // ★Same as a success response — the item is not moved★ (reason = the steer response spot of [`Reader::resolve`]): that item waits for the echo,
            //   and if the turn ends without an echo, the turn end disposes it ([`State::close_turn_items`]). Even if the expired steer is processed late at the vendor, there is no double
            //   delivery — it is tied to that turn by `expectedTurnId` and is refused after the turn ends.
            // ADR-0231
            Waiter::Steer { .. } => tracing::warn!("{reason}"),
            Waiter::Fire => tracing::warn!("{reason}"),
        }
    }
}

/// Hands the received thread id to the assembly point's record verb. `Err` = this incarnation's connection must not be set up.
///
/// ★It is not persisted here★ — this port of the production assembly point is the receipt of the session id latch, and this link's persistence happens when the peer
/// first echoes a user message ([`Reader::witness_first_turn`] · ADR-0226 amendment).
///
/// ★The honest condition of the gate is not 「it is on disk」 but 「the record call returned」★ — that port swallows failures into the log
/// inside itself and does not block the caller, so claiming persistence on top of it would quote a guarantee that does not exist.
/// If the port is `None`, there is nowhere it is recorded either.
/// ★Called before [`Link::Ready`] — do not flip it★: the line where `turn/start` is allowed is `Ready`, so if the order
/// flipped, a turn would go out with that session before it is recorded, and the port would lose the contract 「called before sending」. Because that contract
/// stands, there is no second verb asking back whether it was recorded.
///
/// ★The closed fence **only narrows the window and cannot close it — do not read it as 「it blocks late writes」**★. It blocks one thing:
/// **an end already observed before this thread got here** (the child answered and died right away and the reader saw EOF ·
/// [`AgentTransport::shutdown`] ran first).
///
/// ★**Do not estimate the remaining window as 「a few instructions」 — the dominant term of its size is lock contention**★. The window runs from the moment
/// the state lock above is released to **the moment the profile map actually changes**, and in between this thread waits for the profile registry lock.
/// Another thread can hold that lock **until the whole rewrite of `agents.json` finishes** (ADR-0071
/// puts the save inside the lock). So `shutdown()`, the return of `join_pump`, and the reaper's reaping can
/// all fit inside this window, and a write arriving after them **writes the sid into the profile of an already reaped session and pushes the old sid
/// into history.** The incarnation marker guard does not cover this window either — none of the end paths touches `epoch`, so
/// the markers of a reaped session and a live session are **the same**.
///
/// Closing that window would need the 「is it alive」 decision to go into **one critical section** with the record, but that critical section is on the profile
/// registry side, and this link's state lock cannot be carried that far (it would be a disk write while holding a lock — the lock order of the module
/// header).
///
/// ★Do not inflate the value `catch_unwind` actually buys — it is not 「safe even if the record blows up」★.
/// The call graph under this port has **no panic source our code creates** (decoding returns `Result`,
/// base `now_epoch_ms` is `unwrap_or(0)`, `normalize_hierarchy` does no indexing, and a save failure returns as `Err`
/// and the registry ends it with a log). What can be reached is the **poison guard** (`expect`) of the registry map lock —
/// the store state field recovers from poison and does not panic — and for that to stand, **another panic must first have
/// poisoned that lock**. So this containment buys exactly one thing:
/// ★in a state where the registry is already broken by someone else's panic, the codex writer **does not die on top of it**★.
/// ★**Moreover, this branch is not reached in the production process**★ — the writer is outside the containment marking, so a panic in here
/// has the hook end the process on the spot (ADR-0292). That is, the outcome running as `Err` is observed only in a test build without the hook.
/// There are two reasons it is still kept — in that build it actually stands, and if this place becomes contained, the defence lives on its own.
///
/// What this blocks in a build without the hook: if unwind went up from here, neither [`Link::Ready`] nor [`Link::Down`] would stand,
/// the link would stop at `Connecting`, and in that state [`AgentTransport::send_input`] would see neither the closed marker nor `Down`
/// and **return `Ok` while the queue fills up to the cap** — the agent looks Running but goes mute, and
/// there is no recovery but kill (the spot where ADR-0190's 「do not let it pass silently」 collapses into 「it stops silently」).
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

    // ★Both markers are looked at together★ — [`AgentTransport::shutdown`] sets this atomic **first** and then takes the state
    //   lock to set `closed`. The gap in between is not empty (at that moment the lock may be held by `send_input` · [`next_job`] ·
    //   the reader's refusal path). Looking at only one would leak that whole span, while OR-ing both closes it for free.
    let already_ended = shutdown.load(Ordering::Acquire) || {
        let (lock, _) = &**state;
        let s = sync::lock(lock);
        s.closed
    };
    if already_ended {
        // ★It is a normal end, so no failure outcome is emitted★ — returning `Err` would stand one more error boundary on a session that already ended.
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

/// One reason the connection could not stand. ★The two fields answer **different questions**★ — `reason` is what went wrong,
/// `while_recording` is **at which stage it went wrong**. The latter only decides what to tell a person (headline ·
/// `detail`).
///
/// ★**The stdin disposal no longer splits by this field**★ — if it failed, it is closed regardless of the branch
/// (that spot in [`writer_loop`] is the source of truth for the reason). This field once meant 「is the peer alive」 and doubled as that decision,
/// and that dual role let the combination **round-trip failure but the peer is alive** (codex's `thread/resume` refusal) leak
/// entirely. Do not bring that meaning back here.
struct HandshakeFailure {
    reason: String,
    /// Did the two round trips succeed and it fell over at the record stage? Only splits the sentence given to a person (the doc above).
    while_recording: bool,
}

impl HandshakeFailure {
    /// The headline for the screen · log. ★The two branches are not reported with the same sentence★ — writing a record failure as 「handshake failed」
    /// makes one suspect the protocol round trip, sending the classification cost to the wrong place (the round trip succeeded).
    fn headline(&self) -> &'static str {
        if self.while_recording {
            "codex app-server session id record failed (the protocol round trip succeeded)"
        } else {
            "codex app-server handshake failed"
        }
    }
}

/// Delivers the connection outcome — ★**at most once**★ (zero-or-once). If the user turned it off, it is not delivered.
///
/// ★A handshake released by kill is not 「the peer answered」★: [`AgentTransport::shutdown`] closes the waiting table and
///   wakes the handshake with `Err`, and delivering that as is would **record the user's cancellation as a resume failure** and
///   run cleanup once more on top of that (defect ③). On that branch the only fact the supervisor sees is the end state,
///   and there the discipline 「what the user turned off is not a failure」 already exists.
/// ★Looking at the `shutdown` atomic is enough★ — that marker stands **first** on the kill path (the first line of that function)
///   and never goes back. That is, what is read here is a latch, not a flipping cell.
/// ★★What keeps 「at most once」 is **the shape of the call sites**, not this function★★ — written precisely:
///   this function only borrows `&Option<LinkSink>` and does not consume it, and delivers every time it is called. The grounds for being bound
///   to once are two — ① the production call sites are only **the two branches of one `match`** in [`writer_loop`], so
///   only one runs on an execution path ② only the writer thread holds the port, and the `take` of [`AgentTransport::start`]
///   prevents two of those threads from coming up.
///   ★This spot once said 「the port is consumed with `take()`」, but **there is no such line**★ — the only
///   `take` is in `start`, and what it prevents is not a second **call** but a second **thread**.
///   Adding call sites breaks this invariant at that site, so when adding them, look here too.
/// ★★What is suppressed is **only the message — it cannot stop the channel from disappearing**★★: this port is held by the writer thread
///   alone, so if delivery is skipped and [`writer_loop`] returns, the sending end drops and the receiving side
///   wakes with `Disconnected`. Reading that signal as 「the link could not reach an outcome」 would **make the suppression meaningless**
///   (the user's cancellation would turn back into a resume failure) — so the receiving side looks at the end-intent latch **first**
///   ([`crate::manager::AgentManager::link_activation_verdict`]). This suppression alone is only half.
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
    // ★The two failures are carried **separately**★ — the 「handshake」 this link speaks of is not only the two round trips but **up to the record call
    //   returning** (the gate condition of [`record_session_id`] above), so both block `Ready` and ride the cleanup below
    //   together. Still, there is **one fact that must not be lumped together**: on the branch that fell over at the record, the round trip already succeeded,
    //   so **it is certain the peer is alive and reading our stdin**. On the branch where the round trip itself failed there is no such
    //   certainty — ★no certainty is not 「dead」★: a refusal (`thread/resume` returned with an error) is an answer
    //   the peer gave while alive and well.
    //   ★So what this difference splits is **only what to tell a person**★ — the stdin disposal below is the same
    //   regardless of the branch. Do not tie the two together again (source of truth for the reason = the closing spot below).
    // ★★The budget clock is started **here, once** — the handshake and the history restore share it★★.
    //   Started separately, the time until the connection stands would be up to two budgets, exceeding the manager's backstop, and a resume that
    //   succeeded while fetching history would be decided as a failure (the doc of [`hydrate_history`] is the source of truth for that causality).
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
            // ★The floor decision is written **after** the record returns and **before** the history★ (reason = [`settle_floor`]) — the window in which the list of pre-decision
            //   items comes up late is bound to the handshake round trip (TRD §5-5). It is before the gate, so those items
            //   cannot go out yet.
            // ADR-0231
            settle_floor(&core, &state, &announcer, opened.floor);
            // ★★The past screen is raised **before opening the gate** — do not flip the order★★ (ADR-0203):
            //   if the gate opened first, user turns queued from that moment could go out, and that turn's response would go into the ring **before**
            //   the history not yet carried. Then the restored conversation would attach after the new answer.
            //   ★This order carries in place what the seed-before-publish of ADR-0079 used to keep★ — there the history was
            //   on disk and could be read before launching the child, but here the history is received **by asking a peer that exists only after the handshake
            //   stands**, so it cannot be pulled ahead of the session going into the registry.
            //   Instead ⓐ it is before the gate, so there is no turn of ours yet, and ⓑ the door below **does fanout too**.
            // ★That it is `emit_without_turn_observation` is the concrete form of ⓑ★ — `seed` only puts into the ring and
            //   does no fanout, so used here, where the session is already in the registry, a subscriber that attached in between would **never see** the past
            //   screen (it already replayed an empty ring and has no way to receive it again). This door both puts into the ring and
            //   sends to attached subscribers, so both kinds see the same thing.
            // ★While doing so it does not touch observation · state · finalize★ — the very discipline `seed` kept
            //   (ADR-0005 · ADR-0113 · ADR-0127). Feeding past records as progress signals would mean that turn's end
            //   never comes and mail is blocked until the 30-minute fail-open.
            // ★Raised **as one chunk**★ — called item by item, the reader's live lines would take seqs in each gap and
            //   new lines would be stuck in the middle of the restored conversation ([`OutputCore::emit_batch_without_turn_observation`]).
            core.emit_batch_without_turn_observation(hydrate_history(
                &stdin,
                &state,
                &pending,
                &next_id,
                &opened,
                link_deadline,
            ));
            // ★★The gate opens **only while the connection still stands**★★ — if the stream ended while fetching history,
            //   setting `Ready` here is 「delivering something dead as a successful activation」 ([`open_gate`]).
            match open_gate(&state, opened.thread_id) {
                // ★Delivered **right after** opening the gate — not called while holding the lock★ (ADR-0006: no external calls while holding the state
                //   lock. This port calls assembly point code, and that side touches channels).
                Ok(()) => deliver_link(&shutdown, &link_sink, LinkResolution::Ready),
                Err(reason) => {
                    // ★No error · boundary is added to the screen here★ — the stream having ended means the pump will soon
                    //   emit the end transition (ADR-0005 sole actor), and that path already closes the screen.
                    //   If it is a user kill, `deliver_link` suppresses this delivery (that function's latch).
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
            // ★The reason is **carried** in the delivery★ — this wording is a JSON-RPC error on stdout, so it is in neither the console tail nor the stderr
            //   diagnostic tail. If not carried here, the classification has no way to reach it at all.
            deliver_link(
                &shutdown,
                &link_sink,
                LinkResolution::Failed {
                    reason: reason.clone(),
                },
            );
            // ★This boundary does not correspond to a link-side turn★: the only place that opens a turn ([`take_turn_locked`])
            //   requires [`Link::Ready`], and that state stands only if the handshake succeeds. So the queued
            //   `dropped` inputs, seen from the link side, are **not turns yet but queued bodies**, and the fact layer has never
            //   observed this incarnation as 「mid-turn」 either (the first progress signal comes from the item the peer echoes,
            //   and no synthetic input echo is declared — ADR-0193).
            // ★Still, **whether or not input was dropped**, one failure outcome is emitted★ — because this is the only thing that closes
            //   the screen. The frontend does not read `Error` as a turn end (because retried stream errors come in that vocabulary
            //   — the doc of [`OutputEvent::Error`]), so a slot holding only an error has **its waiting indicator spin
            //   forever**. The link is already [`Link::Down`], so nothing will arrive after that either.
            // ★There is no open turn and so no id to carry, so `turn_id` is not invented★.
            // ★However many were dropped, there is one boundary★ — `dropped` is the number of dropped bodies, not the number of turns.
            // ★The reason sentences of the two branches differ on purpose★ — the dropping side says that bodies a person sent were destroyed,
            //   and the non-dropping side says only that the connection could not stand. Writing input loss on the latter would report something
            //   that did not happen.
            // ★That the reason is carried both in the error line and inside the boundary is intended★ — a consumer reading only the boundary must also know what
            //   happened (the same reason as [`end_turn_if`]), and so no new ordering contract
            //   arises between the two.
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
                // ★Here 「the connection could not stand」 is false★ — the round trip stood, and we decided not to use it.
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

            // ★Our stdin is closed **on both branches**★ — getting here means 「this incarnation is not
            //   used」 is settled, and that conclusion is the same regardless of the peer's liveness. Without closing, the loop below
            //   would spin forever chewing only `Job::Sweep`, holding the child · reader · writer, and the reader would never see EOF, so
            //   [`OutputCore::finish`] would never run and **neither the end transition nor reaping would happen**
            //   (the screen says Running but all input is refused — until a person kills it).
            // ★This spot once said 「close only on the branch that fell over at the record — only that branch has certainty of the peer's survival」.
            //   The premise was true and **the conclusion was wrong**★: certainty of survival is not a *premise* of closing.
            //   If the peer is already dead, `take()` only drops one handle, so it is a no-op (writes after it
            //   fail anyway), and if it is alive it receives EOF — **both sides are safe.** Conversely, the side that does not close
            //   has no safe outcome: the combination of a failed round trip with the peer alive and well really exists, and that is
            //   ★codex **refusing** `thread/resume` with a JSON-RPC error★ (the refusal comes on stdout and
            //   the child does not die). That combination is exactly the wedge above — ★unlike the record failure branch, it **is reached
            //   in production**★ (that one is a panic outside the marking, so in the production process the hook ends it first — ADR-0292) — and on a boot restore
            //   as many stand at once as there are resumable codex profiles.
            // ★Still, **the reason sentences are kept split**★ — what changed is the stdin disposal, not the diagnosis
            //   ([`HandshakeFailure::headline`] and the `detail` above). On the round-trip branch the connection could not stand, and on the record branch
            //   it stood but we decided not to use it.
            // ★This link does not end its own lifetime★ — it still does not call kill (the 2 verbs of ADR-0001
            //   belong to the kill handler). What is done here is only **letting go of our write end**, and after that
            //   the existing causality drives it: peer exit → reader EOF → the `Drop` of [`ReaderExit`] sets
            //   `closed`+`Link::Down` → the pump emits the end transition (ADR-0005 alone) → the reaper reaps.
            //   ★The measured 41–51ms is the figure of **the record failure branch** (closed while there was no turn) — how fast the peer
            //   ends on the refusal branch has never been measured★. Only the causality is the same; do not copy the figure over.
            // ★Why taking the stdin lock here is safe (unlike the order risk of `shutdown`)★: that risk is the shape where
            //   **another thread** waits for the lock of a writer hanging in `write_all`. What takes it here is
            //   the writer itself, and this point is before entering the loop, so it is not inside `write_all`.
            //   ★That decision does not tell branches apart — removing the `if` this time does not touch these grounds★:
            //   the axis of the grounds is 「which thread takes it」, not 「which failure it was」, and the acquisition **place** is still
            //   one (the single line below). So the count also stays two.
            // ★**Write the range of that safety precisely — not 「the whole file」 but 「the production call graph」**★:
            //   the places that take this lock blocking are only **two** in the production section ([`write_line`] and this line right below),
            //   and only the writer calls [`write_line`] (`shutdown` is `try_lock`, so it does not count).
            //   ★But **the test section takes the same lock blocking outside that graph**★ —
            //   [`tests::shutdown_completes_even_if_a_write_blocks_on_a_full_pipe`] pushes 8MiB with [`write_line`]
            //   on a separate thread and holds the lock until kill comes. Running such a filling thread and this
            //   branch **on the same test bench** would block here forever, and then stdin would not close and the causality this round
            //   set up would disappear entirely. No item combines them that way today — that item does not call [`AgentTransport::start`],
            //   so the writer thread itself does not come up (that fact is what makes this exception harmless,
            //   and the moment `start` is added, the harmlessness breaks).
            //   The count in the production section is guarded by [`tests::the_production_blocking_stdin_locks_are_counted`] —
            //   if a third appears the decision above must be made again, so it is kept from quietly growing.
            let had_stdin = sync::lock(&stdin).take().is_some();
            tracing::warn!(
                had_stdin,
                while_recording = *while_recording,
                "the connection could not stand, closing the codex-side stdin — the session is reaped once the peer sees EOF and ends"
            );

            // ★It ends here — this link **kills nothing**★.
            //   Letting go of stdin makes the peer see EOF and end on its own (measured 41–51ms, with no turn), and that EOF
            //   ends the reader, leading to [`OutputCore::finish`] → reaper. ★That first link is **not ours**,
            //   and if the peer remains with stdout open, the whole chain does not run★ — whether the peer ends with EOF even after a refusal
            //   is **unverified** (only the record failure branch was measured).
            // ★That disposal is **the manager's job** — do not kill here★: the [`Link::Down`] above is
            //   immediately visible outside via [`AgentTransport::link_state`], and the activation decision confirms it as a failure
            //   and then runs the existing teardown (the 2 verbs of ADR-0001) — that kill also releases a hanging reader.
            //   ★The writer once called `child.kill()` · `TerminateJobObject` directly at this spot, but that
            //   crossed the line ADR-0199 drew (the writer calling those verbs is a violation the moment it happens), so it was removed.
            //   Do not revive it — to revive it, that ADR must be changed first.★
            // ★**A hole that once remained — closed now**★: previously no decision was hooked at all for codex launched as Fresh
            //   (the supervisor dropped the delivery box), so **nobody looked at** the `Down` of a `thread/start` refusal.
            //   Now the two activation entrances (the Fresh branch of `AgentManager::activate_profile` ·
            //   the non-resume branch of `AgentManager::restore_one`) run the same decision, so both modes
            //   have an owner (`AgentManager::spawn_fresh_settled`).
            // ★One branch without an owner remains, but **it does not reach here today**★: ad-hoc creation that calls `AgentManager::spawn_agent`
            //   directly (the daemon's by-cwd branch) drops the delivery box as is. That the output format that entrance gives codex
            //   is **terminal**, so this link itself does not come up, is all that makes that branch harmless
            //   — the day that default changes to app-server, this hole opens as is.
        }
    }

    // ★The loop runs even if the handshake failed★ — because there must be one way out: when the reader sees EOF and
    //   sets `closed`, [`next_job`] returns `None` and it exits on its own. Returning here would make
    //   two cleanup points.
    // ★But **「it runs to answer the peer's requests」 is no longer true**★ — stdin was already let go right above,
    //   so every [`write_line`] after it fails. The peer sees EOF instead of waiting for an answer. The old sentence
    //   (「the peer may be alive, and then a place to answer server requests is needed」) dates from when closing existed on only one
    //   branch.
    // ★**Do not write 「the peer never stalls」 unconditionally — that much has never been measured**★: what was measured is one thing,
    //   「closing stdin while there is no turn makes it exit within 41–51ms」. Under the condition this header writes separately
    //   — the peer's requests pile up during the handshake and refusals drop at [`OUTBOX_LIMIT`] — after closing,
    //   every [`write_line`] fails, so the peer meets EOF **with unanswered requests remaining**. Whether the peer simply ends
    //   even in that state is **unverified**. If it does not end, it is exactly the 「the first link is the peer's action」 item above.
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
                // ★The waiting entry is registered **before** writing★ — registered after, a fast response would arrive first and be dropped as an "unknown id".
                // ★Known remainder★: leaving here leaves the turn just opened as Active. The waiting table having closed means
                //   the link has already closed, so there is no consumer to read that state, but 「the turn is tidied
                //   on every way out」 does not hold.
                if !pending.register(id, Waiter::TurnStart { seq }, method::TURN_START) {
                    break;
                }
                match write_line(&stdin, &line) {
                    Err(e) => turn_write_failed(&state, &pending, &core, id, seq, &e),
                    // ADR-0231: the handed-over mark of the list item it carried — emitted **after** writing (the value is read from the current stage).
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
/// ★One `try_wait` right after EOF cannot get the code★ — stdout closing and the process being reaped are
/// not the same moment, so that one call usually falls to `Ok(None)` and the code is quietly **lost**. That loss hurts
/// twice: the exit code does not show on screen, and `Exited { code: None }` cannot be told apart from 「the child is still alive」,
/// so the test bench loses its means of telling the two apart.
/// ★Grounds for the value★: between stdout close and process reaping is on the order of milliseconds. 2 seconds is a generous margin,
/// and beyond it, it is 「a peer that has not ended」 — that disposal is carried not by this link but by the manager's teardown.
const CHILD_REAP_GRACE: Duration = Duration::from_secs(2);

/// Cuts arbitrary chunks into lines. ★It does not read as UTF-8 before a complete line is settled★ — the pump throws chunks that do not know
/// character boundaries, so a multibyte character can be cut at a boundary, and a newline (0x0A) cannot appear as a UTF-8 continuation byte,
/// so a byte-level search is safe.
struct LineSplitter {
    buf: Vec<u8>,
    /// Is it in the middle of dropping the whole tail of an over-cap line up to the next newline? ★If only the buffer were emptied, the remaining bytes of that line
    /// up to the next newline would be parsed as a "new line" and make a fake envelope★.
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
            // ★Measure before appending★ — measuring after appending is already past it at that moment.
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

/// Closes the link **however the reader ends** — whether a normal EOF or a panic that came up inside `handle_line`.
///
/// ★Why it must be `Drop`★: `contain` wraps the reader loop **from outside**, so cleanup written at the tail of the loop
///   is simply passed over by unwind. That path is not hypothetical — if a subscriber panics inside `core.emit` or
///   the mutex of `OutputCore` is poisoned (that side is `.expect`), it goes right there. Then the closed marker does not
///   stand and the writer thread spins forever holding core · stdin · the waiting table.
/// ★What is done here is only 「close」★ — decoder flush and producing the end reason are the normal path's job, so they stay
///   inside the loop (nothing is lost if unwind skips them).
struct ReaderExit {
    state: SharedState,
    pending: Arc<Pending>,
    /// The agent that the transition log of a discarded halt carries ([`Transition::Discarded`]).
    agent: AgentId,
}

impl Drop for ReaderExit {
    /// ★★Order is the contract — **write the state first, then wake**★★.
    ///
    /// ★It was once the other way, and that was a defect★: waking the waiting entries first brings the writer back **through a window the death itself
    ///   opened** — the woken writer reads the still `Connecting` state, passes the gate ([`open_gate`]),
    ///   and **delivers `Ready` to an already ended link.** That window was not by chance; it opened on every stream end.
    /// ★The current order closes it **structurally**★: state write (mutex release) → `send` (channel) →
    ///   the writer's `recv` → the writer's state read (mutex acquire) chain by happens-before, so a writer woken by this end
    ///   **always** sees [`Link::Down`]. The grounds are the memory model, not timing slack.
    /// ★Still, do not remove the waking★ — leaving them is a permanent hang, and that hang has no signal. What changed is
    ///   **only the order**.
    /// ★Locks are not held overlapping★ (ADR-0006 state→waiting table): the waiting table is closed **after** letting go of the state lock.
    // ADR-0203
    // ADR-0201
    fn drop(&mut self) {
        let discarded = {
            let (lock, cv) = &*self.state;
            let mut s = sync::lock(lock);
            // The turn that was in progress does not know its outcome — ★one log line is all★. Leaving this fact as state would need a third
            //   caller to erase it (ADR-0127 decision 5), and the next incarnation does not take over this incarnation's link
            //   (ADR-0192), so there is no consumer to read it either.
            if matches!(s.turn, TurnState::Active { .. }) {
                tracing::warn!(
                    "codex app-server: the stream ended without knowing the turn outcome"
                );
            }
            s.link = Link::Down("the stream ended".to_string());
            s.turn = TurnState::Idle;
            // The held ends are dropped too (source of truth for the reason = [`end_turn_if`]).
            s.early_completions.clear();
            // The halt after error is discarded too — after closing there is no turn to open, and the waiting items are closed by the core's end synthesis.
            // ADR-0231
            let halted = std::mem::take(&mut s.halted);
            s.closed = true;
            cv.notify_all();
            halted.then_some(Transition::Discarded)
        };
        // Written outside the lock (reason = [`TurnNoteLog`]).
        if let Some(t) = discarded {
            t.write(self.agent);
        }
        // ★Wake all pending RPCs with an error★ — leaving them is a permanent hang, and that hang has no signal.
        for entry in self.pending.close() {
            match entry.waiter {
                Waiter::Handshake(tx) => {
                    let _ = tx.send(Err(format!("{}: the stream ended", entry.method)));
                }
                // ★The steer item is not touched★ — the stream end is the end of this incarnation, and the remaining items are closed by the core's end
                //   synthesis as `Dropped{AgentEnded}` (the owner of end disposal is one — TRD §5-5 「deadline and close」).
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

/// The one line [`Reader::note_turn`] leaves **after letting go of the lock**.
///
/// ★It is not written while holding the state lock★: tracing's file sink writes synchronously per event, so writing it inside that lock
///   would line up [`AgentTransport::send_input`] and the writer behind that disk write. And
///   [`Self::Unattributable`] is an **everyday** path — when the real `turn/completed` of an abandoned turn comes late,
///   it comes here. One chatty peer creates that many disk writes while holding the lock.
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
                // Left as an empty string, 「there was no id」 and 「the id was an empty string」 would look the same in the log.
                turn = %turn
                    .map(|t| sanitize(t, LOG_STRING_LIMIT))
                    .unwrap_or_else(|| "(no field)".to_string()),
                "codex app-server: an unattributable turn/completed — blocking the boundary"
            ),
        }
    }
}

/// The item ids of approval requests we refused — remembered to attribute that item's live end to our refusal
/// ([`ToolOutcome::Refused`]).
///
/// ★The vendor end alone cannot tell our refusal apart★ (measured fixture `refuse_u2a`): a refused command closes as `failed`, and a refused file change
///   as `declined`, with the same shape as a real failure · the vendor's own denial. So this layer, which refused, holds the id and rewrites it at the end.
///   The translator does not know this memory — it stays a pure translator that receives only notification lines.
/// ★Reader-only and lock-free★ — the writing side ([`Reader::refuse`]) and the reading side (the end notification of the same [`Reader::handle_line`]) are both
///   the reader thread, and the peer emits the end after receiving our answer, so the memory always stands before the end. Putting it in the state lock ([`State`]) would only lengthen the lock
///   section.
/// ★The place that empties it is that item's end line alone — it is not emptied at the turn end★: the end comes even after the turn end (measured fixture `steer_m6`
///   last line). Emptying at the turn end would make that late end show as 「error」.
// ADR-0241
#[derive(Debug, Default)]
struct RefusedItems {
    ids: VecDeque<String>,
}

impl RefusedItems {
    /// Writes one refused item. If full, the oldest is dropped (reason = [`REFUSED_ITEM_SLOTS`]).
    /// ★An already remembered id is not put in again★ — the peer alternates request → end per approval and does not put two refusals on one item, but
    ///   even if that order changes, the slots do not leak.
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

    /// Attributes the translation result of one notification line to our refusal. If that line is `item/completed` of a remembered item, the memory is **taken out**
    /// (regardless of the end status — the end line is the emptying place), and the `Failed` · `Declined` results the translator emitted with that id are changed to `Refused`.
    /// If the translator emitted no result (`completed` etc.), only the memory is erased.
    /// ★It does not split by the vendor end status★ — whether the vendor closes our refusal as `failed` or as `declined`, the reason is our refusal.
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

/// The id if a refused server request points to a tool item to attribute to our refusal — otherwise `None` (only refusing, as today).
///
/// ★Only two requests point to one — command approval and file change approval★. In both, `itemId` is the id of a tool item started earlier, and the peer
///   closes that item right away on our refusal (measured fixture `refuse_u2a`). Other requests (`item/tool/call` ·
///   `item/permissions/requestApproval` etc. — we do not know whether that item is a tool row we draw · approval under the old name has the field
///   `callId`) do not point.
/// ★Even a command approval does not point if `approvalId` or `networkApprovalContext` is carried (only absent or `null`)★ — the former is
///   a sub-command approval, so its refusal is not an execution refusal of the parent item (the parent's end is the result of the whole script), and the latter is a network
///   approval, so the peer does not emit that item's end (reading of upstream source `rust-v0.156.1` — likely · the measured command approvals
///   had only the shape without both).
/// ★The id is filtered by the same cap as the translator ([`MAX_ID_BYTES`])★ — for an id over it the translator emits no result, so the memory finds no pair and
///   only takes a slot. This filtering is also the cap on the bytes held: without it, one slot holds an id up to the line cap ([`MAX_LINE_BYTES`]),
///   and such slots pile up to [`REFUSED_ITEM_SLOTS`].
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
    /// Refuses a server request. ★If not answered, that agent stops forever★ — so even an unknown request is
    /// always answered. ★It does not disguise success★: returning a success response to an approval request is an automatic approval.
    /// ★If a refused approval request points to a tool item, its id is remembered ([`RefusedItems`] · decision = [`refused_item_id`])★ — to attribute that
    ///   item's end to our refusal. It is written only when the refusal line actually went into the control queue: if it could not be put in because closed or overflowing,
    ///   the peer gets no answer and the end does not come.
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
            // ★There is no caller to return an error to here — it is a line the reader itself made★. So if left only in the log, this
            //   event **is visible nowhere**: the peer waits for the answer forever and there is no signal on screen. The sibling
            //   event (turn/start deadline) shows on screen, so this place is raised to the same grade.
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

    /// Feeds one turn-ended notification into this state machine, and ★decides **whether to raise the turn boundary the translator emitted from that line
    /// to the screen**★. The returned value = the boundary to raise now (`None` if none). If attributed to the end of our turn, that turn's
    /// item disposal events are emitted here (before the boundary — [`State::close_turn_items`]).
    ///
    /// ★Only this layer knows attribution★ — the translator sees the same line but does not know our thread · turn id. So
    ///   translation is always done, but whether its output **is our turn's boundary** is decided here. Without this gate, one line of someone else's
    ///   turn end would close our live turn on screen (later output split by a fake boundary), and the fact layer too
    ///   would flip to idle and mail would be plugged in mid-turn.
    /// ★A blocked line does not quietly disappear★ — it remains as a warn. Without a log, there is no knowing whether to look for the cause of 「the boundary did not show」
    ///   in this layer or the translator.
    ///
    /// ★The ending condition is only when both match★: equal to our thread id (if known), and **equal to the turn id we know.**
    ///   Ending by looking only at the thread would let another turn of the same thread (a late signal of an already ended previous turn) close the turn running now,
    ///   and the queue would open on the spot and turns would overlap.
    /// ★If the turn id is not known yet, nothing is ended★. That ignoring does not create a stall **at that point**:
    ///   an empty id means that turn's `turn/start` has not received an answer yet (whether the answer is success, error
    ///   or a decode failure, those three branches all end the turn or fill the id), and that request has [`REQUEST_DEADLINE`]
    ///   on it, so expiry ends that turn via [`end_turn_if`].
    /// ★That argument does not cover the case where the end comes **before** the response — so it is closed separately★: if that notification were
    ///   only dropped here, the response that follows would fill the id and take away the waiting entry too, leaving that turn with nothing
    ///   to end it. So that notification's id **and boundary** are held in [`State::early_completions`],
    ///   and compared at the spot where the response decides our turn's id, and raised then ([`Reader::resolve`]).
    /// ★The key is that what is held is not 「there was an end」 but **that id**★ — the former is a groundless memory that could end anything,
    ///   which is misattribution itself, but the latter only **defers** the id comparison this function already does.
    ///   The ending decision is only the id match on every path. ★Do not widen that property★ — the moment a branch that ends a turn
    ///   without an id comparison is added, the misattribution this file removed comes right back.
    // ADR-0127
    fn note_turn(
        &self,
        method_name: &str,
        params: Option<&Value>,
        boundary: Option<OutputEvent>,
    ) -> Option<OutputEvent> {
        if method_name != TURN_COMPLETED {
            // ★The translator does not emit turn boundaries outside this name★ — still, what was received is returned as is:
            //   on the day that premise breaks, the boundary showing on screen is better than it quietly disappearing.
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
            // ★An unknown threadId is not an error★ — it was measured that a notification arrives before the response that would tell
            //   that thread's id. While our id is not known yet, it is not filtered on that axis.
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
                    // ★An end that came while the current turn's id is not known yet★: at this point it cannot be attributed, so it **does not end
                    //   and does not raise the boundary either**. Instead that id and boundary are held, and compared when the following `turn/start` response
                    //   tells our turn's id authoritatively ([`Reader::resolve`]). Without that comparison this
                    //   turn would remain with nothing to end it — the response takes away the waiting entry and the deadline backstop disappears too.
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
                    // There is no open turn, or it differs from the id we know, or the notification has no turn id at all.
                    _ => (None, Some(TurnNoteLog::Unattributable(incoming_turn))),
                }
            }
        };
        if let Some(entry) = log {
            entry.write();
        }
        // ★Emitted before the boundary, outside the lock★ (reason = [`end_turn_if`] · ADR-0006) — the boundary is emitted by the caller.
        drops.emit(&self.core, &self.state);
        result
    }

    /// Can this notification **be counted as belonging to the turn we have open now** — not whether to raise it to the screen but ★whether to write
    /// a progress signal into the fact layer★ (ADR-0113).
    ///
    /// ★Why the two decisions are split★: blocking a boundary loses 「one boundary」, but blocking a body makes **the words the agent emitted
    ///   disappear from the screen**. So the body is always raised, and only whether that line becomes grounds for 「this incarnation is mid-turn」
    ///   is split here. Without this split, one `item/*` line of a turn we never opened would turn the fact layer
    ///   on, and that turn's end would be blocked by [`Reader::note_turn`] so **nobody could turn it off** — until the 30-minute fail-open
    ///   valve sweeps it, no mail goes to that agent (the same shape as the mechanism ADR-0127 closed).
    /// ★The decision is 「does it not contradict」 — not 「does it match」★: `item/*` arrives **before the `turn/start` response**
    ///   is normal (that window is why [`State::early_completions`] exists), so demanding an id match at that point
    ///   would push our turn's streaming output entirely out of the fact layer. So if there is an open turn
    ///   and it **does not contradict** that turn's id, it counts as ours.
    /// ★If there is no open turn, it is never ours★ — at that moment this link is 「not mid-turn」, and the point of this shape is that both axes
    ///   get the same decision (module header).
    /// ★There is no lock between this decision and the emit (known remainder)★ — if the writer opens or closes a turn in that gap, one line
    ///   counts toward the other side. Dragging the lock up to the emit would violate ADR-0006, and that one-line error is covered by the next line and
    ///   the turn boundary.
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
                // An id we never issued — a value of the server id space, or a place already withdrawn by the deadline.
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
                    // ★The id the response decides is length-capped too★ — this value goes out as is into the params of `turn/interrupt` and
                    //   the field of [`OutputEvent::TurnEnd`], so without a cap the peer could with one response make a boundary exceeding the replay
                    //   ring's single-event cap, and that ring would empty everything else for that one entry.
                    //   ★It is a comparison token, so it is filtered, not truncated★ (same decision = [`MAX_TURN_ID_BYTES`]) — then this
                    //   turn has no material for attribution, so it is closed here with **the same grade** as the error branch below.
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
                            // ★If the marker does not match, the turn this answer opened has already ended★ — writing that id into the current turn's slot
                            //   would make the interrupt after it aim at the wrong turn.
                            match &mut s.turn {
                                TurnState::Active { seq: cur, turn_id } if *cur == seq => {
                                    let newly = turn_id.is_none();
                                    if newly {
                                        *turn_id = Some(r.turn.id);
                                    }
                                    // ★This response is the place that authoritatively decides our turn's id★ — if an end
                                    //   with that id already passed, it is ended here, and **the boundary held then
                                    //   is raised now**. Without this comparison that turn would remain open forever
                                    //   (the waiting entry was just taken away), and the `send_input` after it would report that stall as "the queue
                                    //   is full" — a report with the wrong reason.
                                    // ★The ending decision is still only the id match★ — no turn ends merely by being in
                                    //   the held list.
                                    let ours = turn_id.clone();
                                    // ★Writes the turn id onto the item this request carried — only when (marker, stage) match★ (generation
                                    //   masking: a no-op if already removed by echo). A success response is acceptance, not receipt, so
                                    //   the item keeps waiting for the echo (TRD §5-5).
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
                                            // The rest held in the same window are dropped together with this turn
                                            //   (source of truth for the reason = [`end_turn_if`]).
                                            s.early_completions.clear();
                                            // ★An early end is a turn end too★ — if not disposed here, that turn's
                                            //   `InFlight` remains without an outcome (TRD §5-5 turn-end disposal ②).
                                            // ADR-0231
                                            let drops = s.close_turn_items(seq, held.close);
                                            cv.notify_all();
                                            (drops, held.boundary)
                                        }
                                        // ★The turn id just came★ — if there is user text held while opening this turn (text left at the turn end ·
                                        //   text typed before the response), it is put into this turn right away without waiting for a tool end
                                        //   — the text left at the turn end gathers into one turn (ADR-0235).
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
                        // ★Raised after letting go of the lock★ (ADR-0006) — subscribers run arbitrary code inside emit.
                        //   The disposal events come before the boundary (reason = [`end_turn_if`]).
                        let (drops, boundary) = released;
                        drops.emit(&self.core, &self.state);
                        if let Some(ev) = boundary {
                            self.core.emit(ev);
                        }
                    }
                    // ★It must be **the same grade** as an error response★: the waiting entry was already taken away above, so if
                    //   the turn is not ended here, that turn remains without `turn_id` and with no waiting entry to expire — `note_turn`
                    //   can never attribute and the gate does not open (after that `send_input` reports that stall as
                    //   "the queue is full"). An undecodable success is no less fatal than an explicit error.
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
                // An error response of `turn/start` is a refusal (`Rejected`) — the turn-end disposal returns the user text it carried to the held place and
                //   the halt stands ([`State::close_turn_items`]).
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
            // ★The response reaches only that item — it does not end the turn★: a steer did not open the turn, and even if an error response comes,
            //   that turn keeps running at the vendor (`turn/completed` ends it). The error sometimes comes before `turn/completed`
            //   (M10 — 0.1 ms ahead) — no order is assumed.
            // ★A success response (and the deadline) does not move the item — receipt is only the echo★: the echo of an accepted steer comes on the same stdout
            //   before `turn/completed` (TRD §3-2 · M10), so text without an echo until the turn end was not accepted and may be sent again,
            //   and a late response is made a no-op by the generation comparison ([`State::hold_refused_steer`]).
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
                // Written and emitted outside the lock (reason = [`TurnNoteLog`] · ADR-0006).
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

    /// For every receipt (`Delivered`) the translator emitted, removes that id from pending — regardless of stage
    /// ([`State::take_delivered`]). The returned value = whether a turn boundary must be newly set after the receipt (below).
    ///
    /// ★Removed **before** emitting to the screen★: that text is already in the vendor history and must not go out again, and another thread's
    ///   disposal (deadline · write failure) must not meet that item afterwards and move it twice.
    /// ★A receipt that came outside our turn (late receipt)★ — text returned to the held place at the turn end · a `Held` returned by refusal — is also closed
    ///   as received (the next user turn sees that text in history — TRD §5-5 「when a receipt comes after the turn end」). It is removed while held, so it does not go out
    ///   again. If the old request's echo comes after it was already handed over again, that is the receipt, and the new request's echo passes as an unknown id
    ///   (the registry folds the second receipt of the same id). Once removed, the writer is woken.
    /// ★A receipt that came with no open turn sets a new boundary (`true`)★ — that bubble is the conversation's last bubble, so the frontend turns on the waiting
    ///   indicator, and only a turn boundary turns it off, and there may be no turn following it. Unknown ids (revival after ✕ · Direct · mail) are the same — an unsuppressed vendor echo sets up a bubble.
    ///   If there is an open turn, that turn's end is the boundary.
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

    /// The tool-end trigger (ADR-0235 — t3code shape): when a tool item ([`is_tool_item`]) of our current turn ends
    /// (`item/completed`), sets [`State::steer_due`] and wakes the writer — that tool end hands over all held user text.
    /// The returned value = the trace lines of the tool item (`tool_start` · `tool_end` of [`STEER_TRACE`] — to be written outside the lock).
    ///
    /// ★Only lines of **the current turn id** of our thread (if known) count★ — a late completion of an old turn id (a tool of an interrupted turn — M6) is not a
    ///   trigger. Lines before the turn id came (`Active{turn_id: None}`) do not count either: before that there is no means to hand over (`expectedTurnId`),
    ///   and the spot where the response gives the turn id hands over the held text separately ([`Reader::resolve`]).
    /// ★Parallel tools trigger at every end too★ — running tools are not counted. One finished tool is itself a place to slip in.
    /// ★Called **before** translation★ — the time the trigger reaches the writer is the reaction latency, and doing translation · emit after it
    ///   has no ordering that gets caught.
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
        // The start · end trace lines of tool items — tools whose id cannot be compared (absent · overflowing) are not written (the trigger still stands).
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

    /// When the peer echoes a user message item **for the first time**, calls the first-turn port — the place where a turn appeared in this incarnation's conversation.
    ///
    /// ★Any `userMessage` of live `item/started` · `item/completed` counts★ — whether or not `clientId` is there (below the
    ///   floor · mail turns too), and regardless of our turn decision ([`Reader::claims_our_turn`]) (a late echo after the turn end also has the same fact
    ///   that the text went into the vendor history). Only lines that **declare** another thread are excluded. History restore does not pass this
    ///   door (it is received as a response and the writer carries it — [`hydrate_history`]).
    /// ★Called without holding any lock★ — beyond the port is the latch commit (profile disk write). The state lock taken for the thread comparison
    ///   is let go before that. Called **after** emitting to the screen: so the disk write does not hold up the bubble.
    // ADR-0226 · ADR-0231 · ADR-0233: user decision — the id is persisted when it became real (the peer's first user message echo).
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
                // Trace lines are written right after letting go of the lock — so they come before the steer line (writer) that trigger released.
                if let Some(t) = self.note_tool_item(&method, params.as_ref()) {
                    t.write(self.core.id());
                }
                // ★The translator gets **the original line bytes** as is★ — making a second entrance would move that side's line
                //   reassembly · cap · masking discipline out of the delivery path (source of truth for the reason = the `decoder.rs` header).
                //   The translator splits lines by newline, so the terminator is given too.
                let mut events = match self.decoder.as_mut() {
                    Some(dec) => {
                        let mut events = dec.decode(line);
                        events.extend(dec.decode(b"\n"));
                        // Usage observation is not an event — it is handed straight to the status sink regardless of the attribution gate.
                        self.core.report_usage(dec.take_usage());
                        events
                    }
                    None => Vec::new(),
                };
                // ADR-0241: the translator does not know our refusals — the end of a refused item is rewritten by this layer before emit (without a lock).
                self.refused
                    .attribute(&method, params.as_ref(), &mut events);
                // ★Only the turn boundary is pulled out and passes the **attribution gate** — the rest flows as is★: the translator does not know our
                //   thread · turn id and cannot answer 「is this outcome our turn's」, and only this layer
                //   knows that (source of truth for the reason = [`TURN_COMPLETED`] doc).
                let boundary = events
                    .iter()
                    .position(|e| matches!(e, OutputEvent::TurnEnd { .. }))
                    .map(|i| events.remove(i));
                let boundary_owed = self.note_delivered(&events);
                // ★The body always goes to the screen, but whether to count that line as grounds for 「this incarnation is mid-turn」 is split★
                //   (source of truth for the reason = [`Reader::claims_our_turn`]). If these two doors did not split, an unattributed line would
                //   turn only the fact layer on, and its end would be blocked by the gate below with nobody able to turn it off.
                // On a line that yields only one boundary (most `turn/completed`) this decision is not needed — the lock is saved.
                let ours = !events.is_empty() && self.claims_our_turn(params.as_ref());
                for ev in events {
                    // ★End vocabulary is not sent through that door★ — that door also skips the doorbell (`StatusSink::turn_ended`),
                    //   so consumers waiting for the end would not wake. By the translator contract one line emits
                    //   only one boundary, which was already pulled out above, but on the day that contract breaks, guarding here
                    //   is cheaper than quietly losing it.
                    if ours || matches!(ev, OutputEvent::TurnEnd { .. }) {
                        self.core.emit(ev);
                    } else {
                        self.core.emit_without_turn_observation(ev);
                    }
                }
                // ★The boundary after a receipt outside the open turn (reason = [`Reader::note_delivered`]) — emitted through the observation-free door★: the turn table is already
                //   idle (that receipt also went out without observation), so there is no consumer to wake, and emitting through the ordinary door would make the classifier fold it into a clean end and
                //   erase the preceding error end (`last_end_failed`) — mail that should stay halted would flow.
                //   If a new turn opened in between and the receipt was counted as that turn's progress (`ours`), that turn's end is the boundary.
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
                // ADR-0226: after emitting and tracing, outside the lock (reason = that function's doc).
                self.witness_first_turn(&method, params.as_ref());
            }
            Ok(Inbound::Response { id, result }) => self.resolve(&id, Ok(result)),
            Ok(Inbound::Error { id, error }) => {
                // ★Masking is done **here**, not at the boundary★ — this string goes both to the log and to the screen,
                //   and also goes back to the caller. Filtering again at every outgoing door means one of them is bound to be forgotten.
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
    // ★The point is that this guard stands at the very top of the function★ — wherever unwind comes up from below, `Drop` runs.
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

    // ★Closing is carried by `ReaderExit` above — not written again here★ (unwind must pass the same place too).
    // ★No flush on the kill path★ — that tail is a fragment cut because we killed the process.
    if !shutdown.load(Ordering::Acquire) {
        if let Some(dec) = reader.decoder.as_mut() {
            for ev in dec.flush() {
                reader.core.emit(ev);
            }
            reader.core.report_usage(dec.take_usage());
        }
    }

    // ★One `try_wait` right after EOF cannot get the exit code★ — stdout closing and the process being
    //   reaped are not the same moment, so that one call usually falls to `Ok(None)` and the code is quietly
    //   **lost**. That loss hurts twice: the exit code does not show on screen, and `Exited { code: None }`
    //   cannot be told apart from 「the child is still alive」, so **the test bench loses its means of telling the two apart**.
    // ★So it looks again, bounded★ — the lock is let go on every attempt (sleeping while holding it would block `shutdown`'s kill that long).
    // ★The kill path does not run a single round of this loop★ — `shutdown()` already did `wait()`, so the first attempt
    //   gives the value right away.
    // ★Still, `None` can remain, and that is the honest outcome★ — that is the case where the peer closed only stdout and did not end,
    //   and that disposal is carried separately by [`wait_for_close_or_tear_down`].
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

/// The common receipt of `send_input` · `send_turn`. The `Err` conditions have [`AgentTransport::send_input`] doc as the source of truth. `Ok(true)` = this
/// item lifted the halt after error ([`State::push_pending`] — the caller writes that log outside the lock).
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

/// The body of [`AgentTransport::withdraw`] — that doc is the source of truth for the contract. `core` = where to emit (`None` before `start()`
/// — then there cannot be a list item that finished announcing).
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
            // ★Already handed over — cannot be withdrawn · the row stays in the list★ (the frontend draws it like a handed-over row without ✕): the vendor decides the outcome
            //   (echo = received · steer refusal or turn end without an echo = withdrawn), and here the cancellation receipt and 「could not remove it from the vendor
            //   queue」 follow immediately (codex has no verb to withdraw handed-over text).
            //   ★An overlapping second ✕ (window + LLM — it does not go through the input lock) is `TooLate` without an event★ — the registry's
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
    // ★Emitted after letting go of the lock★ (ADR-0006).
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
        // ★Filled before the writer★ — the input thread announces after the writer's floor decision, so it must stand before that.
        let _ = self.core.set(core.clone());

        // ── stderr drain ──
        // The peer's diagnostic text comes on this stream (measured 0.154.0 — errors went here, not to stdout), so it is the only evidence of an activation
        //   failure.
        if let Some(stderr) = sync::lock(&self.stderr).take() {
            drain_stderr(stderr, &core, "engram-codex-stderr", "codex");
        }

        // ── Writer ──
        // ★Nobody joins it★ — `core` holds only one pump handle (widening that place is a core
        //   change), and waiting inside `shutdown()` is a contract violation. When kill breaks the pipe, this thread's
        //   blocked write is released, and it sees the closed marker and ends on its own.
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
                    // Without a writer, neither the handshake nor input ever goes out — it is not left silent.
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
            // ADR-0292: finish is outside the marking — do not move it into the closure (reason = the same spot in `transport/pty.rs`).
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
    /// ★The receipt has one meaning★: `Ok` = received all of it with the order settled, `Err` = not received. ★It does not report a reduction
    ///   with a short `Ok`★.
    /// ★`Err` is only what is decided **before** saying "we will take it"★ — ① the queue cap is exceeded ② this incarnation's
    ///   connection has already ended. Input queued in the handshake window already received `Ok`, so ② is not widened past
    ///   that point.
    /// ★Known limit★: a write that failed after accepting has no way back to this caller. What remains then is one thing on the output
    ///   stream, **the turn boundary of a failure outcome** ([`end_turn_if`], and the branch where the handshake failed and
    ///   the queue is dropped entirely is emitted directly by [`writer_loop`]), and the `Ok` already given is not corrected.
    fn send_input(&self, input: InputEvent) -> Result<(), PtyError> {
        let InputEvent::Raw(bytes) = input;
        // It is from mail, so it does not lift the halt.
        accept_input(&self.state, None, bytes, InputOrigin::Mail).map(|_| ())
    }

    /// The same receipt as [`AgentTransport::send_input`] — carries the identifier and source on the item, and after the floor decision
    /// finishes the announcement (synthetic bubble · `Queued`) before returning ([`announce`]).
    // ADR-0231
    fn send_turn(&self, turn: TurnInput) -> Result<(), PtyError> {
        let lifted = accept_input(&self.state, Some(turn.id), turn.body, turn.origin)?;
        if let Some(core) = self.core.get() {
            // ADR-0231: written outside the lock (reason = [`TurnNoteLog`]).
            if lifted {
                Transition::HaltLifted.write(core.id());
            }
            announce(core, &self.state, &self.announcer);
        }
        Ok(())
    }

    /// ✕ — a held list item (including text returned to the held place at the turn end) is withdrawn (`Dropped{Withdrawn}` · no vendor call), and a handed-over list
    /// item is `TooLate` (`CancelRequested` followed immediately by `CancelAnswered{removed:false}` — without an event if ✕ already reached it. If that
    /// text's turn ends without an echo, it is withdrawn then as `Dropped{Withdrawn}`), and outside the list (Direct · mail · no identifier ·
    /// below the floor) and unknown ids are `NotHeld`.
    ///
    /// ★The order stands under the same state lock as the handover★ — a ✕ that came after the writer moved `Held` to `InFlight` is on the handed-over
    ///   side. ★A pre-announce item gets only a death mark and `Withdrawn` is answered★ — `Dropped` is followed by the announcing side
    ///   right after `Queued` ([`State::drop_listed`]). If a death mark is already there, the first ✕ removed it, so it is
    ///   `NotHeld`.
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
    /// ★If the turn id is not known yet, it is `Err`★ — the peer requires that field, and an envelope sent without it gets not even
    ///   an answer (measured 0.154.0: an undecodable envelope gets no answer).
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

    /// The app-server version of the ADR-0001 2 verbs. ★Nothing is waited for inside★ (waiting is `core.join_pump`'s job).
    ///
    /// ★Order invariant — stdin close must never come before kill★: the writer holds the stdin lock throughout a blocking
    ///   `write_all`. If the peer does not read stdin (pipe backpressure), that write blocks forever and
    ///   does not let go of the lock, so trying to take `stdin.lock()` before kill would never even reach kill, the pump
    ///   could not wake, and `join_pump` would hang forever. Killing the child first breaks the pipe, so that write is released with an error
    ///   and the lock is released.
    /// ※It was measured that closing stdin alone makes the peer exit on its own (41–51ms with no turn),
    ///   but that observation does not change the order invariant above — without getting the lock, it cannot even reach the closing place.
    // ADR-0001
    fn shutdown(&self) {
        self.shutdown.store(true, Ordering::Release);

        // ★Only the state lock is taken here★ — the thread holding a blocking write does not hold this lock
        //   (it lets go before writing), so this step cannot hang. The queue not yet sent disappears here.
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

        // Grandchildren (codex under the cmd shim, and the MCP children codex launched at thread/start) end together too. On an OS with no group
        //   it is a no-op.
        let _ = self.group.terminate(1);

        // If try_lock cannot be obtained (the moment write_all has not been released yet), skip — the OS reclaims an uncleaned ChildStdin on
        //   drop. ★No blocking lock★.
        if let Ok(mut guard) = self.stdin.try_lock() {
            let _ = guard.take();
        }

        // Pending requests are woken here too — so they do not remain even when the reader ends without seeing EOF.
        for entry in self.pending.close() {
            if let Waiter::Handshake(tx) = entry.waiter {
                let _ = tx.send(Err(format!("{}: the link closed", entry.method)));
            }
        }
    }

    fn capabilities(&self) -> TransportCaps {
        TransportCaps {
            input: InputCaps {
                // ★`raw` is false because this channel does not carry keystrokes★ — the received bytes are
                //   complete message bodies, and streaming them in one character at a time would open a turn per character.
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

    /// Collects emitted events as is — `snapshot()` returns only `TerminalBytes`, so structured events
    /// can be seen only by subscribing.
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

    /// A subscriber that panics inside `core.emit` — one of the two unwind paths the review named (the other is
    /// `OutputCore` mutex poison). `emit` does not catch a sink panic and sends it up as is.
    struct PanickingSink(SinkId);
    impl OutputSink for PanickingSink {
        fn send(&self, _frame: OutputFrame<'_>) -> Result<(), SinkError> {
            panic!("subscriber blew up inside emit");
        }
        fn sink_id(&self) -> SinkId {
            self.0
        }
    }

    /// On receiving one notification line, emits an event — that emit passes through the subscriber above and unwinds the reader
    /// **inside the read loop**.
    ///
    /// ★It must be `decode`, not flush★: the old shape that wrote the close at the loop tail closed **before** flush,
    ///   so a panic blowing up in flush did not leak in that shape either. The leaking spot is the read loop.
    struct EmittingDecoder;
    impl OutputDecoder for EmittingDecoder {
        fn decode(&mut self, _chunk: &[u8]) -> Vec<OutputEvent> {
            vec![OutputEvent::Error("mid-stream".into())]
        }
        fn flush(&mut self) -> Vec<OutputEvent> {
            Vec::new()
        }
    }

    /// A probe that streams one notification line to stdout. ★Not built with `echo`★ — on that path the quotes change while passing the shell
    /// and the envelope is not classified as a notification.
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
        /// Exactly the shape `send_input` puts in (no identifier · mail).
        fn push_legacy(&mut self, body: &[u8]) {
            self.push_pending(None, body.to_vec(), InputOrigin::Mail);
        }
    }

    /// Opens one turn with marker `seq`. ★The point is that the test bench picks the marker directly★ — which turn a late signal
    /// belongs to is told apart only by that marker.
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

    /// A fake translator that sees exactly what the link put into the translator.
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

    /// The test bench with the real translator plugged in. ★Measuring the attribution gate must use this one★ — [`RecordingDecoder`]
    /// emits no events at all, so measuring with it, 「blocked the boundary」 would pass **not because it blocked but because there was none
    /// to begin with**.
    fn harness_with_real_decoder() -> Harness {
        let mut h = harness();
        h.reader.decoder = Some(Box::new(super::super::decoder::CodexAppServerDecoder::new()));
        h
    }

    /// The test bench with the real translator **and** the real fact layer plugged in together — measures in one place whether 「the screen boundary」 and
    /// 「is this incarnation mid-turn」 get the same decision. The incarnation marker is fixed at 1 (there is only one incarnation on this test bench).
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

    /// A one-line `turn/completed` — a shape the real translator can read.
    fn completed_line(thread_id: &str, turn_id: &str, status: &str) -> String {
        serde_json::json!({
            "method": "turn/completed",
            "params": {"threadId": thread_id,
                       "turn": {"id": turn_id, "items": [], "status": status}},
        })
        .to_string()
    }

    /// One notification line **in the middle** of a turn — the kind that becomes a progress signal, not a boundary.
    fn progress_line(thread_id: &str, turn_id: &str) -> String {
        serde_json::json!({
            "method": "item/agentMessage/delta",
            "params": {"threadId": thread_id, "turnId": turn_id, "itemId": "i-1", "delta": "hi"},
        })
        .to_string()
    }

    // ── Carrying usage observations (`take_usage` → `StatusSink::usage_observed`) ──

    /// A status sink that collects only `usage_observed`.
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

    /// ★The observation passes the read loop and reaches the status sink★ — if the translator only gathered it and nobody emptied it, it would disappear without an
    /// error. Nothing goes to the screen (subscribers).
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

    /// A translator blocked by the wrapper (non-default profile) hands nothing to the sink — the decision itself is measured by this folder's `usage` tests.
    #[test]
    fn a_gated_decoder_reports_no_usage() {
        let gated = crate::usage::UsageGate::blocking(Box::new(CodexAppServerDecoder::new()));
        let (mut reader, seen, observed) = usage_reader(Box::new(gated));
        reader.handle_line(rate_limits_line().as_bytes());
        assert!(observed.lock().unwrap().is_empty());
        assert!(seen.lock().unwrap().is_empty());
    }

    /// The test bench with the table + codex classifier + real translator plugged in — ★only for items that actually measure the fact layer★.
    ///
    /// The default [`harness`] is `TurnWiring::detached()`, so the table is silent — measuring with it, 「did not turn mid-turn on」 would pass
    /// **not because it did not turn it on but because the table is off**.
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

    /// ★A line that came while no turn is open goes to the screen but does not turn 「mid-turn」 on★ — if it did, that turn's end would be
    /// blocked by the attribution gate and nobody could turn it off, and until the 30-minute fail-open valve sweeps it, no mail would go to that agent
    /// (the link is Idle = queue open but only the fact layer is busy — the two axes split).
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
        // ★It did not pass by dropping it★ — that line goes to the screen as is. Only the fact layer was split off.
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

    /// Its pair — the same line that came while our turn is open turns it on, and that turn's end turns it off.
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

    /// ★Item notifications that come before the response are normal★ — blocking that window by id match would push our turn's streaming
    /// entirely out of the fact layer. So the decision is not 「does it match」 but 「does it not contradict」.
    #[test]
    fn a_line_that_arrives_before_the_turn_id_is_known_still_counts_as_ours() {
        let mut h = fact_harness();
        make_ready(&h.state, "T");
        activate(&h.state, 0, None);

        h.reader
            .handle_line(progress_line("T", "U-MINE").as_bytes());
        assert!(h.table.is_in_turn(h.id, 1));
    }

    /// ★A held end is dropped together with that turn★ — if left, the moment the peer reuses the id,
    /// [`Reader::resolve`] closes **the freshly opened turn** on the spot and raises a stale boundary to the screen.
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

        // That turn ends by another path (deadline expiry · write failure · error response — all come via end_turn_if).
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

        // Even if the next turn receives the same id, it lives on open.
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

    /// ★The turn id the response decides is capped too★ — without it that value goes out as is into the boundary field, alone exceeding the replay
    /// ring's single-event cap, and that ring evicts everything else. It is a comparison token, so it is filtered, not truncated,
    /// so that turn has no material for attribution — so it is closed with the same grade as an error response.
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

    /// Someone else's turn · someone else's thread are not counted even while our turn is open — the same axis as the boundary gate.
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

    /// ★The core of TRD §4-4★ — server ids and our ids inevitably overlap (because server ids are returned as is).
    /// What tells them apart is not the value but **the envelope shape**.
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

    /// ★The error body the peer gave goes to three places at once — log · screen · caller — so masking is done not in front of those three but **where it is
    /// made**★. With three doors, one of them is bound to be forgotten.
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

    /// ★[`clip`] must be called only inside [`sanitize`]★ — called elsewhere, that string skips masking
    /// and goes out to the log · screen. With two doors one is forgotten, and the forgotten one leaks **quietly**.
    ///
    /// This item measures that property **in the source** — keeping one credential item per leaking path
    /// is not maintainable, and a path without one becomes the leaking path.
    #[test]
    fn clip_is_only_reachable_through_the_masking_door() {
        let src = include_str!("transport.rs");
        // ★It is not split by `#[cfg(test)]`★ — that attribute is inside the production section too (test-bench-only accessors),
        //   and cut there this item would scan only the front part and **not see the back part**.
        // ★A marker containing a newline is not used either★ — this repository checks out with CRLF, so `\n` would not match.
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

    /// If the stream were cut by an unreadable line the agent would die — it drops it and keeps going.
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

    /// ★`turn/started` takes no part in identity — do not revive it★. That notification has no identifier we issued,
    /// so in principle it cannot tell which turn it belongs to. Telling it apart instead by 「is the current turn waiting for an answer」, a late notification of the previous turn
    /// passes that condition **while the new turn waits for its own request** and writes its id into the new turn's slot.
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

    /// ★A late response of the previous turn must not write its id into the current turn's slot★ — if written, the `interrupt` after it
    /// aims at the wrong turn. What tells them apart is the marker, and without the marker both look like "the turn id is empty".
    #[test]
    fn a_late_response_from_a_finished_turn_never_lands_on_the_next_turn() {
        let mut h = harness();
        make_ready(&h.state, "T");
        // Turn 0 opened and its response has not come yet.
        activate(&h.state, 0, None);
        let _ = h
            .pending
            .register(5, Waiter::TurnStart { seq: 0 }, method::TURN_START);
        // That turn ended and turn 1 opened.
        activate(&h.state, 1, None);
        // Now the answer for turn 0 arrives.
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

    /// ★An unknown threadId is not an error★ — but a threadId that is **known and different** is not our turn.
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

    /// ★A signal that **another turn** of the same thread ended must not close our turn either★ — if it did, the next turn would open and
    /// two turns would run at once on one thread. Preventing exactly that is why the input queue exists.
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

    /// ★An end that came while the turn id is unknown ends nothing★ — closing a live turn with an unattributable signal
    /// opens the queue and turns overlap.
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

    /// ★An item that measures exactly the order that used to get stuck★: the end comes first (unattributable → ignored), and **without a sweep in between**
    /// the response comes and decides our turn's id. That response takes away the waiting entry, so if it is not ended here that turn has nothing
    /// to end it forever — no deadline, and no second end comes.
    ///
    /// ★Inserting a sweep would stop measuring this order★ — the deadline would end it instead, and the response-side comparison would never be called.
    #[test]
    fn an_early_completion_is_reconciled_when_the_response_names_it() {
        let mut h = harness();
        make_ready(&h.state, "T");
        activate(&h.state, 0, None);
        with_state(&h.state, |s| s.push_legacy(b"queued"));
        let _ = h
            .pending
            .register(5, Waiter::TurnStart { seq: 0 }, method::TURN_START);

        // ① The end comes first — at this point our turn's id is unknown, so it cannot be attributed.
        h.reader.handle_line(
            br#"{"method":"turn/completed","params":{"threadId":"T","turn":{"id":"U-EARLY"}}}"#,
        );
        assert_eq!(
            turn_seq(&h.state),
            Some(0),
            "an unattributable end closed the turn on the spot"
        );

        // ② Then the response comes — here our turn's id is decided as `U-EARLY`. No sweep is inserted.
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
    // What the items above measure is that **this link's state machine** is not fooled by someone else's end, and what the items below
    // measure is that **the screen and the fact layer** get the same decision. If the two split, the link keeps the queue closed while
    // the screen draws the turn as ended, and the fact layer observes idleness and plugs mail in mid-turn.

    /// ★Exactly the input the review named★: our turn is `U-MINE` but an end of `U-OTHER` of the same thread comes.
    /// The link ignores it, but the translator does not know attribution and emits a boundary — if that boundary flowed as is, the classifier and
    /// the frontend both do not look at the id, so **the live `U-MINE` would be stamped as ended.**
    #[test]
    fn a_foreign_turn_completed_never_ends_the_live_turn() {
        let (mut h, turns, id) = harness_with_facts();
        make_ready(&h.state, "MINE");
        activate(&h.state, 0, Some("U-MINE"));
        // The echoed user message = the first observation that this turn is running.
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

    /// The end of another thread is the same — blocked, but left in the log (reason = [`Reader::note_turn`] doc).
    #[test]
    fn a_turn_completed_from_another_thread_makes_no_boundary() {
        let mut h = harness_with_real_decoder();
        make_ready(&h.state, "MINE");
        activate(&h.state, 0, Some("U-MINE"));
        h.reader
            .handle_line(completed_line("OTHER", "U-MINE", "completed").as_bytes());
        assert!(boundaries(&h.seen).is_empty(), "{:?}", boundaries(&h.seen));
    }

    /// ★If attributed, the boundary flows **as is** — including the outcome★. If the gate only 「blocks」 and cannot let anything through,
    /// the waiting indicator spins forever.
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

    /// ★The boundary of an end that came before the response is **neither dropped nor let through but deferred**★ — letting it through on the spot would let an end not yet
    /// attributed close a live turn, and dropping it would leave nothing to raise after the comparison holds, so the waiting indicator
    /// spins forever. Even though the outcome was known.
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

    /// The comparison is still only **the id match** — no boundary comes up merely by being held.
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

    /// ★The only function that folds a turn **always** emits a boundary★ — that is the only means by which this link's four give-up paths (write failure · response
    /// decode failure · error response · deadline expiry) return the fact layer. And if the marker does not match it
    /// ends nothing and **emits no boundary either** — a late signal must not invent the next turn's boundary.
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

    /// ★This is the very outcome ADR-0127 is meant to prevent★: if the link gave up mid-turn and the fact layer did not return,
    /// that incarnation would remain observed as 「mid-turn」 although idle, and until the 30-minute fail-open valve sweeps it
    /// mail is blocked. ★One [`OutputEvent::Error`] line cannot do this★ — the classifier does not count that vocabulary as a turn signal
    /// (that was the mechanism of this defect).
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

    /// The path where an error response comes to `turn/start` — one of the four give-ups. The reason goes out carried inside the boundary.
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

    /// ★Even if the real end comes late to a turn closed by giving up, there is **one** boundary★ — if two went out, the screen would close the same turn twice,
    /// and an unpaired end would pile up once more in the fact layer.
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

    /// Runs the writer once without stdin to make the handshake fail. `queued` = bodies that had already received `Ok` in that window and
    /// were queued.
    fn run_failed_handshake(queued: &[&str]) -> (SharedState, Arc<Mutex<Vec<OutputEvent>>>) {
        let (core, seen) = core_with_sink();
        let state = shared();
        with_state(&state, |s| {
            for body in queued {
                s.push_legacy(body.as_bytes());
            }
        });

        writer_loop(
            // There is no stdin, so the handshake fails at the first write.
            Arc::new(Mutex::new(None)),
            state.clone(),
            Arc::new(Pending::default()),
            Arc::new(AtomicI64::new(0)),
            // After the handshake the loop ends at the first check — what these items measure is the section before it.
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

    /// Pulls the errors · boundaries raised to the screen **in arrival order** — because what must be measured on this branch is not only the count but
    /// 「does the reason come up before the boundary」.
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

    /// ★Even with no dropped input, **one** failure outcome goes out★ — the place that opens a turn requires [`Link::Ready`],
    /// so this incarnation never had a turn, but the frontend does not read `Error` as a turn end, so a slot holding only an error
    /// has **its waiting indicator spin forever** (the link is `Down`, so nothing will arrive after it either).
    /// ★That reason does not claim input loss★ — because nothing was dropped.
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

    /// ★If queued bodies were destroyed, **one** failure outcome goes out★ — because the sending side had turns,
    /// and `dropped` is the number of dropped bodies, not the number of turns. ★Without this boundary★ the frontend does not read `Error`
    /// as a turn end (retried stream errors come in that vocabulary), so a slot that received only an error has **its waiting indicator
    /// spin forever**.
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

    /// ★If the places that return a turn to idle grow, this goes red★ — each place must answer 「who emits the boundary」,
    /// and a place that does not answer is exactly where the fact layer freezes (ADR-0127). Today's four = [`end_turn_if`] (emits the boundary
    /// itself) · [`Reader::note_turn`] (lets the translator boundary through) · [`Reader::resolve`] (raises the held
    /// boundary) · [`ReaderExit::drop`] (the stream ended — `OutputCore::finish` soon takes the table away).
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
        // ★Every such place must also drop the held ends★ — if not dropped, that slot survives until the next turn,
        //   and the moment the peer reuses the id, the freshly opened turn is closed by a stale boundary (that line of [`end_turn_if`]).
        let drops = production
            .lines()
            .map(|l| l.trim())
            .filter(|l| l.starts_with("s.early_completions.clear()"))
            .count();
        assert_eq!(
            drops, sites,
            "the number of idle places differs from the number of places dropping held ends"
        );
        // ★Every such place must also dispose that turn's items (ADR-0231)★ — at a missed place, the `InFlight` of the ended turn
        //   remains without an outcome and turns all new text into waiting. The only exception is the stream closing (list items are
        //   closed by `OutputCore::finish` as `Dropped{AgentEnded}`, and the link is no longer used).
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

    /// ★Even if an unattributed end is dropped, that turn's **deadline must remain**★ — taking away the waiting entry while ignoring it
    /// would leave that turn with nothing to end it. This item measures that one thing, and **not** 「a turn that could not be attributed
    /// always has a deadline」 (that generalization is false — there are cases where the end comes before the response,
    /// and that slot is carried by the module header's 「known limits」).
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

    /// ★An undecodable success response is **the same grade** as an explicit error★ — the waiting entry was already taken away, so if the turn
    /// is not ended here, that turn remains open forever with neither `turn_id` nor a deadline (after that `send_input` reports that stall as
    /// "the queue is full").
    #[test]
    fn a_success_response_we_cannot_read_ends_the_turn_like_an_error_would() {
        let mut h = harness();
        make_ready(&h.state, "T");
        activate(&h.state, 0, None);
        with_state(&h.state, |s| s.push_legacy(b"queued"));
        let _ = h
            .pending
            .register(5, Waiter::TurnStart { seq: 0 }, method::TURN_START);

        // A success response without the `turn` field — the envelope is sound and only the payload does not read as our type.
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

    /// A handed-over item is also one entry — it is visible in the list until receipt (echo).
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

    /// ★This item **cannot measure** atomicity — the signature already carries that property★: [`take_turn_locked`]
    /// takes `&mut State`, so exclusive access is a compiler guarantee, and the lock is in this file's harness (`with_state`).
    /// So there is practically no mutation that can turn this item red.
    /// It is still kept because it looks at **whether the accounting is right** (even with eight rushing in, only one is consumed from the queue),
    /// and that one thing is all this item measures.
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

    /// What this item measures is **one thing, the `Ready` transition** — not 「that value is on disk」 (this link does not carry
    /// that guarantee: the record port swallows failures inside itself).
    /// ★The point is measuring with only the thread id filled and `Ready` not set★ — filling both together lets the absence of the id
    /// block it instead, so **it stays green even if the gate is removed** (measured by mutation).
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

    /// ★An already ended incarnation is not recorded★ — if written, a dead session's profile holds someone else's conversation as its own,
    /// and the old sid is pushed into history. ★But what this measures is only an end **already observed**★ — a session that ends between that decision and the actual
    /// record is outside the range this item measures, and passes today (`record_session_id` doc).
    /// ★The point is measuring **both** markers★ — `shutdown()` sets the atomic first and then takes the state lock to
    /// set `closed`. The span in between is not empty, so a fence looking at only one side leaks that much.
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

    /// The pair direction — if neither stands, it records. Without it, the item above would be green even as 「always skip」.
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

    /// ★Even if the record panics, it does not take the writer with it★ — if unwind went up, neither `Ready` nor `Down` would stand, so
    /// the link would stop at `Connecting`, and in that state [`AgentTransport::send_input`] would see neither closed nor `Down`,
    /// return `Ok` and only fill the queue (the 「silent stop」 ADR-0190 forbids). Only by turning into `Err` does the writer go
    /// to the failure branch and take the link down. It is the branch of a build without the hook — in the production process the hook ends it (ADR-0292).
    #[test]
    fn a_panicking_sink_becomes_a_failure_instead_of_unwinding() {
        let state = shared();
        let sink: SessionIdSink = Arc::new(|_| panic!("the record port blew up"));

        // ★The hook is not swapped bare-handed★ — the hook is process-global, so other items running concurrently in the same binary
        //   would lose their own panic output (undiagnosable), or an overlapping span would leave the quiet hook forever. A helper that prevents
        //   those two accidents already exists.
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

    /// The writer picks control lines **before** user turns — if a refusal waited behind the queue, the peer would stop forever.
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

    /// ★A waiting entry registered after the link closed holds its thread until its deadline runs out★ — the handshake issues two
    /// requests back to back, so a window of closing in between really exists. If the waiting table itself closes, that wait ends immediately.
    ///
    /// ★The **time** assertion below cannot measure the waiting table closing★ — `shutdown()` already took stdin, so
    /// the write fails right away, and this function returns quickly even if the closed marker is removed. What actually measures that marker is
    /// only the two assertions above (is a new waiting entry refused · is `interrupt` refused).
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

    /// ★Even if the peer ends on its own, the writer thread must end★ — if not, that thread remains holding the `Arc`s of core · stdin · the waiting table,
    /// and one session's worth of memory remains with it. `shutdown()` is not called on this path
    /// (the reaper only removes the session from the registry).
    ///
    /// ★This item is **the only place that actually runs** both loops★ — the other link items call `handle_line` ·
    /// `next_job` · `sweep_deadlines` directly.
    #[cfg(windows)]
    #[test]
    fn the_writer_thread_ends_when_the_peer_exits_on_its_own() {
        let (t, _) = open_probe(&["/c", "echo", "{}"]);
        let (core, _seen) = core_with_sink();
        t.start(core.clone());

        // The reader (pump) ends with EOF.
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

    /// ★The same marker must survive a reader **panic** too★ — `contain` wraps the reader loop from outside,
    /// so cleanup written at the loop tail is simply passed over by unwind. If the marker does not stand on that path, the writer spins forever holding core ·
    /// stdin · the waiting table, and that is the other half the normal EOF item cannot see.
    ///
    /// The unwind is produced by **making a subscriber panic inside `core.emit`** — exactly the path the review named, and
    /// the only way this file reaches that point without the real codex.
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

    /// ★The control queue must go out even while the handshake waits for an answer★ — the writer is the only thread that drains that queue,
    /// so if it parked entirely, a server request refusal the reader put in could not go out until the deadline runs out, and if the peer was waiting for that
    /// answer, both sides would wait for each other.
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

    /// ★The order of the sliced wait is the contract — measure the deadline **first**, then write one line★.
    ///
    /// What is measured is exactly that one order: in a slice whose deadline already passed, **it returns without attempting a write.**
    /// Flip the order and that one write cannot get the stdin lock and hangs, and the deadline is never evaluated.
    ///
    /// ★Make clear **what this item cannot measure**★: if the write blocks in a slice whose deadline has not passed yet, this
    /// wait is still infinite. This order cannot fix that, and the only way out is `shutdown()`'s kill
    /// (module header 「known limits」). Do not quote this item as that guarantee.
    #[cfg(windows)]
    #[test]
    fn an_expired_slice_returns_without_attempting_another_write() {
        // The first request write must succeed (a small line), so only the deadline is made short. By the time one slice (500ms) runs,
        //   it has already passed.
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

        // Hold stdin **after** the first request write ends — the peer does not read stdin, so this write hangs forever
        //   holding the lock.
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

    /// ★Failing to send a refusal because the control queue is full must show on screen★ — left only in the log, the peer waits for the answer
    /// forever and this side has no signal. The sibling event (`turn/start` deadline) already shows on screen.
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

    /// ★The record call must come **before** the gate opens★ — this is the reason the one-verb port exists
    /// ([`crate::backend::SessionIdSink`]: this order is why there is no second verb asking back 「was it written」).
    /// Flipped, a turn would go out with that session before it is recorded, and the port would become **a notification that guarantees nothing**.
    ///
    /// ★Why it is measured in the source★ — the port is `Fn(&str)`, so it cannot see the link state at the time it is called, and making it see that
    /// would break the very one-verb contract this item protects. The other half of the gate (「a turn goes out only when
    /// `Ready`」) is measured by actually running [`tests::nothing_is_sent_before_the_link_is_ready_even_when_the_thread_id_is_known`]
    /// — together the two cover the order 「record → gate → send」.
    /// Precedent · same reason = `the_guard_is_armed_and_the_child_adopted_first_after_spawn` of the `crate::transport::spawn` tests.
    /// ★What this item pins goes only as far as 「**handed** to the port before the gate」 (ADR-0226)★ — behind the port is the first-submission
    ///   latch, so persistence is tied to submission. 「Persist before the first turn」 is carried together by the latch unit test (queueing behind an in-progress commit) ·
    ///   the session's submission counting order test · the assembly point's wiring structure test (`manager`'s
    ///   `the_first_submission_latch_is_wired_through_the_spawn_path`).
    #[test]
    fn the_session_id_is_recorded_before_the_gate_opens() {
        let body = writer_loop_code();

        let recorded = body.find("record_session_id(").expect(
            "there is no `record_session_id(` call in writer_loop — this item's premise is stale",
        );
        // ★The anchor moved from `Link::Ready` to `open_gate(`★ — as the line that sets the gate moved into that function,
        //   the `Link::Ready` left in this body became **only comments**. Left as is, this item would measure a comment
        //   position and pass quietly (it actually passed that way).
        let gate_open = body
            .find("open_gate(")
            .expect("there is no `open_gate(` call in writer_loop — this item's premise is stale");

        assert!(
            recorded < gate_open,
            "the record call stands after the gate (`Link::Ready`) — a turn can go out before it is recorded, and then the one-\
             verb port guarantees nothing (a second verb becomes necessary)"
        );
    }

    /// Only the code of the `writer_loop` body **with comments stripped** — the shared lens of the items that measure order · door choice in the source.
    ///
    /// ★★Stripping comments is why this helper exists★★: this file's comments quote the names they guard
    ///   as is (that is this repository's comment convention), so scanning raw, **one comment line poses as a real call.**
    ///   It actually passed that way once — even after the line setting the gate moved into `open_gate`,
    ///   two items looking for `Link::Ready` were green measuring the **comments** left in the body.
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

    /// Pulls only text — the lens used by the items measuring order.
    fn texts(events: &[OutputEvent]) -> Vec<String> {
        events
            .iter()
            .filter_map(|e| match e {
                OutputEvent::TextDelta { text, .. } => Some(text.clone()),
                _ => None,
            })
            .collect()
    }

    /// ★Pages come newest first, and what goes on the screen must be **in time order**★ — that flip is the only order contract
    /// of this path (measured 2026-09-16, real 0.154.0: the first page's `data[0]` of `sortDirection: desc`
    /// is the last assistant response, and the end of the last page is the first `userMessage`).
    /// ★The same item also measures the request shape★ — if direction · cursor continuation is wrong, the screen stands upside down or the same
    /// page is fetched again forever.
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

    /// ★Exactly one close per turn★ — without closing, the waiting indicator of the restored slot spins forever, and closing twice
    /// piles up empty separators. Closing the last turn too is half of this item.
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

    /// ★If there is nothing at all to carry over, **nothing is emitted**★ — making a list of only boundaries would create separators in an empty
    /// slot. Variants live drops (reasoning etc.) get the same decision in history too.
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

    /// ★★Even if one page fails, it is **not an activation failure**★★ — the screen received up to that point is carried
    /// as is and closed normally (ADR-0203: history failure is quiet and harmless).
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

    /// ★★The ceiling is the ring★★ (ADR-0203) — receiving beyond 2MiB is pure cost, since the ring drops from the front.
    /// That decision must be made on the axis the ring actually counts ([`estimate_cost_bytes`]).
    #[test]
    fn history_stops_once_the_replay_ring_is_full() {
        // One assistant body filled up to the cap (64Ki characters) — 32 of them is 2MiB.
        let big = "x".repeat(64 * 1024);
        let mut calls = 0;
        let events = collect_history("th", "c".to_string(), far_deadline(), |_, _| {
            calls += 1;
            Ok(page(
                (0..HISTORY_PAGE_LIMIT)
                    .map(|i| entry("t1", agent_message(&format!("m{calls}-{i}"), &big)))
                    .collect(),
                // The cursor does not end — the only thing that stops it must be the ceiling.
                Some("more"),
            ))
        });

        let weight: usize = events.iter().map(estimate_cost_bytes).sum();
        // ★★It stops **before** exceeding — do not revert to 「stop after exceeding」★★: measuring after putting always exceeds by one
        //   entry, and that one is dropped by the ring right on landing. Tying the ceiling to the ring constant was meant to remove that
        //   mismatch.
        assert!(
            weight <= REPLAY_MAX_BYTES,
            "received past the ceiling: {weight}B > {REPLAY_MAX_BYTES}B"
        );
        // While doing so it fills it **almost** — it must be within a margin of one entry (64KiB).
        assert!(
            weight + 64 * 1024 >= REPLAY_MAX_BYTES,
            "stopped far short of filling the ring: {weight}B"
        );
        assert_eq!(
            calls, 2,
            "kept asking for pages after the ring filled (wasted round trips) — {calls} times"
        );
    }

    /// ★★The ceiling accounting must count **the synthesized boundaries too**★★ — without counting them, in a history where every item splits a turn the actual
    /// event count becomes twice what was counted, and the ring (4096 entries) drops half right on landing. That half is **the
    /// oldest side**, so the head of the restored conversation is cut off entirely.
    #[test]
    fn the_ceiling_counts_the_boundaries_it_synthesizes() {
        let mut n = 0usize;
        let events = collect_history("th", "c".to_string(), far_deadline(), |_, _| {
            let data = (0..HISTORY_PAGE_LIMIT)
                .map(|_| {
                    n += 1;
                    // Every item has a different turn = the worst case where every item gets one more boundary.
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
        // Back when boundaries were not counted, the item count here would have gone up to the ceiling — it should be around half.
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

    /// ★★`turnId` has no wire cap — it is filtered before holding★★: the line cap applies only to **one** page,
    /// not to the accumulation piled up here, so if items with long ids come on every page, the original strings alone reach hundreds of MiB
    /// (25 × 160KiB does not exceed 4MiB on one page, but gathered up to the count ceiling it is ~650MiB).
    /// ★★While doing so, **different turns must stay two as they are**★★ — folding all ids over the cap into one slot (`None`)
    /// catches the memory, but **two different conversations render as one turn.** So distinction is carried by the original comparison with the previous
    /// item (O(1)), and the filtered id is carried on the boundary only as **display**. What this item measures is those
    /// two axes — two boundaries, but no id carried.
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
        // When the same id continues, then it is one group — pins that the split above is not 「always split」.
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

    /// ★★The same page is not fetched again from a peer that returns the same cursor★★ — if it were, the same conversation would be carried several
    /// times and those duplicates would push the real history out of the ring. The page cap bounds the loop but cannot prevent this contamination.
    #[test]
    fn a_repeating_cursor_stops_instead_of_duplicating_history() {
        let mut calls = 0usize;
        let events = collect_history("th", "c".to_string(), far_deadline(), |_, _| {
            calls += 1;
            Ok(page(
                vec![entry("t1", agent_message("m1", "once"))],
                // The cursor the first answer gave keeps being returned after that.
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

    /// ★It does not hang forever on a cursor that never ends★ — the budget usually cuts first, but for a peer that answers
    /// immediately only this backstop remains.
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
                // ★A **different** cursor every time★ — giving the same value, the repeat guard would cut first, and this item would measure that guard
                //   instead of the cap it means to measure.
                Some(&format!("c{calls}")),
            ))
        });
        assert_eq!(calls, MAX_HISTORY_PAGES);
        assert!(events.is_empty());
    }

    /// ★★If the connection ended while fetching history, the gate is not opened★★ — the history side treats that error as 「a page did not
    /// come」 (which is right), so if the gate did not look at the state, **an already dead link would stand as `Ready` and
    /// be delivered as a successful activation.** After that the manager carries a corpse as a live agent.
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

    /// ★★The stream end **writes the state first and then wakes**★★ — the other way, the writer would come back through a window the death itself opened,
    /// read the still `Connecting` state, pass the gate, and **deliver `Ready` to an already ended link.**
    /// That window was not by chance; it opened on every stream end.
    ///
    /// ★★Why measure by **source order** — it was measured by running, and cannot be measured★★: a version that wakes first was built with an observer
    /// thread attached and run 200 times, and **it was never caught.** The woken side has to wait for the scheduler, while
    /// the waking side takes the lock within the next few lines, so it is a race but practically the waking side always wins. That is, that
    /// harness was **green even with the wrong order**, so it was not a regression net. As long as the subject is 「the order of two statements」,
    /// measuring that directly is honest (sibling = [`tests::the_history_is_emitted_before_the_gate_opens`]).
    /// ★So the share measured by running is carried by **the sibling item below**★ — not order but 「did both happen」.
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

    /// Measures by running not the order but **whether both happened** — removing either one gets caught here
    /// (removing the waking is a permanent hang, removing the state write leaves a dead link as alive).
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

    /// The same when **only the closed marker** stands, not the delivery channel — only the reason wording is missing; the conclusion is the same.
    #[test]
    fn a_gate_does_not_open_over_a_closed_link() {
        let state = shared();
        with_state(&state, |s| s.closed = true);
        assert!(open_gate(&state, "th".to_string()).is_err());
        with_state(&state, |s| {
            assert!(!matches!(s.link, Link::Ready));
        });
    }

    /// On a sound connection it opens as is and the handle is carried — the opposite side of the two above.
    #[test]
    fn a_healthy_link_opens_the_gate_and_carries_the_thread_id() {
        let state = shared();
        assert_eq!(open_gate(&state, "th-9".to_string()), Ok(()));
        with_state(&state, |s| {
            assert!(matches!(s.link, Link::Ready));
            assert_eq!(s.thread_id.as_deref(), Some("th-9"));
        });
    }

    /// ★If one round trip uses up the budget, **the next is not asked**★ — this is where this loop actually enforces the shared budget.
    /// The overrun **inside** one request (one response-wait slice, 500ms) cannot be blocked by this loop.
    ///
    /// ★★This spot once said 「10s + 0.5s」, and that figure was **wrong**★★ — that calculation assumed only one overrun,
    /// but the unit that counts overruns is not the loop but the [`request_blocking`] **call**: the success branch does not look at the deadline
    /// again, and even with a budget of 0 it waits out the whole next slice. So the two round trips of [`handshake`] can each
    /// exceed by one slice (if the first round trip succeeds during its own overrun, the second starts with a remainder of 0
    /// and uses one more slice), and **the cap until `Ready` is 10s + 2×0.5s = 11s**.
    /// ★Why it does not grow further after that is what this item measures★ — the first check of paging sees a remainder of 0 and cuts, so
    /// there is no third overrun. ★Adding one more round trip to the handshake grows it by that much★ (11s → 11.5s).
    /// The 15s backstop stays above it as is.
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

    /// ★If the budget is already gone, **it never asks**★ — that is the case where the handshake used up the budget and barely succeeded,
    /// and if even one more round trip went out there, `Ready` would be pushed behind the manager's backstop.
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

    /// ★If there is nothing to resume, there is no round trip at all★ — an incarnation that came up with `thread/start` (new conversation) falls here.
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

    /// ★★History must go into the ring **before** the gate opens — this is the order contract of this path★★:
    /// if the gate opened first, queued user turns would go out, and their responses would go into the ring before the history not yet carried,
    /// and the restored conversation would attach **after** the new answer. This order carries in place what the seed-before-publish of ADR-0079
    /// kept (it cannot be pulled ahead of registry registration as that one was — because the history exists only after the
    /// handshake).
    ///
    /// ★The reason for measuring in the source is the same as the sibling item★ — measuring by actually running needs a child process and a peer to
    /// round-trip with, and what that harness measures becomes not the order but the harness itself.
    /// Sibling = [`tests::the_session_id_is_recorded_before_the_gate_opens`].
    #[test]
    fn the_history_is_emitted_before_the_gate_opens() {
        let body = writer_loop_code();

        let hydrated = body.find("hydrate_history(").expect(
            "there is no `hydrate_history(` call in writer_loop — this item's premise is stale",
        );
        // The reason for anchoring on `open_gate(` = the same spot of the sibling item.
        let gate_open = body
            .find("open_gate(")
            .expect("there is no `open_gate(` call in writer_loop — this item's premise is stale");
        assert!(
            hydrated < gate_open,
            "the history restore stands after the gate — the restored conversation attaches after the new answer"
        );

        // ★★The entry door must be **the one that does fanout too**★★: at this spot the session is already in the registry,
        //   so with `OutputCore::seed` (puts into the ring only, no fanout), a subscriber that attached in between has already replayed the empty ring
        //   and **never sees** the past screen. `seed` was enough for ADR-0079 because that one was **before**
        //   registry registration, and this path cannot be pulled to that place.
        // ★What is required is **the chunk door**★ — switching to the item-by-item door keeps fanout, but between calls
        //   the reader's live lines take seqs and the middle of the restored conversation is split
        //   ([`crate::output_core::OutputCore::emit_batch_without_turn_observation`]).
        assert!(
            body.contains("emit_batch_without_turn_observation"),
            "history goes in through the item-by-item door — fanout works but the middle of the chunk is split"
        );
        assert!(
            !body.contains("core.seed("),
            "`seed` loses fanout at this spot (the doc above)"
        );
    }

    /// ★★The handshake and the history **share one budget**★★ — if each took its own, the time until the connection stands would be up to two
    /// budgets, exceeding the manager's backstop, and **a resume that succeeded while fetching history would be decided as a failure.**
    /// The concrete form of that contract is 「`writer_loop` starts the clock only once and passes the same value to both」.
    #[test]
    fn the_handshake_and_the_history_share_one_budget() {
        let code = writer_loop_code();

        // ★Measured not by call shape but by **the number of appearances of the name**★ — line breaks in the argument list are decided by `cargo fmt`,
        //   so comparing whole call strings would break this item on a mere formatting change (it actually broke).
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

    /// ★If the user turned it off, **the connection outcome is not delivered**★ — the root of defect ③.
    ///
    /// kill closes the waiting table and wakes the handshake with `Err`. Delivering that as is would **record the user's cancellation as a
    /// resume failure** and run cleanup once more on top of that. On that branch the only fact the supervisor sees must be the end state,
    /// and there the discipline 「what the user turned off is not a failure」 already exists.
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

    /// ★The link's handshake cap must be **smaller** than the manager's liveness backstop★.
    ///
    /// The two do different jobs: the cap **makes a reason** (if the peer does not answer, it writes that fact into `Down`),
    /// the backstop **gives up without a reason** (when the link cannot reach an outcome at all — a hanging write). If the backstop were smaller,
    /// the outcome with a reason would be covered by the give-up without one, and no cause would remain on screen or in 「last failure」.
    /// ★This relation exists only between two constants scattered across two files, so the compiler cannot see it★ — adjusting only one side breaks
    /// nothing and only revives the defect, so it is measured here.
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
    /// Why a count = the grounds that the handshake failure branch of [`writer_loop`] taking that lock is safe are 「in the production
    /// graph only those two take this lock blocking, and of them [`write_line`] is called only by the writer」.
    /// If a third quietly appears, those grounds go stale silently — and what happens then is not a compile error but
    /// **a deadlock**.
    ///
    /// ★This item does not pin places, only the count★ — pinning positions goes stale every time lines shift. If it grew,
    /// fix this number **after deciding directly** on which thread the new place is called (do not just raise the number).
    /// The counting rule is held by [`count_blocking_stdin_locks`]. ★Comment lines are not counted★ — this file quotes
    /// `stdin.lock()` in prose (comments attached at line ends are not filtered and can over-count — that direction is red, so it is fine).
    /// ★The test section is out of range★ — that side has an item that holds the lock on purpose
    /// ([`tests::shutdown_completes_even_if_a_write_blocks_on_a_full_pipe`]). That item is harmless because
    /// it does not call [`AgentTransport::start`], so the writer thread itself does not come up, and the moment `start` is added
    /// the harmlessness breaks.
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

    /// Counts blocking stdin lock acquisition places — counts two forms together.
    ///
    /// ① A place where `.lock()` comes after `stdin`, skipping whitespace (including newlines).
    /// ② A place where `stdin` is in the argument of a `lock(` whose preceding character is not an identifier character (`_` · alphanumeric) — catches `sync::lock(` ·
    ///    full paths · a bare `lock(` after `use` · `Mutex::lock(` all. Argument = up to the matching parenthesis, and if it does not
    ///    match, that line and the next two lines. Today's two production places are of form ② (ADR-0275).
    ///
    /// Not counted = a `lock(` attached to an identifier like `try_lock(` (it does not wait, so it is not in this risk —
    /// the spot in [`AgentTransport::shutdown`]) · `.lock()` with an empty argument (that is ①'s share, so it is not counted twice).
    /// ★Do not count only one form★ — a third place written in the other form would not be caught by the count and pass green. Going back to ① is
    /// blocking too. ★Leaning toward over-counting is intended★ — unrelated arguments with `stdin` in the name like `stdin_buf`
    /// are counted too. Over-counting calls for a decision with red, but under-counting lets a third place pass green.
    /// ★Forms it cannot catch remain★ — method chains whose receiver does not end in `stdin` (`stdin.as_ref().lock()` ·
    /// `(*stdin).lock()`) hit neither ① nor ②.
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

    /// `rest` = right after `lock(`. Returns up to before the matching closing parenthesis, and if there is no match, returns that line and the next two lines.
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

    /// Measures whether [`count_blocking_stdin_locks`] counts each of the false-pass forms reproduced in review.
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

    /// List events (ring order). ★The handed-over mark (`HandedOver`) is excluded★ — it is a row mark, not an outcome, so it is measured separately ([`handed`]).
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

    /// (uuid, body) of user bubbles (synthetic · echoed).
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

    /// One echoed user message line — `clientId` is our id.
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

    /// ★Our edition in the trailing parenthesis is not read · unknown means below the floor★ — carrying `clientUserMessageId` on an unknown edition quietly breaks the echo
    /// comparison.
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

    /// ★Pre-decision items are not announced, and the side the decision lands on emits them all at once★ — putting them up in advance would create list items
    /// that meet a below-floor decision. Items during connecting are not synthetic bubbles (Direct stands only after `Ready`).
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

    /// ★Below the floor = today's path★ (user decision N1 (a)) — 0 events · no identifier · the `turn/start` bytes are the same as before this field
    /// existed · it leaves at the moment of opening.
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

    /// `clientUserMessageId` stands only on items carrying an identifier — the bytes of items without an identifier are as today and
    /// they leave at the moment of opening, while items carrying one remain `InFlight` waiting for the echo. Mail carries only the id without an announcement. The opening order is the Idle
    /// order — user text goes ahead of mail typed earlier (including items without an identifier) (N6).
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

    /// ★Two inputs at the same idle moment → the first Direct · the second Queued★ — Direct also goes into pending, so the second cannot see
    /// 「empty」. With the Direct id, `Queued` goes out on no branch, and one `turn/start` is one item.
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

    /// ★After the link's synthetic bubble the turn table stays idle★ (ADR-0127) — emitted through the ordinary door, the classifier would write it as progress,
    /// and if that `turn/start` were refused, a turn that never opened would remain 「in progress」 and mail would be blocked.
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

    /// ★The writer does not skip a pre-announce head★ — if the handover outran the announcement, the receipt would be dropped with an id the registry does not know,
    /// and skipping would break typing order.
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

    /// A success response writes the turn id onto the item it carried and only releases the response wait (it is not receipt). ★Generation masking★ — a late response
    /// of a turn with a different marker does not touch that item.
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

    /// ★Receipt is right before the bubble, and it is removed from pending whatever stage it came in (receipt wins)★ — if it is a held item,
    /// it does not go out again. There is no open turn, so a turn boundary stands after the receipt ([`Reader::note_delivered`]).
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

    /// A receipt of an id the link does not know (mail · already resolved) does not touch pending — the event flows as is (the registry's
    /// tombstone takes it).
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

    /// ✕ on `Held` → `Withdrawn` · `Dropped{Withdrawn}` · 0 vendor requests. An overlapping second ✕ is `NotHeld`.
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

    /// ★✕ on a pre-announce item → ring order `Queued` → `Dropped`★ — only the death mark is attached, and the announcing side follows right after
    /// `Queued`.
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

    /// Outside the list (Direct · mail · pre-decision) and unknown ids are `NotHeld` with no event. A handed-over list item (here, one
    /// `turn/start` carried) cannot be withdrawn — `TooLate` with the cancellation receipt · 「could not remove」 (ADR-0231 · TRD §5-5 「✕」).
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

    /// A connection where the decision (`Above`) landed — list · synthetic bubble · echo comparison stand.
    fn above(h: &Harness) -> Arc<Announcer> {
        make_ready(&h.state, "T");
        let ann = announcer();
        settle_floor(&h.reader.core, &h.state, &ann, Floor::Above);
        ann
    }

    /// Opens a turn with the oldest `Held` and registers that `turn/start`'s waiting entry — exactly as the writer does.
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

    /// List events · bubbles · turn boundaries in ring order — also checks whether disposal events come before the boundary. ★The handed-over mark (`HandedOver`) is
    /// excluded★ — it is a row mark, not an outcome, so it is measured separately ([`handed`]).
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

    /// Only the handed-over marks in ring order — `sent <id>` · `held <id>`.
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
        /// Text typed mid-turn — it goes into the list.
        Listed,
        /// Text typed while idle — synthetic bubble · outside the list.
        Direct,
        Mail,
    }

    #[derive(Debug, Clone, Copy)]
    enum End {
        /// `turn/completed(status)` after the response.
        Completed(&'static str),
        /// `turn/completed(status)` **before** the response — it ends where the response releases it.
        Early(&'static str),
        ErrorReply,
        WriteFailed,
        Deadline,
        Unreadable,
        LongTurnId,
    }

    /// Ends a turn that handed over one item with `end` and returns the traces after it (from opening the turn on). ★At every end the turn must be
    /// closed and pending must have no `InFlight`★ (TRD §5-5 invariant).
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

    /// Ends the turn of marker `seq` (its `turn/start` request `request`) with `end` — exactly as the writer · reader do.
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

    /// An item outside the list left without an event — pending is empty, so the next idle input becomes Direct again.
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

    /// The handed-over user text `id` returned to the held place — 0 events (list row · drawn bubble as is) · no halt · it rides the next turn
    /// **once**.
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

    /// The handed-over user text `id` returns to the held place after a failure end and waits behind the halt — 0 events (list row · drawn bubble as is) ·
    /// 0 turns while halted · when the next user text lifts the halt, the returned text rides the next turn **once**, before that text (what remains goes first).
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

    /// ★User text (list · Direct) of a turn that ended without an echo returns to the held place regardless of the shape of the end★ (user decision 2026-09-26 · ADR-0235
    /// — replaces 「erase on failure」). Ends that spoke an outcome (`completed` · a status of unknown meaning) and the interruption let it ride the next turn once right away
    /// (no halt), and failure ends (`failed` · write failure of `turn/start` · error response · deadline · decode failure · long turn id) set the halt
    /// and it waits until the next user text — neither side has an event (list row · drawn bubble as is). After the response · before the response (early-end release)
    /// are all the same. Mail leaves without an event.
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

    /// One `held_again` trace line per text returned to the held place — it carries the shape of that end and (if known) the id of the turn it was handed to. Failure ends also
    /// return to the held place, so the line stands — text of a `turn/start` refused before receiving the turn id has no `turn_id`.
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

    /// ★An interruption returns the list items handed over in that turn to the held place and leaves held items as is★ (user decision 2026-09-26 — interruption = held
    /// text goes right away into a new turn) — pre-announce items also only get `Queued` and are not dropped. Held mail · items without an identifier also
    /// stay as is and no halt stands, and the next Idle opens from the oldest user text (the returned text) in typing order.
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

    /// If the echo came before the turn end, there is nothing to dispose — receipt wins.
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

    /// ★If the old request's echo comes late for text returned to the held place, it is receipt — it does not go out again★ (both outcome-speaking ends · interruption).
    /// There is no open turn, so a boundary stands after the receipt.
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

    /// If the old request's echo comes after it was already handed over again, that is the receipt, and the new request's echo passes as an unknown id — the list
    /// closes once and that text does not go out a third time.
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

    /// While returned to the held place, the kernel's 「pending input exists」 is true (it stands in the registry) — false once the echo closes it.
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

    /// The link does not dispose on stream close — list items (handed over · held · returned to the held place) are closed **once each** by the core's end synthesis
    /// as `Dropped{AgentEnded}`.
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

    /// The floor decision is written **after** the record returns and **before** the history — brought forward, the record failure branch would erase items already in the list
    /// without an event, and pushed after the history, the window in which the list comes up late would grow by the history paging.
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

    /// The `Arc` the assembly point carries on the session is **the very one** the decision fills — a separately made one would have the session and the link see different
    /// values.
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

    /// `shutdown` completes even while a blocking write holds the stdin lock — flipping the order (trying to close stdin before
    /// kill) would make this item catch it with a timeout.
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

    /// One item notification line — turn `turn` of our thread (`T`).
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

    /// The steer the writer picks right where our turn's tool just ended — sets the tool-end trigger like [`Reader::note_tool_item`] and
    /// picks it (`None` = nothing to hand over). It does not go through the sweep wait of [`next_job`].
    fn steer_at_tool_end(h: &Harness) -> Option<Steer> {
        with_state(&h.state, |s| {
            s.steer_due = true;
            take_steer_locked(s, &h.next_id)
        })
    }

    /// The steer the writer would pick now — it does not set the trigger and uses only what was standing (`None` = the writer sleeps).
    fn steer_next(h: &Harness) -> Option<Steer> {
        with_state(&h.state, |s| take_steer_locked(s, &h.next_id))
    }

    fn steer_due(state: &SharedState) -> bool {
        with_state(state, |s| s.steer_due)
    }

    /// The next job the writer picks must be a steer — if the trigger stands, [`next_job`] returns right away without waiting.
    fn next_steer_job(h: &Harness, why: &str) -> Steer {
        match next_job(&h.state, &h.next_id) {
            Some(Job::Steer(s)) => s,
            _ => panic!("{why}"),
        }
    }

    /// ★Only the end of a tool item of our current turn is a trigger★ — the end of a non-tool item (output · echo · a known variant with no grounds to be a tool · an unknown
    /// variant) · the start of a tool · another turn id (late completion of a tool of an interrupted turn — M6) · a tool end of another thread hand over nothing (held
    /// text stays held). Only when a tool end of the same turn comes is it handed over.
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

    /// ★A tool end before the turn id is known is not a trigger★ — without `expectedTurnId` there is no means to hand over. Even a line that came with that turn's id
    /// is ignored before the response gives that id, and the held text is handed over where the response gives the turn id ([`Reader::resolve`]).
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

    /// A tool end wakes the waiting writer right away — without the wake-up, the writer sees the trigger only after the sweep interval ([`SWEEP_INTERVAL`])
    /// runs out (the tool end is fed after the writer fell asleep).
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
        // Room for the writer to see the held text and fall asleep.
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

    /// One [`STEER_TRACE`] line — field name → value (as a string).
    type TraceLine = std::collections::BTreeMap<String, String>;

    /// A subscriber that collects only this thread's trace lines — field names are the contract with QA, so it measures whether they go out as documented.
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

    /// The fields of trace lines follow the doc ([`STEER_TRACE`]) — one line per event, every line carries `agent` · `phase` · `item_id` · (if that turn is
    /// known) `turn_id`, `tool_end` also carries `held`, `steer` also carries `request`. The message is the one line of that kind.
    /// Non-tool items have no line.
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
                // Exactly as the writer does — written after finishing the write ([`writer_loop`]).
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

    /// A steer write failure leaves one `write_failed` trace line apart from the warn of the default target — QA reading only the trace sees that the text
    /// did not reach the vendor. The fields are the same as the `steer` line (the turn id it was handed to · that item · request id).
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
    // On a connection where the decision (`Above`) landed, it steers exactly as production — this section is measured with the [`above`] test bench. The trigger to hand over is the tool end
    // ([`steer_at_tool_end`] — measures at the spot where the trigger was set what is handed over and what is not).

    /// The params of that steer — also looks at the envelope's method and request id.
    fn steer_params(steer: &Steer) -> Value {
        let v: Value = serde_json::from_str(&steer.line).unwrap();
        assert_eq!(v["method"], "turn/steer");
        assert_eq!(v["id"], steer.request);
        v["params"].clone()
    }

    /// Registers the waiting entry and writes exactly as the writer does (the write succeeds).
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

    /// The spot where a turn was opened with `head`, text typed while idle, and that `turn/start` response gave the turn id `turn` — returns that turn's marker.
    /// The turn id signal the response set is left unpicked. `head` is already received (the vendor echoes the text `turn/start` carried as the first item of
    /// that turn) — if not received, the turn end would return that text to the held place and it would become the head of the next turn. The [`RecordingDecoder`]
    /// test bench cannot make a receipt from an echo line, so it calls the link's receipt verb directly.
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
    /// gives the turn id, the text held meanwhile is handed over one at a time from the head without waiting for a tool end (`expectedTurnId` ·
    /// `clientUserMessageId`), and the writer hands over the following items in turn (ADR-0235). There are no list events.
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

    /// A pre-announce head at a tool end is not skipped — it waits with the trigger left standing, and when the announcement wakes the writer, that text is handed over. Held mail
    /// is neither handed over nor blocking, and Direct does not go out as a steer (the trigger is lowered).
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

    /// A steer's success response and deadline expiry do not move the item — it is not receipt (the echo is receipt), and it stays in the list at the handed-over stage
    /// · no list event and no turn end.
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

    /// steer error response → that item returns to `Held` (0 list events) and that turn is 「steer refused」 — even when a tool end comes, both the returned
    /// text and newly arrived text are held. When the turn ends, the returned text opens the next `turn/start` in typing order, and when that turn id comes it is handed over
    /// again.
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

    /// If the steer of a handed-over item ✕ reached is refused, it is withdrawn instead of held again (`Dropped{Withdrawn}`) — the vendor never saw that text
    /// and the user already removed it. That turn's 「steer refused」 mark stands as is.
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

    /// The write-failure version of refusal — if a steer ✕ reached after being picked cannot be written, it is withdrawn instead of held again (`Dropped{Withdrawn}` — it did not
    /// reach the vendor and the user already removed it). It could not be written, so there is no handed-over mark, and that turn's 「steer refused」 mark stands.
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

    /// ★Generation masking★ — a late steer response for an item already removed by echo moves nothing (it does not hold it again · does not attach
    /// the mark). A response of a waiting entry with another generation cannot touch that item either.
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

    /// A steer waiting entry registered when the stream closes is taken away without hanging, and the link does not touch that item — the core's
    /// end synthesis closes it once as `Dropped{AgentEnded}`.
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

    /// ✕ on a handed-over item → `TooLate` · `CancelRequested` followed right away by `CancelAnswered{false}` · not sent again. An overlapping
    /// second ✕ is `TooLate` without an event. If the echo comes after that, it is receipt (it leaves the list, and a bubble).
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

    /// (id, handed-over mark) of registry rows — the value the list query · reattach read.
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

    /// Writing a steer sets the handed-over mark on that row (it stays in the list — receipt is only the echo) · when it is refused and returns to the held place, the mark
    /// is lowered (row as is · no outcome event). The list query carries the handed-over row as `sent`.
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

    /// If the turn of a handed-over list item ends without an echo — regardless of the shape of the end — the mark is lowered (`held`). Items outside the list (Direct · mail)
    /// emit no mark. `turn/start` emits the mark after the writer writes, so it is done the same way here.
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

        // The mark emitted after the writer wrote `turn/start` → an end without an echo lowers it.
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

    /// The mark target `turn/start` carries — list items only (`None` for Direct · mail · items without an identifier below the floor). The mark value is read not by the
    /// caller but from the current stage — nothing is emitted for items already gone · items outside the list.
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

    /// ✕ overlapped the handover (✕ after picking to hand over, before writing) — it answers that it could not remove it (`TooLate`), and the row stays in the list and becomes
    /// 「sent」 by the mark after the write. When the echo comes, it leaves as received.
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

    /// A subscriber that flips that item's stage once the moment it receives the handed-over mark `sent` — imitates the spot where, between [`publish_hand_over`] reading and emitting the value,
    /// another thread's transition (the reader's turn end · the writer picking it for the next turn) took the state lock. The emitting side lets go of the state lock
    /// before emitting, so taking it here is fine.
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

    /// The state word the list query carries for that row.
    fn listed_state(h: &Harness, id: &str) -> String {
        let (rows, _) = h.reader.core.queued_inputs().snapshot();
        crate::queued_input::overlay_unconfirmed(rows, &[])
            .into_iter()
            .find(|r| r.id == id)
            .map(|r| r.state.as_str().to_string())
            .expect("not in the list")
    }

    /// ★If the turn closes between reading and emitting the mark and it returns to the held place, the last mark is `held`★ — the spot where the writer read `sent` after writing the steer
    /// but before emitting, the reader returned that text with a turn end without an echo. Without reading again, the stale `sent` would remain as the last mark of the ring
    /// and the held text would be drawn as 「sent」 without ✕.
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

    /// Mirror — ★if the writer opens the next turn with that text between reading and emitting the mark, the last mark is `sent`★. The spot where the reader read `held`
    /// after returning it with a turn end without an echo, but before emitting the writer picked that text with `turn/start`. Without reading again, the handed-over text would be drawn as
    /// 「waiting」 with ✕ standing again.
    #[test]
    fn a_held_mark_overtaken_by_the_next_turn_start_is_followed_by_sent() {
        let mut h = harness();
        let ann = above(&h);
        running_turn(&mut h, &ann, "t1");
        accept_user(&h.state, "q-1", "x");
        announce(&h.reader.core, &h.state, &ann);
        let s = steer_at_tool_end(&h).expect("steer");
        assert!(matches!(issue(&h, s), Issued::Written));
        // The spot where the next turn's (marker 1) `turn/start` carried it — the mark does not look at the generation, so the value only matches the shape.
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

    /// ✕ on an item held while waiting for the turn id withdraws the text — even if the turn id comes after that, the text is not handed over (the handover and ✕
    /// get their order under the same state lock — the opposite order is ✕ on a handed-over item).
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

    /// ★One tool end hands over all held user text in typing order★ — one `turn/steer` per text · its own `clientUserMessageId` ·
    /// the current turn id. Mail is neither handed over nor blocking (held until the turn end). Handed-over text stays in the list (receipt is only the echo — the only list event is
    /// `Queued`) and once everything is handed over the trigger is lowered.
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

    /// ✕ while a tool runs withdraws the held text (`Withdrawn` · `Dropped{Withdrawn}` · 0 vendor requests) — even when the tool end comes there is nothing to hand over,
    /// so the trigger is lowered right away. An overlapping second ✕ is an id the link does not know.
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

    /// ★Without a tool end, it is held until the turn end★ — text typed **after** the tool ended is not handed over if no tool follows (it only writes the answer and ends),
    /// and remains with 0 list events. When the turn ends, the oldest text opens the next turn with `turn/start`, and the moment that turn id comes
    /// the rest goes right into that turn (it does not wait for a tool end — the remaining text gathers into one turn).
    #[test]
    fn text_typed_with_no_tool_afterwards_waits_for_the_turn_end_and_the_rest_ride_the_next_turn_once_its_id_is_known(
    ) {
        let mut h = harness();
        let ann = above(&h);
        running_turn(&mut h, &ann, "t1");
        item_started(&mut h, "t1", "commandExecution", "c");
        item_completed(&mut h, "t1", "commandExecution", "c");
        // Exactly as the writer does — a tool end with no held text lowers the trigger right away.
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

    /// ★The tool-end trigger is lowered with the turn★ — if the turn ends before the writer sees the trigger set by a tool end with no held text (during a blocking write)
    /// and the writer opens from the next turn, text typed after that turn id came with no held text waits for that turn's tool end — it is not handed over
    /// by the previous turn's trigger (if it were, it would go in without a tool end, and could only be recorded without getting an answer).
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

    /// ★The tool-end fixture (M7 trial 1 — measured codex-cli 0.156.1)★: text typed while a tool runs is held and handed over at the tool's `item/completed`
    /// (request id 3 of the fixture), and the vendor echoes that text in the same turn and answers — receipt (`Delivered`) · 0 waiting entries.
    /// The request ids are matched to the fixture's response ids (`fixtures/README.md`).
    #[test]
    fn the_tool_end_fixture_holds_text_typed_during_the_tool_and_delivers_it_in_that_turn() {
        const THREAD: &str = "01a0d4b3-6048-7b41-aa4b-eec199dd1c40";
        const TURN: &str = "01a0d4b3-60f6-78e1-80fb-bf1ac3ecd0c7";
        const CARRIER: &str = "e53bb68a-aea6-421e-b04f-45caea0fc6f1";
        const TYPED: &str = "d05ef2f8-fa23-4c56-b379-b5aba4dc39e1";
        let lines: Vec<&str> = include_str!("fixtures/tool_end_m7.jsonl").lines().collect();
        // 1-based line numbers (the same counting as the README table).
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

    /// ★If the steer cannot be written, that text returns to the held place★ (user decision 2026-09-26 · ADR-0235 — not erased) · 0 list events · the waiting entry is
    /// taken away and the turn keeps running (it is not a trigger of the halt after error). That turn gets the 「steer refused」 mark, so the next tool end of the same turn does not
    /// write that text again right away (no retry loop), and after the turn ends it goes out **once** with `turn/start` at the next Idle.
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

    /// ★Production steers at the tool end★ — in a state built without test seams (floor decision `Above`), text typed in the running turn is held, and when our turn's
    /// tool-end line passes the reader, the next job the writer picks ([`next_job`]) is that text's `turn/steer`. A below-floor incarnation is today's path
    /// as is — even when the turn id comes · the tool ends, it stays held and goes out with `turn/start` after the turn end.
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
    // ★Plugged in the same shape as production★: the handshake's record port = latch receipt, the reader's first-turn port = the same latch.
    //   This mode's session does not count submissions, so commit happens only at the echo.

    /// Real translator test bench + latch — the spot after finishing what the handshake does (handing the thread id to the latch and becoming ready).
    /// The returned record = the values the commit port received.
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

    /// One echoed user message line — without `clientId` (below the floor · mail turn without an identifier), choosing the thread.
    fn anonymous_echo_line(method_name: &str, thread: &str, turn: &str, item_id: &str) -> String {
        serde_json::json!({
            "method": method_name,
            "params": {"threadId": thread, "turnId": turn,
                       "item": {"type": "userMessage", "id": item_id,
                                "content": [{"type": "text", "text": "letter"}]}},
        })
        .to_string()
    }

    /// (a)·(b) — nothing is persisted by the handshake and sending, and it is persisted exactly once at the peer's first echo.
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

    /// (e) — an echo without `clientId` counts too: every turn of a below-floor incarnation · mail turns without an identifier.
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

    /// (d)·(f) — lines that are not a live user message echo open no persistence: an echo that declares another
    ///   thread · an agent answer item · a turn end (a turn that ended before the echo) · a **response** line carrying a user message (the door history restore
    ///   rides).
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

    /// A reader without a first-turn port (the assembly point did not give one) also handles echoes as usual — there is just nowhere to call.
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

    /// The first-turn port is called outside the marking — the reader runs inside the pump's containment, but the latch commit changes the shared profile table.
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
    // The turn end does not wait for the steer response — the test bench where the decision (`Above`) landed ([`above`]) steers exactly as production.

    /// The spot where one `id` was handed to the running turn as a steer and the waiting entry was registered — that steer's request id.
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

    /// Does `act` wake the side sleeping on the state condition variable? ★The sleeping side signals readiness while holding the lock and lets go of the lock as it falls asleep★ — so
    /// the moment `act` gets the lock, the sleeping side is already asleep (there is no window to miss the wake-up).
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

    /// ★The window between the turn end and the steer response★ — whether −32600 (no turn running) comes after or before `turn/completed`, that text remains `Held`
    /// and goes out **exactly once** with `turn/start` (M10 — no order is assumed) · 0 list events · the turn end does not wait for that response
    /// · a refusal of the old request that came after it went out again is a no-op (generation masking). The `failed` end also holds the same text — text returned by a mid-turn refusal
    /// stays as is, and text before the refusal came is returned by the end (user decision 2026-09-26 · ADR-0235). That `turn/start` is after the text typed after the halt
    /// arrived (halt after error).
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
                // Halt after error — text held from before the halt does not go out on its own, and goes in as the head of the turn of the text typed after the halt (AC24).
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

    /// Text of a steer that got no response until the turn end returns to the held place at the turn end (0 list events · stays in the list) and rides the next `turn/start`
    /// once right away without waiting for that response. The old request's success response · deadline · refusal that came after it went out again do not move that text
    /// (generation masking).
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

    /// Text handed over as a steer in an interrupted turn also returns to the held place if there is no echo and rides the next turn once right away — the interruption does not
    /// set the halt. It is the same whether the response was already received or not, and a late response of the old request does not move the text handed over again.
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

    /// If the turn of a steer's text ✕ reached ends without an echo, it does not return to the held place but is withdrawn at that turn end (`Dropped{Withdrawn}` —
    /// returned, it would go out in the next turn). The responses after that (refusal · success) emit no more events.
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

    /// Two steer texts returned to the held place at the turn end go out in typing order without waiting for responses — the earlier typed one with `turn/start`,
    /// the later one as a steer at the wake-up where that turn id comes. Late refusals of the old requests move neither text and attach no mark.
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

    /// Even if a steer whose turn ended before the write cannot be written, text already returned to the held place at the turn end is not touched — it is not closed as a refusal (generation masking),
    /// the 「steer refused」 mark is not attached either, and it rides the next turn once.
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

    /// ★A receipt that came after the turn end★ — if the echo of a steer that got no response comes after `turn/completed` (that text has returned to the held place),
    /// it is closed as received (it leaves the list, and the vendor bubble) · 0 empty `turn/start`. There is no open turn, so **a turn boundary after the receipt**
    /// stands (it turns off the frontend's waiting indicator) — it is the observation-free door, so the turn table stays as is. A late success response emits nothing.
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

    /// ★If the echo came before the steer response, the turn end does not wait for that response★ — the receipt removes the item, so there is no text to dispose at the turn end,
    /// and the next user text's `turn/start` goes out right away before the response. A late success response emits nothing.
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

    /// If an error response comes to an echoed steer, the item is not revived (receipt wins) · 0 list events · the 「steer refused」 mark is
    /// not attached — whether the turn is running or has ended.
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

    /// After a late receipt that came inside an open turn (an old echo of text that returned to the held place in the previous turn and was handed over again in this turn), no boundary
    /// is set — that turn's end is the boundary.
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

    /// A late receipt of text returned to the held place wakes the writer the moment it leaves pending.
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

    /// ✕ on text returned to the held place is the same as on held text — `Withdrawn` · `Dropped{Withdrawn}` right away (no cancellation wait) · it leaves pending
    /// and wakes the writer · the registry empties · it does not go out again. An overlapping second ✕ is `NotHeld`.
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

    /// If the turn of handed-over text ✕ reached (text `turn/start` carried) ends without an echo, it does not return to the held place regardless of the shape of the end but
    /// is withdrawn (`Dropped{Withdrawn}`) — no cancellation wait remains in the registry to block mail, and it does not go out in the next turn.
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

    /// The turn the writer opens now — registers the waiting entry too (exactly as the writer does). (request id, marker, params).
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

    /// The spot where turn `turn` was opened with text typed while idle, that text was echoed and the model wrote and finished an answer — that turn's marker (accepted head ·
    /// the common setup of tests that use the spot after the translator saw that turn's lines).
    fn answered_turn(h: &mut Harness, ann: &Announcer, turn: &str) -> u64 {
        let seq = running_turn(h, ann, turn);
        h.reader
            .handle_line(echo_line(turn, "head", "first").as_bytes());
        item_started(h, turn, "agentMessage", "a-head");
        item_completed(h, turn, "agentMessage", "a-head");
        seq
    }

    /// The spot where `id` was handed to the running turn `turn` as a steer, got a success response and was echoed — the sampling start after it has not come yet.
    fn steer_recorded(h: &mut Harness, ann: &Announcer, turn: &str, id: &str, text: &str) {
        let request = steer_one(h, ann, id, text);
        steer_accepted_reply(h, request, turn);
        h.reader.handle_line(echo_line(turn, id, text).as_bytes());
    }

    /// ★Idle order = user first → mail★ (PRD §3-9 · N6) — when held mail and user text are together, the user turn comes before mail typed earlier,
    /// and the mail opens after that turn ends. ★Even if an accepted steer ends without getting an answer, no empty `turn/start` goes out★ (the follow-up
    /// turn debt was removed — ADR-0235).
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

    /// At Idle `[U1, M, U2]` (M = mail) → U1's `turn/start` · U2 steered right away on the new turn id · M gets `turn/start` only at the Idle after the user `Held`
    /// are empty (TRD §7-1 · AC22 · N6).
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

    /// ★User text returned to the held place goes out before mail held earlier★ (Idle order = user first — N6) — the mail opens at the Idle after that text is
    /// accepted.
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
        /// A user turn opened by a list item typed mid-turn as `Held`.
        Listed,
        /// A turn opened by text typed while idle (synthetic bubble).
        Direct,
        Mail,
    }

    /// The six triggers (`failed` — including the early-end release · the five `turn/start` exits).
    const HALTING_ENDS: [End; 7] = [
        End::Completed("failed"),
        End::Early("failed"),
        End::ErrorReply,
        End::WriteFailed,
        End::Deadline,
        End::Unreadable,
        End::LongTurnId,
    ];

    /// The spot where one turn of `kind` is opened (including the waiting entry) and user text `pre` and mail `m-pre` are held during that turn — (request id, marker).
    /// `pre` comes in while waiting for the turn id and stays held (the test bench does not run the writer).
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

    /// ★Six triggers × three kinds — either way the halt stands★ · the user text that turn carried is not erased but returned to the held place (user
    /// decision 2026-09-26 B — mail leaves as today) · 0 turns opened by Idle while halted (returned text · user text held from before the halt ·
    /// held mail) · the moment the next user text comes in, the halt lifts and a `turn/start` with the remaining text as head opens, and the following text goes into that turn id
    /// as a steer — what remains goes first, in typing order (AC24).
    #[test]
    fn every_abnormal_end_halts_whatever_the_turn_kind_until_the_next_user_item() {
        for kind in [Kind::Listed, Kind::Direct, Kind::Mail] {
            // The head of the turn opened after the halt lifts and the next text going into that turn as a steer — the user text that turn carried returns to the held place and is the head.
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

    /// ★The halt is not lifted by mail — it is lifted the moment user text comes in★ · it stays even if all held text is removed by ✕ and the list empties (0 held
    /// mail). The Idle order after lifting is as usual — when the user turn ends, the held mail goes out.
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

    /// ★An interruption (`interrupted`) does not set the halt★ — the user text the interrupted turn carried (it returns to the held place) and the user text held during that turn
    /// are not dropped and go out right away with `turn/start` from the next Idle in typing order, and the held mail goes out after those user turns
    /// end (user decision 2026-09-26 — interruption = held text goes right away into a new turn).
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

    /// ★A failed turn holds until the next user text — when that text comes in, the held text goes first★ (the text that turn carried returns to the held place
    /// and is the head, and the text held during that turn follows in typing order) · even if the text typed after the halt is removed by ✕ after lifting the halt, the halt does not stand
    /// again — the remaining held text goes out right away (lifting is an arrival event).
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

    /// ★Accepted consequence (1) — if the turn of the text that lifted the halt is interrupted, held text and mail that went into the link go out at the next Idle★ (ADR-0235 「accepted
    /// consequences」 first item): a failure end sets the halt → it lifts the moment user text comes in → even if the turn opened after that is interrupted, the halt does not stand again
    /// → handed-over text returns to the held place and goes out again right away in typing order, and when that user turn ends, the held mail goes out.
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

    /// ★The halt does not stand on a below-floor incarnation★ (N1 (a)) — even after `failed` · an exit, held input goes out right away with `turn/start`
    /// as today.
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

    /// ★Steer text without an echo carried by a `failed` turn is not erased but held behind the halt★ (user decision 2026-09-26 · ADR-0235 — replaces 「erase on failure」)
    /// · 0 list events (row as is) · accepted text leaves and held mail stays · the halt stands. It does not wait for that steer's
    /// response — when the next user text lifts the halt, the returned text goes out right away with `turn/start` before that text (before the held mail),
    /// and a late success response of the old request emits nothing.
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

    /// ★A late receipt during the halt★ — if the echo of text a failure end returned to the held place comes late, receipt + vendor bubble + display-only boundary stand and that
    /// text leaves (it does not go out again), but the halt stays (0 mail turns), and that boundary does not pass the turn table, so the kernel's error
    /// end (`last_end_failed`) stays too (TRD §5-5 「when a receipt comes after the turn end」).
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

    /// ★The error end of the codex classifier (TRD §5-0)★ — an attributed `TurnEnd{Failed}` (translated `failed` · early-end release · each of the five exits)
    /// → `last_end_failed` of the turn table true · the next `completed` → false (lifted) · interruption → unchanged.
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

    /// `turn/completed` without a status field.
    fn statusless_completed_line(thread_id: &str, turn_id: &str) -> String {
        serde_json::json!({
            "method": "turn/completed",
            "params": {"threadId": thread_id, "turn": {"id": turn_id, "items": []}},
        })
        .to_string()
    }

    /// The three statuses of unknown meaning — absent · a value not carried into an outcome · drift.
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

    /// ★An end of unknown meaning neither sets the halt after error nor lifts the kernel's error end★ — if the turn opened after user text lifted the halt after `failed`
    /// ends with a status of unknown meaning, the link's halt does not stand and the kernel's `last_end_failed` stays as is. Item disposal is the same as `completed`
    /// — list items that ended without an echo return to the held place.
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

    /// An end of unknown meaning also returns steer text without an echo to the held place like `completed` and lets it ride the next turn right away (it does not wait for the response
    /// · if nothing is held, no turn opens).
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

    /// (level, field name → value) — every event of this thread.
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

    /// The lines whose message contains `needle`.
    fn logged<'a>(lines: &'a [LogLine], needle: &str) -> Vec<&'a LogLine> {
        lines
            .iter()
            .filter(|(_, l)| l.get("message").is_some_and(|m| m.contains(needle)))
            .collect()
    }

    /// ★The halt's transitions remain in the log★ — the halt setting and lifting = info (end kind · agent). User text is in no line. The lift
    /// line is written by `send_turn` outside the lock — the test bench runs without a link, so it measures only that line's level · fields.
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

    /// A halt discarded as the link closes leaves one line (debug · agent) — the close of a link that had no halt leaves no such line.
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

    /// A command approval request — from the measured shape (fixture `refuse_u2a`), only the fields attribution reads and a few beside them were kept.
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

    /// One end line of a tool item. The command is the measured shape of our refusal (`exitCode` · `aggregatedOutput` are `null`).
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

    /// The memory is one slot per id, and over the cap it drops the oldest and leaves that fact at debug.
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

    /// One end line takes out the memory and changes only the `Failed` · `Declined` results of that id to `Refused` — other ids · other outcomes ·
    /// lines that are not ends stay as is.
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

    /// The boundary of the id cap — up to the same [`MAX_ID_BYTES`] as the translator it points, and over by even one byte it does not point (both approvals).
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

    /// ① Command approval (no `approvalId`) → one refusal line → a `failed` end of the same `itemId` (`exitCode: null`) = `Refused`.
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

    /// ② A `failed` end that came without a refusal stays `Failed`.
    #[test]
    fn a_failed_end_we_never_refused_stays_failed() {
        let mut h = harness_with_real_decoder();
        feed(&mut h, &[command_end("exec-1", "failed")]);
        assert_eq!(
            tool_results(&h.seen),
            vec![result("exec-1", ToolOutcome::Failed)]
        );
    }

    /// ③ Two sources — in one reader, a refused id is `Refused`, and `declined` of an id not refused is the vendor denial `Declined` (the spot where the reason
    /// splits).
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

    /// ④ File change approval refused → `declined` end (measured shape) = `Refused` · even a `failed` end after the same refusal is `Refused`.
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

    /// ⑤ No attribution — other requests · sub-command approval · network approval · no `itemId` are only refused and not remembered.
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

    /// ⑤ continued — an `itemId` over the cap (128 B) is not remembered, and the translator emits no result for its end (there is no row to point to).
    /// ★Whether it is remembered is looked at **before** the end line★ — the end line takes out a remembered id in its original shape, so looking afterwards it would
    ///   look empty even if the filtering were missing.
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

    /// The boundary of ⑤ — `approvalId` · `networkApprovalContext` of a command approval being `null` is the same as absent (it remembers).
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

    /// ⑥ The end line takes out the memory — the second end of the same id is the vendor outcome as is, and a `completed` end also takes it out.
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

    /// ⑦ Cap — with 17 refusals the first id is pushed out and its end is `Failed`, and the second id is `Refused`.
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

    /// ⑧ Not emptied at the turn end — even if the end comes after the turn end (the shape of the last line of measured `steer_m6`), it is `Refused`.
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

    /// ⑨ Overflow · closed — if the refusal line could not go into the control queue, the peer gets no answer, so it is not remembered.
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

    /// ⑩ New reader = empty memory — one reader's refusals do not leak into another reader (= the next incarnation).
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

    /// ⑪ ★Measured (codex-cli 0.156.1 · fixture `refuse_u2a` — third B5 capture)★: after each call of the command turn · the file change turn, there is exactly one
    /// `Refused` result of the same id. One refusal line goes out for each of the two server request lines (those answers are not in the fixture — the reader
    /// builds them itself), and the `serverRequest/resolved` · `thread/status/changed` in between do not shake the attribution.
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

    /// ⑫ The same id is one slot — even if the same `itemId` is refused twice the memory is one, so the first end takes it out as `Refused` and the second end is `Failed`.
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
