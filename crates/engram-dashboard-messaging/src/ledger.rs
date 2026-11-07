//! ledger — message history + request reply tracking + group delivery ledger (spec §2·§3·§5).
//!
//! ★Role★: holds three axes.
//!   ① **History ring buffer** — state transitions + timestamps of every message (`pending→delivered→replied` / `expired` / `skipped`).
//!      "The state-transition time is the reply/send time data" (not exposed in the envelope — spec §5).
//!   ② **Request tracking** — opens `awaiting_reply`, closes by **strict matching** on `in_reply_to`, and computes
//!      `reply_by` timeout overruns (a later increment creates the notice to the sender — here we only compute "who exceeded").
//!      ★There are two closing reasons (amendment 4 — ADR-0116 decisions 2/3 · ADR-0118 decision 1)★: reply accepted (`replied`) or **failure
//!      close** (`reply_failed` — the reply send hits `RECIPIENT_NOT_FOUND` / requester profile deletion cleanup).
//!      Missing the deadline is still **not a closing reason** (ADR-0108 decision 1 — notice ≠ close).
//!   ③ **Group delivery ledger** — 1 message : N delivery records (spec §4). Supports dead members as `skipped`.
//!
//! ★Pure, injected clock (load-bearing — module header invariant)★: every timestamp in state transitions and timeout decisions takes
//!   `now: Instant` as an argument. The ring buffer and the tracking map hold no clock — TTL and reply-by boundaries are asserted in deterministic unit tests.
//!
//! ★Strict reply matching (load-bearing — spec §2 · ADR-0103 invariant)★: a reply closes a request only when `in_reply_to` points
//!   **exactly** at an open request id. Lenient matching (treating the next message from a non-replying peer as a reply)
//!   was rejected as an accidental-close misfire — a wrong id closes nothing.
// ADR-0103

use std::collections::{HashSet, VecDeque};
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

use crate::PeerId;

/// History ring buffer capacity — when exceeded, the oldest record is evicted first (spec §5 "history ring buffer").
///
/// ★4096 (user-ratified 2026-07-26 — raised from 1024)★: the unit of this ring is not a message but a **delivery record**.
///   One broadcast writes **N** `(msg_id, 멤버)` records (spec §4 "1 msg_id : N delivery records"), so broadcasting 100 times
///   to a 10-member group filled 1024 by itself — rotation was too fast, so ① the queryable history got sharply shorter and ② expiry/recall
///   transitions fell to `NotFound` and that fact never reached the ledger (the only evidence left then is the debug log
///   of `sweep`/`flush_for` — C4 review fix J). Raising it 4x absorbs group-sized fan-out.
/// ★Memory profile (honest upper bound)★: a record **keeps the full body as is** (no summary, no truncation). So the worst case is
///   `본문 최대 크기 × 용량` = 64KiB(`control::ingress::MAX_BODY_BYTES`) × 4096 ≈ **256MiB** — human-conversation-sized bodies
///   (hundreds of bytes to a few KiB) come to only ~1–2MB, but a pathological stream of consecutive max-size
///   messages is far larger. This value is for the in-memory stage only and is a tuning target that can be changed without breaking anything.
/// ★Follow-ups (identified only — not implemented now, user mention 2026-07-26)★: ⓐ expose this value as a runtime setting/command
///   ⓑ store truncated bodies for audit purposes (instead of keeping the full text). Both are separate items and no precondition for changing this constant.
/// ★Relation between evict and request tracking (narrowed by C3 review fix 3)★: previously a history evict
///   unconditionally dropped the tracking entry, so an open request whose history was pushed out **lost both its way to be closed by a reply and its deadline notice**
///   (silent contract loss = the worst failure mode). The bound now comes from `MAX_OPEN_REQUESTS`, not the history capacity.
const HISTORY_CAPACITY: usize = 4096;

/// Upper bound on the number of request contracts that can be open at once (not replied, not notified).
///
/// ★Why it is needed (the counterpart of fix 3 — load-bearing)★: once open contracts were made to survive history evict (comment on the constant above),
///   the history capacity **no longer supplies** a bound on the tracking list. Without a cap, requests whose reply never comes
///   (including those with no deadline) pile up and break the bounded guarantee of in-memory v1. So the cap is put on open contracts themselves,
///   and at the cap a **new request is bounced** (old contracts are not silently dropped — no silent loss).
/// ★Basis for 512★: a rough figure that comfortably covers mailbox cap 100 × a few dozen concurrent recipients. If open contracts reach this number at
///   human-conversation message rates, that is not normal load but peers that do not reply piling up,
///   so it is right to make it visible to the sender through a bounce (a tuning target like HISTORY_CAPACITY — can be changed without breaking anything).
const MAX_OPEN_REQUESTS: usize = 512;

/// State of one message delivery (spec §5·§6 state vocabulary — do not invent new vocabulary).
///
/// ★State transitions (load-bearing)★: `Pending → Delivered → Replied` (request only) / `Expired` (TTL) / `Skipped`
///   (notice lane retirement) / `Failed`.
///   Each transition leaves a timestamp (spec §5 "the state-transition time is the reply/send time"). Busy waiting, injection-failure parking and
///   **sleep parking** (ADR-0116 decision 1) are all `Pending` (shared state vocabulary — spec §5 branch 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryStatus {
    Pending,
    /// Actual injection done (delivered = the moment of actual injection, ADR-0104 invariant).
    Delivered,
    Replied,
    /// Parking expired by exceeding TTL (24h) (the ledger keeps the record, spec §5, ADR-0105).
    Expired,
    /// ★Ledger-only terminal★ — the oldest notice was retired because the notice lane (`NOTICE_CAP`) was exceeded (spec §6 mapping table).
    ///   ★It is not in the send-response vocabulary★ (ADR-0111 decision 3 removed group-member skip, so it was dropped from the response axis).
    Skipped,
    /// ★Per-recipient failure terminal (new — ADR-0111 decision 3 · spec §5 "failed recipients stay in the ledger too")★:
    ///   entry bounce (`RECIPIENT_NOT_FOUND`/`RECIPIENT_AMBIGUOUS`) ·
    ///   `MAILBOX_FULL` · `REQUEST_CAPACITY` · **deletion cleanup** (`RECIPIENT_DELETED` — amendment 4).
    ///
    /// ★Why it stays in the ledger (load-bearing)★: the sender must be able to look later via `eg_messages{id}` at "who did not receive it",
    ///   and only then does **ledger expected row count = send response row count** hold, removing `may_be_truncated` false positives
    ///   (spec §5·§6). There is no parking, but there is a record.
    /// ★The moment of recording is the terminal — **with exactly one exception** (amendment 4 · ADR-0116 decision 4)★: entry-bounce kinds are
    ///   `record`ed in this state and no transition leaves it. Only deletion cleanup closes an *already parked* record after the fact,
    ///   so it uses the one edge `Pending → Failed`.
    // ADR-0111
    // ADR-0116 (pending→failed = deletion cleanup only)
    Failed,
}

impl DeliveryStatus {
    /// Is the transition from this state to `next` **legal** (spec §5 state-transition graph — load-bearing).
    ///
    /// Every other edge is illegal — in particular a re-transition out of a **terminal** (`Replied`/`Expired`/`Skipped`/`Failed`),
    ///   and a revert (`*→Pending`) or a skip-ahead (`Pending→Replied`, `Expired→Delivered`, etc.) are rejected.
    ///   Allowing reverts or skip-aheads pollutes the ledger meaning "state-transition time = reply/send time" (misfire closes,
    ///   retroactive timestamps). A self-transition to the same state is also illegal (prevents meaningless timestamp refreshes).
    /// ★No edge into `Failed` exists in this **general** graph (ADR-0111 · kept in amendment 4 as well)★: a failure row is
    ///   recorded in that state **from the start**. Opening it as a general transition would make the impossible history
    ///   "delivered, then turned back into failure" expressible, so the `Pending → Failed` (deletion cleanup) added in amendment 4 is also not put here but opened
    ///   only through a **dedicated verb** (`fail_pending` → `can_fail_by_cleanup`).
    // ADR-0116 (pending→failed is path-limited — general graph unchanged)
    fn can_transition_to(self, next: DeliveryStatus) -> bool {
        use DeliveryStatus::*;
        matches!(
            (self, next),
            (Pending, Delivered)
                | (Pending, Expired)
                | (Pending, Skipped)
                | (Delivered, Skipped)
                | (Delivered, Replied)
        )
    }

    /// ★Deletion-cleanup-only edge (`Pending → Failed`, spec §6 · ADR-0116 decision 4)★ — can this state go to
    /// `Failed` **by deletion cleanup**?
    ///
    /// ★Why a separate predicate (load-bearing)★: this edge is legal only on "the one path that closes an already parked record after the fact".
    ///   Putting it in the general graph (`can_transition_to`) would let **any caller** turn a pending row into a failure
    ///   (e.g. an expiry sweep bug, a flush misbranch), producing the history "mail that was waiting for delivery was written as failed with no reason".
    ///   With the predicates split, that ability lives in the single verb `fail_pending`, and `Delivered → Failed` is in
    ///   neither predicate, so it stays illegal forever.
    fn can_fail_by_cleanup(self) -> bool {
        matches!(self, DeliveryStatus::Pending)
    }
}

/// One history record — the delivery history of one (message, recipient) pair.
///
/// ★1 message : N delivery records (spec §4 · load-bearing)★: a group send delivers one logical message (`msg_id` shared) to several
///   recipients individually, so there is **one delivery record per recipient** (each with independent status and timestamps). A single send is N=1.
/// ★body is kept in full, not as a summary (design decision)★: this is the in-memory stage with no separate store, and re-injecting a parked envelope and
///   ledger lookup (`messages { id }` — spec §6) need the original text. Keeping only a summary loses the original for re-injection and audit.
///   Memory is capped by the ring capacity (HISTORY_CAPACITY) — summary/offload will be revisited with v2 persistence (SQLite) (non-breaking).
/// ★Who is by id, what is shown is the name at send time (ADR-0309 decision 4)★: the axis that picks a row (`transition` ·
///   `fail_pending`) is `(msg_id, to_id)`, while `from` · `to` are **name snapshots at send time**, so a later rename
///   does not change what a lookup displays.
#[derive(Debug, Clone)]
pub struct MessageRecord {
    pub msg_id: String,
    /// Sender name — snapshot at send time (WYSIWYA — ADR-0101).
    pub from: String,
    /// Sender agent id. A daemon notice (`<notice>`) is nil (service `daemon_identity`).
    pub from_id: PeerId,
    /// Recipient name — snapshot at send time. For a failure row where the recipient was not found, it is the spelling the sender wrote.
    pub to: String,
    /// Recipient agent id. `None` = a failure row where the recipient was not found (none · duplicate name) — that row is already a terminal, so
    ///   there is no need to pick it again by id.
    // ADR-0309 (decisions 1 · 4)
    pub to_id: Option<PeerId>,
    pub body: String,
    pub status: DeliveryStatus,
    pub created_at: Instant,
    pub transitioned_at: Instant,
    /// ★The **total number of delivery records** this logical message will leave (fixed at send time — round-2 review F3)★. Single send · notice = 1,
    /// group fan-out = member count N. Every row with the same `msg_id` carries **the same value**.
    ///
    /// ★Why it is needed (where the old "front position" proof was wrong)★: a lookup must answer "are the remaining rows all of them?", but
    ///   group rows are recorded in **two phases** (parked/skipped under the plan lock, then delivered in the per-member section).
    ///   Rows of other messages can slip in between, so the rows of one msg_id are **not contiguous** in the ring — even when the front rows
    ///   are evicted and only the back rows remain, the ring front may be someone else's row. So the position-based proof "if it is not the front, it is complete"
    ///   gives a **false negative**. Pinning the expected count at send time turns it into the **deterministic** comparison `남은 행 수 < 기대`
    ///   (it does not depend on position or order).
    /// ★Why u16★: a broadcast's member count is roster-sized, so 65535 is enough, and at 2 bytes per record it adds only
    ///   8KiB to a 4096-entry ring (negligible next to the cost of keeping bodies).
    // round-2 review F3
    pub expected_rows: u16,
    /// ★Failure code to carry in lookups (new in amendment 4 — spec §6 `RECIPIENT_DELETED`)★.
    ///
    /// ★Why it holds the wire string as is (load-bearing)★: the place where this code first appears is **not the send response but the
    ///   `eg_messages{id}` lookup** (at send time it was `pending` — spec §6). So the value must be **kept** until a later time than the send response,
    ///   hence it rides on the record. The canonical vocabulary is `service::FailCode::as_str` and its return value goes in here
    ///   as is — if the ledger knew service's enum as a type, the pure storage layer would stick to the policy vocabulary, so
    ///   it takes only a `&'static str` layer (no value copy).
    /// ★Only one path fills it now: deletion cleanup★: for `failed` rows of the entry-bounce kinds, the send response already delivered
    ///   code/hint on the spot (spec §6), so they are not kept twice on the lookup axis (`None`).
    // ADR-0116 (RECIPIENT_DELETED — lookup-time code)
    pub fail_code: Option<&'static str>,
}

/// One open-request tracking entry (spec §3). It is a **separate map** from the history records, so ring evict does not affect it.
///
/// ★notified flag (load-bearing — prevents double notification)★: once a `reply_by` overrun is reported through `due_timeouts`,
///   this flag is set and it is **not reported again** (spec §7 "no double-notification"). When a reply arrives it is `closed`, so it drops out of
///   the `due_timeouts` targets (replied never comes out as due).
// R1: `reservation_token`(Weak) has no meaning under value comparison, so the PartialEq/Eq derive was dropped (nobody used it).
#[derive(Debug, Clone)]
struct RequestEntry {
    request_id: String,
    /// The display name **at send time**.
    sender: String,
    /// ★The request sender's PeerId (C3 review fix 2 — load-bearing)★: the queue key under which the deadline-overrun notice is entrusted (ADR-0309 —
    ///   the queue key is the id, so even if the sender renamed in the meantime the notice goes to them. The notice has no re-fire because of `notified`,
    ///   so missing it here silently halves the contract).
    sender_id: PeerId,
    /// The recipient name **at send time** (for lookups · deadline-notice body display). The axis that picks the contract is `recipient_id`.
    recipient: String,
    /// ★The recipient axis of the contract key = agent id (ADR-0309 decisions 1 · 7)★ — the id for a live recipient, and **for a sleeping recipient too it is
    /// that profile's id from the moment of opening** (the daemon passes the sleeping side as an (id, name) pair). So there is no name-fallback matching — even if it is renamed,
    /// or another agent with the same name appears, the obligation stays with this id.
    /// ★epoch is not carried★: a restart of the same agent keeps the PeerId and only raises the epoch (ADR-0007) —
    ///   the restarted agent is still the party to answer, so narrowing by epoch would wrongly make the obligation vanish.
    // ADR-0309
    recipient_id: PeerId,
    /// Reply deadline = (offset from send, **original spelling the sender wrote**). `None` = no deadline (no timeout).
    ///
    /// ★Why the spelling is kept too (C3 review fix 6)★: previously only the Duration was kept, and when building the notice text the upper layer
    ///   **back-computed** the spelling — that back-computation normalized, so a deadline sent as `60m` was notified as `1h`, and the envelope (`reply-by="60m"`)
    ///   and the text disagreed. Contract text must be exactly what the sender wrote, so the spelling is kept as the original (the two
    ///   are bound in one tuple so the type enforces "if there is a deadline, the spelling is always there").
    reply_by: Option<(Duration, String)>,
    created_at: Instant,
    closed: bool,
    /// ★Failure-close marker (new in amendment 4 — ADR-0116 decisions 2/3 · ADR-0118 decision 1)★. It is set **always together with** `closed`,
    /// and when this value is `true` the terminal vocabulary is `reply_failed`, not `replied` (contract axis — spec §6).
    ///
    /// ★Why one bit next to `closed` (load-bearing)★: the **consequences** of closing (removal from the open list, no deadline-sweep fire, exclusion from the 512
    ///   count) are exactly the same as `replied` — so those decisions are not made twice, and the single `closed` predicate is
    ///   used as is (ADR-0118 decision 4 "keep the single `occupied_slots` predicate"). This bit carries only the **reason**:
    ///   the fact that this terminal does not claim "a reply was established" (§6 axis distinction), and the observable face of the rule that deletion cleanup **does not touch a contract that is
    ///   already `replied`** (§3 item 7-④ no reverting).
    /// ★There are exactly two firing paths★: ① the reply send row is `RECIPIENT_NOT_FOUND` (`fail_on_undeliverable_reply`)
    ///   ② requester profile deletion cleanup (`fail_open_requests_from`). Any other reply failure is a **no-op**.
    // ADR-0116 (decision 2) / ADR-0118 (decisions 1·4)
    reply_failed: bool,
    notified: bool,
    /// ★**Marked for retirement** under cap pressure (round-5 mark-and-sweep)★ — it is still alive in the list and can still receive
    /// a reply. It is physically removed only at commit, and on rollback only the mark is cleared, returning to the state where nothing happened.
    ///
    /// ★Why a mark (why physical removal was dropped — load-bearing)★: the old design **took the victim out** of the list at reservation time.
    ///   During that window the contract behaved as if it did not exist in the world, so ① a legitimate reply missed with
    ///   `NoMatch` in `close_on_reply` and ② rollback put it back "open", creating a ghost state and a false notice. If it is not taken out and only
    ///   marked, that window does not exist — replies, lookups and duplicate checks all keep working on the usual path.
    // round-5 mark-and-sweep
    pending_retirement: bool,
    /// ★A new contract whose acceptance is not yet confirmed (round-5 mark-and-sweep)★ — when the send passes dispatch, this mark is cleared at commit,
    /// and on bounce or panic the rollback removes this entry.
    ///
    /// ★Why it is needed★: this entry **occupies a slot but is not yet entitled to take someone else's place**. Without the mark,
    ///   another send arriving at the same time could pick this unconfirmed contract as "the oldest retirable contract" and remove it,
    ///   and then the original send would be in the state **delivered successfully but no contract** (every reply to that request
    ///   is `NoMatch`). So it is explicitly excluded from victim selection.
    // round-5 mark-and-sweep
    provisional: bool,
    /// ★The contract key of the **victim this reservation marked for retirement** (F1)★ — `Some((msg_id, recipient_id))`.
    ///
    /// ★Why remember it (load-bearing)★: a reservation left without settlement must be **reclaimable by the periodic sweep itself**
    ///   (RAII Drop can do nothing if `try_lock` fails — `service::Reservation` header). Then the sweep
    ///   must know "whom did this reservation mark" so it can release exactly that mark. The value the caller was carrying
    ///   (`RetiredContract`) disappears together with the reservation if it is lost, so it is kept inside the ledger.
    marked_victim: Option<(String, PeerId)>,
    /// ★The **owner liveness token** of this reservation (R1)★ — `Some(weak)` = a guard is attached, `None` = one was never attached.
    ///
    /// It is the sweep's reclaim criterion (`reclaim_abandoned_reservations`): if it upgrades, the owner is still working, so
    /// **it is not touched**. For the rationale and the rejected alternative (a time threshold), see the `ReservationLiveness` header.
    /// ★Why `None` is a reclaim target (fail-safe)★: attachment happens in the **same lock section** as `open_request`
    ///   (operations pass A). So if the sweep can see a provisional entry that is `None`, the opener released the lock without creating a guard
    ///   = a reservation nobody will settle. Leaving that kind behind would permanently shrink the cap denominator.
    // R1
    reservation_token: Option<Weak<()>>,
}

impl RequestEntry {
    /// ★Criterion = `!closed` alone (D review B3 — corrected from the old `!closed && !notified`)★: previously a contract for which the deadline-overrun notice
    ///   had gone out (`notified`) was treated as "finished" and dropped together on history evict. But the notice is only the fact that **the sender
    ///   was told**, not that a reply came — the recipient still has the duty to answer (spec §3 "reply even if late"), and D's pending lookup
    ///   contracted to **keep showing** such contracts as `timed_out=true`
    ///   (`open_requests` filters by `!closed`). When the two criteria diverged, the obligation actually evaporated in this order:
    ///   ① request opens → ② 4096 entries push its history row out by evict → ③ deadline passes and `notified=true` →
    ///   ④ `purge_finished_without_history` deletes the contract as "finished and no history either" → it vanishes from the pending list.
    ///   This is a silent loss that both sender and recipient misread as "a finished matter", so the definition is aligned to the pending lookup side.
    /// ★The bound is kept★: notified unreplied contracts are now counted in the cap too, so the tracking list is tied to `MAX_OPEN_REQUESTS` (512)
    ///   — at the limit a new request is bounced with `REQUEST_CAPACITY` (a visible failure instead of silent loss).
    // review B3
    fn is_live(&self) -> bool {
        !self.closed
    }

    /// ★Does it occupy a `MAX_OPEN_REQUESTS` slot (round-5 mark-and-sweep · round-6 I1)★ — the sole criterion of the cap decision.
    ///
    /// - **A contract marked for retirement is not counted**: its slot leaves at commit, so it already belongs to the new contract. If it were counted, the cap
    ///   would still hold right after the mark, the new contract could not be accepted, and the retirement itself would be pointless.
    /// - **A provisional contract **is counted** (important)**: even before confirmation it is an intake that really exists, and if it were not counted, several simultaneous sends
    ///   would all decide "there is room" and get in past the limit (the cap is breached).
    /// - ★**A closed provisional contract keeps being counted until settlement**(round-6 I1 · load-bearing)★: looking only at `!closed`,
    ///   the moment a reply arrives during the provisional window the contract **loses its own slot**. In fact the following 6 steps
    ///   permanently breached the limit: ① A marks V1 and inserts provisional PA (512) ② a fast reply closes PA (511) ③ B sees
    ///   "there is room" and inserts PB **without marking anyone** (512) ④ A rolls back and releases the V1 mark,
    ///   +1, and even deleting the closed PA gives 0 → **stuck at 513**. The slot is **reserved** as the price of accepting the send, so that reservation
    ///   must hold until the guard settles it (commit/rollback) — the fact that the reply was fast must not create someone else's slot.
    /// ★Arithmetic at settlement (both branches come out exactly right)★:
    ///   - **Commit**: the provisional mark is released → if that contract was closed, the slot is released at that point **legitimately**
    ///     (it is a contract that actually got its reply, so releasing the slot is right).
    ///   - **Rollback**: remove the closed provisional contract (-1) and release the victim mark (+1) → the sum is 0, exactly keeping the cap.
    /// ★`reply_failed` also drops out here **automatically** (ADR-0118 decision 4 — no second capacity concept)★: a failure close
    ///   sets `closed`, so this predicate yields false as is. Do not add a separate branch — the very reason this function exists is
    ///   that the cap count is a single predicate (the settlement arithmetic holds in this one place only).
    // round-5 mark-and-sweep / round-6 I1
    // ADR-0108 (capacity predicate single point — a provisional contract keeps its weight even when closed)
    // ADR-0118 (decision 4 — reply_failed drops out of the count)
    fn occupies_slot(&self) -> bool {
        (!self.closed || self.provisional) && !self.pending_retirement
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplyOutcome {
    /// An open request was closed exactly (the first valid reply). History also transitioned normally to `Replied`.
    Closed,
    /// The contract (tracking) was closed but the history `Replied` transition is an illegal edge and did not happen — for anomaly observation.
    ///
    /// ★Why a separate variant (load-bearing — finding 1)★: the reply **actually happened**, and reopening the contract is
    ///   worse (the source of truth is tracking). So the contract is still closed. But if the history is still in a state like `Pending` (not injected) that
    ///   cannot take the `Delivered → Replied` edge, the history stays without reflecting the reply — this is not swallowed silently
    ///   (it used to be hidden as `Closed`); it is returned carrying `from` (the history state at that moment) so the upper layer (MessagingService) can
    ///   **observe and log** it. Closing the contract and history bookkeeping are separate concerns: anomaly = observable, not silent.
    ClosedHistoryAnomaly { from: DeliveryStatus },
    /// No matching open request — a wrong id, or already closed/nonexistent (strict: closes nothing).
    NoMatch,
    /// A second reply to an already closed request — no-op (the first reply already fulfilled the contract).
    AlreadyClosed,
}

/// ★Outcome of a reply-failure close attempt (new in amendment 4 — spec §3 item 7-④ · ADR-0118)★.
///
/// ★The three other than `Failed` are all **no-ops**★ — the contract stays open or is already closed. The caller differs only
///   in the log per outcome (no effect on delivery or the response — the outcome of the reply message itself was already reported as a failure row).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplyFailOutcome {
    /// Closed the contract with the `reply_failed` terminal (removed from the open list, deadline sweep and 512 count; history remains).
    Failed,
    /// That replier has no open contract — an unknown id or someone else's contract (normal path, same shape as `NoMatch` of `close_on_reply`).
    NoMatch,
    /// An already closed contract (`replied` or already `reply_failed`) — not reverted.
    AlreadyClosed,
    /// ★Guard held (ADR-0118 decision 3)★ — a provisional / retirement mark is attached, so its lifetime is owned by that guard. No-op.
    GuardHeld,
}

/// ★What requester deletion cleanup did on the contract axis (spec §5 deletion cleanup ②)★ — the caller logs it **outside the lock**.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct RequesterCleanup {
    /// Contract keys `(request_id, recipient_id)` closed as `reply_failed`.
    pub failed: Vec<(String, PeerId)>,
    /// Number of contracts skipped because of a guard (provisional / marked for retirement) — they do not re-fire (one shot at deletion time, spec §5).
    pub guard_held: usize,
}

/// ★Why a typed error (load-bearing)★: the old `transition` returned only a `bool` (success/nonexistent) and **silently performed
///   illegal transitions** (allowing reverts and skip-aheads like `Expired → Delivered`). That pollutes the ledger meaning
///   "state-transition time = reply/send time". Now an illegal transition is rejected by type so the upper layer notices the bug at once (spec §5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransitionError {
    /// No (msg_id, to) record — evicted or nonexistent.
    NotFound,
    Illegal {
        from: DeliveryStatus,
        to: DeliveryStatus,
    },
}

/// ★One contract **marked for retirement** under cap pressure (round-2 review F1 · user decision 2026-07-27)★ — the material from which the caller
/// writes an instrumentation log **outside the lock** (no silent disappearance).
///
/// ★When it arises★: unreplied contracts have hit `MAX_OPEN_REQUESTS`, a new request came in, and the tracking list has
///   a **retirable** contract (= one with no remaining notice promise to the sender). The oldest of those gives up its
///   place — the same pattern as the mailbox and notice lane "retiring the oldest" at the cap.
///
/// ★It holds values only (it does not carry the original entry)★: the victim never left the list, so there is no state
///   to revert — rollback only has to clear that entry's mark. So this struct does nothing beyond carrying
///   "what was about to retire" in a human-readable way.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetiredContract {
    pub request_id: String,
    pub sender: String,
    pub recipient: String,
    pub recipient_id: PeerId,
    /// Age at marking time (for logs — elapsed time, not wall clock).
    pub age: Duration,
}

/// ★Reservation liveness token (R1 — replaces the time threshold with a **liveness decision**)★. The strong side is held by the `service::Reservation` guard,
/// and the ledger sees only the weak side (`Weak`). While the guard is alive upgrade succeeds, and when it is destroyed without settlement it fails.
///
/// ★Why liveness, not time (load-bearing — the failure mode this design removes)★: a reservation lives through "decide under lock → inject outside lock
///   → settle under lock". That **injection is unbounded** — a child's stdin can block
///   forever under backpressure (see agent `stdio.rs`). So any constant like "over N seconds means abandoned" can be wrong,
///   and the way it was wrong was the worst: if the sweep reclaims a reservation that is **still working**, the later injection succeeds
///   and even when `commit` runs the contract is already gone → a `type="request"` envelope was delivered but there is no contract (the sender's
///   `awaiting_their_reply` is 0 · the recipient's `reply_owed_by_me` is 0 · the deadline notice never comes · a legitimate later reply is
///   `NoMatch`, so the history row stays `Delivered` forever = the audit record lies "no answer"). The liveness decision removes that
///   race itself **structurally** — while the owner is alive the sweep is not entitled to see that reservation.
/// ★The threshold constant was deleted★: do not revive it (basis above = unbounded injection).
// ADR-0108 (mark-and-sweep — reclaim criterion = liveness)
#[derive(Debug)]
pub struct ReservationLiveness(Arc<()>);

impl ReservationLiveness {
    pub fn new() -> Self {
        Self(Arc::new(()))
    }

    pub fn watch(&self) -> Weak<()> {
        Arc::downgrade(&self.0)
    }
}

impl Default for ReservationLiveness {
    fn default() -> Self {
        Self::new()
    }
}

/// ★What the commit **actually did** (R2)★ — a fact, not a plan.
///
/// ★Why it is needed (load-bearing — instrumentation pollution)★: the old `commit_open` had no return value and the caller **assumed** "the planned
///   retirement happened" and printed the instrumentation log. But a marked victim can vanish before commit (in the meantime
///   it is closed by a reply, and if its history row is also pushed out of the ring, `purge_finished_without_history` cleans it up —
///   a known residual noted in the `rollback_open` comment). Then a **retirement that did not happen** is reported, contaminating the axis that ADR-0108 decision 2 nailed down as the
///   sole evidence of retirement (operators chase a phantom retirement).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommitOutcome {
    /// Did it actually clear the provisional mark (= was that contract still in the list).
    pub confirmed: bool,
    /// Did it **actually physically remove** the marked victim = did the retirement actually happen.
    pub retired: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenOutcome {
    Opened,
    /// Opened a new request (provisionally), and under cap pressure **marked the oldest retirable contract** for retirement.
    ///
    /// ★Caller obligation (round-5 mark-and-sweep · load-bearing)★: the mark is **provisional**. The caller must call exactly one of
    ///   the two — ① `commit_open` on send acceptance (physically remove the marked victim + clear the provisional mark, and log
    ///   instrumentation then) ② `rollback_open` on every other exit (bounce, panic) (clear the mark + remove the provisional contract).
    ///   ★The structural guarantee owner of ② = the Drop of `service::Reservation` (RAII guard)★ (ADR-0108 decision 3). If that value is
    ///   destroyed without settlement, a debug build blows up at once and a release build attempts the rollback with an error log — round 1's
    ///   plain `Option<Option<_>>` **had nobody to catch a forgotten branch** (measured by the review prober).
    OpenedAfterMarking(RetiredContract),
    /// Rejected because the same request_id already exists in tracking (open or closed) — no-op. The id is a daemon-generated unique value, so
    /// reuse is a non-scenario (finding 2 — lenient reopen caused a shadowing bug and was removed).
    DuplicateId,
    /// ★Open-contract cap (`MAX_OPEN_REQUESTS`) reached (C3 review fix 3)★ — no new contract is opened (no-op). The upper layer makes it
    /// visible to the sender as a bounce (rejecting the new one instead of silently dropping an old contract — no silent loss).
    Full,
}

/// `drop_request` result — whether it was removed + **whether that contract was already in the notified state** (C3 review fix 5).
///
/// ★Why it returns notified too (load-bearing — observation)★: reclaiming on bounce (`drop_request`) means "the contract was never established",
///   but if that entry was already `notified`, it means **the deadline-overrun notice had already gone out to the sender**
///   (a notice cannot be recalled — it is a message already sent). This double outcome ("the notice went out and it was also bounced") is rare, but
///   it is a state that must not be passed over silently, so the fact is returned along with it so the caller can log it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DropOutcome {
    /// Removed the entry. `notified` = was the timeout notice already out at removal time.
    Removed { notified: bool },
    /// No such id in tracking (idempotent — no-op).
    NotFound,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DueTimeout {
    pub request_id: String,
    /// Name of the sender who will receive the notice (display name **at send time** — it may have been renamed since).
    pub sender: String,
    /// ★Sender id for notice delivery (C3 review fix 2)★ — the upper layer uses it as the queue key under which to entrust the notice + the flush doorbell
    ///   target (`RequestEntry.sender_id` comment).
    pub sender_id: PeerId,
    /// Recipient name (**at send time** — for the notice body).
    pub recipient: String,
    /// The recipient axis of the contract key — the re-check 「그 사이 닫혔나」 (did it close in the meantime) just before notifying finds the contract by this key.
    pub recipient_id: PeerId,
    /// ★The **original spelling** of the exceeded deadline (C3 review fix 6 — for the notice text)★: the spec §1 notice template exposes
    ///   `기한({reply_by})` as is, so the spelling the sender wrote (`"60m"`) is carried **as is**.
    pub reply_by_raw: String,
}

/// ★Lookup view of one unreplied request (S18 D — `eg_messages` no-argument "my pending")★.
///
/// ★Why `RequestEntry` is not exposed directly★: a tracking entry holds the ledger's **internal state** (closed/notified flags,
///   sender_id and other delivery-wiring fields) — exposing it as is would stick the lookup surface to the internal representation, and it would set
///   hard together with it at v2 persistence. Only the facts a lookup needs are copied out as values (pure, read-only).
/// ★Why `notified` is carried★: a contract whose deadline has passed and whose notice went out to the sender is **still unreplied** (it
///   closes when a reply arrives). Dropping it from the pending list would leave "something left to answer but not in the list", so it is included, and
///   the fact that the notice went out is carried along so it can be told apart (the upper layer decides whether to show it).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenRequestView {
    pub request_id: String,
    pub sender: String,
    /// The request sender's PeerId — the axis a pending lookup uses to tell "requests I made" apart **by id, not by name** (review B1).
    pub sender_id: PeerId,
    pub recipient: String,
    /// The axis a pending lookup uses to tell apart 「내가 답할 의무」 (the obligation I owe an answer) (ADR-0309 — not told apart by name).
    pub recipient_id: PeerId,
    pub reply_by_raw: Option<String>,
    pub created_at: Instant,
    pub notified: bool,
}

#[derive(Debug)]
pub struct Ledger {
    history: VecDeque<MessageRecord>,
    requests: Vec<RequestEntry>,
    capacity: usize,
    /// ★Has an evict happened even once (D review B2 — lookup honesty)★.
    /// ★Why not "exactly how many were cut"★: how many rows of which msg_id disappeared is already discarded data and
    ///   cannot be reconstructed. Without inventing accuracy that is not there, it honestly conveys **only the fact that "it may have been truncated"**.
    // review B2
    evicted_any: bool,
}

impl Default for Ledger {
    fn default() -> Self {
        Self::with_capacity(HISTORY_CAPACITY)
    }
}

impl Ledger {
    pub fn new() -> Self {
        Self::default()
    }

    /// Capacity-injected — unit tests verify the evict boundary quickly with a small capacity (consistent with the purity principle).
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            history: VecDeque::new(),
            requests: Vec::new(),
            // capacity 0 is meaningless (always evicts at once) — corrected to a minimum of 1 (defensive).
            capacity: capacity.max(1),
            evicted_any: false,
        }
    }

    /// ★Initial state as a parameter★: a single/group send starts as `Pending` (waiting for injection) or `Delivered` (immediate-injection fallback),
    ///   and a dead group member starts as `Skipped`, so the caller decides the initial state (spec §4·§5).
    /// ★evict = front (the oldest) + clean up **only contracts closed by a reply** together (narrowed by C3 fix 3 → D review B3)★: it is a ring buffer, so
    ///   when capacity is exceeded the oldest history is discarded first. At that time the request tracking entries of the same msg_id are dropped together
    ///   **only if closed by a reply** (dangling cleanup + bound). ★An unreplied contract survives evict whether notified or not★ —
    ///   previously notified ones were dropped together, so after the history was pushed out first, a contract past its deadline **evaporated
    ///   from the pending list entirely** (review B3 — sequence in the `RequestEntry::is_live` comment). The source of truth of a contract is tracking, not history
    ///   (ReplyOutcome comment), so history capacity must not kill a contract. The bound for unreplied contracts is given separately by `MAX_OPEN_REQUESTS`
    ///   (`Full` of open_request).
    /// ★Parties = (name at send time, id) pair★: `MessageRecord` is the source of truth for the field meanings.
    #[allow(clippy::too_many_arguments)]
    pub fn record(
        &mut self,
        msg_id: &str,
        from: &str,
        from_id: PeerId,
        to: &str,
        to_id: Option<PeerId>,
        body: &str,
        status: DeliveryStatus,
        now: Instant,
    ) {
        self.record_with_expected(msg_id, from, from_id, to, to_id, body, status, now, 1);
    }

    /// `record` + **expected delivery record count** (round-2 review F3). A group fan-out records in two phases (plan lock / per-member section),
    /// so both places must pass the same N.
    #[allow(clippy::too_many_arguments)]
    pub fn record_with_expected(
        &mut self,
        msg_id: &str,
        from: &str,
        from_id: PeerId,
        to: &str,
        to_id: Option<PeerId>,
        body: &str,
        status: DeliveryStatus,
        now: Instant,
        expected_rows: u16,
    ) {
        self.history.push_back(MessageRecord {
            msg_id: msg_id.to_string(),
            from: from.to_string(),
            from_id,
            to: to.to_string(),
            to_id,
            body: body.to_string(),
            status,
            created_at: now,
            transitioned_at: now,
            // 0 is meaningless (every send has at least 1 row) — defensively raised to 1 so that the `rows < expected` decision
            //   does not fall into a contradictory state like "there are rows but expected is 0".
            expected_rows: expected_rows.max(1),
            fail_code: None,
        });
        let mut evicted_now = false;
        while self.history.len() > self.capacity {
            if self.history.pop_front().is_some() {
                evicted_now = true;
            }
        }
        if evicted_now {
            // Lookup honesty flag (B2) — once set it never goes back down (discarded data does not come back).
            self.evicted_any = true;
            self.purge_finished_without_history();
        }
    }

    /// ★Removes tracking entries of **closed** (replied) contracts that have no history to point at (round-final fix 1 · load-bearing)★.
    ///
    /// ★What it prevents = zombie tracking entries★: the normal cleanup trigger for a closed entry is "when the same-msg_id history is evicted" (`record`).
    ///   But once fix 3 made **unreplied contracts survive evict**, a contract whose history was pushed out first
    ///   is in the state where **the evict event that would clean it up has already passed** by the time it closes later. That entry has `is_live()` false, so
    ///   it is not counted in `MAX_OPEN_REQUESTS` either, and it piles up forever **with no limit applied** (the bounded guarantee of in-memory v1
    ///   collapses). So at the moment of closing and at evict, we check whether history exists and delete orphans at once.
    /// ★An unreplied contract is never touched (scope narrowed by D review B3)★: previously **notified** (deadline-overrun) contracts were also seen as "finished"
    ///   and deleted together, but a notice is not a reply — that deletion made the obligation evaporate from the pending lookup
    ///   (the 4-step sequence in the `RequestEntry::is_live` comment). Now `is_live() == !closed`, so notified ones are not caught
    ///   here, and their bound is given by `MAX_OPEN_REQUESTS`.
    // review B3
    // ADR-0108 (provisional purge exemption — the lifetime of an unsettled entry belongs to its guard)
    fn purge_finished_without_history(&mut self) {
        if self.requests.iter().all(|r| r.is_live() || r.provisional) {
            return;
        }
        let live_ids: HashSet<&str> = self.history.iter().map(|r| r.msg_id.as_str()).collect();
        // ★A provisional contract is **never deleted here** (round-6 I1 · load-bearing)★: in the provisional window there may be no history
        //   row yet (contract reservation is **before** dispatch, history recording is **inside** dispatch). If a reply arrives in that window
        //   and closes the contract, the condition "finished and no history either" holds and this cleanup deleted the whole entry —
        //   then the reserved slot evaporates and the cap breach I1 was meant to stop comes back **through another door**, and the guard's
        //   rollback gets `NotFound` and settles nothing. The lifetime of an unconfirmed entry is **owned by the guard**.
        self.requests
            .retain(|r| r.is_live() || r.provisional || live_ids.contains(r.request_id.as_str()));
    }

    /// ★Why pick by (msg_id, recipient id)★: a group broadcast has N per-recipient records under one msg_id, so msg_id alone cannot
    ///   identify which delivery — picking the recipient together transitions exactly one record (1:N accounting, spec §4).
    ///   The recipient axis is the id, not the name, so the same row is picked even when the name at send time and the current name differ
    ///   (ADR-0309).
    // ADR-0309
    pub fn transition(
        &mut self,
        msg_id: &str,
        to_id: PeerId,
        status: DeliveryStatus,
        now: Instant,
    ) -> Result<(), TransitionError> {
        let Some(rec) = self
            .history
            .iter_mut()
            .find(|r| r.msg_id == msg_id && r.to_id == Some(to_id))
        else {
            return Err(TransitionError::NotFound);
        };
        if !rec.status.can_transition_to(status) {
            return Err(TransitionError::Illegal {
                from: rec.status,
                to: status,
            });
        }
        rec.status = status;
        rec.transitioned_at = now;
        Ok(())
    }

    /// ★Deletion-cleanup-only close (`Pending → Failed` + reason code — spec §5 deletion cleanup · §6 transition graph amendment)★.
    ///
    /// ★Why the **reason code is a required argument** (load-bearing)★: the type prevents the state where a lookup sees a `failed` with no code and
    ///   cannot answer "why did it fail".
    /// ★`NotFound` (the record was pushed out of the ring) is logged at debug by the caller **outside the lock** — no silent loss.
    // ADR-0116 (decisions 3/4 — deletion cleanup)
    pub fn fail_pending(
        &mut self,
        msg_id: &str,
        to_id: PeerId,
        code: &'static str,
        now: Instant,
    ) -> Result<(), TransitionError> {
        let Some(rec) = self
            .history
            .iter_mut()
            .find(|r| r.msg_id == msg_id && r.to_id == Some(to_id))
        else {
            return Err(TransitionError::NotFound);
        };
        if !rec.status.can_fail_by_cleanup() {
            return Err(TransitionError::Illegal {
                from: rec.status,
                to: DeliveryStatus::Failed,
            });
        }
        rec.status = DeliveryStatus::Failed;
        rec.fail_code = Some(code);
        rec.transitioned_at = now;
        Ok(())
    }

    /// Request open — starts `awaiting_reply` tracking (spec §3 step 2). **Contract key = `(request_id, recipient_id)`**
    /// (ADR-0309 — `recipient` is only a name snapshot at send time, not a key).
    ///
    /// ★Multi-recipient request = N independent contracts (ADR-0111 decision 5 · load-bearing)★: the message id is common to the N contracts
    ///   (the ledger keeps the 1 message : N delivery records principle) and **the contract splits by who the replier is**. So this function is called with the same
    ///   `request_id` **once per recipient**.
    ///   No contract is opened for a recipient that could not be delivered to (entry bounce · `MAILBOX_FULL` · `REQUEST_CAPACITY`).
    ///
    /// ★reply_by clock = from send (spec §3·§5 · ADR-0104)★: absolute deadline = `created_at(now) + reply_by`. It is a contract from the sender's viewpoint,
    ///   independent of receive delay, so it is fixed on now (the send time).
    /// ★Duplicate **key** rejection — rejected if it exists, open or closed (load-bearing · finding 2)★: if **even one** of the same
    ///   `(request_id, recipient_id)` exists in tracking (open OR closed), it is rejected with `DuplicateId`
    ///   (no-op). The message id is a **daemon-generated unique value** and recipients are already deduplicated at the send resolution step
    ///   (spec §5 resolution order ④), so reuse of this combination is a non-scenario. ★A parked recipient also opens a contract (spec §3
    ///   item 2)★ — whether busy or **sleeping** (ADR-0116 decision 1), acceptance is acceptance, so the reply obligation holds and the deadline sweep
    ///   fires normally.
    ///   The old leniency "a closed id may be reopened"
    ///   left two entries at once and caused (a) a reply meeting the earlier closed entry first and misfiring `AlreadyClosed`, (b) a shadowing bug where same-id history
    ///   evict dropped the reopened tracking.
    /// ★At cap = **let the oldest retirable contract go and accept**(user decision 2026-07-27 · round-2 F1)★.
    ///   A full bounce (`Full`) happens only when there is nothing at all to retire.
    ///
    /// ★Why it changed (the hole B3 left)★: once B3 made "notified unreplied contracts" stay in tracking, that kind has
    ///   **neither TTL nor cancel** — if the reply never comes it holds a slot forever. If 512 fill up that way, **every** new request is blocked with `REQUEST_CAPACITY`
    ///   until the daemon restarts (global feature stop). The pressure is relieved with the same pattern as the mailbox and
    ///   notice lane "retiring the oldest" at the cap.
    /// ★Retirable (evictable) = a contract with **no remaining notice promise** to the sender★:
    ///     (a) `notified == true` — the deadline-overrun notice already went out (the sender was told the outcome), or
    ///     (b) `reply_by == None` — there was no deadline to begin with, so no notice was ever promised.
    ///   ★Never retired★: a contract that still has a deadline but has not been notified — that contract is a **debt the daemon owes the sender**
    ///   (the notice on deadline overrun). Deleting it would be a silent breach where the promised notice never goes out.
    /// ★History ring rows are not touched★ (the ring owns its own lifetime — retirement is a matter of **contract tracking** only).
    // round-2 review F1 / user decision 2026-07-27
    // ADR-0309 (contract key = recipient id)
    #[allow(clippy::too_many_arguments)]
    pub fn open_request(
        &mut self,
        request_id: &str,
        sender: &str,
        sender_id: PeerId,
        recipient: &str,
        recipient_id: PeerId,
        reply_by: Option<(Duration, String)>,
        now: Instant,
    ) -> OpenOutcome {
        if self
            .requests
            .iter()
            .any(|r| r.request_id == request_id && r.recipient_id == recipient_id)
        {
            return OpenOutcome::DuplicateId;
        }
        let mut retired = None;
        if self.occupied_slots() >= MAX_OPEN_REQUESTS {
            // ★Two kinds removed from the candidates (round-5 mark-and-sweep · load-bearing)★:
            //   ① a contract already marked for retirement (`pending_retirement`) — if two sends aim at the same victim, one commit
            //      deletes the other's victim first, and the other's rollback/commit points at thin air (the count also goes off).
            //   ② a **provisional contract** (`provisional`) — the failure mode in that field's comment.
            // ADR-0108 (cap retirement — pick the oldest among the retirable)
            // ★"Oldest" is by `created_at`★ — it does not depend on list position. On a tie `min_by_key` gives the
            //   first element, so append order remains as the tie-break.
            let victim = self
                .requests
                .iter_mut()
                .filter(|r| {
                    r.occupies_slot() && !r.provisional && (r.notified || r.reply_by.is_none())
                })
                .min_by_key(|r| r.created_at);
            match victim {
                Some(v) => {
                    v.pending_retirement = true;
                    retired = Some(RetiredContract {
                        request_id: v.request_id.clone(),
                        sender: v.sender.clone(),
                        recipient: v.recipient.clone(),
                        recipient_id: v.recipient_id,
                        age: now.saturating_duration_since(v.created_at),
                    });
                }
                // ★Getting `Full` when only provisional contracts remain is also an honest answer★: at that moment the cap really is full,
                //   and once the racing sender finishes its commit/rollback a slot opens soon.
                None => return OpenOutcome::Full,
            }
        }
        self.requests.push(RequestEntry {
            request_id: request_id.to_string(),
            sender: sender.to_string(),
            sender_id,
            recipient: recipient.to_string(),
            recipient_id,
            reply_by,
            created_at: now,
            closed: false,
            reply_failed: false,
            notified: false,
            pending_retirement: false,
            provisional: true,
            marked_victim: retired
                .as_ref()
                .map(|r: &RetiredContract| (r.request_id.clone(), r.recipient_id)),
            reservation_token: None,
        });
        match retired {
            Some(r) => OpenOutcome::OpenedAfterMarking(r),
            None => OpenOutcome::Opened,
        }
    }

    /// ★Attach the **owner liveness token** to the provisional reservation just opened (R1)★ — it must be called in the **same lock section** as `open_request`
    /// (if the sweep slips in between, it reads an entry before the guard is attached as abandoned —
    /// `RequestEntry::reservation_token` comment).
    ///
    /// If there is no entry it is a no-op (not in the normal flow — it is an entry just created).
    // R1
    pub fn attach_reservation_liveness(
        &mut self,
        request_id: &str,
        recipient_id: PeerId,
        watch: Weak<()>,
    ) {
        if let Some(e) = self
            .requests
            .iter_mut()
            .find(|r| r.request_id == request_id && r.recipient_id == recipient_id && r.provisional)
        {
            e.reservation_token = Some(watch);
        }
    }

    /// ★Reservation confirm (commit) — **physically remove** the marked victim and clear the provisional mark (round-5 mark-and-sweep)★.
    ///
    /// ★Even if the victim was closed by a reply in the meantime, it is simply removed★: `replied` is a terminal with nothing more to look at, and that
    ///   fact is already in the history record as `Replied` (a tracking entry is only for accounting).
    /// ★If the victim is not found, no-op★: not in the normal flow (a marked entry stays in the list until commit/rollback).
    /// ★Return = **what was actually done** (R2)★ — the caller must not assume the planned retirement happened but look at this value
    /// (the instrumentation pollution rationale in the `CommitOutcome` header).
    // round-5 mark-and-sweep
    pub fn commit_open(
        &mut self,
        provisional: Option<(&str, PeerId)>,
        retired: Option<(&str, PeerId)>,
    ) -> CommitOutcome {
        let mut out = CommitOutcome {
            confirmed: false,
            retired: false,
        };
        if let Some((id, recipient_id)) = provisional {
            if let Some(e) = self
                .requests
                .iter_mut()
                .find(|r| r.request_id == id && r.recipient_id == recipient_id)
            {
                e.provisional = false;
                // Confirmed, so the mark memory and liveness token are no longer needed (the victim is physically removed below).
                e.marked_victim = None;
                e.reservation_token = None;
                out.confirmed = true;
            }
        }
        if let Some((id, recipient_id)) = retired {
            let before = self.requests.len();
            self.requests.retain(|r| {
                !(r.request_id == id && r.recipient_id == recipient_id && r.pending_retirement)
            });
            // ★Decide **before** purge (R2)★: purge also deletes entries, so comparing lengths afterwards would mix "what I retired"
            //   with "what vanished by zombie cleanup".
            out.retired = self.requests.len() < before;
            self.purge_finished_without_history();
        }
        out
    }

    /// ★Reservation cancel (rollback) — clear only the mark and remove the provisional contract (round-5 mark-and-sweep)★. Return = the result of removing the provisional
    /// contract (the caller logs outside the lock).
    ///
    /// ★The point of this design is that there is no state to revert★: the victim never left the list, so clearing a single mark bit
    ///   gives the **state where nothing happened** — the old design's re-insert (restore position/age) machinery is gone entirely, and with it
    ///   the failure mode of the "we restored it and someone had already used the slot" kind.
    /// ★Even if the victim was closed by a reply in the meantime, only the mark is cleared★: the close stays as is (it was a legitimate reply) —
    ///   no ghost reopening, no late false deadline notice (`due_timeouts` skips `closed`).
    /// ★Both in one lock section★: the caller finishes with this single call, so there is no window in which the contract count looks over the cap from outside
    ///   (the root of the 513 drift the old split-lock wiring produced).
    /// ★Known residual — id ABA (round-6 review note · no machinery added)★: if a **retirement-marked** victim is closed by a reply in the meantime
    ///   and its own history row is also pushed out of the ring, `purge_finished_without_history` may delete that entry
    ///   (the provisional-contract side is **excluded** from that cleanup, so this residual does not apply to it — see that function's comment).
    ///   Then that id becomes issuable again, and if before the guard settles a new contract is born with **the same random id**
    ///   and happens to also get a retirement mark, the mark release here clears the mark of that **new contract** instead. The probability is
    ///   the product of drawing the same value in a microsecond-to-millisecond window in a 36^8 (2.8×10^12) space and that being picked as a victim again,
    ///   and the damage is bounded to one retirement cancelled (cap pressure released one time less — the next send marks again). To prevent it
    ///   the id reservation set would have to be revived (machinery deleted in round-4), and we judged that complexity not worth this probability —
    ///   **it is a residual left with a price put on it, not something missed**.
    // round-5 mark-and-sweep
    pub fn rollback_open(
        &mut self,
        provisional: Option<(&str, PeerId)>,
        retired: Option<(&str, PeerId)>,
    ) -> Option<DropOutcome> {
        if let Some((id, recipient_id)) = retired {
            if let Some(e) = self.requests.iter_mut().find(|r| {
                r.request_id == id && r.recipient_id == recipient_id && r.pending_retirement
            }) {
                e.pending_retirement = false;
            }
        }
        provisional.map(|(id, recipient_id)| self.drop_request(id, recipient_id))
    }

    /// ★Why every send looks at this (C3 review fix 12 · load-bearing)★: previously only **request sends** caught id collisions
    ///   (DuplicateId of `open_request`). But the id is both the correlation key of history records and the reply matching key, so
    ///   if a notify/reply overlaps an existing id, (a) `records_for` and `transition` pick someone else's record and (b) the observation record
    ///   smears two messages into one id — equally harmful even if it is not a request. So the same check is made at the reservation point regardless of
    ///   kind.
    /// ★Cost (basis for the choice)★: a linear scan of the ring buffer (≤ HISTORY_CAPACITY) + a linear scan of tracking. A separate id set would need
    ///   two data structures synchronized on every evict/close/remove (mismatch = a silent false positive), and the message rate is at
    ///   human-conversation level so scan cost is meaningless — we chose simplicity (revisit with an index at v2 persistence).
    pub fn msg_id_in_use(&self, msg_id: &str) -> bool {
        self.history.iter().any(|r| r.msg_id == msg_id)
            // ★Both retirement-marked contracts and provisional contracts **are right here**(round-5 mark-and-sweep)★ —
            //   since physical removal was dropped, the ordinary tracking lookup is enough without a separate reservation set (the old `reserved_ids`).
            || self.requests.iter().any(|r| r.request_id == msg_id)
    }

    /// ★Use (C3 review fix 5 — narrowing the timeout↔reply race)★: after collecting with `due_timeouts`, the upper layer re-checks just before
    ///   parking the notice — if a reply arrived in the meantime and closed the contract, no "no reply" notice is
    ///   sent. It is intentional that a nonexistent id gives false: when tracking is gone due to evict etc., the timeout
    ///   really did happen, so there is no reason to block the notice.
    /// ★Residual (interaction with fix 1 — stated honestly)★: a contract whose history was already evicted is **removed from tracking at the moment
    ///   it closes** (zombie prevention), so this lookup is false after that — i.e. the "reply arrived after computing" cancel does not
    ///   catch that narrow case and the notice can go out once more. It is a path that holds only in a microsecond window after the history capacity (HISTORY_CAPACITY) has been pushed out, so
    ///   here we chose the bound (zombie removal).
    pub fn is_request_closed(&self, request_id: &str, recipient_id: PeerId) -> bool {
        self.requests
            .iter()
            .any(|r| r.request_id == request_id && r.recipient_id == recipient_id && r.closed)
    }

    /// ★Marks (retirement-marked · provisional) do not obscure matching (round-5 mark-and-sweep · load-bearing)★: the two marks are
    ///   **for accounting**, not about existence. Here only `!closed` is looked at, so ① a legitimate reply to a contract marked for retirement
    ///   also closes normally (in the old physical-removal design this missed with `NoMatch` — the sender got an answer but
    ///   the contract did not close and a false deadline notice could even go out later) ② a fast reply to a still-unconfirmed provisional contract
    ///   also closes. Neither the later commit (removal) nor rollback (mark clear) reverses that fact.
    /// ★The contract is picked by the replier (ADR-0111 decision 5)★: under one `request_id` there can be
    ///   **several** contracts (multi-recipient request), so `in_reply_to` alone cannot
    ///   identify which contract to close. Spec §3 says "only **that replier's contract** closes (other recipients' contracts stay open — no
    ///   reply-all)", so the matching key is `(request_id, 회신자 id)`.
    /// ★"Not my contract" = `NoMatch` = **delivery unchanged** (spec §3 item 7-②)★: an unknown id, an already closed contract, or
    ///   a contract I do not have — in every case **only the contract side is a no-op** and the reply message itself is delivered normally. Having the reply
    ///   vanish entirely would be worse.
    // L1
    // ADR-0309 (decision 7 — no name-fallback matching)
    pub fn close_on_reply(
        &mut self,
        in_reply_to: &str,
        replier_id: PeerId,
        now: Instant,
    ) -> ReplyOutcome {
        // Closes the tracking entry (the source of truth). The history row is picked by the same key as the contract, `(msg_id, 수신자 id)` (ADR-0309).
        let matched = self.match_contract_for_replier(in_reply_to, replier_id);
        let recipient_id = match matched.map(|i| &mut self.requests[i]) {
            Some(r) if r.closed => return ReplyOutcome::AlreadyClosed,
            Some(r) => {
                r.closed = true;
                r.recipient_id
            }
            None => return ReplyOutcome::NoMatch,
        };
        let outcome = match self.transition(in_reply_to, recipient_id, DeliveryStatus::Replied, now)
        {
            Ok(()) => ReplyOutcome::Closed,
            Err(TransitionError::NotFound) => ReplyOutcome::Closed,
            Err(TransitionError::Illegal { from, .. }) => {
                ReplyOutcome::ClosedHistoryAnomaly { from }
            }
        };
        self.purge_finished_without_history();
        outcome
    }

    /// ★The **single rule** for picking the replier's contract — `close_on_reply` and `fail_on_undeliverable_reply` use **the same
    /// function** (load-bearing)★.
    ///
    /// ★Why bundle it into a function★: the two verbs are two outcomes of the same single reply (accepted → `replied` / unreachable confirmed →
    ///   `reply_failed`). If the selection rules diverge, **the same reply points at different contracts**, and the contract the success path closed
    ///   and the contract the failure path terminated come apart (a recurrence of the class A6 caught — then too the cause was "one rule written
    ///   separately in two places").
    /// ★Pick by id alone — never fall back to the name (ADR-0309 decision 7)★: a contract carries an id from the moment of opening even when addressed to a sleeping recipient.
    ///   If name fallback were revived, same-name twins · another agent that newly took an old name would close someone else's contract.
    // ADR-0309
    fn match_contract_for_replier(&self, in_reply_to: &str, replier_id: PeerId) -> Option<usize> {
        self.requests
            .iter()
            .position(|r| r.request_id == in_reply_to && r.recipient_id == replier_id)
    }

    /// ★The reply is **confirmed unreachable**, so the contract is closed as failed (spec §3 item 7-④ · ADR-0116 decision 2 · ADR-0118)★ —
    /// it is called only when the reply send row is `RECIPIENT_NOT_FOUND` (= the requester name is **in neither the roster nor the profiles**)
    /// and in no other case.
    ///
    /// ★No other reply failure calls this verb (load-bearing)★: `MAILBOX_FULL` · `RECIPIENT_AMBIGUOUS`
    ///   are **the environment at that moment**, so a retry can actually succeed in delivering — if the
    ///   ledger then stayed "reply failed" forever it would be a lie (ADR-0118 decision 2). That kind is a **no-op** (the contract stays
    ///   open), and the reason it is not a zombie is "once the deadline notice goes out it earns ADR-0108 retirement eligibility".
    /// ★Guard first (ADR-0118 decision 3 — respecting the ADR-0108 decision 4 invariant)★: **contracts carrying a provisional (`provisional`) or retirement-marked
    ///   (`pending_retirement`) mark are not touched** (`GuardHeld`). The lifetime of an unsettled entry is owned by that guard,
    ///   so closing here would ① let the guard's commit/rollback revive an already terminated contract (ghost reopening) or
    ///   ② let someone else terminate the marked victim, halving the cap exchange. A failed reply arriving in between is a no-op.
    /// ★Delivery state is not touched (spec §6 axis distinction)★: `reply_failed` is a **contract-axis** terminal — the original request's
    ///   delivery record stays at `delivered` (or `pending`). Transitioning history here would make the history
    ///   "delivered, then turned back into failure" expressible.
    // ADR-0116 (decision 2) / ADR-0118 (decisions 1·2·3)
    pub fn fail_on_undeliverable_reply(
        &mut self,
        in_reply_to: &str,
        replier_id: PeerId,
    ) -> ReplyFailOutcome {
        let Some(idx) = self.match_contract_for_replier(in_reply_to, replier_id) else {
            return ReplyFailOutcome::NoMatch;
        };
        let entry = &mut self.requests[idx];
        if entry.closed {
            return ReplyFailOutcome::AlreadyClosed;
        }
        if entry.provisional || entry.pending_retirement {
            return ReplyFailOutcome::GuardHeld;
        }
        entry.closed = true;
        entry.reply_failed = true;
        self.purge_finished_without_history();
        ReplyFailOutcome::Failed
    }

    /// ★Requester profile deletion cleanup — closes open contracts where that id is the **requester** as `reply_failed`★
    /// (spec §5 deletion cleanup ② · ADR-0116 decision 3 · ADR-0309 decision 6).
    ///
    /// ★Target = the requester (`sender`) side only★: the place the reply would go is gone, so the outcome of that contract is settled. **Contracts where the replier
    ///   (`recipient`) side was deleted are kept** — the existing path where the sender learns of no response through the deadline notice (spec §3 items 4~5)
    ///   is alive, and closing that contract here would erase the fact "no answer was received".
    /// ★Already closed contracts are not touched★: `replied` is **not reverted** (spec §3 item 7-④ "acceptance =
    ///   complete" — even if a parked reply is later cleared as `RECIPIENT_DELETED`, the contract stays `replied`).
    ///   The `!closed` filter guarantees that rule structurally.
    /// ★Guard first (ADR-0118 decision 3)★: provisional and retirement-marked contracts are skipped and their count is returned along with it (there is **no** re-fire
    ///   after settlement — deletion cleanup is a one shot at deletion time, and the outcome of that residual is TTL. spec §5).
    /// ★Match by `sender_id` — do not look at the name (`sender`) (ADR-0309 decision 6)★: `sender` is the display name at send
    ///   time, so picking by name would miss a contract opened under the pre-rename name and also close the contract of another agent with the same name. The trigger gate
    ///   (that profile has no live session) is decided first by the caller with the same id
    ///   (`service::handle_profile_deleted`).
    // ADR-0116 (decision 3) / ADR-0118 (decision 3) / ADR-0309 (decision 6)
    pub fn fail_open_requests_from(&mut self, sender_id: PeerId) -> RequesterCleanup {
        let mut out = RequesterCleanup::default();
        for r in self.requests.iter_mut() {
            if r.closed || r.sender_id != sender_id {
                continue;
            }
            if r.provisional || r.pending_retirement {
                out.guard_held += 1;
                continue;
            }
            r.closed = true;
            r.reply_failed = true;
            out.failed.push((r.request_id.clone(), r.recipient_id));
        }
        if !out.failed.is_empty() {
            self.purge_finished_without_history();
        }
        out
    }

    /// ★**Removes** an open request tracking entry **entirely** (C3 — when the send was bounced and the contract was never established)★.
    ///
    /// ★Why a separate exit rather than `close_on_reply` (load-bearing — bounded guarantee)★: closing (`closed=true`) is
    ///   the **history** of "a reply came and the contract was fulfilled", so it stays in the tracking list. That leftover entry is dropped together when the same msg_id's
    ///   **history record is evicted**, which keeps the bound (`record` comment). But a **bounced send**
    ///   never had a history record to begin with (it was not even parked) — so with only closing there is no trigger to ever evict that entry,
    ///   and the more bounces repeat, the tracking list grows without bound. A bounce is not "the contract was fulfilled" but "the contract was never
    ///   established", so erasing it with no trace and leaving no history is also right semantically.
    pub fn drop_request(&mut self, request_id: &str, recipient_id: PeerId) -> DropOutcome {
        let Some(idx) = self
            .requests
            .iter()
            .position(|r| r.request_id == request_id && r.recipient_id == recipient_id)
        else {
            return DropOutcome::NotFound;
        };
        let removed = self.requests.remove(idx);
        DropOutcome::Removed {
            notified: removed.notified,
        }
    }

    /// ★Reclaims **abandoned** reservations (F1 guarantee layer · R1 criterion replaced)★: rolls back whole the **provisional** contracts whose owner guard is gone
    /// (remove the provisional contract + clear the victim mark that reservation attached). Returns the reclaimed contract keys.
    ///
    /// ★Why this is the **guarantee** (RAII Drop is not a guarantee)★: `Reservation::drop` rolls back only when `try_lock` succeeds —
    ///   under lock contention or "the same thread dropping while holding the state lock" it **can do nothing**. The wreckage left then
    ///   has a permanent price: ① a provisional contract is skipped by `due_timeouts` forever (deadline notice lost) ② the marked
    ///   victim drops out of `occupies_slot()` and the cap denominator shrinks permanently ③ the tracking list grows without bound.
    ///   So the **periodic sweep** (the only maintenance point that properly owns the lock) does the same job again —
    ///   Drop is the fast path, and this is the guarantee.
    /// ★Idempotent★: a settled entry is not provisional (commit = mark cleared · rollback = removed), so a second call does
    ///   nothing.
    /// ★Reclaim is not destructive★: a provisional contract is either a reservation that has **not yet reported acceptance to the sender** (a case where the
    ///   response vanished through a panic) or one already reported as a failure row. Either way, rolling it back is right.
    // ADR-0108 (mark-and-sweep — guarantee layer of reservation reclaim)
    pub fn reclaim_abandoned_reservations(&mut self) -> Vec<(String, PeerId)> {
        let abandoned: Vec<(String, PeerId, Option<(String, PeerId)>)> = self
            .requests
            .iter()
            .filter(|r| {
                r.provisional
                    && r.reservation_token
                        .as_ref()
                        .is_none_or(|w| w.upgrade().is_none())
            })
            .map(|r| {
                (
                    r.request_id.clone(),
                    r.recipient_id,
                    r.marked_victim.clone(),
                )
            })
            .collect();
        let mut reclaimed = Vec::with_capacity(abandoned.len());
        for (id, recipient_id, victim) in abandoned {
            // Mark clear + provisional contract removal = **the same rule** as rollback (the same verb is used so the two places do not diverge).
            if let Some((vid, vrecipient_id)) = &victim {
                if let Some(v) = self
                    .requests
                    .iter_mut()
                    .find(|r| &r.request_id == vid && r.recipient_id == *vrecipient_id)
                {
                    v.pending_retirement = false;
                }
            }
            self.drop_request(&id, recipient_id);
            reclaimed.push((id, recipient_id));
        }
        reclaimed
    }

    /// ★Boundary★: it is a `>` comparison, so the exact deadline instant is not yet due (the same convention as the mailbox TTL boundary — deterministic tests).
    /// ★Retirement-marked contracts are not skipped, but **provisional contracts are skipped** (branches split in round-5 → round-6 I2)★:
    ///   - **A retirement-marked contract can never be due in the first place** (structural, kept as is): victim eligibility is
    ///     `notified || reply_by.is_none()` and due eligibility is `!notified && reply_by.is_some()`, so the two sets are
    ///     **disjoint**. So adding a mark check would be a dead branch — leaving it out is more honest.
    ///   - ★**Provisional contracts are explicitly skipped** (round-6 I2 · load-bearing)★. The provisional window covers **from reservation to the
    ///     settlement of that recipient's outcome** (service.rs `PendingContract` — the A2 round pushed commit back to after the outcome),
    ///     and in between sits the child stdin `write_all` — as our `stdio.rs` documents itself, under pipe backpressure that
    ///     write **can block indefinitely**. So a request with a 1-minute deadline could be caught by the sweep inside the provisional window and a notice
    ///     could go out, and if the send is then bounced, the sender holds a deadline-overrun notice about a **request that officially never
    ///     existed** (the one that got a bounce). A notice cannot be recalled, so it cannot be undone either.
    ///   - ★No loss (hand-off)★: the skipped contract's `created_at` is **the original, unchanged**. Once committed, the next sweep
    ///     (60-second period) sees the already-passed deadline and notifies **immediately** — delayed but not lost. If rolled back,
    ///     the contract itself becomes something that never was, so there should be no notice either. It is the only arrangement where both are right.
    // ADR-0108 (provisional skip — after commit the next sweep notifies at the original time)
    pub fn due_timeouts(&mut self, now: Instant) -> Vec<DueTimeout> {
        let mut due = Vec::new();
        for r in self.requests.iter_mut() {
            if r.closed || r.notified || r.provisional {
                continue;
            }
            let Some((reply_by, reply_by_raw)) = r.reply_by.clone() else {
                continue;
            };
            let deadline = r.created_at + reply_by;
            if now > deadline {
                r.notified = true;
                due.push(DueTimeout {
                    request_id: r.request_id.clone(),
                    sender: r.sender.clone(),
                    sender_id: r.sender_id,
                    recipient: r.recipient.clone(),
                    recipient_id: r.recipient_id,
                    reply_by_raw,
                });
            }
        }
        if !due.is_empty() {
            self.purge_finished_without_history();
        }
        due
    }

    /// History record count (observation/tests).
    pub fn history_len(&self) -> usize {
        self.history.len()
    }

    /// Oldest first.
    pub fn records_for(&self, msg_id: &str) -> Vec<&MessageRecord> {
        self.history.iter().filter(|r| r.msg_id == msg_id).collect()
    }

    /// ★`records_for` + **incompleteness signal** (D review B2 · decision replaced by round-2 review F3)★ — `(행 목록, truncated)`.
    ///
    /// ★Why it is needed★: when the ring (4096) fills up, the front rows silently disappear. But the `messages { id }` response shows the remaining
    ///   rows as if they were **all of that message** — if 6 rows of a 10-member broadcast are pushed out, the sender misreads it as "it went to only 4
    ///   people" (in fact it went to all 10 and only the records vanished).
    /// ★Decision = `남은 행 수 < 기대 행 수` (deterministic)★ — `MessageRecord.expected_rows`.
    /// ★The case where rows vanished **entirely** is not visible here★: that goes out as an empty list and the upper layer answers with the contract view or
    ///   `MESSAGE_NOT_FOUND` (whose hint announces the history rotation).
    // review B2 / round-2 review F3
    pub fn records_for_detailed(&self, msg_id: &str) -> (Vec<&MessageRecord>, bool) {
        let rows: Vec<&MessageRecord> =
            self.history.iter().filter(|r| r.msg_id == msg_id).collect();
        let truncated = rows
            .first()
            .is_some_and(|r| rows.len() < usize::from(r.expected_rows));
        (rows, truncated)
    }

    pub fn history_evicted(&self) -> bool {
        self.evicted_any
    }

    /// All history records (oldest first) — an observation/test snapshot. Used by the upper layer (MessagingService) for assertions that do not know msg_id, like "did the
    /// notice stay in the ledger" (lookups that know msg_id use `records_for`).
    pub fn all_records(&self) -> Vec<&MessageRecord> {
        self.history.iter().collect()
    }

    /// ★Number of contracts currently occupying a `MAX_OPEN_REQUESTS` slot (round-6 I1)★ — exactly the value the cap decision looks at.
    ///
    /// It is **different** from `open_request_count` (= number of unreplied contracts): retirement-marked contracts drop out, and provisional contracts
    /// before settlement remain even if closed (`occupies_slot` comment). To assert the cap arithmetic, look at this one.
    pub fn occupied_slots(&self) -> usize {
        self.requests.iter().filter(|r| r.occupies_slot()).count()
    }

    /// Test-only (H2 fixture) — closes that contract without a reply (emptying a slot — a fixture-only shortcut).
    #[cfg(any(test, feature = "test-harness"))]
    pub fn close_for_test(&mut self, request_id: &str, recipient_id: PeerId, now: Instant) {
        if let Some(r) = self
            .requests
            .iter_mut()
            .find(|r| r.request_id == request_id && r.recipient_id == recipient_id)
        {
            r.closed = true;
        }
        let _ = now;
    }

    /// Test-only (H2) — **number of contracts carrying a retirement mark**. Marks drop out of `occupies_slot()`, so if this value
    ///   remains non-zero, the cap denominator shrinks permanently (the observation axis of a forgotten settlement — `service::Reservation` header).
    ///   ★It does not duplicate the victim selection logic★: it counts only "is a mark left", not "who was marked" (no drift).
    #[cfg(any(test, feature = "test-harness"))]
    pub fn marked_retirement_count_for_test(&self) -> usize {
        self.requests
            .iter()
            .filter(|r| r.pending_retirement)
            .count()
    }

    /// ★Observe the contract-axis terminal vocabulary (amendment 4 — spec §6 `awaiting_reply`/`replied`/`reply_failed`)★. If not in tracking,
    ///   `None` (only history remains, or an unknown key).
    ///
    /// ★Why it is needed★: `replied` and `reply_failed` drop out of the open list, deadline sweep and cap count **identically**, so
    ///   they cannot be told apart on that axis. To assert the rule "acceptance counts as complete, only confirmed unreachable counts as failure" (§3 item 7-④)
    ///   and "deletion cleanup does not revert `replied`", the reason itself must be looked at.
    #[cfg(any(test, feature = "test-harness"))]
    pub fn contract_outcome_for_test(
        &self,
        request_id: &str,
        recipient_id: PeerId,
    ) -> Option<&'static str> {
        self.requests
            .iter()
            .find(|r| r.request_id == request_id && r.recipient_id == recipient_id)
            .map(|r| match (r.closed, r.reply_failed) {
                (true, true) => "reply_failed",
                (true, false) => "replied",
                (false, _) => "awaiting_reply",
            })
    }

    /// Test-only (C2) — does that contract key exist in the tracking list (closed or retirement-marked alike). The observation axis of the provisional/retirement window.
    #[cfg(any(test, feature = "test-harness"))]
    pub fn is_tracked_for_test(&self, request_id: &str, recipient_id: PeerId) -> bool {
        self.requests
            .iter()
            .any(|r| r.request_id == request_id && r.recipient_id == recipient_id)
    }

    /// ★Why tests only★: production callers call with **both** parts of the contract key in hand (the send and reply paths already know the recipient)
    ///   — this reverse lookup is just a convenience to fit tests from the old "one-id contract" days to the new key.
    #[cfg(test)]
    fn party_of(&self, request_id: &str) -> Option<PeerId> {
        self.requests
            .iter()
            .find(|r| r.request_id == request_id)
            .map(|r| r.recipient_id)
    }

    /// Number of open (unreplied) requests (observation/tests).
    pub fn open_request_count(&self) -> usize {
        self.requests.iter().filter(|r| !r.closed).count()
    }

    /// **Total number** of tracking entries (including finished ones — observation/tests). Used to assert the bound that no zombies accumulate (fix 1):
    ///   `open_request_count` does not count finished entries, so it cannot see a leak.
    pub fn tracking_len(&self) -> usize {
        self.requests.len()
    }

    /// ★All unreplied (open) requests as lookup views (S18 D — `eg_messages` no-argument)★. **Oldest first** (`created_at`
    /// ascending, on a tie the current list order — stable sort).
    ///
    /// ★Why sort explicitly (round-4 review H4)★: previously the raw Vec order was returned as is on the grounds "append order = send order", but that premise is
    ///   an **assumption** true only when the caller uses a monotonic clock, not a property this data structure enforces
    ///   (the clock is injected — module header purity invariant). When the order the docs
    ///   promise and the actual order diverge, lookup consumers silently see a misordered list (and that misorder appears only on the rare path where a restore
    ///   happened, so it is hard to reproduce). With ≤512 entries the cost of sorting on every lookup is negligible, so the promise is kept
    ///   in code.
    /// ★Not a double sort★: the upper `open_items_for` re-sorts by descending elapsed time **after merging** the three branches, so it
    ///   does not depend on the order here (same result). This sort is to keep this function's contract.
    /// ★Marked contracts are **visible** too (round-5 mark-and-sweep — an explicit choice)★:
    ///   - **Retirement mark**: until commit it is still an open contract. If a lookup lands in that microsecond window it
    ///     shows in the list, and that is the **fact** (nothing has retired yet). Hiding it early would be worse, as the lookup would
    ///     report a removal that may never commit.
    ///   - **Provisional contract**: it is an intake that really exists, so showing it is right. If it ends in a bounce it disappears then.
    ///
    /// ★Inclusion criterion = `!closed` (= `is_live()`) — load-bearing★: whether it was notified is **exposed as a field** and
    ///   the entry is not excluded from the list. **If the criterion here changes, `is_live()` must change with it** (if they diverge the same bug returns —
    ///   the 4-step sequence in the `is_live` comment).
    /// ★The upper layer filters★: the sent/received branch is decided by the caller.
    // ADR-0103 (spec §6 eg_messages no-argument = my pending)
    pub fn open_requests(&self) -> Vec<OpenRequestView> {
        let mut out: Vec<OpenRequestView> = self
            .requests
            .iter()
            .filter(|r| !r.closed)
            .map(|r| OpenRequestView {
                request_id: r.request_id.clone(),
                sender: r.sender.clone(),
                sender_id: r.sender_id,
                recipient: r.recipient.clone(),
                recipient_id: r.recipient_id,
                reply_by_raw: r.reply_by.as_ref().map(|(_, raw)| raw.clone()),
                created_at: r.created_at,
                notified: r.notified,
            })
            .collect();
        out.sort_by_key(|r| r.created_at);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tid;

    /// An adapter that puts the old "one id = one contract" tests on the contract key (ADR-0111 decision 5) — interpreted as
    /// **that contract's recipient replied in person**.
    fn reply(l: &mut Ledger, id: &str, now: Instant) -> ReplyOutcome {
        let replier = l.party_of(id).unwrap_or_else(PeerId::nil);
        l.close_on_reply(id, replier, now)
    }

    fn drop_req(l: &mut Ledger, id: &str) -> DropOutcome {
        let party = l.party_of(id).unwrap_or_else(PeerId::nil);
        l.drop_request(id, party)
    }

    fn closed(l: &Ledger, id: &str) -> bool {
        l.party_of(id)
            .is_some_and(|party| l.is_request_closed(id, party))
    }

    fn commit(l: &mut Ledger, provisional: Option<&str>, retired: Option<&str>) {
        let pv = provisional.and_then(|id| l.party_of(id).map(|p| (id, p)));
        let rt = retired.and_then(|id| l.party_of(id).map(|p| (id, p)));
        l.commit_open(pv, rt);
    }

    fn rollback(
        l: &mut Ledger,
        provisional: Option<&str>,
        retired: Option<&str>,
    ) -> Option<DropOutcome> {
        let pv = provisional.and_then(|id| l.party_of(id).map(|p| (id, p)));
        let rt = retired.and_then(|id| l.party_of(id).map(|p| (id, p)));
        l.rollback_open(pv, rt)
    }

    fn t0() -> Instant {
        Instant::now()
    }

    /// Sender PeerId (fix 2) — most assertions do not look at the value itself, so a fresh one is drawn each time.
    fn sid() -> PeerId {
        PeerId::new_v4()
    }

    /// Deadline tuple (fix 6) — to keep the premise in tests too that the spelling was **written by the sender**, not made from a Duration,
    /// a conventional spelling is used where the assertion does not look at the spelling.
    fn rb(d: Duration) -> Option<(Duration, String)> {
        Some((d, format!("{}s", d.as_secs())))
    }

    /// A cap fixture that forgot to commit stays entirely provisional and yields only `Full` — the production send path
    /// always commits/rolls back when the outcome is settled (`service::commit_contract`), so the tests create the same state.
    fn open_committed(
        l: &mut Ledger,
        id: &str,
        reply_by: Option<(Duration, String)>,
        now: Instant,
    ) -> OpenOutcome {
        let out = l.open_request(id, "alice", tid("alice"), "bob", tid("bob"), reply_by, now);
        commit(l, Some(id), None);
        out
    }

    /// A contract with no history exists only after evict — tests not aiming at that case use this helper to create history alongside
    /// and reproduce the production state.
    fn open_delivered_request(
        l: &mut Ledger,
        id: &str,
        reply_by: Option<(Duration, String)>,
        now: Instant,
    ) {
        l.record(
            id,
            "alice",
            tid("alice"),
            "bob",
            Some(tid("bob")),
            "q",
            DeliveryStatus::Pending,
            now,
        );
        l.open_request(id, "alice", sid(), "bob", tid("bob"), reply_by, now);
        commit(l, Some(id), None);
        assert_eq!(
            l.transition(id, tid("bob"), DeliveryStatus::Delivered, now),
            Ok(()),
            "전제: 주입까지 끝난 계약"
        );
    }

    // ── History ring buffer ──────────────────────────────────────────────────────
    #[test]
    fn record_appends_and_reports_status() {
        let mut l = Ledger::new();
        let now = t0();
        l.record(
            "m1",
            "alice",
            tid("alice"),
            "bob",
            Some(tid("bob")),
            "hi",
            DeliveryStatus::Pending,
            now,
        );
        let recs = l.records_for("m1");
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0].status, DeliveryStatus::Pending);
        assert_eq!(recs[0].from, "alice");
        assert_eq!(recs[0].to, "bob");
    }

    #[test]
    fn ring_buffer_evicts_oldest_at_capacity() {
        let mut l = Ledger::with_capacity(3);
        let now = t0();
        for i in 0..5 {
            l.record(
                &format!("m{i}"),
                "a",
                tid("a"),
                "b",
                Some(tid("b")),
                "x",
                DeliveryStatus::Delivered,
                now,
            );
        }
        assert_eq!(l.history_len(), 3, "용량 상한 유지");
        assert!(l.records_for("m0").is_empty(), "가장 오래된 것부터 evict");
        assert!(l.records_for("m1").is_empty());
        assert_eq!(l.records_for("m4").len(), 1, "최근 것은 잔존");
    }

    #[test]
    fn transition_records_timestamp_and_status() {
        let mut l = Ledger::new();
        let now = t0();
        l.record(
            "m1",
            "a",
            tid("a"),
            "b",
            Some(tid("b")),
            "x",
            DeliveryStatus::Pending,
            now,
        );
        let later = now + Duration::from_secs(5);
        assert_eq!(
            l.transition("m1", tid("b"), DeliveryStatus::Delivered, later),
            Ok(())
        );
        let rec = l.records_for("m1")[0];
        assert_eq!(rec.status, DeliveryStatus::Delivered);
        assert_eq!(rec.transitioned_at, later, "전이 시각 기록");
        assert_eq!(rec.created_at, now, "발신 시각은 불변");
    }

    #[test]
    fn transition_targets_recipient_for_group_broadcast() {
        let mut l = Ledger::new();
        let now = t0();
        l.record(
            "g1",
            "boss",
            tid("boss"),
            "a",
            Some(tid("a")),
            "rebase",
            DeliveryStatus::Pending,
            now,
        );
        l.record(
            "g1",
            "boss",
            tid("boss"),
            "b",
            Some(tid("b")),
            "rebase",
            DeliveryStatus::Pending,
            now,
        );
        l.record(
            "g1",
            "boss",
            tid("boss"),
            "c",
            Some(tid("c")),
            "rebase",
            DeliveryStatus::Skipped,
            now,
        );
        let later = now + Duration::from_secs(1);
        assert_eq!(
            l.transition("g1", tid("a"), DeliveryStatus::Delivered, later),
            Ok(())
        );
        let recs = l.records_for("g1");
        assert_eq!(recs.len(), 3, "한 msg_id 에 배달기록 3개");
        let a = recs.iter().find(|r| r.to == "a").unwrap();
        let b = recs.iter().find(|r| r.to == "b").unwrap();
        let c = recs.iter().find(|r| r.to == "c").unwrap();
        assert_eq!(a.status, DeliveryStatus::Delivered);
        assert_eq!(b.status, DeliveryStatus::Pending, "b 는 안 건드려짐");
        assert_eq!(c.status, DeliveryStatus::Skipped, "죽은 멤버 skipped");
    }

    #[test]
    fn transition_missing_record_returns_not_found() {
        let mut l = Ledger::new();
        assert_eq!(
            l.transition("nope", tid("b"), DeliveryStatus::Delivered, t0()),
            Err(TransitionError::NotFound)
        );
    }

    #[test]
    fn transition_rejects_illegal_edges() {
        let mut l = Ledger::new();
        let now = t0();

        l.record(
            "e1",
            "a",
            tid("a"),
            "b",
            Some(tid("b")),
            "x",
            DeliveryStatus::Pending,
            now,
        );
        assert_eq!(
            l.transition("e1", tid("b"), DeliveryStatus::Expired, now),
            Ok(()),
            "Pending → Expired 는 합법"
        );
        assert_eq!(
            l.transition(
                "e1",
                tid("b"),
                DeliveryStatus::Delivered,
                now + Duration::from_secs(1)
            ),
            Err(TransitionError::Illegal {
                from: DeliveryStatus::Expired,
                to: DeliveryStatus::Delivered
            }),
            "Expired → Delivered 는 불법"
        );
        assert_eq!(
            l.records_for("e1")[0].status,
            DeliveryStatus::Expired,
            "불법 전이는 상태를 안 바꿈"
        );

        l.record(
            "p1",
            "a",
            tid("a"),
            "b",
            Some(tid("b")),
            "x",
            DeliveryStatus::Pending,
            now,
        );
        assert_eq!(
            l.transition("p1", tid("b"), DeliveryStatus::Replied, now),
            Err(TransitionError::Illegal {
                from: DeliveryStatus::Pending,
                to: DeliveryStatus::Replied
            }),
            "Pending → Replied 는 불법(Delivered 경유 필요)"
        );

        l.record(
            "r1",
            "a",
            tid("a"),
            "b",
            Some(tid("b")),
            "x",
            DeliveryStatus::Pending,
            now,
        );
        assert_eq!(
            l.transition("r1", tid("b"), DeliveryStatus::Delivered, now),
            Ok(())
        );
        assert_eq!(
            l.transition("r1", tid("b"), DeliveryStatus::Replied, now),
            Ok(())
        );
        assert_eq!(
            l.transition("r1", tid("b"), DeliveryStatus::Pending, now),
            Err(TransitionError::Illegal {
                from: DeliveryStatus::Replied,
                to: DeliveryStatus::Pending
            }),
            "Replied → Pending 은 불법(terminal 되돌림)"
        );

        l.record(
            "s1",
            "a",
            tid("a"),
            "b",
            Some(tid("b")),
            "x",
            DeliveryStatus::Skipped,
            now,
        );
        assert_eq!(
            l.transition("s1", tid("b"), DeliveryStatus::Pending, now),
            Err(TransitionError::Illegal {
                from: DeliveryStatus::Skipped,
                to: DeliveryStatus::Pending
            }),
            "Skipped → Pending 은 불법"
        );
    }

    #[test]
    fn transition_accepts_legal_edges() {
        let now = t0();
        let mut l = Ledger::new();
        l.record(
            "m",
            "a",
            tid("a"),
            "b",
            Some(tid("b")),
            "x",
            DeliveryStatus::Pending,
            now,
        );
        assert_eq!(
            l.transition("m", tid("b"), DeliveryStatus::Delivered, now),
            Ok(())
        );
        assert_eq!(
            l.transition("m", tid("b"), DeliveryStatus::Replied, now),
            Ok(())
        );
        let mut l2 = Ledger::new();
        l2.record(
            "a",
            "x",
            tid("x"),
            "y",
            Some(tid("y")),
            "b",
            DeliveryStatus::Pending,
            now,
        );
        assert_eq!(
            l2.transition("a", tid("y"), DeliveryStatus::Skipped, now),
            Ok(())
        );
        l2.record(
            "c",
            "x",
            tid("x"),
            "y",
            Some(tid("y")),
            "b",
            DeliveryStatus::Pending,
            now,
        );
        assert_eq!(
            l2.transition("c", tid("y"), DeliveryStatus::Delivered, now),
            Ok(())
        );
        assert_eq!(
            l2.transition("c", tid("y"), DeliveryStatus::Skipped, now),
            Ok(())
        );
        l2.record(
            "d",
            "x",
            tid("x"),
            "y",
            Some(tid("y")),
            "b",
            DeliveryStatus::Pending,
            now,
        );
        assert_eq!(
            l2.transition("d", tid("y"), DeliveryStatus::Expired, now),
            Ok(())
        );
    }

    // ── Request strict reply matching ────────────────────────────────────────────
    #[test]
    fn strict_reply_closes_exact_match() {
        let mut l = Ledger::new();
        let now = t0();
        assert_eq!(
            l.open_request("req-1", "alice", sid(), "bob", tid("bob"), None, now),
            OpenOutcome::Opened
        );
        assert_eq!(l.open_request_count(), 1);
        assert_eq!(reply(&mut l, "req-1", now), ReplyOutcome::Closed);
        assert_eq!(l.open_request_count(), 0, "회신으로 닫힘");
    }

    #[test]
    fn strict_reply_wrong_id_does_not_close() {
        let mut l = Ledger::new();
        let now = t0();
        l.open_request("req-1", "alice", sid(), "bob", tid("bob"), None, now);
        assert_eq!(reply(&mut l, "req-999", now), ReplyOutcome::NoMatch);
        assert_eq!(l.open_request_count(), 1, "틀린 id 는 request 를 안 닫아야");
    }

    #[test]
    fn second_reply_to_same_request_is_already_closed_noop() {
        let mut l = Ledger::new();
        let now = t0();
        open_delivered_request(&mut l, "req-1", None, now);
        assert_eq!(reply(&mut l, "req-1", now), ReplyOutcome::Closed);
        assert_eq!(reply(&mut l, "req-1", now), ReplyOutcome::AlreadyClosed);
        assert_eq!(l.open_request_count(), 0);
    }

    #[test]
    fn duplicate_open_request_id_is_rejected() {
        let mut l = Ledger::new();
        let now = t0();
        assert_eq!(
            l.open_request("req-1", "alice", sid(), "bob", tid("bob"), None, now),
            OpenOutcome::Opened
        );
        assert_eq!(
            l.open_request("req-1", "alice", sid(), "carol", tid("carol"), None, now),
            OpenOutcome::Opened,
            "같은 메시지 id 의 **다른 수신자** = 독립 계약(다중 수신자 request)"
        );
        assert_eq!(
            l.open_request("req-1", "alice", sid(), "bob", tid("bob"), None, now),
            OpenOutcome::DuplicateId,
            "같은 계약 키(id, 수신자)의 재오픈은 거부"
        );
        assert_eq!(
            l.open_request_count(),
            2,
            "열린 계약 = (req-1,bob)·(req-1,carol) 둘뿐 — 중복 키는 추적에 추가 안 됨"
        );
    }

    #[test]
    fn closed_id_cannot_be_reopened_and_reply_stays_already_closed() {
        let mut l = Ledger::new();
        let now = t0();
        open_delivered_request(&mut l, "req-1", None, now);
        assert_eq!(reply(&mut l, "req-1", now), ReplyOutcome::Closed);
        assert_eq!(
            l.open_request("req-1", "alice", sid(), "bob", tid("bob"), None, now),
            OpenOutcome::DuplicateId,
            "닫힌 id 재오픈은 거부(유일성 전제)"
        );
        assert_eq!(l.open_request_count(), 0, "재오픈 안 됐으니 오픈 0");
        assert_eq!(
            reply(&mut l, "req-1", now),
            ReplyOutcome::AlreadyClosed,
            "재오픈 없이 회신하면 AlreadyClosed(shadowing 없음)"
        );
    }

    #[test]
    fn close_on_reply_transitions_history_to_replied_with_timestamp() {
        let mut l = Ledger::new();
        let now = t0();
        l.record(
            "req-1",
            "alice",
            tid("alice"),
            "bob",
            Some(tid("bob")),
            "질문",
            DeliveryStatus::Pending,
            now,
        );
        l.open_request("req-1", "alice", sid(), "bob", tid("bob"), None, now);
        // Injection (Delivered) — only Delivered → Replied is legal, so it is a required precursor.
        let delivered_at = now + Duration::from_secs(1);
        assert_eq!(
            l.transition("req-1", tid("bob"), DeliveryStatus::Delivered, delivered_at),
            Ok(())
        );
        let reply_at = now + Duration::from_secs(30);
        assert_eq!(reply(&mut l, "req-1", reply_at), ReplyOutcome::Closed);
        let rec = l.records_for("req-1")[0];
        assert_eq!(rec.status, DeliveryStatus::Replied, "이력이 Replied 로");
        assert_eq!(
            rec.transitioned_at, reply_at,
            "전이 시각 = 회신 시각(spec §5)"
        );
    }

    #[test]
    fn close_on_reply_against_pending_history_is_anomaly_but_still_closes() {
        let mut l = Ledger::new();
        let now = t0();
        l.record(
            "req-1",
            "alice",
            tid("alice"),
            "bob",
            Some(tid("bob")),
            "q",
            DeliveryStatus::Pending,
            now,
        );
        l.open_request("req-1", "alice", sid(), "bob", tid("bob"), None, now);
        assert_eq!(
            reply(&mut l, "req-1", now),
            ReplyOutcome::ClosedHistoryAnomaly {
                from: DeliveryStatus::Pending
            },
            "불법 이력 전이는 anomaly 로 노출(은폐 금지)"
        );
        assert_eq!(
            l.open_request_count(),
            0,
            "그래도 계약은 닫힘(재오픈이 더 나쁨)"
        );
        assert_eq!(
            l.records_for("req-1")[0].status,
            DeliveryStatus::Pending,
            "이력은 불법 전이라 안 건드려짐(그대로 Pending)"
        );
    }

    #[test]
    fn close_on_reply_with_evicted_history_is_plain_closed_not_anomaly() {
        let mut l = Ledger::with_capacity(1);
        let now = t0();
        // Tracking is a separate map from history, so a contract can be opened without `record` — req-1 has no history row to point at from the start.
        l.open_request("req-1", "alice", sid(), "bob", tid("bob"), None, now);
        l.record(
            "other",
            "x",
            tid("x"),
            "y",
            Some(tid("y")),
            "z",
            DeliveryStatus::Delivered,
            now,
        );
        assert_eq!(
            reply(&mut l, "req-1", now),
            ReplyOutcome::Closed,
            "가리킬 이력 없음(NotFound)은 anomaly 아님 → Closed"
        );
        assert_eq!(l.open_request_count(), 0, "계약 닫힘");
    }

    // ── reply_by timeout ─────────────────────────────────────────────────────────
    #[test]
    fn due_timeout_respects_deadline_boundary() {
        let mut l = Ledger::new();
        let now = t0();
        let reply_by = Duration::from_secs(600);
        open_committed(&mut l, "req-1", rb(reply_by), now);
        assert!(
            l.due_timeouts(now + reply_by).is_empty(),
            "정확히 기한은 due 아님"
        );
        let due = l.due_timeouts(now + reply_by + Duration::from_nanos(1));
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].request_id, "req-1");
        assert_eq!(due[0].sender, "alice", "notice 는 발신자에게(spec §3)");
        assert_eq!(due[0].recipient, "bob");
    }

    #[test]
    fn due_timeout_carries_sender_id_and_raw_notation() {
        // Why the spelling is not normalized — `60m` must stay as is so the envelope reply-by and the notice text agree.
        let mut l = Ledger::new();
        let now = t0();
        let sender = sid();
        let reply_by = Duration::from_secs(3600);
        l.open_request(
            "req-1",
            "alice",
            sender,
            "bob",
            tid("bob"),
            Some((reply_by, "60m".to_string())),
            now,
        );
        commit(&mut l, Some("req-1"), None);
        let due = l.due_timeouts(now + reply_by + Duration::from_secs(1));
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].sender_id, sender, "발신자 id 동봉(개명 대비 힌트)");
        assert_eq!(
            due[0].reply_by_raw, "60m",
            "표기 원본 그대로(1h 로 정규화 금지)"
        );
    }

    #[test]
    fn due_timeout_no_double_notification() {
        let mut l = Ledger::new();
        let now = t0();
        let reply_by = Duration::from_secs(600);
        open_delivered_request(&mut l, "req-1", rb(reply_by), now);
        let over = now + reply_by + Duration::from_secs(1);
        assert_eq!(l.due_timeouts(over).len(), 1, "첫 산출은 보고");
        assert!(
            l.due_timeouts(over).is_empty(),
            "두 번째 호출은 이미 통지된 request 를 다시 안 냄(이중 통지 방지)"
        );
    }

    #[test]
    fn replied_request_excluded_from_due_timeouts() {
        let mut l = Ledger::new();
        let now = t0();
        let reply_by = Duration::from_secs(600);
        l.open_request(
            "req-1",
            "alice",
            sid(),
            "bob",
            tid("bob"),
            rb(reply_by),
            now,
        );
        assert_eq!(reply(&mut l, "req-1", now), ReplyOutcome::Closed);
        let over = now + reply_by + Duration::from_secs(60);
        assert!(
            l.due_timeouts(over).is_empty(),
            "회신된 request 는 타임아웃 대상 아님"
        );
    }

    #[test]
    fn drop_request_removes_the_entry_entirely_unlike_close() {
        let mut l = Ledger::new();
        let now = t0();
        let reply_by = Duration::from_secs(600);

        open_delivered_request(&mut l, "closed-1", rb(reply_by), now);
        assert_eq!(reply(&mut l, "closed-1", now), ReplyOutcome::Closed);
        assert_eq!(
            l.open_request(
                "closed-1",
                "alice",
                sid(),
                "bob",
                tid("bob"),
                rb(reply_by),
                now
            ),
            OpenOutcome::DuplicateId,
            "닫힌 항목은 추적에 남아 재오픈을 막는다"
        );

        l.open_request(
            "dropped-1",
            "alice",
            sid(),
            "bob",
            tid("bob"),
            rb(reply_by),
            now,
        );
        assert_eq!(
            drop_req(&mut l, "dropped-1"),
            DropOutcome::Removed { notified: false },
            "제거 성공 — 통지 전이었으므로 notified=false"
        );
        assert_eq!(
            drop_req(&mut l, "dropped-1"),
            DropOutcome::NotFound,
            "멱등 — 두 번째는 NotFound"
        );
        assert_eq!(
            l.open_request(
                "dropped-1",
                "alice",
                sid(),
                "bob",
                tid("bob"),
                rb(reply_by),
                now
            ),
            OpenOutcome::Opened,
            "제거된 id 는 다시 열 수 있다(계약 미성립 = 흔적 없음)"
        );
        assert_eq!(l.open_request_count(), 1, "열린 계약은 방금 것 하나뿐");
    }

    #[test]
    fn request_without_reply_by_never_times_out() {
        let mut l = Ledger::new();
        let now = t0();
        l.open_request("req-1", "alice", sid(), "bob", tid("bob"), None, now);
        let far = now + Duration::from_secs(100_000);
        assert!(
            l.due_timeouts(far).is_empty(),
            "기한 없는 request 는 타임아웃 없음"
        );
    }

    #[test]
    fn skipped_status_for_group_dead_member() {
        let mut l = Ledger::new();
        let now = t0();
        l.record(
            "g1",
            "boss",
            tid("boss"),
            "dead",
            Some(tid("dead")),
            "msg",
            DeliveryStatus::Skipped,
            now,
        );
        assert_eq!(l.records_for("g1")[0].status, DeliveryStatus::Skipped);
    }

    // ── evict ↔ request tracking coupling ────────────────────────────────────────
    #[test]
    fn eviction_drops_only_finished_request_tracking() {
        let cap = 2;
        let mut l = Ledger::with_capacity(cap);
        let now = t0();
        l.record(
            "done",
            "alice",
            tid("alice"),
            "bob",
            Some(tid("bob")),
            "q",
            DeliveryStatus::Pending,
            now,
        );
        open_committed(&mut l, "done", None, now);
        assert!(matches!(
            reply(&mut l, "done", now),
            ReplyOutcome::ClosedHistoryAnomaly { .. } | ReplyOutcome::Closed
        ));
        for i in 0..cap {
            l.record(
                &format!("x{i}"),
                "alice",
                tid("alice"),
                "bob",
                Some(tid("bob")),
                "q",
                DeliveryStatus::Delivered,
                now,
            );
        }
        assert_eq!(l.history_len(), cap, "이력은 용량 유계");
        assert!(
            l.records_for("done").is_empty(),
            "전제: done 이력은 evict 됐다"
        );
        assert!(
            !l.msg_id_in_use("done"),
            "끝난 계약의 추적은 이력 evict 와 함께 드롭(유계 유지)"
        );
    }

    #[test]
    fn eviction_keeps_live_contract_so_reply_and_timeout_still_work() {
        let cap = 2;
        let mut l = Ledger::with_capacity(cap);
        let now = t0();
        let reply_by = Duration::from_secs(600);
        l.record(
            "req-1",
            "alice",
            tid("alice"),
            "bob",
            Some(tid("bob")),
            "q",
            DeliveryStatus::Pending,
            now,
        );
        open_committed(&mut l, "req-1", rb(reply_by), now);
        for i in 0..cap {
            l.record(
                &format!("x{i}"),
                "alice",
                tid("alice"),
                "bob",
                Some(tid("bob")),
                "q",
                DeliveryStatus::Delivered,
                now,
            );
        }
        assert!(l.records_for("req-1").is_empty(), "전제: 이력은 evict 됐다");
        assert_eq!(
            l.open_request_count(),
            1,
            "살아 있는 계약은 evict 를 견딘다"
        );
        let due = l.due_timeouts(now + reply_by + Duration::from_secs(1));
        assert_eq!(due.len(), 1, "evict 됐어도 타임아웃 통지는 살아 있다");
        assert_eq!(due[0].request_id, "req-1");

        // Reproduced with another ledger — separated so it is not mixed with the item already notified above.
        let mut l2 = Ledger::with_capacity(cap);
        l2.record(
            "req-2",
            "alice",
            tid("alice"),
            "bob",
            Some(tid("bob")),
            "q",
            DeliveryStatus::Pending,
            now,
        );
        open_committed(&mut l2, "req-2", rb(reply_by), now);
        for i in 0..cap {
            l2.record(
                &format!("y{i}"),
                "alice",
                tid("alice"),
                "bob",
                Some(tid("bob")),
                "q",
                DeliveryStatus::Delivered,
                now,
            );
        }
        assert_eq!(
            reply(&mut l2, "req-2", now),
            ReplyOutcome::Closed,
            "이력이 evict 됐어도 회신은 계약을 닫는다(가리킬 이력만 없음)"
        );
        assert_eq!(l2.open_request_count(), 0);
    }

    fn ledger_with_evicted_live_contract(
        cap: usize,
        id: &str,
        reply_by: Option<(Duration, String)>,
        now: Instant,
    ) -> Ledger {
        let mut l = Ledger::with_capacity(cap);
        l.record(
            id,
            "alice",
            tid("alice"),
            "bob",
            Some(tid("bob")),
            "q",
            DeliveryStatus::Pending,
            now,
        );
        l.open_request(id, "alice", sid(), "bob", tid("bob"), reply_by, now);
        commit(&mut l, Some(id), None);
        for i in 0..cap {
            l.record(
                &format!("filler{i}"),
                "alice",
                tid("alice"),
                "bob",
                Some(tid("bob")),
                "x",
                DeliveryStatus::Delivered,
                now,
            );
        }
        assert!(l.records_for(id).is_empty(), "전제: 그 계약의 이력은 evict");
        assert_eq!(
            l.tracking_len(),
            1,
            "전제: 살아 있는 계약은 evict 를 견딘다"
        );
        l
    }

    #[test]
    fn close_after_history_eviction_removes_the_finished_tracking_entry() {
        let now = t0();
        let mut l = ledger_with_evicted_live_contract(2, "req-1", None, now);
        assert_eq!(reply(&mut l, "req-1", now), ReplyOutcome::Closed);
        assert_eq!(
            l.tracking_len(),
            0,
            "닫히는 순간 고아 추적 항목을 제거(좀비 없음)"
        );
        assert!(!l.msg_id_in_use("req-1"), "추적에도 이력에도 없다");
    }

    #[test]
    fn timeout_notice_after_history_eviction_keeps_the_unanswered_contract_tracked() {
        let now = t0();
        let reply_by = Duration::from_secs(600);
        let mut l = ledger_with_evicted_live_contract(2, "req-1", rb(reply_by), now);
        let due = l.due_timeouts(now + reply_by + Duration::from_secs(1));
        assert_eq!(due.len(), 1, "evict 됐어도 통지는 나간다(fix 3 유지)");
        assert_eq!(due[0].request_id, "req-1");
        assert_eq!(
            l.tracking_len(),
            1,
            "통지는 회신이 아니다 — 미회신 계약은 추적에 남는다(B3)"
        );
        let open = l.open_requests();
        assert_eq!(open.len(), 1, "미결 목록에 남아야: {open:?}");
        assert!(open[0].notified, "통지 사실은 플래그로만 구분");
        assert_eq!(reply(&mut l, "req-1", now), ReplyOutcome::Closed);
        assert_eq!(l.tracking_len(), 0, "닫히는 순간 고아 항목 제거");
    }

    #[test]
    fn tracking_stays_bounded_when_evicted_contracts_are_answered() {
        let cap = 2;
        let mut l = Ledger::with_capacity(cap);
        let now = t0();
        let reply_by = Duration::from_secs(60);
        for i in 0..50 {
            let id = format!("r{i}");
            l.record(
                &id,
                "alice",
                tid("alice"),
                "bob",
                Some(tid("bob")),
                "q",
                DeliveryStatus::Pending,
                now,
            );
            open_committed(&mut l, &id, rb(reply_by), now);
            for j in 0..cap {
                l.record(
                    &format!("f{i}-{j}"),
                    "alice",
                    tid("alice"),
                    "bob",
                    Some(tid("bob")),
                    "x",
                    DeliveryStatus::Delivered,
                    now,
                );
            }
            assert!(l.records_for(&id).is_empty(), "전제: 이력 evict");
            assert_eq!(reply(&mut l, &id, now), ReplyOutcome::Closed);
            assert_eq!(
                l.tracking_len(),
                0,
                "라운드마다 추적이 0 으로 수렴(좀비 누적 없음)"
            );
        }
    }

    #[test]
    fn a_cap_full_of_notified_contracts_retires_the_oldest_instead_of_blocking_forever() {
        let mut l = Ledger::new();
        let now = t0();
        let reply_by = Duration::from_secs(60);
        let over = now + reply_by + Duration::from_secs(1);
        for i in 0..MAX_OPEN_REQUESTS {
            let id = format!("r{i}");
            l.record(
                &id,
                "alice",
                tid("alice"),
                "bob",
                Some(tid("bob")),
                "q",
                DeliveryStatus::Pending,
                now,
            );
            assert_eq!(
                open_committed(&mut l, &id, rb(reply_by), now),
                OpenOutcome::Opened
            );
        }
        assert_eq!(
            l.due_timeouts(over).len(),
            MAX_OPEN_REQUESTS,
            "전원 기한 초과 통지(= 발신자는 이미 결말을 통보받았다 → 은퇴 가능)"
        );
        assert_eq!(
            l.tracking_len(),
            MAX_OPEN_REQUESTS,
            "통지돼도 미회신이면 남는다(B3)"
        );

        let outcome = l.open_request("over", "alice", sid(), "bob", tid("bob"), None, over);
        match outcome {
            OpenOutcome::OpenedAfterMarking(r) => {
                assert_eq!(r.request_id, "r0", "가장 오래된 것부터 은퇴 표시");
                assert_eq!(r.sender, "alice");
                assert_eq!(r.recipient, "bob");
            }
            other => panic!("표시 후 수용이어야: {other:?}"),
        }
        assert!(
            l.open_requests().iter().any(|r| r.request_id == "r0"),
            "커밋 전에는 희생자가 여전히 열린 계약이다"
        );
        assert_eq!(
            l.tracking_len(),
            MAX_OPEN_REQUESTS + 1,
            "표시 구간엔 +1(잠정분)"
        );
        commit(&mut l, Some("over"), Some("r0"));
        assert!(
            !l.open_requests().iter().any(|r| r.request_id == "r0"),
            "커밋에서 비로소 은퇴한다"
        );
        assert_eq!(l.tracking_len(), MAX_OPEN_REQUESTS, "유계 유지(512 불변)");
        assert!(l.open_requests().iter().any(|r| r.request_id == "over"));
    }

    #[test]
    fn contracts_without_a_deadline_are_evictable_at_capacity() {
        let mut l = Ledger::new();
        let now = t0();
        for i in 0..MAX_OPEN_REQUESTS {
            assert_eq!(
                open_committed(&mut l, &format!("r{i}"), None, now),
                OpenOutcome::Opened
            );
        }
        let outcome = l.open_request("over", "alice", sid(), "bob", tid("bob"), None, now);
        assert!(
            matches!(outcome, OpenOutcome::OpenedAfterMarking(ref r) if r.request_id == "r0"),
            "기한 없는 계약은 통지 빚이 없어 은퇴 가능: {outcome:?}"
        );
        commit(&mut l, Some("over"), Some("r0"));
        assert_eq!(l.tracking_len(), MAX_OPEN_REQUESTS);
    }

    #[test]
    fn a_cap_full_of_pending_deadline_contracts_rejects_instead_of_breaking_a_notice_promise() {
        let mut l = Ledger::new();
        let now = t0();
        let reply_by = Duration::from_secs(600);
        for i in 0..MAX_OPEN_REQUESTS {
            assert_eq!(
                open_committed(&mut l, &format!("r{i}"), rb(reply_by), now),
                OpenOutcome::Opened
            );
        }
        assert!(l.due_timeouts(now).is_empty(), "전제: 기한 전");
        assert_eq!(
            l.open_request("over", "alice", sid(), "bob", tid("bob"), rb(reply_by), now),
            OpenOutcome::Full,
            "은퇴 가능한 계약이 없으면 반려(통지 약속을 어기지 않는다)"
        );
        assert_eq!(
            l.tracking_len(),
            MAX_OPEN_REQUESTS,
            "아무도 지워지지 않았다"
        );
        let over = now + reply_by + Duration::from_secs(1);
        assert_eq!(l.due_timeouts(over).len(), MAX_OPEN_REQUESTS);
        assert!(matches!(
            l.open_request("over", "alice", sid(), "bob", tid("bob"), None, over),
            OpenOutcome::OpenedAfterMarking(_)
        ));
    }

    #[test]
    fn open_requests_expose_both_party_ids_for_obligation_scoping() {
        let mut l = Ledger::new();
        let now = t0();
        let sender = sid();
        let recipient = sid();
        l.record(
            "r1",
            "alice",
            tid("alice"),
            "worker",
            Some(tid("worker")),
            "q",
            DeliveryStatus::Pending,
            now,
        );
        l.open_request("r1", "alice", sender, "worker", recipient, None, now);
        l.record(
            "r2",
            "alice",
            tid("alice"),
            "ghost",
            Some(tid("ghost")),
            "q",
            DeliveryStatus::Pending,
            now,
        );
        l.open_request("r2", "alice", sender, "ghost", tid("ghost"), None, now);

        let open = l.open_requests();
        assert_eq!(open[0].sender_id, sender);
        assert_eq!(
            open[0].recipient_id, recipient,
            "해석된 수신자는 id 를 남긴다(동명 다수 오귀속 차단의 재료)"
        );
        assert_eq!(
            open[1].recipient_id,
            tid("ghost"),
            "잠듦 파킹도 열 때부터 그 프로필 id 를 든다(ADR-0309)"
        );
    }

    #[test]
    fn records_for_detailed_compares_surviving_rows_against_the_expected_count() {
        let now = t0();
        let mut l = Ledger::with_capacity(8);
        l.record(
            "m1",
            "a",
            tid("a"),
            "b",
            Some(tid("b")),
            "x",
            DeliveryStatus::Delivered,
            now,
        );
        let (rows, truncated) = l.records_for_detailed("m1");
        assert_eq!(rows.len(), 1);
        assert!(!truncated, "기대 1행이 그대로 있으면 완전");

        for to in ["x", "y", "z"] {
            l.record_with_expected(
                "g1",
                "a",
                tid("a"),
                to,
                Some(tid(to)),
                "b",
                DeliveryStatus::Delivered,
                now,
                3,
            );
        }
        let (rows, truncated) = l.records_for_detailed("g1");
        assert_eq!(rows.len(), 3);
        assert!(!truncated, "3/3 이면 완전");

        let mut l2 = Ledger::with_capacity(4);
        l2.record_with_expected(
            "g2",
            "a",
            tid("a"),
            "m1",
            Some(tid("m1")),
            "b",
            DeliveryStatus::Pending,
            now,
            3,
        );
        l2.record_with_expected(
            "g2",
            "a",
            tid("a"),
            "m2",
            Some(tid("m2")),
            "b",
            DeliveryStatus::Pending,
            now,
            3,
        );
        l2.record(
            "other1",
            "a",
            tid("a"),
            "z",
            Some(tid("z")),
            "x",
            DeliveryStatus::Delivered,
            now,
        );
        l2.record_with_expected(
            "g2",
            "a",
            tid("a"),
            "m3",
            Some(tid("m3")),
            "b",
            DeliveryStatus::Delivered,
            now,
            3,
        );
        l2.record(
            "other2",
            "a",
            tid("a"),
            "z",
            Some(tid("z")),
            "x",
            DeliveryStatus::Delivered,
            now,
        );
        l2.record(
            "other3",
            "a",
            tid("a"),
            "z",
            Some(tid("z")),
            "x",
            DeliveryStatus::Delivered,
            now,
        );
        let (rows, truncated) = l2.records_for_detailed("g2");
        assert_eq!(rows.len(), 1, "g2 는 3행 중 1행만 남았다: {rows:?}");
        assert_ne!(
            l2.all_records()[0].msg_id,
            "g2",
            "전제: 링 front 는 남의 행 — 옛 위치 증명이 '완전' 이라 답하던 배치"
        );
        assert!(truncated, "남은 행(1) < 기대(3) 이면 잘림(F3)");
    }

    #[test]
    fn open_request_rejects_at_capacity_with_full() {
        // Fill the cap with **non-retirable** (waiting on deadline) contracts — filling with no-deadline contracts would cause a retirement
        //   and no Full would come out (that branch has a separate test).
        let mut l = Ledger::new();
        let now = t0();
        let reply_by = Duration::from_secs(600);
        for i in 0..MAX_OPEN_REQUESTS {
            assert_eq!(
                open_committed(&mut l, &format!("r{i}"), rb(reply_by), now),
                OpenOutcome::Opened
            );
        }
        assert_eq!(
            l.open_request("over", "alice", sid(), "bob", tid("bob"), rb(reply_by), now),
            OpenOutcome::Full,
            "cap 도달 + 은퇴 가능분 없음 → Full"
        );
        assert_eq!(l.open_request_count(), MAX_OPEN_REQUESTS, "기존 계약 불변");
        assert_eq!(reply(&mut l, "r0", now), ReplyOutcome::Closed);
        assert_eq!(
            l.open_request("over", "alice", sid(), "bob", tid("bob"), rb(reply_by), now),
            OpenOutcome::Opened,
            "닫힌 계약은 cap 계수에서 빠진다"
        );
    }

    // G1
    #[test]
    fn rollback_leaves_the_victim_exactly_as_it_was() {
        let mut l = Ledger::new();
        let now = t0();
        let reply_by = Duration::from_secs(60);
        for i in 0..MAX_OPEN_REQUESTS {
            open_committed(&mut l, &format!("r{i}"), rb(reply_by), now);
        }
        let over = now + reply_by + Duration::from_secs(1);
        assert_eq!(l.due_timeouts(over).len(), MAX_OPEN_REQUESTS);
        let before: Vec<String> = l
            .open_requests()
            .into_iter()
            .map(|r| r.request_id)
            .collect();

        let OpenOutcome::OpenedAfterMarking(victim) =
            l.open_request("new", "alice", sid(), "bob", tid("bob"), None, over)
        else {
            panic!("전제: 표시 후 수용");
        };
        assert_eq!(victim.request_id, "r0");

        rollback(&mut l, Some("new"), Some("r0"));
        let after: Vec<String> = l
            .open_requests()
            .into_iter()
            .map(|r| r.request_id)
            .collect();
        assert_eq!(after, before, "목록·순서가 원상 복구");
        let r0 = l
            .open_requests()
            .into_iter()
            .find(|r| r.request_id == "r0")
            .expect("그대로 있다");
        assert!(r0.notified, "통지 플래그 불변(건드린 적이 없다)");
        assert_eq!(r0.created_at, now, "나이 불변");
        assert!(matches!(
            l.open_request("new2", "alice", sid(), "bob", tid("bob"), None, over),
            OpenOutcome::OpenedAfterMarking(ref v) if v.request_id == "r0"
        ));
    }

    #[test]
    fn a_closed_provisional_keeps_its_reserved_slot_until_its_guard_settles() {
        let cap = MAX_OPEN_REQUESTS;
        let now = t0();

        // Shared fixture: fill the cap with **retirable** (no deadline) confirmed contracts.
        let build = || {
            let mut l = Ledger::new();
            for i in 0..cap {
                open_committed(&mut l, &format!("c{i}"), None, now);
            }
            assert_eq!(l.occupied_slots(), cap, "전제: 상한");
            l
        };

        // ── rollback branch ──────────────────────────────────────────────────────────
        let mut l = build();
        let OpenOutcome::OpenedAfterMarking(v1) =
            l.open_request("PA", "alice", sid(), "bob", tid("bob"), None, now)
        else {
            panic!("전제: 표시 후 수용");
        };
        assert_eq!(v1.request_id, "c0");
        assert_eq!(l.occupied_slots(), cap, "표시+삽입 후에도 정확히 상한");
        assert_eq!(reply(&mut l, "PA", now), ReplyOutcome::Closed);
        assert_eq!(
            l.occupied_slots(),
            cap,
            "닫힌 잠정 계약도 정산 전까지 자리를 지킨다(round-6 I1)"
        );
        let b = l.open_request("PB", "carol", sid(), "dave", tid("dave"), None, now);
        assert!(
            matches!(b, OpenOutcome::OpenedAfterMarking(ref v) if v.request_id == "c1"),
            "B 도 자기 몫의 희생자를 표시해야: {b:?}"
        );
        assert_eq!(l.occupied_slots(), cap);
        rollback(&mut l, Some("PA"), Some("c0"));
        assert_eq!(
            l.occupied_slots(),
            cap,
            "롤백 뒤에도 정확히 상한 — 513 고착 없음(round-6 I1)"
        );
        commit(&mut l, Some("PB"), Some("c1"));
        assert_eq!(l.occupied_slots(), cap, "B 커밋 후에도 상한 유지");

        // ── commit branch ────────────────────────────────────────────────────────────
        let mut l = build();
        let OpenOutcome::OpenedAfterMarking(v1) =
            l.open_request("PA", "alice", sid(), "bob", tid("bob"), None, now)
        else {
            panic!("전제");
        };
        assert_eq!(v1.request_id, "c0");
        assert_eq!(reply(&mut l, "PA", now), ReplyOutcome::Closed);
        assert_eq!(l.occupied_slots(), cap, "정산 전엔 자리 유지");
        commit(&mut l, Some("PA"), Some("c0"));
        assert_eq!(
            l.occupied_slots(),
            cap - 1,
            "회신으로 끝난 계약은 커밋 시점에 자리를 놓는다(초과 아님 — 여유가 생긴 것)"
        );
        assert!(
            l.occupied_slots() <= MAX_OPEN_REQUESTS,
            "어느 갈래도 상한 초과 없음"
        );
    }

    #[test]
    fn a_provisional_contract_is_not_swept_but_is_collected_right_after_commit() {
        let mut l = Ledger::new();
        let now = t0();
        let reply_by = Duration::from_secs(60);
        let over = now + reply_by + Duration::from_secs(1);
        l.open_request("p", "alice", sid(), "bob", tid("bob"), rb(reply_by), now);
        assert!(
            l.due_timeouts(over).is_empty(),
            "잠정 계약은 sweep 대상이 아니다(round-6 I2)"
        );
        assert!(l.open_requests().iter().all(|r| !r.notified));

        commit(&mut l, Some("p"), None);
        let due = l.due_timeouts(over);
        assert_eq!(due.len(), 1, "커밋 후엔 곧바로 수집된다");
        assert_eq!(due[0].request_id, "p");
        assert_eq!(
            due[0].reply_by_raw,
            format!("{}s", reply_by.as_secs()),
            "표기 원본 그대로"
        );

        let mut l = Ledger::new();
        l.open_request("q", "alice", sid(), "bob", tid("bob"), rb(reply_by), now);
        assert!(l.due_timeouts(over).is_empty());
        rollback(&mut l, Some("q"), None);
        assert!(
            l.due_timeouts(over + Duration::from_secs(9999)).is_empty(),
            "반려된 요청엔 기한 통지가 없다"
        );
    }

    #[test]
    fn a_concurrent_opener_cannot_select_someone_elses_provisional_entry_as_victim() {
        let mut l = Ledger::new();
        let base = t0();
        // Fill the cap with retirable (no deadline) contracts but leave **only the oldest one** and make the rest
        //   non-retirable → this forces that one to be B's only legal victim.
        let reply_by = Duration::from_secs(600);
        open_committed(&mut l, "evictable", None, base);
        for i in 1..MAX_OPEN_REQUESTS {
            let at = base + Duration::from_secs(i as u64);
            open_committed(&mut l, &format!("locked{i}"), rb(reply_by), at);
        }
        let a_at = base + Duration::from_secs(1000);
        assert!(matches!(
            l.open_request("A", "alice", sid(), "bob", tid("bob"), None, a_at),
            OpenOutcome::OpenedAfterMarking(ref r) if r.request_id == "evictable"
        ));

        let b = l.open_request("B", "carol", sid(), "dave", tid("dave"), None, a_at);
        assert_eq!(
            b,
            OpenOutcome::Full,
            "잠정 계약을 희생자로 고르면 안 된다 — 고를 게 없으면 Full 이 정답: {b:?}"
        );
        assert!(l.open_requests().iter().any(|r| r.request_id == "A"));
        assert_eq!(reply(&mut l, "A", a_at), ReplyOutcome::Closed);
    }

    #[test]
    fn rollback_unmarks_and_drops_atomically_without_drift() {
        let mut l = Ledger::new();
        let now = t0();
        for i in 0..MAX_OPEN_REQUESTS {
            open_committed(&mut l, &format!("r{i}"), None, now);
        }
        for k in 0..50 {
            let provisional = format!("p{k}");
            let OpenOutcome::OpenedAfterMarking(victim) =
                l.open_request(&provisional, "alice", sid(), "bob", tid("bob"), None, now)
            else {
                panic!("전제: 매 사이클 표시 후 수용");
            };
            assert_eq!(
                victim.request_id, "r0",
                "사이클 {k}: 늘 같은 최고령이 표시된다"
            );
            let dropped = rollback(&mut l, Some(provisional.as_str()), Some("r0"));
            assert_eq!(
                dropped,
                Some(DropOutcome::Removed { notified: false }),
                "사이클 {k}: 잠정 계약이 같은 구간에서 제거됐다"
            );
            assert_eq!(
                l.open_request_count(),
                MAX_OPEN_REQUESTS,
                "사이클 {k}: 계약 수는 정확히 상한(513 로 새지 않는다)"
            );
            assert!(
                !l.open_requests()
                    .iter()
                    .any(|r| r.request_id == provisional),
                "사이클 {k}: 잠정 계약은 남지 않는다"
            );
            assert!(
                l.open_requests().iter().any(|r| r.request_id == "r0"),
                "사이클 {k}: 희생자는 표시만 지워진 채 그대로다"
            );
        }
        assert_eq!(l.tracking_len(), MAX_OPEN_REQUESTS, "추적 총량도 불변");
    }

    #[test]
    fn reclamation_follows_owner_liveness_not_the_clock() {
        let mut l = Ledger::new();
        let now = t0();
        for i in 0..MAX_OPEN_REQUESTS {
            open_committed(&mut l, &format!("r{i}"), None, now);
        }
        let occupied_before = l.occupied_slots();
        let tracking_before = l.tracking_len();

        let OpenOutcome::OpenedAfterMarking(victim) =
            l.open_request("p-live", "alice", sid(), "bob", tid("bob"), None, now)
        else {
            panic!("전제: 상한 압력으로 표시 후 수용");
        };
        assert_eq!(victim.request_id, "r0");
        let live = ReservationLiveness::new();
        l.attach_reservation_liveness("p-live", tid("bob"), live.watch());
        assert_eq!(l.marked_retirement_count_for_test(), 1, "표시 1건");

        for round in 0..3 {
            assert!(
                l.reclaim_abandoned_reservations().is_empty(),
                "round {round}: 소유자 생존 예약은 sweep 대상이 아니다"
            );
        }
        assert!(
            l.is_tracked_for_test("p-live", tid("bob")),
            "그대로 남아 있어야"
        );
        assert_eq!(l.marked_retirement_count_for_test(), 1, "표시도 그대로");

        drop(live);
        let reclaimed = l.reclaim_abandoned_reservations();
        assert_eq!(
            reclaimed,
            vec![("p-live".to_string(), tid("bob"))],
            "회수한 계약 키를 호출자에게 알려야(락 밖 기록용): {reclaimed:?}"
        );
        assert!(
            !l.is_tracked_for_test("p-live", tid("bob")),
            "잠정 계약이 제거돼야"
        );
        assert_eq!(
            l.marked_retirement_count_for_test(),
            0,
            "희생자 표시도 풀려야 — 남으면 cap 분모가 영구히 준다"
        );
        assert!(
            l.open_requests().iter().any(|r| r.request_id == "r0"),
            "희생자 자신은 열린 채 그대로(교환이 성립하지 않았으니)"
        );
        assert_eq!(l.occupied_slots(), occupied_before);
        assert_eq!(l.tracking_len(), tracking_before);

        assert!(l.reclaim_abandoned_reservations().is_empty());
    }

    #[test]
    fn a_settled_reservation_is_never_reclaimed_even_after_its_guard_is_gone() {
        let mut l = Ledger::new();
        let now = t0();
        l.record(
            "p-ok",
            "alice",
            tid("alice"),
            "bob",
            Some(tid("bob")),
            "q",
            DeliveryStatus::Pending,
            now,
        );
        let OpenOutcome::Opened =
            l.open_request("p-ok", "alice", sid(), "bob", tid("bob"), None, now)
        else {
            panic!("전제: 여유 상태에서 오픈");
        };
        let live = ReservationLiveness::new();
        l.attach_reservation_liveness("p-ok", tid("bob"), live.watch());
        let committed = l.commit_open(Some(("p-ok", tid("bob"))), None);
        assert!(committed.confirmed, "전제: 커밋이 항목을 찾았다");
        drop(live);

        assert!(
            l.reclaim_abandoned_reservations().is_empty(),
            "정산된 계약을 회수하면 접수 보고한 request 가 계약 없이 남는다"
        );
        assert!(
            l.is_tracked_for_test("p-ok", tid("bob")),
            "계약은 그대로 살아 있어야"
        );
    }

    #[test]
    fn commit_reports_whether_the_planned_retirement_actually_happened() {
        let mut l = Ledger::new();
        let now = t0();

        for i in 0..MAX_OPEN_REQUESTS {
            open_committed(&mut l, &format!("r{i}"), None, now);
        }
        let OpenOutcome::OpenedAfterMarking(v) =
            l.open_request("p1", "alice", sid(), "bob", tid("bob"), None, now)
        else {
            panic!("전제: 표시 후 수용");
        };
        let out = l.commit_open(
            Some(("p1", tid("bob"))),
            Some((v.request_id.as_str(), v.recipient_id)),
        );
        assert_eq!(
            out,
            CommitOutcome {
                confirmed: true,
                retired: true
            },
            "실제로 은퇴했으면 그렇다고 답해야"
        );

        let OpenOutcome::OpenedAfterMarking(v2) =
            l.open_request("p2", "alice", sid(), "bob2", tid("bob2"), None, now)
        else {
            panic!("전제: 표시 후 수용");
        };
        // Remove the marked victim from outside to create "already gone at commit time" (the same result shape as the purge path).
        assert!(matches!(
            l.drop_request(&v2.request_id, v2.recipient_id),
            DropOutcome::Removed { .. }
        ));
        let out2 = l.commit_open(
            Some(("p2", tid("bob2"))),
            Some((v2.request_id.as_str(), v2.recipient_id)),
        );
        assert_eq!(
            out2,
            CommitOutcome {
                confirmed: true,
                retired: false
            },
            "계약은 확정됐지만 은퇴는 없었다 — 이 값이 계측의 조건이다(R2)"
        );

        let out3 = l.commit_open(Some(("nope", tid("nobody"))), None);
        assert_eq!(
            out3,
            CommitOutcome {
                confirmed: false,
                retired: false
            }
        );
    }

    #[test]
    fn a_reply_to_a_marked_victim_during_the_window_closes_it_properly() {
        let base = t0();
        let reply_by = Duration::from_secs(600);
        let mut l = Ledger::new();
        l.record(
            "v",
            "alice",
            tid("alice"),
            "bob",
            Some(tid("bob")),
            "q",
            DeliveryStatus::Pending,
            base,
        );
        open_committed(&mut l, "v", rb(reply_by), base);
        assert_eq!(
            l.transition("v", tid("bob"), DeliveryStatus::Delivered, base),
            Ok(())
        );
        // Make v retirable (the deadline-overrun notice fires), then fill the cap.
        assert_eq!(
            l.due_timeouts(base + reply_by + Duration::from_secs(1))
                .len(),
            1
        );
        for i in 1..MAX_OPEN_REQUESTS {
            let at = base + Duration::from_secs(3600 + i as u64);
            open_committed(&mut l, &format!("f{i}"), None, at);
        }
        let win = base + Duration::from_secs(9000);
        assert!(matches!(
            l.open_request("new", "alice", sid(), "bob", tid("bob"), None, win),
            OpenOutcome::OpenedAfterMarking(ref r) if r.request_id == "v"
        ));
        assert_eq!(
            reply(&mut l, "v", win),
            ReplyOutcome::Closed,
            "표시는 매칭을 가리지 않는다(round-5)"
        );
        let tracked_before = l.tracking_len();
        rollback(&mut l, Some("new"), Some("v"));
        assert_eq!(
            l.tracking_len(),
            tracked_before - 1,
            "잠정 계약 1건만 줄어야 — 롤백이 희생자를 **지우면** 안 된다(표시 해제뿐)"
        );
        assert!(
            l.msg_id_in_use("v"),
            "닫힌 희생자는 추적에 그대로 남는다(삭제 아님)"
        );
        assert!(
            !l.open_requests().iter().any(|r| r.request_id == "v"),
            "회신으로 닫혔으므로 미결이 아니다"
        );
        assert!(
            l.due_timeouts(win + Duration::from_secs(99999)).is_empty(),
            "닫힌 계약엔 헛 기한 통지가 나가지 않는다"
        );
        assert!(!l.open_requests().iter().any(|r| r.request_id == "new"));

        let mut l = Ledger::new();
        open_committed(&mut l, "v", None, base);
        for i in 1..MAX_OPEN_REQUESTS {
            let at = base + Duration::from_secs(i as u64);
            open_committed(&mut l, &format!("f{i}"), None, at);
        }
        assert!(matches!(
            l.open_request("new", "alice", sid(), "bob", tid("bob"), None, win),
            OpenOutcome::OpenedAfterMarking(ref r) if r.request_id == "v"
        ));
        assert_eq!(reply(&mut l, "v", win), ReplyOutcome::Closed);
        commit(&mut l, Some("new"), Some("v"));
        assert!(!l.open_requests().iter().any(|r| r.request_id == "v"));
        assert_eq!(
            l.open_request_count(),
            MAX_OPEN_REQUESTS,
            "커밋 후 계수는 정확히 상한"
        );
    }

    #[test]
    fn marked_and_provisional_entries_stay_visible_to_the_mint_collision_check() {
        let cap = 2;
        let mut l = Ledger::with_capacity(cap);
        let now = t0();
        for i in 0..MAX_OPEN_REQUESTS {
            let id = format!("r{i}");
            l.record(
                &id,
                "alice",
                tid("alice"),
                "bob",
                Some(tid("bob")),
                "q",
                DeliveryStatus::Pending,
                now,
            );
            open_committed(&mut l, &id, None, now);
        }
        for j in 0..cap {
            l.record(
                &format!("f{j}"),
                "alice",
                tid("alice"),
                "bob",
                Some(tid("bob")),
                "x",
                DeliveryStatus::Delivered,
                now,
            );
        }
        assert!(l.records_for("r0").is_empty(), "전제: 이력 evict");

        assert!(matches!(
            l.open_request("new", "alice", sid(), "bob", tid("bob"), None, now),
            OpenOutcome::OpenedAfterMarking(ref r) if r.request_id == "r0"
        ));
        assert!(
            l.msg_id_in_use("r0"),
            "표시된 희생자는 추적에 그대로 있어 사용 중으로 보인다"
        );
        assert!(l.msg_id_in_use("new"), "잠정 계약도 사용 중");
        commit(&mut l, Some("new"), Some("r0"));
        assert!(!l.msg_id_in_use("r0"));
        assert!(l.msg_id_in_use("new"), "확정된 계약은 계속 사용 중");
    }

    #[test]
    fn open_requests_are_sorted_oldest_first_regardless_of_list_position() {
        let mut l = Ledger::new();
        let base = t0();
        open_committed(&mut l, "late", None, base + Duration::from_secs(300));
        open_committed(&mut l, "middle", None, base + Duration::from_secs(100));
        open_committed(&mut l, "early", None, base);

        let ids: Vec<String> = l
            .open_requests()
            .into_iter()
            .map(|r| r.request_id)
            .collect();
        assert_eq!(
            ids,
            vec!["early", "middle", "late"],
            "추가 순서(late→middle→early)와 무관하게 오래된 순이어야(H4)"
        );
        let times: Vec<_> = l
            .open_requests()
            .into_iter()
            .map(|r| r.created_at)
            .collect();
        assert!(
            times.windows(2).all(|w| w[0] <= w[1]),
            "created_at 오름차순"
        );
    }

    #[test]
    fn open_requests_stay_sorted_after_a_marked_retirement_is_rolled_back() {
        let mut l = Ledger::new();
        let base = t0();
        for i in 0..MAX_OPEN_REQUESTS {
            let at = base + Duration::from_secs(i as u64);
            open_committed(&mut l, &format!("r{i}"), None, at);
        }
        let OpenOutcome::OpenedAfterMarking(victim) = l.open_request(
            "new",
            "alice",
            sid(),
            "bob",
            tid("bob"),
            None,
            base + Duration::from_secs(9999),
        ) else {
            panic!("전제");
        };
        assert_eq!(victim.request_id, "r0", "가장 오래된 것이 희생자");
        rollback(&mut l, Some("new"), Some(victim.request_id.as_str()));
        let times: Vec<_> = l
            .open_requests()
            .into_iter()
            .map(|r| r.created_at)
            .collect();
        assert!(
            times.windows(2).all(|w| w[0] <= w[1]),
            "복원 뒤에도 오래된 순: {times:?}"
        );
        assert_eq!(
            l.open_requests().first().map(|r| r.request_id.clone()),
            Some("r0".to_string()),
            "복원된 최고령 계약이 다시 맨 앞"
        );
    }

    /// ★A contract is picked by the replier id alone (ADR-0309 decision 7)★ — even with two contracts holding the same name snapshot, a reply from a different id
    ///   cannot close either, and a reply sent by another agent with the same name cannot close one either.
    #[test]
    fn a_reply_closes_only_the_contract_keyed_by_its_own_id() {
        let mut l = Ledger::new();
        let now = t0();
        let a = sid();
        let b = sid();
        l.record(
            "r1",
            "boss",
            tid("boss"),
            "worker",
            Some(tid("worker")),
            "q",
            DeliveryStatus::Delivered,
            now,
        );
        l.open_request("r1", "boss", sid(), "worker", a, None, now);
        l.open_request("r1", "boss", sid(), "worker", b, None, now);
        assert_eq!(
            l.close_on_reply("r1", sid(), now),
            ReplyOutcome::NoMatch,
            "이름만 같은 제3자는 닫지 못한다"
        );
        assert_eq!(
            l.fail_on_undeliverable_reply("r1", sid()),
            ReplyFailOutcome::NoMatch,
            "실패 종결도 같은 규칙"
        );
        assert_eq!(l.close_on_reply("r1", b, now), ReplyOutcome::Closed);
        assert_eq!(l.contract_outcome_for_test("r1", b), Some("replied"));
        assert_eq!(
            l.contract_outcome_for_test("r1", a),
            Some("awaiting_reply"),
            "다른 id 의 계약은 그대로 열려 있다"
        );
    }

    #[test]
    fn msg_id_in_use_sees_history_and_tracking() {
        let mut l = Ledger::new();
        let now = t0();
        assert!(!l.msg_id_in_use("m1"), "미사용 id");
        l.record(
            "m1",
            "a",
            tid("a"),
            "b",
            Some(tid("b")),
            "x",
            DeliveryStatus::Delivered,
            now,
        );
        assert!(l.msg_id_in_use("m1"), "이력에 있으면 사용 중");
        l.open_request("r1", "a", sid(), "b", tid("b"), None, now);
        assert!(l.msg_id_in_use("r1"), "추적에만 있어도 사용 중");
        open_delivered_request(&mut l, "r2", None, now);
        assert_eq!(reply(&mut l, "r2", now), ReplyOutcome::Closed);
        assert!(
            l.msg_id_in_use("r2"),
            "닫혀도 이력·추적에 남아 있으면 사용 중"
        );
    }

    #[test]
    fn is_request_closed_only_true_for_closed_entries() {
        let mut l = Ledger::new();
        let now = t0();
        open_delivered_request(&mut l, "r1", None, now);
        assert!(!closed(&l, "r1"), "열린 계약은 false");
        assert!(!closed(&l, "nope"), "없는 id 는 false(통지 막지 않음)");
        assert_eq!(reply(&mut l, "r1", now), ReplyOutcome::Closed);
        assert!(closed(&l, "r1"), "회신으로 닫히면 true");
    }

    #[test]
    fn drop_request_reports_already_notified_entry() {
        let mut l = Ledger::new();
        let now = t0();
        let reply_by = Duration::from_secs(600);
        open_delivered_request(&mut l, "r1", rb(reply_by), now);
        assert_eq!(
            l.due_timeouts(now + reply_by + Duration::from_secs(1))
                .len(),
            1
        );
        assert_eq!(
            drop_req(&mut l, "r1"),
            DropOutcome::Removed { notified: true },
            "이미 통지된 계약의 회수는 그 사실을 동봉"
        );
    }

    // ── S18 D: open_requests (pending lookup view) ───────────────────────────────────────────

    #[test]
    fn open_requests_lists_unanswered_contracts_oldest_first_with_notation() {
        let mut l = Ledger::new();
        let now = t0();
        l.record(
            "r1",
            "alice",
            tid("alice"),
            "bob",
            Some(tid("bob")),
            "q1",
            DeliveryStatus::Pending,
            now,
        );
        l.open_request(
            "r1",
            "alice",
            sid(),
            "bob",
            tid("bob"),
            rb(Duration::from_secs(600)),
            now,
        );
        let later = now + Duration::from_secs(5);
        l.record(
            "r2",
            "carol",
            tid("carol"),
            "alice",
            Some(tid("alice")),
            "q2",
            DeliveryStatus::Pending,
            later,
        );
        l.open_request("r2", "carol", sid(), "alice", tid("alice"), None, later);

        let open = l.open_requests();
        assert_eq!(open.len(), 2);
        assert_eq!(open[0].request_id, "r1", "오래된 순");
        assert_eq!(open[0].sender, "alice");
        assert_eq!(open[0].recipient, "bob");
        assert_eq!(
            open[0].reply_by_raw.as_deref(),
            Some("600s"),
            "표기는 발신자 원본 그대로(역산 금지)"
        );
        assert_eq!(
            open[0].created_at, now,
            "발송 시각 그대로(장부는 벽시계 모름)"
        );
        assert!(!open[0].notified);
        assert_eq!(open[1].request_id, "r2");
        assert_eq!(open[1].reply_by_raw, None, "기한 없는 request");
    }

    #[test]
    fn open_requests_drops_replied_but_keeps_timed_out_ones() {
        let mut l = Ledger::new();
        let now = t0();
        let d = Duration::from_secs(600);
        open_delivered_request(&mut l, "replied", None, now);
        open_delivered_request(&mut l, "timedout", rb(d), now);
        assert_eq!(reply(&mut l, "replied", now), ReplyOutcome::Closed);
        assert_eq!(l.due_timeouts(now + d + Duration::from_secs(1)).len(), 1);

        let open = l.open_requests();
        assert_eq!(open.len(), 1, "회신으로 닫힌 계약은 빠진다: {open:?}");
        assert_eq!(open[0].request_id, "timedout");
        assert!(
            open[0].notified,
            "통지 나간 사실은 필드로 노출(목록에서 제외하지 않는다)"
        );
    }

    #[test]
    fn open_requests_is_empty_without_any_contract() {
        let mut l = Ledger::new();
        let now = t0();
        l.record(
            "m1",
            "alice",
            tid("alice"),
            "bob",
            Some(tid("bob")),
            "hi",
            DeliveryStatus::Delivered,
            now,
        );
        assert!(l.open_requests().is_empty());
    }

    // ── S18.6: `pending → failed` edge + contract failure close (spec §6 · ADR-0116/0118) ────────────────

    #[test]
    fn pending_to_failed_is_legal_only_through_the_cleanup_verb() {
        let mut l = Ledger::new();
        let now = t0();
        l.record(
            "m1",
            "a",
            tid("a"),
            "b",
            Some(tid("b")),
            "x",
            DeliveryStatus::Pending,
            now,
        );
        assert_eq!(
            l.transition("m1", tid("b"), DeliveryStatus::Failed, now),
            Err(TransitionError::Illegal {
                from: DeliveryStatus::Pending,
                to: DeliveryStatus::Failed
            }),
            "임의 pending→failed 호출은 거부된다(간선은 전용 동사에만 열려 있다)"
        );
        assert_eq!(
            l.fail_pending("m1", tid("b"), "RECIPIENT_DELETED", now),
            Ok(())
        );
        let rows = l.records_for("m1");
        assert_eq!(rows[0].status, DeliveryStatus::Failed);
        assert_eq!(
            rows[0].fail_code,
            Some("RECIPIENT_DELETED"),
            "사유 코드가 레코드에 남아 조회가 이유를 답할 수 있다"
        );

        l.record(
            "m2",
            "a",
            tid("a"),
            "b",
            Some(tid("b")),
            "x",
            DeliveryStatus::Delivered,
            now,
        );
        assert_eq!(
            l.fail_pending("m2", tid("b"), "RECIPIENT_DELETED", now),
            Err(TransitionError::Illegal {
                from: DeliveryStatus::Delivered,
                to: DeliveryStatus::Failed
            }),
            "delivered→failed 는 여전히 불법"
        );
        assert_eq!(
            l.fail_pending("nope", tid("b"), "RECIPIENT_DELETED", now),
            Err(TransitionError::NotFound)
        );
    }

    #[test]
    fn a_failed_reply_never_touches_a_guard_marked_contract() {
        let mut l = Ledger::new();
        let now = t0();
        let w = tid("worker");
        l.record(
            "m1",
            "boss",
            tid("boss"),
            "worker",
            Some(tid("worker")),
            "q",
            DeliveryStatus::Delivered,
            now,
        );
        l.open_request("m1", "boss", PeerId::new_v4(), "worker", w, None, now);
        assert_eq!(
            l.fail_on_undeliverable_reply("m1", w),
            ReplyFailOutcome::GuardHeld,
            "잠정 계약은 건드리지 않는다"
        );
        assert_eq!(
            l.contract_outcome_for_test("m1", tid("worker")),
            Some("awaiting_reply")
        );
        l.commit_open(Some(("m1", tid("worker"))), None);
        assert_eq!(
            l.fail_on_undeliverable_reply("m1", w),
            ReplyFailOutcome::Failed
        );
        assert_eq!(
            l.contract_outcome_for_test("m1", tid("worker")),
            Some("reply_failed")
        );
        assert_eq!(
            l.fail_on_undeliverable_reply("m1", w),
            ReplyFailOutcome::AlreadyClosed
        );
        assert_eq!(
            l.fail_on_undeliverable_reply("nope", w),
            ReplyFailOutcome::NoMatch
        );
    }

    #[test]
    fn both_guard_marks_hold_off_the_two_failure_closers() {
        // ★Review fix D7 — a gap in guard coverage★: the existing guard tests created **only provisional (`provisional`)** ones, so
        //   deleting the `pending_retirement` condition by a single line left everything green (measured as undefended).
        // ★The mark is created by the **production path**★: setting the flag with a test setter could diverge from "the path
        //   by which that mark actually arises".
        let mut l = Ledger::new();
        let now = t0();
        let reply_by = Duration::from_secs(60);
        let over = now + reply_by + Duration::from_secs(1);
        for i in 0..MAX_OPEN_REQUESTS {
            let id = format!("r{i}");
            l.record(
                &id,
                "alice",
                tid("alice"),
                "bob",
                Some(tid("bob")),
                "q",
                DeliveryStatus::Pending,
                now,
            );
            assert_eq!(
                open_committed(&mut l, &id, rb(reply_by), now),
                OpenOutcome::Opened
            );
        }
        assert_eq!(
            l.due_timeouts(over).len(),
            MAX_OPEN_REQUESTS,
            "전원 통지 완료 = 은퇴 가능"
        );
        let marked = match l.open_request(
            "over",
            "alice",
            tid("alice"),
            "bob2",
            tid("bob2"),
            None,
            over,
        ) {
            OpenOutcome::OpenedAfterMarking(r) => r,
            other => panic!("표시 후 수용이어야: {other:?}"),
        };
        assert_eq!(marked.request_id, "r0");
        assert_eq!(l.marked_retirement_count_for_test(), 1);

        assert_eq!(
            l.fail_on_undeliverable_reply("r0", tid("bob")),
            ReplyFailOutcome::GuardHeld,
            "은퇴 예정 표시 계약은 실패 종결 대상이 아니다(ADR-0118 결정 3)"
        );
        assert_eq!(
            l.contract_outcome_for_test("r0", tid("bob")),
            Some("awaiting_reply")
        );

        let out = l.fail_open_requests_from(tid("alice"));
        assert!(
            !out.failed.iter().any(|(id, _)| id == "r0" || id == "over"),
            "표시된 계약(r0=은퇴 예정 · over=잠정)은 종결 목록에 없어야: {out:?}"
        );
        assert!(
            out.guard_held >= 2,
            "두 표시가 모두 건너뛴 수로 보고돼야: {out:?}"
        );
        assert_eq!(
            l.contract_outcome_for_test("r0", tid("bob")),
            Some("awaiting_reply"),
            "은퇴 예정 표시 계약의 수명은 그 가드 소유다"
        );
        assert_eq!(
            l.contract_outcome_for_test("over", tid("bob2")),
            Some("awaiting_reply"),
            "잠정 계약도 그대로(기존 커버리지 유지)"
        );
    }

    #[test]
    fn the_general_transition_graph_rejects_both_failed_edges() {
        // ★Mutation probe D9-c★: **adding** `(Delivered, Failed)` and `(Pending, Failed)` to the general transition graph
        //   still left all messaging tests green (the existing illegal-edge test looked only at three unrelated edges).
        let mut l = Ledger::new();
        let now = t0();
        l.record(
            "m-del",
            "a",
            tid("a"),
            "b",
            Some(tid("b")),
            "x",
            DeliveryStatus::Delivered,
            now,
        );
        assert_eq!(
            l.transition("m-del", tid("b"), DeliveryStatus::Failed, now),
            Err(TransitionError::Illegal {
                from: DeliveryStatus::Delivered,
                to: DeliveryStatus::Failed
            }),
            "delivered→failed 는 범용 전이에서 불법(§6)"
        );
        assert_eq!(
            l.records_for("m-del")[0].status,
            DeliveryStatus::Delivered,
            "거부된 전이는 상태를 바꾸지 않는다"
        );
        l.record(
            "m-park",
            "a",
            tid("a"),
            "b",
            Some(tid("b")),
            "x",
            DeliveryStatus::Pending,
            now,
        );
        assert_eq!(
            l.transition("m-park", tid("b"), DeliveryStatus::Failed, now),
            Err(TransitionError::Illegal {
                from: DeliveryStatus::Pending,
                to: DeliveryStatus::Failed
            }),
            "임의 pending→failed 호출은 거부된다(간선은 fail_pending 에만 열려 있다)"
        );
        assert_eq!(l.records_for("m-park")[0].status, DeliveryStatus::Pending);
        assert!(
            l.records_for("m-park")[0].fail_code.is_none(),
            "거부된 전이는 사유 코드도 남기지 않는다"
        );
    }

    #[test]
    fn the_requester_cleanup_closes_open_contracts_but_respects_guards_and_replied() {
        let mut l = Ledger::new();
        let now = t0();
        let (a, b, c) = (tid("w1"), tid("w2"), tid("w3"));
        let gone = PeerId::new_v4();
        l.record(
            "m-open",
            "gone",
            tid("gone"),
            "w1",
            Some(tid("w1")),
            "q",
            DeliveryStatus::Delivered,
            now,
        );
        l.open_request("m-open", "gone", gone, "w1", a, None, now);
        l.commit_open(Some(("m-open", tid("w1"))), None);
        l.open_request("m-done", "gone", gone, "w2", b, None, now);
        l.commit_open(Some(("m-done", tid("w2"))), None);
        l.record(
            "m-done",
            "gone",
            tid("gone"),
            "w2",
            Some(tid("w2")),
            "x",
            DeliveryStatus::Delivered,
            now,
        );
        assert_eq!(l.close_on_reply("m-done", b, now), ReplyOutcome::Closed);
        // Provisional contract — not committed, so the guard remains.
        l.open_request("m-prov", "gone", gone, "w3", c, None, now);
        l.open_request("m-other", "someone", a, "w4", tid("w4"), None, now);
        l.commit_open(Some(("m-other", tid("w4"))), None);
        // ★Pick by id (ADR-0309 decision 6)★: it catches its own contract opened under the pre-rename name, and does not close someone else's contract with the same name.
        l.open_request("m-old-name", "old-name", gone, "w5", tid("w5"), None, now);
        l.commit_open(Some(("m-old-name", tid("w5"))), None);
        l.open_request(
            "m-namesake",
            "gone",
            tid("namesake"),
            "w6",
            tid("w6"),
            None,
            now,
        );
        l.commit_open(Some(("m-namesake", tid("w6"))), None);

        let out = l.fail_open_requests_from(gone);
        assert_eq!(
            out.failed,
            vec![
                ("m-open".to_string(), tid("w1")),
                ("m-old-name".to_string(), tid("w5")),
            ],
            "그 id 가 요청자인 오픈 계약만 종결 — 보낼 때 이름과 무관: {out:?}"
        );
        assert_eq!(
            l.contract_outcome_for_test("m-namesake", tid("w6")),
            Some("awaiting_reply"),
            "같은 이름의 다른 에이전트가 요청자인 계약은 그대로"
        );
        assert_eq!(out.guard_held, 1, "잠정 계약은 건너뛴 수로 보고된다");
        assert_eq!(
            l.contract_outcome_for_test("m-open", tid("w1")),
            Some("reply_failed")
        );
        assert_eq!(
            l.contract_outcome_for_test("m-done", tid("w2")),
            Some("replied"),
            "★되돌리지 않는다★ — 수용 = 완료(spec §3 항목 7-④)"
        );
        assert_eq!(
            l.contract_outcome_for_test("m-prov", tid("w3")),
            Some("awaiting_reply"),
            "가드 보유 계약은 그대로"
        );
        assert_eq!(
            l.contract_outcome_for_test("m-other", tid("w4")),
            Some("awaiting_reply"),
            "남의 계약은 무관"
        );
        assert_eq!(
            l.occupied_slots(),
            3,
            "m-prov(잠정) + m-other + m-namesake 만 자리를 차지한다"
        );
    }
}
