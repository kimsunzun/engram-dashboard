//! mailbox — in-memory parking store (spec §5 · ADR-0103 decision 5 · ADR-0104 idle gate · **ADR-0111/0114**).
//!
//! ★Role★: holds **messages not yet injected** in per-recipient FIFO queues. Queue key = recipient **agent id**
//!   (ADR-0309 — a name is turned into an id at the entrance and never gets here). There are **three** parking reasons:
//!     ① busy kinds — the recipient is mid-turn, or joins FIFO behind earlier parked items. Batch flush on entering idle.
//!     ② injection (write) failure — unreachable at that moment. Waits until the next flush trigger (entering idle · reappearance).
//!     ③ **sleeping** — the process is closed but the agent **has a real profile** (restorable). Flush trigger = reappearance
//!        (spawn/restore/epoch — reusing the existing roster diff machinery).
//!   On appearance (spawn/epoch) or entering idle, the upper service empties the whole queue with `drain` and injects it in one batch, oldest first.
//!   ★**Kinds that still do not come in**(ADR-0116 decision 1 — 3 branches)★: a **name that exists nowhere** (in neither
//!   the roster nor the profiles — typo · never spawned · deleted) is a `RECIPIENT_NOT_FOUND` failure row, and **several agents with the same name**, at any layer, is a
//!   `RECIPIENT_AMBIGUOUS` failure row (the recipient cannot be narrowed to one by name).
//!   Turning these two back into parking violates ADR-0116.
//!   ★**A live session without turn signals is not a parking kind — it is a delivery kind**(ADR-0116 decision 7)★: it is injected
//!   at once without a gate, so there is no busy parking, and the only path by which that kind enters this queue is ② (injection failure).
//!   (The `RECIPIENT_UNREACHABLE` bounce of the amendment-4 draft is **discarded** — do not revive it.)
//!
//! ★Invariants★:
//!   - **FIFO (oldest first)** — `drain`/`sweep_expired` preserve park order (ADR-0104 batch, oldest-first flush).
//!   - **Queue ordering axis = admission sequence number (`ParkedMessage.admission_seq`)** — the sequence number is **strictly increasing** from front to back
//!     (`park` appends a new number at the back, and `restore_ordered` puts items back by a merge on the number). The reason the
//!     canonical axis of "oldest first" is this number and not the clock is in the `admission_seq` comment (round-4 finding 1).
//!   - **Capacity = 2 **independent lanes** by kind (round-6 redesign — single accounting with no exceptions)**:
//!       ① `MAILBOX_CAP` (100 messages per recipient) — counts every message.
//!       ② `NOTICE_CAP` (64 notices per recipient) — a message backlog does not block notices, and vice versa.
//!     ★**Pressure reclaim (retiring leftovers of the message lane) was abolished**(ADR-0114 decision 1 — amendment of ADR-0107 decision 2)★:
//!     the basis then was "with binding abolished (ADR-0111 decision 6) + bounce for nonexistent names (decision 1), 'parked items certain to be undeliverable' is
//!     the empty set". ★That logical claim was **weakened** by amendment 4 (ADR-0116)★ — parked items for a sleeping name that is not restored are
//!     *restorable* but not *restored*, and deletion cleanup is a one shot at deletion time, so some stay until TTL. **The bound's basis
//!     is now not a logical empty set but TTL 24h + the lane cap**, and within that we keep reclaim abolished (spec §5. To reopen
//!     reintroducing reclaim, record a new decision on the basis of this sentence). The only cleanup path of the message lane is the **TTL
//!     sweep** (items older than 24h, 60-second period).
//!   - **Denominator = queue + in-flight (F1 — the core of the structural bound)** — a **batch flush carried out of the lock** also counts in that
//!     recipient lane's denominator (`take_in_flight` → `settle_in_flight`).
//!     (The measured scenario and the invariant proof are in the `MAILBOX_CAP` comment.)
//!   - **TTL = 24h** — over-limit items are collected by `sweep_expired` and the upper layer records them in the ledger as `expired` (ADR-0105).
//!   - **Pure + injected clock** — expiry is decided only by the difference between the `park` time and the argument `now` (module header purity invariant).
//!   - **Incarnation binding (`bound_incarnation`) was abolished (ADR-0111 decision 6)** — parked items are **delivered to a new incarnation (epoch)
//!     of the same agent (same id) too** (the same rule as a personal letter — ADR-0309 decision 1). So this store has no binding axis, no
//!     visibility filter (`visible_to`) that looks at that axis, and no liveness snapshot argument. If "a group broadcast goes only to the incarnation at the moment of sending" is
//!     needed again, it will be added non-breakingly as a v2 personal-mail option (spec §8) — do not revive it as a group-only rule.
//!   - **Order = load order (ADR-0125 — amendment 7)** — every send is loaded at the queue tail without exception (no direct-send shortcut),
//!     and the drain pulls from the front, so older items go out first. So this store has **no seat reservation and no "is there something that should go out
//!     first ahead of me" (the old `has_pending_ahead`) join decision** — if those devices become needed again, it is a sign the send path
//!     has split in two. The only remaining ordering device is `in_flight_len`, which is used to
//!     **keep drains from overlapping each other**.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::PeerId;

/// ★Why 24h (spec §5 policy constant, ADR-0105 — raised from 1h)★: per the precedent survey (/research light, 2026-07-25),
///   industry practice is in days (SQS 4 days · Kafka 7 days · Postfix 5 days — 1h was unusually short against that). "Live
///   recipients are exempt from TTL" (liveness-aware) has no precedent in any of the 6 systems surveyed (RabbitMQ · SQS · Kafka · Postfix · ejabberd · LLM
///   frameworks), so it is not adopted — whether sleeping or busy-waiting, the single clock-based rule is kept as is.
///   With the in-memory stage + cap 100 (`MAILBOX_CAP` below), the cost of a long TTL is ~0 (everything vanishes on daemon restart) —
///   **when the persistence (disk) stage arrives, this is a premise to redesign** (user decision, 2026-07-25). The upper service sets the sweep
///   period (only the value is here).
const PARK_TTL: Duration = Duration::from_secs(24 * 60 * 60);

/// ★Message lane limit — 100 per recipient, **counting everything** (round-6)★.
///
/// ★Why 100 (spec §5 policy constant)★: it is the line of defense against a runaway sender inflating one recipient's mailbox without limit and eating memory.
///   Silently drop-heading on overflow would hide the loss (ADR-0103 rejected alternative), so instead new items are
///   bounced at once to make it visible to the sender.
/// ★Why count everything with no exceptions (round-6 — the exceptions created a hole)★: the old implementation subtracted from the denominator "items undeliverable to this
///   incarnation right now (= bound to a different one)". Then each time the same name was swapped for a new PeerId the denominator
///   reset to 0 and **100 per incarnation** were newly accepted (effectively unbounded within the TTL 24h window), and when a **second accounting**, a physical
///   cap, was layered on to stop that, the path that bypasses that cap (`restore_ordered`) and the kind exempt from both
///   caps (notice) each became a new hole. So every exception was removed from the denominator.
/// ★The denominator is **queue + in-flight**, not the queue (F1 — where round-6's bound claim was wrong)★: round-6 wrote "queue
///   length ≤ cap so the total is bounded", but `restore_ordered` (cap-bypassing lossless restore) broke that argument
///   **across cycles**. Measured interleaving: ① flush empties the whole queue under the lock ② while the lock is released and
///   inject runs, the queue is **empty**, so k concurrent sends pass the cap check unhindered ③ inject
///   fails and a batch of N comes back → queue = N + k. The next cycle's batch is N + k, so it grows by **+k** every cycle
///   (the old comment's "the excess is at most one batch" was circular since that one batch already contained the excess).
///   So the denominator adds **in-flight** (= that recipient's share that flush carried out of the lock and has not yet settled).
/// ★The surface of application widened in amendment 7 — **every send now passes this gate** (ADR-0125 §impact · spec §5)★:
///   through amendment 5 this check ran only in the parking branch, and "a direct send that went straight to stdin without entering the queue is not in the denominator,
///   so 101 landing on an idle recipient is not a cap violation" was an explicit contract. With the direct-send shortcut abolished,
///   **no path skips loading anymore, so that contract died with it** — now a live recipient can also get `MAILBOX_FULL`,
///   and that is a **design consequence, not a regression** (the cap value is unchanged — changing it needs a separate decision).
/// ★But this does not mean "it goes up overall"★: when the recipient is idle the drain empties the queue so nothing accumulates and the cap
///   is never hit in the first place, and when the recipient is mid-turn or sleeping, items were parked and piled up in amendment 5 too, so there is no difference. The only newly hit situation is
///   **when several senders pile on at once and the queue passes the cap before the drain catches up**.
/// ★Even so the purpose of this constant is unchanged★: it is "a cap on mail that piles up", not "a limit on injections per second" — if the latter
///   becomes needed, that is a different device called a rate limit.
///   ★Invariant (exact equality — zero slack)★: `queue_lane + in_flight_lane ≤ cap`. The basis is that every verb
///   preserves or reduces this sum — `take_in_flight` raises in-flight by as much as it took from the queue (sum unchanged),
///   `restore_ordered` + the settlement in the same lock section returns to the queue as much as it took from in-flight (sum unchanged), delivery-complete
///   settlement reduces the sum, and `park` bounces if the sum would pass the cap. Only the notice lane can
///   exceptionally have a **temporary +1 overshoot** (the basis is in the `NOTICE_CAP` comment).
const MAILBOX_CAP: usize = 100; // ADR-0107

/// ★Notice lane limit — 64 per recipient (introduced in round-6, **raised from 20 to 64 by ADR-0114 decision 2**)★.
///
/// ★Why a separate lane (replaces the old "cap exception")★: if the deadline-overrun notice of a reply contract is blocked by a full mailbox,
///   the contract is halved (ADR-0103 invariant). The old implementation solved it with an **exemption**, "notice does not count toward the cap", but
///   the exemption meant unbounded — the stated basis, "the open-contract cap (`ledger::MAX_OPEN_REQUESTS`) bounds the notice count",
///   is **false**: `due_timeouts` sets `notified` at once and frees that contract's slot, so while the earlier notices stay
///   parked in the queue the next wave can open contracts anew (repeating without limit if the recipient sleeps for long).
///   Splitting the lanes makes both requirements hold at once — a message backlog cannot block notices (the purpose of the exemption), and notices
///   are not unbounded (the cost of the exemption is removed).
/// ★Why 64 (policy constant — tunable, ADR-0114 decision 2)★: a notice is a **daemon-made infrastructure notice**, so in normal operation there was no reason for several to
///   pile up in the queue, but with **multi-recipient requests** (ADR-0111 decision 5) one send opens N contracts and the deadline-overrun notice also goes out
///   **once per contract** — 20 is narrow for that width (paired with the decision not to build a notice-merging
///   system). Raising the cap reduces the loss window itself. If it still overflows, the **oldest notice** is already of low value (the newest notices
///   carry the same kind of fact with a more accurate time), so instead of bouncing the new one it is reclaimed (the reclaimed one stays in the ledger as `skipped` and
///   is auditable — `ParkAdmitted`).
/// ★Only this lane can have a **temporary +1 overshoot** (F1 — explicit contract)★: the reclaim targets only notices **in the queue**, but
///   the denominator is queue + in-flight, so at the moment all 64 notices are out in a flush, a new notice cannot find anything to reclaim in the queue and is
///   **accepted anyway** (there is no bounce branch). At that moment `queue + in_flight = NOTICE_CAP + 1`. That
///   state returns by itself — the next notice park computes the whole overshoot as `want` and collects it. The message lane has a
///   bounce branch, so it has no such overshoot at all (the sum stops exactly at cap).
const NOTICE_CAP: usize = 64; // ADR-0107

/// Kind of a parked item — the axis that separates **capacity lanes** (round-6). Each kind sees only its own lane's limit (the
///   length of the other lane is invisible to it — module header "independent lanes").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParkKind {
    /// Message from a peer (`<message>`) — the `MAILBOX_CAP` lane.
    Message,
    /// Daemon notice (`<notice>`) — the `NOTICE_CAP` lane.
    Notice,
}

/// ★envelope = the finished envelope string★: this module is a pure store and does **not assemble** envelopes — it keeps and returns as is the finished string made by the upper layer (the
///   `envelope.rs` wrap point that service calls) (single wrap point invariant, ADR-0096). Here it is not known what
///   format that is (opaque text).
#[derive(Debug, Clone)]
pub struct ParkedMessage {
    /// Logical message id for ledger correlation (assigned by the upper layer). The store carries it only as a value.
    pub msg_id: String,
    pub envelope: String,
    pub kind: ParkKind,
    /// Park time (the injected now). The TTL decision basis.
    pub parked_at: Instant,
    /// ★Admission sequence number — the queue ordering axis (round-4 finding 1)★: a monotonically increasing number that `park` assigns at acceptance time.
    ///   **A value the caller supplies is ignored and overwritten** (the store is the only assigner — with two assigners the order diverges).
    ///
    /// ★Why a separate number and not `parked_at` (the clock) (load-bearing)★: re-park merge and expiry decisions are **different
    ///   axes**. Expiry must be judged by the clock (`parked_at`), but judging "who entered the queue first" by the clock goes wrong in two
    ///   places: ① `park_pending` takes `Instant::now()` **before acquiring** the lock, so when two sends race
    ///   the timestamp and the actual acceptance order can invert ② in an environment with coarse clock resolution, consecutive parks get the **same**
    ///   `parked_at` and the order is undecidable. The sequence number is assigned inside the store while the lock is held, so neither problem exists —
    ///   so the merge key of `restore_ordered` is the sequence number, and the queue is always strictly increasing in it.
    pub admission_seq: u64,
}

impl ParkedMessage {
    /// ★Boundary convention (load-bearing)★: `elapsed >= PARK_TTL`, so an item expires from the exact moment TTL has passed (boundary inclusive).
    ///   The reason for `>=` rather than `>` is to be consistent with the upper-bound meaning "TTL = maximum lifetime" —
    ///   an item that has used up the whole TTL has no reason to live longer (expiring immediately at the boundary fits the upper-bound meaning). This boundary is pinned by the unit
    ///   tests (`ttl_boundary_*`) — changing it is a regression.
    /// ★Why `pub(crate)` (F3)★: when the upper layer (`service::park_into`) splits the ledger vocabulary of pressure-reclaimed items, it must
    ///   **reuse this decision as is** (reclaimed past TTL = `expired`, otherwise `skipped`). If the upper layer duplicated the 24h literal
    ///   the two places could diverge, so both the constant and the comparison sign are tied together here and only the function is opened.
    pub(crate) fn is_expired(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.parked_at) >= PARK_TTL
    }
}

/// Park bounce reason — the upper layer maps it to a wire error code (currently the only one: cap exceeded → `MAILBOX_FULL`, spec §5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParkError {
    MailboxFull,
}

/// ★Why a return value (the same pattern as `DrainOutcome`/`ExpiredParked`)★: the store is pure and does not know the ledger. If collected
///   items were silently dropped here, that would be a **hidden loss** (`pending` in the ledger but not in the queue = a ghost pending).
///   So the fact "it was taken out of the queue" is handed to the caller together with the item, so the upper layer leaves `skipped` in the ledger and
///   logs outside the lock — this is the module's existing discipline of preventing loss by the **return value**, not by a path.
/// ★Today **only the notice lane** uses this channel (ADR-0114 decision 1)★: pressure reclaim of the message lane was
///   abolished (module header), so the only remaining reclaim reason is "the oldest notice pushed out by a newer notice". The reason to keep the value channel anyway
///   is that the no-silent-loss discipline is unchanged (the fact of reclaim must always remain as a ledger terminal).
/// ★Vocabulary is **`skipped` by default, but TTL takes priority** (F3)★: for a pushed-out old notice the accurate fact is
///   `skipped` ("not delivered to that recipient"), not `expired` (a clock fact). **However**, because of the gap between the sweep period (60s) and TTL (24h),
///   an item "already past TTL but not yet collected" can be reclaimed, and under the spec §5 contract that is
///   `expired` (the clock decided that item's fate first). So **the store does not decide** — it returns the reclaimed items
///   whole and the upper layer splits the two with `ParkedMessage::is_expired(now)` and transitions (`service::park_into`).
#[derive(Debug, Default)]
pub struct ParkAdmitted {
    /// Items collected by this park to make room (**oldest first**). Usually empty.
    pub retired: Vec<ParkedMessage>,
}

/// ★Why both collections are returned together (load-bearing — spec §5 "expired stays in the ledger")★: if drain silently
///   discarded expired items, they would be recorded nowhere and the loss would be hidden ("no silent loss"). So drain
///   empties the queue (preventing re-park) and also returns the expired items, so the upper layer **injects the injectable ones in one batch, oldest first** and
///   **leaves the expired ones in the ledger as `expired`** (loss is prevented by the return value, not by a path). `sweep_expired`
///   is a separate periodic cleanup path, and what has already expired at drain time is also always entered in the ledger through this return (the two paths complement each other).
#[derive(Debug, Default)]
pub struct DrainOutcome {
    /// Unexpired = targets of the oldest-first batch injection (the upper layer pushes them into stdin as is).
    pub deliverable: Vec<ParkedMessage>,
    /// Past TTL = targets not to be injected but recorded in the ledger as `expired` (oldest first).
    pub expired: Vec<ParkedMessage>,
}

/// ★Why the recipient is returned too (load-bearing — 1 msg_id : N delivery records, spec §4 · ADR-0104)★: the key of a ledger record
///   is `(msg_id, 수신자 id)`. A group broadcast has **N** per-recipient records under one msg_id, so which record an expired
///   item is cannot be identified by msg_id alone — if the upper layer looked up "the first pending record" by msg_id,
///   it could transition **the wrong member's record** to expired (the danger of the C1-era helper, the defect that anchor pointed at).
///   The store is the only place that knows which queue the item was in, so it ships that fact along here.
#[derive(Debug, Clone)]
pub struct ExpiredParked {
    /// The recipient queue key this item was in (= `to_id` of the ledger record).
    pub recipient: PeerId,
    pub msg: ParkedMessage,
}

/// ★In-flight receipt (F1 · load-bearing)★ — the **per-lane count** of the batch a flush carried out of the lock.
///
/// ★Why a value-type ticket★: the messaging lock is **released** between drain and restore (inject is a contract outside the lock). So
///   "how many are out right now" cannot be held as a stack guard and must be held by the store, but the store cannot know by itself who
///   took how many. So a receipt is issued to the taker (`take_in_flight`) and must be handed back
///   (`settle_in_flight`).
/// ★Partial return is possible★: items delivered and items put back within the batch are split off and returned as they happen (`split`) — only then
///   does the denominator equal the real outstanding count.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct FlightTicket {
    message: usize,
    notice: usize,
}

impl FlightTicket {
    /// Is anything left to return (on most paths 0 right away — used to save acquiring the lock at all).
    pub fn is_zero(&self) -> bool {
        self.message == 0 && self.notice == 0
    }

    fn lane_mut(&mut self, kind: ParkKind) -> &mut usize {
        match kind {
            ParkKind::Message => &mut self.message,
            ParkKind::Notice => &mut self.notice,
        }
    }

    fn lane(&self, kind: ParkKind) -> usize {
        match kind {
            ParkKind::Message => self.message,
            ParkKind::Notice => self.notice,
        }
    }

    fn add(&mut self, kind: ParkKind) {
        *self.lane_mut(kind) += 1;
    }

    /// Splits `kinds` worth off this receipt to make a **receipt for partial return**. ★Never split off beyond the remainder★ —
    ///   that is the only device that prevents double return (an accident where the denominator becomes smaller than reality and the cap is breached).
    pub fn split(&mut self, kinds: impl IntoIterator<Item = ParkKind>) -> FlightTicket {
        let mut out = FlightTicket::default();
        for kind in kinds {
            if self.lane(kind) > 0 {
                *self.lane_mut(kind) -= 1;
                out.add(kind);
            }
        }
        out
    }
}

/// Recipient agent id → FIFO queue store.
// ADR-0309 (decision 1 — queue key = agent id)
#[derive(Debug, Default)]
pub struct Mailbox {
    queues: HashMap<PeerId, std::collections::VecDeque<ParkedMessage>>,
    /// ★Per-recipient in-flight count (F1)★ — the share that left the queue but whose terminal (delivered/restored) is not yet fixed.
    in_flight: HashMap<PeerId, FlightTicket>,
    /// Next admission sequence number (a monotonic counter shared by all recipients — the assigner of `ParkedMessage.admission_seq`). Being u64, it is
    ///   practically never exhausted.
    next_seq: u64,
}

/// ★Notice lane pressure reclaim — collect up to `want` of the oldest notices (round-6 · narrowed by ADR-0114)★.
///
/// - The target is **notices only**. Items that do not match the predicate are **never** touched (the guarantee that reclaim does not
///   eat live mail).
/// - **Basis of "oldest first" = the queue ordering axis**: the queue is admission sequence **strictly increasing** (module header invariant), so a front-to-back walk
///   is oldest first. It is the reason no separate sort is done, and it means that if that invariant breaks the order here breaks too.
fn retire_oldest(
    queue: &mut std::collections::VecDeque<ParkedMessage>,
    want: usize,
    mut evictable: impl FnMut(&ParkedMessage) -> bool,
) -> Vec<ParkedMessage> {
    let mut retired = Vec::with_capacity(want);
    let mut kept = std::collections::VecDeque::with_capacity(queue.len());
    for m in std::mem::take(queue) {
        if retired.len() < want && evictable(&m) {
            retired.push(m);
        } else {
            kept.push_back(m);
        }
    }
    *queue = kept;
    retired
}

impl Mailbox {
    pub fn new() -> Self {
        Self::default()
    }

    /// ★Capacity = independent lanes by kind, no exceptions (round-6 redesign · F1 denominator fix · ADR-0114 reclaim abolished)★:
    ///   - **Denominator (common to both lanes)** = the number of that lane's items in the queue **+ that recipient lane's in-flight count** (= the share flush
    ///     carried out of the lock and has not yet settled — `take_in_flight`).
    ///   - **message**: count everything. At `MAILBOX_CAP` or more it is an **immediate** `MailboxFull` bounce **with no reclaim attempt**
    ///     (ADR-0114 decision 1 — reclaim device abolished. The bound's basis is TTL 24h + this cap, module header amendment-4 correction).
    ///     The upper layer reports only that recipient as a failure row (it does not escalate to a whole bounce
    ///     in the multi-recipient case — spec §5).
    ///   - **notice**: messages are not counted in the denominator. At `NOTICE_CAP` or more it reclaims the **oldest notice** and
    ///     accepts the new one — **there is no bounce branch**. Why: `deliver_notice` discards the park result, so bouncing here would make
    ///     the notice **silently vanish** (the reply contract is halved — ADR-0103 invariant). The price is a temporary +1 overshoot when the reclaim target is
    ///     short in the queue (`NOTICE_CAP` comment). The reclaimed items are **returned** via `ParkAdmitted.retired` (the store is pure — stamping the ledger
    ///     terminal is the upper layer's job, no silent loss).
    /// ★The bounce path has zero side effects★: on a bounce it neither touches the queue nor burns a sequence number — consistent with the upper contract "on a bounce nothing is stored
    ///   at all" (nothing stays in the ledger either).
    /// ★The notice `want` can be greater than 1★: `restore_ordered` (re-park) bypasses the cap, so the queue can temporarily
    ///   pass the limit. So instead of "secure 1 slot" it computes **the whole overshoot** and cleans it up at once.
    pub fn park(
        &mut self,
        recipient: PeerId,
        mut msg: ParkedMessage,
    ) -> Result<ParkAdmitted, ParkError> {
        let kind = msg.kind;
        // The fields differ, so this coexists with the mutable borrow of queues (disjoint field borrow).
        let in_flight = self
            .in_flight
            .get(&recipient)
            .map(|t| t.lane(kind))
            .unwrap_or(0);
        let queue = self.queues.entry(recipient).or_default();
        let lane_len = queue.iter().filter(|m| m.kind == kind).count() + in_flight;
        let cap = match kind {
            ParkKind::Message => MAILBOX_CAP,
            ParkKind::Notice => NOTICE_CAP,
        };
        let want = (lane_len + 1).saturating_sub(cap);
        let mut retired = Vec::new();
        if want > 0 {
            match kind {
                ParkKind::Message => return Err(ParkError::MailboxFull),
                ParkKind::Notice => {
                    retired = retire_oldest(queue, want, |m| m.kind == ParkKind::Notice);
                }
            }
        }
        // The admission sequence number is assigned by the store **after acceptance is confirmed** (a bounced item does not burn a number).
        msg.admission_seq = self.next_seq;
        self.next_seq += 1;
        queue.push_back(msg);
        Ok(ParkAdmitted { retired })
    }

    /// ★Can a new park **be accepted now** (pure query — a prediction made possible after ADR-0114)★.
    ///
    /// ★Why it is needed (load-bearing — the premise of freezing the envelope `to`)★: a multi-recipient send can fix the envelope `to` value
    ///   **only after every recipient's acceptance decision is done** (spec §1 — failure rows are not counted), and parking must carry that value in the
    ///   payload, so the order "decide → fix to → park" is forced. That is, the cap result must be known **before** parking. In the era of pressure reclaim
    ///   this prediction was impossible (whether reclaim happened was decided inside park), but
    ///   with reclaim abolished the decision became pure arithmetic — so prediction and reality **never diverge** (as long as it is called within the same lock
    ///   section).
    pub fn can_admit(&self, recipient: PeerId, kind: ParkKind) -> bool {
        let cap = match kind {
            ParkKind::Message => MAILBOX_CAP,
            ParkKind::Notice => return true,
        };
        let in_flight = self
            .in_flight
            .get(&recipient)
            .map(|t| t.lane(kind))
            .unwrap_or(0);
        let queued = self
            .queues
            .get(&recipient)
            .map(|q| q.iter().filter(|m| m.kind == kind).count())
            .unwrap_or(0);
        queued + in_flight < cap
    }

    /// ★Re-park (lossless restore) primitive — cap bypass + admission-sequence merge (ADR-0103/0104 · finding 1)★: puts back the **already admitted**
    ///   items that could not be delivered mid-flush-batch, **mixed in global admission order** with the rest of the queue
    ///   (it is not a simple front insertion — see "why merge" below).
    ///
    /// ★Why a separate primitive and not `park` (load-bearing — no silent loss)★: `park` controls **new
    ///   admission**, so it counts the cap and bounces on overflow. But a re-park is a **deferred restore** of an item already entered in the ledger as `pending`
    ///   (admitted), not a new send — the cap is only inflow control and **not storage control**.
    ///   If the queue is full to the cap again from a concurrent park between drain and inject and we put back with `park`, `MailboxFull` results,
    ///   and ignoring that error silently loses an admitted message (the ledger says pending but it is not in the queue — a ghost pending).
    ///   So re-park **bypasses** the cap (storage is unlimited — only inflow is capped). If an upper-limit violation is a worry, the inflow
    ///   path (`park`) already blocks it, and re-parked items were originally admitted within that cap.
    /// ★The basis of the bound is **not** "at most one batch" but in-flight accounting (F1 — the old argument is discarded)★:
    ///   a batch that drain took out is **still held in the denominator** (`take_in_flight`),
    ///   so the new inflow k that can arrive in the drain↔restore window is already limited by `cap − (큐 + in-flight)`,
    ///   and restore **only moves slots** from in-flight to the queue (it settles that much in the same lock section — service.rs
    ///   `flush_for` failure branch), so the sum does not grow. So this bypass cannot breach the cap — `queue + in_flight ≤ cap`
    ///   still holds. **The property of not counting the cap itself is kept** (lossless restore is the purpose, and the inflow
    ///   limit was already applied on the park side).
    ///   ★Reducing the batch that comes back also still applies★: flush is the only point holding the roster, so **before** restoring
    ///   it collects the items bound to a confirmed-dead incarnation (evidence: a higher epoch of the same PeerId) and stamps `skipped` in the ledger (service.rs
    ///   `flush_for`) — so the returning batch already has the evidenced-dead ones removed.
    /// ★Why merge rather than a simple FRONT insertion (global oldest-first — round-4 finding 1)★: new items can enter the queue
    ///   between restores, and if re-park is called **several times** on the same queue, front insertion puts a later call ahead of an earlier one and
    ///   **inverts** the age order across calls (e.g. restore [m0,m2] → restore [m1,m3] = [m1,m3,m0,m2]). So
    ///   instead of front insertion it puts back by an **admission-sequence merge**, so however many times it is called the queue always keeps global admission order (oldest
    ///   first). Re-parked items have smaller sequence numbers than newly parked ones, so in the single-call, empty-queue case
    ///   the merge result is identical to the old FRONT insertion (no behavior regression).
    /// ★Why the order must be exact (two losses)★: ① the delivery order the recipient sees is inverted (ADR-0104 "oldest-first
    ///   batch" is a promise about the **order the recipient sees**, not the inside of the queue) ② drain pulls **from the queue head**
    ///   (ADR-0125 — order = load order), so an age inversion is directly a delivery-order inversion: the synchronous drain of a following send would
    ///   put out a newer letter before an older one.
    /// ★Precondition (caller contract)★: `items` must be in **ascending sequence order** (as drain emitted them or a subsequence of that —
    ///   the caller guarantees it by index sorting). Merge combines two ascending sequences, so if this precondition breaks the result
    ///   breaks too.
    /// ★Both `parked_at` and the sequence number keep their original values★: the caller passes the original ParkedMessage as is — TTL is not
    ///   extended (misdelivery defense) and the admission order is not reassigned (it is not pushed back).
    /// ★Unconditional acceptance regardless of notice/message★: re-park does not look at kind (already admitted). Cap accounting is for new parks only.
    pub fn restore_ordered(&mut self, recipient: PeerId, items: Vec<ParkedMessage>) {
        if items.is_empty() {
            return;
        }
        let queue = self.queues.entry(recipient).or_default();
        if queue.is_empty() {
            // Common path (restore under the lock = right after drain, so the queue is empty) — no merge needed.
            queue.extend(items);
            return;
        }
        // Ties structurally do not exist (the sequence number has the store as sole assigner and is strictly increasing) — even so, defensively
        //   the re-parked item goes first (the older side).
        let existing = std::mem::take(queue);
        let mut merged = std::collections::VecDeque::with_capacity(existing.len() + items.len());
        let mut restored = items.into_iter().peekable();
        let mut remaining = existing.into_iter().peekable();
        loop {
            let take_restored = match (restored.peek(), remaining.peek()) {
                (Some(r), Some(q)) => r.admission_seq <= q.admission_seq,
                (Some(_), None) => true,
                (None, Some(_)) => false,
                (None, None) => break,
            };
            if take_restored {
                merged.push_back(restored.next().expect("peek 직후"));
            } else {
                merged.push_back(remaining.next().expect("peek 직후"));
            }
        }
        *queue = merged;
    }

    /// Empties the recipient queue and returns **both** the injectable and expired items, oldest first (flush primitive).
    ///   Injection is done only for `deliverable` (unexpired, oldest first) (ADR-0104 batch flush · basis in the `DrainOutcome` comment).
    /// ★Queue removal★: after emptying, the empty queue is removed from the map (so empty queues do not accumulate without limit).
    pub fn drain(&mut self, recipient: PeerId, now: Instant) -> DrainOutcome {
        let Some(queue) = self.queues.remove(&recipient) else {
            return DrainOutcome::default();
        };
        let mut outcome = DrainOutcome::default();
        for m in queue {
            if m.is_expired(now) {
                outcome.expired.push(m);
            } else {
                outcome.deliverable.push(m);
            }
        }
        outcome
    }

    /// ★Registers the batch leaving the lock as in-flight (F1 · load-bearing)★ — the drain reports here only the share of the `drain` result that **actually
    ///   leaves the lock to become an injection target**, and must hand the returned receipt back with `settle_in_flight`.
    ///
    /// ★Why `drain` does not do this automatically and this is a separate verb (intentional)★: not every item drain emits
    ///   leaves the lock — the share for busy targets and the share with no delivery path go back **in the same lock section** right away (`restore_ordered`),
    ///   and confirmed-dead and expired items stamp a ledger terminal on the spot and disappear. If that share were also held as in-flight, the denominator
    ///   would swell without grounds (= reflecting a window that existed only under the lock in the denominator) and normal inflow would be bounced with `MailboxFull`.
    ///   The hole is **only in the span where the lock is released**, so the registration unit is set to exactly that span.
    /// ★Caller contract★: the registered count must be **fully** handed back whether it ends as delivered, restored or abandoned. If a return
    ///   is missed, that lane's denominator swells permanently and inflow is blocked, so the upper layer carries it in a single-exit guard
    ///   (service.rs `FlightSettle` — covers early break and unwinding too).
    pub fn take_in_flight<'m>(
        &mut self,
        recipient: PeerId,
        batch: impl IntoIterator<Item = &'m ParkedMessage>,
    ) -> FlightTicket {
        let mut ticket = FlightTicket::default();
        for m in batch {
            ticket.add(m.kind);
        }
        if !ticket.is_zero() {
            let slot = self.in_flight.entry(recipient).or_default();
            slot.message += ticket.message;
            slot.notice += ticket.notice;
        }
        ticket
    }

    /// ★Why saturating★: panicking here would kill the process with a double panic during unwinding. Over-return is structurally prevented on the
    ///   receipt side (`FlightTicket::split`), so if it ever goes off, **converging the denominator to 0**
    ///   (= the side that does not block inflow) is the safe side — conversely, leaving it would mean that recipient never receives mail again.
    pub fn settle_in_flight(&mut self, recipient: PeerId, ticket: FlightTicket) {
        if ticket.is_zero() {
            return;
        }
        let Some(slot) = self.in_flight.get_mut(&recipient) else {
            return;
        };
        slot.message = slot.message.saturating_sub(ticket.message);
        slot.notice = slot.notice.saturating_sub(ticket.notice);
        if slot.is_zero() {
            self.in_flight.remove(&recipient);
        }
    }

    /// ★Used in production decisions (round-7 — no longer test-only)★: **blocking duplicate drain entry
    ///   for the same recipient** (service.rs `drain_queue` step 0 — keeps batch B from overtaking batch A's remainder). Since everything became queued, this is the device that makes
    ///   "delivery order = load order" hold, and **in synchronous drain two sending threads
    ///   draining the same recipient at once is the routine case, so it matters even more** (ADR-0125 §impact — do not remove).
    ///   Tests also use it for settlement-leak assertions.
    pub fn in_flight_len(&self, recipient: PeerId) -> usize {
        self.in_flight
            .get(&recipient)
            .map(|t| t.message + t.notice)
            .unwrap_or(0)
    }

    /// Collects TTL-exceeded items **across all recipients** and returns them (the upper layer records each item in the ledger as `expired`).
    ///
    /// ★Return = the collected expired items + **the recipient queue key the item was in** (`ExpiredParked`)★: the upper layer walks this list,
    ///   points at the ledger by `(msg_id, recipient)` and records the expired transition (spec §5 "TTL exceeded is expired, stays
    ///   in the ledger"). Emptied queues are removed from the map.
    /// ★Order★: the walk order across queues is non-deterministic because it is a HashMap, but **within one recipient the oldest-first order** is preserved
    ///   (the upper-layer transitions are independent per item so they do not depend on the order across queues).
    /// ★It is a full scan — no early exit at the front (round-4 finding 1 · load-bearing)★: the old implementation broke at the first unexpired item on the grounds "it is FIFO, so
    ///   if the front is unexpired the rest is unexpired too". That premise assumes **the queue ordering axis (admission sequence) and
    ///   the expiry axis (`parked_at`) are the same**, but they are not (why the axes diverge is in the `admission_seq`
    ///   comment). In that case, if a more recent item stands in front, **an expired item behind it stays hidden from the sweep
    ///   forever** (TTL is neutralized and it is not in the ledger either = another face of silent loss). So it was changed to a full
    ///   scan — the cost is linear in the queue length and the scale is tiny (cap 100 per recipient, few queues), so it is meaningless.
    ///   `drain` splits the whole queue for the same reason (that side always did).
    pub fn sweep_expired(&mut self, now: Instant) -> Vec<ExpiredParked> {
        let mut expired = Vec::new();
        self.queues.retain(|recipient, queue| {
            let scanned = std::mem::take(queue);
            for m in scanned {
                if m.is_expired(now) {
                    expired.push(ExpiredParked {
                        recipient: *recipient,
                        msg: m,
                    });
                } else {
                    queue.push_back(m);
                }
            }
            !queue.is_empty()
        });
        expired
    }

    /// ★Empties the recipient queue and returns the items — **only for profile deletion cleanup** (spec §5 · ADR-0116 decision 3)★.
    ///
    /// ★How it differs from `drain` (do not merge the two)★: `drain` "takes out the share to inject now", so it splits expired/unexpired
    ///   and leaves room for items to come back. This one is a notice that **that agent (profile) has vanished from the world**, so there is no branching —
    ///   everything is handed over as a terminal-processing target (the upper layer leaves `failed` + `RECIPIENT_DELETED` in the ledger).
    /// ★Loss is prevented by the return value (this module's existing discipline)★: the store is pure and does not know the ledger. Emptying silently
    ///   would leave a ghost pending, "`pending` in the ledger but not in the queue", so the items are returned whole ("no silently discarding
    ///   live mail" — spec §5 invariant).
    /// ★Notices are cleared together too★: notices addressed to that agent also have no one left to receive them — lanes are not distinguished (the ledger terminal is
    ///   left by the upper layer with the same code).
    /// ★In-flight is not touched (load-bearing)★: the share a flush has carried out of the lock right now **belongs to the receipt
    ///   holder** (`FlightTicket` contract). Touching accounting here would put it out of step twice with that side's settlement. The
    ///   outcome of that share is decided by its original path (delivery success or restore), and the restored remainder is **TTL's concern** — the same nature as the accepted
    ///   residual in spec §5 that deletion cleanup is a one shot at deletion time.
    pub fn purge_recipient(&mut self, recipient: PeerId) -> Vec<ParkedMessage> {
        self.queues
            .remove(&recipient)
            .map(|q| q.into_iter().collect())
            .unwrap_or_default()
    }

    /// ★Different from capacity accounting★: the cap is counted per lane and also looks at in-flight (`park`/`can_admit`).
    pub fn len(&self, recipient: PeerId) -> usize {
        self.queues.get(&recipient).map(|q| q.len()).unwrap_or(0)
    }

    /// For order assertions — length alone cannot catch an age-order inversion of re-park (round-4 finding 1).
    pub fn msg_ids(&self, recipient: PeerId) -> Vec<String> {
        self.queues
            .get(&recipient)
            .map(|q| q.iter().map(|m| m.msg_id.clone()).collect())
            .unwrap_or_default()
    }

    pub fn is_empty(&self) -> bool {
        self.queues.is_empty()
    }

    /// Sum of items in all recipient queues (in-flight excluded) — for the assertion 「어느 큐에도 안 들어갔다」 (it entered no queue).
    #[cfg(any(test, feature = "test-harness"))]
    pub fn total_len(&self) -> usize {
        self.queues.values().map(|q| q.len()).sum()
    }

    /// Test convenience — a normal park with no reclaim (most tests run below the lane limit).
    #[cfg(test)]
    fn park_ok(&mut self, recipient: PeerId, msg: ParkedMessage) -> Result<(), ParkError> {
        self.park(recipient, msg).map(|admitted| {
            assert!(
                admitted.retired.is_empty(),
                "park_ok 경로에서 예기치 않은 회수"
            );
        })
    }

    /// ★Test-only corruption injection (C3 review fix 4)★ — a seam to assert, through the real flush path, that in a situation where the parked payload is
    ///   broken (format drift · memory corruption) **only that one item degrades and the batch
    ///   keeps going out**. Not called from production code.
    #[cfg(any(test, feature = "test-harness"))]
    pub fn corrupt_envelope_for_test(&mut self, recipient: PeerId, idx: usize, envelope: String) {
        if let Some(q) = self.queues.get_mut(&recipient) {
            if let Some(m) = q.get_mut(idx) {
                m.envelope = envelope;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tid;

    /// `admission_seq` is overwritten by `park`, so it is 0 here (placeholder).
    fn parked(id: &str, kind: ParkKind, at: Instant) -> ParkedMessage {
        ParkedMessage {
            msg_id: id.to_string(),
            envelope: format!("<message>{id}</message>"),
            kind,
            parked_at: at,
            admission_seq: 0,
        }
    }

    #[test]
    fn a_queue_full_of_deliverable_items_rejects_instead_of_cannibalizing_live_mail() {
        let mut mb = Mailbox::new();
        let t0 = Instant::now();
        for i in 0..MAILBOX_CAP {
            mb.park(tid("w"), parked(&format!("m{i}"), ParkKind::Message, t0))
                .unwrap_or_else(|_| panic!("{i}번째"));
        }
        assert_eq!(
            mb.park(tid("w"), parked("m-new", ParkKind::Message, t0))
                .err(),
            Some(ParkError::MailboxFull),
            "회수할 stale 이 없으면 반려"
        );
        assert_eq!(
            mb.len(tid("w")),
            MAILBOX_CAP,
            "반려 경로는 큐를 건드리지 않는다(부작용 0)"
        );
        assert_eq!(mb.msg_ids(tid("w"))[0], "m0", "아무것도 걷어내지 않았다");
    }

    // ── round-6: notice lane (independent limit + no bounce) ─────────────────────────────────────────
    #[test]
    fn the_notice_lane_retires_its_oldest_instead_of_rejecting() {
        let mut mb = Mailbox::new();
        let t0 = Instant::now();
        for i in 0..NOTICE_CAP {
            mb.park_ok(tid("w"), parked(&format!("n{i}"), ParkKind::Notice, t0))
                .unwrap_or_else(|_| panic!("{i}번째 notice(레인 이내)"));
        }
        assert_eq!(mb.len(tid("w")), NOTICE_CAP);

        let admitted = mb
            .park(tid("w"), parked("n-new", ParkKind::Notice, t0))
            .expect("notice 는 절대 반려되지 않는다");
        assert_eq!(
            admitted
                .retired
                .iter()
                .map(|m| m.msg_id.as_str())
                .collect::<Vec<_>>(),
            vec!["n0"],
            "가장 오래된 통지 1건이 회수돼 장부에 남는다(조용한 유실 금지)"
        );
        let ids = mb.msg_ids(tid("w"));
        assert_eq!(ids.len(), NOTICE_CAP, "notice 레인은 상한을 넘지 않는다");
        assert_eq!(ids.first().unwrap(), "n1");
        assert_eq!(ids.last().unwrap(), "n-new");
    }

    #[test]
    fn the_two_lanes_never_block_each_other() {
        let mut mb = Mailbox::new();
        let t0 = Instant::now();
        for i in 0..MAILBOX_CAP {
            mb.park(tid("w"), parked(&format!("m{i}"), ParkKind::Message, t0))
                .unwrap();
        }
        assert_eq!(
            mb.park(tid("w"), parked("m-over", ParkKind::Message, t0))
                .err(),
            Some(ParkError::MailboxFull)
        );
        for i in 0..NOTICE_CAP {
            let admitted = mb
                .park(tid("w"), parked(&format!("n{i}"), ParkKind::Notice, t0))
                .unwrap_or_else(|_| panic!("가득 찬 message 큐가 notice 를 막으면 안 된다({i})"));
            assert!(
                admitted.retired.is_empty(),
                "notice 레인은 아직 여유가 있다"
            );
        }
        assert_eq!(mb.len(tid("w")), MAILBOX_CAP + NOTICE_CAP);
        assert_eq!(
            mb.park(tid("w"), parked("m-over2", ParkKind::Message, t0))
                .err(),
            Some(ParkError::MailboxFull),
            "notice 가 message 분모를 부풀리지 않는다(반려 사유는 message 백로그뿐)"
        );
        let admitted = mb
            .park(tid("w"), parked("n-new", ParkKind::Notice, t0))
            .expect("notice 반려 없음");
        assert_eq!(admitted.retired.len(), 1);
        assert_eq!(
            mb.len(tid("w")),
            MAILBOX_CAP + NOTICE_CAP,
            "두 레인 합이 큐 총량의 상한이다(물리 상한 상수 불요)"
        );
    }

    // ── F1: denominator = queue + in-flight (batch outside the lock) ─────────────────────────────────────────────────
    #[test]
    fn an_in_flight_batch_still_counts_against_the_cap() {
        let mut mb = Mailbox::new();
        let t0 = Instant::now();
        for i in 0..MAILBOX_CAP {
            mb.park_ok(tid("w"), parked(&format!("m{i}"), ParkKind::Message, t0))
                .unwrap_or_else(|_| panic!("{i}번째"));
        }
        let batch = mb.drain(tid("w"), t0).deliverable;
        assert_eq!(mb.len(tid("w")), 0, "전제: drain 이 큐를 통째로 비웠다");
        let ticket = mb.take_in_flight(tid("w"), batch.iter());
        assert_eq!(mb.in_flight_len(tid("w")), MAILBOX_CAP);

        assert_eq!(
            mb.park(tid("w"), parked("c0", ParkKind::Message, t0)).err(),
            Some(ParkError::MailboxFull),
            "큐가 비어 보여도 분모는 in-flight 를 센다(옛 구현 = 100건 더 수용)"
        );
        mb.settle_in_flight(tid("w"), ticket);
        assert_eq!(mb.in_flight_len(tid("w")), 0);
        mb.park(tid("w"), parked("c1", ParkKind::Message, t0))
            .expect("정산 뒤에는 수용");
        assert_eq!(mb.len(tid("w")), 1);
    }

    // ── round-7 → ADR-0125: observation means of an in-progress batch (store layer of the duplicate drain entry guard) ──────────
    #[test]
    fn an_out_of_lock_batch_stays_observable_until_it_settles() {
        let mut mb = Mailbox::new();
        let t0 = Instant::now();
        mb.park_ok(tid("w"), parked("m0", ParkKind::Message, t0))
            .expect("park");

        let batch = mb.drain(tid("w"), t0).deliverable;
        assert_eq!(mb.len(tid("w")), 0, "큐 항은 0(= 큐만 보는 판정의 전부)");
        let ticket = mb.take_in_flight(tid("w"), batch.iter());
        assert_eq!(
            mb.in_flight_len(tid("w")),
            1,
            "큐를 떠났어도 아직 수신자에게 닿지 않았다 — 진행 중인 배치가 보여야 한다"
        );

        mb.settle_in_flight(tid("w"), ticket);
        assert_eq!(
            mb.in_flight_len(tid("w")),
            0,
            "정산되면(배달 완료) 비로소 그 이름 앞이 빈다"
        );
    }

    #[test]
    fn repeated_drain_restore_cycles_never_grow_past_the_cap() {
        let mut mb = Mailbox::new();
        let t0 = Instant::now();
        for i in 0..MAILBOX_CAP {
            mb.park_ok(tid("w"), parked(&format!("m{i}"), ParkKind::Message, t0))
                .unwrap_or_else(|_| panic!("{i}번째"));
        }
        for cycle in 0..4 {
            let batch = mb.drain(tid("w"), t0).deliverable;
            let ticket = mb.take_in_flight(tid("w"), batch.iter());
            let admitted = (0..20)
                .filter(|i| {
                    mb.park(
                        tid("w"),
                        parked(&format!("c{cycle}-{i}"), ParkKind::Message, t0),
                    )
                    .is_ok()
                })
                .count();
            assert_eq!(
                admitted, 0,
                "사이클 {cycle}: 이미 cap 만큼 나가 있으므로 창이 열려 있어도 한 건도 못 받는다"
            );
            mb.restore_ordered(tid("w"), batch);
            mb.settle_in_flight(tid("w"), ticket);
            assert_eq!(
                mb.len(tid("w")),
                MAILBOX_CAP,
                "사이클 {cycle}: 큐 + in-flight 합이 cap 을 넘지 않는다"
            );
        }
        assert_eq!(mb.in_flight_len(tid("w")), 0, "정산 누수 없음");
    }

    #[test]
    fn a_partially_settled_ticket_frees_only_what_was_resolved() {
        let mut mb = Mailbox::new();
        let t0 = Instant::now();
        for i in 0..3 {
            mb.park_ok(tid("w"), parked(&format!("m{i}"), ParkKind::Message, t0))
                .unwrap();
        }
        let batch = mb.drain(tid("w"), t0).deliverable;
        let mut ticket = mb.take_in_flight(tid("w"), batch.iter());
        assert_eq!(mb.in_flight_len(tid("w")), 3);

        let one = ticket.split([ParkKind::Message]);
        mb.settle_in_flight(tid("w"), one);
        assert_eq!(
            mb.in_flight_len(tid("w")),
            2,
            "배달된 1건만 분모에서 빠진다"
        );

        let rest = ticket.split([ParkKind::Message; 5]);
        mb.settle_in_flight(tid("w"), rest);
        assert_eq!(mb.in_flight_len(tid("w")), 0);
        assert!(ticket.is_zero(), "영수증은 잔량 이상을 발급하지 않는다");
        mb.settle_in_flight(tid("w"), mb_ticket_of(2));
        assert_eq!(mb.in_flight_len(tid("w")), 0);
    }

    fn mb_ticket_of(n: usize) -> FlightTicket {
        let mut t = FlightTicket::default();
        for _ in 0..n {
            t.add(ParkKind::Message);
        }
        t
    }

    #[test]
    fn the_notice_lane_overshoot_while_a_batch_is_in_flight_is_exactly_one() {
        let mut mb = Mailbox::new();
        let t0 = Instant::now();
        for i in 0..NOTICE_CAP {
            mb.park_ok(tid("w"), parked(&format!("n{i}"), ParkKind::Notice, t0))
                .unwrap_or_else(|_| panic!("{i}번째"));
        }
        let batch = mb.drain(tid("w"), t0).deliverable;
        let ticket = mb.take_in_flight(tid("w"), batch.iter());
        let mut retired_total = 0usize;
        for i in 0..5 {
            let admitted = mb
                .park(tid("w"), parked(&format!("x{i}"), ParkKind::Notice, t0))
                .expect("통지는 절대 반려되지 않는다");
            retired_total += admitted.retired.len();
        }
        assert_eq!(
            mb.len(tid("w")) + mb.in_flight_len(tid("w")),
            NOTICE_CAP + 1,
            "일시 초과는 +1 에서 멈춘다(다음 park 이 초과분 전체를 걷어낸다)"
        );
        assert_eq!(
            retired_total, 4,
            "밀려난 통지는 전부 반환돼 장부에 남는다(조용한 유실 0)"
        );
        mb.settle_in_flight(tid("w"), ticket);
        assert_eq!(mb.in_flight_len(tid("w")), 0);
    }

    #[test]
    fn park_and_drain_preserves_fifo_order() {
        let mut mb = Mailbox::new();
        let t0 = Instant::now();
        for i in 0..5 {
            mb.park_ok(
                tid("alice"),
                parked(&format!("m{i}"), ParkKind::Message, t0),
            )
            .expect("park 성공");
        }
        let drained = mb.drain(tid("alice"), t0);
        let ids: Vec<&str> = drained
            .deliverable
            .iter()
            .map(|m| m.msg_id.as_str())
            .collect();
        assert_eq!(
            ids,
            vec!["m0", "m1", "m2", "m3", "m4"],
            "drain 은 park 순서(오래된 순)를 보존해야"
        );
        assert!(drained.expired.is_empty(), "만료 없음");
    }

    #[test]
    fn drain_empties_the_queue() {
        let mut mb = Mailbox::new();
        let t0 = Instant::now();
        mb.park_ok(tid("alice"), parked("m0", ParkKind::Message, t0))
            .unwrap();
        assert_eq!(mb.len(tid("alice")), 1);
        let _ = mb.drain(tid("alice"), t0);
        assert_eq!(mb.len(tid("alice")), 0, "drain 후 큐가 비어야");
        assert!(mb.is_empty(), "빈 큐는 맵에서 제거돼야");
    }

    #[test]
    fn drain_absent_recipient_is_empty() {
        let mut mb = Mailbox::new();
        let drained = mb.drain(tid("nobody"), Instant::now());
        assert!(
            drained.deliverable.is_empty(),
            "없는 수신자 drain 은 빈 목록"
        );
        assert!(drained.expired.is_empty());
    }

    #[test]
    fn cap_rejects_message_beyond_100() {
        let mut mb = Mailbox::new();
        let t0 = Instant::now();
        for i in 0..MAILBOX_CAP {
            mb.park_ok(tid("bob"), parked(&format!("m{i}"), ParkKind::Message, t0))
                .unwrap_or_else(|_| panic!("{i}번째(cap 이내)는 수용해야"));
        }
        assert_eq!(mb.len(tid("bob")), MAILBOX_CAP);
        let over = mb.park_ok(tid("bob"), parked("overflow", ParkKind::Message, t0));
        assert_eq!(
            over,
            Err(ParkError::MailboxFull),
            "cap 초과 message 는 MailboxFull 반려"
        );
        assert_eq!(
            mb.len(tid("bob")),
            MAILBOX_CAP,
            "반려된 항목은 큐에 안 들어감"
        );
    }

    #[test]
    fn a_full_message_queue_still_accepts_notices() {
        let mut mb = Mailbox::new();
        let t0 = Instant::now();
        for i in 0..MAILBOX_CAP {
            mb.park_ok(
                tid("carol"),
                parked(&format!("m{i}"), ParkKind::Message, t0),
            )
            .unwrap();
        }
        assert_eq!(
            mb.park_ok(tid("carol"), parked("msg-over", ParkKind::Message, t0)),
            Err(ParkError::MailboxFull)
        );
        for i in 0..3 {
            mb.park_ok(tid("carol"), parked(&format!("n{i}"), ParkKind::Notice, t0))
                .unwrap_or_else(|_| panic!("가득 찬 message 큐가 notice 를 막으면 안 됨({i})"));
        }
        assert_eq!(mb.len(tid("carol")), MAILBOX_CAP + 3);
        assert_eq!(
            mb.park_ok(tid("carol"), parked("msg-over2", ParkKind::Message, t0)),
            Err(ParkError::MailboxFull),
            "notice 가 message 분모를 부풀리면 안 됨"
        );
    }

    #[test]
    fn ttl_boundary_exactly_at_ttl_is_expired() {
        let mut mb = Mailbox::new();
        let t0 = Instant::now();
        mb.park_ok(tid("d"), parked("m0", ParkKind::Message, t0))
            .unwrap();
        let at_ttl = t0 + PARK_TTL;
        let drained = mb.drain(tid("d"), at_ttl);
        assert!(
            drained.deliverable.is_empty(),
            "정확히 TTL 인 순간은 만료(경계 포함) — deliverable 아님"
        );
        assert_eq!(
            drained.expired.len(),
            1,
            "정확히 TTL 인 순간은 expired 로 표면화"
        );
        assert_eq!(drained.expired[0].msg_id, "m0");
    }

    #[test]
    fn ttl_boundary_just_over_ttl_is_expired() {
        let mut mb = Mailbox::new();
        let t0 = Instant::now();
        mb.park_ok(tid("d"), parked("m0", ParkKind::Message, t0))
            .unwrap();
        let over = t0 + PARK_TTL + Duration::from_nanos(1);
        let drained = mb.drain(tid("d"), over);
        assert!(
            drained.deliverable.is_empty(),
            "TTL 초과분은 주입 대상 아님"
        );
        assert_eq!(drained.expired.len(), 1, "TTL 초과분은 expired 로 표면화");
    }

    #[test]
    fn drain_surfaces_expired_never_silently_dropped() {
        let mut mb = Mailbox::new();
        let t0 = Instant::now();
        // Make old expire (t0) and recent stay alive (t0 + 30m). now = t0 + TTL.
        mb.park_ok(tid("h"), parked("old", ParkKind::Message, t0))
            .unwrap();
        mb.park_ok(
            tid("h"),
            parked("recent", ParkKind::Message, t0 + Duration::from_secs(1800)),
        )
        .unwrap();
        let now = t0 + PARK_TTL;
        let drained = mb.drain(tid("h"), now);
        let deliverable: Vec<&str> = drained
            .deliverable
            .iter()
            .map(|m| m.msg_id.as_str())
            .collect();
        let expired: Vec<&str> = drained.expired.iter().map(|m| m.msg_id.as_str()).collect();
        assert_eq!(deliverable, vec!["recent"], "미만료만 주입 대상");
        assert_eq!(
            expired,
            vec!["old"],
            "만료는 조용히 사라지지 않고 expired 로"
        );
        assert!(mb.is_empty(), "drain 은 큐를 비운다(재-park 방지)");
    }

    #[test]
    fn sweep_expired_removes_and_returns_expired_only() {
        let mut mb = Mailbox::new();
        let t0 = Instant::now();
        // Old (t0) + recent (t0 + 30m). With now = t0 + TTL + 1ns only the old one expires.
        mb.park_ok(tid("e"), parked("old", ParkKind::Message, t0))
            .unwrap();
        mb.park_ok(
            tid("e"),
            parked("recent", ParkKind::Message, t0 + Duration::from_secs(1800)),
        )
        .unwrap();
        let now = t0 + PARK_TTL + Duration::from_nanos(1);
        let expired = mb.sweep_expired(now);
        assert_eq!(expired.len(), 1, "만료분 1건만 반환");
        assert_eq!(expired[0].msg.msg_id, "old");
        assert_eq!(
            expired[0].recipient,
            tid("e"),
            "만료 항목이 있던 수신자 큐 키를 함께 돌려줘야(1:N 장부 전이 — C4)"
        );
        assert_eq!(mb.len(tid("e")), 1, "최근 것은 큐에 잔존");
        let rest = mb.drain(tid("e"), now);
        assert_eq!(rest.deliverable[0].msg_id, "recent");
        assert!(rest.expired.is_empty());
    }

    #[test]
    fn sweep_expired_removes_empty_queues() {
        let mut mb = Mailbox::new();
        let t0 = Instant::now();
        mb.park_ok(tid("f"), parked("only", ParkKind::Message, t0))
            .unwrap();
        let now = t0 + PARK_TTL + Duration::from_nanos(1);
        let expired = mb.sweep_expired(now);
        assert_eq!(expired.len(), 1);
        assert!(mb.is_empty(), "전부 만료돼 비면 큐가 맵에서 제거돼야");
    }

    #[test]
    fn sweep_expired_labels_each_item_with_its_own_recipient() {
        let mut mb = Mailbox::new();
        let t0 = Instant::now();
        mb.park_ok(tid("alice"), parked("g1", ParkKind::Message, t0))
            .unwrap();
        mb.park_ok(tid("bob"), parked("g1", ParkKind::Message, t0))
            .unwrap();
        let now = t0 + PARK_TTL;
        let expired = mb.sweep_expired(now);
        assert_eq!(expired.len(), 2, "두 멤버 큐의 같은 msg_id 가 각각 만료");
        // The order across queues is non-deterministic because of HashMap — view it as a set.
        let mut got: Vec<PeerId> = expired.iter().map(|e| e.recipient).collect();
        let mut want = vec![tid("alice"), tid("bob")];
        got.sort();
        want.sort();
        assert_eq!(got, want);
        assert!(
            expired.iter().all(|e| e.msg.msg_id == "g1"),
            "둘 다 같은 논리 메시지(1 msg_id : N 배달기록)"
        );
    }

    // ── restore_ordered (re-park lossless restore — finding 1 · round-4 finding 1) ─────────────
    #[test]
    fn restore_ordered_bypasses_cap_no_loss() {
        let mut mb = Mailbox::new();
        let t0 = Instant::now();
        for i in 0..MAILBOX_CAP {
            mb.park_ok(tid("r"), parked(&format!("new{i}"), ParkKind::Message, t0))
                .unwrap();
        }
        assert_eq!(mb.len(tid("r")), MAILBOX_CAP);
        assert_eq!(
            mb.park_ok(tid("r"), parked("would-reject", ParkKind::Message, t0)),
            Err(ParkError::MailboxFull)
        );
        let older = vec![
            parked("old0", ParkKind::Message, t0),
            parked("old1", ParkKind::Message, t0),
        ];
        mb.restore_ordered(tid("r"), older);
        assert_eq!(
            mb.len(tid("r")),
            MAILBOX_CAP + 2,
            "재파킹은 cap 을 우회해 admitted 항목을 되돌린다(유실 0)"
        );
    }

    #[test]
    fn restore_ordered_preserves_oldest_first_before_concurrent_parks() {
        // Scenario: [old0,old1,old2] were taken out by drain and new0·new1 were parked in the meantime → delivered old0, then
        //   failed → re-park [old1,old2]. The queue is then [new0,new1], so after restore_ordered = [old1,old2,new0,new1].
        let mut mb = Mailbox::new();
        let t0 = Instant::now();
        mb.park_ok(
            tid("r"),
            parked("new0", ParkKind::Message, t0 + Duration::from_secs(10)),
        )
        .unwrap();
        mb.park_ok(
            tid("r"),
            parked("new1", ParkKind::Message, t0 + Duration::from_secs(11)),
        )
        .unwrap();
        let older = vec![
            parked("old1", ParkKind::Message, t0 + Duration::from_secs(1)),
            parked("old2", ParkKind::Message, t0 + Duration::from_secs(2)),
        ];
        mb.restore_ordered(tid("r"), older);
        let drained = mb.drain(tid("r"), t0 + Duration::from_secs(20));
        let ids: Vec<&str> = drained
            .deliverable
            .iter()
            .map(|m| m.msg_id.as_str())
            .collect();
        assert_eq!(
            ids,
            vec!["old1", "old2", "new0", "new1"],
            "재파킹분이 동시 park 신규분보다 앞서야(오래된 순 보존)"
        );
    }

    #[test]
    fn restore_ordered_merges_two_batches_into_global_age_order() {
        let mut mb = Mailbox::new();
        let t0 = Instant::now();
        for i in 0..4 {
            mb.park_ok(tid("dup"), parked(&format!("m{i}"), ParkKind::Message, t0))
                .unwrap();
        }
        let drained = mb.drain(tid("dup"), t0).deliverable;
        let group_a: Vec<ParkedMessage> = drained.iter().step_by(2).cloned().collect();
        let group_b: Vec<ParkedMessage> = drained.iter().skip(1).step_by(2).cloned().collect();
        assert_eq!(
            group_a
                .iter()
                .map(|m| m.msg_id.as_str())
                .collect::<Vec<_>>(),
            vec!["m0", "m2"]
        );
        mb.restore_ordered(tid("dup"), group_a);
        mb.restore_ordered(tid("dup"), group_b);
        assert_eq!(
            mb.msg_ids(tid("dup")),
            vec!["m0", "m1", "m2", "m3"],
            "여러 번 복원해도 큐는 전역 수용 순서(오래된 순)를 유지해야"
        );
        mb.park_ok(tid("dup"), parked("new", ParkKind::Message, t0))
            .unwrap();
        assert_eq!(
            mb.msg_ids(tid("dup")),
            vec!["m0", "m1", "m2", "m3", "new"],
            "재파킹분이 신규분보다 앞"
        );
    }

    #[test]
    fn restore_ordered_interleaves_with_concurrently_parked_newer_items() {
        let mut mb = Mailbox::new();
        let t0 = Instant::now();
        for i in 0..3 {
            mb.park_ok(tid("r"), parked(&format!("old{i}"), ParkKind::Message, t0))
                .unwrap();
        }
        let drained = mb.drain(tid("r"), t0).deliverable; // old0..old2 (sequence 0..2)
        mb.park_ok(tid("r"), parked("new0", ParkKind::Message, t0))
            .unwrap(); // sequence 3
        mb.restore_ordered(tid("r"), drained);
        assert_eq!(
            mb.msg_ids(tid("r")),
            vec!["old0", "old1", "old2", "new0"],
            "동시 park 된 신규분은 재파킹분 뒤"
        );
    }

    #[test]
    fn sweep_surfaces_expired_hidden_behind_newer_front() {
        let mut mb = Mailbox::new();
        let t0 = Instant::now();
        // Admission order is newer → older (inverted) — the front is unexpired and what follows is expired.
        mb.park_ok(
            tid("z"),
            parked("newer", ParkKind::Message, t0 + Duration::from_secs(3600)),
        )
        .unwrap();
        mb.park_ok(tid("z"), parked("older", ParkKind::Message, t0))
            .unwrap();
        let now = t0 + PARK_TTL;
        let expired = mb.sweep_expired(now);
        assert_eq!(
            expired
                .iter()
                .map(|e| e.msg.msg_id.as_str())
                .collect::<Vec<_>>(),
            vec!["older"],
            "미만료 항목 뒤에 숨은 만료분도 sweep 이 걷어내야(전량 스캔)"
        );
        assert_eq!(
            mb.msg_ids(tid("z")),
            vec!["newer"],
            "미만료분은 순서 그대로 잔존"
        );
    }

    #[test]
    fn restore_ordered_empty_is_noop() {
        let mut mb = Mailbox::new();
        mb.restore_ordered(tid("r"), Vec::new());
        assert!(mb.is_empty(), "빈 재파킹은 큐를 만들지 않는다");
    }

    #[test]
    fn sweep_preserves_oldest_first_within_recipient() {
        let mut mb = Mailbox::new();
        let t0 = Instant::now();
        // Make all three expire (park in oldest-first order).
        mb.park_ok(tid("g"), parked("first", ParkKind::Message, t0))
            .unwrap();
        mb.park_ok(
            tid("g"),
            parked("second", ParkKind::Message, t0 + Duration::from_secs(1)),
        )
        .unwrap();
        mb.park_ok(
            tid("g"),
            parked("third", ParkKind::Message, t0 + Duration::from_secs(2)),
        )
        .unwrap();
        let now = t0 + PARK_TTL + Duration::from_secs(10);
        let expired = mb.sweep_expired(now);
        let ids: Vec<&str> = expired.iter().map(|e| e.msg.msg_id.as_str()).collect();
        assert_eq!(
            ids,
            vec!["first", "second", "third"],
            "sweep 도 오래된 순 보존"
        );
    }
}
