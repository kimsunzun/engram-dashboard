//! mailbox — in-memory parking store (spec §5, ADR-0103 decision 5, ADR-0104 idle gate, ADR-0111/0114).
//!
//! ★Role★: holds **messages not yet injected** in per-recipient FIFO queues. Queue key = recipient **agent id** (ADR-0309 — a name becomes
//!   an id at the entrance).
//! Three parking reasons: ① busy kinds — recipient mid-turn, or joining FIFO behind earlier parked items (batch flush on entering idle) ②
//!   injection (write) failure — waits for the next flush
//! trigger (idle, reappearance) ③ **sleeping** — process closed but the agent has a real profile (trigger = reappearance via the roster
//!   diff). The upper service empties the queue with `drain` and injects
//! one batch, oldest first.
//! ★Kinds that do not come in (ADR-0116 decision 1)★: a name existing nowhere → `RECIPIENT_NOT_FOUND` row; several agents with one name at
//!   any layer → `RECIPIENT_AMBIGUOUS` row. Turning these back into
//! parking violates ADR-0116. ★A live session without turn signals is a delivery kind, not parking (decision 7)★: injected at once without
//!   a gate, so it enters this queue only via ②.
//! (The amendment-4 draft's `RECIPIENT_UNREACHABLE` bounce is **discarded** — do not revive it.)
//!
//! ★Invariants★:
//!   - **FIFO (oldest first)** — `drain`/`sweep_expired` preserve park order (ADR-0104).
//!   - **Ordering axis = admission sequence number (`ParkedMessage.admission_seq`)**, strictly increasing front to back (`park` appends a
//!   new number, `restore_ordered` merges by it); why not the clock:
//!     the `admission_seq` comment (round-4 finding 1).
//!   - **Capacity = 2 independent lanes by kind**: ① `MAILBOX_CAP` (100 messages per recipient) ② `NOTICE_CAP` (64 notices) — a message
//!   backlog does not block notices and vice versa.
//!     ★Pressure reclaim of the message lane is abolished (ADR-0114 decision 1, amending ADR-0107 decision 2)★: its basis ("undeliverable
//!   parked items = empty set") was **weakened** by ADR-0116 — parked items
//!     for a sleeping name that is not restored are restorable but not restored, and deletion cleanup is a one shot, so some stay until
//!   TTL. The bound's basis is now TTL 24h + lane cap; reopening reclaim
//!     needs a new decision. The only message-lane cleanup is the **TTL sweep** (older than 24h, 60 s period).
//!   - **Denominator = queue + in-flight (F1)** — a batch flush carried out of the lock also counts in that recipient lane's denominator
//!   (`take_in_flight` → `settle_in_flight`; proof in the `MAILBOX_CAP` comment).
//!   - **TTL = 24h** — `sweep_expired` collects over-limit items and the upper layer records them `expired` (ADR-0105).
//!   - **Pure + injected clock** — expiry is decided only by `park` time vs the argument `now`.
//!   - **Incarnation binding (`bound_incarnation`) was abolished (ADR-0111 decision 6)** — parked items are delivered to a new incarnation
//!   (epoch) of the same agent too (ADR-0309 decision 1), so there is no
//!     binding axis, `visible_to` filter or liveness snapshot argument. If "a group broadcast goes only to the send-time incarnation" is
//!   needed again, add it non-breakingly as a v2 personal-mail option
//!     (spec §8), not a group-only rule.
//!   - **Order = load order (ADR-0125)** — every send is loaded at the tail without exception and drain pulls from the front, so this
//!   store has **no seat reservation and no `has_pending_ahead` join decision**
//!     (needing them again means the send path split in two). The only remaining ordering device is `in_flight_len`, which **keeps drains
//!   from overlapping**.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::PeerId;

/// ★Why 24h (spec §5 policy constant, ADR-0105 — raised from 1h)★: industry practice is days (SQS 4, Kafka 7, Postfix 5). "Live recipients
///   are exempt from TTL" has no precedent in the 6 systems surveyed
/// (/research light, 2026-07-25), so one clock-based rule is kept for sleeping and busy-waiting alike. With the in-memory stage + cap 100
///   a long TTL costs ~0 (all vanishes on restart) —
/// **when the persistence (disk) stage arrives this is a premise to redesign**. The sweep period is the upper service's (only the value is
///   here).
const PARK_TTL: Duration = Duration::from_secs(24 * 60 * 60);

/// ★Message lane limit — 100 per recipient, **counting everything** (round-6)★.
/// ★Why 100 (spec §5)★: defense against a runaway sender inflating one recipient's mailbox; silent drop-head would hide the loss (ADR-0103
///   rejected), so new items bounce at once.
/// ★Why no exceptions (round-6)★: subtracting "items undeliverable to this incarnation" reset the denominator on every PeerId swap (100
///   per incarnation, effectively unbounded within TTL), and a second physical
/// cap plus the cap-bypassing path (`restore_ordered`) and the exempt kind (notice) each became a hole; every exception was removed.
/// ★Denominator = **queue + in-flight** (F1 — round-6's bound claim was wrong)★: `restore_ordered` (cap-bypassing lossless restore) broke
///   "queue ≤ cap" across cycles — ① flush empties the queue under the lock
/// ② during out-of-lock inject the queue is empty, so k concurrent sends pass ③ inject fails and N comes back → N + k; the next batch is N
///   + k, **+k every cycle**. So the denominator adds **in-flight** (the share flush
/// carried out and not yet settled).
/// ★Widened in ADR-0125 §Impact (spec §5): **every send passes this gate**★: formerly a direct send straight to stdin was outside the
///   denominator (101 on an idle recipient was no violation); with the
/// shortcut abolished a live recipient can also get `MAILBOX_FULL` — a design consequence, not a regression (changing the value needs a
///   separate decision). It does not mean "up overall": an idle
/// recipient's drain empties the queue, and mid-turn/sleeping parking piled up before too; newly hit only when several senders pile on
///   before the drain catches up.
/// ★Purpose★: a cap on mail that piles up, not injections per second (that would be a rate limit).
/// ★Invariant (zero slack)★: `queue_lane + in_flight_lane ≤ cap` — `take_in_flight` moves queue→in-flight (sum unchanged),
///   `restore_ordered` + same-section settlement moves back (unchanged), delivery
/// settlement reduces it, `park` bounces if the sum would pass the cap. Only the notice lane can have a temporary +1 overshoot (`NOTICE_CAP`).
const MAILBOX_CAP: usize = 100; // ADR-0107

/// ★Notice lane limit — 64 per recipient (round-6; **raised from 20 by ADR-0114 decision 2**)★.
/// ★Why a separate lane (replaces the old "cap exception")★: a deadline-overrun notice blocked by a full mailbox halves the contract
///   (ADR-0103). The old exemption ("notice does not count") meant unbounded:
/// its stated basis (`ledger::MAX_OPEN_REQUESTS` bounds notices) is **false** — `due_timeouts` sets `notified` at once and frees the slot,
///   so while earlier notices stay parked the next wave can open contracts
/// again. Separate lanes: a message backlog cannot block notices, and notices are not unbounded.
/// ★Why 64 (tunable)★: a notice is a daemon-made infrastructure notice, but a multi-recipient request (ADR-0111 decision 5) opens N
///   contracts and the overrun notice goes out once per contract — 20 was narrow
/// (paired with the decision not to build notice merging). On overflow the **oldest notice** is reclaimed (the newest carries the same
///   kind of fact with a more accurate time); it stays in the ledger as `skipped`
/// (`ParkAdmitted`).
/// ★Only this lane can have a temporary +1 overshoot (F1, explicit contract)★: reclaim targets only notices **in the queue**, but the
///   denominator is queue + in-flight; when all 64 are out in a flush a new notice
/// finds nothing to reclaim and is **accepted anyway** (no bounce branch) → `queue + in_flight = NOTICE_CAP + 1`. The next notice park
///   computes the whole overshoot as `want` and collects it. The message
/// lane bounces, so it never overshoots.
const NOTICE_CAP: usize = 64; // ADR-0107

/// Kind of a parked item — separates **capacity lanes** (round-6); each kind sees only its own lane's limit (module header).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParkKind {
    /// Message from a peer (`<message>`) — the `MAILBOX_CAP` lane.
    Message,
    /// Daemon notice (`<notice>`) — the `NOTICE_CAP` lane.
    Notice,
}

/// ★envelope = the finished envelope string★: this is a pure store and does **not assemble** envelopes — it keeps and returns the finished
///   string made by the upper layer (`envelope.rs` wrap point; single wrap
///   point invariant, ADR-0096) as opaque text.
#[derive(Debug, Clone)]
pub struct ParkedMessage {
    /// Logical message id for ledger correlation (assigned by the upper layer); carried only as a value.
    pub msg_id: String,
    pub envelope: String,
    pub kind: ParkKind,
    /// Park time (the injected now). The TTL decision basis.
    pub parked_at: Instant,
    /// ★Admission sequence number — the queue ordering axis (round-4 finding 1)★: monotonically increasing, assigned by `park` at
    ///   acceptance; **a caller-supplied value is overwritten** (the store is the only
    /// assigner — two assigners diverge). ★Why not `parked_at` (load-bearing)★: expiry is judged by the clock, but "who entered first" by
    ///   the clock fails: ① `park_pending` takes `Instant::now()` **before
    /// acquiring** the lock, so racing sends can invert ② coarse clocks give the **same** `parked_at`. The number is assigned inside the
    ///   store under the lock, so `restore_ordered` merges on it and the queue is strictly increasing in it.
    pub admission_seq: u64,
}

impl ParkedMessage {
    /// ★Boundary (load-bearing)★: `elapsed >= PARK_TTL` — expires from the exact moment TTL has passed (inclusive; "TTL = maximum
    ///   lifetime"). Pinned by `ttl_boundary_*`; changing it is a regression.
    /// ★`pub(crate)` (F3)★: `service::park_into` splits the ledger vocabulary of pressure-reclaimed items (past TTL = `expired`, else
    ///   `skipped`) and must **reuse this decision**; duplicating the 24h literal
    /// could diverge, so the constant and comparison sign are tied here and only the function is opened.
    pub(crate) fn is_expired(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.parked_at) >= PARK_TTL
    }
}

/// Park bounce reason — the upper layer maps it to a wire error code (currently only cap exceeded → `MAILBOX_FULL`, spec §5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParkError {
    MailboxFull,
}

/// ★Why a return value (same pattern as `DrainOutcome`/`ExpiredParked`)★: the store is pure and does not know the ledger; silently
///   dropping collected items would be a hidden loss (`pending` in the ledger, not in
/// the queue = ghost pending). The fact "taken out of the queue" goes to the caller with the item so the upper layer leaves `skipped` in
///   the ledger and logs outside the lock.
/// ★Only the notice lane uses this channel today (ADR-0114 decision 1)★ — the only reclaim reason left is "oldest notice pushed out by a
///   newer one"; the channel stays because the no-silent-loss discipline is unchanged.
/// ★Vocabulary is `skipped` by default, **TTL takes priority** (F3)★: a pushed-out old notice is `skipped` ("not delivered"), not
///   `expired`; but because the sweep period (60 s) is shorter than TTL (24 h) only by a gap,
/// an item already past TTL but uncollected can be reclaimed, and per spec §5 that is `expired`. So **the store does not decide** — it
///   returns items whole and the upper layer splits them with
/// `ParkedMessage::is_expired(now)` (`service::park_into`).
#[derive(Debug, Default)]
pub struct ParkAdmitted {
    /// Items collected by this park to make room (**oldest first**). Usually empty.
    pub retired: Vec<ParkedMessage>,
}

/// ★Why both collections are returned (spec §5 "expired stays in the ledger")★: silently discarding expired items at drain would record
///   them nowhere ("no silent loss"). Drain empties the queue (preventing re-park) and
/// returns expired items too, so the upper layer injects the injectable ones in one batch, oldest first, and leaves the expired ones
///   `expired` in the ledger. `sweep_expired` is a separate periodic path; what had
/// already expired at drain time also enters the ledger through this return (the two complement each other).
#[derive(Debug, Default)]
pub struct DrainOutcome {
    /// Unexpired = targets of the oldest-first batch injection.
    pub deliverable: Vec<ParkedMessage>,
    /// Past TTL = not injected, recorded `expired` in the ledger (oldest first).
    pub expired: Vec<ParkedMessage>,
}

/// ★Why the recipient is returned too (1 msg_id : N delivery records, spec §4, ADR-0104)★: a ledger record's key is `(msg_id, recipient
///   id)`; a group broadcast has N records per msg_id, so msg_id alone cannot
/// identify which record an expired item is ("first pending record" lookup could expire the wrong member — the C1-era helper's defect).
///   Only the store knows which queue the item was in.
#[derive(Debug, Clone)]
pub struct ExpiredParked {
    /// The recipient queue key this item was in (= `to_id` of the ledger record).
    pub recipient: PeerId,
    pub msg: ParkedMessage,
}

/// ★In-flight receipt (F1, load-bearing)★ — the **per-lane count** of the batch a flush carried out of the lock. ★Value-type ticket★: the
///   messaging lock is released between drain and restore (inject runs outside),
/// so the store must hold "how many are out" but cannot know who took how many; the taker gets a receipt (`take_in_flight`) and hands it
///   back (`settle_in_flight`).
/// ★Partial return is possible★: delivered and put-back items are split off as they happen (`split`) so the denominator equals the real
///   outstanding count.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct FlightTicket {
    message: usize,
    notice: usize,
}

impl FlightTicket {
    /// Is anything left to return (most paths 0 right away — saves acquiring the lock).
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

    /// Splits `kinds` worth off this receipt for a partial return. ★Never split off beyond the remainder★ — the only device preventing
    ///   double return (denominator below reality, cap breached).
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
// ADR-0309
#[derive(Debug, Default)]
pub struct Mailbox {
    queues: HashMap<PeerId, std::collections::VecDeque<ParkedMessage>>,
    /// ★Per-recipient in-flight count (F1)★ — left the queue but terminal (delivered/restored) not yet fixed.
    in_flight: HashMap<PeerId, FlightTicket>,
    /// Next admission sequence number (monotonic counter shared by all recipients — assigner of `ParkedMessage.admission_seq`); u64,
    ///   practically never exhausted.
    next_seq: u64,
}

/// ★Notice lane pressure reclaim — collect up to `want` of the oldest notices (round-6; narrowed by ADR-0114)★. Targets **notices only**;
///   non-matching items are never touched (reclaim does not eat live mail).
/// ★"Oldest first" = the queue ordering axis★: admission sequence is strictly increasing (module header), so a front-to-back walk is
///   oldest first; if that invariant breaks the order here breaks too.
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

    /// ★Capacity = independent lanes by kind, no exceptions (round-6, F1, ADR-0114)★:
    ///   - **Denominator (both lanes)** = that lane's items in the queue **+ that recipient lane's in-flight count** (`take_in_flight`).
    ///   - **message**: count everything; at `MAILBOX_CAP` or more an **immediate** `MailboxFull` bounce, **no reclaim attempt** (ADR-0114
    ///   decision 1; bound basis = TTL 24h + cap). The upper layer reports only that
    ///     recipient as a failure row (no whole bounce — spec §5).
    ///   - **notice**: messages are not in the denominator; at `NOTICE_CAP` or more it reclaims the **oldest notice** and accepts the new
    ///   one — **no bounce branch**, since `deliver_notice` discards the park result and a bounce
    ///     would silently lose the notice (ADR-0103). Price: a temporary +1 overshoot when the queue has too few to reclaim
    ///   (`NOTICE_CAP`). Reclaimed items are **returned** via `ParkAdmitted.retired`.
    /// ★A bounce has zero side effects★: it touches neither the queue nor the sequence counter ("on a bounce nothing is stored at all").
    /// ★Notice `want` can exceed 1★: `restore_ordered` bypasses the cap, so the queue can temporarily pass the limit; it computes the
    ///   **whole overshoot** and cleans it at once.
    pub fn park(
        &mut self,
        recipient: PeerId,
        mut msg: ParkedMessage,
    ) -> Result<ParkAdmitted, ParkError> {
        let kind = msg.kind;
        // Disjoint field borrow alongside the mutable borrow of queues.
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
        // The sequence number is assigned **after acceptance is confirmed** (a bounced item burns no number).
        msg.admission_seq = self.next_seq;
        self.next_seq += 1;
        queue.push_back(msg);
        Ok(ParkAdmitted { retired })
    }

    /// ★Can a new park be accepted now (pure query — a prediction possible since ADR-0114)★. ★Premise of freezing the envelope `to`★: a
    ///   multi-recipient send fixes `to` only after every acceptance decision (spec §1),
    /// and parking carries it in the payload, forcing "decide → fix to → park", so the cap result must be known **before** parking. With
    ///   pressure reclaim this was impossible (decided inside park); with it
    /// abolished the decision is pure arithmetic and prediction never diverges (within the same lock section).
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

    /// ★Re-park (lossless restore) primitive — cap bypass + admission-sequence merge (ADR-0103/0104, finding 1)★: puts back **already
    ///   admitted** items that could not be delivered mid-flush-batch, mixed in global
    /// admission order with the rest of the queue (not a simple front insertion).
    /// ★Why not `park` (no silent loss)★: `park` controls **new admission** (counts the cap, bounces); a re-park is a deferred restore of
    ///   an item already `pending` in the ledger, and the cap is inflow control, not storage
    /// control. If a concurrent park refilled the queue to the cap between drain and inject, `park` would bounce `MailboxFull` and
    ///   ignoring it silently loses an admitted message (ghost pending). So re-park **bypasses** the cap;
    /// inflow was already capped on the park side.
    /// ★The bound is in-flight accounting, not "at most one batch" (F1)★: the drained batch is still in the denominator
    ///   (`take_in_flight`), so inflow k in the drain↔restore window is limited by `cap − (queue + in-flight)`,
    /// and restore only moves slots from in-flight to the queue (settled in the same lock section — service.rs `flush_for` failure
    ///   branch), so the sum does not grow and `queue + in_flight ≤ cap` holds. The returning batch
    /// is also already reduced: flush stamps `skipped` on items bound to a confirmed-dead incarnation (higher epoch of the same PeerId)
    ///   before restoring.
    /// ★Why merge, not FRONT insertion (round-4 finding 1)★: new items can enter between restores, and calling re-park several times would
    ///   put a later call ahead of an earlier one and invert the age order
    /// (restore [m0,m2] → restore [m1,m3] = [m1,m3,m0,m2]). Merging by admission sequence keeps global oldest-first for any number of
    ///   calls; re-parked items have smaller numbers, so the single-call empty-queue case equals
    /// the old FRONT insertion. ★Order must be exact★: ① the recipient's delivery order would invert (ADR-0104) ② drain pulls from the
    ///   head (ADR-0125), so an age inversion is a delivery-order inversion.
    /// ★Precondition★: `items` ascending in sequence (as drain emitted, or a subsequence); merge combines two ascending sequences.
    ///   ★`parked_at` and sequence keep original values★ (TTL not extended — misdelivery defense;
    /// order not reassigned). ★Accepts notice and message alike★ — already admitted; cap accounting is for new parks only.
    pub fn restore_ordered(&mut self, recipient: PeerId, items: Vec<ParkedMessage>) {
        if items.is_empty() {
            return;
        }
        let queue = self.queues.entry(recipient).or_default();
        if queue.is_empty() {
            // Common path (restore under the lock right after drain, so the queue is empty) — no merge needed.
            queue.extend(items);
            return;
        }
        // Ties do not occur structurally (sole assigner, strictly increasing); defensively the re-parked (older) item goes first.
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
                merged.push_back(restored.next().expect("right after peek"));
            } else {
                merged.push_back(remaining.next().expect("right after peek"));
            }
        }
        *queue = merged;
    }

    /// Empties the recipient queue and returns **both** injectable and expired items, oldest first (flush primitive); injection is only
    ///   for `deliverable` (ADR-0104; `DrainOutcome`).
    /// ★Queue removal★: the emptied queue is removed from the map (no unbounded empty queues).
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

    /// ★Registers the batch leaving the lock as in-flight (F1, load-bearing)★ — drain reports here only the share of the `drain` result
    ///   that **actually leaves the lock as an injection target**, and must hand the receipt
    /// back with `settle_in_flight`. ★Separate from `drain` (intentional)★: not every drained item leaves the lock — busy and no-path
    ///   shares return in the same lock section (`restore_ordered`), and confirmed-dead/expired items
    /// stamp a ledger terminal on the spot; holding those as in-flight would swell the denominator without grounds and bounce normal
    ///   inflow with `MailboxFull`. The hole exists only while the lock is released, so the unit is that span.
    /// ★Caller contract★: the registered count must be **fully** handed back (delivered, restored or abandoned); a miss swells that lane's
    ///   denominator permanently, so the upper layer uses a single-exit guard
    /// (service.rs `FlightSettle` — covers early break and unwinding).
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

    /// ★Why saturating★: panicking would double-panic-abort during unwinding. Over-return is prevented on the receipt side
    ///   (`FlightTicket::split`); if it ever happens, **converging to 0** (the side that does not block inflow)
    /// is safe — leaving it would mean that recipient never receives mail again.
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

    /// ★Used in production decisions (ADR-0125 §Impact — do not remove)★: **blocks duplicate drain entry for one recipient** (service.rs
    ///   `drain_queue` step 0 — keeps batch B from overtaking batch A's remainder). It is
    /// what makes "delivery order = load order" hold, and in synchronous drain two sending threads draining one recipient is routine.
    ///   Tests also use it for settlement-leak assertions.
    pub fn in_flight_len(&self, recipient: PeerId) -> usize {
        self.in_flight
            .get(&recipient)
            .map(|t| t.message + t.notice)
            .unwrap_or(0)
    }

    /// Collects TTL-exceeded items **across all recipients** (the upper layer records each `expired`). Returns the items + **the recipient
    ///   queue key** (`ExpiredParked`) so the upper layer points at the ledger by
    /// `(msg_id, recipient)` (spec §5). Emptied queues are removed. ★Order★: across queues non-deterministic (HashMap), oldest-first
    ///   within one recipient (transitions are per-item independent).
    /// ★Full scan — no early exit at the front (round-4 finding 1, load-bearing)★: breaking at the first unexpired item assumed the
    ///   ordering axis (admission sequence) equals the expiry axis (`parked_at`), which is false
    /// (`admission_seq` comment); a more recent item in front would hide an expired one behind it from the sweep forever (TTL neutralized,
    ///   not in the ledger — silent loss). Cost is linear in a tiny queue (cap 100).
    /// `drain` splits the whole queue for the same reason.
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

    /// ★Empties the recipient queue and returns the items — **only for profile deletion cleanup** (spec §5, ADR-0116 decision 3)★. ★Not
    ///   `drain` (do not merge)★: `drain` takes "the share to inject now" and splits
    /// expired/unexpired with room for items to come back; this signals that **the agent has vanished**, so no branching — everything is
    ///   handed over as a terminal target (the upper layer leaves `failed` + `RECIPIENT_DELETED`).
    /// ★Loss is prevented by the return value★: silently emptying would leave a ghost pending ("no silently discarding live mail" — spec
    ///   §5). ★Notices are cleared too★ (no one left to receive them; lanes not distinguished).
    /// ★In-flight is not touched (load-bearing)★: the share a flush carried out **belongs to the receipt holder** (`FlightTicket`);
    ///   touching accounting here would double the mismatch with its settlement. That share's outcome
    /// is decided by its original path, and a restored remainder is TTL's concern (same nature as the accepted one-shot residual in spec §5).
    pub fn purge_recipient(&mut self, recipient: PeerId) -> Vec<ParkedMessage> {
        self.queues
            .remove(&recipient)
            .map(|q| q.into_iter().collect())
            .unwrap_or_default()
    }

    /// ★Not capacity accounting★: the cap is per lane and also counts in-flight (`park`/`can_admit`).
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

    /// Sum of items in all recipient queues (in-flight excluded) — for the assertion 「it entered no queue」.
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
                "unexpected reclaim on the park_ok path"
            );
        })
    }

    /// ★Test-only corruption injection (C3 review fix 4)★ — asserts through the real flush path that a broken parked payload (format
    ///   drift, memory corruption) degrades **only that item** and the batch keeps going. Not called from production.
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
                .unwrap_or_else(|_| panic!("#{i}"));
        }
        assert_eq!(
            mb.park(tid("w"), parked("m-new", ParkKind::Message, t0))
                .err(),
            Some(ParkError::MailboxFull),
            "bounced if there is no stale item to reclaim"
        );
        assert_eq!(
            mb.len(tid("w")),
            MAILBOX_CAP,
            "the bounce path does not touch the queue (0 side effects)"
        );
        assert_eq!(mb.msg_ids(tid("w"))[0], "m0", "nothing was taken out");
    }

    // ── round-6: notice lane (independent limit + no bounce) ─────────────────────────────────────────
    #[test]
    fn the_notice_lane_retires_its_oldest_instead_of_rejecting() {
        let mut mb = Mailbox::new();
        let t0 = Instant::now();
        for i in 0..NOTICE_CAP {
            mb.park_ok(tid("w"), parked(&format!("n{i}"), ParkKind::Notice, t0))
                .unwrap_or_else(|_| panic!("notice #{i} (within the lane)"));
        }
        assert_eq!(mb.len(tid("w")), NOTICE_CAP);

        let admitted = mb
            .park(tid("w"), parked("n-new", ParkKind::Notice, t0))
            .expect("a notice is never bounced");
        assert_eq!(
            admitted
                .retired
                .iter()
                .map(|m| m.msg_id.as_str())
                .collect::<Vec<_>>(),
            vec!["n0"],
            "the oldest notice is reclaimed and remains in the ledger (no silent loss)"
        );
        let ids = mb.msg_ids(tid("w"));
        assert_eq!(
            ids.len(),
            NOTICE_CAP,
            "the notice lane does not exceed the cap"
        );
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
                .unwrap_or_else(|_| panic!("a full message queue must not block notices ({i})"));
            assert!(
                admitted.retired.is_empty(),
                "the notice lane still has room"
            );
        }
        assert_eq!(mb.len(tid("w")), MAILBOX_CAP + NOTICE_CAP);
        assert_eq!(
            mb.park(tid("w"), parked("m-over2", ParkKind::Message, t0))
                .err(),
            Some(ParkError::MailboxFull),
            "notices do not inflate the message denominator (the only bounce reason is the message backlog)"
        );
        let admitted = mb
            .park(tid("w"), parked("n-new", ParkKind::Notice, t0))
            .expect("no notice bounce");
        assert_eq!(admitted.retired.len(), 1);
        assert_eq!(
            mb.len(tid("w")),
            MAILBOX_CAP + NOTICE_CAP,
            "the sum of the two lanes is the cap of the total queue size (no physical cap constant needed)"
        );
    }

    // ── F1: denominator = queue + in-flight (batch outside the lock) ─────────────────────────────────────────────────
    #[test]
    fn an_in_flight_batch_still_counts_against_the_cap() {
        let mut mb = Mailbox::new();
        let t0 = Instant::now();
        for i in 0..MAILBOX_CAP {
            mb.park_ok(tid("w"), parked(&format!("m{i}"), ParkKind::Message, t0))
                .unwrap_or_else(|_| panic!("#{i}"));
        }
        let batch = mb.drain(tid("w"), t0).deliverable;
        assert_eq!(
            mb.len(tid("w")),
            0,
            "premise: the drain emptied the whole queue"
        );
        let ticket = mb.take_in_flight(tid("w"), batch.iter());
        assert_eq!(mb.in_flight_len(tid("w")), MAILBOX_CAP);

        assert_eq!(
            mb.park(tid("w"), parked("c0", ParkKind::Message, t0)).err(),
            Some(ParkError::MailboxFull),
            "even if the queue looks empty, the denominator counts in-flight (old implementation = accepted 100 more)"
        );
        mb.settle_in_flight(tid("w"), ticket);
        assert_eq!(mb.in_flight_len(tid("w")), 0);
        mb.park(tid("w"), parked("c1", ParkKind::Message, t0))
            .expect("accepted after settlement");
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
        assert_eq!(
            mb.len(tid("w")),
            0,
            "queue term is 0 (= all a queue-only decision sees)"
        );
        let ticket = mb.take_in_flight(tid("w"), batch.iter());
        assert_eq!(
            mb.in_flight_len(tid("w")),
            1,
            "it left the queue but has not reached the recipient yet — the batch in progress must be visible"
        );

        mb.settle_in_flight(tid("w"), ticket);
        assert_eq!(
            mb.in_flight_len(tid("w")),
            0,
            "only once settled (delivery complete) does nothing remain in flight addressed to that name"
        );
    }

    #[test]
    fn repeated_drain_restore_cycles_never_grow_past_the_cap() {
        let mut mb = Mailbox::new();
        let t0 = Instant::now();
        for i in 0..MAILBOX_CAP {
            mb.park_ok(tid("w"), parked(&format!("m{i}"), ParkKind::Message, t0))
                .unwrap_or_else(|_| panic!("#{i}"));
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
                "cycle {cycle}: cap's worth is already out, so even with the window open it cannot accept a single one"
            );
            mb.restore_ordered(tid("w"), batch);
            mb.settle_in_flight(tid("w"), ticket);
            assert_eq!(
                mb.len(tid("w")),
                MAILBOX_CAP,
                "cycle {cycle}: queue + in-flight does not exceed the cap"
            );
        }
        assert_eq!(mb.in_flight_len(tid("w")), 0, "no settlement leak");
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
            "only the 1 delivered item leaves the denominator"
        );

        let rest = ticket.split([ParkKind::Message; 5]);
        mb.settle_in_flight(tid("w"), rest);
        assert_eq!(mb.in_flight_len(tid("w")), 0);
        assert!(
            ticket.is_zero(),
            "the receipt does not issue more than the remainder"
        );
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
                .unwrap_or_else(|_| panic!("#{i}"));
        }
        let batch = mb.drain(tid("w"), t0).deliverable;
        let ticket = mb.take_in_flight(tid("w"), batch.iter());
        let mut retired_total = 0usize;
        for i in 0..5 {
            let admitted = mb
                .park(tid("w"), parked(&format!("x{i}"), ParkKind::Notice, t0))
                .expect("a notice is never bounced");
            retired_total += admitted.retired.len();
        }
        assert_eq!(
            mb.len(tid("w")) + mb.in_flight_len(tid("w")),
            NOTICE_CAP + 1,
            "a temporary excess stops at +1 (the next park takes out the whole excess)"
        );
        assert_eq!(
            retired_total, 4,
            "pushed-out notices are all returned and remain in the ledger (0 silent loss)"
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
            .expect("park succeeds");
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
            "drain must preserve park order (oldest first)"
        );
        assert!(drained.expired.is_empty(), "no expiry");
    }

    #[test]
    fn drain_empties_the_queue() {
        let mut mb = Mailbox::new();
        let t0 = Instant::now();
        mb.park_ok(tid("alice"), parked("m0", ParkKind::Message, t0))
            .unwrap();
        assert_eq!(mb.len(tid("alice")), 1);
        let _ = mb.drain(tid("alice"), t0);
        assert_eq!(
            mb.len(tid("alice")),
            0,
            "the queue must be empty after drain"
        );
        assert!(mb.is_empty(), "an empty queue must be removed from the map");
    }

    #[test]
    fn drain_absent_recipient_is_empty() {
        let mut mb = Mailbox::new();
        let drained = mb.drain(tid("nobody"), Instant::now());
        assert!(
            drained.deliverable.is_empty(),
            "draining a nonexistent recipient gives an empty list"
        );
        assert!(drained.expired.is_empty());
    }

    #[test]
    fn cap_rejects_message_beyond_100() {
        let mut mb = Mailbox::new();
        let t0 = Instant::now();
        for i in 0..MAILBOX_CAP {
            mb.park_ok(tid("bob"), parked(&format!("m{i}"), ParkKind::Message, t0))
                .unwrap_or_else(|_| panic!("#{i} (within the cap) must be accepted"));
        }
        assert_eq!(mb.len(tid("bob")), MAILBOX_CAP);
        let over = mb.park_ok(tid("bob"), parked("overflow", ParkKind::Message, t0));
        assert_eq!(
            over,
            Err(ParkError::MailboxFull),
            "a message over the cap is bounced with MailboxFull"
        );
        assert_eq!(
            mb.len(tid("bob")),
            MAILBOX_CAP,
            "a bounced item does not enter the queue"
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
                .unwrap_or_else(|_| panic!("a full message queue must not block notices ({i})"));
        }
        assert_eq!(mb.len(tid("carol")), MAILBOX_CAP + 3);
        assert_eq!(
            mb.park_ok(tid("carol"), parked("msg-over2", ParkKind::Message, t0)),
            Err(ParkError::MailboxFull),
            "notices must not inflate the message denominator"
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
            "the exact TTL moment is expired (boundary inclusive) — not deliverable"
        );
        assert_eq!(
            drained.expired.len(),
            1,
            "the exact TTL moment surfaces as expired"
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
            "items past TTL are not inject targets"
        );
        assert_eq!(
            drained.expired.len(),
            1,
            "items past TTL surface as expired"
        );
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
        assert_eq!(
            deliverable,
            vec!["recent"],
            "only unexpired items are inject targets"
        );
        assert_eq!(
            expired,
            vec!["old"],
            "expiry does not vanish silently but becomes expired"
        );
        assert!(mb.is_empty(), "drain empties the queue (prevents re-park)");
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
        assert_eq!(expired.len(), 1, "only 1 expired item returned");
        assert_eq!(expired[0].msg.msg_id, "old");
        assert_eq!(
            expired[0].recipient,
            tid("e"),
            "must also return the recipient queue keys that had expired items (1:N ledger transition — C4)"
        );
        assert_eq!(mb.len(tid("e")), 1, "the recent one stays in the queue");
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
        assert!(
            mb.is_empty(),
            "when everything expires and it is empty, the queue must be removed from the map"
        );
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
        assert_eq!(
            expired.len(),
            2,
            "the same msg_id in the two member queues each expire"
        );
        // The order across queues is non-deterministic because of HashMap — view it as a set.
        let mut got: Vec<PeerId> = expired.iter().map(|e| e.recipient).collect();
        let mut want = vec![tid("alice"), tid("bob")];
        got.sort();
        want.sort();
        assert_eq!(got, want);
        assert!(
            expired.iter().all(|e| e.msg.msg_id == "g1"),
            "both are the same logical message (1 msg_id : N delivery records)"
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
            "re-parking bypasses the cap and puts back admitted items (0 loss)"
        );
    }

    #[test]
    fn restore_ordered_preserves_oldest_first_before_concurrent_parks() {
        // Scenario: drain took [old0,old1,old2] and new0·new1 were parked meanwhile → old0 delivered, then failed → re-park [old1,old2];
        //   queue [new0,new1] → after restore_ordered = [old1,old2,new0,new1].
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
            "re-parked items must come before new concurrent park items (oldest-first preserved)"
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
            "even after several restores the queue must keep the global acceptance order (oldest first)"
        );
        mb.park_ok(tid("dup"), parked("new", ParkKind::Message, t0))
            .unwrap();
        assert_eq!(
            mb.msg_ids(tid("dup")),
            vec!["m0", "m1", "m2", "m3", "new"],
            "re-parked items come before new ones"
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
            "new items parked concurrently come after the re-parked items"
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
            "the sweep must also take out expired items hidden behind unexpired ones (full scan)"
        );
        assert_eq!(
            mb.msg_ids(tid("z")),
            vec!["newer"],
            "unexpired items remain in order"
        );
    }

    #[test]
    fn restore_ordered_empty_is_noop() {
        let mut mb = Mailbox::new();
        mb.restore_ordered(tid("r"), Vec::new());
        assert!(mb.is_empty(), "an empty re-park does not create a queue");
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
            "the sweep also preserves oldest-first"
        );
    }
}
