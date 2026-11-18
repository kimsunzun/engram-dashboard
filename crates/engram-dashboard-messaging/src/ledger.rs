//! ledger — message history + request reply tracking + group delivery ledger (spec §2·§3·§5).
//!
//! ★Three axes★:
//!   ① **History ring buffer** — state transitions + timestamps of every message (`pending→delivered→replied` / `expired` / `skipped`);
//!      transition time is the reply/send time data (not exposed in the envelope — spec §5).
//!   ② **Request tracking** — opens `awaiting_reply`, closes by **strict matching** on `in_reply_to`, computes `reply_by` overruns (here only
//!      "who exceeded"). ★Two closing reasons (ADR-0116 decisions 2/3, ADR-0118 decision 1)★: reply accepted (`replied`) or failure close
//!      (`reply_failed` — reply hits `RECIPIENT_NOT_FOUND` / requester profile deletion cleanup). Missing the deadline is **not** a
//!   closing reason (ADR-0108 decision 1).
//!   ③ **Group delivery ledger** — 1 message : N delivery records (spec §4); dead members as `skipped`.
//!
//! ★Pure, injected clock (load-bearing)★: every state transition and timeout decision takes `now: Instant`; the ring and tracking map hold
//!   no clock,
//! so TTL and reply-by boundaries are asserted in deterministic tests.
//!
//! ★Strict reply matching (spec §2, ADR-0103 invariant)★: a reply closes a request only when `in_reply_to` points **exactly** at an open
//!   request id;
//! lenient matching (next message from a non-replying peer) was rejected as an accidental-close misfire.
// ADR-0103

use std::collections::{HashSet, VecDeque};
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

use crate::PeerId;

/// History ring buffer capacity — the oldest record is evicted first (spec §5).
/// ★4096★: the unit is a **delivery record**, not a message — one broadcast writes N `(msg_id, member)` records (spec §4), so 100
///   broadcasts to a
/// 10-member group filled the old 1024: history got too short and expiry/recall transitions fell to `NotFound`, never reaching the ledger
///   (C4 review fix J).
/// ★Memory upper bound★: a record keeps the full body, so worst case = `MAX_BODY_BYTES` 64KiB × 4096 ≈ **256MiB**; human-sized bodies come
///   to ~1–2MB.
/// In-memory stage only; a tuning target. Follow-ups (not implemented): ⓐ runtime setting ⓑ truncated bodies for audit.
/// ★Evict vs request tracking (C3 review fix 3)★: an evict used to drop the tracking entry, so an open request whose history was pushed
///   out lost both
/// its reply path and its deadline notice (silent contract loss). The bound now comes from `MAX_OPEN_REQUESTS`, not this capacity.
const HISTORY_CAPACITY: usize = 4096;

/// Upper bound on request contracts open at once (not replied, not notified).
/// ★Why (counterpart of fix 3)★: once open contracts survive history evict, history capacity no longer bounds the tracking list; without a cap,
/// never-replied requests pile up. At the cap a **new request is bounced** (old contracts are not silently dropped).
/// ★512★: a rough figure above mailbox cap 100 × a few dozen concurrent recipients; reaching it at human message rates means non-replying
///   peers piling up,
/// which the sender should see as a bounce. Tuning target like HISTORY_CAPACITY.
const MAX_OPEN_REQUESTS: usize = 512;

/// State of one message delivery (spec §5·§6 state vocabulary — do not invent new vocabulary).
/// ★Transitions (load-bearing)★: `Pending → Delivered → Replied` (request only) / `Expired` (TTL) / `Skipped` (notice lane retirement) /
///   `Failed`.
/// Each leaves a timestamp (spec §5). Busy waiting, injection-failure parking and **sleep parking** (ADR-0116 decision 1) are all `Pending`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryStatus {
    Pending,
    /// Actual injection done (delivered = the moment of actual injection, ADR-0104 invariant).
    Delivered,
    Replied,
    /// Parking expired by exceeding TTL (24h) (the ledger keeps the record, spec §5, ADR-0105).
    Expired,
    /// ★Ledger-only terminal★ — the oldest notice was retired because the notice lane (`NOTICE_CAP`) overflowed (spec §6). Not in the
    ///   send-response vocabulary (ADR-0111 decision 3).
    Skipped,
    /// ★Per-recipient failure terminal (ADR-0111 decision 3, spec §5 "failed recipients stay in the ledger too")★: entry bounce
    ///   (`RECIPIENT_NOT_FOUND`/`RECIPIENT_AMBIGUOUS`),
    /// `MAILBOX_FULL`, `REQUEST_CAPACITY`, **deletion cleanup** (`RECIPIENT_DELETED`).
    /// ★Stays in the ledger (load-bearing)★: the sender must be able to look up "who did not receive it" via `eg_messages{id}`, and only
    ///   then does
    /// **expected row count = response row count** hold (no `may_be_truncated` false positives; spec §5·§6). No parking, but a record.
    /// ★Recorded as terminal — with exactly one exception (ADR-0116 decision 4)★: entry bounces are `record`ed in this state and never
    ///   leave it; only
    /// deletion cleanup closes an *already parked* record after the fact, via the one edge `Pending → Failed`.
    // ADR-0111, ADR-0116
    Failed,
}

impl DeliveryStatus {
    /// Is the transition from this state to `next` **legal** (spec §5 graph — load-bearing). Every other edge is illegal: re-transition
    ///   out of a **terminal**
    /// (`Replied`/`Expired`/`Skipped`/`Failed`), reverts (`*→Pending`), skip-aheads (`Pending→Replied`, `Expired→Delivered`) and
    ///   self-transitions (meaningless timestamp
    /// refresh); they would pollute "transition time = reply/send time" (misfire closes, retroactive timestamps).
    /// ★No edge into `Failed` in this **general** graph (ADR-0111, kept by ADR-0116)★: a failure row is recorded `Failed` from the start;
    ///   a general edge would make
    /// "delivered, then turned into failure" expressible, so `Pending → Failed` (deletion cleanup) is opened only through the dedicated
    ///   verb `fail_pending` (`can_fail_by_cleanup`).
    // ADR-0116
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

    /// ★Deletion-cleanup-only edge (`Pending → Failed`, spec §6, ADR-0116 decision 4)★. ★Separate predicate (load-bearing)★: in the
    ///   general graph (`can_transition_to`)
    /// any caller bug (expiry sweep, flush misbranch) could turn a pending row into a reasonless failure; split, that ability lives only
    ///   in `fail_pending`, and
    /// `Delivered → Failed` is in neither predicate, so it stays illegal.
    fn can_fail_by_cleanup(self) -> bool {
        matches!(self, DeliveryStatus::Pending)
    }
}

/// One history record — the delivery history of one (message, recipient) pair.
/// ★1 message : N delivery records (spec §4, load-bearing)★: a group send shares `msg_id` and has one record per recipient with
///   independent status and timestamps; a single send is N=1.
/// ★body is kept in full★: no separate store in this in-memory stage, and re-injecting a parked envelope and `messages { id }` lookup
///   (spec §6) need the original;
/// memory is capped by HISTORY_CAPACITY (summary/offload revisited with v2 SQLite persistence).
/// ★Rows are picked by id, shown by send-time name (ADR-0309 decision 4)★: the pick axis (`transition`, `fail_pending`) is `(msg_id,
///   to_id)`; `from`/`to` are name snapshots, so a rename does not change a lookup.
#[derive(Debug, Clone)]
pub struct MessageRecord {
    pub msg_id: String,
    /// Sender name — snapshot at send time (WYSIWYA — ADR-0101).
    pub from: String,
    /// Sender agent id. A daemon notice (`<notice>`) is nil (service `daemon_identity`).
    pub from_id: PeerId,
    /// Recipient name — snapshot at send time. For a failure row where the recipient was not found, it is the spelling the sender wrote.
    pub to: String,
    /// Recipient agent id. `None` = a failure row where the recipient was not found (none / duplicate name) — already terminal, never
    ///   picked by id.
    // ADR-0309
    pub to_id: Option<PeerId>,
    pub body: String,
    pub status: DeliveryStatus,
    pub created_at: Instant,
    pub transitioned_at: Instant,
    /// ★Total delivery records this logical message will leave (fixed at send time — round-2 review F3)★: single send / notice = 1, group
    ///   = member count N; every row of one `msg_id`
    /// carries the same value. ★Why★: a lookup must answer "are the remaining rows all of them?", but group rows are recorded in two
    ///   phases and other messages' rows can slip between,
    /// so rows of one msg_id are not contiguous and the position-based proof ("not the front → complete") gives a false negative; pinning
    ///   N turns it into the deterministic
    /// `remaining rows < expected`. ★u16★: roster-sized; 2 bytes × 4096 = 8KiB.
    // round-2 review F3
    pub expected_rows: u16,
    /// ★Failure code carried in lookups (spec §6 `RECIPIENT_DELETED`)★. ★Holds the wire string as is★: the code first appears not in the
    ///   send response but in the `eg_messages{id}` lookup
    /// (at send time the row was `pending`), so it must be kept on the record. Canonical vocabulary = `service::FailCode::as_str` (a
    ///   `&'static str` — the pure storage layer must not know
    /// service's enum). ★Only deletion cleanup fills it★: entry-bounce `failed` rows already delivered code/hint in the send response (`None`).
    // ADR-0116
    pub fail_code: Option<&'static str>,
}

/// One open-request tracking entry (spec §3) — a **separate map** from history, so ring evict does not affect it.
/// ★notified (prevents double notification)★: once a `reply_by` overrun is reported through `due_timeouts` it is set and not reported
///   again (spec §7); a reply sets `closed`,
/// dropping it from `due_timeouts` targets.
// R1: `reservation_token`(Weak) has no meaning under value comparison, so the PartialEq/Eq derive was dropped.
#[derive(Debug, Clone)]
struct RequestEntry {
    request_id: String,
    /// The display name **at send time**.
    sender: String,
    /// ★The request sender's PeerId (C3 review fix 2)★: the queue key under which the overrun notice is entrusted (ADR-0309 — an id, so a
    ///   rename in between still reaches them);
    /// `notified` prevents re-fire, so missing it silently halves the contract.
    sender_id: PeerId,
    /// The recipient name **at send time** (lookups, notice body). The contract is picked by `recipient_id`.
    recipient: String,
    /// ★Recipient axis of the contract key = agent id (ADR-0309 decisions 1·7)★ — the id for a live recipient and, for a sleeping one,
    ///   that profile's id from the moment of opening
    /// (the daemon passes the sleeping side as an (id, name) pair). No name-fallback: a rename or a same-name agent leaves the obligation
    ///   with this id.
    /// ★epoch is not carried★: a restart keeps the PeerId and only raises the epoch (ADR-0007); narrowing by epoch would make the
    ///   obligation vanish.
    // ADR-0309
    recipient_id: PeerId,
    /// Reply deadline = (offset from send, **original spelling the sender wrote**). `None` = no deadline.
    /// ★Spelling kept too (C3 review fix 6)★: back-computing it from the Duration normalized `60m` to `1h`, so the envelope
    ///   (`reply-by="60m"`) and notice text disagreed;
    /// one tuple makes the type enforce "deadline ⇒ spelling".
    reply_by: Option<(Duration, String)>,
    created_at: Instant,
    closed: bool,
    /// ★Failure-close marker (ADR-0116 decisions 2/3, ADR-0118 decision 1)★: set **always together with** `closed`; `true` ⇒ terminal
    ///   vocabulary `reply_failed`, not `replied` (contract axis, spec §6).
    /// ★One bit next to `closed`★: the consequences (leave the open list, no deadline-sweep fire, leave the 512 count) equal `replied`, so
    ///   the single `closed` predicate is reused
    /// (ADR-0118 decision 4 "keep the single `occupied_slots` predicate"); this bit carries only the **reason** and the rule that deletion
    ///   cleanup does not touch an already `replied`
    /// contract (§3 item 7-④). ★Exactly two firing paths★: ① reply row `RECIPIENT_NOT_FOUND` (`fail_on_undeliverable_reply`) ② requester
    ///   profile deletion cleanup
    /// (`fail_open_requests_from`); any other reply failure is a no-op.
    // ADR-0116, ADR-0118
    reply_failed: bool,
    notified: bool,
    /// ★**Marked for retirement** under cap pressure (round-5 mark-and-sweep)★ — still in the list and can still receive a reply;
    ///   physically removed only at commit, and rollback clears only the mark.
    /// ★Why a mark★: the old design took the victim out at reservation time, so in that window ① a legitimate reply missed with `NoMatch`
    ///   in `close_on_reply` ② rollback re-inserted it "open",
    /// creating a ghost state and a false notice.
    // round-5 mark-and-sweep
    pending_retirement: bool,
    /// ★A new contract whose acceptance is not yet confirmed (round-5 mark-and-sweep)★ — cleared at commit; on bounce or panic the
    ///   rollback removes the entry.
    /// ★Why★: it occupies a slot but is not entitled to take someone else's place; without the mark a concurrent send could pick this
    ///   unconfirmed contract as "oldest retirable" and remove it,
    /// leaving the original send delivered but with no contract (every reply `NoMatch`). So it is excluded from victim selection.
    // round-5 mark-and-sweep
    provisional: bool,
    /// ★Contract key of the victim this reservation marked for retirement (F1)★ — `Some((msg_id, recipient_id))`. ★Why remember it★: an
    ///   unsettled reservation must be reclaimable by the periodic
    /// sweep (RAII Drop does nothing if `try_lock` fails — `service::Reservation` header), which must know whom it marked; the caller's
    ///   `RetiredContract` is lost with the reservation.
    marked_victim: Option<(String, PeerId)>,
    /// ★Owner liveness token of this reservation (R1)★ — `Some(weak)` = a guard is attached, `None` = never attached. The sweep's reclaim
    ///   criterion (`reclaim_abandoned_reservations`): if it
    /// upgrades the owner is working and the entry is **not touched** (rationale + rejected time threshold: `ReservationLiveness` header).
    ///   ★`None` is a reclaim target (fail-safe)★: attachment is in
    /// the same lock section as `open_request`, so a provisional `None` means the opener released the lock without a guard — a reservation
    ///   nobody will settle that would permanently shrink the cap denominator.
    // R1
    reservation_token: Option<Weak<()>>,
}

impl RequestEntry {
    /// ★Criterion = `!closed` alone (D review B3 — was `!closed && !notified`)★: a contract whose overrun notice went out (`notified`)
    ///   used to count as "finished" and was dropped on history evict, but
    /// the notice only means the sender was told — the recipient still owes an answer (spec §3 "reply even if late") and the pending
    ///   lookup keeps showing it as `timed_out=true` (`open_requests` filters
    /// `!closed`). With diverging criteria the obligation evaporated: ① request opens ② 4096 entries evict its history row ③ deadline
    ///   passes, `notified=true` ④ `purge_finished_without_history`
    /// deletes it as "finished, no history" → gone from the pending list, a silent loss. ★Bound kept★: notified unreplied contracts count
    ///   toward `MAX_OPEN_REQUESTS` (512); at the limit a new request
    /// is bounced with `REQUEST_CAPACITY`.
    // review B3
    fn is_live(&self) -> bool {
        !self.closed
    }

    /// ★Does it occupy a `MAX_OPEN_REQUESTS` slot (round-5 mark-and-sweep, round-6 I1)★ — the sole criterion of the cap decision.
    /// - **A contract marked for retirement is not counted**: its slot already belongs to the new contract; counting it would hold the cap
    ///   after the mark and make the retirement pointless.
    /// - **A provisional contract is counted**: it is a real intake; uncounted, simultaneous sends would all see room and breach the limit.
    /// - ★**A closed provisional contract keeps being counted until settlement** (round-6 I1, load-bearing)★: `!closed` alone loses the
    ///   slot the moment a reply arrives in the provisional window,
    ///   which breached the limit: ① A marks V1, inserts provisional PA (512) ② a fast reply closes PA (511) ③ B sees room and inserts PB
    ///   without marking (512) ④ A rolls back, V1 mark released (+1),
    ///   deleting closed PA gives 0 → **513**. The slot is reserved as the price of accepting the send and holds until the guard settles
    ///   (commit/rollback).
    /// ★Settlement arithmetic★: **Commit** — mark released; if the contract was closed its slot is released then, legitimately.
    ///   **Rollback** — remove the closed provisional (-1) and release the victim mark (+1) → 0.
    /// ★`reply_failed` drops out automatically (ADR-0118 decision 4 — no second capacity concept)★: failure close sets `closed`. Do not
    ///   add a separate branch — the cap count is a single predicate
    /// and the settlement arithmetic holds only here.
    // round-5 mark-and-sweep, round-6 I1, ADR-0108, ADR-0118
    fn occupies_slot(&self) -> bool {
        (!self.closed || self.provisional) && !self.pending_retirement
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplyOutcome {
    /// An open request was closed exactly (the first valid reply); history also transitioned to `Replied`.
    Closed,
    /// The contract was closed but the history `Replied` transition is an illegal edge and did not happen — for anomaly observation.
    /// ★Separate variant (finding 1)★: the reply actually happened and tracking is the source of truth, so the contract stays closed; but
    ///   if history is still e.g. `Pending` and cannot take
    /// `Delivered → Replied`, that is not hidden as `Closed` — it is returned with `from` (the history state then) so MessagingService can
    ///   observe and log it.
    ClosedHistoryAnomaly { from: DeliveryStatus },
    /// No matching open request — a wrong id, or already closed/nonexistent (strict: closes nothing).
    NoMatch,
    /// A second reply to an already closed request — no-op (the first reply already fulfilled the contract).
    AlreadyClosed,
}

/// ★Outcome of a reply-failure close attempt (spec §3 item 7-④, ADR-0118)★. The three other than `Failed` are all **no-ops**; the caller
///   differs only in the log
/// (no effect on delivery or the response — the reply message itself was already reported as a failure row).
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
    /// Number of contracts skipped because of a guard (provisional / marked for retirement) — they do not re-fire (one shot at deletion
    ///   time, spec §5).
    pub guard_held: usize,
}

/// ★Typed error (load-bearing)★: the old `transition` returned a `bool` and silently performed illegal transitions (reverts, `Expired →
///   Delivered`), polluting "transition time = reply/send time";
/// now an illegal transition is rejected by type so the upper layer notices at once (spec §5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransitionError {
    /// No (msg_id, to) record — evicted or nonexistent.
    NotFound,
    Illegal {
        from: DeliveryStatus,
        to: DeliveryStatus,
    },
}

/// ★One contract **marked for retirement** under cap pressure (round-2 review F1)★ — material for the caller's out-of-lock instrumentation
///   log (no silent disappearance).
/// ★Arises★: unreplied contracts hit `MAX_OPEN_REQUESTS`, a new request came, and a **retirable** contract exists (no remaining notice
///   promise to the sender); the oldest gives up its place
/// (same pattern as mailbox/notice lane). ★Holds values only★: the victim never left the list, so rollback only clears its mark.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetiredContract {
    pub request_id: String,
    pub sender: String,
    pub recipient: String,
    pub recipient_id: PeerId,
    /// Age at marking time (for logs — elapsed time, not wall clock).
    pub age: Duration,
}

/// ★Reservation liveness token (R1 — replaces the time threshold with a **liveness decision**)★. The strong side is held by the
///   `service::Reservation` guard; the ledger sees only a `Weak`:
/// upgrade succeeds while the guard lives and fails once it is destroyed unsettled.
/// ★Why liveness, not time (load-bearing)★: a reservation spans "decide under lock → inject outside lock → settle under lock", and
///   **injection is unbounded** (a child's stdin can block forever
/// under backpressure — agent `stdio.rs`), so any "over N seconds means abandoned" constant can reclaim a **still-working** reservation:
///   the later injection succeeds, `commit` finds the contract
/// gone, and a `type="request"` envelope is delivered with no contract (`awaiting_their_reply` 0, `reply_owed_by_me` 0, no deadline
///   notice, a later reply `NoMatch`, history `Delivered` forever).
/// Liveness removes that race structurally. ★The threshold constant was deleted — do not revive it★ (unbounded injection).
// ADR-0108
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

/// ★What the commit **actually did** (R2)★ — a fact, not a plan. ★Why (instrumentation pollution)★: `commit_open` used to return nothing
///   and the caller assumed the planned retirement happened,
/// but a marked victim can vanish before commit (closed by a reply and its history row pushed out → `purge_finished_without_history`;
///   known residual in `rollback_open`), so a retirement that
/// did not happen was reported, contaminating the sole evidence of retirement (ADR-0108 decision 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommitOutcome {
    /// Did it actually clear the provisional mark (= was that contract still in the list).
    pub confirmed: bool,
    /// Did it **actually physically remove** the marked victim = did the retirement happen.
    pub retired: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenOutcome {
    Opened,
    /// Opened a new request (provisionally), and under cap pressure **marked the oldest retirable contract** for retirement.
    /// ★Caller obligation (round-5, load-bearing)★: the mark is provisional — call exactly one of ① `commit_open` on acceptance (remove
    ///   the victim + clear the mark; log instrumentation then)
    /// ② `rollback_open` on every other exit (clear the mark + remove the provisional contract). ★Structural guarantee owner of ② = Drop
    ///   of `service::Reservation` (RAII)★ (ADR-0108 decision 3):
    /// unsettled → debug blows up, release attempts rollback with an error log (a plain `Option<Option<_>>` had nobody to catch a
    ///   forgotten branch).
    OpenedAfterMarking(RetiredContract),
    /// Rejected because the same request_id already exists in tracking (open or closed) — no-op. Ids are daemon-generated unique, so reuse
    ///   is a non-scenario (a lenient reopen caused a shadowing bug).
    DuplicateId,
    /// ★Open-contract cap (`MAX_OPEN_REQUESTS`) reached (C3 review fix 3)★ — no contract opened (no-op); the upper layer shows a bounce
    ///   rather than silently dropping an old contract.
    Full,
}

/// `drop_request` result — removed or not + **whether the contract was already `notified`** (C3 review fix 5). ★Why notified too★:
///   reclaiming on bounce means "never established", but a `notified`
/// entry means the overrun notice already went out (not recallable); "noticed and also bounced" is rare but must not pass silently, so the
///   caller can log it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DropOutcome {
    /// Removed the entry. `notified` = was the timeout notice already out at removal.
    Removed { notified: bool },
    /// No such id in tracking (idempotent — no-op).
    NotFound,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DueTimeout {
    pub request_id: String,
    /// Name of the sender who will receive the notice (display name **at send time** — may have been renamed since).
    pub sender: String,
    /// ★Sender id for notice delivery (C3 review fix 2)★ — queue key for the notice + flush doorbell target (`RequestEntry.sender_id`).
    pub sender_id: PeerId,
    /// Recipient name (**at send time** — for the notice body).
    pub recipient: String,
    /// The recipient axis of the contract key — the re-check "did it close in the meantime" just before notifying finds the contract by it.
    pub recipient_id: PeerId,
    /// ★Original spelling of the exceeded deadline (C3 review fix 6)★: the spec §1 notice template exposes `deadline ({reply_by})` as
    ///   written (`"60m"`).
    pub reply_by_raw: String,
}

/// ★Lookup view of one unreplied request (`eg_messages` no-argument "my pending")★. ★`RequestEntry` is not exposed★: it holds internal
///   state (closed/notified flags, sender_id wiring) and
/// would tie the lookup surface to it (and to v2 persistence); only needed facts are copied. ★`notified` is carried★: an overdue, notified
///   contract is still unreplied (closes on reply);
/// dropping it would leave "something to answer but not in the list", so it is listed with the notice fact so the upper layer can tell it apart.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenRequestView {
    pub request_id: String,
    pub sender: String,
    /// The request sender's PeerId — a pending lookup tells "requests I made" apart **by id, not name** (review B1).
    pub sender_id: PeerId,
    pub recipient: String,
    /// How a pending lookup tells apart "the obligation I owe an answer" (ADR-0309 — by id, not name).
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
    /// ★Has an evict happened even once (D review B2 — lookup honesty)★. Not "how many were cut": that data is gone, so it honestly says
    ///   only "it may have been truncated".
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

    /// Capacity-injected — unit tests verify the evict boundary with a small capacity (purity principle).
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            history: VecDeque::new(),
            requests: Vec::new(),
            // capacity 0 would always evict at once — raised to 1 (defensive).
            capacity: capacity.max(1),
            evicted_any: false,
        }
    }

    /// ★Initial state is a parameter★: a single/group send starts `Pending` or `Delivered` (immediate-injection fallback), a dead group
    ///   member `Skipped` (spec §4·§5).
    /// ★evict = front (oldest) + clean up **only reply-closed contracts** of the same msg_id (C3 fix 3 → D review B3)★: dangling cleanup +
    ///   bound. ★An unreplied contract survives evict, notified or not★ —
    /// dropping notified ones let a past-deadline contract evaporate from the pending list (sequence: `RequestEntry::is_live`). The
    ///   contract's source of truth is tracking, not history
    /// (ReplyOutcome), so history capacity must not kill it; the unreplied bound is `MAX_OPEN_REQUESTS` (`Full` of open_request).
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

    /// `record` + **expected delivery record count** (round-2 review F3). A group fan-out records in two phases (plan lock / per-member
    ///   section), so both must pass the same N.
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
            // 0 is meaningless (every send has ≥1 row) — raised to 1 so `rows < expected` cannot be contradictory ("rows exist but expected 0").
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
            // Lookup honesty flag (B2) — once set never lowered (discarded data does not return).
            self.evicted_any = true;
            self.purge_finished_without_history();
        }
    }

    /// ★Removes tracking entries of **closed** (replied) contracts that have no history to point at (round-final fix 1, load-bearing)★.
    ///   ★Prevents zombie entries★: a closed entry is normally
    /// cleaned when same-msg_id history is evicted (`record`), but since unreplied contracts survive evict, a contract whose history was
    ///   pushed out first has already missed that event when it closes;
    /// it has `is_live()` false, so it is outside `MAX_OPEN_REQUESTS` too and would pile up unbounded. So at close and at evict we check
    ///   for history and delete orphans.
    /// ★Unreplied contracts are never touched (D review B3)★: notified ones are not "finished" (a notice is not a reply — deleting them
    ///   evaporated the obligation, see `RequestEntry::is_live`);
    /// `is_live() == !closed`, and their bound is `MAX_OPEN_REQUESTS`.
    // review B3, ADR-0108
    fn purge_finished_without_history(&mut self) {
        if self.requests.iter().all(|r| r.is_live() || r.provisional) {
            return;
        }
        let live_ids: HashSet<&str> = self.history.iter().map(|r| r.msg_id.as_str()).collect();
        // ★A provisional contract is **never deleted here** (round-6 I1, load-bearing)★: reservation is **before** dispatch and history
        //   recording **inside** it, so in the provisional window there may be
        //   no history row; a reply closing the contract then would match "finished, no history" and delete the entry — the reserved slot
        //   evaporates (I1's cap breach through another door) and the
        //   guard's rollback gets `NotFound`. An unconfirmed entry's lifetime is owned by the guard.
        self.requests
            .retain(|r| r.is_live() || r.provisional || live_ids.contains(r.request_id.as_str()));
    }

    /// ★Pick by (msg_id, recipient id)★: a group broadcast has N records per msg_id, so the recipient is needed to transition exactly one
    ///   (1:N accounting, spec §4); the axis is the id, so a renamed
    /// recipient still matches (ADR-0309).
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

    /// ★Deletion-cleanup-only close (`Pending → Failed` + reason code — spec §5·§6)★. ★The reason code is a required argument★ so a lookup
    ///   never sees a codeless `failed`.
    /// ★`NotFound` (record pushed out of the ring)★ is logged at debug by the caller outside the lock — no silent loss.
    // ADR-0116
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

    /// Request open — starts `awaiting_reply` tracking (spec §3 step 2). **Contract key = `(request_id, recipient_id)`** (ADR-0309 —
    ///   `recipient` is only a send-time name snapshot).
    /// ★Multi-recipient request = N independent contracts (ADR-0111 decision 5, load-bearing)★: the message id is common and the contract
    ///   splits by replier, so this is called with the same
    /// `request_id` **once per recipient**; none is opened for an undeliverable recipient (entry bounce, `MAILBOX_FULL`, `REQUEST_CAPACITY`).
    /// ★reply_by clock = from send (spec §3·§5, ADR-0104)★: absolute deadline = `created_at(now) + reply_by`, independent of receive delay.
    /// ★Duplicate key rejection, open or closed (finding 2)★: an existing `(request_id, recipient_id)` is rejected with `DuplicateId`
    ///   (no-op); ids are daemon-unique and recipients are deduplicated at resolution
    /// (spec §5 ④), so reuse is a non-scenario. The old leniency "reopen a closed id" left two entries (a reply met the earlier closed one
    ///   first → `AlreadyClosed` misfire; same-id history evict dropped
    /// the reopened tracking). ★A parked recipient also opens a contract (spec §3 item 2)★ — busy or **sleeping** (ADR-0116 decision 1):
    ///   acceptance is acceptance, the obligation and deadline sweep hold.
    /// ★At cap = let the oldest retirable contract go and accept (round-2 F1)★; `Full` only when nothing is retirable. ★Why (B3's hole)★:
    ///   notified unreplied contracts have neither TTL nor cancel, so if
    /// 512 fill up **every** new request is blocked with `REQUEST_CAPACITY` until restart; relieved like the mailbox/notice lane "retire
    ///   the oldest".
    /// ★Retirable = no remaining notice promise★: (a) `notified == true` or (b) `reply_by == None`. ★Never retired★: a contract with a
    ///   deadline not yet notified — a **debt the daemon owes the sender**;
    /// deleting it silently breaks the promised notice. ★History ring rows are not touched★ (retirement concerns contract tracking only).
    // round-2 review F1, ADR-0309
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
            // ★Two kinds removed from the candidates (round-5, load-bearing)★: ① already marked (`pending_retirement`) — two sends aiming
            //   at one victim would have one commit delete the other's victim,
            //   leaving its rollback/commit pointing at thin air (and the count off) ② **provisional** (`provisional`) — see that field.
            // ADR-0108
            // ★"Oldest" is by `created_at`★, not list position; `min_by_key` returns the first on a tie, so append order is the tie-break.
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
                // ★`Full` with only provisional contracts left is also honest★: the cap really is full, and a slot opens soon when the
                //   racing sender commits/rolls back.
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

    /// ★Attach the owner liveness token to the provisional reservation just opened (R1)★ — call in the **same lock section** as
    ///   `open_request` (a sweep in between would read the entry
    /// as abandoned — `RequestEntry::reservation_token`). No-op if the entry is missing (not in normal flow).
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

    /// ★Reservation confirm (commit) — physically remove the marked victim and clear the provisional mark (round-5)★. A victim closed by a
    ///   reply meanwhile is simply removed (`replied` is terminal and
    /// history already holds `Replied`; tracking is accounting). Victim not found → no-op (not in normal flow). ★Return = what was
    ///   actually done (R2)★: callers must not assume the planned retirement
    /// happened (`CommitOutcome` header).
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
            // ★Decide **before** purge (R2)★: purge also deletes entries, so comparing lengths afterwards would mix "what I retired" with
            //   "what vanished by zombie cleanup".
            out.retired = self.requests.len() < before;
            self.purge_finished_without_history();
        }
        out
    }

    /// ★Reservation cancel (rollback) — clear only the mark and remove the provisional contract (round-5)★. Return = the result of
    ///   removing the provisional contract (the caller logs outside the lock).
    /// ★No state to revert★: the victim never left the list, so clearing one mark bit gives "nothing happened" — the old re-insert
    ///   (restore position/age) machinery and its "restored but someone used the
    /// slot" failure mode are gone. A victim closed by a reply meanwhile keeps its close (legitimate reply; no ghost reopening, no late
    ///   false notice — `due_timeouts` skips `closed`).
    /// ★Both in one lock section★: no window where the count looks over the cap (root of the old split-lock 513 drift).
    /// ★Known residual — id ABA (round-6 note, no machinery added)★: a retirement-marked victim closed by a reply whose history row is
    ///   also pushed out may be deleted by
    /// `purge_finished_without_history` (provisional contracts are excluded from it); if before the guard settles a new contract is born
    ///   with the **same random id** (36^8 space, microsecond-millisecond
    /// window) and also gets marked, the release here clears the new contract's mark. Damage = one retirement cancelled (next send marks
    ///   again); preventing it needs the id reservation set (deleted in round-4) —
    /// judged not worth that probability: **a residual with a price, not a miss**.
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

    /// ★Why every send looks at this (C3 review fix 12, load-bearing)★: only request sends caught id collisions (`open_request`
    ///   DuplicateId), but the id is also the history correlation key and reply matching
    /// key — a notify/reply overlapping an existing id makes `records_for`/`transition` pick someone else's record and the observation
    ///   record smear two messages, so the check is made at the reservation
    /// point for every kind. ★Cost★: a linear scan of the ring (≤ HISTORY_CAPACITY) + tracking; a separate id set would need two
    ///   structures synced on every evict/close/remove (mismatch = silent false
    /// positive) and the message rate is human-level — simplicity chosen (index at v2 persistence).
    pub fn msg_id_in_use(&self, msg_id: &str) -> bool {
        self.history.iter().any(|r| r.msg_id == msg_id)
            // ★Retirement-marked and provisional contracts are right here (round-5)★ — physical removal was dropped, so the ordinary
            //   tracking lookup suffices (no separate `reserved_ids`).
            || self.requests.iter().any(|r| r.request_id == msg_id)
    }

    /// ★Use (C3 review fix 5 — narrowing the timeout↔reply race)★: the upper layer re-checks just before parking the notice — if a reply
    ///   closed the contract meanwhile, no "no reply" notice. A nonexistent
    /// id gives false on purpose: tracking gone (evict etc.) means the timeout did happen. ★Residual (interaction with fix 1)★: a contract
    ///   whose history was already evicted is removed from tracking when it closes
    /// (zombie prevention), so this is false afterwards and the notice can go out once more — a microsecond window after HISTORY_CAPACITY
    ///   was pushed out; the zombie-removal bound was chosen.
    pub fn is_request_closed(&self, request_id: &str, recipient_id: PeerId) -> bool {
        self.requests
            .iter()
            .any(|r| r.request_id == request_id && r.recipient_id == recipient_id && r.closed)
    }

    /// ★Marks (retirement-marked, provisional) do not obscure matching (round-5, load-bearing)★: they are for accounting, not existence;
    ///   only `!closed` is looked at, so ① a legitimate reply to a
    /// retirement-marked contract closes normally (physical removal made it `NoMatch`: answered but unclosed, possible false deadline
    ///   notice) ② a fast reply to an unconfirmed provisional contract closes too;
    /// later commit/rollback does not reverse it. ★Contract picked by the replier (ADR-0111 decision 5)★: one `request_id` can have
    ///   several contracts (multi-recipient), so `in_reply_to` alone is not
    /// enough; spec §3 closes only **that replier's contract** (no reply-all) → key `(request_id, replier id)`.
    /// ★"Not my contract" = `NoMatch` = delivery unchanged (spec §3 item 7-②)★: unknown id, already closed, or a contract I do not have →
    ///   only the contract side is a no-op; the reply message itself is delivered
    /// (vanishing would be worse).
    // L1, ADR-0309 (decision 7 — no name-fallback matching)
    pub fn close_on_reply(
        &mut self,
        in_reply_to: &str,
        replier_id: PeerId,
        now: Instant,
    ) -> ReplyOutcome {
        // Closes the tracking entry (source of truth); the history row is picked by the same key `(msg_id, recipient id)` (ADR-0309).
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

    /// ★The **single rule** for picking the replier's contract — `close_on_reply` and `fail_on_undeliverable_reply` share this function
    ///   (load-bearing)★: they are two outcomes of one reply (accepted → `replied` /
    /// unreachable confirmed → `reply_failed`); diverging selection rules would point the same reply at different contracts (the A6
    ///   class). ★Pick by id alone — never fall back to the name (ADR-0309 decision 7)★:
    /// a contract carries an id from opening even when addressed to a sleeper; name fallback would let same-name twins or a new holder of
    ///   an old name close someone else's contract.
    // ADR-0309
    fn match_contract_for_replier(&self, in_reply_to: &str, replier_id: PeerId) -> Option<usize> {
        self.requests
            .iter()
            .position(|r| r.request_id == in_reply_to && r.recipient_id == replier_id)
    }

    /// ★The reply is **confirmed unreachable**, so the contract is closed as failed (spec §3 item 7-④, ADR-0116 decision 2, ADR-0118)★ —
    ///   called only when the reply row is `RECIPIENT_NOT_FOUND`
    /// (requester in neither roster nor profiles). ★No other reply failure calls it★: `MAILBOX_FULL`/`RECIPIENT_AMBIGUOUS` are the
    ///   environment of that moment and a retry may deliver — a permanent "reply failed"
    /// would be a lie (ADR-0118 decision 2); that kind is a no-op (contract stays open; not a zombie because the deadline notice grants
    ///   ADR-0108 retirement eligibility).
    /// ★Guard first (ADR-0118 decision 3, ADR-0108 decision 4)★: contracts with a `provisional` or `pending_retirement` mark are not
    ///   touched (`GuardHeld`) — the guard owns their lifetime; closing here would
    /// ① let the guard's commit/rollback revive a terminated contract ② let someone else terminate the marked victim, halving the cap exchange.
    /// ★Delivery state untouched (spec §6)★: `reply_failed` is a contract-axis terminal; the request's delivery record stays `delivered`
    ///   (or `pending`) — transitioning history would make "delivered, then failure" expressible.
    // ADR-0116, ADR-0118
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

    /// ★Requester profile deletion cleanup — closes open contracts where that id is the **requester** as `reply_failed`★ (spec §5 ②,
    ///   ADR-0116 decision 3, ADR-0309 decision 6).
    /// ★Requester (`sender`) side only★: the reply destination is gone, so the outcome is settled; **contracts whose replier (`recipient`)
    ///   was deleted are kept** — the sender still learns of no response through the
    /// deadline notice (spec §3 items 4~5), and closing would erase "no answer received". ★Closed contracts untouched★: `replied` is not
    ///   reverted (spec §3 item 7-④ — even if a parked reply is later cleared as
    /// `RECIPIENT_DELETED`); the `!closed` filter guarantees it. ★Guard first (ADR-0118 decision 3)★: provisional/marked contracts are
    ///   skipped and counted (no re-fire after settlement — a one shot at deletion time;
    /// the residue ends by TTL, spec §5). ★Match by `sender_id`, not the name (ADR-0309 decision 6)★: `sender` is the send-time display
    ///   name, so by name a pre-rename contract is missed and a same-name agent's
    /// contract closed; the trigger gate (no live session for that profile) is decided by the caller with the same id
    ///   (`service::handle_profile_deleted`).
    // ADR-0116, ADR-0118, ADR-0309
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

    /// ★**Removes** an open request tracking entry entirely (C3 — send bounced, contract never established)★. ★Separate from
    ///   `close_on_reply` (bounded guarantee)★: a closed entry stays and is dropped with its
    /// same-msg_id history evict (`record`), but a bounced send never had a history record, so with closing alone nothing would ever evict
    ///   it and repeated bounces would grow tracking without bound; "never
    /// established" is also semantically a no-trace erase.
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

    /// ★Reclaims **abandoned** reservations (F1 guarantee layer, R1 criterion)★: rolls back whole the provisional contracts whose owner
    ///   guard is gone (remove the contract + clear the victim mark); returns
    /// the reclaimed keys. ★This is the guarantee, RAII Drop is not★: `Reservation::drop` rolls back only when `try_lock` succeeds; under
    ///   contention or same-thread-holding-the-lock it does nothing, leaving
    /// ① a provisional contract skipped by `due_timeouts` forever (deadline notice lost) ② a marked victim outside `occupies_slot()` (cap
    ///   denominator shrinks permanently) ③ unbounded tracking growth.
    /// The periodic sweep (the only maintenance point that properly owns the lock) redoes the job — Drop is the fast path. ★Idempotent★ (a
    ///   settled entry is not provisional). ★Not destructive★: a provisional
    /// contract is either not yet reported accepted (response lost via panic) or already reported as a failure row — rolling back is right
    ///   either way.
    // ADR-0108
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
            // Mark clear + provisional removal = **the same rule** as rollback (same verb, no divergence).
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

    /// ★Boundary★: `>` comparison — the exact deadline instant is not yet due (same convention as the mailbox TTL boundary; deterministic
    ///   tests).
    /// ★Retirement-marked contracts are not skipped, **provisional contracts are** (round-5 → round-6 I2)★:
    ///   - A retirement-marked contract can never be due (structural): victim eligibility `notified || reply_by.is_none()` and due
    ///   eligibility `!notified && reply_by.is_some()` are **disjoint**, so a mark check would be a dead branch.
    ///   - ★Provisional contracts are explicitly skipped (round-6 I2, load-bearing)★: the window spans reservation to the settlement of
    ///   that recipient's outcome (service.rs `PendingContract`; commit moves after
    ///     the outcome) and includes the child stdin `write_all`, which under pipe backpressure can block indefinitely (`stdio.rs`). A
    ///   1-minute-deadline request could be swept inside the window and a notice
    ///     sent, then the send bounced — the sender would hold an overrun notice for a request that officially never existed, and a notice
    ///   cannot be recalled.
    ///   - ★No loss★: the skipped contract keeps its original `created_at`; after commit the next sweep (60 s) sees the passed deadline
    ///   and notifies immediately — delayed, not lost. After rollback the contract never
    ///     was, so no notice is right.
    // ADR-0108
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

    /// ★`records_for` + **incompleteness signal** (D review B2, replaced by round-2 review F3)★ — `(rows, truncated)`. ★Why★: when the
    ///   ring (4096) fills the front rows vanish silently, yet `messages { id }` would show
    /// the rest as all of that message — 6 of 10 broadcast rows pushed out reads as "went to only 4". ★Decision = `remaining rows <
    ///   expected rows`★ (deterministic — `MessageRecord.expected_rows`).
    /// ★Rows vanished entirely are not visible here★: an empty list; the upper layer answers with the contract view or `MESSAGE_NOT_FOUND`
    ///   (its hint announces the rotation).
    // review B2, round-2 review F3
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

    /// All history records (oldest first) — observation/test snapshot for assertions that do not know msg_id (e.g. "did the notice stay in
    ///   the ledger"); lookups with a msg_id use `records_for`.
    pub fn all_records(&self) -> Vec<&MessageRecord> {
        self.history.iter().collect()
    }

    /// ★Contracts currently occupying a `MAX_OPEN_REQUESTS` slot (round-6 I1)★ — exactly what the cap decision looks at. **Differs** from
    ///   `open_request_count`: retirement-marked contracts drop out and provisional ones
    /// stay even if closed (`occupies_slot`). Assert cap arithmetic with this.
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

    /// Test-only (H2) — **number of contracts carrying a retirement mark**. Marks leave `occupies_slot()`, so a non-zero remainder
    ///   permanently shrinks the cap denominator (observes a forgotten settlement —
    ///   `service::Reservation` header). Counts only "is a mark left", not "who" (no duplicated victim logic).
    #[cfg(any(test, feature = "test-harness"))]
    pub fn marked_retirement_count_for_test(&self) -> usize {
        self.requests
            .iter()
            .filter(|r| r.pending_retirement)
            .count()
    }

    /// ★Contract-axis terminal vocabulary (spec §6 `awaiting_reply`/`replied`/`reply_failed`)★; `None` if not tracked. `replied` and
    ///   `reply_failed` leave the open list, deadline sweep and cap count identically, so
    ///   asserting "acceptance counts as complete, only confirmed unreachable is failure" (§3 item 7-④) and "deletion cleanup does not
    ///   revert `replied`" needs the reason itself.
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

    /// Test-only (C2) — does that contract key exist in tracking (closed or retirement-marked alike); observes the provisional/retirement
    ///   window.
    #[cfg(any(test, feature = "test-harness"))]
    pub fn is_tracked_for_test(&self, request_id: &str, recipient_id: PeerId) -> bool {
        self.requests
            .iter()
            .any(|r| r.request_id == request_id && r.recipient_id == recipient_id)
    }

    /// ★Tests only★: production callers hold both parts of the contract key; this reverse lookup just fits old "one-id contract" tests to
    ///   the new key.
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

    /// **Total** tracking entries (including finished — observation/tests); asserts the no-zombie bound (fix 1) since `open_request_count`
    ///   cannot see a leak.
    pub fn tracking_len(&self) -> usize {
        self.requests.len()
    }

    /// ★All unreplied (open) requests as lookup views (`eg_messages` no-argument)★, **oldest first** (`created_at` ascending, ties keep
    ///   list order — stable sort).
    /// ★Explicit sort (round-4 review H4)★: "append order = send order" holds only with a monotonic clock, not as a property of this
    ///   structure (injected clock), and a misorder would appear only on the rare
    /// restore path; with ≤512 entries sorting is negligible. Not a double sort: `open_items_for` re-sorts after merging three branches.
    /// ★Marked contracts are visible (round-5, explicit choice)★: a retirement-marked contract is still open until commit (hiding it would
    ///   report a removal that may never commit); a provisional one is a real intake
    /// (a bounce removes it then). ★Inclusion = `!closed` (= `is_live()`, load-bearing)★: whether it was notified is a field, not a
    ///   filter; **if this criterion changes, `is_live()` must change with it**
    /// (see its 4-step sequence). The sent/received branch is the caller's filter.
    // ADR-0103
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

    /// Adapter putting old "one id = one contract" tests on the contract key (ADR-0111 decision 5) — **that contract's recipient replied
    ///   in person**.
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

    /// Sender PeerId (fix 2) — most assertions ignore it, so a fresh one is drawn each time.
    fn sid() -> PeerId {
        PeerId::new_v4()
    }

    /// Deadline tuple (fix 6) — the spelling is **written by the sender**, not made from a Duration; a conventional spelling where
    ///   assertions ignore it.
    fn rb(d: Duration) -> Option<(Duration, String)> {
        Some((d, format!("{}s", d.as_secs())))
    }

    /// A cap fixture that forgot to commit stays entirely provisional and yields only `Full`; production always settles
    ///   (`service::commit_contract`), so tests do the same.
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

    /// A contract with no history exists only after evict — tests not aiming at that use this helper to create history alongside
    ///   (production state).
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
            "premise: a contract that has finished up to inject"
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
        assert_eq!(l.history_len(), 3, "capacity cap kept");
        assert!(l.records_for("m0").is_empty(), "evicted oldest first");
        assert!(l.records_for("m1").is_empty());
        assert_eq!(l.records_for("m4").len(), 1, "the recent ones remain");
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
        assert_eq!(rec.transitioned_at, later, "transition time recorded");
        assert_eq!(rec.created_at, now, "send time unchanged");
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
        assert_eq!(recs.len(), 3, "3 delivery records for one msg_id");
        let a = recs.iter().find(|r| r.to == "a").unwrap();
        let b = recs.iter().find(|r| r.to == "b").unwrap();
        let c = recs.iter().find(|r| r.to == "c").unwrap();
        assert_eq!(a.status, DeliveryStatus::Delivered);
        assert_eq!(b.status, DeliveryStatus::Pending, "b is untouched");
        assert_eq!(c.status, DeliveryStatus::Skipped, "dead member skipped");
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
            "Pending → Expired is legal"
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
            "Expired → Delivered is illegal"
        );
        assert_eq!(
            l.records_for("e1")[0].status,
            DeliveryStatus::Expired,
            "an illegal transition does not change the state"
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
            "Pending → Replied is illegal (must go through Delivered)"
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
            "Replied → Pending is illegal (reverting a terminal)"
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
            "Skipped → Pending is illegal"
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
        assert_eq!(l.open_request_count(), 0, "closed by the reply");
    }

    #[test]
    fn strict_reply_wrong_id_does_not_close() {
        let mut l = Ledger::new();
        let now = t0();
        l.open_request("req-1", "alice", sid(), "bob", tid("bob"), None, now);
        assert_eq!(reply(&mut l, "req-999", now), ReplyOutcome::NoMatch);
        assert_eq!(
            l.open_request_count(),
            1,
            "a wrong id must not close the request"
        );
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
            "**a different recipient** of the same message id = an independent contract (multi-recipient request)"
        );
        assert_eq!(
            l.open_request("req-1", "alice", sid(), "bob", tid("bob"), None, now),
            OpenOutcome::DuplicateId,
            "reopening the same contract key (id, recipient) is refused"
        );
        assert_eq!(
            l.open_request_count(),
            2,
            "open contracts = only (req-1,bob)·(req-1,carol) — a duplicate key is not added to tracking"
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
            "reopening a closed id is refused (uniqueness premise)"
        );
        assert_eq!(l.open_request_count(), 0, "not reopened, so 0 open");
        assert_eq!(
            reply(&mut l, "req-1", now),
            ReplyOutcome::AlreadyClosed,
            "replying without reopening gives AlreadyClosed (no shadowing)"
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
            "question",
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
        assert_eq!(rec.status, DeliveryStatus::Replied, "history to Replied");
        assert_eq!(
            rec.transitioned_at, reply_at,
            "transition time = reply time (spec §5)"
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
            "an illegal history transition is exposed as an anomaly (no hiding)"
        );
        assert_eq!(
            l.open_request_count(),
            0,
            "the contract still closes (reopening would be worse)"
        );
        assert_eq!(
            l.records_for("req-1")[0].status,
            DeliveryStatus::Pending,
            "the history is untouched because it is an illegal transition (stays Pending)"
        );
    }

    #[test]
    fn close_on_reply_with_evicted_history_is_plain_closed_not_anomaly() {
        let mut l = Ledger::with_capacity(1);
        let now = t0();
        // Tracking is a separate map from history, so a contract can open without `record` — req-1 has no history row from the start.
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
            "no history to point to (NotFound) is not an anomaly → Closed"
        );
        assert_eq!(l.open_request_count(), 0, "contract closed");
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
            "exactly at the deadline is not due"
        );
        let due = l.due_timeouts(now + reply_by + Duration::from_nanos(1));
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].request_id, "req-1");
        assert_eq!(
            due[0].sender, "alice",
            "the notice goes to the sender (spec §3)"
        );
        assert_eq!(due[0].recipient, "bob");
    }

    #[test]
    fn due_timeout_carries_sender_id_and_raw_notation() {
        // The spelling is not normalized — `60m` must stay so the envelope reply-by and notice text agree.
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
        assert_eq!(
            due[0].sender_id, sender,
            "sender id attached (hint in case of rename)"
        );
        assert_eq!(
            due[0].reply_by_raw, "60m",
            "original notation as is (no normalizing to 1h)"
        );
    }

    #[test]
    fn due_timeout_no_double_notification() {
        let mut l = Ledger::new();
        let now = t0();
        let reply_by = Duration::from_secs(600);
        open_delivered_request(&mut l, "req-1", rb(reply_by), now);
        let over = now + reply_by + Duration::from_secs(1);
        assert_eq!(
            l.due_timeouts(over).len(),
            1,
            "the first computation reports"
        );
        assert!(
            l.due_timeouts(over).is_empty(),
            "the second call does not emit an already notified request again (prevents double notice)"
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
            "a replied request is not a timeout target"
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
            "a closed item stays in tracking and blocks reopening"
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
            "removal succeeded — it was before the notice, so notified=false"
        );
        assert_eq!(
            drop_req(&mut l, "dropped-1"),
            DropOutcome::NotFound,
            "idempotent — the second is NotFound"
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
            "a removed id can be opened again (contract not established = no trace)"
        );
        assert_eq!(
            l.open_request_count(),
            1,
            "the only open contract is the one just made"
        );
    }

    #[test]
    fn request_without_reply_by_never_times_out() {
        let mut l = Ledger::new();
        let now = t0();
        l.open_request("req-1", "alice", sid(), "bob", tid("bob"), None, now);
        let far = now + Duration::from_secs(100_000);
        assert!(
            l.due_timeouts(far).is_empty(),
            "a request without a deadline has no timeout"
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
        assert_eq!(l.history_len(), cap, "the history is capacity-bounded");
        assert!(
            l.records_for("done").is_empty(),
            "premise: the done history was evicted"
        );
        assert!(
            !l.msg_id_in_use("done"),
            "tracking of a finished contract is dropped along with the history evict (stays bounded)"
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
        assert!(
            l.records_for("req-1").is_empty(),
            "premise: the history was evicted"
        );
        assert_eq!(l.open_request_count(), 1, "a live contract survives evict");
        let due = l.due_timeouts(now + reply_by + Duration::from_secs(1));
        assert_eq!(
            due.len(),
            1,
            "even after evict, the timeout notice is alive"
        );
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
            "even if the history was evicted, the reply closes the contract (only the history to point to is missing)"
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
        assert!(
            l.records_for(id).is_empty(),
            "premise: that contract's history is evicted"
        );
        assert_eq!(
            l.tracking_len(),
            1,
            "premise: a live contract survives evict"
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
            "removes the orphan tracking entry the moment it closes (no zombies)"
        );
        assert!(!l.msg_id_in_use("req-1"), "in neither tracking nor history");
    }

    #[test]
    fn timeout_notice_after_history_eviction_keeps_the_unanswered_contract_tracked() {
        let now = t0();
        let reply_by = Duration::from_secs(600);
        let mut l = ledger_with_evicted_live_contract(2, "req-1", rb(reply_by), now);
        let due = l.due_timeouts(now + reply_by + Duration::from_secs(1));
        assert_eq!(
            due.len(),
            1,
            "the notice goes out even after evict (fix 3 kept)"
        );
        assert_eq!(due[0].request_id, "req-1");
        assert_eq!(
            l.tracking_len(),
            1,
            "a notice is not a reply — an unreplied contract stays in tracking (B3)"
        );
        let open = l.open_requests();
        assert_eq!(open.len(), 1, "must stay in the outstanding list: {open:?}");
        assert!(
            open[0].notified,
            "the notice fact is told apart only by a flag"
        );
        assert_eq!(reply(&mut l, "req-1", now), ReplyOutcome::Closed);
        assert_eq!(
            l.tracking_len(),
            0,
            "removes the orphan item the moment it closes"
        );
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
            assert!(l.records_for(&id).is_empty(), "premise: history evicted");
            assert_eq!(reply(&mut l, &id, now), ReplyOutcome::Closed);
            assert_eq!(
                l.tracking_len(),
                0,
                "tracking converges to 0 each round (no zombie accumulation)"
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
            "all got deadline-exceeded notices (= the sender was already told the outcome → retirable)"
        );
        assert_eq!(
            l.tracking_len(),
            MAX_OPEN_REQUESTS,
            "stays if unreplied even after being notified (B3)"
        );

        let outcome = l.open_request("over", "alice", sid(), "bob", tid("bob"), None, over);
        match outcome {
            OpenOutcome::OpenedAfterMarking(r) => {
                assert_eq!(r.request_id, "r0", "marked for retirement oldest first");
                assert_eq!(r.sender, "alice");
                assert_eq!(r.recipient, "bob");
            }
            other => panic!("must be accepted after marking: {other:?}"),
        }
        assert!(
            l.open_requests().iter().any(|r| r.request_id == "r0"),
            "before commit, the victim is still an open contract"
        );
        assert_eq!(
            l.tracking_len(),
            MAX_OPEN_REQUESTS + 1,
            "+1 during the marked section (the provisional one)"
        );
        commit(&mut l, Some("over"), Some("r0"));
        assert!(
            !l.open_requests().iter().any(|r| r.request_id == "r0"),
            "it retires only at commit"
        );
        assert_eq!(
            l.tracking_len(),
            MAX_OPEN_REQUESTS,
            "stays bounded (512 unchanged)"
        );
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
            "a contract without a deadline owes no notice, so it is retirable: {outcome:?}"
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
        assert!(
            l.due_timeouts(now).is_empty(),
            "premise: before the deadline"
        );
        assert_eq!(
            l.open_request("over", "alice", sid(), "bob", tid("bob"), rb(reply_by), now),
            OpenOutcome::Full,
            "bounced if there is no retirable contract (does not break the notice promise)"
        );
        assert_eq!(l.tracking_len(), MAX_OPEN_REQUESTS, "nobody was erased");
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
            "a resolved recipient leaves its id (material for blocking misattribution among many same-named ones)"
        );
        assert_eq!(
            open[1].recipient_id,
            tid("ghost"),
            "asleep parking also holds that profile id from opening (ADR-0309)"
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
        assert!(!truncated, "complete if the expected 1 row is still there");

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
        assert!(!truncated, "complete if 3/3");

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
        assert_eq!(rows.len(), 1, "g2 has only 1 of 3 rows left: {rows:?}");
        assert_ne!(
            l2.all_records()[0].msg_id,
            "g2",
            "premise: the ring front is someone else's row — the layout where the old position proof answered 'complete'"
        );
        assert!(
            truncated,
            "truncated if remaining rows (1) < expected (3) (F3)"
        );
    }

    #[test]
    fn open_request_rejects_at_capacity_with_full() {
        // Fill the cap with **non-retirable** (deadline-pending) contracts — no-deadline ones would trigger a retirement and no Full
        //   (separate test).
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
            "cap reached + nothing retirable → Full"
        );
        assert_eq!(
            l.open_request_count(),
            MAX_OPEN_REQUESTS,
            "existing contracts unchanged"
        );
        assert_eq!(reply(&mut l, "r0", now), ReplyOutcome::Closed);
        assert_eq!(
            l.open_request("over", "alice", sid(), "bob", tid("bob"), rb(reply_by), now),
            OpenOutcome::Opened,
            "a closed contract leaves the cap count"
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
            panic!("premise: accepted after marking");
        };
        assert_eq!(victim.request_id, "r0");

        rollback(&mut l, Some("new"), Some("r0"));
        let after: Vec<String> = l
            .open_requests()
            .into_iter()
            .map(|r| r.request_id)
            .collect();
        assert_eq!(after, before, "the list·order restored to the original");
        let r0 = l
            .open_requests()
            .into_iter()
            .find(|r| r.request_id == "r0")
            .expect("stays as is");
        assert!(r0.notified, "notice flag unchanged (never touched)");
        assert_eq!(r0.created_at, now, "age unchanged");
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
            assert_eq!(l.occupied_slots(), cap, "premise: cap");
            l
        };

        // ── rollback branch ──────────────────────────────────────────────────────────
        let mut l = build();
        let OpenOutcome::OpenedAfterMarking(v1) =
            l.open_request("PA", "alice", sid(), "bob", tid("bob"), None, now)
        else {
            panic!("premise: accepted after marking");
        };
        assert_eq!(v1.request_id, "c0");
        assert_eq!(
            l.occupied_slots(),
            cap,
            "exactly the cap even after mark+insert"
        );
        assert_eq!(reply(&mut l, "PA", now), ReplyOutcome::Closed);
        assert_eq!(
            l.occupied_slots(),
            cap,
            "a closed provisional contract also keeps its spot until settlement (round-6 I1)"
        );
        let b = l.open_request("PB", "carol", sid(), "dave", tid("dave"), None, now);
        assert!(
            matches!(b, OpenOutcome::OpenedAfterMarking(ref v) if v.request_id == "c1"),
            "B must also mark its own victim: {b:?}"
        );
        assert_eq!(l.occupied_slots(), cap);
        rollback(&mut l, Some("PA"), Some("c0"));
        assert_eq!(
            l.occupied_slots(),
            cap,
            "exactly the cap even after rollback — no getting stuck at 513 (round-6 I1)"
        );
        commit(&mut l, Some("PB"), Some("c1"));
        assert_eq!(
            l.occupied_slots(),
            cap,
            "the cap holds even after B commits"
        );

        // ── commit branch ────────────────────────────────────────────────────────────
        let mut l = build();
        let OpenOutcome::OpenedAfterMarking(v1) =
            l.open_request("PA", "alice", sid(), "bob", tid("bob"), None, now)
        else {
            panic!("premise");
        };
        assert_eq!(v1.request_id, "c0");
        assert_eq!(reply(&mut l, "PA", now), ReplyOutcome::Closed);
        assert_eq!(l.occupied_slots(), cap, "keeps its spot before settlement");
        commit(&mut l, Some("PA"), Some("c0"));
        assert_eq!(
            l.occupied_slots(),
            cap - 1,
            "a contract finished by reply gives up its spot at commit time (not an excess — room was freed)"
        );
        assert!(
            l.occupied_slots() <= MAX_OPEN_REQUESTS,
            "no branch exceeds the cap"
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
            "a provisional contract is not a sweep target (round-6 I2)"
        );
        assert!(l.open_requests().iter().all(|r| !r.notified));

        commit(&mut l, Some("p"), None);
        let due = l.due_timeouts(over);
        assert_eq!(due.len(), 1, "collected right away after commit");
        assert_eq!(due[0].request_id, "p");
        assert_eq!(
            due[0].reply_by_raw,
            format!("{}s", reply_by.as_secs()),
            "original notation as is"
        );

        let mut l = Ledger::new();
        l.open_request("q", "alice", sid(), "bob", tid("bob"), rb(reply_by), now);
        assert!(l.due_timeouts(over).is_empty());
        rollback(&mut l, Some("q"), None);
        assert!(
            l.due_timeouts(over + Duration::from_secs(9999)).is_empty(),
            "a bounced request has no deadline notice"
        );
    }

    #[test]
    fn a_concurrent_opener_cannot_select_someone_elses_provisional_entry_as_victim() {
        let mut l = Ledger::new();
        let base = t0();
        // Fill the cap with retirable (no deadline) contracts but leave **only the oldest** retirable, the rest non-retirable → it is B's
        //   only legal victim.
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
            "a provisional contract must not be picked as the victim — if there is nothing to pick, Full is the right answer: {b:?}"
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
                panic!("premise: accepted after marking in every cycle");
            };
            assert_eq!(
                victim.request_id, "r0",
                "cycle {k}: the same oldest one is always marked"
            );
            let dropped = rollback(&mut l, Some(provisional.as_str()), Some("r0"));
            assert_eq!(
                dropped,
                Some(DropOutcome::Removed { notified: false }),
                "cycle {k}: the provisional contract was removed within the same section"
            );
            assert_eq!(
                l.open_request_count(),
                MAX_OPEN_REQUESTS,
                "cycle {k}: the contract count is exactly the cap (does not leak to 513)"
            );
            assert!(
                !l.open_requests()
                    .iter()
                    .any(|r| r.request_id == provisional),
                "cycle {k}: no provisional contract remains"
            );
            assert!(
                l.open_requests().iter().any(|r| r.request_id == "r0"),
                "cycle {k}: the victim stays as is with only its mark cleared"
            );
        }
        assert_eq!(
            l.tracking_len(),
            MAX_OPEN_REQUESTS,
            "the tracking total is unchanged too"
        );
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
            panic!("premise: accepted after marking under cap pressure");
        };
        assert_eq!(victim.request_id, "r0");
        let live = ReservationLiveness::new();
        l.attach_reservation_liveness("p-live", tid("bob"), live.watch());
        assert_eq!(l.marked_retirement_count_for_test(), 1, "1 mark");

        for round in 0..3 {
            assert!(
                l.reclaim_abandoned_reservations().is_empty(),
                "round {round}: a reservation whose owner is alive is not a sweep target"
            );
        }
        assert!(
            l.is_tracked_for_test("p-live", tid("bob")),
            "must remain as is"
        );
        assert_eq!(
            l.marked_retirement_count_for_test(),
            1,
            "the mark stays too"
        );

        drop(live);
        let reclaimed = l.reclaim_abandoned_reservations();
        assert_eq!(
            reclaimed,
            vec![("p-live".to_string(), tid("bob"))],
            "must tell the caller the reclaimed contract keys (for out-of-lock recording): {reclaimed:?}"
        );
        assert!(
            !l.is_tracked_for_test("p-live", tid("bob")),
            "the provisional contract must be removed"
        );
        assert_eq!(
            l.marked_retirement_count_for_test(),
            0,
            "the victim mark must be released too — if it remains, the cap denominator shrinks forever"
        );
        assert!(
            l.open_requests().iter().any(|r| r.request_id == "r0"),
            "the victim itself stays open (since the exchange did not happen)"
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
            panic!("premise: opened while there is room");
        };
        let live = ReservationLiveness::new();
        l.attach_reservation_liveness("p-ok", tid("bob"), live.watch());
        let committed = l.commit_open(Some(("p-ok", tid("bob"))), None);
        assert!(committed.confirmed, "premise: the commit found the item");
        drop(live);

        assert!(
            l.reclaim_abandoned_reservations().is_empty(),
            "reclaiming a settled contract would leave a request reported as accepted without a contract"
        );
        assert!(
            l.is_tracked_for_test("p-ok", tid("bob")),
            "the contract must stay alive"
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
            panic!("premise: accepted after marking");
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
            "if it actually retired, it must say so"
        );

        let OpenOutcome::OpenedAfterMarking(v2) =
            l.open_request("p2", "alice", sid(), "bob2", tid("bob2"), None, now)
        else {
            panic!("premise: accepted after marking");
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
            "the contract was settled but there was no retirement — this value is the metering condition (R2)"
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
            "the mark does not hide matching (round-5)"
        );
        let tracked_before = l.tracking_len();
        rollback(&mut l, Some("new"), Some("v"));
        assert_eq!(
            l.tracking_len(),
            tracked_before - 1,
            "only 1 provisional contract must go — the rollback must not **erase** the victim (only unmark it)"
        );
        assert!(
            l.msg_id_in_use("v"),
            "a closed victim stays in tracking as is (not deleted)"
        );
        assert!(
            !l.open_requests().iter().any(|r| r.request_id == "v"),
            "closed by the reply, so not outstanding"
        );
        assert!(
            l.due_timeouts(win + Duration::from_secs(99999)).is_empty(),
            "no spurious deadline notice goes out for a closed contract"
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
            "the count after commit is exactly the cap"
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
        assert!(l.records_for("r0").is_empty(), "premise: history evicted");

        assert!(matches!(
            l.open_request("new", "alice", sid(), "bob", tid("bob"), None, now),
            OpenOutcome::OpenedAfterMarking(ref r) if r.request_id == "r0"
        ));
        assert!(
            l.msg_id_in_use("r0"),
            "the marked victim stays in tracking and looks in use"
        );
        assert!(
            l.msg_id_in_use("new"),
            "the provisional contract is in use too"
        );
        commit(&mut l, Some("new"), Some("r0"));
        assert!(!l.msg_id_in_use("r0"));
        assert!(l.msg_id_in_use("new"), "a settled contract stays in use");
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
            "must be oldest first regardless of the insertion order (late→middle→early) (H4)"
        );
        let times: Vec<_> = l
            .open_requests()
            .into_iter()
            .map(|r| r.created_at)
            .collect();
        assert!(
            times.windows(2).all(|w| w[0] <= w[1]),
            "created_at ascending"
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
            panic!("premise");
        };
        assert_eq!(victim.request_id, "r0", "the oldest is the victim");
        rollback(&mut l, Some("new"), Some(victim.request_id.as_str()));
        let times: Vec<_> = l
            .open_requests()
            .into_iter()
            .map(|r| r.created_at)
            .collect();
        assert!(
            times.windows(2).all(|w| w[0] <= w[1]),
            "oldest first even after restoring: {times:?}"
        );
        assert_eq!(
            l.open_requests().first().map(|r| r.request_id.clone()),
            Some("r0".to_string()),
            "the restored oldest contract is at the front again"
        );
    }

    /// ★A contract is picked by the replier id alone (ADR-0309 decision 7)★ — with two contracts holding the same name snapshot, a reply
    ///   from a different id (even a same-name agent) closes neither.
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
            "a third party with only the same name cannot close it"
        );
        assert_eq!(
            l.fail_on_undeliverable_reply("r1", sid()),
            ReplyFailOutcome::NoMatch,
            "failure termination follows the same rule"
        );
        assert_eq!(l.close_on_reply("r1", b, now), ReplyOutcome::Closed);
        assert_eq!(l.contract_outcome_for_test("r1", b), Some("replied"));
        assert_eq!(
            l.contract_outcome_for_test("r1", a),
            Some("awaiting_reply"),
            "the contract of another id stays open"
        );
    }

    #[test]
    fn msg_id_in_use_sees_history_and_tracking() {
        let mut l = Ledger::new();
        let now = t0();
        assert!(!l.msg_id_in_use("m1"), "unused id");
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
        assert!(l.msg_id_in_use("m1"), "in use if in the history");
        l.open_request("r1", "a", sid(), "b", tid("b"), None, now);
        assert!(l.msg_id_in_use("r1"), "in use even if only in tracking");
        open_delivered_request(&mut l, "r2", None, now);
        assert_eq!(reply(&mut l, "r2", now), ReplyOutcome::Closed);
        assert!(
            l.msg_id_in_use("r2"),
            "in use if it remains in history·tracking even when closed"
        );
    }

    #[test]
    fn is_request_closed_only_true_for_closed_entries() {
        let mut l = Ledger::new();
        let now = t0();
        open_delivered_request(&mut l, "r1", None, now);
        assert!(!closed(&l, "r1"), "an open contract is false");
        assert!(
            !closed(&l, "nope"),
            "a nonexistent id is false (does not block the notice)"
        );
        assert_eq!(reply(&mut l, "r1", now), ReplyOutcome::Closed);
        assert!(closed(&l, "r1"), "true once closed by a reply");
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
            "reclaiming an already notified contract carries that fact"
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
        assert_eq!(open[0].request_id, "r1", "oldest first");
        assert_eq!(open[0].sender, "alice");
        assert_eq!(open[0].recipient, "bob");
        assert_eq!(
            open[0].reply_by_raw.as_deref(),
            Some("600s"),
            "the notation is the sender's original as is (no back-calculation)"
        );
        assert_eq!(
            open[0].created_at, now,
            "send time as is (the ledger does not know the wall clock)"
        );
        assert!(!open[0].notified);
        assert_eq!(open[1].request_id, "r2");
        assert_eq!(open[1].reply_by_raw, None, "request without a deadline");
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
        assert_eq!(
            open.len(),
            1,
            "contracts closed by a reply are left out: {open:?}"
        );
        assert_eq!(open[0].request_id, "timedout");
        assert!(
            open[0].notified,
            "the fact that a notice went out is exposed as a field (not excluded from the list)"
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
            "an arbitrary pending→failed call is refused (the edge is open only to the dedicated verb)"
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
            "the reason code remains in the record so a query can answer why"
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
            "delivered→failed is still illegal"
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
            "the provisional contract is not touched"
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
        // ★Review fix D7 — guard coverage gap★: the existing guard tests created only **provisional** ones, so deleting the
        //   `pending_retirement` condition left everything green (measured undefended).
        // ★The mark is created by the **production path**★: a test setter could diverge from how the mark actually arises.
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
            "all notified = retirable"
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
            other => panic!("must be accepted after marking: {other:?}"),
        };
        assert_eq!(marked.request_id, "r0");
        assert_eq!(l.marked_retirement_count_for_test(), 1);

        assert_eq!(
            l.fail_on_undeliverable_reply("r0", tid("bob")),
            ReplyFailOutcome::GuardHeld,
            "a contract marked for retirement is not a target of failure termination (ADR-0118 decision 3)"
        );
        assert_eq!(
            l.contract_outcome_for_test("r0", tid("bob")),
            Some("awaiting_reply")
        );

        let out = l.fail_open_requests_from(tid("alice"));
        assert!(
            !out.failed.iter().any(|(id, _)| id == "r0" || id == "over"),
            "marked contracts (r0=retiring · over=provisional) must not be in the termination list: {out:?}"
        );
        assert!(
            out.guard_held >= 2,
            "both marks must be reported in the skipped count: {out:?}"
        );
        assert_eq!(
            l.contract_outcome_for_test("r0", tid("bob")),
            Some("awaiting_reply"),
            "the lifetime of a contract marked for retirement is owned by its guard"
        );
        assert_eq!(
            l.contract_outcome_for_test("over", tid("bob2")),
            Some("awaiting_reply"),
            "the provisional contract stays too (existing coverage kept)"
        );
    }

    #[test]
    fn the_general_transition_graph_rejects_both_failed_edges() {
        // ★Mutation probe D9-c★: **adding** `(Delivered, Failed)` and `(Pending, Failed)` to the general graph left all messaging tests
        //   green (the illegal-edge test looked at three unrelated edges).
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
            "delivered→failed is illegal in the general transition (§6)"
        );
        assert_eq!(
            l.records_for("m-del")[0].status,
            DeliveryStatus::Delivered,
            "a refused transition does not change the state"
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
            "an arbitrary pending→failed call is refused (the edge is open only to fail_pending)"
        );
        assert_eq!(l.records_for("m-park")[0].status, DeliveryStatus::Pending);
        assert!(
            l.records_for("m-park")[0].fail_code.is_none(),
            "a refused transition leaves no reason code either"
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
        // ★Pick by id (ADR-0309 decision 6)★: it catches its own contract opened under the pre-rename name, and does not close someone
        //   else's contract with the same name.
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
            "only open contracts whose requester is that id are terminated — regardless of the name at send time: {out:?}"
        );
        assert_eq!(
            l.contract_outcome_for_test("m-namesake", tid("w6")),
            Some("awaiting_reply"),
            "contracts whose requester is another agent with the same name stay"
        );
        assert_eq!(
            out.guard_held, 1,
            "the provisional contract is reported in the skipped count"
        );
        assert_eq!(
            l.contract_outcome_for_test("m-open", tid("w1")),
            Some("reply_failed")
        );
        assert_eq!(
            l.contract_outcome_for_test("m-done", tid("w2")),
            Some("replied"),
            "★not reverted★ — accepted = complete (spec §3 item 7-④)"
        );
        assert_eq!(
            l.contract_outcome_for_test("m-prov", tid("w3")),
            Some("awaiting_reply"),
            "guard-held contracts stay"
        );
        assert_eq!(
            l.contract_outcome_for_test("m-other", tid("w4")),
            Some("awaiting_reply"),
            "others' contracts are unrelated"
        );
        assert_eq!(
            l.occupied_slots(),
            3,
            "only m-prov (provisional) + m-other + m-namesake take spots"
        );
    }
}
