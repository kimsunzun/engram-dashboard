//! service — MessagingService: the orchestrator that wires the pure structures (mailbox·ledger) into the
//! send pipeline on tokio (S18 messaging v1 · ADR-0103/0104 · **ADR-0111/0112/0114 send overhaul** · **ADR-0125 enqueue-then-drain**).
//!
//! ★Role★: owns **multi-recipient sending** (spec §5), appearance/idle flush (ADR-0104), the TTL sweep, the **idle gate**,
//!   and the **reply contract** (opening the request ledger entry · closing it on reply · overdue notice). There is
//!   **one** send entry point, `handle_send` — individual, multi-recipient and `@all` all ride the same rail (one path, ADR-0111 decisions 2/4).
//!
//! ★Enqueue → drain (ADR-0125 · spec §5 — the backbone of this file)★: a send is **enqueued at the queue tail without
//!   exception**, then within the same call that recipient's queue is drained from the front and the call returns (enqueue = inside the lock, drain = outside the lock).
//!
//! ★Three branches per recipient (spec §5 · **ADR-0116 decision 1** — multiple recipients run this decision once
//!   per recipient = fan-out)★. This decision looks at the **identity axis only** ("who is this recipient") — "can they receive right now" is
//!   left to the drain that follows (ADR-0125). There are **two** decision sources (roster / profile list) and the principle in one line is
//!   **"send if alive, wait if it may come alive later, fail loudly if it does not exist"**:
//!   1. **Entrance bounce → per-recipient failure row**:
//!      - **Not found** — **in neither** the roster nor the profiles (typo · unspawned · deleted) → `RECIPIENT_NOT_FOUND`.
//!      - Duplicate names are `RECIPIENT_AMBIGUOUS` in **any layer**. For a name spanning alive and asleep, **the alive side wins**
//!        (the roster decision comes first).
//!   2. **Alive recipient — enqueue, then the drain in the same call decides the outcome.** **Capability only decides
//!      "when to put it in"** (ADR-0116 decision 7 — it is not an eligibility condition).
//!   3. **Asleep — enqueue and mark `pending`** (not in the roster, but the **profile exists** so it can be restored — ADR-0116 decision 1).
//!      There is no live entity to drain to, so it stays in the queue until restore (re-appearance flush). This differs from the old
//!      "park even without a name": only names that exist in a profile are accepted, so there is no path where typos pile up silently.
//!   ★Mailbox overflow, in any branch, is `failed` + `MAILBOX_FULL` for **that recipient only**★ (it is not escalated to a whole bounce —
//!   spec §5 partial progress).
//!   **There is no wake** — an asleep recipient is only parked (spec §8 v2 candidate).
//!
//! ★The decision-source snapshot is **exactly one** per send (invariant — ADR-0111 decision 2 · sources extended by ADR-0116)★:
//!   re-taking it per recipient would bring back half-decisions caused by list changes mid-resolution.
//!   ★There are **2** physical lookups★: one session snapshot (`list_agents()`) + one profile list. The race with the profile list is
//!   bounded, so it is accepted (spec §5), and the residue is collected by TTL and delete cleanup.
//!
//! ★Scope of the ordering guarantee (finding 8 · ADR-0125 · load-bearing)★: the order a single recipient sees
//!   is the **enqueue order** — because a send goes onto the queue tail without exception and the drain pulls from the front. So there is
//!   no seat reservation and no "is anything ahead of me to go out first" joining decision (if it becomes necessary again, that is a sign the
//!   send path has split in two).
//!   **One** accepted residue remains: the global order **between different recipients** — it closes only by putting inject inside the lock
//!   (a head-on violation of the lock discipline), so it is accepted on purpose at human-conversation message rates.
//!
//! ★Single lock (load-bearing — in the spirit of ADR-0006)★: Mailbox+Ledger sit behind **one `Mutex<MessagingState>`**.
//!   There is no lock-order risk (one lock) and the message rate is extremely low (human conversation level), so contention is meaningless.
//!   ★Absolute discipline★: **never call `DeliveryPort` (inject/roster) while holding this lock** — finish what is decided under the lock
//!   (collecting parking/inject targets) first, release the lock, then call DeliveryPort (an external call). Breaking this lets
//!   inject take another lock internally (the sessions RwLock etc.), risking lock-order inversion and deadlock.
//!
//! ★Envelope = assembled at injection time (single wrap point, ADR-0096)★: parking stores the **unwrapped body + sender name + send
//!   meta**, and the envelope is built **at injection** by `wrap_message`/`wrap_notice` (the single wrap
//!   point, this crate's `envelope.rs`). Why: the envelope format (the colon/xml global switch) can change between parking and flush, and flush must wrap in the
//!   **current** format. So it carries the raw body + attribute materials (`ParkPayload`) and assembles in
//!   one place at the moment of injection — the envelopes of immediate delivery and late delivery **must be the same**.
//!
//! Workspace crate imports: 0 (ADR-0110 — enforced by the compiler).
// ADR-0103
// ADR-0104
// ADR-0111
// ADR-0112
// ADR-0114
// ADR-0116 (entrance 3 branches · no turn signal means immediate injection · asleep parking · reply-contract failure close · delete cleanup)
// ADR-0118 (contract lifetime join — guard first · 512 count)
// ADR-0121 (@all/@here split · ordering guarantee for the class without a turn signal · single definition of the gate predicate)
// ADR-0125 (enqueue everything then synchronous drain — direct-send shortcut and seat reservation abolished · delivered restored)
// ADR-0309 (internal mail key = agent id — a name becomes an id only at the entrance)

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::busy::{AlwaysIdleGate, BusyGate, BusyReason};
use super::envelope::{
    new_msg_id, wrap_message, wrap_notice, DeliveryObservation, Entrance, EnvelopeFields,
    EnvelopeFormat,
};
use super::groups::{
    normalize_group_name, BuiltinGroups, GroupError, GroupMember, GroupSource, MemberPools,
};
use super::ledger::{
    DeliveryStatus, DueTimeout, Ledger, OpenOutcome, ReplyFailOutcome, ReplyOutcome,
    ReservationLiveness, RetiredContract, TransitionError,
};
use super::mailbox::{FlightTicket, Mailbox, ParkError, ParkKind, ParkedMessage};
use crate::{PeerId, SenderIdentity};

/// ★The ledger sender label for a notice (C3)★ — `<notice>` is a daemon notice **with no from**, so there is no real sender.
///   The ledger record still requires a `from` field (no silent loss — a notice always lands in the ledger too), so a fixed
///   label marking the daemon as the source is used. ★Not an address★: no message can be sent to this string (it is not a
///   roster name) and it is never rendered in an envelope (a notice envelope has no from attribute at all — ADR-0103 invariant).
const NOTICE_SENDER_LABEL: &str = "engram";

// ★Removed (R1) — the old `STALE_RESERVATION_AFTER` (5 s deferral)★: a constant that decided reservation reclaim by **age**. Do not
//   revive it — no value is safe. The out-of-lock injection in a reservation span is a child stdin write, and that write can
//   **block without bound** on backpressure (agent `stdio.rs`). So "older than N seconds means abandoned" misjudges an owner that is still working,
//   and the result of that misjudgment was **delivering a request without a contract** (full failure chain = `ledger::ReservationLiveness` header).
//   The reclaim criterion is now the **liveness** of the owner guard (`ledger::reclaim_abandoned_reservations`).
// ★7th-round correction — the premise of the argument above changed (ADR-0125). The conclusion stands★: with everything queued, the reservation window (open→settle)
//   **now sits entirely inside the enqueue lock**, so the normal path no longer has an "out-of-lock injection in a reservation span". So do not
//   read the paragraph above and conclude "the unbounded block is gone, so reviving the age criterion is fine" — the real reason the age criterion is forbidden is that
//   **no threshold can stand in for deciding owner liveness**, and that does not depend on the window length. Reviving this
//   constant would reopen ADR-0108.
// ADR-0108 (reservation reclaim guarantee layer — criterion = liveness)
// ADR-0125 (the reservation window moved inside the enqueue lock — argument premise corrected)

/// ★Why it carries **both** the raw notation and the parsed value (load-bearing)★: the envelope attribute `reply-by` must show the **notation
///   exactly as the sender wrote it** (`"10m"`) (spec §1 — the contract the receiving LLM reads), and the ledger timer needs a
///   `Duration` to compute the **absolute deadline** (spec §3 "the daemon converts to absolute time"). Carrying only one forces re-parsing and re-rendering
///   on the other side, and the notation drifts subtly (`60m` → `1h`).
/// ★`Default` = notification★: all fields inactive is the default message (no type).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SendMeta {
    pub request: bool,
    /// The deadline notation exactly as the sender wrote it (`"10m"`). The value of the envelope `reply-by` attribute. Meaningful only for `request`.
    pub reply_by_raw: Option<String>,
    /// The parsed deadline — the ledger fixes the absolute deadline as `발송시각 + 이것` (send time + this). If raw is Some, this must be Some.
    pub reply_by: Option<Duration>,
    pub reply_to: Option<String>,
    /// ★The value of the envelope `to` attribute (spec §1 — redefined from "group label" by ADR-0111)★. If `Some`, that string becomes
    ///   the XML `to` attribute as is. The shape of the value = the **canonical names of accepted explicit targets + `@`address tokens (not expanded)**
    ///   joined in **input notation order** (`"@all,carol"`).
    ///
    /// ★Exposure condition = 2 or more recipients decided as accepted★ (not whether it is a group — ADR-0111 abolished group-only semantics,
    ///   so an `@all` send and a direct multi-person target take the same path). What is counted is the **acceptance
    ///   decision**, not the resulting state — `delivered` is only fixed at the actual injection time, so using it as the criterion would make a cycle
    ///   that references a value still unknown when the envelope is built.
    /// ★Why it rides on `SendMeta` (load-bearing)★: because a parked item must carry **the same envelope even when delivered late**.
    ///   The envelope is assembled at injection time, not park time (module header "envelope = assembled at injection time"), so the attribute materials
    ///   must ride in `ParkPayload` and survive until flush — **exactly the same reason** as the contract fields (request/reply_to),
    ///   so they take the same channel. Creating a separate axis would make two flush paths.
    /// ★The entrance does not fill this value★: its value is fixed by `handle_send` exactly once, after every recipient's acceptance
    ///   decision has finished (pinned with debug_assert) — so that all envelopes of that send carry the **same** `to`.
    // ADR-0111
    pub to_attr: Option<String>,
}

impl SendMeta {
    /// The attributes this send puts on the envelope (spec §1 exposure principle — **only fields that change behavior**). The send argument `reply_to`
    /// appears as the receive attribute `in-reply-to` (spec §1 notation mapping).
    ///
    /// ★`id` only on a request★: putting an id on a notification/reply envelope confuses the receiving LLM about "do I need to reply to this" —
    ///   an exposed field is itself a behavior signal, so an unneeded axis is hidden entirely (spec §1 · ADR-0103 decision 1).
    /// ★Visibility `pub` (promoted by the ADR-0110 move)★: unit tests of the host entrance (daemon ingress) assert "validated arguments →
    ///   envelope" in one line (so tests do not bypass and re-implement the single source of the attribute assembly rules).
    pub fn envelope_fields(&self, msg_id: &str) -> EnvelopeFields {
        EnvelopeFields {
            id: self.request.then(|| msg_id.to_string()),
            msg_type: self.request.then(|| "request".to_string()),
            reply_by: if self.request {
                self.reply_by_raw.clone()
            } else {
                None
            },
            in_reply_to: self.reply_to.clone(),
            to: self.to_attr.clone(),
        }
    }
}

/// The send result for one recipient (an element of spec §6 `results[]`). The upper layer (the entrance) converts it to wire JSON.
///
/// ★One recipient = one row (spec §5 resolution order ④)★: duplicate targets (`@all` + the same name targeted directly) were already
///   folded in the resolution step, so the same recipient cannot appear twice here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecipientResult {
    /// Recipient notation — **the token the sender wrote** (trimmed only, WYSIWYA · ADR-0101). The `@all` expansion result is the roster name.
    pub to: String,
    pub status: SendStatus,
    /// The failure code, **required** when `Failed` (spec §6 row code). `None` otherwise.
    pub code: Option<FailCode>,
    /// Self-correction hint (parking reason · failure reason). `None` for `Delivered`.
    pub hint: Option<String>,
}

/// The outcome vocabulary for one recipient (spec §6 — `delivered|pending|failed`).
///
/// ★`skipped` was **abolished from the response vocabulary** (ADR-0111 decision 3)★ — it disappeared together with the group-member skip rule.
/// The `skipped` in the ledger terminal vocabulary (retirement of the notice lane) is a different axis and remains (spec §6 mapping table).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendStatus {
    /// Actually injected — ledger `delivered` (ADR-0104 actual injection time).
    Delivered,
    /// Parked — ledger `pending`.
    Pending,
    /// ★Failure for that recipient only (new in ADR-0111 decision 3)★ — the other recipients are still delivered as is (partial progress).
    Failed,
}

/// Per-recipient failure code (spec §6 row code — a **different axis** from the send-level bounce code).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailCode {
    /// ★A name in **neither the roster nor the profiles** (typo · unspawned · already deleted)★
    ///   (ADR-0116 decision 1): an asleep session is parking (`pending`), not this code, and **a live peer with no structured
    ///   output is not this code either** — it is injected immediately with no gate and delivered normally (decision 7).
    ///   ★The old `RecipientUnreachable` was discarded★ — there is no state
    ///   for that code to point at. Reviving it needs a new decision first (an alternative ADR-0116 rejected).
    // ADR-0116 (meaning narrowed · decision 7)
    RecipientNotFound,
    /// Two or more agents share the same name — the daemon has no basis to choose whom to send to (ADR-0114 decision 4 transitional rule).
    ///   ★The same code in any layer★: not only the alive layer (duplicate roster names) but also the **asleep layer** (two or more
    ///   profiles deriving the same name) fails with this code — a name cannot pick a single recipient
    ///   (ADR-0116 decision 1). ADR-0115 (spawn name uniqueness) will make this class a dead letter.
    RecipientAmbiguous,
    /// That recipient's mailbox (message lane, 100 items) is full — **immediately, with no reclaim attempt** (ADR-0114 decision 1).
    MailboxFull,
    /// That recipient's reply contract cannot be opened (open contract limit 512, ADR-0108). **No delivery to that recipient either** —
    ///   tracking replies is the essence of a request, so delivery without tracking violates the contract (ADR-0114 impact section).
    RequestCapacity,
    /// ★The recipient profile was deleted while the item waited parked, so it was closed as undelivered (ADR-0116 decision 3)★.
    ///
    /// ★**Never appears in the send response** (spec §6)★: at send time it was `pending`, and this code is stamped after the fact by delete cleanup —
    ///   so the sending LLM first sees it in an `eg_messages{id}` **lookup** (which is why the lookup row carries both the code and
    ///   the hint — `message_state`). The **timing differs** from `RecipientNotFound` ("did not exist from the start"): the sender
    ///   must tell "I wrote the name wrong" from "I waited and the peer got deleted", because the resend decision
    ///   differs.
    // ADR-0116 (decisions 3/4)
    RecipientDeleted,
}

impl FailCode {
    /// The wire code string (stable contract — the sending LLM branches on this value, spec §6).
    pub fn as_str(self) -> &'static str {
        match self {
            FailCode::RecipientNotFound => "RECIPIENT_NOT_FOUND",
            FailCode::RecipientAmbiguous => "RECIPIENT_AMBIGUOUS",
            FailCode::MailboxFull => "MAILBOX_FULL",
            FailCode::RequestCapacity => "REQUEST_CAPACITY",
            FailCode::RecipientDeleted => "RECIPIENT_DELETED",
        }
    }
}

/// ★Send-**level** bounce (spec §5·§6 `{status:"error", code, hint}`)★ — only problems that cannot be split per recipient.
///
/// ★Layering (ADR-0114 decision 3 — the reason this enum exists)★: **absence of a name = runtime state** (that agent just
///   is not there at that moment; the notation is fine) → per-recipient failure row / **an `@`address error = an address-space error** (a nonexistent address syntax ·
///   a mistyped group name = something the sender must fix and resend) → whole bounce. This one line sets the boundary of partial progress.
/// ★Argument errors (`INVALID_SEND_ARGS`) and oversized body (`BODY_TOO_LARGE`) are not here★ — the entrance (ingress) bounces them before
///   they come down to the service (the entrance-agnostic single validation point).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SendReject {
    /// An `@주소` (an @address) other than `@all` (or an `@` rule violation) → `GROUP_NOT_FOUND`. Even with a mixed `to` it is a **whole bounce**
    ///   (nothing goes to the alive recipients either — so a typo does not slip through half-succeeded).
    GroupNotFound { name: String },
    /// The **final recipient set** (expansion ∪ explicit targets) is empty → `GROUP_EMPTY` (ADR-0114 decision 3 — judged on the set after
    ///   resolution completes, not per element). Typical case: `["@all"]` where the sender is the only survivor.
    GroupEmpty,
    /// ★The logical message id already exists in the ledger★ (practically impossible: a 2.8×10^12 space). If the same id is present in
    ///   **either** the history record or the request tracking, this is the value.
    ///
    /// ★No-side-effect guarantee (load-bearing — the caller retry contract)★: the id check is this function's **first side-effect point**, so
    ///   when it returns this value, no delivery, parking or ledger record has happened. So the caller (ingress) can
    ///   call again with a new id as is and retry safely with no duplicate delivery.
    IdCollision,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ProfileDeletedOutcome {
    /// ★Condition not met★ — the session of that profile id is still alive, so nothing was done.
    ///   When this value is `true`, all remaining fields are 0.
    pub skipped_live: bool,
    /// The number of parked items closed as `failed` + `RECIPIENT_DELETED` (any lane — including notice).
    pub failed_parked: usize,
    /// The number of open contracts closed as `reply_failed` (only those where that profile id is the **requester**).
    pub failed_contracts: usize,
    /// The number of contracts skipped because of the guard (provisional · slated for retirement).
    pub guard_held_contracts: usize,
}

/// ★Injection receipt (ADR-0110 decision 2 — this crate's own type)★ — a 4-field value `DeliveryPort` returns to prove it
///   "actually plugged in the envelope bytes". The host adapter copies its own write result into this shape.
///
/// ★Why not take the host type directly★: that would make this crate know the host crate (agent), breaking full mutual
///   ignorance. All four fields are pure scalars, so the boundary conversion costs a single copy (ADR-0110 rationale).
/// ★Meaning (observation correlation — ADR-0088)★: `msg_uuid` is the replay-dedup key the recipient session assigned to this user turn,
///   and `epoch` is the generation of the incarnation that **executed** the write (not a resolution-time snapshot — that asymmetry
///   lets a mid-flight epoch race be asserted from the record alone).
/// ★`bytes_written` is not a short-write detection axis★: on success it equals `bytes_requested` by construction
///   (the primary evidence of delivery completeness is Ok/Err — `DeliveryObservation::is_delivered`).
// ADR-0110
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InjectReceipt {
    /// The byte count of the logical message the injection requested (envelope string length).
    pub bytes_requested: usize,
    pub bytes_written: usize,
    pub msg_uuid: uuid::Uuid,
    pub epoch: u32,
}

/// ★Control-plane port (ADR-0110 decision 3 — the port lives in the lib, the adapter in the host)★ — a narrow contract holding only the **two things**
///   the service asks the host's control plane: ① the envelope format to use now ② recording a delivery observation record.
///
/// ★Why only these two (measured basis)★: before the split the service held the daemon's whole `ControlRegistry`, but the actual
///   calls were only `envelope_format()` and `record_delivery()` (ADR-0110 rationale). With the port trimmed to the surface actually used,
///   a future host (a standalone mail service with no agent manager at all) has only two things to implement.
/// ★fail-open property★: both methods are **optional features** — the format has a default (Xml) and observation is a no-op
///   in the production daemon. Delivery goes through even if the implementation does nothing (ADR-0110 impact §"turn observation and envelope
///   format are optional").
/// ★Out-of-lock call contract★: the same as `DeliveryPort` — it is called only **with the messaging lock released**.
// ADR-0110
pub trait ControlPlanePort: Send + Sync {
    /// The envelope format to use for assembly now (host global state — switchable at runtime, ADR-0096).
    fn envelope_format(&self) -> EnvelopeFormat;
    /// Records one delivery-boundary observation record to the host sink (ADR-0088).
    fn record_delivery(&self, obs: DeliveryObservation);
}

/// The delivery-boundary seam (ADR-0012) — MessagingService does not call the host's real agent directly but calls across this.
///
/// ★Why a trait★: a seam so headless unit tests can verify the 3 branches · flush · sweep deterministically without a claude
///   binary, a real PTY or a spawn pipe (FakeDeliveryPort). The production implementation is host-owned
///   (daemon `messaging_host::ManagerDeliveryPort` — ADR-0110 decision 3).
/// ★Out-of-lock call contract (load-bearing)★: this trait's methods are called only with MessagingService's messaging lock
///   **released** (module header lock discipline). That premise is set so it is safe for an implementation to take other locks internally.
pub trait DeliveryPort: Send + Sync {
    /// Injects the finished envelope bytes into the recipient (= a write to the recipient's stdin). `Err` = unreachable or write failure, and
    /// **the upper layer handles it as parking**. The upper layer has already finished envelope assembly.
    ///
    /// ★The contract = up to "the recipient **receives this and starts processing** it"★: some recipients are not done once the bytes are
    ///   pushed into the channel (a class that buffers input and waits for a separate submit signal). What submits it and with how many
    ///   writes is **knowledge behind the host adapter**, and this crate only hands over the envelope bytes — branching on
    ///   the recipient kind here would make the kernel know the host (ADR-0110 no-dependency). ★So do not work around it by appending a submit
    ///   character to the envelope★: the envelope is content and submission is a channel act.
    ///
    /// ★This is an **unconditional-incarnation** injection★: it writes to whatever session that PeerId points at now — even if a restart
    ///   raised the epoch, it lands on the new incarnation. That is **correct**: the unit of address is the name and re-spawn takeover is
    ///   a feature (ADR-0101). ★The old `inject_if_epoch` (conditional injection to bind to the incarnation at send time) was removed★ —
    ///   since the binding itself was abolished (ADR-0111 decision 6), no caller remains to use a conditional write. To revive it, first
    ///   formally reopen "this letter goes only to the incarnation at send time" as a v2 personal-mail option (spec §8).
    // ADR-0111 (binding abolished — conditional injection verb removed)
    fn inject(&self, to_id: PeerId, bytes: &[u8]) -> Result<InjectReceipt, String>;

    /// ★Roster = every agent that **currently has a process attached** (`Running`|`Exiting`)★ — a snapshot of (name, id, epoch,
    /// turn_signal). Shared by resolve (name→id) · flush (appearance/epoch-change detection) · `@`address expansion · the entrance decision.
    ///
    /// ★**Turn observability** is not an eligibility condition (load-bearing — ADR-0116 decisions 1·7)★: it does not filter on
    ///   structured output (the turn signal). The old predicate also required `structured`, dropping terminal/console-mode sessions from the roster
    ///   and bouncing that class (`RECIPIENT_UNREACHABLE`), but that premise was wrong (basis = `LiveAgent::turn_signal`).
    ///
    /// ★But there is one more axis — **what the input is interpreted as** (2026-08-17 user decision)★: the host removes the class that
    ///   *executes letters without reading them* (shells) from this list. This is a **different question** from the clause above — not "can the turn be
    ///   observed" but "is this something that reads letters". So a terminal claude still comes in.
    ///   ★Do not merge the two and revert to "list = every live session"★ — reverting makes a shell execute the envelope as a command.
    ///   Capability is an axis that decides **"when to put it in"**, not membership.
    /// ★It is not "is in the session list" (load-bearing)★: a session stays in the map until the reaper collects it, so judging by
    ///   mere existence misclassifies an **agent that just exited** as a delivery target instead of asleep (parking). This predicate
    ///   was found undefended in mutation measurement (deleting it left every test green) → the real-adapter-level sealing test is canonical.
    // ADR-0116 (roster predicate = state only · decision 7)
    fn live_agents(&self) -> Vec<LiveAgent>;

    /// ★Entrance decision source: **a one-shot snapshot** (spec §5 3 branches — ADR-0116)★: called **exactly once** per send.
    ///
    /// ★Why bundle into one verb instead of splitting into two lookups (load-bearing)★: the 3 branches look at two predicates (roster / profile), and
    ///   splitting the lookups lets the list change in between, bringing back the half-decision where **each recipient within one send sees a different
    ///   world** (the class ADR-0111 decision 2 forbade). Bundling into one verb makes the adapter guarantee "1 session snapshot + 1 profile"
    ///   **structurally**.
    /// ★Implementation contract (what the adapter must keep — breaking it misclassifies silently)★:
    ///   - `roster` = the **same predicate** as `live_agents()` (`Running|Exiting` + only the class that reads letters — there is
    ///     still no capability condition), derived from the **same single** `list_agents()` snapshot. Taking it twice is itself a defect
    ///     (re-querying revives TOCTOU, non-atomicity and per-session locks together — 2026-08-17 review).
    ///   - `dormant` = the (id, canonical name) pairs of profiles **whose session is not alive**. The id is that
    ///     profile id (= the session id when relaunched), and the name must be **derived by the same rule as a live session** (spec §5 —
    ///     if it differs, a recipient called by name cannot be found). **Duplicates are not folded** for the duplicate-name decision (two of the same name
    ///     stay two).
    // ADR-0116 (2 decision sources — 2 physical lookups)
    // ADR-0309 (decision 3 — the asleep side is also an (id, name) pair)
    fn addressing_sources(&self) -> AddressingSources;

    /// ★Is this id's session **alive right now** (`Running`|`Exiting` = the same state predicate as the roster — mail eligibility is not checked)★.
    ///
    /// ★Reason to exist = the delete-cleanup gate (spec §5 · ADR-0116 decision 3 — review fix D1)★. The rationale for the gate to be
    ///   on the **id axis** (rename immunity) is canonical in the `handle_profile_deleted` doc. The profile id equals the session id
    ///   (`activate_profile`) and is not shaken by a rename, so it is used as the axis.
    /// ★Why not use `addressing_sources().roster`★: that is a **send-purpose** snapshot that also drags in the profile-list lookup + asleep-name
    ///   derivation (fs access in some cases). The gate needs just one single predicate, so a narrow verb is kept
    ///   separately (the adapter shares the same `is_live` predicate so the two decisions do not diverge).
    /// ★Do not substitute `live_agents()`★: that is the **recipient list**, so alive sessions the host filtered out by mail eligibility
    ///   (shells etc.) are missing from it. This question is not "does it receive letters" but "is it alive" — asking the list means that when the profile of
    ///   such a session is deleted, the contract that session is still waiting on (a request made from a shell via the CLI etc.) is closed as a failure.
    // ADR-0116 (decision 3 — delete cleanup gate) / review fix D1
    fn is_agent_live(&self, id: PeerId) -> bool;
}

/// ★The entrance decision source snapshot (spec §5 3 branches · ADR-0116)★ — the output of `DeliveryPort::addressing_sources`.
///
/// The two fields are **facts at one moment**, so they must be consistent with each other: `dormant` is the profiles with no session in
/// `roster` (the intersection is empty). The adapter guarantees that consistency (the implementation contract of the method above), and the service
/// decides the 3 branches assuming it.
#[derive(Debug, Default, Clone)]
pub struct AddressingSources {
    /// **The only source the `@here` expansion looks at** (spec §4 — "everyone here right now"). `@all` looks here **plus**
    ///   `dormant` (ADR-0121 decision 1 — if the two vocabularies looked at the same source, that would violate the decision).
    pub roster: Vec<LiveAgent>,
    /// ★**Duplicates are not folded** — this is the input of the duplicate-name decision for name targets★: if two share a name, a recipient called by that name
    ///   is `RECIPIENT_AMBIGUOUS`. The `@all` expansion resolves by id, not by name, so the two each get their own row
    ///   (ADR-0309 · TRD S29 §4 X1).
    pub dormant: Vec<DormantAgent>,
}

/// ★Flush doorbell seam (C2 review fix 11)★ — an exit that **hands off** "flush this agent's parking queue now" **to another thread**.
///   Production does a non-blocking enqueue onto the flush channel (host-side `messaging_host.rs`
///   `ChannelIdleNotifier` — the kernel does not know that implementation).
///
/// ★Why it is needed (ADR-0125)★: inject is a child stdin **blocking write**, so if the send path
///   (the tokio worker thread handling an MCP/HTTP request) pushes other people's queues in batches too, one blocked pipe eats the daemon's request
///   handling. But **the sending thread emptying the queue of the recipient it targeted is now the normal path**
///   (synchronous drain — ADR-0125 decision 1 explicitly accepted that cost). So the share this doorbell peels off is
///   **only queues unrelated to its own send**: someone else's drain request that backed off (paying back the deferral marker) · idle transition · re-appearance.
/// ★Contract★: `request_flush` is **non-blocking** (a channel enqueue only). The actual flush is done by the consumer (the flush lane).
/// ★Unwired fallback (two documented branches)★: an assembly that does not plug in the doorbell (experiment bin · unit tests) falls back to
///   an **inline flush** — a safe default (fail-open) so the absence of the doorbell does not spread into "delivery stops". The cost is only
///   that the calling thread carries the batch write in that assembly, and the production assembly (lib.rs) always plugs in the doorbell.
pub trait FlushTrigger: Send + Sync {
    /// Duplicate requests are folded by the consumer (coalescing).
    fn request_flush(&self, id: PeerId);
}

#[derive(Debug, Clone)]
pub struct LiveAgent {
    pub id: PeerId,
    pub name: String,
    pub epoch: u32,
    /// ★Can the turn boundary be observed (= does the idle gate hold for this recipient) — ADR-0116 decision 7 · load-bearing★.
    ///
    /// `true` (structured-output backend) → ask `BusyGate` before injecting and park if busy.
    /// `false` (terminal/console mode) → **do not ask the idle gate**. The basis is the observation that that CLI has **its own input queue**
    /// (it holds input during a turn and consumes it after the turn) — we have no reason to observe idle. So this
    /// value decides **only timing, not membership**.
    /// ★What this value does **not** decide = order (ADR-0121 decision 2 · ADR-0125)★: the basis above justifies only "is it ready to receive"
    /// and does not justify "in what order does it arrive".
    // ADR-0116 (decision 7)
    // ADR-0121 (decision 2 — order is class-independent)
    pub turn_signal: bool,
}

/// One asleep agent — it has a profile and no live session (spec §5 branch 3).
///
/// `id` = the profile id, so when relaunched that value is the session id (a new incarnation of the same agent). The reply contract addressed to an asleep recipient
/// carries this id from the moment it is opened (ADR-0309 decision 1).
// ADR-0309
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DormantAgent {
    pub id: PeerId,
    pub name: String,
}

struct MessagingState {
    mailbox: Mailbox,
    ledger: Ledger,
    /// ★The deferred-flush re-doorbell ledger (round-7 · round-8 correction · load-bearing)★ — key = the park queue (recipient id),
    /// value = **every doorbell id that backed off** for that queue (a de-duplicated set).
    ///
    /// `drain_queue` **does not run overlapped** for the same recipient (if the earlier batch is injecting outside the lock, a later batch
    /// would overtake its remainder — an order inversion). So a later drain backs off without touching the queue, but if it only backs off
    /// that wake-up **evaporates** (lost wakeup — the same failure mode as C1 finding 3). Leaving a fact here means
    /// **the one holding the receipt** (the drain in progress) finishes settling and, on the way out, presses the doorbell again.
    /// ★In round 7 this marker is also the trigger that opens a concurrent send's letter (ADR-0125)★ — the backed-off sender answered the response as
    /// `pending` (cannot confirm), and the outcome of that letter is opened either by the winning batch or by this echo.
    ///
    /// ★The value has a set shape, but the id it holds is always just that queue's id★ — what a backed-off drain records as the waiter is the queue id
    /// (`drain_queue` step 0 · ADR-0309). The shape cleanup goes together with the ADR-0142 target axis (TRD S29 §3).
    /// ★Bounded★: duplicates are not inserted, so at most one per key. A linear `contains` is enough.
    /// ★Why not "if the queue is not empty, always re-doorbell"★: that would make flush restart itself endlessly in the normal state where parking
    /// persists because delivery is impossible (busy · no target). Only when there was a real deferral does it echo exactly once
    /// per id that deferred (the set is finite and taken out whole at settle time, so termination is unchanged).
    // ADR-0107 (flush overlap deferral marker — id set, everyone retakes)
    deferred_flush: HashMap<PeerId, Vec<PeerId>>,
    /// ★Target-axis in-flight — the set of **sessions currently being injected into** outside the lock (the resolved target PeerId)★.
    ///
    /// ★The occasion that set up this axis (ADR-0142) does not come from the id key★: back then the delivery target was a per-item resolution result (hint
    ///   first → unique name reachable), so **two queues of different names could converge on the same session**, and when they overlapped, since one delivery is
    ///   body write → wait for submit → submit (CR) write, the second body reached the session ahead of the first CR and **two envelopes were
    ///   submitted merged into one turn**. Since the queue key became the id (ADR-0309), the target is always that queue's single id, so that convergence is gone. It does no
    ///   harm to correctness, so it was left in place (TRD S29 §3 · §7 Q2 — to remove it, amend ADR-0142 first). The invariant below must be
    ///   kept as is while it remains.
    /// ★A set is enough (not a refcount)★: one drain registers at most once per target (the group is
    ///   unique per target), and two drains grabbing the same target
    ///   at once is exactly what this guard prevents.
    /// ★`FlightSettle` alone bears the release★ — a leak = that session **never** receives mail again.
    /// ★This axis has no fail-open and no sweep (intended — unlike its neighbor axes)★: the recipient-id axis denominator converges by `saturating_sub`,
    ///   and the busy gate is woken by the `BUSY_MAX_TURN` cap, but getting trapped here is **silently permanent**. The only invariant that prevents it
    ///   is one — **on a path where `insert` ran, until the `FlightSettle` that owns its release is constructed, there must be nothing
    ///   that can fail (early return · `?` · a call that panics)** (after construction, whichever path exits, Drop repays it). "Failure" here
    ///   means a failure that **unwinds** — something that ends in abort, like an allocation failure, leaves no process to repay, so it is out of scope.
    // ADR-0142
    in_flight_targets: std::collections::HashSet<PeerId>,
    /// ★Target-axis deferral waiters★ — key = the **target id** being injected at that time, value = the doorbell ids that backed off because of that target.
    ///
    /// ★Why it could not be written into `deferred_flush` (ADR-0142)★: that ledger's key is the **park queue**, but a target-axis overlap
    /// happened between two different queues — the winner takes out only its own queue's key, so it does not see the loser's marker
    /// (lost wakeup). The **wake path this marker opens is the same**: right after settle it is **merged** with the `deferred_flush` share and goes out through the same
    /// `request_flush` loop (step 6 tail).
    /// ★Residue — if the winner unwinds mid-injection, this marker is not pressed (same property as `deferred_flush`)★:
    /// `FlightSettle` (Drop) always releases the target hold, but the step-6 echo sits on the stack above it. Then the backed-off
    /// side's letter **stays in the queue** and waits for the next appearance, a turn-end notification, or TTL (not lost). The marker is also cleared
    /// together at the next settle of that target's drain. In the production daemon the hook ends an uncontained panic before it unwinds, so this
    /// branch does not exist there — it is reachable only in test builds without the hook (ADR-0292).
    // ADR-0142
    deferred_by_target: HashMap<PeerId, Vec<PeerId>>,
}

/// ★Shared★: built as an `Arc<MessagingService>` at daemon boot, and ingress (handle_send) · the flush observer · the sweep
///   task share the same instance (lib.rs run()).
pub struct MessagingService {
    state: Mutex<MessagingState>,
    /// ★`@`address resolution source (seam — ADR-0104 decision 1 · ADR-0112 decision 1)★. v1 has only the built-in `@all` and it is **stateless**, so
    ///   it is an out-of-lock field (stored groups are gone, so there is no shared mutable state — groups.rs header).
    groups: BuiltinGroups,
    port: Arc<dyn DeliveryPort>,
    registry: Arc<dyn ControlPlanePort>,
    /// Production uses `BusyPolicy`; unwired/unobservable uses `AlwaysIdleGate` (immediate-injection fallback — busy.rs header).
    busy: Arc<dyn BusyGate>,
    /// `None` = unwired → inline drain fallback (the two documented branches in the FlushTrigger comment).
    trigger: Option<Arc<dyn FlushTrigger>>,
    /// ★Acceptance critical-section observation hook (test only — review fix D2 atomicity assertion)★: **while still holding the very lock that recorded
    ///   the acceptance**, it shows the ledger as is. The firing point is **one** place, just before leaving the `handle_send` main critical section.
    ///
    /// ★Why it must be shaped this way (determinism)★: "acceptance and contract closing in one critical section" **cannot be asserted from the final
    ///   state alone** — even split across two locks the result is the same without a race. So the test checks **inside the lock** whether the contract is
    ///   already closed: if the closing slips out to another lock, the observation at this point is `awaiting_reply` and goes red immediately (no sleep · no thread).
    /// ★Hook contract★: reads only the ledger handed to it — **does not re-take the service lock** (a std Mutex is non-reentrant = deadlock).
    /// ★Why `OnceLock`★: reads need no lock (called while holding the state lock), so it does not touch the lock-order convention.
    #[cfg(any(test, feature = "test-harness"))]
    accept_hook: std::sync::OnceLock<Arc<dyn Fn(&Ledger) + Send + Sync>>,
    /// Calls of `flush_for` that have **returned**, per recipient id (test only) — including those reached through
    ///   `flush_for_agent`; a send's own `drain_queue` is not counted.
    /// A leaf lock, taken only after `drain_queue` has returned — never held with `state`.
    #[cfg(any(test, feature = "test-harness"))]
    async_drains_done: Mutex<HashMap<PeerId, usize>>,
}

impl MessagingService {
    /// An assembly that verifies the gate uses `new_gated`.
    pub fn new(port: Arc<dyn DeliveryPort>, registry: Arc<dyn ControlPlanePort>) -> Self {
        Self::new_gated(port, registry, Arc::new(AlwaysIdleGate))
    }

    pub fn new_gated(
        port: Arc<dyn DeliveryPort>,
        registry: Arc<dyn ControlPlanePort>,
        busy: Arc<dyn BusyGate>,
    ) -> Self {
        Self {
            state: Mutex::new(MessagingState {
                mailbox: Mailbox::new(),
                ledger: Ledger::new(),
                deferred_flush: HashMap::new(),
                in_flight_targets: std::collections::HashSet::new(),
                deferred_by_target: HashMap::new(),
            }),
            groups: BuiltinGroups,
            port,
            registry,
            busy,
            trigger: None,
            #[cfg(any(test, feature = "test-harness"))]
            accept_hook: std::sync::OnceLock::new(),
            #[cfg(any(test, feature = "test-harness"))]
            async_drains_done: Mutex::new(HashMap::new()),
        }
    }

    /// Installed only once (idempotent, later calls ignored).
    #[cfg(any(test, feature = "test-harness"))]
    pub fn set_accept_hook_for_test(&self, hook: Arc<dyn Fn(&Ledger) + Send + Sync>) {
        let _ = self.accept_hook.set(hook);
    }

    #[cfg(any(test, feature = "test-harness"))]
    fn fire_accept_hook(&self, ledger: &Ledger) {
        if let Some(h) = self.accept_hook.get() {
            h(ledger);
        }
    }

    /// Why it is not taken as a constructor argument: the doorbell is the flush channel = wiring **assembled after** the service, and an assembly
    ///   that does not verify the gate (experiment bin · unit tests) must work without plugging it in (fallback = inline flush).
    pub fn with_flush_trigger(mut self, trigger: Arc<dyn FlushTrigger>) -> Self {
        self.trigger = Some(trigger);
        self
    }

    /// ★The **single query point** of "is this recipient in a turn right now" (ADR-0121 §impact invariant · load-bearing)★.
    ///
    /// The idle gate applies **only to backends whose turn boundary can be observed** (ADR-0116 decision 7). It bundles the class decision
    /// (`turn_signal`) and the actual busy lookup **inside one function**, making it impossible for decision points to use different predicates —
    /// if the same condition is written separately in several places, only one gets fixed and the two points **see different worlds**
    /// (the class of defect the 2026-07-30 review caught elsewhere).
    /// Even with one call site, the discipline of keeping the predicate in one place is kept (if the class decision splits inside the drain, the same
    /// accident recurs).
    ///
    /// ★The basis for not asking the gate for the class with no turn signal does not cover order★: "is it ready to receive" and "in what order
    /// do letters arrive" are different problems, and the latter is answered by the **enqueue order** (ADR-0125). So this function does not
    /// look at the queue (mixing that in here would tangle the two rules into one condition again).
    // ADR-0121 (single definition of the gate predicate)
    // ADR-0116 (decision 7 — the class with no turn signal skips the gate)
    fn gate_busy_reason(&self, target: &LiveAgent) -> Option<BusyReason> {
        if !target.turn_signal {
            return None;
        }
        self.busy.busy_reason(target.id, target.epoch)
    }

    /// The only send function the entrance (ingress) calls after finishing argument validation and auth.
    ///
    ///   - `msg_id`: the logical message id assigned by the upper layer (ledger correlation · the response `id` axis).
    ///   - `from`: the sender identity (token-derived, ADR-0086). Used for the observation record and the `@all` sender exclusion.
    ///   - `sender_name`: the envelope sender display name (already resolved by the upper layer as canonical — WYSIWYA ADR-0101).
    ///   - `to`: the **recipient token list**. Each element = an agent name · an exact PeerId string · an `@`address (mixing allowed).
    ///     Comma splitting is **CLI-entrance only** and is done before reaching here (MCP array elements are never split —
    ///     spec §6). An empty list is bounced by the entrance as `INVALID_SEND_ARGS`, so it is non-empty here.
    ///   - `body`: the **unwrapped** body (envelope assembly is wrap_message at injection time — the single wrap point).
    ///
    /// ★Return★: N per-recipient result rows (`results[]`, spec §6) or a **send-level bounce** (`SendReject` — only address-space
    ///   errors and id collisions. Zero side effects). **Even if every recipient fails it is `Ok(전 행 failed)` (all rows failed)** — it is not escalated to a whole bounce
    ///   (a single missing recipient is no exception either: the answer is one `failed` row, not an `error` — spec §5 partial progress).
    ///
    /// ★Response vocabulary★: if the drain **actually injected this letter, `delivered`**; if it could not confirm, `pending`
    ///   (vocabulary definition = spec §6).
    ///
    /// ★Per-recipient 3 branches (spec §5) — the module header is canonical★. This function splits that decision into **two passes**.
    ///   ★Why two passes (load-bearing)★: parking stores envelope materials, but `to` cannot be known before every recipient is decided.
    ///   With one pass, the first recipient's parked item would fix an **incomplete `to`**, and the envelope would diverge from later deliveries.
    ///
    /// ★Reply contract (spec §3 · ADR-0111 decision 5)★: if `meta.request`, a contract is opened **for each accepted recipient** (key =
    ///   `(msg_id, 수신자 id)` (msg_id, recipient id) — an asleep recipient too uses its profile id · ADR-0309). That open decision finishes **before enqueue**
    ///   — deferring it would deliver a request with no tracking (spec §3 item 5).
    ///   If `meta.reply_to`, once the send is accepted the contract is closed by strict matching on the replier basis (delivery is unaffected even if this fails).
    // ADR-0111 (multi-recipient fan-out · absence bounce · partial progress)
    // ADR-0114 (MAILBOX_FULL row failure — no reclaim attempt)
    // ADR-0125 (enqueue everything then synchronous drain — direct-send shortcut abolished · delivered restored)
    #[allow(clippy::too_many_arguments)]
    pub fn handle_send(
        &self,
        msg_id: &str,
        from: SenderIdentity,
        sender_name: &str,
        to: &[String],
        body: &str,
        entrance: Entrance,
        meta: &SendMeta,
    ) -> Result<Vec<RecipientResult>, SendReject> {
        // ★Mutual exclusion: ingress is the **only** validator★: this function and `SendMeta` are pub, so another assembly (smoke
        //   bin · a future entrance) can call them directly, and carrying request+reply_to together makes one send open a contract while
        //   closing someone else's — a tangled state. The reason **validation is not duplicated** here is that the bounce code/wording
        //   must not diverge per entrance (entrance-agnostic); instead a debug build blows up a wiring mistake.
        debug_assert!(
            !(meta.request && meta.reply_to.is_some()),
            "ingress가 유일 검증자 — request와 reply_to는 상호배타(spec §6)"
        );
        debug_assert_eq!(
            meta.reply_by.is_some(),
            meta.reply_by_raw.is_some(),
            "reply_by 는 파싱값과 표기 원본을 쌍으로 나른다(SendMeta)"
        );
        debug_assert!(
            meta.to_attr.is_none(),
            "봉투 to 는 전 수신자 수용 판정 뒤 handle_send 가 1회 확정한다(spec §1)"
        );
        debug_assert!(
            !to.is_empty(),
            "빈 수신자 목록은 입구가 INVALID_SEND_ARGS 로 반려한다(spec §6)"
        );

        // 1) One decision-source snapshot (outside the lock).
        let sources = self.port.addressing_sources();
        let mut closing = ReplyClosing {
            in_reply_to: meta.reply_to.as_deref(),
            replier_id: from.peer_id,
            done: None,
        };

        // 2) Address resolution (pure).
        let addressing = resolve_addressing(&self.groups, to, from, &sources)?;
        // The total delivery records this send will leave = the recipient count **including failure rows** (spec §5 "failed recipients stay in the ledger too").
        //   The lookup's truncation decision (`may_be_truncated`) compares this value with the remaining row count, so every row carries the same value.
        let expected_rows = u16::try_from(addressing.recipients.len()).unwrap_or(u16::MAX);
        let roster_names = live_names_hint(&sources.roster);

        let now = Instant::now();
        let mut plans: Vec<RecipientPlan> = Vec::with_capacity(addressing.recipients.len());
        let mut park_effects = ParkSideEffects::default();
        let mut retirements = RetirementLog::default();
        let mut duplicate_contracts: Vec<String> = Vec::new();

        {
            let mut st = self.state.lock().expect("messaging state poisoned");
            // ★The id collision check is the **first side-effect point** (the zero-side-effect bounce contract — `SendReject::IdCollision`)★.
            //   ★The remaining window: "the same fresh id drawn concurrently" — the **decision that rejected full reservation**★ This check catches only
            //   collisions with ids already in the ledger. A window remains where two sends draw the same value at once and both pass, but
            //   ① probability = an identical value within a microsecond window in a 36^8 (2.8×10^12) space ② damage = history record aliasing
            //   (delivery/contract keys are `(msg_id, 수신자)` (msg_id, recipient), so with different recipients they do not overwrite each other) ③ the probability of a bug in
            //   reservation lifecycle wiring is far larger than that. So only the check is done, with no reservation.
            if st.ledger.msg_id_in_use(msg_id) {
                return Err(SendReject::IdCollision);
            }

            // ── pass A: per-recipient acceptance decision ────────────────
            for r in &addressing.recipients {
                // 2-a) Entrance 3 branches (spec §5 · ADR-0116 decision 1). ★Asleep = accepted★ — there is no doorbell (there is
                //      no live session — the re-appearance flush picks it up).
                let (recipient_id, live) = match admission(r, &roster_names) {
                    Ok(admitted) => admitted,
                    Err((code, hint)) => {
                        // ★A failed recipient stays in the ledger too (spec §5)★ — it is a terminal row, so it moves no further.
                        st.ledger.record_with_expected(
                            msg_id,
                            sender_name,
                            from.peer_id,
                            r.name(),
                            None,
                            body,
                            DeliveryStatus::Failed,
                            now,
                            expected_rows,
                        );
                        plans.push(RecipientPlan::Failed { code, hint });
                        continue;
                    }
                };

                // 2-b) ★Parking opens a contract too — asleep included (spec §3 item 2 · ADR-0116 decision 1)★: it is a message that will be injected
                //      someday, so the reply obligation holds, and so the deadline sweep fires normally as well.
                let mut contract: PendingContract = None;
                if meta.request {
                    let reply_by = meta.reply_by.zip(meta.reply_by_raw.clone());
                    match st.ledger.open_request(
                        msg_id,
                        sender_name,
                        from.peer_id,
                        r.name(),
                        recipient_id,
                        reply_by,
                        now,
                    ) {
                        OpenOutcome::Opened => {
                            contract =
                                Some(open_reservation(&mut st, self, msg_id, recipient_id, None))
                        }
                        // Under cap pressure the oldest retirable contract was given a **retirement-pending mark** (mark-and-sweep,
                        //   ADR-0108). The actual removal happens in the commit below — if this recipient ends in failure, the mark is reverted.
                        OpenOutcome::OpenedAfterMarking(rc) => {
                            contract = Some(open_reservation(
                                &mut st,
                                self,
                                msg_id,
                                recipient_id,
                                Some(rc),
                            ))
                        }
                        // Recipients were already de-duplicated in the resolution step, so a duplicate `(id, 수신자)` (id, recipient) is unreachable.
                        OpenOutcome::DuplicateId => duplicate_contracts.push(r.name().to_string()),
                        OpenOutcome::Full => {
                            st.ledger.record_with_expected(
                                msg_id,
                                sender_name,
                                from.peer_id,
                                r.name(),
                                Some(recipient_id),
                                body,
                                DeliveryStatus::Failed,
                                now,
                                expected_rows,
                            );
                            plans.push(RecipientPlan::Failed {
                                code: FailCode::RequestCapacity,
                                hint: format!(
                                    "Too many replies are still outstanding, so the daemon could not open a reply contract for '{}' — it was NOT delivered. Send it as a plain notification, or wait for earlier requests to be answered or to time out.",
                                    r.display
                                ),
                            });
                            continue;
                        }
                    }
                }

                // 2-c) ★Enqueue is unconditional (ADR-0125 · spec §5 — load-bearing)★. The entrance does **not ask**
                //      "can it go in now": it checks neither busy (the gate) nor what is ahead (the old joining decision) and puts it on the queue
                //      tail. Putting a gate back here revives the branch that skips enqueue = direct send, and
                //      the order comes loose from the enqueue order (at that moment seat reservation and the joining decision become necessary again = a
                //      signal of running the overhaul backwards).
                //      ★The only axis deciding whether to enqueue = the mailbox limit★ — so **every send passes the cap
                //      gate** (the 5th-round contract where direct send bypassed the cap is abolished: `MAILBOX_FULL` increasing
                //      is a design consequence, not a regression — spec §5).
                if st.mailbox.can_admit(recipient_id, ParkKind::Message) {
                    plans.push(RecipientPlan::Park {
                        recipient_id,
                        contract,
                        live,
                    });
                    continue;
                }
                // Revert the contract just opened (blocks a ghost timeout where a request that was never delivered fires an overdue notice).
                if let Some(res) = contract {
                    res.rollback(&mut st);
                }
                st.ledger.record_with_expected(
                    msg_id,
                    sender_name,
                    from.peer_id,
                    r.name(),
                    Some(recipient_id),
                    body,
                    DeliveryStatus::Failed,
                    now,
                    expected_rows,
                );
                plans.push(RecipientPlan::Failed {
                    code: FailCode::MailboxFull,
                    hint: mailbox_full_hint(&r.display),
                });
            }

            // ── Envelope `to` freeze (spec §1) ─────────────────────
            let admitted: Vec<bool> = plans
                .iter()
                .map(|p| !matches!(p, RecipientPlan::Failed { .. }))
                .collect();
            let admitted_count = admitted.iter().filter(|ok| **ok).count();
            // ★The freeze must come **before injection** (M7 — decision fixed, review blind r2 #3 ACCEPTED)★: the envelope is
            //   assembled at injection time, and its material `to` is fixed here.
            //   ★Do not "fix" it by "wrapping after the outcome"★ — that would wreck the `delivered` accounting of the drain below
            //   wholesale (the responses all get flattened to pending).
            let fanout_meta = SendMeta {
                to_attr: (admitted_count >= 2).then(|| build_to_attr(&addressing, &admitted)),
                ..meta.clone()
            };

            // ── pass B: the actual enqueue ─────────────────
            let park_targets: Vec<usize> = plans
                .iter()
                .enumerate()
                .filter_map(|(i, p)| matches!(p, RecipientPlan::Park { .. }).then_some(i))
                .collect();
            for i in park_targets {
                let (recipient_id, contract) = match &mut plans[i] {
                    RecipientPlan::Park {
                        recipient_id,
                        contract,
                        ..
                    } => (*recipient_id, contract.take()),
                    _ => unreachable!("park_targets 는 Park 만 담는다"),
                };
                let r = &addressing.recipients[i];
                match park_into(
                    &mut st,
                    ParkRequest {
                        msg_id,
                        sender_name,
                        from,
                        entrance,
                        recipient: recipient_id,
                        recipient_name: r.name(),
                        body,
                        kind: ParkKind::Message,
                        meta: &fanout_meta,
                        expected_rows,
                    },
                    now,
                    &mut park_effects,
                ) {
                    Ok(()) => {
                        if let Some(res) = contract {
                            res.commit(&mut st, &mut retirements);
                        }
                        closing.close_in_lock(&mut st);
                    }
                    // ★Unreachable path★: `can_admit` passed just above, and the same lock was not released in between.
                    //   Even so it is not swallowed silently — it means the store accounting went off, so it degrades to a failure row.
                    Err(ParkError::MailboxFull) => {
                        if let Some(res) = contract {
                            res.rollback(&mut st);
                        }
                        st.ledger.record_with_expected(
                            msg_id,
                            sender_name,
                            from.peer_id,
                            r.name(),
                            Some(recipient_id),
                            body,
                            DeliveryStatus::Failed,
                            now,
                            expected_rows,
                        );
                        plans[i] = RecipientPlan::Failed {
                            code: FailCode::MailboxFull,
                            hint: mailbox_full_hint(&r.display),
                        };
                    }
                }
            }
            // Moving the hook **outside** this critical section would itself create a window and make the test meaningless.
            #[cfg(any(test, feature = "test-harness"))]
            self.fire_accept_hook(&st.ledger);
            drop(st);

            park_effects.log("send");
            for name in &duplicate_contracts {
                tracing::error!(
                    msg_id = %msg_id,
                    recipient = %name,
                    "회신 계약 키 (msg_id, 수신자) 중복 — 수신자 중복 제거가 어긋났다는 배선 결함 신호(ADR-0111 결정 5)"
                );
            }

            // 3) Synchronous drain (outside the lock — ADR-0125 decision 1).
            let mut deferred_inline: Vec<PeerId> = Vec::new();
            let mut results: Vec<RecipientResult> = Vec::with_capacity(plans.len());
            for (i, plan) in plans.into_iter().enumerate() {
                let display = addressing.recipients[i].display.clone();
                let result = match plan {
                    RecipientPlan::Failed { code, hint } => RecipientResult {
                        to: display,
                        status: SendStatus::Failed,
                        code: Some(code),
                        hint: Some(hint),
                    },
                    RecipientPlan::Park { live: false, .. } => RecipientResult {
                        to: display.clone(),
                        status: SendStatus::Pending,
                        code: None,
                        hint: Some(park_hint_dormant(&display)),
                    },
                    RecipientPlan::Park {
                        recipient_id,
                        live: true,
                        ..
                    } => {
                        let report = self.drain_queue(recipient_id, Some(&sources.roster));
                        if report.injected.iter().any(|id| id == msg_id) {
                            RecipientResult {
                                to: display,
                                status: SendStatus::Delivered,
                                code: None,
                                hint: None,
                            }
                        } else {
                            // ★A share that could not go out must keep an occasion to be opened (spec §5 "no isolation")★ — but the condition for pressing
                            //   the doorbell is narrow: ① in the backed-off case (㉯), **the winner echoes it back while settling**
                            //   (`deferred_flush` — pressing again here only shuttles the marker back and forth) ② pressing **because of one's own injection
                            //   failure** is forbidden (repeated re-injection to a recipient that became unreachable — spec §5). Only the remaining
                            //   branches (caught by the gate · the target is not in the roster at drain time) press it.
                            if !report.retreated && report.caller_inject_error.is_none() {
                                self.ring_or_defer(recipient_id, &mut deferred_inline);
                            }
                            RecipientResult {
                                to: display.clone(),
                                status: SendStatus::Pending,
                                code: None,
                                hint: Some(pending_hint(&display, &report)),
                            }
                        }
                    }
                };
                results.push(result);
            }

            // ★Retirement metering is recorded **after the outcome loop** (M1)★: the commit moved down into pass B and the injection loop
            //   (A2), so recording it **before** the loop would make the retirement on the main path (a request to an idle recipient) **leave no log at all**
            //   — and under ADR-0108 decision 2 that info log is the **only** evidence of retirement.
            log_contract_retirements(msg_id, &retirements.real);
            // ★A retirement that was planned but did not happen (R2)★ — not counted as a retirement, left as an anomaly (no metering pollution).
            for r in &retirements.phantom {
                tracing::warn!(
                    planned_msg_id = %r.request_id,
                    to = %r.recipient,
                    new_msg_id = %msg_id,
                    "은퇴 예정 표시했던 계약이 커밋 시점에 이미 없었다 — 은퇴는 일어나지 않았다(상한 압력이 한 번 덜 풀렸다)"
                );
            }

            // 4) The inline drain of the doorbell-unwired assembly (the `ring_or_defer` division of labor).
            //    ★This is not a direct send either (spec §5 exception item)★: the sending thread runs the **shared drain routine**
            //    on the spot (it is also the means of determinism for kernel unit tests). Misreading it as "sender-thread injection" and
            //    ripping it out makes the harness nondeterministic and cuts the normal path along with it.
            for id in deferred_inline {
                self.flush_for_agent(id);
            }

            // 5) Reply contract handling (spec §3 item 7-④ · ADR-0116 decision 2 · ADR-0118).
            //
            // ★Why these three branches (load-bearing — avoids the ledger lying in both directions at once)★:
            //   - The old implementation (unconditional close) **claimed a message that never arrived as `replied`**. If a worker replied after the requester
            //     died, the message went to no one, yet the contract closed, the worker's `reply_owed_by_me`
            //     vanished and the deadline notice never went out.
            //   - The overhaul-era provisional plan (A1 — keep open regardless of reason) did the opposite and **left a contract with nowhere to go neglected forever**.
            //   - The ratified rule separates the two: **parking is also acceptance** ("plugging in completes the contract" — once replies
            //     pile up for an asleep requester, the replier's obligation was fulfilled at that point. Even if the requester is deleted afterwards and that parked item
            //     is cleared as `RECIPIENT_DELETED`, **the contract stays `replied`** — it is not reverted: the contract axis
            //     records the fact for "did they answer", the delivery axis for "did it arrive", spec §6 axis distinction). Only when the requester name is
            //     **nowhere in the world** (`RECIPIENT_NOT_FOUND`) is there nowhere to go, so it closes as a failure. Mailbox full ·
            //     duplicate name · unreachable are **the environment of that moment**, so a retry may succeed and it is a no-op (closing would leave the ledger
            //     saying "reply failed" forever even if the retry actually delivered — the opposite direction of lying).
            // ★Codes that cannot occur on the reply path★: `REQUEST_CAPACITY` (a reply opens no contract because `reply_to`↔`request` are
            //   mutually exclusive) · `RECIPIENT_DELETED` (a lookup-time code, not a send response) — no branch is placed for them.
            // ADR-0116 (decision 2) / ADR-0118 (decisions 1·2·3)
            if let Some(in_reply_to) = &meta.reply_to {
                match (closing.done, reply_disposition(&results)) {
                    // Accepted → already closed in-lock. Only the result is logged outside the lock.
                    (Some(outcome), _) => log_reply_close(outcome, in_reply_to, sender_name),
                    (None, ReplyDisposition::RequesterGone) => {
                        self.fail_reply_contract(in_reply_to, sender_name, from.peer_id)
                    }
                    (None, ReplyDisposition::NoOp) => tracing::debug!(
                        in_reply_to,
                        "회신이 일시 사유(보관함 가득·동명)로 실패 — 계약은 **오픈 유지**(무동작). 재시도가 정상 경로로 닫는다(spec §3 항목 7-④)"
                    ),
                    // ★An accepted row exists but the in-lock close did not happen = a wiring defect★: every accept branch goes through
                    //   `close_in_lock`, so it is unreachable. It is not passed over silently but recorded (the outcome would be a deadline
                    //   notice going out with the contract unclosed, so observation is needed).
                    (None, ReplyDisposition::Accepted) => tracing::error!(
                        in_reply_to,
                        replier = %sender_name,
                        "회신이 수용됐는데 계약 닫기가 임계구역에서 일어나지 않았다 — 수용 갈래 배선 누락(리뷰 fix D2)"
                    ),
                }
            }
            Ok(results)
        }
    }

    /// ★Reply contract close★ — strict matching + **contract selection on the replier basis** (ADR-0111 decision 5). Depending on the result only **logging**
    /// differs (no effect on delivery or the response — spec §3 item 7-②).
    ///
    /// ★No new error is created for a match failure (`NoMatch`/`AlreadyClosed`)★: it is a state that cannot be fixed, and bouncing would
    ///   trigger a retry for an already-delivered message and cause a duplicate.
    /// ★The production send path does not use this verb — **it is test-seam only** (review fix D2)★. Production closes **in the very lock
    ///   span that recorded the acceptance** with `ReplyClosing::close_in_lock` (that struct header is the canonical rationale): split
    ///   across two locks, a delete cleanup in between would close the contract as `reply_failed`, giving "accepted but reply failed".
    ///   So this verb is blocked by cfg from **compiling in a production build** — left in, the next session would pick the "simpler
    ///   one" and revive that race (it uses only the legacy parking seam — hence `cfg(test)` alone:
    ///   a build with only the `test-harness` feature on has no caller).
    #[cfg(test)]
    fn close_reply_contract(&self, in_reply_to: &str, replier_name: &str, replier_id: PeerId) {
        let outcome = {
            let mut st = self.state.lock().expect("messaging state poisoned");
            st.ledger
                .close_on_reply(in_reply_to, replier_id, Instant::now())
        };
        log_reply_close(outcome, in_reply_to, replier_name);
    }

    /// ★Reply contract **failure close** (spec §3 item 7-④ · ADR-0116 decision 2 · ADR-0118 decision 1)★ — called only when the reply send row was
    /// `RECIPIENT_NOT_FOUND` (the requester name is in neither the roster nor the profiles = nowhere to go).
    ///
    /// ★What this verb does = closing the contract axis only★: the delivery axis was already recorded as a failure row (the outcome of the reply message itself),
    ///   and the original request's delivery record stays `delivered` (spec §6 axis distinction — a delivery state transition like
    ///   `delivered → reply_failed` does not exist).
    /// ★Unlike the accept branch, **splitting the lock is safe** (review fix D2)★: this path has no acceptance (every row failed) — even if a delete
    ///   cleanup in between closes the same contract as `reply_failed`, the outcome is the same, so no revert applies.
    /// `replier_name` is for logging only — the contract is chosen only by `replier_id` (ADR-0309).
    // ADR-0116 (decision 2) / ADR-0118 (decisions 1·3)
    fn fail_reply_contract(&self, in_reply_to: &str, replier_name: &str, replier_id: PeerId) {
        let outcome = {
            let mut st = self.state.lock().expect("messaging state poisoned");
            st.ledger
                .fail_on_undeliverable_reply(in_reply_to, replier_id)
        };
        match outcome {
            ReplyFailOutcome::Failed => tracing::info!(
                in_reply_to,
                replier = %replier_name,
                "회신이 도달 불가 확정(요청자 이름이 어디에도 없음) — 계약을 reply_failed 로 실패 종결(오픈 목록·기한 스윕·512 계수에서 제거, 이력 잔존 — ADR-0116 결정 2)"
            ),
            ReplyFailOutcome::NoMatch => tracing::debug!(
                in_reply_to,
                replier = %replier_name,
                "reply_to 가 이 회신자의 오픈 계약을 가리키지 않음 — 종결할 계약 없음(정상 경로, spec §3 항목 7-②)"
            ),
            ReplyFailOutcome::AlreadyClosed => tracing::debug!(
                in_reply_to,
                replier = %replier_name,
                "이미 종결된 계약 — 되돌리지 않는다(no-op, spec §3 항목 7-③/7-④)"
            ),
            ReplyFailOutcome::GuardHeld => tracing::debug!(
                in_reply_to,
                replier = %replier_name,
                "잠정·은퇴 예정 표시 계약이라 실패 종결을 적용하지 않음 — 수명은 그 가드 소유(ADR-0118 결정 3, 무동작)"
            ),
        }
    }

    /// ★Bulk cleanup on profile deletion (spec §5 · ADR-0116 decision 3 · ADR-0309 decision 6)★ — called by the host (daemon)
    ///   **right after** it deletes a profile.
    ///
    /// ★Trigger condition = deleted ∧ **no live session for that profile id** (user decision 2026-07-30 · review fix D1 —
    ///   load-bearing)★: the profile-delete command does not kill a running session, so a state exists where "only the tree entry was deleted and the process
    ///   is still alive". That peer is a **live recipient that can receive as soon as it goes idle**, so it is not a cleanup target —
    ///   cleaning up would kill mail that would be delivered and mark a contract that would hold as a failure (the exact opposite of this revision's purpose). When that session actually
    ///   disappears the profile is gone too, so it naturally converges to the "absent" class.
    /// ★The gate is on the id axis (`is_agent_live(profile_id)`) — rename immunity★: this function is called **after the profile is deleted**, and
    ///   without the profile the live session's canonical name **changes** from `display_name` to `basename(session.cwd)`.
    ///   So looking up the post-deletion list with a name taken before deletion never matches and the cleanup **always** fires —
    ///   an ordinary path reproduced by a single `RenameProfile`. The predicate is **liveness** (`Running|Exiting`), not the recipient list
    ///   (`is_agent_live` doc).
    /// ★The cleanup also sweeps by id — so there is no name-axis gate (ADR-0309 decision 6)★: targets ① (parked items addressed to that id) and
    ///   ② (open contracts where that id is the requester) are both found by id. The share of a **different** agent with the same name (a live duplicate · an agent that
    ///   newly took the old name) has a different key and is not touched, and the share accumulated under a pre-rename name is caught without omission. Formerly
    ///   contracts were deleted by name and 「그 이름을 쥔 산 세션이 있나」 (is there a live session holding that name) was asked as a second gate (review fix N1). ★Do not
    ///   revive that gate★: if a live duplicate exists, even the deleted profile's share is skipped wholesale and left to drain away via TTL.
    /// ★Two things it does★: ① close **all parked items** addressed to that profile id as `failed` + `RECIPIENT_DELETED` (ledger terminal +
    ///   lookup hint — it disappears only from the queue and the history stays, so the sender sees the reason via `eg_messages{id}`. The "never
    ///   silently discard live mail" invariant is kept) ② close open contracts where that id is the **requester** as `reply_failed` (reply
    ///   unreachability confirmed). A contract already `replied` is not reverted, and **a contract whose replier side was deleted is kept**
    ///   (the existing path stays alive where the sender learns of the non-response via the deadline notice).
    /// ★Deletion time is **one-shot** — no re-evaluation trigger is placed (stated honestly · spec §5)★: the trigger condition is evaluated only at
    ///   the moment of deletion. So ⓐ if a deletion slips in between the decision and the parking insert, that item misses this scan, and ⓑ if a live session
    ///   whose profile alone was deleted dies later, there is no event to collect it — **the outcome of that residue is TTL 24h `expired`**
    ///   and not `RECIPIENT_DELETED` (a known and accepted property). A symmetric trigger and an insert-time tombstone are **not adopted**
    ///   (an alternative ADR-0116 rejected) — reviving them needs a new decision first.
    /// ★Caller duty (ADR-0006)★: call it **after releasing the profile registry lock** (taking the messaging lock while holding the registry lock
    ///   inverts the lock order — spec §5). `name` is **for logging only** — neither the decision nor the cleanup looks at this value.
    // ADR-0116 (decision 3) / ADR-0118 (decision 3 — guard first) / review fix D1 (id axis) / ADR-0309 (decision 6)
    pub fn handle_profile_deleted(&self, profile_id: PeerId, name: &str) -> ProfileDeletedOutcome {
        // The gate = the id axis alone (doc above).
        if self.port.is_agent_live(profile_id) {
            tracing::debug!(
                deleted = %name,
                profile = %profile_id,
                "프로필 삭제 정리 건너뜀 — 그 프로필의 세션이 아직 살아 있다(트리 항목만 지운 상태). 파킹분·계약은 그대로 두고 idle 전이 시 정상 배달된다(spec §5)"
            );
            return ProfileDeletedOutcome {
                skipped_live: true,
                ..Default::default()
            };
        }

        let now = Instant::now();
        let mut evicted: Vec<EvictedTransition> = Vec::new();
        let mut illegal = 0usize;
        let (purged, contracts) = {
            let mut st = self.state.lock().expect("messaging state poisoned");
            // ADR-0309 (decision 6 — queue · delivery records · contracts are all swept by profile id)
            let purged = st.mailbox.purge_recipient(profile_id);
            for m in &purged {
                // ★`pending → failed` (an edge limited to delete cleanup — spec §6)★: a dedicated verb, so a generic transition cannot make it.
                match st.ledger.fail_pending(
                    &m.msg_id,
                    profile_id,
                    FailCode::RecipientDeleted.as_str(),
                    now,
                ) {
                    Ok(()) => {}
                    Err(TransitionError::NotFound) => evicted.push(EvictedTransition {
                        msg_id: m.msg_id.clone(),
                        recipient: profile_id,
                        intended: DeliveryStatus::Failed,
                    }),
                    Err(TransitionError::Illegal { .. }) => illegal += 1,
                }
            }
            let contracts = st.ledger.fail_open_requests_from(profile_id);
            (purged, contracts)
        };

        log_evicted_transitions(&evicted);
        if illegal > 0 {
            tracing::error!(
                deleted = %name,
                count = illegal,
                "삭제 정리: 큐에 있던 항목의 장부 상태가 pending 이 아니었다 — 큐↔장부 회계 어긋남(배선 결함 신호)"
            );
        }
        if !purged.is_empty() || !contracts.failed.is_empty() {
            tracing::info!(
                deleted = %name,
                parked_failed = purged.len(),
                contracts_failed = contracts.failed.len(),
                guard_held = contracts.guard_held,
                "프로필 삭제 정리 — 파킹분을 RECIPIENT_DELETED 로 종결하고 그 프로필이 요청자인 오픈 계약을 reply_failed 로 종결(ADR-0116 결정 3 · ADR-0309 결정 6)"
            );
        }
        if contracts.guard_held > 0 {
            tracing::debug!(
                deleted = %name,
                held = contracts.guard_held,
                "삭제 정리: 잠정·은퇴 예정 표시 계약은 건드리지 않았다(수명 = 가드 소유, ADR-0118 결정 3). 재발화 없음 — 잔여는 TTL 소관(spec §5)"
            );
        }
        ProfileDeletedOutcome {
            skipped_live: false,
            failed_parked: purged.len(),
            failed_contracts: contracts.failed.len(),
            guard_held_contracts: contracts.guard_held,
        }
    }

    /// ★Division of labor by doorbell firing time★ — if wired, enqueue **immediately** (non-blocking); if unwired, put it in `deferred` and
    ///   have the caller run it inline **after the response is finalized**.
    ///
    /// ★Why split★: the two branches have opposite cost structures. The wired branch is a channel send, so calling it immediately is always
    ///   right (deferring pushes that recipient's wake-up behind the remaining blocking writes, and a panic loses it entirely),
    ///   while the unwired branch **runs the batch write as is on this thread**, so calling it in the middle of the injection loop
    ///   creates flush re-entry + accounting skew (response `pending` vs ledger `delivered`).
    fn ring_or_defer(&self, id: PeerId, deferred: &mut Vec<PeerId>) {
        match &self.trigger {
            Some(t) => t.request_flush(id),
            None => deferred.push(id),
        }
    }

    /// ★The inline fallback must also be called outside the messaging lock★ — every call site is a point after park returns (lock released).
    fn request_flush(&self, id: PeerId) {
        match &self.trigger {
            Some(t) => t.request_flush(id),
            None => self.flush_for_agent(id),
        }
    }

    /// ★`<notice>` parking + recording ledger `pending`★ — a wrapper for daemon notices only (the send path uses `park_into` directly).
    ///
    /// ★A notice is never bounced (mailbox `park`)★: `ParkKind::Notice` is accounted in **its own lane**
    ///   (`mailbox::NOTICE_CAP` = 64) and, when it overflows, merely reclaims the oldest notice. So this function has
    ///   no return value and the call site need not look at a result (a bounce would have been a place for silent loss).
    /// `recipient_name` is the send-time name snapshot of the ledger row (the queue key is `recipient`).
    fn park_notice(&self, msg_id: &str, recipient: PeerId, recipient_name: &str, body: &str) {
        let now = Instant::now();
        let mut effects = ParkSideEffects::default();
        {
            let mut st = self.state.lock().expect("messaging state poisoned");
            let _ = park_into(
                &mut st,
                ParkRequest {
                    msg_id,
                    sender_name: NOTICE_SENDER_LABEL,
                    from: daemon_identity(),
                    entrance: Entrance::Daemon,
                    recipient,
                    recipient_name,
                    body,
                    kind: ParkKind::Notice,
                    meta: &SendMeta::default(),
                    // A notice has exactly 1 delivery record row.
                    expected_rows: 1,
                },
                now,
                &mut effects,
            );
        }
        effects.log(recipient_name);
    }

    /// ★A thin entrance for async occasions (appearance/epoch/idle/doorbell — ADR-0104)★: called by the daemon-side flush lane and roster diff.
    ///   **It calls the shared drain routine as is and discards only the report** — those occasions have no sender to answer.
    ///   `recipient` = that agent id = the queue key (ADR-0309).
    pub fn flush_for(&self, recipient: PeerId) {
        let _ = self.drain_queue(recipient, None);
        #[cfg(any(test, feature = "test-harness"))]
        {
            *self
                .async_drains_done
                .lock()
                .expect("async drain counter poisoned")
                .entry(recipient)
                .or_default() += 1;
        }
    }

    /// ★One injection code — this function is that one (invariant · ADR-0125 §impact · load-bearing)★: injects the parked items
    ///   addressed to a recipient id **in a bulk, oldest first**.
    ///   ★Making a send-only injection code separately violates this decision★ — that is the "two paths" defect 0124/0125 removed, and
    ///   the moment it revives, ordering devices like seat reservation and the joining decision become necessary again.
    ///
    /// ★Argument `snapshot`★: `Some` = the caller reuses the roster it just took (synchronous drain — the one-snapshot-per-send
    ///   invariant, ADR-0111 decision 2). `None` = take it here (async occasion — the recipient may have died or reappeared in the meantime,
    ///   so look again at the current roster).
    ///
    /// ★"Oldest first" is a guarantee **inside this batch** (finding 8)★ — re-parking is a sequence-number merge, so the older item
    ///   takes priority even when carried over across batches. The device that establishes the order one recipient sees as the **enqueue order** is step 0 below.
    ///
    /// ★In-flight accounting — plugs the hole where the cap slid every cycle (F1 · load-bearing)★: at the `take_in_flight` registration
    ///   point (just before releasing the lock), the batch leaving the lock is not in the queue but is **still an outstanding message addressed to that recipient**.
    ///   Leaving it out of the denominator lets the queue grow
    ///   without bound on the next interleaving — ① drain empties the queue ② during the out-of-lock inject, k concurrent sends see an "empty queue" and pass the cap check
    ///   ③ the inject fails and N batch items are restored → queue = N + k, and the next cycle's batch is also N + k, so **+k
    ///   every cycle**. So the outgoing batch stays in the denominator via `take_in_flight`, and is settled by that amount at each terminal (delivery/restore).
    ///   A missed settlement is covered by `FlightSettle` (Drop). The observable cost: **while flush is running, a
    ///   new send to that recipient can be bounced as `MAILBOX_FULL`** (the queue looks empty but the denominator is full) — the same choice as the spec §5
    ///   stance that chose a visible bounce over silent growth.
    ///
    /// ★★Drains for the same recipient must not run overlapped — do not remove (round-7 · **the premise of ADR-0125**)★★: while batch A is
    ///   injecting outside the lock, if batch B drains the queue again and injects, **B reaches the recipient before A's
    ///   remaining items** (an order inversion). So step 0 looks at in-flight and backs off.
    ///   ★In round 7 this guard matters **more**★: since the drain runs inside the send call, **two sending threads draining the same
    ///   recipient at once became a routine case**. Reading it as "a surplus replaceable by in-flight accounting" and
    ///   ripping it out tangles the batches and immediately breaks "delivery order = enqueue order".
    ///   ★In the production wiring too, two drains **exist at the same time** — do not read this branch as a "line of defense never reached"★:
    ///   the synchronous drain on the send path (the "round 7" paragraph above) · the doorbell-unwired fallback (experiment bin · unit tests — `request_flush`
    ///   runs inline on the calling thread) · and the assembly where the host's delivery lane runs different recipients in parallel —
    ///   all three are real paths.
    ///
    /// ★★Exclusion has two axes (do not remove)★★
    ///   - **Queue axis (step 0 · `mailbox` in-flight)** — key = the park queue (recipient id). It is the axis that blocks the order inversion above, and
    ///     **the cap denominator accounting rides on it** (the in-flight paragraph above). So it bears not only correctness but also
    ///     inflow control.
    ///   - **Target axis (`in_flight_targets` — ADR-0142)** — key = the session id being injected into. The target is always that queue id, so it looks at the same thing as the queue axis, and
    ///     catches separately only in the gap where the queue axis is empty (from the batch's last settlement to the target release). The occasion that set up this axis
    ///     is gone — the reason it was kept is in the `in_flight_targets` comment.
    ///   ★The backed-off side's letter is not lost — common to both axes★: it rides out in the winner's batch, and if it could not go out, **the one holding
    ///     the receipt** presses the doorbell again while finishing settlement (just backing off is a lost wakeup — it is stuck until the next idle
    ///     notice/appearance. The markers = `deferred_flush` (queue axis) · `deferred_by_target` (target axis),
    ///     and the pressing verb is one).
    ///   ★So the guard and its echo must **both** stay alive★: remove the guard and the batches tangle, immediately breaking "delivery
    ///     order = enqueue order"; remove the echo and the backed-off side's letter is stuck until the next appearance or turn-end
    ///     notice.
    ///
    /// ★Recipient = **that one id** in the current roster (ADR-0309 decisions 1 · 5 — load-bearing)★: the queue key is the agent that
    ///   receives. The async occasion's snapshot is from enqueue time and may be stale, so just before drain **look that id up again in the current roster**. If absent
    ///   (death · asleep), skip — parking stays, and it remains in the queue until the same id comes up again (a new incarnation) or it expires by TTL.
    ///   ★Do not look it up by name instead★ — a **different** agent that newly took that name after a rename · deletion
    ///   would receive someone else's mail.
    ///
    /// ★The gate is looked at **exactly once per drain** (C2 review fix 4 · load-bearing)★:
    ///   - **One check before the first injection**: if that recipient is in a turn now, return the batch in its original order (parking stays — that
    ///     turn's MessageDone fires the trigger again). It is the device that blocks "injection mid-turn" in cases such as an appearance flush
    ///     landing in the middle of a priming turn.
    ///   - **No mid-batch recheck (intentional — invariant within a batch)**: the batch's first injection immediately triggers the recipient-side "input-time
    ///     user echo" (claude emits this as `Structured` = it is observed as turn progress), so looking at the gate
    ///     per item **stops the batch after 1 item** (= dribble injection = the ADR-0104 rejected alternative). Once a batch has started,
    ///     push it through to the end.
    ///   The premise that makes the gate safe is that observation starts **only from live output** (a resumed transcript is
    ///   not observed — agent `OutputCore::seed`). busy = an actual turn in progress now, so its end notice must come
    ///   (no unwakeable busy caused by past records). The abnormal turn whose notice
    ///   is lost is woken fail-open by the `BUSY_MAX_TURN` cap sweep (busy.rs).
    /// ★Undelivered items **never leave the queue** (lock atomicity — load-bearing)★: drain · gate · restoring the skipped items are finished in
    ///   **one lock span**, and only **the items to deliver** are carried out of the lock. It used to release the lock after drain,
    ///   look at the gate, then take the lock again to restore, but in between a **window where the queue looks empty** appeared, the old mail to be returned
    ///   vanished from observation, and the decision of a concurrent send wobbled.
    /// ★Restore order (round-4 finding 1)★: new items may enter the queue while returning, so they are returned via `restore_ordered`
    ///   (admission sequence merge) — the queue always keeps the global oldest-first order (the old FRONT insertion put the second
    ///   call ahead of the first and flipped the age order — that inversion also spread into the sweep hiding expired items).
    /// ★Accepted residual★: if the recipient **starts a new turn on its own** mid-batch, the remaining injections go into the CLI's internal stdin
    ///   queue (nothing lost, only "when it gets read" gets blurred — spec §7 unverified item).
    // ADR-0125 (one injection code — the sync and async occasions share this routine)
    // ADR-0142
    // ADR-0309
    fn drain_queue(&self, recipient: PeerId, snapshot: Option<&[LiveAgent]>) -> DrainReport {
        let owned;
        let roster: &[LiveAgent] = match snapshot {
            Some(r) => r,
            None => {
                owned = self.port.live_agents();
                &owned
            }
        };
        let target = roster.iter().find(|a| a.id == recipient).cloned();
        let mut report = DrainReport::default();

        let now = Instant::now();
        let mut no_target_kept = 0usize;
        let mut busy_skipped: Option<(u32, usize)> = None;
        // The count backed off on the target axis — logged outside the lock (the tracing discipline below).
        let mut target_deferred = 0usize;
        let mut evicted_transitions: Vec<EvictedTransition> = Vec::new();
        // 1~4) Drain + recording expiry in the ledger + gate + immediate restore of undelivered items.
        //
        // ★No tracing while holding the lock — collect, then log outside the lock (finding 3)★: a synchronous-formatting subscriber can
        //   get stuck on the stdout lock, and logging inside the critical section spreads that delay into messaging lock waits.
        // ★Why calling the gate under the lock is not a discipline violation★: `BusyGate` is a **pure lookup + a short lock**, and a seam where
        //   "safe to be called while holding the messaging lock" is nailed down as a contract (busy.rs `BusyGate` comment). The reverse direction
        //   (busy lock → messaging lock) does not exist (the notice does only a non-blocking channel send) → no lock-order inversion.
        //   What is forbidden is **DeliveryPort (inject/roster)** — that is still entirely outside the lock (step 5 below).
        // ★Why a labeled block (`'drained`)★: the evict fact of the expiry transition must be logged **outside the lock** (the discipline above), and
        //   if it did `return` here because there is nothing to deliver, that log would vanish entirely. So the block
        //   exits by value (an empty batch) and logging is done on the common path after the lock is released (the loops below are no-ops on empty input).
        /// The receipt + the batch to deliver (recipient · items). `None` = **deferral** (backed off at step 0 — meaning it touched neither the queue nor the receipt,
        ///   so it must be distinguished from an empty batch (`Some`): an empty batch is "there was nothing to look at", a deferral is "it is not
        ///   its turn to look now").
        type Drained = Option<(FlightTicket, Option<(LiveAgent, Vec<ParkedMessage>)>)>;
        let drained_or_deferred: Drained = 'drained: {
            let mut st = self.state.lock().expect("messaging state poisoned");
            // 0) Block duplicate entry into flush for the same recipient (the doc above "why they must not run overlapped"). Both the decision and the record are in **the same lock
            //    span**, so it does not race with the receipt holder's settlement finish (the same lock): if the holder is first, in-flight is
            //    0 here and it proceeds normally; if here is first, the holder sees this marker.
            if st.mailbox.in_flight_len(recipient) > 0 {
                let waiters = st.deferred_flush.entry(recipient).or_default();
                if !waiters.contains(&recipient) {
                    waiters.push(recipient);
                }
                report.retreated = true;
                break 'drained None;
            }
            let drained = st.mailbox.drain(recipient, now);
            for ex in &drained.expired {
                transition_or_collect_evicted(
                    &mut st.ledger,
                    &ex.msg_id,
                    recipient,
                    DeliveryStatus::Expired,
                    now,
                    &mut evicted_transitions,
                );
            }
            // `drain` yields in queue order (= ascending admission sequence), so the wholesale restore below keeps the
            //   caller contract of `restore_ordered` as is.
            let deliverable = drained.deliverable;
            if deliverable.is_empty() {
                break 'drained Some((FlightTicket::default(), None));
            }
            let Some(target) = target else {
                no_target_kept = deliverable.len();
                st.mailbox.restore_ordered(recipient, deliverable);
                break 'drained Some((FlightTicket::default(), None));
            };

            // ★Target-axis deferral (ADR-0142) — back off if a target is still held on this session★. The reason it is looked at before the gate:
            //   the wake-up of the busy branch hangs on the other side's turn-end notice (+ the cap sweep), whereas the share backed off here is
            //   **always echoed back by the receipt holder while settling** (`deferred_by_target`). The axis with the surer wake-up
            //   is spent first.
            // ★Neither a drop nor forgetting★: the batch is **returned in its original order** and a waiter is registered — if either one is missing,
            //   that letter is stuck until TTL.
            if st.in_flight_targets.contains(&target.id) {
                target_deferred = deliverable.len();
                st.mailbox.restore_ordered(recipient, deliverable);
                report.retreated = true;
                let waiters = st.deferred_by_target.entry(target.id).or_default();
                if !waiters.contains(&recipient) {
                    waiters.push(recipient);
                }
                break 'drained Some((FlightTicket::default(), None));
            }
            if let Some(reason) = self.gate_busy_reason(&target) {
                busy_skipped = Some((target.epoch, deliverable.len()));
                st.mailbox.restore_ordered(recipient, deliverable);
                report.gated = Some(reason);
                break 'drained Some((FlightTicket::default(), None));
            }
            // ★Known residual (review blind #3 — not fixed in this round)★: if the batch that left here **panics** in the out-of-lock inject,
            //   `FlightSettle` (Drop) repays the in-flight accounting but **the items themselves do not
            //   return to the queue** (ownership was on the stack). It is a condition from before v1 and not something the overhaul created — do not
            //   mistake it for a guarded path (the production daemon ends that panic via the hook, so the actual exposure surface is only the hookless test assembly — ADR-0292).
            // ★In-flight registration — **only the share leaving the lock** (F1 · load-bearing)★: the hole in the cap denominator exists only
            //   in the span where this lock is released. The share returned above (busy · no target · target-axis deferral) and the share that hit a terminal (expiry) are outside that
            //   hole, so they are not counted — counting them inflates the denominator without basis and normal inflow is bounced as `MailboxFull`.
            //   The receipt is **always** returned by `FlightSettle` below (a miss = that recipient's lane is blocked permanently).
            let flight = st.mailbox.take_in_flight(recipient, deliverable.iter());
            // Target-axis registration is also in **the same lock span** — if the decision (above) and the registration split, two batches get into the same
            //   session through that gap. The release is by `FlightSettle` alone (a leak = that session blocked permanently).
            st.in_flight_targets.insert(target.id);
            Some((flight, Some((target, deliverable))))
        };
        // Deferral (step 0) — it touched neither the queue nor the receipt, so there is no fact to log, nothing to settle, nothing to echo
        //   (echoing is the receipt holder's job — the tail below). Logging is a single debug line only.
        let Some((flight, batch)) = drained_or_deferred else {
            tracing::debug!(
                recipient = %recipient,
                "드레인 유예: 같은 수신자 앞 배치가 아직 주입 중(in-flight) — 드레인 없이 물러남, 정산 시 재-도어벨(round-7 · ADR-0125)"
            );
            return report;
        };
        // ★Single-exit settlement guard (F1)★ — the delivery loop below has an early break and mixes in out-of-lock external calls (inject).
        //   If settlement is scattered over each branch, missing even one place permanently inflates that recipient lane's denominator so
        //   mail never gets in again. So the remaining receipt is returned by Drop (common to normal exit and unwinding).
        let mut flight = FlightSettle {
            svc: self,
            recipient,
            owed: flight,
            targets: batch.iter().map(|(t, _)| t.id).collect(),
        };

        log_evicted_transitions(&evicted_transitions);
        if target_deferred > 0 {
            tracing::debug!(
                recipient = %recipient,
                parked = target_deferred,
                "드레인 유예(타깃 축): 이 세션의 앞 배치가 아직 타깃을 쥐고 있음 — 배치 되돌림, 정산 시 재-도어벨"
            );
        }
        if no_target_kept > 0 {
            tracing::debug!(
                recipient = %recipient,
                keeping = no_target_kept,
                "flush skip: 그 id 의 에이전트가 지금 로스터에 없다(잠듦·죽음) — 파킹 유지, 같은 id 가 다시 뜨면 등장 flush 가 연다(ADR-0309)"
            );
        }
        if let Some((epoch, parked)) = busy_skipped {
            tracing::debug!(
                recipient = %recipient,
                epoch,
                parked,
                "flush skip: 수신자가 턴 진행 중 — 배치 미시작(파킹 유지, 턴 종료 통지가 재시도)"
            );
        }

        // 5) Delivery (★outside the lock★ — inject is a child stdin blocking write).
        if let Some((target, items)) = &batch {
            for (n, parked) in items.iter().enumerate() {
                let to_id = target.id;
                let payload = ParkPayload::decode(&parked.envelope);
                let wrapped = match parked.kind {
                    ParkKind::Notice => wrap_notice(&payload.body),
                    ParkKind::Message => self.wrap_now(
                        &payload.sender_name,
                        &parked.msg_id,
                        &payload.body,
                        &payload.meta.envelope_fields(&parked.msg_id),
                    ),
                };
                match self.port.inject(to_id, wrapped.as_bytes()) {
                    Ok(outcome) => {
                        // This 1 item hit its terminal — detach it from the in-flight receipt immediately (F1). Holding it until the end of
                        //   the batch would bounce the normal inflow to that recipient wholesale while a long batch runs.
                        let settled = flight.owed.split([parked.kind]);
                        {
                            let mut st = self.state.lock().expect("messaging state poisoned");
                            let _ = st.ledger.transition(
                                &parked.msg_id,
                                recipient,
                                DeliveryStatus::Delivered,
                                Instant::now(),
                            );
                            st.mailbox.settle_in_flight(recipient, settled);
                        }
                        // The observation record's `to_name` = the roster name at the moment of injection (the queue key is an id, so it has no name).
                        self.observe_success(
                            &parked.msg_id,
                            target,
                            payload.from,
                            payload.entrance,
                            &wrapped,
                            &outcome,
                            payload.meta.reply_to.clone(),
                        );
                        report.injected.push(parked.msg_id.clone());
                    }
                    Err(e) => {
                        // ★Lossless on partial failure (load-bearing — finding 1, ADR-0103/0104)★: the recipient died between drain↔inject
                        //   — return the remaining share (including the failed part). Why `restore_ordered` rather than `park`:
                        //     ① cap bypass — if a **concurrent park** between drain↔inject refilled the queue to the cap, park would
                        //        bounce with MailboxFull. Ignoring that error silently loses an admitted message
                        //        (the ledger says pending but it is not in the queue — a ghost pending). restore_ordered does not count
                        //        the cap, so it returns unconditionally (the cap is inflow control, not retention control — mailbox comment).
                        //     ② admission sequence merge — the re-parked part (older) must go ahead of the concurrently parked
                        //        new part (more recent) or "oldest first" breaks (mailbox comment).
                        //   parked_at keeps its original value via clone — no TTL extension (misdelivery defense).
                        //   The ledger is already pending, so no transition is needed (re-parking = pending kept).
                        //   Retry at the next appearance.
                        // ★Cutting the batch here is correct★: this failure means the recipient restarted or became
                        //   unreachable, so (a) the failed part must be returned and left to the next flush and (b) the remaining items must also be returned
                        //   **to go through the gate again on the new incarnation basis** (just pushing on could inject
                        //   in the middle of a new turn). So cutting the batch here is correct, and the reason is left by `e`.
                        let remaining: Vec<ParkedMessage> = items[n..].to_vec();
                        let remaining_count = remaining.len();
                        // ★Restore and settlement must be **paired in one lock span** (the basis of the F1 invariant)★: restore **moves** these
                        //   items from in-flight to the queue, so releasing the lock between the two lets the same item be held in both the queue and in-flight
                        //   at that instant (double counting) and the denominator inflates, while settling first
                        //   reopens the window. The sum (`queue + in_flight`) is invariant thanks to this pairing.
                        let settled = flight.owed.split(remaining.iter().map(|m| m.kind));
                        {
                            let mut st = self.state.lock().expect("messaging state poisoned");
                            st.mailbox.restore_ordered(recipient, remaining);
                            st.mailbox.settle_in_flight(recipient, settled);
                        }
                        // ★This spot is the only place that leaves a send's failure as an observation record (ADR-0088)★.
                        self.observe_failure(
                            &parked.msg_id,
                            target,
                            payload.from,
                            payload.entrance,
                            &wrapped,
                            &e,
                            payload.meta.reply_to.clone(),
                        );
                        tracing::warn!(
                            recipient = %recipient,
                            remaining = remaining_count,
                            "메시지 드레인 중 inject 실패/거부 — 남은 배치 재파킹(무손실 restore_ordered, ADR-0103/0104): {e}"
                        );
                        report.caller_inject_error = Some(e);
                        break;
                    }
                }
            }
        }

        // 6) Echoing the deferred drain — the wake-up of the caller that backed off at step 0 (the `deferred_flush` comment).
        // ★Empty the receipt first★: the echoed flush looks at step 0 again (**right on this thread** in the inline fallback), so
        //    if our receipt remains it gets deferred again on the spot and the wake-up only shuttles as a marker.
        //    This order is the reason it is dropped explicitly rather than left to Drop (on the normal path it is already 0, so a no-op).
        // ★The target axis is also released **before echoing** (same reason)★: echoing while holding it makes the re-entered flush
        //    back off on the target axis again and only the marker shuttles. The id list received while releasing is the key of the waiter lookup below.
        let released_targets = flight.release_targets();
        drop(flight);
        // ★Take the set out **whole inside the lock** (round-8 · load-bearing)★: the echo below, in the inline fallback, invokes this
        //   thread's flush re-entry, and if that flush is deferred again it **newly inserts** an id under the same key. Iterating
        //   while holding the map mixes in that addition, blurring what was pressed, and also risks a re-borrow during re-entry. Taking it out whole
        //   and owning it makes us responsible for "this snapshot" only, and deferrals newly accumulated during iteration are taken by step 6 of **the one holding the receipt
        //   at that time** (the re-entered flush) — responsibility neither overlaps nor leaks.
        // ★Merge the waiters of both axes **into one list** and press them with the same loop★: splitting the wake path in two makes the outcome depend on which one
        //   pressed, and a regression that fixes only one side appears silently. That there are two axes is only about the **key to find the marker**
        //   and the verb that wakes is one (`request_flush`).
        let deferred = {
            let mut st = self.state.lock().expect("messaging state poisoned");
            let mut waiters = st.deferred_flush.remove(&recipient).unwrap_or_default();
            for t in &released_targets {
                for id in st.deferred_by_target.remove(t).unwrap_or_default() {
                    if !waiters.contains(&id) {
                        waiters.push(id);
                    }
                }
            }
            waiters
        };
        // ★Press them all — do not pick (round-8 high)★: a surplus doorbell costs next to nothing thanks to the early return on an empty queue in `flush_for_agent`,
        //   while missing even one binds mail addressed to a live recipient until TTL.
        for id in deferred {
            self.request_flush(id);
        }
        report
    }

    /// ★Turn end (idle transition) · doorbell flush (C2 · ADR-0104 decision 3)★: a flush targeted **by id**. The queue key is the agent
    ///   id (ADR-0309), so there is no need to resolve a name — it opens that one id's queue. So even if the name changes during a turn, the letter
    ///   entrusted in the meantime opens as is.
    /// ★Early return on an empty queue (cost saving — the real cost of surplus-notice idempotency is cut here)★: the idle notice comes
    ///   **every turn** (busy.rs `IdleNotifier` comment — a design that chose surplus over omission). Most of those are
    ///   no-ops with nothing parked, so **before** running the roster snapshot (list_agents = a walk over all sessions), look at the queue under a short lock,
    ///   and if it is empty, exit immediately.
    // ADR-0309
    pub fn flush_for_agent(&self, to_id: PeerId) {
        let has_parked = {
            let st = self.state.lock().expect("messaging state poisoned");
            st.mailbox.len(to_id) > 0
        };
        if has_parked {
            self.flush_for(to_id);
        }
    }

    /// ★TTL sweep + overdue-reply notice (spec §5·§3 — C1/C3)★: the single maintenance entry point called by the periodic task (lib.rs).
    ///   ① collect every recipient's expired parked items and leave them in the ledger as `pending→expired` (the ledger keeps them)
    ///   ② collect overdue unanswered requests and deliver a `<notice>` **to the sender** (spec §3 step 4).
    ///
    /// ★A notice is not a nudge to the recipient★: the notice goes to the **side that made** the request — whether to nudge or give up is
    ///   the sending LLM's call (ADR-0103 decision 3), so the daemon only reports the fact.
    /// ★Exactly once (spec §7)★: `due_timeouts` sets `notified` as it returns, so the same request does not
    ///   come out again in the next sweep — the duty to prevent a double notice lies with the ledger, and this only delivers what was collected.
    /// ★Non-blocking (load-bearing — protects the sweep task)★: this function **does not inject**. A notice goes only as far as parking + doorbell,
    ///   and the actual batch write is done by the flush lane (rationale = the `deliver_notice` comment).
    /// ★A `NotFound` on the expiry transition is not swallowed silently (C4 review fix J)★: a broadcast writes **N** records per send,
    ///   so the history ring (`ledger::HISTORY_CAPACITY`) rotates much faster — by the time a parked item expires that record
    ///   may already have been pushed out, and then the fact "it expired" is left **nowhere**. The transition is still
    ///   best-effort (it does not resurrect the ledger), but that fact is left at debug level so it is observable (logging outside the lock).
    pub fn sweep(&self, now: Instant) {
        // ★F1 guarantee layer — reclaiming **abandoned** contract reservations★: RAII Drop can do nothing when `try_lock`
        //   fails (`Reservation` header), so this point, which owns the lock normally, redoes the same job.
        // ★From round 7 this reclaim is **not reached** on the normal path (ADR-0125 — premise correction)★: the reservation window sits entirely inside the enqueue lock,
        //   so there is no span where the owner holds a reservation with the lock released. So the only debris that gets caught here is of the **forgotten-branch ·
        //   unwinding** class, and this point is the line of defense for it. Do not read it as "it finds nothing when it runs, so delete it" —
        //   removing it reopens ADR-0108, and at that moment the guarantee on the Drop-failure branch of `Reservation` disappears.
        let reclaimed = {
            let mut st = self.state.lock().expect("messaging state poisoned");
            st.ledger.reclaim_abandoned_reservations()
        };
        for (msg_id, recipient_id) in &reclaimed {
            tracing::error!(
                msg_id = %msg_id,
                recipient_id = %recipient_id,
                "정산되지 않은 계약 예약을 sweep 이 회수 — 발송 경로에 커밋/롤백 누락이 있다(훅 없는 시험 빌드면 그 스레드의 패닉일 수도 — F1 보증 계층)"
            );
        }

        let mut evicted_transitions: Vec<EvictedTransition> = Vec::new();
        let due: Vec<DueTimeout> = {
            let mut st = self.state.lock().expect("messaging state poisoned");
            let expired = st.mailbox.sweep_expired(now);
            for ex in expired {
                // ★Target exactly by (msg_id, recipient) (C4 — ADR-0104 anchor resolution)★: the ledger holds 1 msg_id : N delivery
                //   records (group broadcast), so msg_id alone cannot identify which member's delivery it is. The mailbox returns
                //   **its own queue key** with each expired item (`ExpiredParked`), so the transition uses that pair —
                //   the old "reverse lookup of the first pending record" helper could expire the wrong member and was removed.
                transition_or_collect_evicted(
                    &mut st.ledger,
                    &ex.msg.msg_id,
                    ex.recipient,
                    DeliveryStatus::Expired,
                    now,
                    &mut evicted_transitions,
                );
            }
            st.ledger.due_timeouts(now)
        };
        // Logged first, regardless of whether anything is due.
        log_evicted_transitions(&evicted_transitions);
        if due.is_empty() {
            return;
        }
        for d in due {
            // The spec §1 notice template (a Korean-language contract — changing the wording diverges from the priming and acceptance criteria).
            //   The deadline notation uses **the sender's original** as is (fix 6 — so it does not diverge from the envelope `reply-by`).
            // ★The `[engram]` prefix = a mark of the sender origin (user request 2026-07-26)★: the structural signal that `<notice>` has no `from`
            //   alone can leave the receiving LLM missing "who sent this" (when it did not read the priming or the envelope is
            //   passed on summarized). So the first token of the body states the system origin — so it
            //   reads even without the priming. ★It is not a parsing contract★: it is a label for human/LLM readability, and machine decisions still use the tag shape
            //   (`<notice>` · no `from`) as canonical (the code never reads this prefix back).
            let body = format!(
                "[engram] 요청 {} 기한({}) 초과 — {} 회신 없음",
                d.request_id, d.reply_by_raw, d.recipient
            );
            self.deliver_notice(&d, &body);
        }
    }

    /// ★`<notice>` delivery (C3)★ — sends a daemon notice to one recipient. It is **always parking + doorbell**.
    ///
    /// ★Why it does not inject directly here (load-bearing — protects the sweep task)★: this function's only caller is
    ///   `sweep`, and the sweep runs every 60 seconds on a **tokio task** that lives as long as the daemon (lib.rs). `inject` is a
    ///   **blocking write** to the child stdin, so calling it directly here lets one recipient's blocked pipe eat the sweep task
    ///   whole — TTL expiry handling and the **busy cap sweep** (the fail-open wake of a stalled turn) stop with it. So it follows the
    ///   **same discipline** as `FlushTrigger` (fix 11): put it in the queue, press only the doorbell, return immediately,
    ///   and the actual batch write is done by the flush lane (a dedicated thread).
    /// ★FIFO consistency★: if earlier parked items exist, the notice goes behind them too — a notice never overtakes earlier mail.
    /// ★Queue key = **the id of the request sender** (review fix 2 · ADR-0309 · load-bearing)★: a notice is `notified`, so it never
    ///   fires again; missing it here leaves that notice **stranded forever** (the contract silently half-done). The queue key is an id, so even if the
    ///   sender renamed in between, it reaches them. The recipient name in the ledger row is the send-time name (`due.sender`).
    /// ★The doorbell is pressed **always after parking** (same fix)★: not pressing it yields a lost wakeup where something is queued and no one is woken
    ///   (if the sender is asleep that doorbell is empty-handed, and the appearance flush opens it when they come back up).
    // ADR-0103
    // ADR-0309
    fn deliver_notice(&self, due: &DueTimeout, body: &str) {
        // ★Narrowing the timeout↔reply race (review fix 5)★: a reply may have arrived between the due computation (lock released) and here
        //   and closed the contract — then the sender receives the reply and then also a "no reply" notice (contradictory notices).
        //   Look at the ledger again **right before** parking and drop the notice if it has closed. The remaining residue: this check and park are separate lock
        //   spans, so a close in between (microseconds) still lets the notice go out — tying them in one lock would require pulling park
        //   into the ledger (mixing concerns), so here we stop at narrowing the window.
        let closed_since_collection = {
            let st = self.state.lock().expect("messaging state poisoned");
            st.ledger
                .is_request_closed(&due.request_id, due.recipient_id)
        };
        if closed_since_collection {
            tracing::debug!(
                request_id = %due.request_id,
                "기한 초과 통지 취소 — 산출 후 파킹 전에 회신이 도착해 계약이 닫힘(spec §3)"
            );
            return;
        }

        let drawn = self.draw_daemon_msg_id();
        if let Some(collided) = &drawn.collided {
            tracing::error!(
                collided = %collided,
                replacement = %drawn.id,
                still_colliding = drawn.still_colliding,
                "notice id 충돌 — 새 id 로 1회 재시도(ADR-0103 · 사실상 불가한 경로라 난수/장부 배선을 의심할 것)"
            );
        }
        let notice_id = drawn.id;
        self.park_notice(&notice_id, due.sender_id, &due.sender, body);
        self.request_flush(due.sender_id);
    }

    /// ★Draws the msg_id of a daemon self-sent (`<notice>`) message with **the same collision check** as a send (round-final fix 2)★.
    ///
    /// ★Why a notice is checked too (load-bearing)★: it used to use the `new_msg_id()` result as is — only agent sends
    ///   passed `Ledger::msg_id_in_use` and **an id the daemon made was unchecked**. msg_id is the history record key
    ///   ((msg_id, to)) and also the observation correlation key, so if a notice id overlaps an existing id, `transition` picks up someone else's record
    ///   and the ledger smears two messages into one id — the sender being the daemon is no less harmful. So the same check goes
    ///   through the same discipline (a check in one lock span + **one** re-draw).
    /// ★If the second one collides too, that id is used as is (a rule only for notices)★: an agent send can be bounced with `IdCollision` and
    ///   the caller simply retries, but a notice **cannot be discarded** — if a reply contract notice disappears, the contract is silently
    ///   half-done (the same reason the mailbox cap exception exists). Observational pollution (history aliasing) is better than losing the notice. That
    ///   state is exposed as `still_colliding` so the caller logs it as an error outside the lock.
    /// ★Known residual (review blind #5 — not fixed in this round)★: there is **no reservation window** between this check and the actual record —
    ///   if two sends draw the same fresh id at once, both can pass (a condition from before v1, not something the overhaul
    ///   created). For the probability, damage and cost judgment see the reservation comment in `handle_send`. Do not mistake it for a guarded path.
    fn draw_daemon_msg_id(&self) -> DrawnMsgId {
        self.draw_daemon_msg_id_with(new_msg_id)
    }

    /// The **test seam** of the function above — injects the id generator so the collision branch is asserted deterministically (with random numbers it is a
    ///   1-in-36^8 chance and cannot be reproduced). The injected closure is called while holding the lock, so it must be pure (the test only
    ///   takes from a pre-built list).
    fn draw_daemon_msg_id_with(&self, mut draw: impl FnMut() -> String) -> DrawnMsgId {
        let st = self.state.lock().expect("messaging state poisoned");
        let first = draw();
        if !st.ledger.msg_id_in_use(&first) {
            return DrawnMsgId {
                id: first,
                collided: None,
                still_colliding: false,
            };
        }
        let second = draw();
        let still_colliding = st.ledger.msg_id_in_use(&second);
        DrawnMsgId {
            id: second,
            collided: Some(first),
            still_colliding,
        }
    }

    /// Wraps the envelope in the **current** global format (the single wrap point — ADR-0096).
    fn wrap_now(&self, sender: &str, msg_id: &str, body: &str, fields: &EnvelopeFields) -> String {
        let format: EnvelopeFormat = self.registry.envelope_format();
        wrap_message(sender, msg_id, body, format, fields)
    }

    /// ★`in_reply_to` = the structured value the caller passed (F1 review fix, load-bearing — security)★: the old implementation derived it by searching the rendered
    ///   envelope string (`wrapped`) for `in-reply-to="…"` as a substring, but the body escape
    ///   (`escape_xml_text`) does not escape quotes, so a sender who mimicked that attribute string inside the body
    ///   forged the observation (reproduced — envelope.rs `DeliveryObservation.in_reply_to` comment). So now
    ///   `wrapped` is not parsed, and the call site passes as a parameter the `SendMeta.reply_to` it already used when assembling the envelope (the value ingress
    ///   `validate_contract` validated) as is — no text reinterpretation.
    fn observe_success(
        &self,
        msg_id: &str,
        target: &LiveAgent,
        from: SenderIdentity,
        entrance: Entrance,
        wrapped: &str,
        outcome: &InjectReceipt,
        in_reply_to: Option<String>,
    ) {
        self.registry.record_delivery(DeliveryObservation {
            msg_id: msg_id.to_string(),
            to_id: target.id,
            to_name: target.name.clone(),
            from,
            entrance,
            bytes_requested: outcome.bytes_requested,
            bytes_written: Some(outcome.bytes_written),
            msg_uuid: Some(outcome.msg_uuid),
            to_epoch: Some(outcome.epoch),
            in_reply_to,
            error: None,
        });
        // Security: body/token not logged — byte counts and ids only.
        tracing::info!(
            from = %from.peer_id,
            to = %target.id,
            to_name = %target.name,
            msg_id = %msg_id,
            entrance = ?entrance,
            bytes = wrapped.len(),
            "메시징 배달(delivered, ADR-0103/0104)"
        );
    }

    /// Evidence that a failure is not swallowed as a success (ADR-0088).
    ///
    /// ★`in_reply_to` = a structured value (F1 — same reason as the `observe_success` comment)★: the value received as a parameter is carried
    ///   as is (no envelope re-parsing). Even if the write failed and it did not actually arrive, "what was this a reply to" is
    ///   a fact already fixed in the send arguments, so it is recorded as is — but since this record has `is_delivered()==false`,
    ///   a completeness-deciding consumer must not use this value as evidence that "the reply went out" (caller discipline).
    fn observe_failure(
        &self,
        msg_id: &str,
        target: &LiveAgent,
        from: SenderIdentity,
        entrance: Entrance,
        wrapped: &str,
        err: &str,
        in_reply_to: Option<String>,
    ) {
        self.registry.record_delivery(DeliveryObservation {
            msg_id: msg_id.to_string(),
            to_id: target.id,
            to_name: target.name.clone(),
            from,
            entrance,
            bytes_requested: wrapped.len(),
            bytes_written: None,
            msg_uuid: None,
            to_epoch: None,
            in_reply_to,
            error: Some(err.to_string()),
        });
        tracing::warn!(
            to = %target.id,
            to_name = %target.name,
            msg_id = %msg_id,
            "메시징 주입 실패 — 파킹으로 전환(무손실): {err}"
        );
    }

    /// How many async drains (`flush_for`) for this recipient have returned — see the `async_drains_done` field.
    #[cfg(any(test, feature = "test-harness"))]
    pub fn async_drains_done_for_test(&self, recipient: PeerId) -> usize {
        self.async_drains_done
            .lock()
            .expect("async drain counter poisoned")
            .get(&recipient)
            .copied()
            .unwrap_or(0)
    }

    #[cfg(any(test, feature = "test-harness"))]
    pub fn occupied_slots_for_test(&self) -> usize {
        let st = self.state.lock().expect("messaging state poisoned");
        st.ledger.occupied_slots()
    }

    #[cfg(any(test, feature = "test-harness"))]
    pub fn close_contract_for_test(&self, msg_id: &str, recipient_id: PeerId) {
        let mut st = self.state.lock().expect("messaging state poisoned");
        st.ledger
            .close_for_test(msg_id, recipient_id, Instant::now());
    }

    #[cfg(any(test, feature = "test-harness"))]
    pub fn marked_retirements_for_test(&self) -> usize {
        let st = self.state.lock().expect("messaging state poisoned");
        st.ledger.marked_retirement_count_for_test()
    }

    /// Total tracked items — **including closed ones** (a different axis from `open_request_count`).
    #[cfg(any(test, feature = "test-harness"))]
    pub fn tracking_len_for_test(&self) -> usize {
        let st = self.state.lock().expect("messaging state poisoned");
        st.ledger.tracking_len()
    }

    /// ★Contract-axis terminal vocabulary★: `awaiting_reply` | `replied` | `reply_failed` (`None` if not in
    ///   tracking). `replied` and `reply_failed` drop out of the open list · deadline sweep · limit count **identically**, so they cannot be
    ///   told apart on that axis — to assert "acceptance counts as completion, only confirmed unreachability is failure" (spec §3 item 7-④) and "delete cleanup
    ///   does not revert `replied`", one must look at the reason itself.
    #[cfg(any(test, feature = "test-harness"))]
    pub fn contract_outcome_for_test(
        &self,
        msg_id: &str,
        recipient_id: PeerId,
    ) -> Option<&'static str> {
        let st = self.state.lock().expect("messaging state poisoned");
        st.ledger.contract_outcome_for_test(msg_id, recipient_id)
    }

    /// Is that contract key **still in the tracking list** (regardless of closed · retirement mark). Retirement is only a **mark**
    ///   and physical removal happens at commit, so this value is the axis that directly observes the provisional window (it is unrelated to the history ring —
    ///   `msg_id_in_use` stays true even after the window has closed because of the 4096 ring, so it cannot observe that window).
    #[cfg(any(test, feature = "test-harness"))]
    pub fn contract_tracked_for_test(&self, msg_id: &str, recipient_id: PeerId) -> bool {
        let st = self.state.lock().expect("messaging state poisoned");
        st.ledger.is_tracked_for_test(msg_id, recipient_id)
    }

    /// Does the issuing-side collision check see this id as "in use" — tracking · history · provisional reservation **summed**.
    #[cfg(any(test, feature = "test-harness"))]
    pub fn msg_id_in_use_for_test(&self, msg_id: &str) -> bool {
        let st = self.state.lock().expect("messaging state poisoned");
        st.ledger.msg_id_in_use(msg_id)
    }

    /// The list of ledger statuses for a given msg_id — **oldest first**.
    #[cfg(any(test, feature = "test-harness"))]
    pub fn ledger_statuses(&self, msg_id: &str) -> Vec<DeliveryStatus> {
        let st = self.state.lock().expect("messaging state poisoned");
        st.ledger
            .records_for(msg_id)
            .iter()
            .map(|r| r.status)
            .collect()
    }

    /// The list of delivery-record recipient names (`to`) for a given msg_id, oldest first.
    ///
    /// ★Not the terminal key★: the ledger terminal key is `(msg_id, to_id)` (ADR-0309). Same-name twins can have the same `to`,
    ///   so a duplicate in this list is not a key collision — whether each row has exactly one terminal is checked via `ledger_statuses`.
    #[cfg(any(test, feature = "test-harness"))]
    pub fn ledger_endpoint_keys(&self, msg_id: &str) -> Vec<String> {
        let st = self.state.lock().expect("messaging state poisoned");
        st.ledger
            .records_for(msg_id)
            .iter()
            .map(|r| r.to.clone())
            .collect()
    }

    #[cfg(any(test, feature = "test-harness"))]
    pub fn open_request_count(&self) -> usize {
        let st = self.state.lock().expect("messaging state poisoned");
        st.ledger.open_request_count()
    }

    /// Ledger history snapshot `(msg_id, from, to, status)` — oldest first.
    #[cfg(any(test, feature = "test-harness"))]
    pub fn ledger_snapshot(&self) -> Vec<(String, String, String, DeliveryStatus)> {
        let st = self.state.lock().expect("messaging state poisoned");
        st.ledger
            .all_records()
            .iter()
            .map(|r| (r.msg_id.clone(), r.from.clone(), r.to.clone(), r.status))
            .collect()
    }

    /// The predicted value of `Mailbox::can_admit`. The envelope `to` freeze leans on this prediction (the cap result must be
    ///   known before park), so the test directly pins that the prediction and the actual admission do not diverge.
    #[cfg(any(test, feature = "test-harness"))]
    pub fn can_admit_for_test(&self, recipient: PeerId) -> bool {
        let st = self.state.lock().expect("messaging state poisoned");
        st.mailbox.can_admit(recipient, ParkKind::Message)
    }

    #[cfg(any(test, feature = "test-harness"))]
    pub fn parked_len(&self, recipient: PeerId) -> usize {
        let st = self.state.lock().expect("messaging state poisoned");
        st.mailbox.len(recipient)
    }

    /// The sum of all recipient queue items — for asserting 「어느 큐에도 안 들어갔다」 (it did not go into any queue) (a failure row did not create a ghost queue).
    #[cfg(any(test, feature = "test-harness"))]
    pub fn parked_total_for_test(&self) -> usize {
        let st = self.state.lock().expect("messaging state poisoned");
        st.mailbox.total_len()
    }

    /// The recipient queue's **current order** (msg_id, front→back).
    #[cfg(any(test, feature = "test-harness"))]
    pub fn parked_msg_ids(&self, recipient: PeerId) -> Vec<String> {
        let st = self.state.lock().expect("messaging state poisoned");
        st.mailbox.msg_ids(recipient)
    }

    /// The count of **in-flight items not yet settled** for that recipient — the same source as the value the flush deferral decision looks at.
    #[cfg(any(test, feature = "test-harness"))]
    pub fn in_flight_len(&self, recipient: PeerId) -> usize {
        let st = self.state.lock().expect("messaging state poisoned");
        st.mailbox.in_flight_len(recipient)
    }

    /// The **number of sessions (targets)** currently held as being injected — the target-axis counterpart of the value above.
    ///
    /// ★This value failing to return to 0 is this subsystem's worst failure mode★: that session backs off from every later drain and
    ///   its mail is cut off **silently** (the `in_flight_targets` comment — no fail-open). So the test asserts not only the delivery
    ///   outcome but also the reclaim of this value.
    #[cfg(any(test, feature = "test-harness"))]
    pub fn in_flight_target_count(&self) -> usize {
        let st = self.state.lock().expect("messaging state poisoned");
        st.in_flight_targets.len()
    }

    /// **Deliberately corrupts** the payload of one parked item — a seam that asserts via the real path that a broken item does not abort the flush batch and only
    ///   that item degrades to a fallback envelope (C3 review fix 4).
    #[cfg(any(test, feature = "test-harness"))]
    pub fn corrupt_parked_payload_for_test(&self, recipient: PeerId, idx: usize) {
        let mut st = self.state.lock().expect("messaging state poisoned");
        st.mailbox
            .corrupt_envelope_for_test(recipient, idx, "CORRUPT-PAYLOAD".to_string());
    }

    // ── Ledger lookup surface (D · spec §6 `messages { id? }`) ──────────────────────────────────────

    /// ★`messages { id }` — the delivery ledger of that message (spec §6)★. `None` if absent (the entrance translates it to a bounce).
    ///
    /// ★Why time is exposed as elapsed seconds (`age_secs`)★: the ledger's time is `Instant` (a monotonic clock), not a
    ///   wall-clock value — turning it into absolute time would require adding a `SystemTime` axis to the ledger, which is outside the v1 in-memory scope
    ///   (spec §5 "state transition time" is data for relative comparison). So it is converted to **elapsed seconds** from the lookup moment (`now`)
    ///   and sent out — for the receiving LLM too, "3 minutes ago" is more directly useful than "17:42:03".
    /// ★`now` is taken as an argument★: the same discipline as ledger purity (the injected clock, ledger.rs header) — deterministic unit tests.
    /// ★An **open contract** whose history was pushed out entirely is answered with the contract view (review NOTE — resolving a cross-verb contradiction)★: after B3 an
    ///   unanswered contract outlives the history. Then `eg_messages{}` shows that id as outstanding while `eg_messages{id}`
    ///   returns `MESSAGE_NOT_FOUND` — a self-contradiction; the lookup reader takes it as "the list is lying". So
    ///   even with no rows, if the contract is alive it answers with **0 rows + `awaiting_reply=true` + a truncation marker**.
    pub fn message_state(&self, msg_id: &str, now: Instant) -> Option<MessageStateView> {
        let st = self.state.lock().expect("messaging state poisoned");
        let (records, may_be_truncated) = st.ledger.records_for_detailed(msg_id);
        let open = st
            .ledger
            .open_requests()
            .into_iter()
            .find(|r| r.request_id == msg_id);
        if records.is_empty() {
            let open = open?;
            return Some(MessageStateView {
                id: msg_id.to_string(),
                from: open.sender,
                awaiting_reply: true,
                // No row is left at all = the history was pushed out entirely, so truncation is **certain**.
                may_be_truncated: true,
                rows: Vec::new(),
            });
        }
        // The sender is shared by every record (the same logical message) — take it from the first row and put it once at the top.
        let from = records[0].from.clone();
        let rows = records
            .iter()
            .map(|r| DeliveryRowView {
                to: r.to.clone(),
                status: status_label(r.status),
                age_secs: secs_since(r.created_at, now),
                updated_secs_ago: secs_since(r.transitioned_at, now),
                // The hint wording is the single point `row_hint` — the ledger keeps only the code.
                code: r.fail_code,
                hint: r.fail_code.and_then(row_hint),
            })
            .collect();
        Some(MessageStateView {
            id: msg_id.to_string(),
            from,
            awaiting_reply: open.is_some(),
            may_be_truncated,
            rows,
        })
    }

    /// ★`eg_messages` with no argument — the caller's "outstanding" (spec §6)★. Merges three branches into **one list** and gives it oldest first:
    ///   ① what I sent that is not yet plugged in (`from=me` + `pending`) ② the reply wait of a request I made
    ///   (`from=me` + awaiting_reply) ③ requests I received that I have not yet answered (`to=me` + awaiting_reply).
    ///
    /// ★The caller is picked by a single PeerId — all three branches compare ids (ADR-0309 decisions 4 · 5)★: ledger rows and contracts
    ///   hold the party's id, so names are not looked at. So the shares of same-name twins do not mix, and even after a rename, the waiting items sent
    ///   before the rename · the contracts placed · the duty to answer stay in that agent's list. The `from` · `to` seen in a row are
    ///   **the names at send time** (decision 4). The epoch is not looked at (a restart of the same agent is still the same party —
    ///   ADR-0007).
    /// ★Sort = oldest first★: what has aged longest is the work that should be handled first (the same direction as the mailbox flush rule).
    // Review B1 / ADR-0309
    pub fn open_items_for(&self, me_id: PeerId, now: Instant) -> Vec<OpenItemView> {
        let st = self.state.lock().expect("messaging state poisoned");
        let mut items: Vec<OpenItemView> = Vec::new();
        // ① My undelivered items — compared by sender id (`from_id`). The `from_id` of a daemon notice row is nil, so
        //    no agent ever catches it.
        for r in st.ledger.all_records() {
            if r.from_id == me_id && r.status == DeliveryStatus::Pending {
                items.push(OpenItemView {
                    direction: Direction::OutboundPending,
                    id: r.msg_id.clone(),
                    from: r.from.clone(),
                    to: r.to.clone(),
                    age_secs: secs_since(r.created_at, now),
                    reply_by: None,
                    timed_out: false,
                });
            }
        }
        // ②·③ Unanswered contracts. The same contract can also appear in ① (a parked request) — that is not a duplicate but a **different
        //    fact** ("not yet plugged in" and "the reply has not come" are separate states and the prescriptions differ).
        for r in st.ledger.open_requests() {
            // ★Look at the receive duty (③) first★: for a self-send (a request I made to myself) both branches are true at once, and
            //   the fact to report then is the "you must answer" side (just waiting would never close it).
            let direction = if r.recipient_id == me_id {
                Direction::ReplyOwedByMe
            } else if r.sender_id == me_id {
                Direction::AwaitingTheirReply
            } else {
                continue;
            };
            items.push(OpenItemView {
                direction,
                id: r.request_id.clone(),
                from: r.sender.clone(),
                to: r.recipient.clone(),
                age_secs: secs_since(r.created_at, now),
                reply_by: r.reply_by_raw.clone(),
                timed_out: r.notified,
            });
        }
        drop(st);
        // For the same elapsed time, a stable sort by direction · id makes the response deterministic.
        items.sort_by(|a, b| {
            b.age_secs
                .cmp(&a.age_secs)
                .then_with(|| a.direction.as_str().cmp(b.direction.as_str()))
                .then_with(|| a.id.cmp(&b.id))
                .then_with(|| a.to.cmp(&b.to))
        });
        items
    }
}

/// The elapsed seconds between two `Instant`s (never negative). If `now` is in the past it is 0 — the ledger time is a monotonic clock so this does not occur in normal flow,
/// but without saturating, a `now` that a test pushes in by hand would panic.
fn secs_since(then: Instant, now: Instant) -> u64 {
    now.saturating_duration_since(then).as_secs()
}

/// Ledger state → wire vocabulary (spec §5). No new string is invented — it must be **the same vocabulary** as the send response `results[].status`
/// so the sending LLM reads the two responses by the same rule.
fn status_label(s: DeliveryStatus) -> &'static str {
    match s {
        DeliveryStatus::Pending => "pending",
        DeliveryStatus::Delivered => "delivered",
        DeliveryStatus::Replied => "replied",
        DeliveryStatus::Expired => "expired",
        DeliveryStatus::Skipped => "skipped",
        DeliveryStatus::Failed => "failed",
    }
}

/// The `messages { id }` lookup result (the entrance converts it to JSON — the canonical shape is the ingress `handle_messages` comment).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageStateView {
    pub id: String,
    pub from: String,
    pub awaiting_reply: bool,
    pub may_be_truncated: bool,
    pub rows: Vec<DeliveryRowView>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeliveryRowView {
    pub to: String,
    pub status: &'static str,
    pub age_secs: u64,
    pub updated_secs_ago: u64,
    pub code: Option<&'static str>,
    pub hint: Option<String>,
}

/// The lookup row's code → hint wording (**single point** — the ledger keeps only the code and the wording is made only here).
///
/// ★Why the wording is not stored★: carrying a string per record grows the history ring (4096) memory without basis, and when the wording
///   is fixed the old stored wording remains and two expressions coexist. The code is the stable contract and the wording is expression, so the expression is
///   made at lookup time.
fn row_hint(code: &'static str) -> Option<String> {
    match code {
        c if c == FailCode::RecipientDeleted.as_str() => Some(deleted_hint()),
        // Other codes put no hint on the lookup axis (the send response already delivered it on the spot — spec §6).
        _ => None,
    }
}

/// The **direction** of an outstanding item (spec §6 no-argument lookup). The three branches have different prescriptions, so they must be exposed distinctly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// I sent it and it is not yet plugged into the recipient (parked · busy wait). To do: wait (or check the peer's state).
    OutboundPending,
    /// Waiting for the reply to a request I made. To do: wait.
    AwaitingTheirReply,
    /// I received a request and have not yet answered it. **To do: reply now.**
    ReplyOwedByMe,
}

impl Direction {
    /// The wire token (stable contract — the receiving LLM branches on this string).
    pub fn as_str(self) -> &'static str {
        match self {
            Direction::OutboundPending => "outbound_pending",
            Direction::AwaitingTheirReply => "awaiting_their_reply",
            Direction::ReplyOwedByMe => "reply_owed_by_me",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenItemView {
    pub direction: Direction,
    pub id: String,
    pub from: String,
    pub to: String,
    pub age_secs: u64,
    /// If it was a request, the original deadline notation the sender wrote (`"10m"`). None for a notification or a request with no deadline.
    pub reply_by: Option<String>,
    /// Has the overdue notice already gone out (the contract is still open — ledger `OpenRequestView` comment).
    pub timed_out: bool,
}

/// The **decision result** for one recipient (decided inside the lock and carried out outside the lock).
///
/// ★Why split decision and execution (lock discipline)★: enqueue and the ledger must finish atomically inside the lock (no queue "looks-empty window" —
///   `drain_queue` comment), and injection must never be done inside the lock (a port call). So inside the lock only "what to do
///   with this recipient" is decided and carried out as this value.
/// ★That there are only two branches is the shape of this overhaul (ADR-0125)★: if identity holds, **unconditional enqueue** (`Park`), otherwise
///   a failure row — the "inject right now" branch (the old `Deliver`) disappeared with the direct-send shortcut. Reviving it creates a path
///   that skips enqueue, and the order comes loose from the enqueue order.
enum RecipientPlan<'a> {
    Park {
        /// Queue key = the receiving agent id (ADR-0309 — the session id if alive, the profile id if asleep).
        recipient_id: PeerId,
        /// ★`false` = asleep (ADR-0116 decision 1)★: there is no live entity to drain to, so this send does not drain — the delivery
        ///   occasion is the appearance drain the roster diff opens when that id comes back up.
        live: bool,
        contract: PendingContract<'a>,
    },
    /// Failure for this recipient only (ledger terminal recorded) — nothing to do outside the lock. The other recipients go through as is.
    Failed { code: FailCode, hint: String },
}

/// ★What one drain did (ADR-0125)★ — the endpoint through which the caller of a synchronous drain (`handle_send`) reads **the outcome of its own letter**.
/// An async occasion (`flush_for`) has no sender to answer, so it discards this as is.
///
/// ★Why it returns "what was injected"★: because the definition of the response `delivered` is **"the drain of this call actually injected it"**
///   (spec §6). Re-reading the queue state or ledger later to infer it would count what another drain injected in between
///   as its own credit (a false `delivered`), and that would wreck the one value this vocabulary guarantees.
/// ★`retreated` is a separate axis★: it is not "did not go out" but **"cannot know"** (spec §6 ㉯) — the winner's batch
///   may already have sent it. So it is not mixed into the three reasons (㉮).
/// ★The three reason fields are themselves the facts of the caller's share (ADR-0309)★: a drain's recipient is just the one queue id, so the whole
///   drain's outcome is the outcome of the caller's recipient.
// ADR-0142
#[derive(Debug, Default)]
struct DrainReport {
    injected: Vec<String>,
    /// **This drain backed off** — common to both axes (step 0 queue axis · target axis).
    /// ★So the caller need not press its own doorbell again★: when backing off, both axes **register the queue id as a waiter**, and that
    ///   value is exactly the value `ring_or_defer` would have pressed (`handle_send` calls
    ///   `drain_queue(recipient_id, _)` and presses the same id). That is, the winner's settlement **reissues the identical press
    ///   as is** — it is not a probabilistic argument like "pressing again hits the same gate".
    ///   ★But if the winner unwinds mid-injection that reissue does not happen (the only exception)★ — the backed-off side waits until the next doorbell or
    ///   TTL. The conditions and scope are canonical in the residual item of `deferred_by_target`, and the production daemon has no such
    ///   branch (the hook ends it — ADR-0292).
    retreated: bool,
    /// If it was deferred by the idle gate, `Some(그 바쁨 사유)` (Some of that busy reason) (a backend with a turn signal is busy — in a turn · waiting for user input ·
    ///   stopped after an error), otherwise `None`. The value is used by `pending_hint` to choose the hint wording (ADR-0231).
    gated: Option<BusyReason>,
    /// ★Injection failure reason★ — both the basis for the no-own-doorbell decision and the source of the response reason wording.
    caller_inject_error: Option<String>,
}

/// ★The contract reservation's **single-exit RAII guard** (H1 · the guarantee owner of ADR-0108 decision 3)★ — while this value is alive the
/// contract is **provisional**, and it must **always** pass through one of commit (`commit`) or rollback (`rollback`) to disappear.
///
/// ★Why commit is deferred until after the outcome (A2/A3 — closes two defects at once)★:
///   - **A2 (early loss of the victim)**: commit **physically removes** the marked victim. If it commits right at the place where the contract is opened (pass A),
///     then when the **cap gate of the same pass A** later drops this recipient to a failure row, someone else's unanswered contract is
///     **already gone** — nobody got a seat yet someone else's duty evaporates, the ADR-0108 round-3 failure
///     mode. ★In round 7 this window narrowed (ADR-0125)★: the outcome is fixed **inside the enqueue lock** (the out-of-lock "recheck just before
///     injection" branch disappeared with direct send), so the only remaining failure branch is inside that pass A. It narrowed but did not
///     disappear, and moving commit back to the contract-open place regresses that branch at once.
///   - **A3 (self-erosion within one send)**: a provisional contract is excluded from victim candidates (the `!r.provisional` filter of
///     `ledger::open_request`). Deferring commit structurally makes it impossible for recipient 1's contract to be eaten by recipient 2 **of the same send**
///     (no need for a separate batch-scope marker).
///
/// ★Why it must be a **type** (RAII) (H1 — why round 1's `Option<Option<_>>` fell short)★: that is an ordinary
///   value, so **neither the compiler nor tests caught a forgotten branch** (review prober measurement: even changing the settlement of the re-park branch to
///   `let _ = contract.take();` kept the whole suite green). Yet the cost of forgetting is **permanent**:
///     ① the contract stays `provisional` and `due_timeouts` skips it forever (deadline notice gone = the contract silently half-done)
///     ② the marked victim stays `pending_retirement` and drops out of `occupies_slot()` → the cap denominator shrinks **permanently**
///        and `requests` grows without bound.
///   So "forgetting to settle **fails visibly**" is made into a type: if dropped unconsumed, it blows up at once with
///   `debug_assert` (a forgotten branch goes red immediately in tests · debug builds), and in release it **attempts a rollback** together
///   with an error log.
/// ★In the production daemon a panic on this path is not rewound★ — it is an uncontained panic, so the hook ends it (ADR-0292). That is, this guard's
///   motive is **not** "panic unwinding recovery" but ① the forgotten-branch class (path omissions not caught statically) ② unwinding in
///   hookless test builds. Those two are the real exposure surface.
/// ★Drop is the **fast path** and is not itself the guarantee (F1 — load-bearing)★: the settlement point is **while holding the lock**
///   (`commit`/`rollback` take `&mut MessagingState`), so taking `lock()` in Drop would have the same thread re-take its own
///   lock and **deadlock**. So it tries only with `try_lock`, and if that fails (lock contention · the same thread
///   holding the lock · poison) it **skips** the rollback.
/// ★Guarantee = the periodic sweep's reclaim of **abandoned** reservations (`ledger::reclaim_abandoned_reservations`)★: a maintenance point that owns
///   the lock normally rolls back a provisional reservation whose owner is gone. That is, the final guarantee **does not depend on
///   Drop timing** — if Drop succeeds it is immediate, and if it fails it is cleaned up at the next sweep tick at the latest.
/// ★Round-7 correction — where this window lives changed (ADR-0125). The conclusion (keep the guard) stands★: with everything queued,
///   open (`open_reservation`) → settle (`commit`/`rollback`) all came **inside one span of the enqueue lock**. So
///   the normal path has no span where "a reservation is held while the lock is released and injection runs", and the cases where `try_lock` fails
///   narrowed to the **unwinding-while-the-same-thread-holds-the-lock** class (on that branch the sweep's poison `expect` cannot get past either — i.e. the
///   "next sweep tick" fallback above has no effect in that narrow class). ★So do not read it as "there is no out-of-lock injection anymore, so this
///   machinery is obsolete"★ — what this guard really blocks is **a settlement omission not caught statically (a forgotten branch)**, and that risk is
///   independent of the window length (it arises more easily in the current shape that moves between two passes). Removal = reopening ADR-0108, and at that moment
///   a regression of the `STALE_RESERVATION_AFTER` class returns.
/// ★That reclaim looks at **this guard's liveness** (R1 — load-bearing)★: the full rationale is the `ledger::ReservationLiveness`
///   header. Deleting this field or bypassing it with `mem::forget` breaks that guarantee.
/// ★During unwinding `debug_assert` is skipped★: Drop is also called during a panic, and panicking again there is a **double
///   panic → abort** (the test process dies and even the cause is unseen). So if `thread::panicking()` it only
///   leaves a record, and the reclaim is left to the sweep.
// ADR-0108 (mark-and-sweep — the guarantee owner of the reservation window)
struct Reservation<'a> {
    svc: &'a MessagingService,
    msg_id: String,
    // ADR-0309 (contract key = recipient id)
    recipient_id: PeerId,
    /// The victim **marked as pending retirement** by limit pressure (if any). Commit = physical removal + metering, rollback = unmark.
    retired: Option<RetiredContract>,
    /// The settlement-complete marker — set by `commit`/`rollback`. Drop intervenes only when this is false.
    settled: bool,
    /// ★Owner liveness token (R1)★ — while this value is alive the sweep does not reclaim this reservation. Its role is to be
    /// **held only**, not read (the ledger observes it from the weak side).
    _liveness: ReservationLiveness,
}

/// ★Bundles guard creation + liveness token attachment **into one verb** (R1 — load-bearing)★.
///
/// If the caller does the two steps separately, a branch appears where "the guard was made but attaching was forgotten", and that reservation looks abandoned to the sweep even while the owner
/// is working (= a recurrence of the failure chain round 4 caught). So the only way to open a reservation
/// is made this function — do not add a new path that calls `Reservation::new` directly.
/// ★Call site = **the same lock span** as `open_request`★ (so it takes `st` — no window is created for the sweep to
/// slip in between).
fn open_reservation<'a>(
    st: &mut MessagingState,
    svc: &'a MessagingService,
    msg_id: &str,
    recipient_id: PeerId,
    retired: Option<RetiredContract>,
) -> Reservation<'a> {
    let liveness = ReservationLiveness::new();
    st.ledger
        .attach_reservation_liveness(msg_id, recipient_id, liveness.watch());
    Reservation::new(svc, msg_id, recipient_id, retired, liveness)
}

impl<'a> Reservation<'a> {
    /// ★Caller duty (R1)★: `liveness.watch()` must be attached to the ledger entry **in the same lock span**
    /// (`Ledger::attach_reservation_liveness`).
    fn new(
        svc: &'a MessagingService,
        msg_id: &str,
        recipient_id: PeerId,
        retired: Option<RetiredContract>,
        liveness: ReservationLiveness,
    ) -> Self {
        Self {
            svc,
            msg_id: msg_id.to_string(),
            recipient_id,
            retired,
            settled: false,
            _liveness: liveness,
        }
    }

    fn commit(mut self, st: &mut MessagingState, log: &mut RetirementLog) {
        self.settled = true;
        let retired = self.retired.take();
        let outcome = st.ledger.commit_open(
            Some((self.msg_id.as_str(), self.recipient_id)),
            retired
                .as_ref()
                .map(|r| (r.request_id.as_str(), r.recipient_id)),
        );
        // ★Metering reports **only what happened** (R2)★: the planned victim may have vanished in the meantime (if it was closed by a reply and
        //   its history row pushed out, `purge_finished_without_history` cleans it up — the known residual of `ledger::rollback_open`).
        //   Reporting the plan as fact pollutes the axis that ADR-0108 decision 2 nailed down as the only evidence of retirement.
        //   So it is counted as a retirement only **when the ledger answers that it actually removed it**, and a mismatch is routed to an anomaly record.
        if let Some(r) = retired {
            match outcome.retired {
                true => log.real.push(r),
                false => log.phantom.push(r),
            }
        }
    }

    fn rollback(mut self, st: &mut MessagingState) {
        self.settled = true;
        let retired = self.retired.take();
        rollback_reservation(
            st,
            self.msg_id.as_str(),
            self.recipient_id,
            retired.as_ref(),
        );
    }
}

impl Drop for Reservation<'_> {
    fn drop(&mut self) {
        if self.settled {
            return;
        }
        let retired = self.retired.take();
        match self.svc.state.try_lock() {
            Ok(mut st) => {
                rollback_reservation(
                    &mut st,
                    self.msg_id.as_str(),
                    self.recipient_id,
                    retired.as_ref(),
                );
                tracing::error!(
                    msg_id = %self.msg_id,
                    recipient_id = %self.recipient_id,
                    "정산되지 않은 계약 예약을 Drop 에서 롤백 — 호출 경로에 커밋/롤백 누락이 있다(H1)"
                );
            }
            Err(_) => tracing::error!(
                msg_id = %self.msg_id,
                recipient_id = %self.recipient_id,
                "정산되지 않은 계약 예약을 롤백하지 못함(락 보유 중이거나 poison) — 잠정 계약과 은퇴 표시가 남는다(H1)"
            ),
        }
        // ★Blow up **after** the rollback★: reversing the order lets the panic block recovery so the test cannot observe "is the state intact when settlement is forgotten"
        //   (and in release there is no debug_assert, leaving only recovery — the same code path).
        debug_assert!(
            std::thread::panicking(),
            "회신 계약 예약이 정산 없이 소멸했다 — commit/rollback 중 하나를 반드시 거쳐야 한다(H1)"
        );
    }
}

/// The core of reservation cancellation (shared by the guard's `rollback` and Drop — so the rule does not diverge in two places).
fn rollback_reservation(
    st: &mut MessagingState,
    msg_id: &str,
    recipient_id: PeerId,
    retired: Option<&RetiredContract>,
) {
    st.ledger.rollback_open(
        Some((msg_id, recipient_id)),
        retired.map(|r| (r.request_id.as_str(), r.recipient_id)),
    );
}

/// A reservation not yet settled (`None` for a notification/reply, which has no contract).
type PendingContract<'a> = Option<Reservation<'a>>;

/// The resolution result for one recipient — resolved **only once** from one snapshot taken at send time (ADR-0111 decision 2). Re-resolving by
/// name in the decision step would demote an id target to name resolution, and the answer would diverge within the same snapshot.
// ADR-0309 (decision 3 — name → id only at this entrance)
enum Resolution {
    Live(LiveAgent),
    /// One asleep profile — parking (spec §5 branch 3 · ADR-0116 decision 1).
    Dormant {
        id: PeerId,
        name: String,
    },
    /// The name matched two or more in one layer → `RECIPIENT_AMBIGUOUS`.
    Ambiguous {
        count: usize,
        layer: NameLayer,
    },
    /// Not anywhere → `RECIPIENT_NOT_FOUND` (no parking — ADR-0116 decision 1).
    NotFound,
}

/// The layer where the duplicate name was caught — the hint's count and label must point at the same layer (`ambiguous_hint`).
#[derive(Debug, Clone, Copy)]
enum NameLayer {
    Live,
    Saved,
}

impl NameLayer {
    fn label(self) -> &'static str {
        match self {
            NameLayer::Live => "live",
            NameLayer::Saved => "saved",
        }
    }
}

struct ResolvedRecipient {
    /// The **notation as the sender wrote it** to put in the response row (trimmed only — WYSIWYA, ADR-0101). If it is an `@` expansion result, it is the registry name.
    display: String,
    to: Resolution,
}

impl ResolvedRecipient {
    /// The receiving id — the session id if alive, the profile id if asleep. `None` if it could not be resolved.
    fn id(&self) -> Option<PeerId> {
        match &self.to {
            Resolution::Live(a) => Some(a.id),
            Resolution::Dormant { id, .. } => Some(*id),
            Resolution::Ambiguous { .. } | Resolution::NotFound => None,
        }
    }

    /// The name written to the ledger row · parked item · envelope `to` — the registry name if resolved (the name stays even when targeted by id),
    /// the notation if not resolved.
    fn name(&self) -> &str {
        match &self.to {
            Resolution::Live(a) => &a.name,
            Resolution::Dormant { name, .. } => name,
            Resolution::Ambiguous { .. } | Resolution::NotFound => &self.display,
        }
    }

    /// Is it the same row — the same id if resolved, the same notation if not (`push_recipient`).
    fn same_row(&self, other: &ResolvedRecipient) -> bool {
        match (self.id(), other.id()) {
            (Some(a), Some(b)) => a == b,
            (None, None) => self.display == other.display,
            _ => false,
        }
    }
}

/// One input token (input order preserved) — the order axis of envelope `to` assembly and the source of **the rows that token contributed**
/// (a slot in `Addressing.recipients`).
///
/// ★Why the token holds "the rows it contributed" (A5 regression · load-bearing)★: if inclusion in the `to` attribute is decided by "did that token
///   **create a row** after passing dedup", then when the expansion of `@all` is **completely absorbed** by earlier explicit
///   targets, as in `["bob","carol","@all"]`, `@all` drops out wholesale (`to="bob,carol"`). spec §1 nails down that the value is a listing of **the tokens
///   the sender wrote** and that only **failed tokens** drop out — absorption is not failure. So each token records
///   all the rows it resolved, and the inclusion condition is "was any of those rows decided as accepted".
enum AddressToken {
    Name { row: usize },
    Group { label: String, rows: Vec<usize> },
}

struct Addressing {
    recipients: Vec<ResolvedRecipient>,
    tokens: Vec<AddressToken>,
}

/// ★Recipient resolution (spec §5 resolution order · pure — it looks only at the entrance decision-source snapshot)★.
///
/// Order: ① trim (comma splitting is **CLI-entrance only** so it is already done) → ② expand `@` elements (**excluding the sender**) →
/// ③ join with name elements → ④ resolve per element (registry lookup — expanded elements are already ids) → ⑤ dedup (fold key = the resolved id,
/// or the notation if unresolved · one recipient = one delivery · one result row). The 3-branch decision is made by the caller (`admission`).
///
/// ★Row order = deterministic (pinned by tests — spec §5)★: ① the **explicit tokens in the order written** in the input, ② then
///   the `@`address **expansion results in name lexicographic order**. Dedup **keeps the one that appeared first** (explicit targets
///   always precede expansion results, so an overlapping recipient's row position follows its explicit position).
/// ★`@`address error = whole send-level bounce (ADR-0114 decision 3)★: an `@이름` (an @name) other than `@all`, or a rule violation, is
///   `GROUP_NOT_FOUND` (even with a mixed `to`, nothing goes to the remaining recipients). **If the final set is empty**
///   `GROUP_EMPTY` — decided not per element but by the result of expansion ∪ explicit targets.
/// ★Sender exclusion in expansion (spec §4 — **the canonical source is the spec, not ADR-0111**)★: both vocabularies mean "everyone but me".
///   ADR-0111 decision 4 has no such wording, so do not look for it there — omit it and the sender receives their own broadcast.
///   **A direct-target self-send is delivered as is** (the exclusion applies only to expansion), so `["@all", "<자기이름>"]` (the sender's own name) sends
///   1 row to oneself.
// ADR-0111 (group = a resolution macro · joining multiple recipients)
// ADR-0114 (@address error layer · GROUP_EMPTY = the final set)
// ADR-0121 (two vocabularies — @all = the whole registry / @here = everyone alive)
fn resolve_addressing(
    groups: &dyn GroupSource,
    to: &[String],
    from: SenderIdentity,
    sources: &AddressingSources,
) -> Result<Addressing, SendReject> {
    // ★Which vocabulary reads which pool is decided by the **source** (`groups::MemberPools`)★ — all that is done here is to build the
    //   pool and **remove the sender** (ADR-0121 decision 1).
    // ★The sender is removed by id only (ADR-0309 · TRD S29 §4 X3)★: removing by name would also drop from the broadcast another agent that merely shares the sender's name
    //   (a live twin · an asleep profile with the same name). Expanded members resolve directly by id, so there is also no path where an asleep same-name
    //   resolves to the sender's own live session and receives its own broadcast.
    // ★A live session with no turn signal **enters** the live pool★ — since the roster eligibility has no capability condition (ADR-0116
    //   decision 7), a broadcast does not drop that class (it is injected immediately with no gate).
    let live_pool: Vec<GroupMember> = sources
        .roster
        .iter()
        .filter(|a| a.id != from.peer_id)
        .map(|a| GroupMember {
            id: a.id,
            name: a.name.clone(),
        })
        .collect();
    let dormant_pool: Vec<GroupMember> = sources
        .dormant
        .iter()
        .filter(|d| d.id != from.peer_id)
        .map(|d| GroupMember {
            id: d.id,
            name: d.name.clone(),
        })
        .collect();
    let pools = MemberPools {
        live: &live_pool,
        dormant: &dormant_pool,
    };

    let mut tokens: Vec<AddressToken> = Vec::with_capacity(to.len());
    let mut recipients: Vec<ResolvedRecipient> = Vec::new();
    let mut expanded: Vec<(usize, GroupMember)> = Vec::new();

    for (i, raw) in to.iter().enumerate() {
        let token = raw.trim();
        if token.starts_with('@') {
            let norm = normalize_group_name(token).map_err(|e| group_reject(e, token))?;
            let members = groups
                .resolve(&norm, pools)
                .map_err(|e| group_reject(e, token))?;
            for m in members {
                expanded.push((i, m));
            }
            tokens.push(AddressToken::Group {
                label: norm,
                rows: Vec::new(),
            });
        } else {
            let row = push_recipient(
                &mut recipients,
                ResolvedRecipient {
                    display: token.to_string(),
                    to: resolve_token(token, sources),
                },
            );
            tokens.push(AddressToken::Name { row });
        }
    }
    expanded.sort_by(|a, b| a.1.name.cmp(&b.1.name).then(a.0.cmp(&b.0)));
    for (i, member) in expanded {
        // ★Expanded members resolve directly by id (ADR-0309 decision 3)★ — resolving again by name folds same-name twins into a single
        //   `RECIPIENT_AMBIGUOUS` row (TRD S29 §4 X1). An id not in the pool is a source defect, so it is surfaced as not found.
        let to = resolve_id(member.id, sources).unwrap_or(Resolution::NotFound);
        let row = push_recipient(
            &mut recipients,
            ResolvedRecipient {
                display: member.name,
                to,
            },
        );
        if let AddressToken::Group { rows, .. } = &mut tokens[i] {
            if !rows.contains(&row) {
                rows.push(row);
            }
        }
    }

    if recipients.is_empty() {
        // If there is even one explicit target it cannot come here (a name token creates a row even if resolution fails) — so this branch is only
        //   "there were only `@`addresses and all expanded to 0 people" (a typical case is `["@all"]` with only the sender alive).
        return Err(SendReject::GroupEmpty);
    }
    Ok(Addressing { recipients, tokens })
}

/// ★One token → the recipient (spec §5)★ — the **same rule** as the control entrance's `resolve_in` (agent crate).
///
/// ① If it reads as a UUID (`PeerId::parse_str` — including uppercase · braces · no hyphens · the `urn:uuid:` spelling) look it up as a live id → asleep
///    id in that order, and if absent fall through to the name. The id comes first so a **name** shaped like a UUID cannot hijack
///    someone else's id target, and an id-shaped string not in the registry resolves as such a name.
/// ② Name (exact match — case-sensitive · no prefix matching): live 1 = live · live 2+ = ambiguous (live) · live 0 and asleep 1 = asleep ·
///    asleep 2+ = ambiguous (stored) · 0 = not found. ★If even one exists in the live layer the asleep layer is not looked at (spec §5 branch order)★ —
///    that is what makes "1 live + 1 asleep with the same name → delivered to the live one" hold.
/// ★If the two entrances diverge, the agent that received the letter and the agent the control command reached differ★ — the daemon crate's cross-
///   check test (`tests/control_agent.rs` `resolver_alignment`) guards this by running this function through `handle_send`.
///   The domains differ: control sees live and asleep agents as one registry, while mail has the live layer beat the asleep layer.
// ADR-0116 (decision 1 — 3 branches · same name in the asleep layer)
// ADR-0120 (the live layer beats the asleep layer · blocking same name in the asleep layer — remains as a name-target rule)
// ADR-0309 (decision 3 — the asleep side is also targeted by id · the id spelling is the same as `resolve_in`)
fn resolve_token(token: &str, sources: &AddressingSources) -> Resolution {
    if let Ok(id) = PeerId::parse_str(token) {
        if let Some(found) = resolve_id(id, sources) {
            return found;
        }
    }
    let live: Vec<&LiveAgent> = sources.roster.iter().filter(|a| a.name == token).collect();
    match live.as_slice() {
        [] => {}
        [only] => return Resolution::Live((*only).clone()),
        many => {
            return Resolution::Ambiguous {
                count: many.len(),
                layer: NameLayer::Live,
            }
        }
    }
    let dormant: Vec<&DormantAgent> = sources.dormant.iter().filter(|d| d.name == token).collect();
    match dormant.as_slice() {
        [] => Resolution::NotFound,
        [only] => Resolution::Dormant {
            id: only.id,
            name: only.name.clone(),
        },
        many => Resolution::Ambiguous {
            count: many.len(),
            layer: NameLayer::Saved,
        },
    }
}

/// One registry id → the recipient. `None` if it is in neither live nor asleep.
fn resolve_id(id: PeerId, sources: &AddressingSources) -> Option<Resolution> {
    if let Some(a) = sources.roster.iter().find(|a| a.id == id) {
        return Some(Resolution::Live(a.clone()));
    }
    sources
        .dormant
        .iter()
        .find(|d| d.id == id)
        .map(|d| Resolution::Dormant {
            id: d.id,
            name: d.name.clone(),
        })
}

/// Puts one recipient into the row list and returns that row's position — the existing position if the same row is already there.
///
/// ★Dedup = keep the position that appeared first (spec §5)★ — the **row position and notation** of an overlapping recipient follow the token written first
///   (the response `to` is the notation the sender wrote first — WYSIWYA).
/// ★Same row = the same id if resolved, the same notation if unresolved (ADR-0309)★: targeting the same agent by overlapping name · id · `@all`
///   is still one row, and different agents are each their own row even with the same name — targeting same-name twins each by id
///   delivers to both (TRD S29 §4 X2). ★Do not fold by name★ — one twin vanishes without a row (violates spec §6 "one recipient
///   = 1 row"). An unresolved notation does not mix with resolved rows, so `["dup", "<dup1 id>"]` is a name row
///   `RECIPIENT_AMBIGUOUS` + an id row delivered — two rows.
fn push_recipient(recipients: &mut Vec<ResolvedRecipient>, candidate: ResolvedRecipient) -> usize {
    if let Some(pos) = recipients.iter().position(|r| r.same_row(&candidate)) {
        return pos;
    }
    recipients.push(candidate);
    recipients.len() - 1
}

/// An `@`address resolution error → send-level bounce. A rule violation is also folded into "there is no such address" — it is a fact that fits the sender and
/// no new code outside the spec §6 vocabulary is made (the entrance's hint tells how to correct it).
fn group_reject(e: GroupError, raw: &str) -> SendReject {
    match e {
        GroupError::NotFound { name } => SendReject::GroupNotFound { name },
        GroupError::InvalidName { .. } => SendReject::GroupNotFound {
            name: raw.to_string(),
        },
    }
}

/// ★Envelope `to` value assembly (spec §1 — freeze target)★: **accepted explicit targets use the canonical name**, **`@`addresses are not expanded
/// but kept as the token**, **failed tokens are excluded**, and the listing is in **input notation order** (`"@all,carol"`).
///
/// ★Inclusion condition = "the recipients that token resolved are **not all failures**" (A5 regression — load-bearing)★:
///   the decision axis is **not** "did it create a row". A token absorbed by dedup (the `@all` in `["bob","carol","@all"]`)
///   creates no row but is **not a failure either** — those recipients are delivered normally under other tokens' positions. spec §1 said to
///   drop only **failed tokens**, so absorption does not remove the reason for inclusion.
/// ★The same value is written only once (M4)★: a notation duplicate (`["bob","bob"]`) or a name · id target of the same agent
///   (`["<bob id>","bob"]`) coming out as `to="bob,bob"` reports a false headcount to the receiving LLM. Keep the position that appeared first
///   and fold later duplicates (the same direction as the row order rule). Values are compared, so even if two same-name twins are accepted the name is
///   written once.
/// ★The expanded list is not carried instead★ — disclosing the recipient list·roles (cc) is v2 (spec §8).
fn build_to_attr(addr: &Addressing, admitted: &[bool]) -> String {
    // The index of `admitted` is the same as the `recipients` order.
    let mut parts: Vec<String> = Vec::new();
    for token in &addr.tokens {
        match token {
            AddressToken::Name { row } => {
                let name = addr.recipients[*row].name();
                if admitted[*row] && !parts.iter().any(|p| p == name) {
                    parts.push(name.to_string());
                }
            }
            AddressToken::Group { label, rows } => {
                if rows.iter().any(|r| admitted[*r]) && !parts.iter().any(|p| p == label) {
                    parts.push(label.clone());
                }
            }
        }
    }
    parts.join(",")
}

fn live_names_hint(roster: &[LiveAgent]) -> String {
    if roster.is_empty() {
        return "(none)".to_string();
    }
    roster
        .iter()
        .map(|a| a.name.clone())
        .collect::<Vec<_>>()
        .join(", ")
}

/// ★The **single decision point** of the entrance 3 branches (spec §5 · ADR-0116 decision 1)★ — if accepted, (receiving id, alive?), if failed,
/// (code, hint). Alive = delivery target · asleep = parking (accepted) · duplicate names = `RECIPIENT_AMBIGUOUS` · not found =
/// `RECIPIENT_NOT_FOUND` (no parking).
/// ★A live session is a delivery target even with no turn signal (ADR-0116 decision 7)★ — the old `RECIPIENT_UNREACHABLE` branch was discarded.
///   Do not revive it.
// ADR-0116 (decision 1 — 3-branch decision)
fn admission(
    r: &ResolvedRecipient,
    roster_names: &str,
) -> Result<(PeerId, bool), (FailCode, String)> {
    match &r.to {
        Resolution::Live(a) => Ok((a.id, true)),
        Resolution::Dormant { id, .. } => Ok((*id, false)),
        Resolution::Ambiguous { count, layer } => Err((
            FailCode::RecipientAmbiguous,
            ambiguous_hint(&r.display, *count, layer.label()),
        )),
        Resolution::NotFound => Err((
            FailCode::RecipientNotFound,
            format!(
                "No agent named '{}' exists right now — not running and no saved agent by that name, so nothing was queued for it (fix the name, or create/spawn it and send again). Live agents: {roster_names}.",
                r.display
            ),
        )),
    }
}

/// ★The contract-handling branch of a reply send (spec §3 item 7-④)★ — `handle_send` step 5 branches on this value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReplyDisposition {
    /// Accepted (`delivered`|`pending`) → closed as `replied`. **Parking is acceptance too**.
    Accepted,
    /// The requester name is nowhere (`RECIPIENT_NOT_FOUND`) → `reply_failed` failure close.
    RequesterGone,
    /// Any other failure (`MAILBOX_FULL`·`RECIPIENT_AMBIGUOUS`) → **no-op** (stays open).
    NoOp,
}

/// ★Reply outcome decision (spec §3 item 7-④ · ADR-0116 decision 2)★ — decided by looking only at the response rows (the ledger is not looked at again).
///
/// ★A reply has exactly 1 recipient (spec §3 item 7-①, enforced by ingress at the notation step)★. Even so this function
///   decides from the row list — `handle_send` is pub, so another assembly (smoke bin · a future entrance) can call it with several rows,
///   and then "accepted if even one is accepted" is the only safe interpretation (if one reply actually arrived, the duty
///   was fulfilled). ★`RECIPIENT_NOT_FOUND` is looked at only when **every row failed**★: if it went to even one person, acceptance wins.
// ADR-0116 (decision 2)
fn reply_disposition(results: &[RecipientResult]) -> ReplyDisposition {
    if results.iter().any(|r| r.status != SendStatus::Failed) {
        return ReplyDisposition::Accepted;
    }
    if results
        .iter()
        .any(|r| r.code == Some(FailCode::RecipientNotFound))
    {
        return ReplyDisposition::RequesterGone;
    }
    ReplyDisposition::NoOp
}

/// ★The common tail of same-name failure hints (ADR-0116 decision 5 — user decision 2026-07-30)★.
///
/// ★It does **not** give exact-id resend guidance★: same-name agents themselves are unsupported (root removal = ADR-0115), so the id-rescue
///   flow must not be taught as if it were supported policy. What the sending LLM should do is not retry but **tell the
///   user** — resending repeats the same result and only burns context.
const AMBIGUOUS_ESCALATION: &str = "Duplicate agent names are not supported: do NOT resend this message — tell the user, who can rename or retire one of them.";

/// The hint of a duplicate-names failure row (shared by the live and asleep layers — `layer` is a layer name a human reads).
///
/// ★The count and the label must point at the same layer (review fix D6-a)★: the live layer counts the **roster count** and
///   its label is `live` — the 4th-round roster is "every live session" (regardless of turn signal), so that label matches the facts exactly
///   (in round 3 a structured condition was attached, so "live" pointed at fewer than actually existed). The asleep layer is the profile count +
///   `saved`. Changing only one side makes the sender read "it says two but I see only one" (spec §5 hint label rule).
fn ambiguous_hint(display: &str, count: usize, layer: &str) -> String {
    format!("'{display}' matches {count} {layer} agents, so the broker cannot tell which one you mean — nothing was queued. {AMBIGUOUS_ESCALATION}")
}

/// ★The lookup hint of a parked item closed by delete cleanup (spec §6 `RECIPIENT_DELETED`)★ — carries the next action next to the code.
fn deleted_hint() -> String {
    "The recipient was deleted while this message was still parked, so it was closed as undelivered. The name may now belong to a different agent — do not resend automatically; tell the user.".to_string()
}

/// Asleep-parking hint (spec §5 branch 3 — ADR-0116 decision 1). Lets the sender read "why is it pending" and the outcome.
///
/// ★Expiry is quiet (user decision 2026-07-30 — known and accepted)★: if no one restores it, it becomes `expired` by the 24h TTL and
///   **there is no active notice to the sender** (the send response's `pending` + a later `eg_messages` lookup are all there is). So that fact is
///   written in the hint — so the sending LLM does not misread it as "it will surely arrive someday".
fn park_hint_dormant(display: &str) -> String {
    format!(
        "'{display}' is not running right now but it is a saved agent — parked; it will be delivered as one batch when that agent is restored. Nobody is notified if it expires first (24h TTL), so check with `eg_messages` if it matters."
    )
}

/// Busy hint (spec §6 ㉮①) — the wording states the real busy reason. Telling a recipient stopped after an error that it is "in a turn"
/// makes the sending LLM misread it as waiting for a turn that is about to end.
// ADR-0231
fn park_hint_busy(display: &str, reason: BusyReason) -> String {
    match reason {
        BusyReason::HaltedAfterError => format!(
            "'{display}' stopped after a failed turn — queued; it will be delivered after the user's next turn succeeds. Nobody is notified if it expires first (24h TTL), so check with `eg_messages` if it matters."
        ),
        BusyReason::InputsPending => format!(
            "'{display}' has user input waiting — queued; it will be delivered after the turn that handles that input ends."
        ),
        BusyReason::InTurn => format!(
            "'{display}' is mid-turn — queued; it will be delivered as one batch when that turn ends."
        ),
    }
}

/// Injection failure hint (spec §6 ㉮③).
fn park_hint_inject_failed(display: &str, err: &str) -> String {
    format!(
        "Delivery to '{display}' failed ({err}) — it stays queued and is retried on the next drain (expires after TTL)."
    )
}

/// ★Backed off from an overlapping drain — the "cannot confirm" hint (spec §6 ㉯ · load-bearing)★.
///
/// ★The wording must not read as "it did not go"★: this letter **may already have been delivered on the winner's batch**. The sending
///   LLM concluding that `pending` means "the peer did not receive it" and reporting so is the misreading the spec §7 priming blocks, and
///   the hint must not encourage that misreading — the real outcome is seen via an `eg_messages{id}` lookup.
fn park_hint_overlapping(display: &str) -> String {
    format!(
        "Another send to '{display}' was draining that mailbox at the same time, so this call could not confirm delivery — it was NOT lost: it either went out with that batch or is still queued. Check `eg_messages` if you need to know which."
    )
}

/// At drain time the agent of that id was not in the roster so it stayed in the queue (ADR-0309).
fn park_hint_queued(display: &str) -> String {
    format!(
        "'{display}' could not be reached by this drain — it stays queued and goes out on the next occasion (appearance / idle transition / a later send's drain)."
    )
}

/// ★Choosing the `pending` hint when the drain could not send this letter (spec §6)★ — backing off (㉯) takes top priority (mixed with other reasons
/// the sender misreads the outcome — `park_hint_overlapping`).
///
/// ★The reason chosen is that of **this letter**★: a drain's recipient is just one queue id, so the report's reason is the reason of this letter's
///   recipient (`DrainReport`).
fn pending_hint(display: &str, report: &DrainReport) -> String {
    if report.retreated {
        return park_hint_overlapping(display);
    }
    if let Some(err) = &report.caller_inject_error {
        return park_hint_inject_failed(display, err);
    }
    if let Some(reason) = report.gated {
        return park_hint_busy(display, reason);
    }
    park_hint_queued(display)
}

/// Mailbox-full failure row hint (no reclaim attempt — ADR-0114 decision 1).
fn mailbox_full_hint(display: &str) -> String {
    format!(
        "'{display}' mailbox is full (100 parked messages) — nothing was queued for it; its oldest messages expire by TTL, so retry later. Other recipients were unaffected."
    )
}

/// ★The retirement facts one send gathered (R2)★ — carries **what happened** and **what was planned but did not happen** separately.
///
/// Putting both in one bin pollutes metering (`Reservation::commit` comment). Outside the lock they are each logged at a different level.
#[derive(Default)]
struct RetirementLog {
    real: Vec<RetiredContract>,
    phantom: Vec<RetiredContract>,
}

/// ★Test observation surface for retirement metering (F5)★ — the `(계약 msg_id, 수신자 id)` (contract msg_id, recipient id) that `log_contract_retirements` actually reported.
///
/// Without a log-capture harness (tracing subscriber) it looks only at **whether the call happened**. What is observed is not the log wording but
/// **whether that call remains after the outcome loop** (the M1 regression axis).
#[cfg(any(test, feature = "test-harness"))]
pub(crate) mod retirement_reports {
    use std::cell::RefCell;

    thread_local! {
        static REPORTS: RefCell<Vec<(String, crate::PeerId)>> = const { RefCell::new(Vec::new()) };
    }

    pub(crate) fn push(request_id: &str, recipient_id: crate::PeerId) {
        REPORTS.with(|r| r.borrow_mut().push((request_id.to_string(), recipient_id)));
    }

    /// **Takes out and empties** the reports so far (per-section assertions — so the surplus a fixture left does not mix in).
    ///
    /// In a lib build with only the `test-harness` feature on (not a test target) there is no caller and a dead_code warning appears —
    /// the observation surface is test-only, so that is normal.
    #[allow(dead_code)]
    pub(crate) fn drain() -> Vec<(String, crate::PeerId)> {
        REPORTS.with(|r| std::mem::take(&mut *r.borrow_mut()))
    }
}

/// Meters contracts actually retired by limit pressure **outside the lock** (logging before commit reports something that did not happen).
///
/// ★The call site is **after the outcome loop** (M1)★: the commit moved down into pass B (A2), so logging before the loop leaves the retirement on the main path
///   (a request to an idle recipient) with no log at all — under ADR-0108 decision 2 this info log is the
///   **only** evidence of retirement.
/// ★M2 — do not revert to "commit right where the contract is opened"★: the cap gate of the same pass A can drop
///   this recipient to a failure row afterwards, and that branch really needs the rollback (the window only narrowed in round 7 —
///   the `Reservation` header is canonical). Reverting is an A2/A3 regression.
fn log_contract_retirements(new_msg_id: &str, retired: &[RetiredContract]) {
    for r in retired {
        // ★Why thread-local (F5)★: this function is a free function with no service handle (the signature is not changed
        //   for observation), and the send path runs to the end on the calling thread, so the observation does not cross a thread boundary.
        #[cfg(any(test, feature = "test-harness"))]
        retirement_reports::push(&r.request_id, r.recipient_id);
        // Field-only metering — body and token are not carried. By the retirement-eligible condition there is no notice debt owed to the sender
        //   (only notified items or contracts with no deadline — ledger `open_request` comment).
        tracing::info!(
            retired_msg_id = %r.request_id,
            from = %r.sender,
            to = %r.recipient,
            to_id = %r.recipient_id,
            age_secs = r.age.as_secs(),
            new_msg_id = %new_msg_id,
            "미회신 계약 상한 압력 — 가장 오래된 은퇴 가능 계약을 내보내고 새 request 수용(ADR-0108)"
        );
    }
}

struct ParkRequest<'a> {
    msg_id: &'a str,
    sender_name: &'a str,
    from: SenderIdentity,
    entrance: Entrance,
    /// Queue key = the receiving agent id (ADR-0309).
    recipient: PeerId,
    /// The recipient name to write on the ledger row (a send-time snapshot — ADR-0309 decision 4).
    recipient_name: &'a str,
    body: &'a str,
    kind: ParkKind,
    meta: &'a SendMeta,
    /// ★The total delivery records this logical message will leave (round-2 review F3)★ — notice = 1, send = the recipient count **including failure rows**.
    /// Parking also leaves one history row (`Pending`), so the same expected count must be stamped on that row for the lookup's truncation decision to hold.
    expected_rows: u16,
}

/// ★The device that finishes the reply contract close **in the same critical section as the acceptance record** (review fix D2 — load-bearing)★.
///
/// ★What it blocks = the reversal "accepted but the contract is `reply_failed`"★ (violating the ratified rule "acceptance = completion, no reversal"):
///   the old wiring was ① park the reply under lock A (acceptance confirmed) → release the lock → ② close the contract as `replied` under lock B.
///   If a **delete cleanup** (`handle_profile_deleted`) slips between the two locks, it closes that contract as `reply_failed` because the requester was deleted,
///   and the late ② quietly backs off with `AlreadyClosed` — the result is "the reply was received as `pending` but the
///   contract is reply-failed". On the parking acceptance path there is **no reason at all** to split the two locks (parking does not
///   call inject = nothing that cannot be done inside the lock). So it is closed on the spot.
/// ★In round 7 the acceptance branch is **enqueue alone** (ADR-0125)★: a send is enqueued without exception, so the acceptance-confirmation point is
///   the one enqueue lock, and closing inside it makes atomicity hold on **every branch** (the second confirmation
///   point of the old immediate-delivery branch disappeared with direct-send abolition). What the following drain promotes to delivery is the **delivery axis**, so it is unrelated to
///   the contract axis's atomicity. (The alternative, "block cleanup with a mid-flight marker", introduces new state and risks the contract
///   never closing if it leaks, so it was not adopted — this approach has zero new state.)
/// ★Idempotent★: once `done` is filled it does not close again (with a multi-recipient reply the first acceptance closes the contract and the rest are no-ops —
///   the same direction as the "accepted if even one is accepted" rule, `reply_disposition`).
/// ★Caveat — the idempotency **does not distinguish outcome kinds** (harmless now · a future trap, review fix N-nit)★: `NoMatch` (not
///   my contract) is also recorded as "closed", so later rows do not retry. It is harmless now — the entrance forces a reply send to have **exactly
///   1 recipient** (`control/ingress.rs` rule 2 · spec §3 item 7-①), so there is only one row to begin with. But
///   `handle_send` is pub, so if a caller that skips that enforcement passes a multi-row reply, **an earlier row's `NoMatch` hides a later row's
///   real match**. To allow multi-row replies, this condition must be narrowed to success outcomes (`Closed`/`ClosedHistoryAnomaly`) —
///   then `NoMatch` must be left as "not yet closed".
/// ★Logging is outside the lock★: only the result is carried out and the caller logs it via `log_reply_close` (no tracing while holding the lock).
// ADR-0116 (decision 2) / ADR-0118 (decisions 1·2) / review fix D2
struct ReplyClosing<'a> {
    /// `Some` = this send is a reply (that contract id). If `None`, this device is a complete no-op.
    in_reply_to: Option<&'a str>,
    /// The only axis that picks the contract (ADR-0309 — no name-fallback comparison).
    replier_id: PeerId,
    /// If already closed, that result (for logging outside the lock + blocking a re-close).
    done: Option<ReplyOutcome>,
}

impl ReplyClosing<'_> {
    fn close_in_lock(&mut self, st: &mut MessagingState) {
        let Some(in_reply_to) = self.in_reply_to else {
            return;
        };
        if self.done.is_some() {
            return;
        }
        self.done = Some(
            st.ledger
                .close_on_reply(in_reply_to, self.replier_id, Instant::now()),
        );
    }
}

/// ★Logging the reply contract close result (**only outside the lock** — module header discipline)★ — depending on the result only logging differs (no effect on
/// delivery or the response, spec §3 item 7-②).
///
/// - `Closed` = normal (that replier's first valid reply). Other recipients' contracts stay open (there is no reply-all).
/// - `ClosedHistoryAnomaly` = the contract closed but the history could not take the `Delivered → Replied` edge (e.g. the reply was processed before the
///   original so the original is still `pending`). The contract's canonical source is tracking, so it is left as is and only **observed**.
/// - `NoMatch`/`AlreadyClosed` = wrong id · not my contract · already closed. This is a **normal path** (the reply message itself was already
///   delivered/parked) — no new error is made for the sender: it is a state that cannot be fixed, and bouncing would trigger a retry for an already-delivered
///   message and cause a duplicate.
fn log_reply_close(outcome: ReplyOutcome, in_reply_to: &str, replier_name: &str) {
    match outcome {
        ReplyOutcome::Closed => {}
        ReplyOutcome::ClosedHistoryAnomaly { from } => tracing::warn!(
            in_reply_to,
            history_status = ?from,
            "회신으로 계약은 닫혔으나 이력이 Replied 로 전이 못 함(불법 간선) — 계약 정본은 추적, 이력만 미반영(ADR-0103)"
        ),
        ReplyOutcome::NoMatch => tracing::debug!(
            in_reply_to,
            replier = %replier_name,
            "reply_to 가 이 회신자의 오픈된 request 를 가리키지 않음 — 메시지는 정상 배달, 닫힌 계약 없음(spec §3 항목 7-②. 계약은 회신자 id 로만 고른다 — ADR-0309)"
        ),
        ReplyOutcome::AlreadyClosed => tracing::debug!(
            in_reply_to,
            replier = %replier_name,
            "이미 닫힌 계약에 대한 추가 회신 — 메시지는 정상 배달, no-op(spec §3 항목 7-③)"
        ),
    }
}

/// ★One terminal transition that **could not be left in the ledger** (C4 review fix J · round-5 finding 2)★ — carries the fact that the record had already
///   been pushed out of the history ring (`TransitionError::NotFound`) so the transition was impossible, **together with the state it intended to stamp**.
///
/// ★Why it carries `intended` (finding 2 — load-bearing)★: this collector was at first expiry (`expired`) only, but
///   later the `skipped` transition (notice lane retirement) also came to go into the same bin. Without carrying the state, the log
///   is flattened to all "expired" — when the record is gone, **the only audit evidence left** would state a false cause.
///   So the state used in the transition is sent as is (the transition and the log use **one same value** — no room to diverge).
#[derive(Debug, Clone, PartialEq, Eq)]
struct EvictedTransition {
    msg_id: String,
    recipient: PeerId,
    intended: DeliveryStatus,
}

/// One terminal transition + collecting the evict fact (**single point** — the intended state cannot diverge between transition and log).
///
/// `Illegal` is ignored as best-effort (a ledger graph violation is an upper-layer bug but there is no reason to block delivery).
/// Only `NotFound` is collected and the caller logs it **outside the lock** (no tracing while holding the lock — module header discipline).
fn transition_or_collect_evicted(
    ledger: &mut Ledger,
    msg_id: &str,
    recipient: PeerId,
    intended: DeliveryStatus,
    now: Instant,
    evicted: &mut Vec<EvictedTransition>,
) {
    if let Err(TransitionError::NotFound) = ledger.transition(msg_id, recipient, intended, now) {
        evicted.push(EvictedTransition {
            msg_id: msg_id.to_string(),
            recipient,
            intended,
        });
    }
}

/// Logs the collected evict facts — called **only outside the lock** (module header discipline).
fn log_evicted_transitions(evicted: &[EvictedTransition]) {
    for e in evicted {
        tracing::debug!(
            msg_id = %e.msg_id,
            recipient = %e.recipient,
            intended_status = ?e.intended,
            "record evicted before terminal transition — 종점(만료/회수)은 실제로 일어났으나 이력 링버퍼에서 이미 밀려나 장부에 남길 레코드가 없음(ledger HISTORY_CAPACITY)"
        );
    }
}

/// ★The single-exit settlement guard of the in-flight receipt (F1 · load-bearing)★ — it always lowers the cap denominator raised with
///   `Mailbox::take_in_flight` **whichever path flush exits by**.
///
/// ★Why a stack guard★: the delivery loop has an early break per target and mixes in an external call (inject) in the middle.
///   If settlement is scattered over each branch, missing even one place inflates that recipient lane's denominator **permanently** (a leak) and the mail
///   to that id never gets in again — a failure mode harder to fix than silent loss. So it is flipped to "Drop repays what remains",
///   making per-branch settlement merely an **optimization** (fitting the denominator to the real outstanding count) and not a correctness condition.
/// ★Why a lock failure is swallowed in Drop★: it may be during unwinding, and then the messaging lock is already poisoned. Panicking
///   again there kills the process by double panic — and a poisoned lock means this service is already dead
///   (every path panics via `expect`), so there is no real benefit in restoring the denominator.
/// ★No settlement is missed, but **temporary over-counting** exists (explicit contract)★: delivery-complete and restore branches detach on the spot,
///   but in the span between, the denominator is higher by the number of items out — that is exactly the window F1 aims to block, so it is an intended value.
struct FlightSettle<'a> {
    svc: &'a MessagingService,
    recipient: PeerId,
    owed: FlightTicket,
    /// The **resolved target** id this batch holds — released together by the same guard as the recipient-id-axis receipt.
    ///   ★If leaked, that session never receives mail again★ (every later drain backs off on the target axis).
    // ADR-0142
    targets: Vec<PeerId>,
}

impl FlightSettle<'_> {
    /// Releases the held targets and returns those ids — **idempotent** (the second call gets an empty Vec so Drop does not release
    ///   twice). The reason it is called explicitly before echoing is canonical in the `drain_queue` step 6 comment.
    fn release_targets(&mut self) -> Vec<PeerId> {
        if self.targets.is_empty() {
            return Vec::new();
        }
        // ★Take the list out **after** taking the lock (the order is load-bearing)★: taking it first means that if lock acquisition fails the id has
        //   already vanished from `self` and **Drop's belt cannot see what to recover** — it hardens into a state where that session never receives mail
        //   again. And here poison is not swallowed (this file's convention for the normal path = `expect`).
        //   The swallowing idiom belongs to `Drop`, which may be unwinding, not to the normal path.
        let mut st = self.svc.state.lock().expect("messaging state poisoned");
        let targets = std::mem::take(&mut self.targets);
        for t in &targets {
            st.in_flight_targets.remove(t);
        }
        targets
    }
}

impl Drop for FlightSettle<'_> {
    fn drop(&mut self) {
        // ★Target release comes before receipt settlement★: if unwinding, this is the only release point, and both
        //   finish with one lock acquisition.
        let targets = std::mem::take(&mut self.targets);
        if targets.is_empty() && self.owed.is_zero() {
            return;
        }
        if let Ok(mut st) = self.svc.state.lock() {
            for t in &targets {
                st.in_flight_targets.remove(t);
            }
            if !self.owed.is_zero() {
                st.mailbox.settle_in_flight(self.recipient, self.owed);
            }
        }
    }
}

/// ★The **out-of-lock facts** one park left (round-6)★ — pressure reclaim happens inside the lock but logging must be outside the lock
///   (module header discipline). So the caller carries it in accumulator form and calls `log` after releasing the lock.
///
/// ★Count the branches **separately**★: because the reclaim reasons differ (notice = pushed out by a newer notice / TTL exceeded = the clock
///   decided its fate first). Merged into one field the log blurs the reason and invites misdiagnosis in operation, and especially the TTL branch has
///   **a different ledger vocabulary itself** (`expired` — F3).
#[derive(Debug, Default)]
struct ParkSideEffects {
    retired_notices: Vec<String>,
    retired_expired: Vec<String>,
    evicted: Vec<EvictedTransition>,
}

impl ParkSideEffects {
    fn log(&self, recipient: &str) {
        if !self.retired_expired.is_empty() {
            tracing::debug!(
                recipient,
                retired = self.retired_expired.len(),
                msg_ids = ?self.retired_expired,
                "park retire: 회수 시점에 이미 TTL(mailbox PARK_TTL) 초과 — sweep(60s 주기)이 아직 안 걷었을 뿐이라 장부 어휘는 skipped 가 아니라 expired(spec §5 · F3)"
            );
        }
        if !self.retired_notices.is_empty() {
            tracing::debug!(
                recipient,
                retired = self.retired_notices.len(),
                msg_ids = ?self.retired_notices,
                "park retire: notice 레인 상한(mailbox NOTICE_CAP) 초과 — 가장 오래된 통지를 회수하고 장부 skipped(신규 통지는 항상 수용 — round-6)"
            );
        }
        log_evicted_transitions(&self.evicted);
    }
}

/// `ParkRequest` → one stored item. Gathering the assembly in one place keeps the envelope payload and TTL reference time from diverging per insert
/// path (since the old seat-restore path disappeared the caller is just `park_into` — ADR-0125).
fn build_parked(
    req: &ParkRequest<'_>,
    payload: ParkPayload,
    now: Instant,
    admission_seq: u64,
) -> ParkedMessage {
    ParkedMessage {
        msg_id: req.msg_id.to_string(),
        envelope: payload.encode(),
        kind: req.kind,
        parked_at: now,
        admission_seq,
    }
}

/// ★The **in-lock core** of parking + recording ledger `pending` (shared by single send and group fan-out)★.
///
/// ★Why it was extracted as a free function★: group fan-out must finish the decision·parking of N members in **one lock span**
///   (`handle_send` lock discipline), and calling the wrapper that takes the lock itself (`park_notice`) as is would take and release the lock
///   per recipient, opening a "queue looks empty window". So the core that can be called **with the caller holding the lock** is
///   separated, and `park_notice` for notices remains as a thin wrapper around it (keeping one set of parking rules —
///   no silent loss · payload encoding · ledger recording must not diverge across the two paths).
/// ★No silent loss (ADR-0103)★: when park succeeds it always leaves `pending` in the ledger. A cap-exceeded bounce does not store
///   at all, so it leaves nothing in the ledger either (the caller makes it visible as a bounce/skip).
/// ★Ledgering of pressure reclaim (round-6 · F3 vocabulary correction)★: the item this park swept out to make room
///   (`ParkAdmitted.retired` — the undeliverable debris of the message lane or an old notice of the notice lane) is moved **here** to a ledger
///   terminal — the store is pure and does not know the ledger, and this boundary is the only point that knows that fact. Sweeping out
///   without fixing the ledger leaves a ghost pending (not in the queue yet a lookup says "waiting for delivery"). The logging material per reason is
///   gathered separately into `effects` and logged **outside the lock** (module header discipline).
/// ★The vocabulary differs per item — TTL takes priority over `skipped` (F3)★: the default is `skipped` ("not delivered to that recipient"),
///   but because of the gap between the sweep period (60s) and the TTL (24h), an item that **already passed the TTL but was not yet collected** may
///   be reclaimed. By the spec §5 contract that is `expired` (the clock decided that item's fate first, and the reclaim merely discovered that
///   fact late). The decision reuses mailbox's `ParkedMessage::is_expired(now)` as is —
///   the TTL constant and the `>=` boundary convention are kept in one place in the store, so the literal is not duplicated.
// ADR-0107 (reclaimed items' ledger terminal — skipped/expired classification)
fn park_into(
    st: &mut MessagingState,
    req: ParkRequest<'_>,
    now: Instant,
    effects: &mut ParkSideEffects,
) -> Result<(), ParkError> {
    let payload = ParkPayload {
        sender_name: req.sender_name.to_string(),
        from: req.from,
        entrance: req.entrance,
        body: req.body.to_string(),
        meta: req.meta.clone(),
    };
    // The admission sequence number is assigned by `park` at acceptance time (the store is the only assigner — mailbox comment). The value here is
    //   ignored, so it is a placeholder.
    let parked = build_parked(&req, payload, now, 0);
    let admitted = st.mailbox.park(req.recipient, parked)?;
    for m in admitted.retired {
        let expired = m.is_expired(now);
        let intended = if expired {
            DeliveryStatus::Expired
        } else {
            DeliveryStatus::Skipped
        };
        transition_or_collect_evicted(
            &mut st.ledger,
            &m.msg_id,
            req.recipient,
            intended,
            now,
            &mut effects.evicted,
        );
        match (expired, m.kind) {
            (true, _) => effects.retired_expired.push(m.msg_id),
            // ADR-0114: pressure reclaim of the message lane was abolished — only notice retirement comes through this channel.
            //   The branch is still kept so that when the store someday emits another reclaim reason it is not silently flattened.
            (false, ParkKind::Message) => effects.retired_notices.push(m.msg_id),
            (false, ParkKind::Notice) => effects.retired_notices.push(m.msg_id),
        }
    }
    st.ledger.record_with_expected(
        req.msg_id,
        req.sender_name,
        req.from.peer_id,
        req.recipient_name,
        Some(req.recipient),
        req.body,
        DeliveryStatus::Pending,
        now,
        req.expected_rows,
    );
    Ok(())
}

/// ★Encoding (version header introduced in C3 review fix 4)★:
///   `<ver>\n<sender_len>\n<reply_by_len>\n<reply_to_len>\n<group_len>\n<from_agent_id>\n<from_epoch>\n`
///   `<entrance>\n<flags>\n<sender><reply_by><reply_to><group><body>`
///   The first 9 lines are fields without newlines (numbers/uuid/short literals) so they are safely split by newline, and the first 4 variable strings are bounded by a **length
///   prefix** (body is everything remaining — safe even if newlines appear in body·reply_to, and reply_to is
///   agent input so it can be an arbitrary string). Length 0 = `None` (an empty string `Some("")` is already bounced by entrance
///   validation, so it is not ambiguous).
///
/// ★Why a version tag (fix 4)★: this format is used only **inside the process** (parking is in-memory so it vanishes on daemon restart), but
///   the moment the format changes, an old payload meeting the new decode can be silently misinterpreted (a length field pushed to a different
///   position and body cut off). With a 1-character tag, **an unknown version = immediate fallback** forks the path, and instead of misinterpretation it fails as
///   "envelope attributes lost and only body left" (a visible degradation). If v2 persistence stores parking on disk,
///   this tag becomes the migration branch point.
/// ★If you change the layout, bump this value too★: it is a format used only within the same process so an old payload cannot really exist,
///   but leaving the tag as is makes a state where **the layout changed but the version is the same**, and the tag's meaning (= the
///   layout contract of this string) collapses.
const PARK_PAYLOAD_VERSION: &str = "3";

/// Parking payload — the minimum meta needed at flush injection time to assemble the envelope and emit the observation record (sender name · sender
///   identity · entrance · raw body). An internal encoding to carry the raw value around the ParkedMessage.envelope contract ("finished envelope").
///   At flush injection time it decodes → wrap_now assembles the envelope in the **current** format (the single wrap point), and emits the
///   delivery observation record with the sender identity (appearance delivery is observed the same as the send path — ADR-0088).
///
/// ★Why from/entrance are carried too★: parking→flush auto-delivery is also a target of delivery-boundary observation (ADR-0088) — the harness must
///   also recover "who was delivered to whom" on the flush path (park→spawn→auto-delivery acceptance, spec §7).
///   For that, the original sender identity (SenderIdentity) and entrance must be preserved until flush time.
/// ★Why meta (the reply contract) is carried too (C3 · load-bearing)★: when a parked request/reply is delivered late it must also carry **the same envelope
///   attributes as immediate delivery** (id/type/reply-by/in-reply-to). If not carried, a request that went through parking arrives as a
///   plain message without attributes, the receiving LLM does not know the id to reply to, and only the sender receives the overdue notice
///   (the worst mode where the contract silently breaks). The materials are carried as the cost of the design that does not harden the envelope at park time.
struct ParkPayload {
    sender_name: String,
    from: SenderIdentity,
    entrance: Entrance,
    body: String,
    /// C3 reply-contract meta + C4 group label (the parsed Duration is **not restored** — the contract was already opened in the
    ///   ledger at send time, and all flush needs is the envelope attributes). So only the raw notation of `reply_by` survives.
    meta: SendMeta,
}

impl ParkPayload {
    fn encode(&self) -> String {
        let ent = match self.entrance {
            Entrance::Mcp => "mcp",
            Entrance::Cli => "cli",
            Entrance::Daemon => "daemon",
        };
        let reply_by = self.meta.reply_by_raw.as_deref().unwrap_or("");
        let reply_to = self.meta.reply_to.as_deref().unwrap_or("");
        // The envelope `to` (frozen value) — carried together so a parked item delivered late does not lose the send-moment notation (spec §1).
        let group = self.meta.to_attr.as_deref().unwrap_or("");
        let flags = if self.meta.request { "r" } else { "-" };
        format!(
            "{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}{}{}{}{}",
            PARK_PAYLOAD_VERSION,
            self.sender_name.len(),
            reply_by.len(),
            reply_to.len(),
            group.len(),
            self.from.peer_id,
            self.from.epoch,
            ent,
            flags,
            self.sender_name,
            reply_by,
            reply_to,
            group,
            self.body
        )
    }

    /// The inverse of encode. If the format is broken (impossible — our own encoding) it falls back to treating the whole thing as body (better than silent loss).
    ///
    /// ★Never panics (fix 4 · load-bearing)★: this function is called **per item inside** the flush batch loop —
    ///   a panic here throws away the rest of that batch (the production daemon ends the whole daemon via the hook — ADR-0292) and
    ///   one broken item drags down healthy messages. So every failure path ends in an **item-level fallback**:
    ///   if a length field is false (the sum exceeds the remaining length) or **does not split on a char boundary** (cut in the middle of a multibyte char —
    ///   the very case where `split_at` panics) or the version is an unknown value, the envelope attributes are lost but the original is sent on as body.
    ///   Thanks to this, the other items in the batch are delivered normally (test: one corrupt item does not abort the batch).
    fn decode(s: &str) -> Self {
        // splitn(10) — the last piece is not cut so newlines inside body are preserved.
        let mut it = s.splitn(10, '\n');
        let fallback = || Self {
            sender_name: String::new(),
            from: SenderIdentity {
                peer_id: PeerId::nil(),
                epoch: 0,
            },
            entrance: Entrance::Cli,
            body: s.to_string(),
            meta: SendMeta::default(),
        };
        let (
            Some(ver),
            Some(sender_len),
            Some(rb_len),
            Some(rt_len),
            Some(grp_len),
            Some(id_str),
            Some(ep_str),
            Some(ent_str),
            Some(flags),
            Some(rest),
        ) = (
            it.next(),
            it.next(),
            it.next(),
            it.next(),
            it.next(),
            it.next(),
            it.next(),
            it.next(),
            it.next(),
            it.next(),
        )
        else {
            return fallback();
        };
        if ver != PARK_PAYLOAD_VERSION {
            return fallback();
        }
        // ★Enums are parsed **strictly** (round-final fix 3 · load-bearing)★: it used to be that an unknown entrance string silently fell to
        //   `Cli` and an unknown flag silently to "not a request". That is the same kind of damage as a broken length field, but it means it has
        //   **a failure mode of its own** (arbitrary reinterpretation, not a fallback), so something that should go out as `<message>` is recorded as a different entrance
        //   by observation or arrives as plain having lost the request envelope (id/reply-by) — the latter is a contract break where the receiving LLM does not know
        //   the id to reply to. A value outside the vocabulary is evidence of damage, so it goes to the **same item-level fallback** as length damage
        //   (no silent reinterpretation).
        let (Some(entrance), Some(request)) = (parse_entrance(ent_str), parse_request_flag(flags))
        else {
            return fallback();
        };
        let (Ok(sender_len), Ok(rb_len), Ok(rt_len), Ok(grp_len), Ok(agent_id), Ok(epoch)) = (
            sender_len.parse::<usize>(),
            rb_len.parse::<usize>(),
            rt_len.parse::<usize>(),
            grp_len.parse::<usize>(),
            id_str.parse::<PeerId>(),
            ep_str.parse::<u32>(),
        ) else {
            return fallback();
        };
        let Some(total) = sender_len
            .checked_add(rb_len)
            .and_then(|v| v.checked_add(rt_len))
            .and_then(|v| v.checked_add(grp_len))
        else {
            return fallback();
        };
        if rest.len() < total {
            return fallback();
        }
        let cut1 = sender_len;
        let cut2 = cut1 + rb_len;
        let cut3 = cut2 + rt_len;
        let cut4 = cut3 + grp_len;
        if !rest.is_char_boundary(cut1)
            || !rest.is_char_boundary(cut2)
            || !rest.is_char_boundary(cut3)
            || !rest.is_char_boundary(cut4)
        {
            return fallback();
        }
        let (sender, tail) = rest.split_at(cut1);
        let (reply_by, tail) = tail.split_at(rb_len);
        let (reply_to, tail) = tail.split_at(rt_len);
        let (group, body) = tail.split_at(grp_len);
        Self {
            sender_name: sender.to_string(),
            from: SenderIdentity {
                peer_id: agent_id,
                epoch,
            },
            entrance,
            body: body.to_string(),
            meta: SendMeta {
                request,
                reply_by_raw: (!reply_by.is_empty()).then(|| reply_by.to_string()),
                reply_by: None,
                reply_to: (!reply_to.is_empty()).then(|| reply_to.to_string()),
                to_attr: (!group.is_empty()).then(|| group.to_string()),
            },
        }
    }
}

/// It pairs with the literal in `encode`, so when entrance kinds increase fix both together (if they diverge a normal payload falls back and the
///   round-trip test catches it).
fn parse_entrance(s: &str) -> Option<Entrance> {
    match s {
        "mcp" => Some(Entrance::Mcp),
        "cli" => Some(Entrance::Cli),
        "daemon" => Some(Entrance::Daemon),
        _ => None,
    }
}

fn parse_request_flag(s: &str) -> Option<bool> {
    match s {
        "r" => Some(true),
        "-" => Some(false),
        _ => None,
    }
}

/// ★The identity of a daemon self-send (C3)★ — a `<notice>` has no sending agent. The observation record (`DeliveryObservation.from`)
///   requires an identity, so the **nil PeerId** is used as the daemon-origin marker (it overlaps with no real agent).
///   Paired with `Entrance::Daemon`, it lets "this is an infrastructure notice" be told from the record alone.
fn daemon_identity() -> SenderIdentity {
    SenderIdentity {
        peer_id: PeerId::nil(),
        epoch: 0,
    }
}

/// The `draw_daemon_msg_id` result — the selected id + a collision trace for observation (logging is done by the caller **outside the lock** — module header discipline).
struct DrawnMsgId {
    /// The id actually to use (it passed the check, or the last value caught even after the re-draw).
    id: String,
    /// If the first draw collided with the ledger and was discarded, that value — the investigation clue is on **the side that collided** (the same discipline as the ingress retry log:
    ///   keeping only the replacement id leaves no way to trace what it collided with).
    collided: Option<String>,
    /// Did **even** the re-drawn id collide. true = a signal to suspect random-number/ledger wiring, but the notice is sent out anyway.
    still_colliding: bool,
}

// ★The duration-notation reverse mapping (duration_notation) was removed (C3 review fix 6)★: restoring `Duration` → notation is normalization, so
//   a deadline sent as `"60m"` was announced as `"1h"` and the envelope attribute (`reply-by="60m"`) and the wording diverged. Now the ledger
//   keeps the sender notation as the original (ledger.rs `RequestEntry.reply_by`) and the notice uses it as is — if a reverse-mapping function
//   appears again, the same mismatch recurs.

// ★Removed (C4 — ADR-0104 anchor resolution)★: the old `transition_expired_by_msg_id` looked up sweep-expired items in the
//   ledger by msg_id alone and transitioned the "first pending record" to expired. That choice is right **only for a single recipient** —
//   a group broadcast has N per-recipient records under one msg_id so it can expire the wrong member, and above all the structure leaned its correctness on
//   a **coincidental correlation** between history insertion order and expiry order. Now the mailbox returns the queue key with each expired item
//   (`ExpiredParked`), so `sweep` targets `(msg_id, recipient)` directly. Do not revive this helper —
//   a msg_id-only reverse lookup is structurally incompatible with a 1:N ledger.
// ADR-0104

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tid;
    use std::sync::Mutex as StdMutex;
    use std::time::Duration;

    /// Why delivery observation is a no-op — the production daemon also has no observer installed, and the verification that asserts observation records is
    ///   done by the daemon integration harness (ADR-0088 — outside this unit test's scope).
    struct FakeControlPlane;
    impl ControlPlanePort for FakeControlPlane {
        fn envelope_format(&self) -> EnvelopeFormat {
            EnvelopeFormat::default()
        }
        fn record_delivery(&self, _obs: DeliveryObservation) {}
    }

    struct FakeDeliveryPort {
        /// ★Why an Arc★: the test must be able to clone the handle and swap the roster **during inject** (inside the on_inject hook) —
        ///   the only way to reproduce deterministically the TOCTOU "the recipient restarted after the roster snapshot and before the write"
        ///   (capturing the port itself in the hook would make a self-referential cycle).
        roster: Arc<StdMutex<Vec<LiveAgent>>>,
        injected: StdMutex<Vec<(PeerId, Vec<u8>)>>,
        call_count: StdMutex<usize>,
        fail_at_call: StdMutex<Vec<usize>>,
        /// ★Why an Arc, not a Box (round-7)★: **injection can happen again** inside the hook (e.g. a deferred
        ///   flush echoes and delivers on the spot). Calling a Box under a lock guard makes that re-entry hit this mutex and
        ///   **deadlock** (std Mutex is non-reentrant), so it is changed to a cloneable Arc and called with the lock released.
        on_inject: StdMutex<Option<Arc<dyn Fn(usize) + Send + Sync>>>,
        /// The live_agents lookup count — the axis the "one snapshot per send" (ADR-0111 decision 2) regression counts (fix 8).
        roster_calls: StdMutex<usize>,
        /// Once set, it returns this roster from **after the first lookup** — for reproducing finding 3 TOCTOU self-heal.
        roster_after_first: StdMutex<Option<Vec<LiveAgent>>>,
        /// ★One-shot roster lookup hook (reproducing the fix 5 race)★ — it is used once and emptied, so even if a send is made again inside the hook
        ///   it does not recurse.
        on_roster: StdMutex<Option<Box<dyn Fn() + Send>>>,
        /// ★The asleep (id, name) list (profile exists · no session — spec §5 branch 3)★. **Duplicates are not folded** (two of the same name
        ///   = same name in the asleep layer).
        dormant: StdMutex<Vec<DormantAgent>>,
        /// ★The ids for which `is_agent_live` returns true (the delete-cleanup gate scenario — review fix D1)★. It simulates cases **not caught by name**,
        ///   such as a session alive in terminal mode or a renamed profile.
        ///
        /// ★Scripted **separately** from the roster (intended)★: the gate asks with that narrow verb, so deriving it from the roster would make the fake
        ///   build the wiring "the gate looks at the roster" on its own, and the test could not see the real wiring.
        live_ids: StdMutex<Vec<PeerId>>,
    }

    impl FakeDeliveryPort {
        fn new() -> Self {
            Self {
                roster: Arc::new(StdMutex::new(Vec::new())),
                injected: StdMutex::new(Vec::new()),
                call_count: StdMutex::new(0),
                fail_at_call: StdMutex::new(Vec::new()),
                on_inject: StdMutex::new(None),
                roster_calls: StdMutex::new(0),
                roster_after_first: StdMutex::new(None),
                on_roster: StdMutex::new(None),
                dormant: StdMutex::new(Vec::new()),
                live_ids: StdMutex::new(Vec::new()),
            }
        }
        /// Puts `tid(이름)` (tid of the name) as the id per name — launching `live` with the same name means that agent woke up.
        fn set_dormant(&self, names: &[&str]) {
            *self.dormant.lock().unwrap() = names
                .iter()
                .map(|n| DormantAgent {
                    id: tid(n),
                    name: n.to_string(),
                })
                .collect();
        }
        /// For when the id must be chosen directly, like same-name twins.
        fn set_dormant_agents(&self, agents: &[(PeerId, &str)]) {
            *self.dormant.lock().unwrap() = agents
                .iter()
                .map(|(id, n)| DormantAgent {
                    id: *id,
                    name: n.to_string(),
                })
                .collect();
        }
        fn set_live_ids(&self, ids: &[PeerId]) {
            *self.live_ids.lock().unwrap() = ids.to_vec();
        }
        fn arm_on_next_roster(&self, f: Box<dyn Fn() + Send>) {
            *self.on_roster.lock().unwrap() = Some(f);
        }
        fn roster_call_count(&self) -> usize {
            *self.roster_calls.lock().unwrap()
        }
        fn set_roster(&self, roster: Vec<LiveAgent>) {
            *self.roster.lock().unwrap() = roster;
        }
        /// Unlike `live_agents` it does not touch the call counter or hook — it is a decision at the injection boundary, not a roster
        ///   **lookup** (the production implementation also looks directly at the session map and does not re-take the roster).
        fn effective_roster(&self) -> Vec<LiveAgent> {
            match self.roster_after_first.lock().unwrap().as_ref() {
                Some(r) => r.clone(),
                None => self.roster.lock().unwrap().clone(),
            }
        }
        /// Returns Err at the given inject call indexes (0-based).
        fn fail_at(&self, indices: &[usize]) {
            *self.fail_at_call.lock().unwrap() = indices.to_vec();
        }
        fn set_on_inject(&self, f: Arc<dyn Fn(usize) + Send + Sync>) {
            *self.on_inject.lock().unwrap() = Some(f);
        }
        fn roster_calls(&self) -> usize {
            *self.roster_calls.lock().unwrap()
        }
        fn injected_bodies(&self) -> Vec<String> {
            self.injected
                .lock()
                .unwrap()
                .iter()
                .map(|(_, b)| String::from_utf8_lossy(b).to_string())
                .collect()
        }
        fn injected_targets(&self) -> Vec<PeerId> {
            self.injected
                .lock()
                .unwrap()
                .iter()
                .map(|(id, _)| *id)
                .collect()
        }
    }

    impl FakeDeliveryPort {
        fn inject_inner(
            &self,
            to_id: PeerId,
            expected_epoch: Option<u32>,
            bytes: &[u8],
        ) -> Result<InjectReceipt, String> {
            let idx = {
                let mut c = self.call_count.lock().unwrap();
                let i = *c;
                *c += 1;
                i
            };
            let hook = self.on_inject.lock().unwrap().clone();
            if let Some(f) = hook {
                f(idx);
            }
            if let Some(expected) = expected_epoch {
                let now_epoch = self
                    .effective_roster()
                    .iter()
                    .find(|a| a.id == to_id)
                    .map(|a| a.epoch);
                if now_epoch != Some(expected) {
                    return Err(format!(
                        "epoch mismatch: agent {to_id} is now at epoch {now_epoch:?}, caller required {expected} — nothing was written"
                    ));
                }
            }
            if self.fail_at_call.lock().unwrap().contains(&idx) {
                return Err("fake inject fail".to_string());
            }
            self.injected.lock().unwrap().push((to_id, bytes.to_vec()));
            Ok(InjectReceipt {
                bytes_requested: bytes.len(),
                bytes_written: bytes.len(),
                msg_uuid: uuid::Uuid::new_v4(),
                epoch: expected_epoch.unwrap_or(0),
            })
        }
    }

    impl DeliveryPort for FakeDeliveryPort {
        fn inject(&self, to_id: PeerId, bytes: &[u8]) -> Result<InjectReceipt, String> {
            self.inject_inner(to_id, None, bytes)
        }

        fn live_agents(&self) -> Vec<LiveAgent> {
            let call = {
                let mut c = self.roster_calls.lock().unwrap();
                let i = *c;
                *c += 1;
                i
            };
            let hook = self.on_roster.lock().unwrap().take();
            if let Some(f) = hook {
                f();
            }
            if call >= 1 {
                if let Some(r) = self.roster_after_first.lock().unwrap().as_ref() {
                    return r.clone();
                }
            }
            self.roster.lock().unwrap().clone()
        }
        /// ★The entrance decision-source snapshot (ADR-0116)★ — simulates **the same relationship** as the production adapter: `roster` is
        ///   **the same value** as `live_agents()` (predicate = state only) and `dormant` is a separate source.
        ///   Reading `self.roster` directly would not raise the roster lookup counter and would blind the "one snapshot per send" regression.
        fn addressing_sources(&self) -> AddressingSources {
            AddressingSources {
                roster: self.live_agents(),
                dormant: self.dormant.lock().unwrap().clone(),
            }
        }

        fn is_agent_live(&self, id: PeerId) -> bool {
            self.live_ids.lock().unwrap().contains(&id)
        }
    }

    /// ★Replier identity (A6)★ — a reply must be sent with **that recipient's own identity** to close its own contract (matching is id first —
    ///   ledger `close_on_reply`). The old tests replied with a fixed `ident()` and leaned on name matching, and that leniency was
    ///   a path that closed someone else's contract.
    fn reply_from(id: PeerId) -> SenderIdentity {
        SenderIdentity {
            peer_id: id,
            epoch: 0,
        }
    }

    /// The sender must also be in the roster for the `@all` exclusion rule to apply.
    fn live_sender(name: &str) -> (SenderIdentity, LiveAgent) {
        let (id, agent) = live(name);
        (
            SenderIdentity {
                peer_id: id,
                epoch: agent.epoch,
            },
            agent,
        )
    }

    /// ★Test-only single-recipient shim (preserving the regression net of the ADR-0111 overhaul)★ — the new entry point is only the multi-recipient
    ///   `handle_send`, but most of this file's existing regression tests look only at the "1 recipient" axis (parking · flush · gate ·
    ///   contract). To not throw that asset away, it folds the 1-row response into the old outcome vocabulary.
    ///
    /// ★What the shim does not hide★: absence · duplicate name · mailbox full are now **row failures** so they come out as `Err(SendFail::…)` —
    ///   that is, a test that expected "park if absent" **still fails even using this shim** (the overhaul gets caught in the net).
    #[derive(Debug, Clone, PartialEq, Eq)]
    enum SendOutcome {
        Delivered,
        Parked { hint: String },
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum SendFail {
        Row(FailCode),
        Reject(SendReject),
    }

    impl MessagingService {
        #[allow(clippy::too_many_arguments)]
        fn handle_single_send(
            &self,
            msg_id: &str,
            from: SenderIdentity,
            sender_name: &str,
            to: &str,
            body: &str,
            entrance: Entrance,
            meta: &SendMeta,
        ) -> Result<SendOutcome, SendFail> {
            let rows = self
                .handle_send(
                    msg_id,
                    from,
                    sender_name,
                    &[to.to_string()],
                    body,
                    entrance,
                    meta,
                )
                .map_err(SendFail::Reject)?;
            assert_eq!(rows.len(), 1, "단일 수신자 발송은 행이 정확히 1개다");
            let row = &rows[0];
            match row.status {
                SendStatus::Delivered => Ok(SendOutcome::Delivered),
                SendStatus::Pending => Ok(SendOutcome::Parked {
                    hint: row.hint.clone().unwrap_or_default(),
                }),
                SendStatus::Failed => Err(SendFail::Row(row.code.expect("실패 행은 code 필수"))),
            }
        }
    }

    impl MessagingService {
        /// ★Legacy parking seam (for the ADR-0111 overhaul · test-only)★ — creates the **state** "there is mail in the queue addressed to that name". Pre-overhaul
        ///   tests made that state by **absent parking**, but absence is now a failure row
        ///   (ADR-0111 decision 1) so that entry road disappeared. The **subject of those tests is not parking entry but the queue's behavior**
        ///   (flush order · gate · TTL · re-parking · in-flight accounting), so only the entry is replaced and every path after it
        ///   runs the real code.
        ///
        /// ★This seam does not obscure the "absence is failure" contract★: the regression net for that contract is carried by the newly added tests that call `handle_send`
        ///   directly (absence → failure row + no parking record).
        #[allow(clippy::too_many_arguments)]
        fn park_absent_for_test(
            &self,
            msg_id: &str,
            from: SenderIdentity,
            sender_name: &str,
            to: &str,
            body: &str,
            entrance: Entrance,
            meta: &SendMeta,
        ) -> Result<SendOutcome, SendFail> {
            // It first goes through **the same wiring guard** as the production entry point (so the bypass path does not loosen that contract).
            debug_assert!(
                !(meta.request && meta.reply_to.is_some()),
                "ingress가 유일 검증자 — request와 reply_to는 상호배타(spec §6)"
            );
            debug_assert!(
                meta.to_attr.is_none(),
                "봉투 to 는 전 수신자 수용 판정 뒤 handle_send 가 1회 확정한다(spec §1)"
            );
            // ★Look at the roster **first** (load-bearing)★: if it ran the production path and then looked at the failure row and reverted, the ledger terminal row (`failed`)
            //   left by that bounce would overlap with the queue item and a lookup would return 2 rows. Only when absence is certain
            //   does it bypass the entry.
            let live_only = AddressingSources {
                roster: self.port.live_agents(),
                dormant: Vec::new(),
            };
            let absent = !matches!(resolve_token(to, &live_only), Resolution::Live(_));
            match absent {
                true => {
                    // The id collision check is at the same place as production (the first side-effect point) — the bypass path is also a zero-side-effect bounce.
                    {
                        let st = self.state.lock().expect("messaging state poisoned");
                        if st.ledger.msg_id_in_use(msg_id) {
                            return Err(SendFail::Reject(SendReject::IdCollision));
                        }
                    }
                    // The old absent parking **opened a contract too** (spec §3 "opens once received, whether delivered or parked").
                    //   To keep alive the tests that look at that axis (overdue notice · outstanding lookup), it is opened here too.
                    if meta.request {
                        let mut st = self.state.lock().expect("messaging state poisoned");
                        let reply_by = meta.reply_by.zip(meta.reply_by_raw.clone());
                        // ★Always hand the retirement mark over to commit★: if not handed over, the marked victim stays in the list forever
                        //   (ledger `occupies_slot`) and the limit **never gets reached** — the cap fixture
                        //   becomes an infinite loop (measured). It is the same discipline as the production path (`commit_contract`).
                        // ★Receiving id = `tid(to)`★ — an agent later launched as `live(to)` with the same name is that
                        //   absent recipient itself (`live` doc). It is the same place as the asleep id on the production path.
                        let to_id = tid(to);
                        let retired = match st.ledger.open_request(
                            msg_id,
                            sender_name,
                            from.peer_id,
                            to,
                            to_id,
                            reply_by,
                            Instant::now(),
                        ) {
                            OpenOutcome::OpenedAfterMarking(rc) => Some(rc),
                            OpenOutcome::Full => {
                                return Err(SendFail::Row(FailCode::RequestCapacity))
                            }
                            _ => None,
                        };
                        st.ledger.commit_open(
                            Some((msg_id, to_id)),
                            retired
                                .as_ref()
                                .map(|r| (r.request_id.as_str(), r.recipient_id)),
                        );
                    }
                    let mut effects = ParkSideEffects::default();
                    {
                        let mut st = self.state.lock().expect("messaging state poisoned");
                        park_into(
                            &mut st,
                            ParkRequest {
                                msg_id,
                                sender_name,
                                from,
                                entrance,
                                recipient: tid(to),
                                recipient_name: to,
                                body,
                                kind: ParkKind::Message,
                                meta,
                                expected_rows: 1,
                            },
                            Instant::now(),
                            &mut effects,
                        )
                        .map_err(|_| SendFail::Row(FailCode::MailboxFull))?;
                    }
                    effects.log(to);
                    if let Some(in_reply_to) = &meta.reply_to {
                        self.close_reply_contract(in_reply_to, sender_name, from.peer_id);
                    }
                    Ok(SendOutcome::Parked {
                        hint: format!("(legacy park seam) queued for '{to}'"),
                    })
                }
                false => {
                    self.handle_single_send(msg_id, from, sender_name, to, body, entrance, meta)
                }
            }
        }
    }

    fn svc() -> (Arc<MessagingService>, Arc<FakeDeliveryPort>) {
        let port = Arc::new(FakeDeliveryPort::new());
        let registry = Arc::new(FakeControlPlane);
        let svc = Arc::new(MessagingService::new(port.clone(), registry));
        (svc, port)
    }

    fn ident() -> SenderIdentity {
        SenderIdentity {
            peer_id: PeerId::new_v4(),
            epoch: 0,
        }
    }

    /// A live agent — its id is `tid(name)`, so calling again with the same name is the **same agent** (restarted · woke from asleep).
    ///   For same-name agents that must be different entities, use `live_distinct` (ADR-0309 — the mail key is an id, so mixing the two makes the test
    ///   fold into one entity and pass vacuously).
    fn live(name: &str) -> (PeerId, LiveAgent) {
        live_with_id(tid(name), name)
    }

    /// A live agent — random id. For same-name twins · 「같은 이름의 다른 에이전트」 (a different agent with the same name) scenarios.
    fn live_distinct(name: &str) -> (PeerId, LiveAgent) {
        live_with_id(PeerId::new_v4(), name)
    }

    fn live_with_id(id: PeerId, name: &str) -> (PeerId, LiveAgent) {
        (
            id,
            LiveAgent {
                id,
                name: name.to_string(),
                epoch: 0,
                turn_signal: true,
            },
        )
    }

    /// The `live_distinct` version of `live_sender`.
    fn live_sender_distinct(name: &str) -> (SenderIdentity, LiveAgent) {
        let (id, agent) = live_distinct(name);
        (
            SenderIdentity {
                peer_id: id,
                epoch: agent.epoch,
            },
            agent,
        )
    }

    /// ★A live agent with no turn signal (terminal/console mode — ADR-0116 decision 7)★ — the class that is **in** the roster (membership is not a
    ///   condition) and is injected immediately with no gate.
    fn live_no_turn_signal(name: &str) -> (PeerId, LiveAgent) {
        let (id, mut a) = live(name);
        a.turn_signal = false;
        (id, a)
    }

    // ── C2 idle gate test harness ─────────────────────────────────────────────────
    struct FakeGate {
        busy: StdMutex<std::collections::HashSet<(PeerId, u32)>>,
        calls: StdMutex<usize>,
        idle_after_call: StdMutex<Option<usize>>,
    }
    impl FakeGate {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                busy: StdMutex::new(std::collections::HashSet::new()),
                calls: StdMutex::new(0),
                idle_after_call: StdMutex::new(None),
            })
        }
        fn set_busy(&self, id: PeerId, epoch: u32) {
            self.busy.lock().unwrap().insert((id, epoch));
        }
        fn clear(&self) {
            self.busy.lock().unwrap().clear();
        }
        /// Answers busy only until the first `n` lookups and idle after that (= the situation where the turn ended in the meantime).
        fn arm_idle_after_call(&self, n: usize) {
            *self.idle_after_call.lock().unwrap() = Some(n);
        }
        fn call_count(&self) -> usize {
            *self.calls.lock().unwrap()
        }
    }
    impl super::super::busy::BusyGate for FakeGate {
        fn is_busy(&self, id: PeerId, epoch: u32) -> bool {
            let idx = {
                let mut c = self.calls.lock().unwrap();
                let i = *c;
                *c += 1;
                i
            };
            if let Some(n) = *self.idle_after_call.lock().unwrap() {
                if idx >= n {
                    return false;
                }
            }
            self.busy.lock().unwrap().contains(&(id, epoch))
        }
    }

    /// ★Doorbell unwired (C2)★ = the inline flush fallback (FlushTrigger comment) — self-healing is synchronous so assertions are deterministic.
    ///   The doorbell wiring itself is verified by `svc_gated_with_doorbell`.
    fn svc_gated() -> (Arc<MessagingService>, Arc<FakeDeliveryPort>, Arc<FakeGate>) {
        let port = Arc::new(FakeDeliveryPort::new());
        let gate = FakeGate::new();
        let registry = Arc::new(FakeControlPlane);
        let svc = Arc::new(MessagingService::new_gated(
            port.clone(),
            registry,
            gate.clone(),
        ));
        (svc, port, gate)
    }

    /// Simulates the shape that **hands flush off to another thread** like the production assembly — nobody consumes here, so
    ///   "no injection happens on the sending thread" can be asserted.
    struct FakeTrigger {
        seen: StdMutex<Vec<PeerId>>,
    }
    impl FakeTrigger {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                seen: StdMutex::new(Vec::new()),
            })
        }
        fn seen(&self) -> Vec<PeerId> {
            self.seen.lock().unwrap().clone()
        }
        /// Per-section assertions — take out and empty so the surplus doorbells of earlier sections do not mix in.
        fn take(&self) -> Vec<PeerId> {
            std::mem::take(&mut *self.seen.lock().unwrap())
        }
    }
    impl FlushTrigger for FakeTrigger {
        fn request_flush(&self, id: PeerId) {
            self.seen.lock().unwrap().push(id);
        }
    }

    /// Gate + **doorbell** wired assembly (a production mirror — fix 11).
    fn svc_gated_with_doorbell() -> (
        Arc<MessagingService>,
        Arc<FakeDeliveryPort>,
        Arc<FakeGate>,
        Arc<FakeTrigger>,
    ) {
        let port = Arc::new(FakeDeliveryPort::new());
        let gate = FakeGate::new();
        let bell = FakeTrigger::new();
        let registry = Arc::new(FakeControlPlane);
        let svc = Arc::new(
            MessagingService::new_gated(port.clone(), registry, gate.clone())
                .with_flush_trigger(bell.clone()),
        );
        (svc, port, gate, bell)
    }

    #[test]
    fn deliver_ok_marks_delivered_and_injects_wrapped() {
        let (svc, port) = svc();
        let (_id, alice) = live("alice");
        port.set_roster(vec![alice]);
        let out = svc
            .park_absent_for_test(
                "m1",
                ident(),
                "bob",
                "alice",
                "hi",
                Entrance::Mcp,
                &SendMeta::default(),
            )
            .expect("no reject");
        assert_eq!(out, SendOutcome::Delivered);
        assert_eq!(
            svc.ledger_statuses("m1"),
            vec![DeliveryStatus::Delivered],
            "즉시 주입 = delivered(ADR-0104)"
        );
        assert_eq!(
            port.injected_bodies(),
            vec![r#"<message from="bob">hi</message>"#.to_string()],
            "주입 = 현재 포맷(xml) 봉투"
        );
    }

    #[test]
    fn inject_failure_parks_pending() {
        let (svc, port) = svc();
        let (_id, alice) = live("alice");
        port.set_roster(vec![alice]);
        port.fail_at(&[0]);
        let out = svc
            .park_absent_for_test(
                "m1",
                ident(),
                "bob",
                "alice",
                "hi",
                Entrance::Cli,
                &SendMeta::default(),
            )
            .expect("파킹은 반려 아님");
        assert!(
            matches!(out, SendOutcome::Parked { .. }),
            "inject 실패 → 파킹"
        );
        assert_eq!(
            svc.ledger_statuses("m1"),
            vec![DeliveryStatus::Pending],
            "inject 실패도 pending 장부(유실 금지)"
        );
        assert_eq!(svc.parked_len(tid("alice")), 1);
    }

    #[test]
    fn inject_failure_parks_pending_without_a_turn_signal() {
        // ★The 2 regression kinds it blocks★: ① a change that turns this class's injection failure into a failure row/loss (ADR-0116 decision 7) ② a change that makes
        //   a **new send overtake** the backlog accumulated that way (ADR-0121 decision 2 — keep the gate omission but
        //   keep the order). What makes ② hold is not sealing the decision points but the **enqueue order** (ADR-0125 — a new letter also goes to the queue
        //   tail, so there is no way to overtake in the first place).
        // ★The occasion ㉠ for no isolation is asserted here too (spec §7 ④)★: what opens m1's injection-failed share is **the synchronous drain
        //   of the following send** — not the doorbell, not its own retry (pressing its own doorbell because of its own injection failure is
        //   forbidden — spec §5). So m2 alone, as a single send, must make m1 go out.
        let (svc, port) = svc();
        let (from, me) = live_sender("alice");
        let (_tui_id, tui) = live_no_turn_signal("tui");
        port.set_roster(vec![me, tui]);
        port.fail_at(&[0]);

        let out =
            send_body(&svc, "m1", from, "alice", &["tui"], "묵은 편지").expect("파킹은 반려 아님");
        assert_eq!(
            (out[0].status, out[0].code),
            (SendStatus::Pending, None),
            "주입 실패 → 파킹(실패 행도, 조용한 유실도 아니다): {out:?}"
        );
        assert_eq!(svc.parked_len(tid("tui")), 1);
        assert_eq!(
            svc.ledger_statuses("m1"),
            vec![DeliveryStatus::Pending],
            "장부도 pending 으로 남아야(유실 금지)"
        );

        let second = send_body(&svc, "m2", from, "alice", &["tui"], "새 편지").expect("행 응답");
        assert_eq!(
            second[0].status,
            SendStatus::Delivered,
            "적재 후 드레인이 묵은 것과 함께 냈다 = delivered(직발송이 아니라 큐를 거친 결과다): {second:?}"
        );
        assert_eq!(
            port.injected_bodies(),
            vec![
                r#"<message from="alice">묵은 편지</message>"#.to_string(),
                r#"<message from="alice">새 편지</message>"#.to_string()
            ],
            "묵은 것(m1) → 새것(m2) 순으로 한 배치에 나갔다(오래된 순 — 뒤이은 발송의 드레인이 열었다)"
        );
        assert_eq!(
            svc.parked_len(tid("tui")),
            0,
            "고립 없음 — 배치가 큐를 비웠다"
        );
        assert_eq!(
            (svc.ledger_statuses("m1"), svc.ledger_statuses("m2")),
            (
                vec![DeliveryStatus::Delivered],
                vec![DeliveryStatus::Delivered]
            ),
            "둘 다 실제 주입 시점에 delivered"
        );
    }

    #[test]
    fn a_mid_batch_injection_failure_stops_there_and_leaves_the_rest_pending() {
        // ★The latter half of ADR-0121 decision 2★: partial success is not reverted to total failure — `delivered → failed` is **illegal** on the ledger
        //   transition graph (the `fail_pending` guard in ledger is canonical), so there is no means to express a rollback at all.
        //   So this assertion is itself the observation of "the ledger rows keep the graph".
        // ★Why the backlog is made with the gate (ADR-0125)★: **every send runs the drain in its own call**, so
        //   an unconsumed doorbell alone does not make the queue pile up. To make a 3-item batch the drain must be **unable to send**,
        //   and the formal means for that is the idle gate (a recipient with a turn signal set busy).
        let (svc, port, gate) = svc_gated();
        let (from, me) = live_sender("alice");
        let (recv_id, recv) = live("recv"); // Has a turn signal = a gate target.
        port.set_roster(vec![me, recv]);
        gate.set_busy(recv_id, 0);

        for (id, body) in [("m1", "하나"), ("m2", "둘"), ("m3", "셋")] {
            let out = send_body(&svc, id, from, "alice", &["recv"], body).expect("행 응답");
            assert_eq!(
                out[0].status,
                SendStatus::Pending,
                "{id} 는 적재되고 드레인이 게이트에 걸린다: {out:?}"
            );
        }
        assert_eq!(
            svc.parked_msg_ids(tid("recv")),
            vec!["m1", "m2", "m3"],
            "적재 순서 = 큐 순서(오래된 순)"
        );

        gate.clear();
        port.fail_at(&[1]);
        svc.flush_for(recv_id);

        assert_eq!(
            port.injected_bodies(),
            vec![r#"<message from="alice">하나</message>"#.to_string()],
            "실패 지점에서 배치가 끊긴다 — 그 뒤 항목은 쓰이지 않았다"
        );
        assert_eq!(
            svc.parked_msg_ids(tid("recv")),
            vec!["m2", "m3"],
            "실패 지점 이후는 나이 순서를 지킨 채 큐로 되돌아온다(TTL 연장 없음)"
        );
        assert_eq!(
            (
                svc.ledger_statuses("m1"),
                svc.ledger_statuses("m2"),
                svc.ledger_statuses("m3")
            ),
            (
                vec![DeliveryStatus::Delivered],
                vec![DeliveryStatus::Pending],
                vec![DeliveryStatus::Pending]
            ),
            "앞부분은 delivered 로 남고(되돌리지 않는다) 나머지는 pending 유지"
        );
    }

    #[test]
    fn an_overlapping_drain_retreats_with_pending_and_the_letter_still_goes_out_in_order() {
        // ★spec §7 ⑦ · ADR-0125 §impact unverified ②★ — seat reservation was abolished, but the two properties it sealed remain:
        //   **no order inversion** and **no isolation (`parked_len == 0`)**.
        // The interleaving that reproduces it (the drain runs inside the send call, so this is a routine case):
        //   ① m1 enqueues, enters its own drain and stops inside inject (the batch is in-flight)
        //   ② meanwhile m2 arrives and targets the same recipient → it is enqueued, and its drain **backs off at step 0**
        //   ③ m1's inject fails and that batch is re-parked, and settlement echoes the backed-off side's doorbell
        // Without the guard, ② drains the queue again and reaches the recipient **before m1's remainder** (an order inversion).
        // ★Determinism★: thanks to the hook running **on the same thread inside** the fake port's `inject`, there is zero dependence on
        //   sleep or thread timing.
        let (svc, port) = svc();
        let (from, me) = live_sender("alice");
        let (_tui_id, tui) = live_no_turn_signal("tui");
        port.set_roster(vec![me, tui]);

        // An assertion inside the hook lets a panic leak out by unwinding and blur the cause, so only the result is brought out and asserted outside.
        let observed = Arc::new(StdMutex::new(None::<RecipientResult>));
        let observed_h = observed.clone();
        let svc_h = svc.clone();
        let armed = Arc::new(StdMutex::new(true));
        let armed_h = armed.clone();
        port.set_on_inject(Arc::new(move |idx| {
            if idx != 0 || !std::mem::replace(&mut *armed_h.lock().unwrap(), false) {
                return;
            }
            let second =
                send_body(&svc_h, "m2", from, "alice", &["tui"], "새 편지").expect("행 응답");
            *observed_h.lock().unwrap() = Some(second[0].clone());
        }));
        port.fail_at(&[0]);

        let first =
            send_body(&svc, "m1", from, "alice", &["tui"], "묵은 편지").expect("파킹은 반려 아님");
        assert_eq!(first[0].status, SendStatus::Pending, "{first:?}");
        let loser = observed.lock().unwrap().clone().expect("동시 발송 행");
        assert_eq!(
            loser.status,
            SendStatus::Pending,
            "겹친 드레인에서 물러난 쪽은 **보수적으로** pending 을 답한다(자기 편지의 주입 여부를 모른다)"
        );
        assert!(
            loser.hint.unwrap_or_default().contains("at the same time"),
            "그 pending 은 ㉯(확인 불가)여야 — ㉮(큐에 남음) 사유로 답하면 '안 갔다' 로 오독된다(spec §6)"
        );
        // Getting here without a manual flush is because the receipt holder echoes the backed-off side's doorbell
        //   (spec §7 ④ ㉡ — which occasion emitted it is exactly this assertion).
        assert_eq!(
            port.injected_bodies(),
            vec![
                r#"<message from="alice">묵은 편지</message>"#.to_string(),
                r#"<message from="alice">새 편지</message>"#.to_string()
            ],
            "배달 순서 = 적재 순서(m1 → m2). 겹침 가드가 없으면 m2 가 m1 을 앞지른다"
        );
        assert_eq!(svc.parked_len(tid("tui")), 0, "고립 없음 — 큐가 비워졌다");
        assert_eq!(
            (svc.ledger_statuses("m1"), svc.ledger_statuses("m2")),
            (
                vec![DeliveryStatus::Delivered],
                vec![DeliveryStatus::Delivered]
            ),
            "응답이 pending 이어도 장부는 실제 주입 시점에 delivered 다(두 축은 다르다 — spec §6)"
        );
    }

    #[test]
    fn a_concurrent_burst_loses_nothing_and_keeps_each_senders_own_order() {
        // ★This is a **soak test** — it is not a seal of the overlap guard (review finding, two reviewers agree)★.
        //   ★What this name does **not** promise★: there is no assertion here that the global delivery order matches the mutex acquisition (= enqueue) order.
        //   The enqueue order **between threads** is decided by the scheduler so it cannot be observed at all, and even if the overlap guard is
        //   deleted this test turns red only **probabilistically**. The deterministic seal of the guard is borne by `an_overlapping_drain_...`
        //   (a hook plugs the interleaving at that point) and the two deferral-echo hook tests — do not delete those and
        //   rely on this one.
        //   Several sending threads draining the same recipient at once is a **routine case**, so there is value in checking that the assertions below
        //   do not collapse under real thread contention.
        // ★The cap is deliberately left alone★ — the total is kept under the cap (100) so contention does not leak into `MAILBOX_FULL`
        //   (that axis belongs to `the_cap_gate_applies_to_a_live_idle_recipient_too`).
        const SENDERS: usize = 4;
        const PER_SENDER: usize = 20;

        let (svc, port) = svc();
        let (from, me) = live_sender("alice");
        let (_tui_id, tui) = live_no_turn_signal("tui");
        port.set_roster(vec![me, tui]);

        let handles: Vec<_> = (0..SENDERS)
            .map(|k| {
                let svc_h = svc.clone();
                std::thread::spawn(move || {
                    (0..PER_SENDER)
                        .map(|n| {
                            let tag = format!("t{k}-{n}");
                            let out = send_body(&svc_h, &tag, from, "alice", &["tui"], &tag)
                                .expect("행 응답");
                            out[0].status
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        let statuses: Vec<SendStatus> = handles
            .into_iter()
            .flat_map(|h| h.join().expect("발신 스레드"))
            .collect();

        assert!(
            statuses
                .iter()
                .all(|s| matches!(s, SendStatus::Delivered | SendStatus::Pending)),
            "경합은 실패 행을 만들지 않는다(신원 축은 경합과 무관): {statuses:?}"
        );
        assert_eq!(
            svc.parked_len(tid("tui")),
            0,
            "고립 없음 — 큐가 끝내 비워졌다"
        );

        let bodies = port.injected_bodies();
        assert_eq!(
            bodies.len(),
            SENDERS * PER_SENDER,
            "유실 0 · 중복 0 — 모든 편지가 정확히 한 번 나갔다"
        );
        let order: Vec<(usize, usize)> = bodies
            .iter()
            .map(|b| {
                let start = b.find('>').expect("여는 태그") + 1;
                let end = b.rfind('<').expect("닫는 태그");
                let (k, n) = b[start..end]
                    .trim_start_matches('t')
                    .split_once('-')
                    .expect("태그 표기");
                (k.parse().expect("발신자"), n.parse().expect("순번"))
            })
            .collect();
        for k in 0..SENDERS {
            let seen: Vec<usize> = order
                .iter()
                .filter(|(s, _)| *s == k)
                .map(|(_, n)| *n)
                .collect();
            assert_eq!(seen.len(), PER_SENDER, "발신자 {k} 의 편지가 전부 나가야");
            assert!(
                seen.windows(2).all(|w| w[0] < w[1]),
                "발신자 {k} 가 적재한 순서가 배달에서 뒤집혔다 — 배치가 큐 앞머리를 건너뛰었다: {seen:?}"
            );
        }
    }

    #[test]
    fn the_cap_gate_applies_to_a_live_idle_recipient_too() {
        // ★ADR-0125 §impact · spec §5 — wider surface of cap application★: **since every send passes the enqueue gate**, even a live
        //   recipient can get `MAILBOX_FULL` — **a design consequence, not a regression**. Do not revive the old contract ("with an empty queue +
        //   idle, even the 101st is injected immediately and is not a cap violation").
        // ★The narrow span is pinned too so it is not read as "it increases overall" (spec §5 correction)★: if the recipient is idle,
        //   the drain empties the queue, so no matter how much is sent the cap is never hit in the first place (①). The span that really differs is
        //   only when the queue exceeds the cap before the drain catches up, and here that is made deterministic with the idle gate (②③).
        let (svc, port, gate) = svc_gated();
        let (from, me) = live_sender("boss");
        let (r_id, recv) = live("recv");
        port.set_roster(vec![me, recv]);

        // ① An idle recipient — the drain empties the queue every time.
        for i in 0..200 {
            let out = send(&svc, &format!("idle{i}"), from, "boss", &["recv"]).expect("행 응답");
            assert_eq!(
                out[0].status,
                SendStatus::Delivered,
                "유휴 수신자에겐 cap 이 물리지 않는다(#{i}): {out:?}"
            );
        }
        assert_eq!(
            svc.parked_len(tid("recv")),
            0,
            "큐가 비어 있으니 cap 분모도 0"
        );

        // ② Only in the span where the drain cannot run (in a turn) does the queue fill up to the cap.
        gate.set_busy(r_id, 0);
        for i in 0..100 {
            let out = send(&svc, &format!("busy{i}"), from, "boss", &["recv"]).expect("행 응답");
            assert_eq!(
                out[0].status,
                SendStatus::Pending,
                "턴 중 = 큐에 남는다(#{i})"
            );
        }
        assert_eq!(svc.parked_len(tid("recv")), 100);

        // ③ The next send is caught by the cap gate **even though the recipient is live**.
        let over = send(&svc, "over", from, "boss", &["recv"]).expect("행 응답");
        assert_eq!(
            (over[0].status, over[0].code),
            (SendStatus::Failed, Some(FailCode::MailboxFull)),
            "산 수신자도 cap 게이트를 지난다(직발송 우회 폐지 — ADR-0125): {over:?}"
        );
        assert_eq!(
            svc.parked_len(tid("recv")),
            100,
            "반려분은 큐에 안 들어간다"
        );
        assert_eq!(svc.ledger_statuses("over"), vec![DeliveryStatus::Failed]);
    }

    #[test]
    fn a_drain_that_deferred_a_flush_repays_it_even_when_its_own_inject_fails() {
        // ★The receipt holder repays the deferral wake-up (spec §5 invariant · ADR-0121 · ADR-0125)★: unless the receipt holder echoes the
        //   `deferred_flush` marker while settling, that wake-up evaporates (lost wakeup) and the re-parked share
        //   is bound until the next event or TTL. The drain runs on the sending thread so holding the receipt is a routine case,
        //   and so this branch is the main path, not an exception.
        // ★This is not a self-retry★: what is repaid here is a wake-up *another caller requested*. A lone failure
        //   path with no marker still presses no doorbell (the "parked_len == 1 after send" assertion of
        //   `inject_failure_parks_pending_without_a_turn_signal` guards that side).
        let (svc, port) = svc();
        let (from, me) = live_sender("alice");
        let (tui_id, tui) = live_no_turn_signal("tui");
        port.set_roster(vec![me, tui]);

        let svc_h = svc.clone();
        let armed = Arc::new(StdMutex::new(true));
        let armed_h = armed.clone();
        port.set_on_inject(Arc::new(move |idx| {
            if idx != 0 || !std::mem::replace(&mut *armed_h.lock().unwrap(), false) {
                return;
            }
            svc_h.flush_for(tui_id);
        }));
        port.fail_at(&[0]);

        send_body(&svc, "m1", from, "alice", &["tui"], "묵은 편지").expect("파킹");

        assert_eq!(
            port.injected_bodies(),
            vec![r#"<message from="alice">묵은 편지</message>"#.to_string()],
            "물러났던 flush 가 정산 후 되울려져 재파킹분을 집어야 한다"
        );
        assert_eq!(svc.parked_len(tid("tui")), 0);
        assert_eq!(svc.ledger_statuses("m1"), vec![DeliveryStatus::Delivered]);
    }

    #[test]
    fn a_successful_drain_also_repays_the_flush_it_deferred() {
        // ★The success branch of the same contract★: a batch receipt is in-flight until settlement time even when the injection succeeds — the duty to
        //   repay a backed-off flush in the meantime is the same as in the failure branch. Dropping just the success branch makes delivery depend on "whether there happens to be another
        //   doorbell" (a recipient with no turn signal has no turn-end doorbell, so unless repaid it does not open until the next appearance — `deferred_flush`).
        let (svc, port) = svc();
        let (from, me) = live_sender("alice");
        let (tui_id, tui) = live_no_turn_signal("tui");
        port.set_roster(vec![me, tui]);

        let svc_h = svc.clone();
        let armed = Arc::new(StdMutex::new(true));
        let armed_h = armed.clone();
        port.set_on_inject(Arc::new(move |idx| {
            if idx != 0 || !std::mem::replace(&mut *armed_h.lock().unwrap(), false) {
                return;
            }
            // Makes the parked items by a **path that does not press the doorbell** — so that the following flush backs off.
            svc_h
                .park_absent_for_test(
                    "m0",
                    ident(),
                    "s",
                    "tui",
                    "덤",
                    Entrance::Mcp,
                    &SendMeta::default(),
                )
                .expect("park");
            svc_h.flush_for(tui_id);
        }));

        send_body(&svc, "m1", from, "alice", &["tui"], "내 편지").expect("행 응답");

        assert_eq!(
            svc.parked_len(tid("tui")),
            0,
            "성공 갈래도 유예 표식을 갚아야 — 안 갚으면 m0 이 다음 사건까지 큐에 묶인다"
        );
        assert_eq!(svc.ledger_statuses("m0"), vec![DeliveryStatus::Delivered]);
    }

    #[test]
    fn the_flush_gate_asks_the_same_predicate_the_send_path_asks() {
        // ★ADR-0121 §impact — the gate predicate is defined in one place only★: if the send side decides "no gate for this class" and
        //   the flush side backs off on the gate, then because of the order guarantee **no one opens** the letter that joined the queue (delivery
        //   stops). So here it is sealed that the two sides call the same predicate (`gate_busy_reason`).
        // ★Assert with the gate set to busy★: the class with no turn signal must **not look at** that value —
        //   if `flush_for` goes back to the old form that skips `gate_busy_reason` and calls the gate (`busy.busy_reason`·`busy.is_busy`) directly,
        //   the batch is skipped and this turns red.
        let (svc, port, gate) = svc_gated();
        let (from, me) = live_sender("alice");
        let (tui_id, tui) = live_no_turn_signal("tui");
        port.set_roster(vec![me, tui]);
        gate.set_busy(tui_id, 0);

        port.fail_at(&[0]);
        let out = send(&svc, "m1", from, "alice", &["tui"]).expect("행 응답");
        assert_eq!(out[0].status, SendStatus::Pending, "{out:?}");
        assert_eq!(svc.parked_len(tid("tui")), 1);

        port.fail_at(&[]);
        svc.flush_for(tui_id);
        assert_eq!(
            svc.ledger_statuses("m1"),
            vec![DeliveryStatus::Delivered],
            "턴 신호 없는 수신자의 배치는 게이트가 busy 라 해도 나간다(발송측과 같은 술어)"
        );
        assert_eq!(svc.parked_len(tid("tui")), 0);
    }

    #[test]
    fn flush_on_appearance_delivers_oldest_first_individually_wrapped() {
        let (svc, port) = svc();
        port.set_roster(vec![]);
        for (i, m) in ["m0", "m1", "m2"].iter().enumerate() {
            svc.park_absent_for_test(
                m,
                ident(),
                "sndr",
                "late",
                &format!("body{i}"),
                Entrance::Mcp,
                &SendMeta::default(),
            )
            .expect("park");
        }
        assert_eq!(svc.parked_len(tid("late")), 3);
        // finding 2: `flush_for` re-verifies unique reachability with the execution-time roster — so raise it before the flush.
        let (late_id, late) = live("late");
        port.set_roster(vec![late]);
        svc.flush_for(late_id);
        assert_eq!(
            port.injected_bodies(),
            vec![
                r#"<message from="sndr">body0</message>"#.to_string(),
                r#"<message from="sndr">body1</message>"#.to_string(),
                r#"<message from="sndr">body2</message>"#.to_string(),
            ],
            "flush = 오래된 순 개별 봉투 일괄 주입(ADR-0104)"
        );
        assert_eq!(svc.ledger_statuses("m0"), vec![DeliveryStatus::Delivered]);
        assert_eq!(svc.ledger_statuses("m2"), vec![DeliveryStatus::Delivered]);
        assert_eq!(svc.parked_len(tid("late")), 0, "flush 후 큐 비움");
    }

    #[test]
    fn flush_partial_failure_reparks_remaining_no_loss() {
        let (svc, port) = svc();
        port.set_roster(vec![]);
        for (i, m) in ["m0", "m1", "m2"].iter().enumerate() {
            svc.park_absent_for_test(
                m,
                ident(),
                "s",
                "late",
                &format!("b{i}"),
                Entrance::Mcp,
                &SendMeta::default(),
            )
            .expect("park");
        }
        let (late_id, late) = live("late");
        port.set_roster(vec![late]);
        port.fail_at(&[1]);
        svc.flush_for(late_id);
        assert_eq!(
            port.injected_bodies(),
            vec![r#"<message from="s">b0</message>"#.to_string()],
            "실패 전(b0)만 주입"
        );
        assert_eq!(
            svc.ledger_statuses("m0"),
            vec![DeliveryStatus::Delivered],
            "b0 delivered"
        );
        assert_eq!(
            svc.parked_len(tid("late")),
            2,
            "실패분+미시도분 재파킹(무손실)"
        );
        assert_eq!(
            svc.ledger_statuses("m1"),
            vec![DeliveryStatus::Pending],
            "b1 재파킹 pending"
        );
        assert_eq!(
            svc.ledger_statuses("m2"),
            vec![DeliveryStatus::Pending],
            "b2 재파킹 pending"
        );
        port.fail_at(&[]);
        svc.flush_for(late_id);
        assert_eq!(
            port.injected_bodies(),
            vec![
                r#"<message from="s">b0</message>"#.to_string(),
                r#"<message from="s">b1</message>"#.to_string(),
                r#"<message from="s">b2</message>"#.to_string(),
            ],
            "재파킹분 재-flush 도 오래된 순 보존(b1→b2)"
        );
        assert_eq!(svc.parked_len(tid("late")), 0, "재-flush 후 큐 비움");
    }

    #[test]
    fn flush_repark_with_full_queue_loses_nothing_and_preserves_order() {
        // ★finding 1 (BLOCK) regression★: when re-parking after an inject failure during flush, even if a **concurrent park** between drain↔inject
        //   refilled the queue to the cap, an admitted item must not be lost (no silent loss), and the re-parked share (older)
        //   must go ahead of the concurrent park's new share (preventing FIFO inversion). The old park-based re-parking was lost to MailboxFull.
        let (svc, port) = svc();
        let (late_id, late) = live("late");
        port.set_roster(vec![]);
        for (i, m) in ["m0", "m1", "m2"].iter().enumerate() {
            svc.park_absent_for_test(
                m,
                ident(),
                "s",
                "late",
                &format!("b{i}"),
                Entrance::Mcp,
                &SendMeta::default(),
            )
            .expect("park");
        }
        // The hook's concurrent sends must be **absent** to be parked, so on entry at idx 0 it empties the roster briefly — flush_for's
        //   unique-reachability re-verification has already passed by then, so there is no effect.
        let svc_hook = svc.clone();
        let port_hook = port.clone();
        let late_for_hook = late.clone();
        port.set_on_inject(Arc::new(move |idx| {
            if idx == 0 {
                port_hook.set_roster(vec![]);
                for i in 0..MAILBOX_CAP_FOR_TEST {
                    let _ = svc_hook.park_absent_for_test(
                        &format!("c{i}"),
                        ident(),
                        "s",
                        "late",
                        &format!("c{i}"),
                        Entrance::Mcp,
                        &SendMeta::default(),
                    );
                }
                // Restores unique reachability again so the 2nd flush re-verification passes.
                port_hook.set_roster(vec![late_for_hook.clone()]);
            }
        }));
        port.set_roster(vec![late.clone()]);
        port.fail_at(&[1]);
        svc.flush_for(late_id);

        assert_eq!(
            port.injected_bodies(),
            vec![r#"<message from="s">b0</message>"#.to_string()],
            "실패 전(b0)만 주입"
        );
        // ★Zero loss — but it does not exceed the cap either (expected value corrected in F1: 102 → 99)★. The old expected value was "the concurrent park
        //   fills in all the way to the cap (100) and the 2 re-parked items sit on top of it", and that is exactly the hole that grew the queue every cycle
        //   (the empty-queue window). Now the 3 items of the drained batch remain in the denominator as in-flight, so concurrent park is accepted only 97 times, and
        //   the 2 re-parked items come back, making 97 + 2 = 99 ≤ cap.
        assert_eq!(
            svc.parked_len(tid("late")),
            MAILBOX_CAP_FOR_TEST - 1,
            "재파킹분(b1,b2)은 유실되지 않고, 합계는 cap 을 넘지 않는다(97 동시 park + 2 재파킹)"
        );
        assert!(
            svc.ledger_statuses("c99").is_empty(),
            "in-flight 때문에 반려된 동시 발송은 장부에도 안 남는다(반려 = 저장 자체를 안 함)"
        );
        assert_eq!(
            svc.ledger_statuses("m1"),
            vec![DeliveryStatus::Pending],
            "b1 재파킹 pending(유령 아님)"
        );
        assert_eq!(
            svc.ledger_statuses("m2"),
            vec![DeliveryStatus::Pending],
            "b2 재파킹 pending(유령 아님)"
        );
        // ★Order★
        port.set_on_inject(Arc::new(|_| {}));
        port.fail_at(&[]);
        svc.flush_for(late_id);
        let bodies = port.injected_bodies();
        assert_eq!(bodies[0], r#"<message from="s">b0</message>"#);
        assert_eq!(
            bodies[1], r#"<message from="s">b1</message>"#,
            "재파킹분이 동시 park 신규분보다 앞서야(FIFO 역전 방지)"
        );
        assert_eq!(bodies[2], r#"<message from="s">b2</message>"#);
        assert_eq!(
            bodies[3], r#"<message from="s">c0</message>"#,
            "동시 park 신규분은 재파킹분 뒤"
        );
        assert_eq!(svc.parked_len(tid("late")), 0, "재-flush 후 큐 비움");
    }

    /// A mirror of mailbox's private `MAILBOX_CAP` — the values must be fixed together.
    const MAILBOX_CAP_FOR_TEST: usize = 100;

    /// A mirror of mailbox's private `PARK_TTL` — the values must be fixed together (ADR-0105).
    const PARK_TTL_FOR_TEST: Duration = Duration::from_secs(24 * 60 * 60);

    #[test]
    fn cap_exceeded_rejects_mailbox_full() {
        let (svc, port) = svc();
        port.set_roster(vec![]);
        for i in 0..100 {
            svc.park_absent_for_test(
                &format!("m{i}"),
                ident(),
                "s",
                "full",
                "x",
                Entrance::Mcp,
                &SendMeta::default(),
            )
            .expect("cap 이내 파킹");
        }
        assert_eq!(svc.parked_len(tid("full")), 100);
        let rej = svc
            .park_absent_for_test(
                "over",
                ident(),
                "s",
                "full",
                "x",
                Entrance::Mcp,
                &SendMeta::default(),
            )
            .expect_err("cap 초과는 반려");
        assert_eq!(rej, SendFail::Row(FailCode::MailboxFull));
        assert_eq!(svc.parked_len(tid("full")), 100, "반려된 건 큐에 안 들어감");
        // It is made visible to the sender at once as a bounce, so it is not left in the ledger.
        assert!(
            svc.ledger_statuses("over").is_empty(),
            "cap 반려는 ledger 미기록"
        );
    }

    #[test]
    fn sweep_expires_parked_and_records_in_ledger() {
        let (svc, port) = svc();
        port.set_roster(vec![]);
        svc.park_absent_for_test(
            "m1",
            ident(),
            "s",
            "ghost",
            "hi",
            Entrance::Mcp,
            &SendMeta::default(),
        )
        .expect("park");
        assert_eq!(svc.ledger_statuses("m1"), vec![DeliveryStatus::Pending]);
        // The parking time is the Instant::now() inside handle_single_send, so give a now sufficiently in the future of that.
        let now = Instant::now() + PARK_TTL_FOR_TEST + Duration::from_secs(1);
        svc.sweep(now);
        assert_eq!(
            svc.ledger_statuses("m1"),
            vec![DeliveryStatus::Expired],
            "sweep = pending→expired 장부 전이"
        );
        assert_eq!(svc.parked_len(tid("ghost")), 0, "만료분 큐에서 제거");
    }

    #[test]
    fn flush_absent_queue_is_noop() {
        let (svc, port) = svc();
        let (id, a) = live("nobody");
        port.set_roster(vec![a]); // finding 2: to pass the flush_for re-verification (the queue is empty so it is a no-op anyway).
        svc.flush_for(id);
        assert!(port.injected_bodies().is_empty());
    }

    #[test]
    fn flush_skips_when_target_died_at_execution_time() {
        let (svc, port) = svc();
        port.set_roster(vec![]);
        svc.park_absent_for_test(
            "m1",
            ident(),
            "s",
            "gone",
            "hi",
            Entrance::Mcp,
            &SendMeta::default(),
        )
        .expect("park");
        assert_eq!(svc.parked_len(tid("gone")), 1);
        let (stale_id, _gone) = live("gone");
        svc.flush_for(stale_id);
        assert_eq!(
            svc.parked_len(tid("gone")),
            1,
            "execution 시점 부재 → flush skip(파킹 유지)"
        );
        assert!(
            port.injected_bodies().is_empty(),
            "죽은 대상엔 주입 안 함(finding 2)"
        );
    }

    /// ★A new incarnation of the same id receives it (ADR-0111 decision 6 → ADR-0309 decision 1)★: a letter parked for a busy incarnation (epoch 0) goes to
    ///   the new incarnation when the same profile comes back up as a new incarnation (same id · epoch 1). The gate is asked by (id, epoch), so
    ///   even if the old incarnation's busy mark remains it does not block the new incarnation.
    #[test]
    fn parked_mail_goes_to_the_next_incarnation_of_the_same_id() {
        let (svc, port, gate) = svc_gated();
        let (id, old) = live_distinct("recv");
        port.set_roster(vec![old.clone()]);
        gate.set_busy(id, 0);
        svc.park_absent_for_test(
            "m1",
            ident(),
            "s",
            "recv",
            "hi",
            Entrance::Mcp,
            &SendMeta::default(),
        )
        .expect("park");
        assert_eq!(svc.parked_len(id), 1);

        port.set_roster(vec![LiveAgent { epoch: 1, ..old }]);
        svc.flush_for(id);

        assert_eq!(svc.parked_len(id), 0, "새 화신 배달로 큐 비움");
        assert_eq!(svc.ledger_statuses("m1"), vec![DeliveryStatus::Delivered]);
        assert_eq!(port.injected_targets(), vec![id]);
        assert_eq!(
            port.injected_bodies(),
            vec![r#"<message from="s">hi</message>"#.to_string()],
        );
    }

    #[test]
    fn park_payload_roundtrip_survives_newline_in_body() {
        let from = SenderIdentity {
            peer_id: PeerId::new_v4(),
            epoch: 3,
        };
        let p = ParkPayload {
            sender_name: "qa-alpha".to_string(),
            from,
            entrance: Entrance::Mcp,
            body: "line1\nline2".to_string(),
            meta: SendMeta::default(),
        };
        let d = ParkPayload::decode(&p.encode());
        assert_eq!(d.sender_name, "qa-alpha");
        assert_eq!(d.body, "line1\nline2");
        assert_eq!(d.from.peer_id, from.peer_id);
        assert_eq!(d.from.epoch, 3);
        assert!(matches!(d.entrance, Entrance::Mcp));
        assert_eq!(d.meta, SendMeta::default(), "통보는 계약 메타가 비어야");
    }

    #[test]
    fn park_payload_roundtrip_preserves_contract_meta_with_newlines() {
        // ★C3★: reply_to is **agent input**, so newlines and multibyte characters can be mixed in.
        let from = SenderIdentity {
            peer_id: PeerId::new_v4(),
            epoch: 7,
        };
        let meta = SendMeta {
            request: true,
            reply_by_raw: Some("10m".to_string()),
            reply_by: Some(Duration::from_secs(600)),
            reply_to: None,
            to_attr: None,
        };
        let p = ParkPayload {
            sender_name: "발신자-한글".to_string(),
            from,
            entrance: Entrance::Daemon,
            body: "본문\n둘째 줄".to_string(),
            meta: meta.clone(),
        };
        let d = ParkPayload::decode(&p.encode());
        assert_eq!(
            d.sender_name, "발신자-한글",
            "멀티바이트 sender 경계(바이트 길이 접두)"
        );
        assert_eq!(d.body, "본문\n둘째 줄");
        assert!(matches!(d.entrance, Entrance::Daemon));
        assert!(d.meta.request, "request 플래그 복원");
        assert_eq!(d.meta.reply_by_raw.as_deref(), Some("10m"), "표기 복원");
        assert_eq!(d.meta.reply_by, None, "파싱값은 의도적으로 미복원");
        assert_eq!(d.meta.reply_to, None);

        let reply = ParkPayload {
            sender_name: "s".to_string(),
            from,
            entrance: Entrance::Cli,
            body: "b".to_string(),
            meta: SendMeta {
                request: false,
                reply_by_raw: None,
                reply_by: None,
                reply_to: Some("m-abc\nxyz".to_string()),
                to_attr: None,
            },
        };
        let d = ParkPayload::decode(&reply.encode());
        assert_eq!(d.meta.reply_to.as_deref(), Some("m-abc\nxyz"));
        assert_eq!(d.body, "b");
        assert!(!d.meta.request);
    }

    // ── C2: idle gate (spec §5 injection timing · ADR-0104 decision 3) ────────────────────────────────

    #[test]
    fn busy_recipient_parks_pending_without_inject() {
        // The state vocabulary is shared with injection-failure parking (no inventing a new state) and only the hint distinguishes the reason.
        let (svc, port, gate) = svc_gated();
        let (alice_id, alice) = live("alice");
        port.set_roster(vec![alice]);
        gate.set_busy(alice_id, 0);
        let out = svc
            .park_absent_for_test(
                "m1",
                ident(),
                "bob",
                "alice",
                "hi",
                Entrance::Mcp,
                &SendMeta::default(),
            )
            .expect("busy 파킹은 반려 아님");
        match out {
            SendOutcome::Parked { hint } => {
                assert!(
                    hint.contains("mid-turn"),
                    "hint 가 busy 사유를 알린다: {hint}"
                )
            }
            other => panic!("busy 수신자는 파킹이어야: {other:?}"),
        }
        assert_eq!(
            svc.ledger_statuses("m1"),
            vec![DeliveryStatus::Pending],
            "busy 파킹 = pending 장부(delivered 는 실제 주입 시점에만 — ADR-0104)"
        );
        assert_eq!(svc.parked_len(tid("alice")), 1, "그 id 큐에 대기");
        assert!(
            port.injected_bodies().is_empty(),
            "턴 중에는 stdin 에 밀지 않는다(CLI 큐 우회 금지)"
        );
    }

    // ── ADR-0231: the parking hint states the real busy reason (production gate `BusyPolicy` + planted facts) ─────────────

    struct SilentNotifier;
    impl super::super::busy::IdleNotifier for SilentNotifier {
        fn notify_idle(&self, _id: PeerId) {}
    }

    /// Builds the same `BusyPolicy` as production on top of planted facts — a reason cannot be produced with `FakeGate` (bool only).
    fn park_hint_under_policy(
        seed: impl FnOnce(&super::super::busy::ScriptedTurnFacts, PeerId),
    ) -> String {
        let facts = super::super::busy::ScriptedTurnFacts::new();
        let policy = Arc::new(super::super::busy::BusyPolicy::new(
            facts.clone(),
            Arc::new(SilentNotifier),
        ));
        let port = Arc::new(FakeDeliveryPort::new());
        let svc = MessagingService::new_gated(port.clone(), Arc::new(FakeControlPlane), policy);
        let (alice_id, alice) = live("alice");
        port.set_roster(vec![alice]);
        seed(&facts, alice_id);
        let out = svc
            .park_absent_for_test(
                "m1",
                ident(),
                "bob",
                "alice",
                "hi",
                Entrance::Mcp,
                &SendMeta::default(),
            )
            .expect("busy 파킹은 반려 아님");
        assert!(
            port.injected_bodies().is_empty(),
            "바쁜 수신자에게 밀지 않는다"
        );
        match out {
            SendOutcome::Parked { hint } => hint,
            other => panic!("busy 수신자는 파킹이어야: {other:?}"),
        }
    }

    #[test]
    fn a_recipient_halted_after_a_failed_turn_is_not_called_mid_turn() {
        // QA measurement: a "mid-turn" hint went out to a recipient stopped after an error turn (not in a turn).
        let hint = park_hint_under_policy(|f, id| {
            f.set_last_end_failed(id, 0, true, Instant::now());
        });
        assert_eq!(
            hint,
            park_hint_busy("alice", BusyReason::HaltedAfterError),
            "{hint}"
        );
        assert!(!hint.contains("mid-turn"), "{hint}");
    }

    #[test]
    fn halted_after_error_wins_over_a_running_turn_and_waiting_input() {
        let hint = park_hint_under_policy(|f, id| {
            let now = Instant::now();
            f.set_in_turn(id, 0, now);
            f.set_inputs_pending(id, 0, true, now);
            f.set_last_end_failed(id, 0, true, now);
        });
        assert_eq!(hint, park_hint_busy("alice", BusyReason::HaltedAfterError));
    }

    #[test]
    fn waiting_user_input_outside_a_turn_says_so() {
        let hint = park_hint_under_policy(|f, id| {
            f.set_inputs_pending(id, 0, true, Instant::now());
        });
        assert_eq!(hint, park_hint_busy("alice", BusyReason::InputsPending));
        assert!(hint.contains("user input waiting"), "{hint}");
    }

    #[test]
    fn a_running_turn_keeps_the_mid_turn_hint_even_with_waiting_input() {
        let hint = park_hint_under_policy(|f, id| {
            let now = Instant::now();
            f.set_in_turn(id, 0, now);
            f.set_inputs_pending(id, 0, true, now);
        });
        assert_eq!(hint, park_hint_busy("alice", BusyReason::InTurn));
        assert!(hint.contains("mid-turn"), "{hint}");
    }

    #[test]
    fn an_idle_recipient_gets_it_within_the_same_send_call() {
        let (svc, port, _gate) = svc_gated();
        let (_id, alice) = live("alice");
        port.set_roster(vec![alice]);
        let out = svc
            .park_absent_for_test(
                "m1",
                ident(),
                "bob",
                "alice",
                "hi",
                Entrance::Mcp,
                &SendMeta::default(),
            )
            .expect("no reject");
        assert_eq!(out, SendOutcome::Delivered);
        assert_eq!(svc.ledger_statuses("m1"), vec![DeliveryStatus::Delivered]);
        assert_eq!(
            port.injected_bodies(),
            vec![r#"<message from="bob">hi</message>"#.to_string()]
        );
        assert_eq!(svc.parked_len(tid("alice")), 0, "드레인이 큐를 비웠다");
    }

    #[test]
    fn unobserved_recipient_is_treated_idle_and_injected() {
        let (svc, port, gate) = svc_gated();
        let (alice_id, alice) = live("alice"); // Roster epoch = 0
        port.set_roster(vec![alice]);
        gate.set_busy(alice_id, 1); // Only an old/different incarnation is reported as busy
        let out = svc
            .park_absent_for_test(
                "m1",
                ident(),
                "bob",
                "alice",
                "hi",
                Entrance::Mcp,
                &SendMeta::default(),
            )
            .expect("no reject");
        assert_eq!(
            out,
            SendOutcome::Delivered,
            "미관측 (id, epoch) 는 idle 폴백 → 즉시 주입"
        );
    }

    #[test]
    fn multiple_parked_during_busy_flush_as_one_batch_oldest_first_on_idle() {
        let (svc, port, gate) = svc_gated();
        let (recv_id, recv) = live("recv");
        port.set_roster(vec![recv]);
        gate.set_busy(recv_id, 0);
        for (i, m) in ["m0", "m1", "m2"].iter().enumerate() {
            let out = svc
                .park_absent_for_test(
                    m,
                    ident(),
                    "s",
                    "recv",
                    &format!("b{i}"),
                    Entrance::Mcp,
                    &SendMeta::default(),
                )
                .expect("park");
            assert!(matches!(out, SendOutcome::Parked { .. }));
        }
        assert_eq!(svc.parked_len(tid("recv")), 3);
        assert!(port.injected_bodies().is_empty(), "턴 중엔 주입 0");

        gate.clear();
        svc.flush_for_agent(recv_id);
        assert_eq!(
            port.injected_bodies(),
            vec![
                r#"<message from="s">b0</message>"#.to_string(),
                r#"<message from="s">b1</message>"#.to_string(),
                r#"<message from="s">b2</message>"#.to_string(),
            ],
            "idle 진입 시 오래된 순 개별 봉투 일괄 주입(ADR-0104)"
        );
        assert_eq!(svc.ledger_statuses("m0"), vec![DeliveryStatus::Delivered]);
        assert_eq!(svc.ledger_statuses("m2"), vec![DeliveryStatus::Delivered]);
        assert_eq!(svc.parked_len(tid("recv")), 0);
    }

    #[test]
    fn busy_park_self_heals_when_turn_ends_during_park() {
        // ★busy-park TOCTOU self-healing (symmetric with C1 finding 3)★: if the turn ends between the gate check (busy)↔park,
        //   the idle trigger has already passed the empty queue (lost wakeup).
        let (svc, port, gate) = svc_gated();
        let (recv_id, recv) = live("recv");
        port.set_roster(vec![recv]);
        gate.set_busy(recv_id, 0);
        gate.arm_idle_after_call(1);
        let out = svc
            .park_absent_for_test(
                "m1",
                ident(),
                "s",
                "recv",
                "hi",
                Entrance::Mcp,
                &SendMeta::default(),
            )
            .expect("park");
        assert!(
            matches!(out, SendOutcome::Parked { .. }),
            "발신 응답은 파킹(pending) — self-heal 은 그 뒤의 배달"
        );
        assert_eq!(gate.call_count(), 2, "확인 + park 후 재확인 = 2회");
        assert_eq!(
            svc.ledger_statuses("m1"),
            vec![DeliveryStatus::Delivered],
            "park 직후 idle 이면 self-heal flush 로 즉시 배달(pending→delivered)"
        );
        assert_eq!(svc.parked_len(tid("recv")), 0, "self-heal flush 로 큐 비움");
        assert_eq!(
            port.injected_bodies(),
            vec![r#"<message from="s">hi</message>"#.to_string()]
        );
    }

    #[test]
    fn busy_park_does_not_self_heal_while_still_busy() {
        let (svc, port, gate) = svc_gated();
        let (recv_id, recv) = live("recv");
        port.set_roster(vec![recv]);
        gate.set_busy(recv_id, 0);
        svc.park_absent_for_test(
            "m1",
            ident(),
            "s",
            "recv",
            "hi",
            Entrance::Mcp,
            &SendMeta::default(),
        )
        .expect("park");
        assert_eq!(svc.parked_len(tid("recv")), 1, "여전히 busy → 파킹 유지");
        assert_eq!(svc.ledger_statuses("m1"), vec![DeliveryStatus::Pending]);
        assert!(port.injected_bodies().is_empty());
    }

    #[test]
    fn flush_skips_while_busy_then_delivers_whole_batch_when_idle() {
        let (svc, port, gate) = svc_gated();
        let (recv_id, recv) = live("recv");
        port.set_roster(vec![]);
        for (i, m) in ["m0", "m1", "m2"].iter().enumerate() {
            svc.park_absent_for_test(
                m,
                ident(),
                "s",
                "recv",
                &format!("b{i}"),
                Entrance::Mcp,
                &SendMeta::default(),
            )
            .expect("park");
        }
        port.set_roster(vec![recv]);
        gate.set_busy(recv_id, 0);
        let calls_before = gate.call_count();
        svc.flush_for(recv_id);
        assert!(
            port.injected_bodies().is_empty(),
            "턴 중 flush 는 배치를 시작하지 않는다(pre-drain 게이트)"
        );
        assert_eq!(svc.parked_len(tid("recv")), 3, "파킹 유지");
        assert_eq!(
            gate.call_count() - calls_before,
            1,
            "게이트 조회는 배치당 1회(pre-drain)"
        );
        gate.clear();
        let calls_before = gate.call_count();
        svc.flush_for(recv_id);
        assert_eq!(
            port.injected_bodies().len(),
            3,
            "idle 이면 배치 완주(mid-batch 재검사 없음 — 드리블 금지)"
        );
        assert_eq!(
            gate.call_count() - calls_before,
            1,
            "3건 배치에도 게이트 조회는 1회뿐"
        );
        assert_eq!(svc.parked_len(tid("recv")), 0);
    }

    #[test]
    fn flush_skip_while_busy_still_ledgers_expired_items() {
        let (svc, port, gate) = svc_gated();
        let (recv_id, recv) = live("recv");
        port.set_roster(vec![recv.clone()]);
        gate.set_busy(recv_id, 0);
        for (i, m) in ["m0", "m1"].iter().enumerate() {
            svc.park_absent_for_test(
                m,
                ident(),
                "s",
                "recv",
                &format!("b{i}"),
                Entrance::Mcp,
                &SendMeta::default(),
            )
            .expect("park");
        }
        assert_eq!(svc.parked_len(tid("recv")), 2);
        svc.flush_for(recv_id);
        assert_eq!(
            svc.parked_len(tid("recv")),
            2,
            "스킵은 큐를 건드리지 않는다"
        );
        assert_eq!(svc.ledger_statuses("m0"), vec![DeliveryStatus::Pending]);
        gate.clear();
        svc.flush_for(recv_id);
        assert_eq!(
            port.injected_bodies(),
            vec![
                r#"<message from="s">b0</message>"#.to_string(),
                r#"<message from="s">b1</message>"#.to_string(),
            ]
        );
    }

    /// Targeting one of the twins by its exact id parks it in that id's queue and it goes only to that agent — even if the names overlap
    ///   the queues do not mix (ADR-0309 decision 1). Twin B's turn end does not open A's letter.
    #[test]
    fn exact_id_send_to_a_busy_twin_parks_on_that_id_and_reaches_only_it() {
        let (svc, port, gate) = svc_gated();
        let (id_a, dup_a) = live_distinct("dup");
        let (id_b, dup_b) = live_distinct("dup");
        port.set_roster(vec![dup_a, dup_b]);
        gate.set_busy(id_a, 0);
        let out = svc
            .park_absent_for_test(
                "m1",
                ident(),
                "s",
                &id_a.to_string(),
                "hi",
                Entrance::Mcp,
                &SendMeta::default(),
            )
            .expect("park");
        assert!(matches!(out, SendOutcome::Parked { .. }));
        assert_eq!(svc.parked_len(id_a), 1, "큐 키 = 지목한 id");
        assert_eq!(svc.parked_len(id_b), 0, "쌍둥이 B 의 큐는 비어 있다");

        svc.flush_for_agent(id_b);
        assert!(
            port.injected_bodies().is_empty(),
            "B 의 턴 종료는 A 의 편지를 내지 않는다"
        );

        gate.clear();
        svc.flush_for_agent(id_a);
        assert_eq!(
            port.injected_bodies(),
            vec![r#"<message from="s">hi</message>"#.to_string()],
        );
        assert_eq!(
            port.injected_targets(),
            vec![id_a],
            "지목한 그 에이전트에 배달"
        );
        assert_eq!(svc.ledger_statuses("m1"), vec![DeliveryStatus::Delivered]);
        assert_eq!(svc.parked_len(id_a), 0);
    }

    /// ★B2 — the busy-parking side (ADR-0309 decisions 1 · 5)★: a letter parked for busy X does not go to Y even if X disappears and **a different
    ///   agent with the same name** Y (a different id) comes up. If X (same id) comes back up as a new incarnation, X receives it — even if Y
    ///   is still up with the same name.
    #[test]
    fn a_namesake_with_another_id_never_receives_the_parked_mail_the_original_id_does() {
        let (svc, port, gate) = svc_gated();
        let (x_id, x) = live_distinct("recv");
        port.set_roster(vec![x.clone()]);
        gate.set_busy(x_id, 0);
        svc.park_absent_for_test(
            "m1",
            ident(),
            "s",
            "recv",
            "hi",
            Entrance::Mcp,
            &SendMeta::default(),
        )
        .expect("park");
        assert_eq!(svc.parked_len(x_id), 1);
        gate.clear();

        let (y_id, y) = live_distinct("recv");
        port.set_roster(vec![y.clone()]);
        svc.flush_for(y_id);
        svc.flush_for_agent(y_id);
        svc.flush_for(x_id);
        assert!(
            port.injected_bodies().is_empty(),
            "X 가 없는 동안 동명 Y 는 X 의 편지를 받지 않는다(B2)"
        );
        assert_eq!(svc.parked_len(x_id), 1, "X 의 큐에 그대로");
        assert_eq!(svc.ledger_statuses("m1"), vec![DeliveryStatus::Pending]);

        port.set_roster(vec![y, LiveAgent { epoch: 1, ..x }]);
        svc.flush_for(x_id);
        assert_eq!(port.injected_targets(), vec![x_id], "다시 뜬 X 가 받는다");
        assert_eq!(svc.ledger_statuses("m1"), vec![DeliveryStatus::Delivered]);
    }

    #[test]
    fn a_new_send_never_overtakes_an_existing_queue() {
        // ★fix 5 regression (FIFO consistency)★ — a new send does not overtake the existing queue.
        let (svc, port, gate) = svc_gated();
        let (recv_id, recv) = live("recv");
        port.set_roster(vec![recv]);
        gate.set_busy(recv_id, 0);
        for (i, m) in ["m0", "m1"].iter().enumerate() {
            svc.park_absent_for_test(
                m,
                ident(),
                "s",
                "recv",
                &format!("b{i}"),
                Entrance::Mcp,
                &SendMeta::default(),
            )
            .expect("park");
        }
        gate.clear();
        let out = svc
            .park_absent_for_test(
                "m2",
                ident(),
                "s",
                "recv",
                "b2",
                Entrance::Mcp,
                &SendMeta::default(),
            )
            .expect("no reject");
        assert_eq!(
            out,
            SendOutcome::Delivered,
            "적재 후 그 호출의 드레인이 묵은 것과 함께 냈다 = delivered: {out:?}"
        );
        assert_eq!(
            port.injected_bodies(),
            vec![
                r#"<message from="s">b0</message>"#.to_string(),
                r#"<message from="s">b1</message>"#.to_string(),
                r#"<message from="s">b2</message>"#.to_string(),
            ],
            "수신자가 보는 순서 = 적재 순서(새 발송이 큐를 앞지르지 않는다)"
        );
        assert_eq!(svc.parked_len(tid("recv")), 0);
    }

    #[test]
    fn an_empty_queue_and_an_idle_recipient_still_ends_as_delivered() {
        let (svc, port, _gate) = svc_gated();
        let (_id, recv) = live("recv");
        port.set_roster(vec![recv]);
        let out = svc
            .park_absent_for_test(
                "m1",
                ident(),
                "s",
                "recv",
                "hi",
                Entrance::Mcp,
                &SendMeta::default(),
            )
            .expect("no reject");
        assert_eq!(
            out,
            SendOutcome::Delivered,
            "빈 큐 + idle 이어도 적재를 거치고, 같은 호출의 드레인이 내면 delivered"
        );
        assert_eq!(port.injected_bodies().len(), 1);
        assert_eq!(svc.parked_len(tid("recv")), 0, "드레인이 그 항목을 비웠다");
        assert_eq!(
            svc.in_flight_len(tid("recv")),
            0,
            "영수증 정산 누락 없음 — 남으면 그 수신자 레인이 영구 봉쇄된다"
        );
    }

    #[test]
    fn an_idle_sends_letter_is_observably_taken_out_of_the_queue_not_bypassed() {
        // ★The **counterfactual seal** of spec §7 ① (review blind MEDIUM)★ — the counterfactual it kills = "with an empty queue + idle, a shortcut that skips enqueue and
        //   goes straight to `port.inject` and then answers `Delivered`". Reviving that branch turns this
        //   red. `an_empty_queue_and_an_idle_recipient_still_ends_as_delivered` and
        //   `an_idle_recipient_gets_it_within_the_same_send_call` right above look only at the *outcome*, and the shortcut makes the same outcome,
        //   so those two cannot guard this contract.
        // ★Observation point★: the `on_inject` hook = outside the lock, the moment the drain holds the receipt — **in the middle** of injection. The ledger row being
        //   `pending` is also the implementation site of unspecified (ii) "does the row pass through pending".
        let (svc, port, _gate) = svc_gated();
        let (from, me) = live_sender("alice");
        let (_r, recv) = live("recv");
        port.set_roster(vec![me, recv]);

        // A panic inside the hook leaks out by unwinding and blurs the cause, so only the observation is brought out and asserted outside.
        let seen: Arc<StdMutex<Option<(usize, usize, Vec<DeliveryStatus>)>>> =
            Arc::new(StdMutex::new(None));
        let seen_h = seen.clone();
        let svc_h = svc.clone();
        port.set_on_inject(Arc::new(move |_| {
            *seen_h.lock().unwrap() = Some((
                svc_h.in_flight_len(tid("recv")),
                svc_h.parked_len(tid("recv")),
                svc_h.ledger_statuses("m1"),
            ));
        }));

        let out = send(&svc, "m1", from, "alice", &["recv"]).expect("행 응답");
        assert_eq!(
            out[0].status,
            SendStatus::Delivered,
            "전제: 빈 큐 + idle 갈래여야 이 반사실이 의미를 갖는다: {out:?}"
        );

        let (in_flight, queued, statuses) = seen.lock().unwrap().clone().expect("hook 이 돌았다");
        assert_eq!(
            in_flight, 1,
            "★직발송 갈래가 되살아나면 여기가 0 이 된다★ — 주입 중인 편지는 큐에서 빠져 나온 몫으로 회계에 잡혀 있어야 한다"
        );
        assert_eq!(
            queued, 0,
            "빠져 나왔으니 큐엔 없다(①이 '아직 안 뺐다' 가 아님을 고정)"
        );
        assert_eq!(
            statuses,
            vec![DeliveryStatus::Pending],
            "★적재가 락 안에서 남긴 행★ — 직발송이면 이 시점에 행이 아예 없거나 이미 delivered 다"
        );

        assert_eq!(svc.ledger_statuses("m1"), vec![DeliveryStatus::Delivered]);
        assert_eq!(svc.parked_len(tid("recv")), 0);
        assert_eq!(svc.in_flight_len(tid("recv")), 0, "영수증 정산 누락 없음");
    }

    // ── round-7 high → ADR-0125: does not overtake an in-progress batch ──────────────────────────────────

    #[test]
    fn a_send_arriving_mid_batch_is_appended_and_delivered_after_it() {
        let (svc, port, gate) = svc_gated();
        let (recv_id, recv) = live("recv");
        port.set_roster(vec![recv]);
        gate.set_busy(recv_id, 0);
        for (i, m) in ["m0", "m1"].iter().enumerate() {
            svc.park_absent_for_test(
                m,
                ident(),
                "s",
                "recv",
                &format!("b{i}"),
                Entrance::Mcp,
                &SendMeta::default(),
            )
            .expect("park");
        }
        gate.clear();

        // A new send comes in **at the very moment** the first item (b0) is written to the recipient's stdin.
        let svc_hook = svc.clone();
        let outcome: Arc<StdMutex<Option<SendOutcome>>> = Arc::new(StdMutex::new(None));
        let outcome_hook = outcome.clone();
        port.set_on_inject(Arc::new(move |idx| {
            if idx != 0 {
                return;
            }
            let out = svc_hook
                .park_absent_for_test(
                    "m2",
                    ident(),
                    "s",
                    "recv",
                    "b2",
                    Entrance::Mcp,
                    &SendMeta::default(),
                )
                .expect("no reject");
            *outcome_hook.lock().unwrap() = Some(out);
        }));

        svc.flush_for(recv_id);

        assert!(
            matches!(
                outcome.lock().unwrap().as_ref(),
                Some(SendOutcome::Parked { .. })
            ),
            "주입 중인 배치가 있으면 새 발송은 꼬리에 적재되고 그 드레인은 물러난다(응답 pending): {:?}",
            outcome.lock().unwrap()
        );
        assert_eq!(
            port.injected_bodies(),
            vec![
                r#"<message from="s">b0</message>"#.to_string(),
                r#"<message from="s">b1</message>"#.to_string(),
                r#"<message from="s">b2</message>"#.to_string(),
            ],
            "수신자가 보는 순서 = 적재 순서(새 발송이 진행 중인 배치를 앞지르지 않는다)"
        );
        assert_eq!(
            svc.parked_len(tid("recv")),
            0,
            "물러났던 몫도 결국 배달된다(고립 없음)"
        );
        assert_eq!(svc.in_flight_len(tid("recv")), 0, "영수증 정산 누수 없음");
        assert_eq!(svc.ledger_statuses("m2"), vec![DeliveryStatus::Delivered]);
    }

    #[test]
    fn a_second_flush_defers_while_a_batch_is_in_flight_and_the_settlement_re_rings() {
        let (svc, port, gate, bell) = svc_gated_with_doorbell();
        let (recv_id, recv) = live("recv");
        port.set_roster(vec![recv]);
        gate.set_busy(recv_id, 0);
        for (i, m) in ["m0", "m1"].iter().enumerate() {
            svc.park_absent_for_test(
                m,
                ident(),
                "s",
                "recv",
                &format!("b{i}"),
                Entrance::Mcp,
                &SendMeta::default(),
            )
            .expect("park");
        }
        gate.clear();

        let svc_hook = svc.clone();
        let port_hook = port.clone();
        let bell_hook = bell.clone();
        let probe: Arc<StdMutex<(usize, usize)>> = Arc::new(StdMutex::new((0, 0)));
        let probe_hook = probe.clone();
        port.set_on_inject(Arc::new(move |idx| {
            if idx != 0 {
                return;
            }
            let before = port_hook.injected_bodies().len();
            svc_hook
                .park_absent_for_test(
                    "m2",
                    ident(),
                    "s",
                    "recv",
                    "b2",
                    Entrance::Mcp,
                    &SendMeta::default(),
                )
                .expect("no reject");
            svc_hook.flush_for(recv_id);
            let after = port_hook.injected_bodies().len();
            *probe_hook.lock().unwrap() = (after - before, bell_hook.seen().len());
        }));

        svc.flush_for(recv_id);

        let (overlapped_injects, bells_at_hook) = *probe.lock().unwrap();
        assert_eq!(
            overlapped_injects, 0,
            "겹친 flush 는 드레인도 주입도 하지 않는다(유예)"
        );
        assert_eq!(
            port.injected_bodies(),
            vec![
                r#"<message from="s">b0</message>"#.to_string(),
                r#"<message from="s">b1</message>"#.to_string(),
            ],
            "이 조립엔 도어벨 소비자가 없으므로 합류분은 아직 큐에 있다"
        );
        assert_eq!(
            svc.parked_msg_ids(tid("recv")),
            vec!["m2".to_string()],
            "유예분이 사라지지 않는다(무손실)"
        );
        assert_eq!(svc.in_flight_len(tid("recv")), 0, "영수증 정산 누수 없음");
        assert_eq!(
            bell.seen().len(),
            bells_at_hook + 1,
            "정산을 마치며 유예된 flush 를 정확히 1회 되울린다(lost wakeup 금지)"
        );
        // ★Here there is **only one** id that deferred, so the echo is also 1 (the same value as in the single-slot days)★ — that the marker is
        //   a set is enforced not by this test but by the stale-id test below (round-8).
        assert_eq!(
            bell.seen().last().copied(),
            Some(recv_id),
            "되울림 대상 = 유예한 쪽이 열려던 큐의 도어벨 id"
        );
        svc.flush_for(recv_id);
        assert_eq!(
            port.injected_bodies(),
            vec![
                r#"<message from="s">b0</message>"#.to_string(),
                r#"<message from="s">b1</message>"#.to_string(),
                r#"<message from="s">b2</message>"#.to_string(),
            ],
        );
        assert_eq!(svc.parked_len(tid("recv")), 0);
    }

    #[test]
    fn busy_park_rings_doorbell_instead_of_flushing_on_sender_thread() {
        let (svc, port, gate, bell) = svc_gated_with_doorbell();
        let (recv_id, recv) = live("recv");
        port.set_roster(vec![recv]);
        gate.set_busy(recv_id, 0);
        svc.park_absent_for_test(
            "m1",
            ident(),
            "s",
            "recv",
            "hi",
            Entrance::Mcp,
            &SendMeta::default(),
        )
        .expect("park");
        assert_eq!(
            bell.seen(),
            vec![recv_id],
            "도어벨 요청 1건(무조건 enqueue)"
        );
        assert!(
            port.injected_bodies().is_empty(),
            "발신 스레드에서 주입하지 않는다(소비는 flush lane)"
        );
        assert_eq!(svc.parked_len(tid("recv")), 1, "파킹 유지");
        gate.clear();
        svc.flush_for_agent(recv_id);
        assert_eq!(svc.ledger_statuses("m1"), vec![DeliveryStatus::Delivered]);
    }

    // ── Even after a rename, that id's letter goes to that agent (ADR-0309 decisions 1 · 5) ────────────────────────
    /// Park for busy live X → X renames mid-turn → the turn-end flush opens that id's queue. The queue key is an id, so a rename cannot
    ///   orphan the queue (the outcome of round-3 finding 2 — back then it was blocked by sweeping the name queue with an id hint).
    #[test]
    fn a_rename_mid_turn_still_delivers_the_mail_parked_on_that_id() {
        let (svc, port, gate) = svc_gated();
        let (id, agent) = live_distinct("old-name");
        port.set_roster(vec![agent]);
        gate.set_busy(id, 0);
        svc.park_absent_for_test(
            "m1",
            ident(),
            "s",
            "old-name",
            "hi",
            Entrance::Mcp,
            &SendMeta::default(),
        )
        .expect("park");
        assert_eq!(svc.parked_len(id), 1, "그 에이전트 id 큐에 파킹");

        port.set_roster(vec![LiveAgent {
            id,
            name: "new-name".to_string(),
            epoch: 0,
            turn_signal: true,
        }]);
        gate.clear();
        svc.flush_for_agent(id);

        assert_eq!(
            port.injected_bodies(),
            vec![r#"<message from="s">hi</message>"#.to_string()],
            "개명 뒤 턴 종료 flush 가 그 id 큐를 연다"
        );
        assert_eq!(port.injected_targets(), vec![id]);
        assert_eq!(svc.parked_len(id), 0, "큐 비움");
        assert_eq!(svc.ledger_statuses("m1"), vec![DeliveryStatus::Delivered]);
    }

    /// ★B1★ A letter entrusted to asleep X (name a) still goes to X even if X renames to b while asleep and wakes up as b. When it was a name queue
    ///   it was stuck in the a queue and became `expired` by TTL.
    #[test]
    fn mail_parked_for_a_dormant_agent_reaches_it_after_a_rename_while_asleep() {
        let (svc, port) = svc();
        let (from, me) = live_sender("alice");
        let x_id = PeerId::new_v4();
        port.set_roster(vec![me.clone()]);
        port.set_dormant_agents(&[(x_id, "a")]);
        let out = send_body(&svc, "m1", from, "alice", &["a"], "hi").expect("행 응답");
        assert_eq!(
            out[0].status,
            SendStatus::Pending,
            "잠든 수신자 → 파킹: {out:?}"
        );
        assert_eq!(svc.parked_len(x_id), 1, "큐 키 = 잠든 X 의 id");

        // Rename to b while asleep → wake up as b and appearance flush.
        port.set_dormant_agents(&[]);
        port.set_roster(vec![me, live_with_id(x_id, "b").1]);
        svc.flush_for(x_id);

        assert_eq!(port.injected_targets(), vec![x_id]);
        assert_eq!(
            port.injected_bodies(),
            vec![r#"<message from="alice">hi</message>"#.to_string()],
            "개명한 X 가 받는다(B1)"
        );
        assert_eq!(svc.parked_len(x_id), 0);
        assert_eq!(svc.ledger_statuses("m1"), vec![DeliveryStatus::Delivered]);
    }

    /// ★B2★ Even if, after X renames a → b while asleep, another profile Y wakes up as a and flushes, X's letter does not go to Y.
    ///   After that, when X wakes, X receives it. Reviving the branch that looks up by name instead makes Y receive someone else's letter.
    #[test]
    fn a_new_holder_of_the_old_name_does_not_take_mail_parked_for_the_renamed_agent() {
        let (svc, port) = svc();
        let (from, me) = live_sender("alice");
        let x_id = PeerId::new_v4();
        port.set_roster(vec![me.clone()]);
        port.set_dormant_agents(&[(x_id, "a")]);
        let out = send_body(&svc, "m1", from, "alice", &["a"], "hi").expect("행 응답");
        assert_eq!(
            out[0].status,
            SendStatus::Pending,
            "잠든 수신자 → 파킹: {out:?}"
        );

        port.set_dormant_agents(&[(x_id, "b")]);
        let (y_id, y) = live_distinct("a");
        port.set_roster(vec![me.clone(), y.clone()]);
        svc.flush_for(y_id);
        svc.flush_for_agent(y_id);
        svc.flush_for(x_id);
        assert!(
            port.injected_bodies().is_empty(),
            "옛 이름을 새로 가진 Y 는 X 의 편지를 받지 않는다(B2)"
        );
        assert_eq!(svc.parked_len(x_id), 1, "X 의 큐에 그대로");
        assert_eq!(svc.parked_len(y_id), 0);
        assert_eq!(svc.ledger_statuses("m1"), vec![DeliveryStatus::Pending]);

        port.set_dormant_agents(&[]);
        port.set_roster(vec![me, y, live_with_id(x_id, "b").1]);
        svc.flush_for(x_id);
        assert_eq!(port.injected_targets(), vec![x_id], "X 가 깨면 X 가 받는다");
        assert_eq!(svc.ledger_statuses("m1"), vec![DeliveryStatus::Delivered]);
    }

    // ── Same-name twins have separate queues — one side's busy · injection failure does not hold the other's letter (ADR-0309 decision 1) ──
    /// In the name-queue days, letters that targeted twins by exact id mixed into one name queue, and within one batch a per-target gate ·
    ///   per-target restore was needed separately (round-3 finding 3). The queue key is an id, so the batch's target is always one — that outcome
    ///   is pinned: each has its own queue · its own order.
    #[test]
    fn same_named_twins_keep_separate_queues_so_one_busy_or_failing_never_holds_the_other() {
        let (svc, port, gate) = svc_gated();
        let (id_a, dup_a) = live_distinct("dup");
        let (id_b, dup_b) = live_distinct("dup");
        port.set_roster(vec![dup_a, dup_b]);
        gate.set_busy(id_a, 0);
        gate.set_busy(id_b, 0);
        for (to, msg_id) in [(id_a, "m0"), (id_b, "m1"), (id_a, "m2"), (id_b, "m3")] {
            svc.park_absent_for_test(
                msg_id,
                ident(),
                "s",
                &to.to_string(),
                &format!("body-{msg_id}"),
                Entrance::Mcp,
                &SendMeta::default(),
            )
            .expect("park");
        }
        assert_eq!(
            svc.parked_msg_ids(id_a),
            vec!["m0", "m2"],
            "지목한 id 의 큐에만 쌓인다"
        );
        assert_eq!(svc.parked_msg_ids(id_b), vec!["m1", "m3"]);

        // A is free · B is busy. A's first injection fails.
        gate.clear();
        gate.set_busy(id_b, 0);
        port.fail_at(&[0]);
        svc.flush_for(id_a);
        assert!(
            port.injected_bodies().is_empty(),
            "A 의 배치가 첫 항목에서 끊겼다"
        );
        assert_eq!(
            svc.parked_msg_ids(id_a),
            vec!["m0", "m2"],
            "실패분은 오래된 순 그대로 A 큐로 돌아온다"
        );
        assert_eq!(
            svc.parked_msg_ids(id_b),
            vec!["m1", "m3"],
            "A 의 실패가 B 큐를 건드리지 않는다"
        );

        port.fail_at(&[]);
        svc.flush_for(id_a);
        assert_eq!(
            port.injected_bodies(),
            vec![
                r#"<message from="s">body-m0</message>"#.to_string(),
                r#"<message from="s">body-m2</message>"#.to_string(),
            ],
            "B 가 바빠도 A 몫은 나간다"
        );
        assert_eq!(port.injected_targets(), vec![id_a, id_a]);
        assert_eq!(svc.ledger_statuses("m1"), vec![DeliveryStatus::Pending]);
        assert_eq!(svc.ledger_statuses("m3"), vec![DeliveryStatus::Pending]);

        svc.flush_for(id_b);
        assert_eq!(port.injected_bodies().len(), 2, "턴 중인 B 에는 안 든다");

        gate.clear();
        svc.flush_for(id_b);
        assert_eq!(
            port.injected_bodies()[2..],
            [
                r#"<message from="s">body-m1</message>"#.to_string(),
                r#"<message from="s">body-m3</message>"#.to_string(),
            ],
            "B 몫도 오래된 순"
        );
        assert_eq!(port.injected_targets()[2..], [id_b, id_b]);
        assert_eq!(svc.parked_total_for_test(), 0);
    }

    // ── round-3 finding 4: the busy cap sweep releases an unwakeable busy and delivers the waiting mail ────────────
    struct RecordingIdle {
        seen: StdMutex<Vec<PeerId>>,
    }
    impl super::super::busy::IdleNotifier for RecordingIdle {
        fn notify_idle(&self, id: PeerId) {
            self.seen.lock().unwrap().push(id);
        }
    }

    #[test]
    fn stale_busy_sweep_unblocks_parked_mail() {
        use super::super::busy::{BusyPolicy, ScriptedTurnFacts, BUSY_MAX_TURN};
        let port = Arc::new(FakeDeliveryPort::new());
        let notifier = Arc::new(RecordingIdle {
            seen: StdMutex::new(Vec::new()),
        });
        let facts = ScriptedTurnFacts::new();
        let gate = Arc::new(BusyPolicy::new(facts.clone(), notifier.clone()));
        let svc = Arc::new(MessagingService::new_gated(
            port.clone(),
            Arc::new(FakeControlPlane),
            gate.clone(),
        ));
        let (id, agent) = live("recv");
        port.set_roster(vec![agent]);

        // Time is injected — avoids waiting a real 30 minutes.
        let t0 = Instant::now();
        facts.set_in_turn(id, 0, t0);
        svc.park_absent_for_test(
            "m1",
            ident(),
            "s",
            "recv",
            "hi",
            Entrance::Mcp,
            &SendMeta::default(),
        )
        .expect("park");
        assert_eq!(svc.parked_len(tid("recv")), 1, "busy 라 파킹");
        assert!(
            port.injected_bodies().is_empty(),
            "턴 중이므로 주입 없음(정상)"
        );

        assert_eq!(
            gate.sweep_stale_busy(t0 + BUSY_MAX_TURN + Duration::from_secs(1)),
            1,
            "상한 초과 잔해를 깨운다"
        );
        assert_eq!(
            notifier.seen.lock().unwrap().clone(),
            vec![id],
            "깨워야 대기 메일이 나간다(판정만 뒤집으면 다음 트리거가 없다)"
        );
        // In production the flush lane consumes that doorbell — here the consumption is performed directly.
        svc.flush_for_agent(id);
        assert_eq!(
            port.injected_bodies(),
            vec![r#"<message from="s">hi</message>"#.to_string()],
            "상한 sweep 이후 배달됨(TTL blackhole 해소)"
        );
        assert_eq!(svc.ledger_statuses("m1"), vec![DeliveryStatus::Delivered]);
    }

    /// A doorbell with only an empty list · a turn end whose stall was released — either way the flush lane asks about busy again and drains the parking.
    /// Until then even the 30-minute cap does not release it.
    // ADR-0231
    #[test]
    fn parked_mail_waits_out_the_list_and_the_halt_then_the_doorbell_delivers() {
        use super::super::busy::{BusyPolicy, ScriptedTurnFacts, BUSY_MAX_TURN};
        let port = Arc::new(FakeDeliveryPort::new());
        let notifier = Arc::new(RecordingIdle {
            seen: StdMutex::new(Vec::new()),
        });
        let facts = ScriptedTurnFacts::new();
        let gate = Arc::new(BusyPolicy::new(facts.clone(), notifier.clone()));
        let svc = Arc::new(MessagingService::new_gated(
            port.clone(),
            Arc::new(FakeControlPlane),
            gate.clone(),
        ));
        let (id, agent) = live("recv");
        port.set_roster(vec![agent]);

        let t0 = Instant::now();
        facts.set_idle(id, 0, t0);
        facts.set_inputs_pending(id, 0, true, t0);
        facts.set_last_end_failed(id, 0, true, t0);
        svc.park_absent_for_test(
            "m1",
            ident(),
            "s",
            "recv",
            "hi",
            Entrance::Mcp,
            &SendMeta::default(),
        )
        .expect("park");
        assert_eq!(svc.parked_len(tid("recv")), 1, "목록·멈춤이라 파킹");

        assert_eq!(
            gate.sweep_stale_busy(t0 + BUSY_MAX_TURN + Duration::from_secs(1)),
            0,
            "상한은 목록·멈춤을 깨우지 않는다"
        );
        svc.flush_for_agent(id);
        assert!(
            port.injected_bodies().is_empty(),
            "목록이 남은 동안 안 든다"
        );

        facts.set_inputs_pending(id, 0, false, t0);
        svc.flush_for_agent(id);
        assert!(
            port.injected_bodies().is_empty(),
            "목록이 비어도 오류 뒤 멈춤이면 안 든다"
        );

        facts.set_last_end_failed(id, 0, false, t0);
        svc.flush_for_agent(id);
        assert_eq!(
            port.injected_bodies(),
            vec![r#"<message from="s">hi</message>"#.to_string()],
            "풀린 뒤의 초인종이 파킹을 흘린다"
        );
    }

    #[test]
    fn flush_for_agent_is_noop_without_parked_and_opens_its_own_id_queue() {
        let (svc, port, _gate) = svc_gated();
        let (recv_id, recv) = live("recv");
        port.set_roster(vec![recv]);
        svc.flush_for_agent(recv_id);
        assert!(port.injected_bodies().is_empty(), "파킹 없으면 no-op");
        port.set_roster(vec![]);
        svc.park_absent_for_test(
            "m1",
            ident(),
            "s",
            "recv",
            "hi",
            Entrance::Mcp,
            &SendMeta::default(),
        )
        .expect("park");
        let (_again, recv2) = live("recv");
        let recv2 = LiveAgent {
            id: recv_id,
            ..recv2
        };
        port.set_roster(vec![recv2]);
        svc.flush_for_agent(recv_id);
        assert_eq!(svc.ledger_statuses("m1"), vec![DeliveryStatus::Delivered]);
    }

    #[test]
    fn flush_for_agent_unknown_id_is_noop() {
        let (svc, port, _gate) = svc_gated();
        port.set_roster(vec![]);
        svc.park_absent_for_test(
            "m1",
            ident(),
            "s",
            "ghost",
            "hi",
            Entrance::Mcp,
            &SendMeta::default(),
        )
        .expect("park");
        svc.flush_for_agent(PeerId::new_v4());
        assert_eq!(
            svc.parked_len(tid("ghost")),
            1,
            "미존재 id flush 는 무해한 no-op"
        );
        assert!(port.injected_bodies().is_empty());
    }
    // ── C3: reply contract (request/reply/notice — spec §2·§3 · ADR-0103 decisions 2/3) ─────────────────

    fn req_meta(raw: &str, secs: u64) -> SendMeta {
        SendMeta {
            request: true,
            reply_by_raw: Some(raw.to_string()),
            reply_by: Some(Duration::from_secs(secs)),
            reply_to: None,
            to_attr: None,
        }
    }

    fn reply_meta(in_reply_to: &str) -> SendMeta {
        SendMeta {
            request: false,
            reply_by_raw: None,
            reply_by: None,
            reply_to: Some(in_reply_to.to_string()),
            to_attr: None,
        }
    }

    #[test]
    fn notice_uses_the_senders_own_notation_not_a_normalized_one() {
        let (svc, port) = svc();
        let (_a, alice) = live("alice");
        port.set_roster(vec![alice]);
        svc.park_absent_for_test(
            "m-60m",
            reply_from(tid("alice")),
            "alice",
            "ghost",
            "해줘",
            Entrance::Mcp,
            &req_meta("60m", 3600),
        )
        .expect("parked");
        svc.sweep(Instant::now() + Duration::from_secs(3601));
        assert_eq!(
            port.injected_bodies(),
            vec![
                "<notice>[engram] 요청 m-60m 기한(60m) 초과 — ghost 회신 없음</notice>".to_string()
            ],
            "봉투에 쓴 표기 그대로(1h 로 정규화 금지)"
        );
    }

    #[test]
    fn delivered_request_opens_contract_and_renders_request_envelope() {
        // ★Golden★: the request envelope attribute order is fixed by spec §1.
        let (svc, port) = svc();
        let (_id, alice) = live("alice");
        port.set_roster(vec![alice]);
        let out = svc
            .park_absent_for_test(
                "m-req1",
                ident(),
                "qa-alpha",
                "alice",
                "코드 짜고 회신해",
                Entrance::Mcp,
                &req_meta("10m", 600),
            )
            .expect("delivered");
        assert_eq!(out, SendOutcome::Delivered);
        assert_eq!(
            port.injected_bodies(),
            vec![concat!(
                r#"<message from="qa-alpha" id="m-req1" type="request" reply-by="10m">"#,
                "코드 짜고 회신해</message>"
            )
            .to_string()],
            "request 봉투 골든(속성 순서·kebab-case)"
        );
        assert_eq!(
            svc.open_request_count(),
            1,
            "배달된 request 는 계약이 열린다"
        );
        assert_eq!(
            svc.ledger_statuses("m-req1"),
            vec![DeliveryStatus::Delivered]
        );
    }

    #[test]
    fn parked_request_opens_contract_too_and_keeps_envelope_on_flush() {
        // ★spec §3 step 2★: the envelope attributes must be the same even when delivered late — otherwise the receiver does not know the id to reply to and
        //   only the sender receives a timeout notice.
        let (svc, port) = svc();
        port.set_roster(vec![]);
        let out = svc
            .park_absent_for_test(
                "m-req2",
                ident(),
                "qa-alpha",
                "bob",
                "하고 알려줘",
                Entrance::Mcp,
                &req_meta("1h", 3600),
            )
            .expect("parked");
        assert!(matches!(out, SendOutcome::Parked { .. }));
        assert_eq!(
            svc.open_request_count(),
            1,
            "파킹된 request 도 계약이 열린다"
        );
        assert_eq!(svc.ledger_statuses("m-req2"), vec![DeliveryStatus::Pending]);

        let (bob_id, bob) = live("bob");
        port.set_roster(vec![bob]);
        svc.flush_for(bob_id);
        assert_eq!(
            port.injected_bodies(),
            vec![concat!(
                r#"<message from="qa-alpha" id="m-req2" type="request" reply-by="1h">"#,
                "하고 알려줘</message>"
            )
            .to_string()],
            "파킹→flush 배달도 같은 request 봉투(C3 ParkPayload meta 보존)"
        );
        assert_eq!(
            svc.ledger_statuses("m-req2"),
            vec![DeliveryStatus::Delivered]
        );
    }

    #[test]
    fn reply_envelope_carries_in_reply_to_immediate_and_flushed() {
        // ★Golden (reply)★: the send argument reply_to → the receive attribute in-reply-to (spec §1 notation mapping). There is no id/type
        //   (exposure principle — a reply is not something to reply to).
        let (svc, port) = svc();
        let (_id, alice) = live("alice");
        port.set_roster(vec![alice]);
        svc.park_absent_for_test(
            "m-rep1",
            ident(),
            "qa-bravo",
            "alice",
            "다 짰음",
            Entrance::Mcp,
            &reply_meta("m-7f3k"),
        )
        .expect("delivered");
        assert_eq!(
            port.injected_bodies(),
            vec![r#"<message from="qa-bravo" in-reply-to="m-7f3k">다 짰음</message>"#.to_string()],
        );

        let (svc2, port2) = super::tests::svc();
        port2.set_roster(vec![]);
        svc2.park_absent_for_test(
            "m-rep2",
            ident(),
            "qa-bravo",
            "carol",
            "늦은 회신",
            Entrance::Cli,
            &reply_meta("m-7f3k"),
        )
        .expect("parked");
        let (carol_id, carol) = live("carol");
        port2.set_roster(vec![carol]);
        svc2.flush_for(carol_id);
        assert_eq!(
            port2.injected_bodies(),
            vec![
                r#"<message from="qa-bravo" in-reply-to="m-7f3k">늦은 회신</message>"#.to_string()
            ],
            "파킹→flush 회신도 in-reply-to 를 유지"
        );
    }

    #[test]
    fn plain_send_envelope_is_unchanged_by_c3() {
        let (svc, port) = svc();
        let (_id, alice) = live("alice");
        port.set_roster(vec![alice]);
        svc.park_absent_for_test(
            "m1",
            ident(),
            "s",
            "alice",
            "hi",
            Entrance::Mcp,
            &SendMeta::default(),
        )
        .expect("delivered");
        assert_eq!(
            port.injected_bodies(),
            vec![r#"<message from="s">hi</message>"#.to_string()]
        );
        assert_eq!(svc.open_request_count(), 0, "통보는 계약을 열지 않는다");
    }

    #[test]
    fn correct_reply_id_closes_contract_and_marks_replied() {
        let (svc, port) = svc();
        let (_a, alice) = live("alice");
        let (b_id, bob) = live("bob");
        port.set_roster(vec![alice, bob]);
        svc.park_absent_for_test(
            "m-req",
            ident(),
            "alice",
            "bob",
            "해줘",
            Entrance::Mcp,
            &req_meta("10m", 600),
        )
        .expect("delivered");
        assert_eq!(svc.open_request_count(), 1);

        svc.park_absent_for_test(
            "m-rep",
            reply_from(b_id),
            "bob",
            "alice",
            "했음",
            Entrance::Mcp,
            &reply_meta("m-req"),
        )
        .expect("delivered");
        assert_eq!(
            svc.open_request_count(),
            0,
            "정확한 id 회신은 계약을 닫는다"
        );
        assert_eq!(
            svc.ledger_statuses("m-req"),
            vec![DeliveryStatus::Replied],
            "원본 레코드가 Replied 로 전이"
        );
    }

    #[test]
    fn wrong_reply_id_still_delivers_but_closes_nothing() {
        // ★spec §2 strict matching★: escalating a wrong id to a bounce attaches a retry to an already-delivered message and causes a duplicate —
        //   so it is not closed, but the delivery and response shape are left as is.
        let (svc, port) = svc();
        let (_a, alice) = live("alice");
        let (_b, bob) = live("bob");
        port.set_roster(vec![alice, bob]);
        svc.park_absent_for_test(
            "m-req",
            ident(),
            "alice",
            "bob",
            "해줘",
            Entrance::Mcp,
            &req_meta("10m", 600),
        )
        .expect("delivered");

        let out = svc
            .park_absent_for_test(
                "m-rep",
                ident(),
                "bob",
                "alice",
                "엉뚱한 id 로 회신",
                Entrance::Mcp,
                &reply_meta("m-nope"),
            )
            .expect("회신은 정상 배달돼야");
        assert_eq!(out, SendOutcome::Delivered);
        assert_eq!(svc.open_request_count(), 1, "틀린 id 는 계약을 못 닫는다");
        assert_eq!(
            svc.ledger_statuses("m-req"),
            vec![DeliveryStatus::Delivered],
            "원본은 여전히 미회신"
        );
        assert_eq!(
            port.injected_bodies().len(),
            2,
            "request + 회신 둘 다 실제로 주입됐다"
        );
    }

    #[test]
    fn second_reply_to_closed_request_is_noop_and_still_delivers() {
        let (svc, port) = svc();
        let (_a, alice) = live("alice");
        let (b_id, bob) = live("bob");
        port.set_roster(vec![alice, bob]);
        svc.park_absent_for_test(
            "m-req",
            ident(),
            "alice",
            "bob",
            "해줘",
            Entrance::Mcp,
            &req_meta("10m", 600),
        )
        .expect("delivered");
        svc.park_absent_for_test(
            "m-r1",
            reply_from(b_id),
            "bob",
            "alice",
            "1차",
            Entrance::Mcp,
            &reply_meta("m-req"),
        )
        .expect("delivered");
        svc.park_absent_for_test(
            "m-r2",
            reply_from(b_id),
            "bob",
            "alice",
            "2차",
            Entrance::Mcp,
            &reply_meta("m-req"),
        )
        .expect("2차 회신도 배달돼야");
        assert_eq!(svc.open_request_count(), 0);
        assert_eq!(port.injected_bodies().len(), 3, "3건 모두 주입");
    }

    #[test]
    fn parked_request_reply_closes_contract_even_before_delivery() {
        let (svc, port) = svc();
        let (_a, alice) = live("alice");
        port.set_roster(vec![alice]);
        svc.park_absent_for_test(
            "m-req",
            ident(),
            "alice",
            "ghost",
            "해줘",
            Entrance::Mcp,
            &req_meta("10m", 600),
        )
        .expect("parked");
        assert_eq!(svc.open_request_count(), 1);
        svc.park_absent_for_test(
            "m-rep",
            reply_from(tid("ghost")),
            "ghost",
            "alice",
            "했음",
            Entrance::Mcp,
            &reply_meta("m-req"),
        )
        .expect("delivered");
        assert_eq!(svc.open_request_count(), 0, "계약은 닫힌다");
        assert_eq!(
            svc.ledger_statuses("m-req"),
            vec![DeliveryStatus::Pending],
            "이력은 미주입 상태 유지(불법 간선 — anomaly 로 관측만)"
        );
    }

    #[test]
    fn reply_by_timeout_delivers_notice_to_sender_exactly_once() {
        // ★spec §3 step 4 · §1 notice template★: overdue → a notice **to the sender**.
        let (svc, port) = svc();
        let (_a, alice) = live("alice");
        port.set_roster(vec![alice]);
        svc.park_absent_for_test(
            "m-7f3k",
            reply_from(tid("alice")),
            "alice",
            "qa-bravo",
            "해줘",
            Entrance::Mcp,
            &req_meta("10m", 600),
        )
        .expect("수신자 부재 → 파킹(계약은 열림)");
        assert!(port.injected_bodies().is_empty(), "부재라 아직 주입 없음");

        svc.sweep(Instant::now() + Duration::from_secs(601));
        assert_eq!(
            port.injected_bodies(),
            vec![
                "<notice>[engram] 요청 m-7f3k 기한(10m) 초과 — qa-bravo 회신 없음</notice>"
                    .to_string()
            ],
            "notice 골든(from 속성 없음 = 회신 대상 아님)"
        );

        svc.sweep(Instant::now() + Duration::from_secs(1200));
        assert_eq!(port.injected_bodies().len(), 1, "notice 는 정확히 1회");
    }

    #[test]
    fn replied_request_never_produces_a_timeout_notice() {
        let (svc, port) = svc();
        let (_a, alice) = live("alice");
        let (b_id, bob) = live("bob");
        port.set_roster(vec![alice, bob]);
        svc.park_absent_for_test(
            "m-req",
            ident(),
            "alice",
            "bob",
            "해줘",
            Entrance::Mcp,
            &req_meta("1m", 60),
        )
        .expect("delivered");
        svc.park_absent_for_test(
            "m-rep",
            reply_from(b_id),
            "bob",
            "alice",
            "했음",
            Entrance::Mcp,
            &reply_meta("m-req"),
        )
        .expect("delivered");
        svc.sweep(Instant::now() + Duration::from_secs(600));
        assert_eq!(
            port.injected_bodies().len(),
            2,
            "회신된 계약엔 notice 가 없다(request + reply 2건뿐)"
        );
    }

    #[test]
    fn request_without_reply_by_never_times_out() {
        let (svc, port) = svc();
        let (_a, alice) = live("alice");
        port.set_roster(vec![alice]);
        let meta = SendMeta {
            request: true,
            reply_by_raw: None,
            reply_by: None,
            reply_to: None,
            to_attr: None,
        };
        svc.park_absent_for_test(
            "m-req",
            ident(),
            "alice",
            "ghost",
            "해줘",
            Entrance::Mcp,
            &meta,
        )
        .expect("parked");
        svc.sweep(Instant::now() + Duration::from_secs(100_000));
        assert!(
            port.injected_bodies().is_empty(),
            "기한 없는 request 는 타임아웃 없음"
        );
        assert_eq!(svc.open_request_count(), 1, "계약은 열린 채 유지");
    }

    #[test]
    fn timeout_notice_parks_in_its_own_lane_when_sender_is_absent_and_is_ledgered() {
        let (svc, port) = svc();
        port.set_roster(vec![]);
        for i in 0..100 {
            svc.park_absent_for_test(
                &format!("m{i}"),
                ident(),
                "s",
                "alice",
                "x",
                Entrance::Mcp,
                &SendMeta::default(),
            )
            .expect("cap 이내");
        }
        assert_eq!(
            svc.park_absent_for_test(
                "over",
                ident(),
                "s",
                "alice",
                "x",
                Entrance::Mcp,
                &SendMeta::default()
            ),
            Err(SendFail::Row(FailCode::MailboxFull)),
            "message 는 cap 에서 반려(대조군)"
        );

        svc.park_absent_for_test(
            "m-req",
            reply_from(tid("alice")),
            "alice",
            "bob",
            "해줘",
            Entrance::Mcp,
            &req_meta("1m", 60),
        )
        .expect("parked");
        svc.sweep(Instant::now() + Duration::from_secs(61));

        assert_eq!(
            svc.parked_len(tid("alice")),
            101,
            "message 백로그가 통지를 막지 않는다(100 message + 1 notice — 레인이 다르다)"
        );
        let noticed: Vec<_> = svc
            .ledger_snapshot()
            .into_iter()
            .filter(|(_, from, _, _)| from == NOTICE_SENDER_LABEL)
            .collect();
        assert_eq!(
            noticed.len(),
            1,
            "notice 도 장부에 남는다(조용한 유실 금지)"
        );
        assert_eq!(noticed[0].2, "alice", "notice 수신자 = 요청 발신자");
        assert_eq!(noticed[0].3, DeliveryStatus::Pending, "파킹이므로 pending");
    }

    #[test]
    fn the_notice_lane_is_bounded_and_retires_its_oldest_to_the_ledger() {
        /// A mirror of mailbox's private `NOTICE_CAP` (value in sync).
        const NOTICE_CAP_FOR_TEST: usize = 64;
        let (svc, port) = svc();
        port.set_roster(vec![]); // Both alice (sender) and bob (recipient) are absent.
        for i in 0..(NOTICE_CAP_FOR_TEST + 1) {
            svc.park_absent_for_test(
                &format!("req{i}"),
                reply_from(tid("alice")),
                "alice",
                "bob",
                "해줘",
                Entrance::Mcp,
                &req_meta("1m", 60),
            )
            .expect("parked");
        }
        svc.sweep(Instant::now() + Duration::from_secs(61));

        assert_eq!(
            svc.parked_len(tid("alice")),
            NOTICE_CAP_FOR_TEST,
            "통지는 자기 레인 상한에서 멈춘다(옛 구현 = cap+1 — 무계)"
        );
        let noticed: Vec<_> = svc
            .ledger_snapshot()
            .into_iter()
            .filter(|(_, from, _, _)| from == NOTICE_SENDER_LABEL)
            .collect();
        assert_eq!(
            noticed.len(),
            NOTICE_CAP_FOR_TEST + 1,
            "밀려난 통지도 장부에 남는다(조용한 유실 0 — 반려가 아니라 회수)"
        );
        assert_eq!(
            noticed[0].3,
            DeliveryStatus::Skipped,
            "가장 오래된 통지가 회수돼 skipped(신규 통지는 절대 반려되지 않는다)"
        );
        assert!(
            noticed[1..]
                .iter()
                .all(|(_, _, _, s)| *s == DeliveryStatus::Pending),
            "나머지는 큐에 남아 pending"
        );
    }

    #[test]
    fn parked_notice_flushes_as_notice_tag_and_marks_delivered() {
        let (svc, port) = svc();
        port.set_roster(vec![]);
        svc.park_absent_for_test(
            "m-9x",
            reply_from(tid("alice")),
            "alice",
            "bob",
            "해줘",
            Entrance::Mcp,
            &req_meta("1m", 60),
        )
        .expect("parked");
        svc.sweep(Instant::now() + Duration::from_secs(61));
        assert!(
            port.injected_bodies().is_empty(),
            "발신자도 부재 → notice 파킹"
        );

        let (alice_id, alice) = live("alice");
        port.set_roster(vec![alice]);
        svc.flush_for(alice_id);
        assert_eq!(
            port.injected_bodies(),
            vec!["<notice>[engram] 요청 m-9x 기한(1m) 초과 — bob 회신 없음</notice>".to_string()],
            "flush 배달도 notice 태그(from 없음)"
        );
        let noticed: Vec<_> = svc
            .ledger_snapshot()
            .into_iter()
            .filter(|(_, from, _, _)| from == NOTICE_SENDER_LABEL)
            .collect();
        assert_eq!(
            noticed[0].3,
            DeliveryStatus::Delivered,
            "실제 주입 시점 delivered"
        );
    }

    #[test]
    fn timeout_notice_parks_when_sender_is_mid_turn() {
        // The ban on injecting mid-turn (keeping batch control) applies to a notice too.
        let (svc, port, gate) = svc_gated();
        let (alice_id, alice) = live("alice");
        port.set_roster(vec![alice]);
        gate.set_busy(alice_id, 0);
        svc.park_absent_for_test(
            "m-b1",
            reply_from(tid("alice")),
            "alice",
            "ghost",
            "해줘",
            Entrance::Mcp,
            &req_meta("1m", 60),
        )
        .expect("parked");
        svc.sweep(Instant::now() + Duration::from_secs(61));
        assert!(
            port.injected_bodies().is_empty(),
            "발신자가 턴 중이면 notice 도 주입하지 않는다"
        );
        assert_eq!(svc.parked_len(tid("alice")), 1, "notice 가 alice 앞에 파킹");

        gate.clear();
        svc.flush_for(alice_id);
        assert_eq!(
            port.injected_bodies(),
            vec!["<notice>[engram] 요청 m-b1 기한(1m) 초과 — ghost 회신 없음</notice>".to_string()],
            "턴 종료 flush 로 배달"
        );
    }

    // ── C3 review round fix ────────────────────────────────────────────────────────────

    #[test]
    fn timeout_notice_reaches_a_renamed_sender_by_its_id() {
        // ★fix 2 regression★: a notice is notified so it never fires again — if it cannot reach a renamed sender it is permanently stranded. The queue key is
        //   the sender id, so it reaches them regardless of a rename (ADR-0309).
        let (svc, port) = svc();
        let alice_id = PeerId::new_v4();
        let old = LiveAgent {
            id: alice_id,
            name: "alice".to_string(),
            epoch: 0,
            turn_signal: true,
        };
        port.set_roster(vec![old]);
        svc.park_absent_for_test(
            "m-req",
            SenderIdentity {
                peer_id: alice_id,
                epoch: 0,
            },
            "alice",
            "ghost",
            "해줘",
            Entrance::Mcp,
            &req_meta("1m", 60),
        )
        .expect("parked(수신자 부재)");

        let renamed = LiveAgent {
            id: alice_id,
            name: "alice-v2".to_string(),
            epoch: 0,
            turn_signal: true,
        };
        port.set_roster(vec![renamed]);

        svc.sweep(Instant::now() + Duration::from_secs(61));
        // Doorbell unwired = inline flush fallback, so delivery finishes inside the sweep.
        assert_eq!(
            port.injected_bodies(),
            vec![
                "<notice>[engram] 요청 m-req 기한(1m) 초과 — ghost 회신 없음</notice>".to_string()
            ],
            "개명한 발신자에게도 배달돼야(큐 키 = 발신자 id)"
        );
        assert_eq!(port.injected_targets(), vec![alice_id]);
        assert_eq!(svc.parked_len(alice_id), 0, "큐가 비었다 = 실제로 나갔다");
    }

    /// ★The sweep does not read the roster (ADR-0309 · protects the sweep task)★: a notice parks in the sender id's queue and presses the doorbell by that id,
    ///   that is all. In the days when the recipient was resolved by name, the sweep took one roster snapshot per tick (the predecessor of the test that
    ///   counted that cost). The actual injection is the share of the flush lane that received the doorbell (the `deliver_notice` comment).
    #[test]
    fn sweep_parks_notices_by_sender_id_and_rings_without_reading_the_roster() {
        let (svc, port, _gate, bell) = svc_gated_with_doorbell();
        port.set_roster(vec![]);
        let senders: Vec<SenderIdentity> = (0..3).map(|_| ident()).collect();
        for (i, from) in senders.iter().enumerate() {
            svc.park_absent_for_test(
                &format!("m-r{i}"),
                *from,
                &format!("sender{i}"),
                "ghost",
                "해줘",
                Entrance::Mcp,
                &req_meta("1m", 60),
            )
            .expect("parked");
        }
        bell.take();
        let before = port.roster_call_count();

        svc.sweep(Instant::now() + Duration::from_secs(61));

        assert_eq!(
            port.roster_call_count() - before,
            0,
            "sweep 은 due 수와 무관하게 로스터를 읽지 않는다"
        );
        let mut rung = bell.take();
        rung.sort();
        let mut expected: Vec<PeerId> = senders.iter().map(|s| s.peer_id).collect();
        expected.sort();
        assert_eq!(rung, expected, "통지마다 그 발신자 id 로 도어벨 한 번");
        for s in &senders {
            assert_eq!(
                svc.parked_len(s.peer_id),
                1,
                "notice 는 발신자 id 큐에 파킹"
            );
        }
        assert!(
            port.injected_bodies().is_empty(),
            "sweep 은 주입하지 않는다"
        );

        // The lane consumes that doorbell.
        port.set_roster(vec![live_with_id(senders[0].peer_id, "sender0").1]);
        svc.flush_for_agent(senders[0].peer_id);
        assert_eq!(
            port.injected_bodies(),
            vec!["<notice>[engram] 요청 m-r0 기한(1m) 초과 — ghost 회신 없음</notice>".to_string()],
        );
    }

    /// ★A reply that arrives after the due computation · before the notice parking drops that notice (review fix 5)★. The race point = the moment the earlier notice's
    ///   inline flush takes the roster (`arm_on_next_roster`) — at that time both dues are already `notified` and the second
    ///   notice is not yet parked. The sweep itself does not read the roster, so this gap arises only when there are two dues.
    #[test]
    fn timeout_notice_is_skipped_when_reply_lands_before_parking() {
        let (svc, port) = svc();
        port.set_roster(vec![]);
        for (msg_id, sender) in [("m-req0", "alice"), ("m-req1", "carol")] {
            svc.park_absent_for_test(
                msg_id,
                ident(),
                sender,
                "bob",
                "해줘",
                Entrance::Mcp,
                &req_meta("1m", 60),
            )
            .expect("parked");
        }

        let svc2 = svc.clone();
        port.arm_on_next_roster(Box::new(move || {
            svc2.park_absent_for_test(
                "m-rep",
                reply_from(tid("bob")),
                "bob",
                "carol",
                "했음",
                Entrance::Mcp,
                &reply_meta("m-req1"),
            )
            .expect("parked(carol 부재)");
        }));

        svc.sweep(Instant::now() + Duration::from_secs(61));
        let notice_recipients: Vec<String> = svc
            .ledger_snapshot()
            .into_iter()
            .filter(|(_, from, _, _)| from == NOTICE_SENDER_LABEL)
            .map(|(_, _, to, _)| to)
            .collect();
        assert_eq!(
            notice_recipients,
            vec!["alice".to_string()],
            "회신이 먼저 닿은 m-req1 의 통지는 취소, 앞선 m-req0 통지는 그대로"
        );
        assert_eq!(
            svc.contract_outcome_for_test("m-req1", tid("bob")),
            Some("replied"),
            "m-req1 은 회신으로 닫혔다"
        );
        assert_eq!(svc.open_request_count(), 1, "m-req0 만 열려 있다");
    }

    #[test]
    fn plain_send_with_colliding_id_is_rejected_without_side_effects() {
        let (svc, port) = svc();
        let (_id, alice) = live("alice");
        port.set_roster(vec![alice]);
        svc.park_absent_for_test(
            "dup",
            ident(),
            "s",
            "alice",
            "1",
            Entrance::Mcp,
            &SendMeta::default(),
        )
        .expect("delivered");
        let before = svc.ledger_snapshot().len();
        let injected_before = port.injected_bodies().len();

        assert_eq!(
            svc.park_absent_for_test(
                "dup",
                ident(),
                "s",
                "alice",
                "2",
                Entrance::Mcp,
                &SendMeta::default()
            ),
            Err(SendFail::Reject(SendReject::IdCollision)),
            "통보도 id 충돌이면 반려"
        );
        assert_eq!(
            svc.park_absent_for_test(
                "dup",
                ident(),
                "s",
                "alice",
                "3",
                Entrance::Mcp,
                &reply_meta("m-other"),
            ),
            Err(SendFail::Reject(SendReject::IdCollision)),
            "회신도 id 충돌이면 반려"
        );
        assert_eq!(svc.ledger_snapshot().len(), before, "장부 레코드 증가 없음");
        assert_eq!(
            port.injected_bodies().len(),
            injected_before,
            "주입 없음(부작용 0)"
        );
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "ingress가 유일 검증자")]
    fn mutually_exclusive_contract_fields_trip_the_debug_assert() {
        // ★fix 11★: this function is pub so another assembly can call it directly — a wiring that violates mutual exclusion blows up
        //   at once in debug (the production bounce wording is canonical in ingress, so validation is not duplicated).
        let (svc, port) = svc();
        port.set_roster(vec![]);
        let bad = SendMeta {
            request: true,
            reply_by_raw: None,
            reply_by: None,
            reply_to: Some("m-1".to_string()),
            to_attr: None,
        };
        let _ = svc.park_absent_for_test("m-x", ident(), "s", "bob", "x", Entrance::Mcp, &bad);
    }

    // ── ParkPayload robustness (fix 4) ─────────────────────────────────────────────────────

    #[test]
    fn park_payload_decode_falls_back_on_corrupt_input_without_panicking() {
        let good = ParkPayload {
            sender_name: "qa".to_string(),
            from: SenderIdentity {
                peer_id: PeerId::new_v4(),
                epoch: 3,
            },
            entrance: Entrance::Mcp,
            body: "안녕\n둘째 줄".to_string(),
            meta: req_meta("10m", 600),
        };
        let encoded = good.encode();
        // Control: a normal payload.
        let ok = ParkPayload::decode(&encoded);
        assert_eq!(ok.body, "안녕\n둘째 줄");
        assert_eq!(ok.sender_name, "qa");
        assert!(ok.meta.request);

        // The tag is always at the very front so it is cut by position — the old `replacen('1', …)` touched a wrong spot when the version character
        //   changed and silently became a valid payload.
        let unknown_version = {
            let mut s = encoded.clone();
            s.replace_range(0..PARK_PAYLOAD_VERSION.len(), "9");
            s
        };
        // v2 layout: ver·sender_len·rb_len·rt_len·grp_len·uuid·epoch·entrance·flags 9 lines + the rest.
        for corrupt in [
            // No header at all.
            "그냥 본문".to_string(),
            // The version tag is an unknown value (format drift).
            unknown_version,
            // A length field is not a number.
            "2\nx\n0\n0\n0\n00000000-0000-0000-0000-000000000000\n0\nmcp\n-\nbody".to_string(),
            // The sum of lengths is larger than the remaining string (truncation).
            "2\n99\n0\n0\n0\n00000000-0000-0000-0000-000000000000\n0\nmcp\n-\nshort".to_string(),
            // ★Cut in the middle of a char boundary★: '한' is 3 bytes but sender_len=1 so it splits in the middle of the character.
            "2\n1\n0\n0\n0\n00000000-0000-0000-0000-000000000000\n0\nmcp\n-\n한글".to_string(),
            // A case where the reply_to length makes the boundary misaligned (the second cut point).
            "2\n0\n1\n0\n0\n00000000-0000-0000-0000-000000000000\n0\nmcp\n-\n한글".to_string(),
            // ★The group label cut point (C4 — the fourth cut point)★.
            "2\n0\n0\n0\n1\n00000000-0000-0000-0000-000000000000\n0\nmcp\n-\n한글".to_string(),
            // Too few header lines.
            "2\n0\n0\n0\n".to_string(),
            // ★An out-of-vocabulary entrance (fix 3)★.
            "2\n0\n0\n0\n0\n00000000-0000-0000-0000-000000000000\n0\nsmtp\n-\nbody".to_string(),
            // The entrance field is empty (format drift).
            "2\n0\n0\n0\n0\n00000000-0000-0000-0000-000000000000\n0\n\n-\nbody".to_string(),
            // Case differences are also out of vocabulary.
            "2\n0\n0\n0\n0\n00000000-0000-0000-0000-000000000000\n0\nMCP\n-\nbody".to_string(),
            // ★An out-of-vocabulary flag (fix 3)★.
            "2\n0\n0\n0\n0\n00000000-0000-0000-0000-000000000000\n0\nmcp\nR\nbody".to_string(),
            "2\n0\n0\n0\n0\n00000000-0000-0000-0000-000000000000\n0\nmcp\n\nbody".to_string(),
            "2\n0\n0\n0\n0\n00000000-0000-0000-0000-000000000000\n0\nmcp\nr-\nbody".to_string(),
        ] {
            let p = ParkPayload::decode(&corrupt);
            assert_eq!(
                p.body, corrupt,
                "깨진 payload 는 전체를 body 로 폴백(조용한 유실 금지): {corrupt:?}"
            );
            assert!(!p.meta.request, "폴백은 계약 속성을 주장하지 않는다");
        }
    }

    #[test]
    fn one_corrupt_parked_item_does_not_abort_the_flush_batch() {
        let (svc, port) = svc();
        port.set_roster(vec![]);
        for (id, body) in [("m-a", "첫째"), ("m-b", "둘째"), ("m-c", "셋째")] {
            svc.park_absent_for_test(
                id,
                ident(),
                "qa",
                "bob",
                body,
                Entrance::Mcp,
                &SendMeta::default(),
            )
            .expect("parked");
        }
        svc.corrupt_parked_payload_for_test(tid("bob"), 1);

        let (bob_id, bob) = live("bob");
        port.set_roster(vec![bob]);
        svc.flush_for(bob_id);

        let bodies = port.injected_bodies();
        assert_eq!(bodies.len(), 3, "깨진 항목 하나가 배치를 끊지 않는다");
        assert!(bodies[0].contains("첫째") && bodies[2].contains("셋째"));
        assert!(
            bodies[1].contains("CORRUPT"),
            "깨진 항목은 폴백 봉투로라도 나간다(조용한 유실 금지): {}",
            bodies[1]
        );
    }

    // ── Daemon self-send id collision check (round-final fix 2) ────────────────────────────────

    #[test]
    fn daemon_notice_id_goes_through_the_same_collision_check_as_sends() {
        let (svc, port) = svc();
        port.set_roster(vec![]);
        svc.park_absent_for_test(
            "m-taken",
            ident(),
            "alice",
            "bob",
            "x",
            Entrance::Mcp,
            &SendMeta::default(),
        )
        .expect("parked");

        // ① The first draw collides. (`pop()` takes from the back, so the list is in the reverse of draw order.)
        let mut two = vec!["m-fresh".to_string(), "m-taken".to_string()];
        let drawn = svc.draw_daemon_msg_id_with(|| two.pop().expect("draw"));
        assert_eq!(drawn.id, "m-fresh", "충돌한 id 는 버리고 새 id 를 쓴다");
        assert_eq!(
            drawn.collided.as_deref(),
            Some("m-taken"),
            "충돌한 쪽이 조사 단서 — 로그가 그 값을 찍는다"
        );
        assert!(!drawn.still_colliding);

        // ② The second one collides too.
        let mut both = vec!["m-taken".to_string(), "m-taken".to_string()];
        let drawn = svc.draw_daemon_msg_id_with(|| both.pop().expect("draw"));
        assert_eq!(drawn.id, "m-taken", "통지 유실보다 관측상 오염이 낫다");
        assert!(
            drawn.still_colliding,
            "재-draw 마저 충돌하면 신호(로그 대상)"
        );

        // ③ Control: no collision.
        let mut once = vec!["m-new".to_string()];
        let drawn = svc.draw_daemon_msg_id_with(|| once.pop().expect("draw 는 1회뿐이어야"));
        assert_eq!(drawn.id, "m-new");
        assert!(drawn.collided.is_none());
    }

    /// ★H3★: a victim under provisional retirement may be in neither tracking nor history (when the history was already pushed out), and if
    /// the same id is newly issued in that window, the restore loses the original. So the reservation set must take part in the issuance check.
    #[test]
    fn the_mint_path_regenerates_when_it_draws_a_provisionally_retired_id() {
        // ★After ADR-0125 the means of opening the window changed★: the old edition tested issuance in the `on_inject` hook, but the reservation
        //   window moved inside the enqueue lock and the issuing path (`draw_daemon_msg_id_with`) takes the same lock too, so **the hook
        //   in principle cannot see this window**. Instead the reservation guard is opened with the same verb as production and the test is done outside it.
        let (svc, port) = svc();
        let (boss_from, boss) = live_sender("boss");
        port.set_roster(vec![boss]);
        fill_open_request_cap_evictable(&svc, boss_from);
        let cap = svc.occupied_slots_for_test();
        assert!(cap > 0, "상한까지 채워졌다");
        assert!(
            svc.msg_id_in_use_for_test("cap0"),
            "전제: 희생자가 될 픽스처 계약이 존재한다"
        );

        // After the reservation (retirement mark) · before commit — while this guard is alive is the provisional window.
        let res = reserve_marked_contract(&svc, boss_from, "m-provisional");
        // ★C2★: the old assertion was only `msg_id_in_use("cap0")`, but that value is also true because of a row remaining in the history ring, so
        //   it stayed green even when reservation visibility was removed — so the window is observed directly by the tracking count (cap + 1 → cap).
        assert_eq!(
            svc.open_request_count(),
            cap + 1,
            "잠정 창: 표시된 희생자와 새 계약이 동시에 추적에 있어야"
        );
        let mut two = vec!["m-fresh".to_string(), "cap0".to_string()];
        let drawn = svc.draw_daemon_msg_id_with(|| two.pop().expect("draw"));
        let in_use = svc.msg_id_in_use_for_test("cap0");
        let drawn_id = drawn.id.clone();

        let mut log = RetirementLog::default();
        {
            let mut st = svc.state.lock().expect("lock");
            res.commit(&mut st, &mut log);
        }
        assert_eq!(
            svc.open_request_count(),
            cap,
            "커밋(결말 확정) 뒤에는 희생자가 물리 제거돼 상한으로 돌아온다"
        );
        assert!(
            in_use,
            "잠정 은퇴 중인 cap0 은 발급 검사에서 '사용 중' 이어야(H3)"
        );
        assert_eq!(
            drawn_id, "m-fresh",
            "그래서 발급 경로가 그 id 를 버리고 새 id 로 갈아탄다"
        );
    }

    #[test]
    fn id_collision_rejects_without_side_effects() {
        let (svc, port) = svc();
        port.set_roster(vec![]);
        svc.park_absent_for_test(
            "dup",
            ident(),
            "alice",
            "bob",
            "1",
            Entrance::Mcp,
            &req_meta("10m", 600),
        )
        .expect("parked");
        let before = svc.ledger_snapshot().len();
        let again = svc.park_absent_for_test(
            "dup",
            ident(),
            "alice",
            "bob",
            "2",
            Entrance::Mcp,
            &req_meta("10m", 600),
        );
        assert_eq!(again, Err(SendFail::Reject(SendReject::IdCollision)));
        assert_eq!(svc.ledger_snapshot().len(), before, "장부 레코드 증가 없음");
        assert_eq!(svc.parked_len(tid("bob")), 1, "파킹 증가 없음");
        assert!(port.injected_bodies().is_empty());
    }

    // ── D: ledger lookup surface (message_state / open_items_for) ─────────────────────────────────

    #[test]
    fn message_state_marks_an_open_request_as_awaiting_reply_until_the_answer_lands() {
        let (svc, port) = svc();
        let (from, _s) = live_sender("alice");
        let (b_id, bob) = live("bob");
        port.set_roster(vec![bob]);
        let t0 = Instant::now();
        svc.park_absent_for_test(
            "m-req",
            from,
            "alice",
            "bob",
            "해줘",
            Entrance::Mcp,
            &req_meta("10m", 600),
        )
        .expect("request");
        let view = svc.message_state("m-req", t0).expect("조회");
        assert!(view.awaiting_reply, "회신 전엔 대기 중");
        assert_eq!(view.rows.len(), 1);
        assert_eq!(view.rows[0].status, "delivered");

        let bob_from = reply_from(b_id);
        let (_a, alice_agent) = live("alice");
        port.set_roster(vec![alice_agent]);
        svc.park_absent_for_test(
            "m-ans",
            bob_from,
            "bob",
            "alice",
            "했음",
            Entrance::Mcp,
            &reply_meta("m-req"),
        )
        .expect("회신");
        let view = svc.message_state("m-req", t0).expect("조회");
        assert!(!view.awaiting_reply, "회신 도착 후엔 대기 해제");
        assert_eq!(view.rows[0].status, "replied");
    }

    #[test]
    fn message_state_ages_are_measured_from_the_ledger_instants() {
        let (svc, port) = svc();
        let (from, _s) = live_sender("alice");
        let (_b, bob) = live("bob");
        port.set_roster(vec![bob]);
        let t0 = Instant::now();
        svc.park_absent_for_test(
            "m1",
            from,
            "alice",
            "bob",
            "hi",
            Entrance::Mcp,
            &SendMeta::default(),
        )
        .expect("발송");
        let view = svc
            .message_state("m1", t0 + Duration::from_secs(90))
            .expect("조회");
        assert!(
            (85..=95).contains(&view.rows[0].age_secs),
            "발송 경과 ≈ 90초: {}",
            view.rows[0].age_secs
        );
        let view = svc
            .message_state("m1", t0 - Duration::from_secs(10))
            .expect("조회");
        assert_eq!(view.rows[0].age_secs, 0);
    }

    #[test]
    fn open_items_tags_each_direction_and_ignores_other_peoples_business() {
        let (svc, port) = svc();
        // A fixture where one logical agent's sender identity and its roster entry share **the same PeerId** (review B1).
        let (alice_from, alice_agent) = live_sender("alice");
        let (bob_from, bob_agent) = live_sender("bob");
        let (carol_from, _carol_agent) = live_sender("carol");
        port.set_roster(vec![bob_agent.clone()]);
        let t0 = Instant::now();

        svc.park_absent_for_test(
            "m-park",
            alice_from,
            "alice",
            "ghost",
            "hi",
            Entrance::Mcp,
            &SendMeta::default(),
        )
        .expect("파킹");
        svc.park_absent_for_test(
            "m-out",
            alice_from,
            "alice",
            "bob",
            "해줘",
            Entrance::Mcp,
            &req_meta("10m", 600),
        )
        .expect("request");
        port.set_roster(vec![alice_agent]);
        svc.park_absent_for_test(
            "m-in",
            carol_from,
            "carol",
            "alice",
            "이거 해줘",
            Entrance::Mcp,
            &req_meta("30m", 1800),
        )
        .expect("수신 request");
        port.set_roster(vec![bob_agent]);
        svc.park_absent_for_test(
            "m-other",
            carol_from,
            "carol",
            "bob",
            "너 해라",
            Entrance::Mcp,
            &req_meta("10m", 600),
        )
        .expect("남의 request");

        let items = svc.open_items_for(alice_from.peer_id, t0);
        let tags: Vec<(&str, &str)> = items
            .iter()
            .map(|i| (i.direction.as_str(), i.id.as_str()))
            .collect();
        assert_eq!(
            tags,
            vec![
                ("awaiting_their_reply", "m-out"),
                ("outbound_pending", "m-park"),
                ("reply_owed_by_me", "m-in"),
            ],
            "세 방향이 태그로 구분되고 남의 계약은 빠진다: {items:?}"
        );
        assert_eq!(items[0].reply_by.as_deref(), Some("10m"), "기한 표기 원본");
        assert_eq!(items[2].from, "carol", "내가 답할 상대");
        assert!(items.iter().all(|i| !i.timed_out));
        let bob_items = svc.open_items_for(bob_from.peer_id, t0);
        let bob_tags: Vec<(&str, &str)> = bob_items
            .iter()
            .map(|i| (i.direction.as_str(), i.id.as_str()))
            .collect();
        assert_eq!(
            bob_tags,
            vec![
                ("reply_owed_by_me", "m-other"),
                ("reply_owed_by_me", "m-out"),
            ],
            "bob 은 자기가 답할 두 건만: {bob_items:?}"
        );
    }

    #[test]
    fn open_items_drops_answered_contracts_but_keeps_timed_out_ones() {
        let (svc, port) = svc();
        let (alice_from, _s) = live_sender("alice");
        let (b_id, bob) = live("bob");
        port.set_roster(vec![bob]);
        let t0 = Instant::now();
        svc.park_absent_for_test(
            "m-a",
            alice_from,
            "alice",
            "bob",
            "1",
            Entrance::Mcp,
            &req_meta("10m", 600),
        )
        .expect("req a");
        svc.park_absent_for_test(
            "m-b",
            alice_from,
            "alice",
            "bob",
            "2",
            Entrance::Mcp,
            &req_meta("10m", 600),
        )
        .expect("req b");
        let bob_from = reply_from(b_id);
        let (_a, alice_agent) = live("alice");
        port.set_roster(vec![alice_agent]);
        svc.park_absent_for_test(
            "m-ans",
            bob_from,
            "bob",
            "alice",
            "됨",
            Entrance::Mcp,
            &reply_meta("m-a"),
        )
        .expect("회신");
        svc.sweep(t0 + Duration::from_secs(601));

        let items = svc.open_items_for(alice_from.peer_id, t0 + Duration::from_secs(601));
        let ids: Vec<&str> = items.iter().map(|i| i.id.as_str()).collect();
        assert_eq!(ids, vec!["m-b"], "회신 온 계약만 빠진다: {items:?}");
        assert!(
            items[0].timed_out,
            "기한 초과 사실은 플래그로 보이되 목록에는 남는다"
        );
    }

    #[test]
    fn a_request_addressed_by_exact_id_only_obligates_that_twin() {
        let (svc, port) = svc();
        let (boss_from, _boss) = live_sender("boss");
        let (twin_a_from, twin_a) = live_sender_distinct("worker");
        let (twin_b_from, twin_b) = live_sender_distinct("worker"); // Same name, different PeerId.
        port.set_roster(vec![twin_a.clone(), twin_b.clone()]);
        let t0 = Instant::now();

        svc.park_absent_for_test(
            "m-req",
            boss_from,
            "boss",
            &twin_a.id.to_string(),
            "해줘",
            Entrance::Mcp,
            &req_meta("10m", 600),
        )
        .expect("exact-id request");

        let a_items = svc.open_items_for(twin_a_from.peer_id, t0);
        assert_eq!(
            a_items
                .iter()
                .map(|i| (i.direction.as_str(), i.id.as_str()))
                .collect::<Vec<_>>(),
            vec![("reply_owed_by_me", "m-req")],
            "지목된 쌍둥이 A 는 의무를 진다: {a_items:?}"
        );
        let b_items = svc.open_items_for(twin_b_from.peer_id, t0);
        assert!(
            b_items.is_empty(),
            "받은 적 없는 쌍둥이 B 에게 의무가 붙으면 안 된다(B1): {b_items:?}"
        );
        let boss_items = svc.open_items_for(boss_from.peer_id, t0);
        assert_eq!(
            boss_items
                .iter()
                .map(|i| i.direction.as_str())
                .collect::<Vec<_>>(),
            vec!["awaiting_their_reply"]
        );
    }

    /// ★Sent waiting items are found by sender id — they stay in the list after a rename and the row shows the send-time name (ADR-0309 decisions 4 ·
    ///   5 · TRD S29 B3)★. In the days of comparing by name, a renamed sender lost the pre-rename waiting items from its list, and an agent that
    ///   newly took the old name saw them.
    #[test]
    fn a_renamed_sender_still_sees_its_parked_outbound_mail_under_the_send_time_name() {
        let (svc, port) = svc();
        let (alice_from, alice) = live_sender("alice");
        port.set_roster(vec![alice]);
        port.set_dormant(&["bob"]);
        svc.handle_send(
            "m-out",
            alice_from,
            "alice",
            &["bob".to_string()],
            "hi",
            Entrance::Mcp,
            &SendMeta::default(),
        )
        .expect("파킹");
        assert_eq!(svc.parked_len(tid("bob")), 1, "전제: 잠든 bob 앞 파킹");

        // alice renames — same id, new name. The old name was taken by another agent.
        let (_, renamed) = live_with_id(alice_from.peer_id, "alice2");
        let (other_id, other) = live_distinct("alice");
        port.set_roster(vec![renamed, other]);
        let items = svc.open_items_for(alice_from.peer_id, Instant::now());
        assert_eq!(
            items
                .iter()
                .map(|i| (
                    i.direction.as_str(),
                    i.id.as_str(),
                    i.from.as_str(),
                    i.to.as_str()
                ))
                .collect::<Vec<_>>(),
            vec![("outbound_pending", "m-out", "alice", "bob")],
            "개명 전에 보낸 대기분이 남고 행은 보낼 때 이름이다: {items:?}"
        );
        assert!(
            svc.open_items_for(other_id, Instant::now()).is_empty(),
            "옛 이름을 새로 가진 다른 에이전트에게는 보이지 않는다"
        );
    }

    /// ★Fills the open-contract limit with **non-retirable** contracts (A7 — reproduces `REQUEST_CAPACITY`)★.
    fn fill_open_request_cap(svc: &Arc<MessagingService>, from: SenderIdentity) {
        fill_open_request_cap_with(svc, from, &req_meta("10m", 600));
    }

    /// ★Fills to the limit **leaving one slot** with non-retirable contracts (H2 — a fixture to make A3's real window)★.
    ///   If the caller fills that remaining slot with 1 retirable contract, then the retirable candidate that the **second** recipient of the following send can see
    ///   becomes **only the first recipient's contract that send just opened** (= the only batch where self-erosion can occur).
    fn fill_open_request_cap_leaving_one_free(svc: &Arc<MessagingService>, from: SenderIdentity) {
        fill_open_request_cap(svc, from);
        let last = svc
            .open_items_for(from.peer_id, Instant::now())
            .into_iter()
            .filter(|i| i.direction == Direction::AwaitingTheirReply)
            .last()
            .expect("채운 계약이 있다");
        svc.close_contract_for_test(&last.id, tid(&last.to));
        assert!(
            svc.occupied_slots_for_test() > 0,
            "전제: 상한 근처까지 채워졌다"
        );
    }

    /// ★Fills the limit with **retirable** contracts (A2/A3 — reproduces victim exchange)★. If there is no deadline (= the daemon owes no notice
    /// debt) that contract is a retirement candidate.
    fn fill_open_request_cap_evictable(svc: &Arc<MessagingService>, from: SenderIdentity) {
        fill_open_request_cap_with(
            svc,
            from,
            &SendMeta {
                request: true,
                ..SendMeta::default()
            },
        );
    }

    /// ★Do not duplicate the constant (A7)★ — the point where it stops growing is the limit, so the fixture follows even if the limit changes.
    ///   ★Do not use `open_request_count` (= `!closed`)★: a victim marked for retirement does not count a slot but remains in
    ///   that count and grows without bound (measured — the loop never ends when filling with retirable ones).
    fn fill_open_request_cap_with(
        svc: &Arc<MessagingService>,
        from: SenderIdentity,
        meta: &SendMeta,
    ) {
        let mut i = 0usize;
        loop {
            let before = svc.occupied_slots_for_test();
            svc.park_absent_for_test(
                &format!("cap{i}"),
                from,
                "boss",
                &format!("victim{i}"),
                "q",
                Entrance::Mcp,
                meta,
            )
            .ok();
            i += 1;
            if svc.occupied_slots_for_test() == before {
                break;
            }
            assert!(i < 100_000, "상한이 관측되지 않는다 — 픽스처 전제 붕괴");
        }
    }

    /// ★The reply duty stays with the agent (id) that received the request — even if a different agent with the same name comes up, neither the letter nor the duty
    ///   moves (ADR-0309 decisions 1 · 5 · 7)★. Reviving a device that re-stamps the duty onto the landing side or a branch that finds the recipient by name
    ///   lets B take over A's duty and A sees 「답할 게 없다」 (there is nothing to answer).
    #[test]
    fn the_reply_obligation_stays_with_the_addressed_agent_not_a_namesake() {
        let (svc, port, gate) = svc_gated();
        let (boss_from, boss) = live_sender("boss");
        let (a_from, a_agent) = live_sender_distinct("worker");
        port.set_roster(vec![a_agent.clone()]);
        gate.set_busy(a_agent.id, 0);
        let t0 = Instant::now();

        svc.park_absent_for_test(
            "m-req",
            boss_from,
            "boss",
            &a_agent.id.to_string(),
            "해줘",
            Entrance::Mcp,
            &req_meta("10m", 600),
        )
        .expect("busy 수신자 → 파킹된 request");
        assert_eq!(svc.parked_len(a_agent.id), 1, "전제: 파킹");

        let (b_from, b_agent) = live_sender_distinct("worker");
        port.set_roster(vec![boss, b_agent.clone()]);
        gate.clear();
        svc.flush_for(b_agent.id);
        assert!(
            port.injected_bodies().is_empty(),
            "동명 B 는 A 앞 편지를 받지 않는다(B2)"
        );
        assert_eq!(svc.parked_len(a_agent.id), 1, "A 의 큐에 그대로");

        let b_items = svc.open_items_for(b_from.peer_id, t0);
        assert!(
            b_items.is_empty(),
            "요청을 받은 적 없는 동명 B 에게 의무가 붙으면 안 된다: {b_items:?}"
        );
        let a_items = svc.open_items_for(a_from.peer_id, t0);
        assert_eq!(
            a_items
                .iter()
                .map(|i| (i.direction.as_str(), i.id.as_str()))
                .collect::<Vec<_>>(),
            vec![("reply_owed_by_me", "m-req")],
            "의무는 지목된 A 의 id 에 남는다: {a_items:?}"
        );
        svc.handle_send(
            "m-rep",
            b_from,
            "worker",
            &["boss".to_string()],
            "했음",
            Entrance::Mcp,
            &reply_meta("m-req"),
        )
        .expect("행 응답");
        assert_eq!(
            svc.contract_outcome_for_test("m-req", a_agent.id),
            Some("awaiting_reply"),
            "B 의 회신은 A 의 계약을 닫지 못한다"
        );
        let boss_items = svc.open_items_for(boss_from.peer_id, t0);
        assert_eq!(
            boss_items
                .iter()
                .map(|i| (i.direction.as_str(), i.id.as_str()))
                .collect::<Vec<_>>(),
            vec![
                ("awaiting_their_reply", "m-req"),
                ("outbound_pending", "m-req"),
            ],
            "boss 는 회신을 기다리고, 편지는 A 가 아직 안 받아 대기분으로도 보인다: {boss_items:?}"
        );
    }

    #[test]
    fn a_reply_racing_its_own_injection_still_finds_the_contract() {
        // ★A reply that arrives concurrently with injection also finds the contract★ — the contract holds the receiving id from the moment it opens, so it is independent of injection order
        //   (ADR-0309). The receiving side is the **same agent** that restarted (`live_sender("worker")` twice = the same id).
        // ★How the race is reproduced deterministically★: the `on_inject` hook is called right before the write. Sending a reply inside it
        //   **exactly** simulates "a reply that arrived concurrently with injection" (deterministic with no sleep or thread).
        let (svc, port, gate) = svc_gated();
        let (boss_from, boss) = live_sender("boss");
        let (a_from, a_agent) = live_sender("worker");
        port.set_roster(vec![boss.clone(), a_agent.clone()]);
        gate.set_busy(a_agent.id, 0);
        let t0 = Instant::now();

        svc.park_absent_for_test(
            "m-req",
            boss_from,
            "boss",
            &a_agent.id.to_string(),
            "해줘",
            Entrance::Mcp,
            &req_meta("10m", 600),
        )
        .expect("busy 수신자 → 파킹된 request");
        assert_eq!(svc.parked_len(tid("worker")), 1, "전제: 그 id 큐에 파킹");
        assert_eq!(svc.open_request_count(), 1, "전제: 계약 1건 열림");

        // A new incarnation of the same agent (same id).
        let b_agent = LiveAgent {
            epoch: 1,
            ..a_agent.clone()
        };
        let b_id = b_agent.id;
        port.set_roster(vec![boss.clone(), b_agent.clone()]);
        gate.clear();

        // idx 0 = flush's injection. The reply's own injection is idx 1, so it is filtered out — re-entering would recurse infinitely.
        {
            let svc2 = svc.clone();
            port.set_on_inject(Arc::new(move |idx| {
                if idx != 0 {
                    return;
                }
                svc2.handle_send(
                    "m-reply",
                    reply_from(b_id),
                    "worker",
                    &["boss".to_string()],
                    "다 했다",
                    Entrance::Mcp,
                    &reply_meta("m-req"),
                )
                .expect("회신 발송은 성공해야");
            }));
        }
        svc.flush_for(b_id);

        let bodies = port.injected_bodies();
        assert_eq!(
            bodies.len(),
            2,
            "flush 배달 1 + 회신 1(전제 — 회신이 실제로 나갔다): {bodies:?}"
        );
        // The reply is recorded before the outer write, so the order assertion is by presence, not by index.
        assert!(
            bodies.iter().any(|b| b.contains(r#"in-reply-to="m-req""#)),
            "회신 봉투가 실제로 나갔어야: {bodies:?}"
        );
        assert!(
            bodies.iter().any(|b| b.contains(r#"type="request""#)),
            "파킹돼 있던 request 도 배달됐어야: {bodies:?}"
        );

        assert_eq!(
            svc.open_request_count(),
            0,
            "주입 도중 도착한 회신이 계약을 닫았어야"
        );
        assert!(
            svc.open_items_for(b_id, t0).is_empty(),
            "회신한 쪽에 남은 의무도 없다"
        );
        assert!(
            svc.open_items_for(boss_from.peer_id, t0).is_empty(),
            "발신자도 더 기다리지 않는다"
        );

        svc.sweep(t0 + Duration::from_secs(700));
        assert_eq!(
            port.injected_bodies().len(),
            2,
            "닫힌 계약엔 기한 통지가 없다: {:?}",
            port.injected_bodies()
        );
        assert_eq!(
            svc.parked_len(tid("boss")),
            0,
            "통지가 파킹으로 새지도 않는다"
        );
        let _ = a_from;
    }

    #[test]
    fn message_state_reports_whether_its_row_list_can_be_trusted_as_complete() {
        let (svc, port) = svc();
        let (from, _s) = live_sender("alice");
        let (_b, bob) = live("bob");
        port.set_roster(vec![bob]);
        let t0 = Instant::now();
        svc.park_absent_for_test(
            "m1",
            from,
            "alice",
            "bob",
            "hi",
            Entrance::Mcp,
            &SendMeta::default(),
        )
        .expect("발송");
        let view = svc.message_state("m1", t0).expect("조회");
        assert!(
            !view.may_be_truncated,
            "evict 전이면 rows 가 전부라고 단언할 수 있다"
        );
    }

    #[test]
    fn open_items_is_empty_when_nothing_is_outstanding() {
        let (svc, port) = svc();
        let (from, _s) = live_sender("alice");
        let (_b, bob) = live("bob");
        port.set_roster(vec![bob]);
        svc.park_absent_for_test(
            "m1",
            from,
            "alice",
            "bob",
            "hi",
            Entrance::Mcp,
            &SendMeta::default(),
        )
        .expect("배달 완료");
        assert!(
            svc.open_items_for(from.peer_id, Instant::now()).is_empty(),
            "배달된 통보는 미결이 아니다"
        );
    }

    // ══════════════════════════════════════════════════════════════════════════════════════════
    // S18 send overhaul acceptance criteria (spec §7) — ADR-0111/0112/0114
    //
    // ★The contract this section is canonical for★: absence · duplicate name = per-recipient failure row · multi-recipient fan-out (one snapshot · one path) ·
    //   `@all` = everyone but me (a macro) · partial progress · envelope `to` freeze · multi-request N contracts · binding/reclaim abolished.
    //   A change that loosens the assertions here is a **contract change** (no relaxing without user ratification).
    // ══════════════════════════════════════════════════════════════════════════════════════════

    fn rows(v: &[RecipientResult]) -> Vec<(String, SendStatus, Option<FailCode>)> {
        v.iter().map(|r| (r.to.clone(), r.status, r.code)).collect()
    }

    fn send(
        svc: &Arc<MessagingService>,
        msg_id: &str,
        from: SenderIdentity,
        sender_name: &str,
        to: &[&str],
    ) -> Result<Vec<RecipientResult>, SendReject> {
        send_body(svc, msg_id, from, sender_name, to, "body")
    }

    /// A notification send that specifies the body — used when the **injection order must be asserted in bytes** (with the same body the
    ///   two letters in a batch cannot be told apart).
    fn send_body(
        svc: &Arc<MessagingService>,
        msg_id: &str,
        from: SenderIdentity,
        sender_name: &str,
        to: &[&str],
        body: &str,
    ) -> Result<Vec<RecipientResult>, SendReject> {
        let list: Vec<String> = to.iter().map(|t| t.to_string()).collect();
        svc.handle_send(
            msg_id,
            from,
            sender_name,
            &list,
            body,
            Entrance::Mcp,
            &SendMeta::default(),
        )
    }

    #[test]
    fn an_absent_recipient_is_a_failed_row_that_never_parks_but_always_ledgers() {
        let (svc, port) = svc();
        let (from, me) = live_sender("alice");
        port.set_roster(vec![me]);

        let out = send(&svc, "m1", from, "alice", &["ghost"]).expect("행 응답(전체 반려 아님)");
        assert_eq!(
            rows(&out),
            vec![(
                "ghost".to_string(),
                SendStatus::Failed,
                Some(FailCode::RecipientNotFound)
            )],
            "부재 1인 발송의 답은 error 가 아니라 failed 행 1개다(경로 1벌 — spec §5)"
        );
        assert!(out[0].hint.is_some(), "실패 행엔 code + hint 필수(spec §6)");

        assert_eq!(svc.parked_total_for_test(), 0, "파킹 큐에 안 실린다");
        assert!(
            svc.ledger_statuses("m1")
                .iter()
                .all(|s| *s != DeliveryStatus::Pending),
            "장부에 pending 상태 레코드가 없어야(파킹한 적 없음)"
        );
        assert_eq!(
            svc.ledger_statuses("m1"),
            vec![DeliveryStatus::Failed],
            "종점 `failed` 행 1개만 남는다"
        );

        let view = svc
            .message_state("m1", Instant::now())
            .expect("실패 행도 조회된다");
        assert_eq!(view.rows.len(), out.len());
        assert!(!view.may_be_truncated, "기대 행수와 남은 행수가 같다");
        assert_eq!(view.rows[0].status, "failed");
    }

    #[test]
    fn one_roster_snapshot_drives_the_whole_fan_out_and_at_all_uses_the_direct_path() {
        let (svc, port) = svc();
        let (from, me) = live_sender("alice");
        let (_b, bob) = live("bob");
        let (_c, carol) = live("carol");
        port.set_roster(vec![me, bob.clone(), carol.clone()]);

        let via_all = send(&svc, "m-all", from, "alice", &["@all"]).expect("ok");
        let via_names = send(&svc, "m-names", from, "alice", &["bob", "carol"]).expect("ok");
        assert_eq!(
            rows(&via_all),
            rows(&via_names),
            "`@all` 펼침 결과와 직접 지목이 **같은 결말**이어야(경로 1벌 — ADR-0111 결정 4)"
        );
        assert_eq!(
            rows(&via_all)
                .iter()
                .map(|(t, s, _)| (t.clone(), *s))
                .collect::<Vec<_>>(),
            vec![
                ("bob".to_string(), SendStatus::Delivered),
                ("carol".to_string(), SendStatus::Delivered),
            ]
        );

        let before = port.roster_calls();
        let _ = send(&svc, "m-snap", from, "alice", &["@all", "bob"]).expect("ok");
        assert_eq!(
            port.roster_calls() - before,
            1,
            "수신자 수와 무관하게 로스터 조회는 발송당 1회(ADR-0111 결정 2)"
        );
    }

    #[test]
    fn at_all_excludes_the_sender_but_explicit_self_naming_still_delivers() {
        let (svc, port) = svc();
        let (from, me) = live_sender("alice");
        port.set_roster(vec![me]);

        assert_eq!(
            send(&svc, "m1", from, "alice", &["@all"]),
            Err(SendReject::GroupEmpty),
            "발신자만 살아 있으면 @all 은 최종 집합 0명 → 전체 반려"
        );
        assert!(
            svc.ledger_snapshot().is_empty(),
            "전체 반려는 부작용 0(장부 레코드 없음)"
        );

        let out = send(&svc, "m2", from, "alice", &["@all", "alice"]).expect("반려 아님");
        assert_eq!(
            rows(&out),
            vec![("alice".to_string(), SendStatus::Delivered, None)],
            "직접 지목 자기발송은 살아남는다(제외는 **펼침에만** 적용 — spec §4)"
        );

        port.set_dormant(&["sleepy"]);
        let all = send(&svc, "m3", from, "alice", &["@all"]).expect("잠든 이름이 있으면 반려 아님");
        assert_eq!(
            rows(&all),
            vec![("sleepy".to_string(), SendStatus::Pending, None)],
            "@all 은 잠든 이름을 펼쳐 파킹한다(발신자는 여전히 제외): {all:?}"
        );
        assert_eq!(
            send(&svc, "m4", from, "alice", &["@here"]),
            Err(SendReject::GroupEmpty),
            "@here 는 잠든 이름을 보지 않으므로 여전히 전체 반려"
        );
    }

    #[test]
    fn an_explicit_dormant_token_and_at_all_fold_into_one_row() {
        let (svc, port) = svc();
        let (from, me) = live_sender("alice");
        let (_b, bob) = live("bob");
        port.set_roster(vec![me, bob]);
        port.set_dormant(&["sleepy"]);

        let out = send(&svc, "m1", from, "alice", &["sleepy", "@all"]).expect("반려 아님");
        assert_eq!(
            rows(&out),
            vec![
                ("sleepy".to_string(), SendStatus::Pending, None),
                ("bob".to_string(), SendStatus::Delivered, None),
            ],
            "잠든 이름은 명시 지목 자리에 **1행**만 남는다(펼침분은 흡수): {out:?}"
        );
        assert_eq!(
            svc.parked_len(tid("sleepy")),
            1,
            "파킹도 1건이어야(이중 배달 금지)"
        );
        assert_eq!(
            svc.ledger_statuses("m1"),
            vec![DeliveryStatus::Pending, DeliveryStatus::Delivered],
            "장부 행수 = 응답 행수(2행)"
        );
    }

    #[test]
    fn at_all_delivers_to_each_live_twin_in_its_own_row() {
        // ★Expansion resolves directly by registry id (ADR-0309 decision 3 · TRD S29 §4 X1)★ — two live sessions with the same name are different
        //   recipients. Resolving again by name folds them into one row `RECIPIENT_AMBIGUOUS` and neither receives it.
        let (svc, port) = svc();
        let (from, me) = live_sender("alice");
        let (d1, dup1) = live_distinct("dup");
        let (d2, dup2) = live_distinct("dup");
        let (z, zed) = live("zed");
        port.set_roster(vec![me, dup1, dup2, zed]);

        let out = send(&svc, "m1", from, "alice", &["@all"]).expect("ok");
        assert_eq!(
            rows(&out),
            vec![
                ("dup".to_string(), SendStatus::Delivered, None),
                ("dup".to_string(), SendStatus::Delivered, None),
                ("zed".to_string(), SendStatus::Delivered, None),
            ],
            "쌍둥이는 각자 행 · 둘 다 배달"
        );
        assert_eq!(port.injected_targets(), vec![d1, d2, z]);
    }

    #[test]
    fn mixed_to_folds_duplicates_once_and_row_order_is_deterministic() {
        let (svc, port) = svc();
        let (from, me) = live_sender("alice");
        let (_b, bob) = live("bob");
        let (_c, carol) = live("carol");
        let (_z, zed) = live("zed");
        port.set_roster(vec![me, bob, carol, zed]);

        let out = send(&svc, "m1", from, "alice", &["zed", "@all"]).expect("ok");
        assert_eq!(
            out.iter().map(|r| r.to.as_str()).collect::<Vec<_>>(),
            vec!["zed", "bob", "carol"],
            "명시 토큰(zed) 먼저 → 그 뒤 펼침 결과 사전순(bob, carol). zed 는 한 줄뿐"
        );
        assert_eq!(
            out.len(),
            3,
            "@all 에 이미 든 이름을 겹쳐 적어도 행이 늘지 않는다"
        );
        assert_eq!(port.injected_bodies().len(), 3, "겹친 수신자에게 배달 1회");
    }

    #[test]
    fn a_name_typo_fails_only_its_own_row_while_live_recipients_still_receive() {
        let (svc, port) = svc();
        let (from, me) = live_sender("alice");
        let (_b, bob) = live("bob");
        port.set_roster(vec![me, bob]);

        let out = send(&svc, "m1", from, "alice", &["없는이름", "bob"]).expect("부분 진행");
        assert_eq!(
            rows(&out),
            vec![
                (
                    "없는이름".to_string(),
                    SendStatus::Failed,
                    Some(FailCode::RecipientNotFound)
                ),
                ("bob".to_string(), SendStatus::Delivered, None),
            ]
        );
        assert_eq!(port.injected_bodies().len(), 1, "산 수신자에겐 그대로 간다");

        let before = port.injected_bodies().len();
        assert!(matches!(
            send(&svc, "m2", from, "alice", &["@typo", "bob"]),
            Err(SendReject::GroupNotFound { .. })
        ));
        assert_eq!(port.injected_bodies().len(), before, "전체 반려는 배달 0");
    }

    #[test]
    fn partial_progress_mixes_delivered_pending_and_failed_in_one_send() {
        let (svc, port, gate) = svc_gated();
        let (from, me) = live_sender("alice");
        let (b_id, bob) = live("bob");
        let (_c, carol) = live("carol");
        port.set_roster(vec![me, bob, carol]);
        gate.set_busy(b_id, 0);

        let out = svc
            .handle_send(
                "m1",
                from,
                "alice",
                &["bob".to_string(), "carol".to_string(), "ghost".to_string()],
                "body",
                Entrance::Mcp,
                &SendMeta::default(),
            )
            .expect("부분 진행은 Ok");
        assert_eq!(
            rows(&out),
            vec![
                ("bob".to_string(), SendStatus::Pending, None),
                ("carol".to_string(), SendStatus::Delivered, None),
                (
                    "ghost".to_string(),
                    SendStatus::Failed,
                    Some(FailCode::RecipientNotFound)
                ),
            ]
        );

        let all_failed = send(&svc, "m2", from, "alice", &["ghost1", "ghost2"]).expect("Ok shape");
        assert!(
            all_failed.iter().all(|r| r.status == SendStatus::Failed),
            "전 행 failed"
        );
        assert_eq!(all_failed.len(), 2);
    }

    #[test]
    fn a_full_mailbox_fails_only_that_row_without_any_reclaim_attempt() {
        let (svc, port, gate) = svc_gated();
        let (from, me) = live_sender("alice");
        let (f_id, full) = live("full");
        let (_c, carol) = live("carol");
        port.set_roster(vec![me, full, carol]);
        gate.set_busy(f_id, 0);

        for i in 0..100 {
            let out = send(&svc, &format!("f{i}"), from, "alice", &["full"]).expect("cap 이내");
            assert_eq!(out[0].status, SendStatus::Pending, "{i}번째");
        }
        assert_eq!(svc.parked_len(tid("full")), 100);
        let oldest_before = svc.parked_msg_ids(tid("full"))[0].clone();

        let out = send(&svc, "over", from, "alice", &["full", "carol"]).expect("부분 진행");
        assert_eq!(
            rows(&out),
            vec![
                (
                    "full".to_string(),
                    SendStatus::Failed,
                    Some(FailCode::MailboxFull)
                ),
                ("carol".to_string(), SendStatus::Delivered, None),
            ],
            "가득 찬 한 수신자가 나머지 배달을 막지 않는다(전체 반려 승격 없음)"
        );
        assert_eq!(svc.parked_len(tid("full")), 100, "큐 길이 불변");
        assert_eq!(
            svc.parked_msg_ids(tid("full"))[0],
            oldest_before,
            "가장 오래된 항목이 그대로 있다 = **회수 시도 없음**(ADR-0114 결정 1)"
        );
        assert_eq!(
            svc.ledger_statuses("over"),
            vec![DeliveryStatus::Failed, DeliveryStatus::Delivered],
            "실패 수신자도 장부 종점 행을 남긴다"
        );
    }

    #[test]
    fn a_multi_recipient_request_opens_one_independent_contract_per_recipient() {
        let (svc, port) = svc();
        let (from, me) = live_sender("alice");
        let (b_id, bob) = live("bob");
        let (c_id, carol) = live("carol");
        port.set_roster(vec![me, bob, carol]);

        let out = svc
            .handle_send(
                "m-req",
                from,
                "alice",
                &["bob".to_string(), "carol".to_string()],
                "해줘",
                Entrance::Mcp,
                &req_meta("10m", 600),
            )
            .expect("ok");
        assert!(out.iter().all(|r| r.status == SendStatus::Delivered));
        assert_eq!(svc.open_request_count(), 2, "수신자마다 계약 1건");

        let bob_from = SenderIdentity {
            peer_id: b_id,
            epoch: 0,
        };
        svc.handle_send(
            "m-rep",
            bob_from,
            "bob",
            &["alice".to_string()],
            "했음",
            Entrance::Mcp,
            &reply_meta("m-req"),
        )
        .expect("회신 배달");
        assert_eq!(
            svc.open_request_count(),
            1,
            "전체회신 없음 — carol 계약은 그대로 열려 있다"
        );

        svc.sweep(Instant::now() + Duration::from_secs(601));
        let notices: Vec<_> = svc
            .ledger_snapshot()
            .into_iter()
            .filter(|(_, from, _, _)| from == NOTICE_SENDER_LABEL)
            .collect();
        assert_eq!(notices.len(), 1, "회신한 계약엔 통지가 없다: {notices:?}");
        let _ = c_id;
    }

    #[test]
    fn the_envelope_to_attribute_is_frozen_at_send_time_and_survives_the_flush() {
        let (svc, port, gate) = svc_gated();
        let (from, me) = live_sender("alice");
        let (b_id, bob) = live("bob");
        let (_c, carol) = live("carol");
        port.set_roster(vec![me, bob, carol]);
        gate.set_busy(b_id, 0);

        let out = svc
            .handle_send(
                "m1",
                from,
                "alice",
                &["@all".to_string(), "ghost".to_string()],
                "전원 대기",
                Entrance::Mcp,
                &SendMeta::default(),
            )
            .expect("ok");
        assert_eq!(
            rows(&out),
            vec![
                (
                    "ghost".to_string(),
                    SendStatus::Failed,
                    Some(FailCode::RecipientNotFound)
                ),
                ("bob".to_string(), SendStatus::Pending, None),
                ("carol".to_string(), SendStatus::Delivered, None),
            ]
        );
        assert_eq!(
            port.injected_bodies(),
            vec![r#"<message from="alice" to="@all">전원 대기</message>"#.to_string()],
            "수용 2인 → to 노출 · @주소는 펼치지 않고 토큰 그대로 · 실패 토큰 제외"
        );

        gate.clear();
        svc.flush_for(b_id);
        assert_eq!(
            port.injected_bodies().len(),
            2,
            "파킹분이 flush 로 배달됐다"
        );
        assert_eq!(
            port.injected_bodies()[1],
            r#"<message from="alice" to="@all">전원 대기</message>"#,
            "발송 순간 표기 그대로(park payload 가 동결값을 flush 까지 나른다)"
        );

        drop(svc);
        let (svc2, port2) = super::tests::svc();
        let (from2, me2) = live_sender("alice");
        let (_z, zed) = live("zed");
        port2.set_roster(vec![me2, zed]);
        send(&svc2, "m2", from2, "alice", &["zed"]).expect("ok");
        assert_eq!(
            port2.injected_bodies(),
            vec![r#"<message from="alice">body</message>"#.to_string()],
            "수용 1인 → to 속성 생략"
        );
    }

    #[test]
    fn the_to_attribute_lists_tokens_in_input_notation_order() {
        let (svc, port) = svc();
        let (from, me) = live_sender("alice");
        let (_b, bob) = live("bob");
        let (_c, carol) = live("carol");
        port.set_roster(vec![me, bob, carol]);

        send(&svc, "m1", from, "alice", &["@all", "carol"]).expect("ok");
        assert!(
            port.injected_bodies()[0].contains(r#"to="@all,carol""#),
            "발신자가 쓴 순서 그대로(@all 먼저, 그 다음 명시 이름): {:?}",
            port.injected_bodies()
        );
    }

    // ── Revived regressions (the load-bearing axes the old C4 suite held) ────────────────────────────────

    #[test]
    fn the_cap_denominator_counts_the_batch_that_left_the_queue_during_a_flush() {
        let (svc, port, gate) = svc_gated();
        let (from, me) = live_sender("alice");
        let (r_id, recv) = live("recv");
        port.set_roster(vec![me, recv]);
        gate.set_busy(r_id, 0);
        for i in 0..100 {
            send(&svc, &format!("p{i}"), from, "alice", &["recv"]).expect("cap 이내 파킹");
        }
        assert_eq!(svc.parked_len(tid("recv")), 100);

        gate.clear();
        let during: Arc<StdMutex<Vec<SendStatus>>> = Arc::new(StdMutex::new(Vec::new()));
        let (svc_h, from_h, seen_h) = (svc.clone(), from, during.clone());
        port.set_on_inject(Arc::new(move |idx| {
            if idx != 0 {
                return;
            }
            let out = svc_h
                .handle_send(
                    "concurrent",
                    from_h,
                    "alice",
                    &["recv".to_string()],
                    "b",
                    Entrance::Mcp,
                    &SendMeta::default(),
                )
                .expect("행 응답");
            seen_h.lock().unwrap().push(out[0].status);
        }));
        svc.flush_for(r_id);

        assert_eq!(
            during.lock().unwrap().as_slice(),
            &[SendStatus::Failed],
            "배치가 나가 있는 동안의 신규 유입은 분모(큐 + in-flight)에 걸려 반려된다(F1)"
        );
        assert_eq!(
            svc.ledger_statuses("concurrent"),
            vec![DeliveryStatus::Failed],
            "반려도 실패 행으로 장부에 남는다(ADR-0111 — 옛 '반려는 장부 미기록' 과 반전)"
        );
    }

    #[test]
    fn a_second_flush_defers_while_a_batch_is_in_flight_and_the_settlement_re_rings_the_doorbell() {
        let (svc, port, gate, bell) = svc_gated_with_doorbell();
        let (from, me) = live_sender("alice");
        let (r_id, recv) = live("recv");
        port.set_roster(vec![me, recv]);
        gate.set_busy(r_id, 0);
        for i in 0..2 {
            send(&svc, &format!("p{i}"), from, "alice", &["recv"]).expect("파킹");
        }
        gate.clear();
        bell.take(); // The parking doorbell is out of interest here — only the deferral/retake axis is looked at.

        let svc_h = svc.clone();
        port.set_on_inject(Arc::new(move |idx| {
            if idx == 0 {
                svc_h.flush_for(r_id);
            }
        }));
        svc.flush_for(r_id);

        assert_eq!(
            port.injected_bodies().len(),
            2,
            "겹쳐 돌지 않으므로 배치는 한 번만, 그러나 전부 나간다"
        );
        assert!(
            bell.take().contains(&r_id),
            "유예된 깨우기는 정산 시 **되울려야** 한다(lost wakeup 금지)"
        );
    }

    // ══════════════════════════════════════════════════════════════════════════════════════════
    // /review code deep — fix round 1 regression net (A2·A3·A5·A6·A7·A8 · C2·C4)
    //   ★A1 (reply contract handling) was **replaced in round 4 by the ratified rule**★ — that place is guarded by the two tests below
    //   (confirmed unreachable → `reply_failed` / transient reason → stays open + retry closes it). ADR-0116 decision 2 · ADR-0118.
    // ══════════════════════════════════════════════════════════════════════════════════════════

    #[test]
    fn a_reply_to_a_requester_that_exists_nowhere_fails_the_contract_and_stops_the_timeout() {
        let (svc, port) = svc();
        let (boss_from, boss) = live_sender("boss");
        let (w_id, worker) = live("worker");
        port.set_roster(vec![boss.clone(), worker]);

        svc.handle_send(
            "m-req",
            boss_from,
            "boss",
            &["worker".to_string()],
            "해줘",
            Entrance::Mcp,
            &req_meta("10m", 600),
        )
        .expect("배달");
        assert_eq!(svc.open_request_count(), 1);
        let slots_before = svc.occupied_slots_for_test();

        // The requester (boss) is dead and **has no profile either** — it becomes a name in no source.
        port.set_roster(vec![LiveAgent {
            id: w_id,
            name: "worker".to_string(),
            epoch: 0,
            turn_signal: true,
        }]);
        let worker_from = SenderIdentity {
            peer_id: w_id,
            epoch: 0,
        };
        let rows_out = svc
            .handle_send(
                "m-rep",
                worker_from,
                "worker",
                &["boss".to_string()],
                "했음",
                Entrance::Mcp,
                &reply_meta("m-req"),
            )
            .expect("행 응답");
        assert_eq!(
            rows_out[0].code,
            Some(FailCode::RecipientNotFound),
            "없는 요청자에게는 회신이 가지 않는다"
        );
        assert_eq!(
            svc.open_request_count(),
            0,
            "도달 불가 확정 회신은 계약을 reply_failed 로 **종결**한다(ADR-0116 결정 2)"
        );
        assert_eq!(
            svc.contract_outcome_for_test("m-req", tid("worker")),
            Some("reply_failed"),
            "종점 어휘는 replied 가 아니다 — 회신이 성립했다고 주장하지 않는다(spec §6 축 구분)"
        );
        assert_eq!(
            svc.occupied_slots_for_test(),
            slots_before - 1,
            "512 계수에서 빠진다(ADR-0118 결정 4 — 좀비가 상한을 먹지 않게)"
        );
        let owed = svc.open_items_for(w_id, Instant::now());
        assert!(
            !owed.iter().any(|i| i.id == "m-req"),
            "종결된 계약은 미결 목록에 없어야: {owed:?}"
        );
        assert_eq!(
            svc.ledger_statuses("m-req"),
            vec![DeliveryStatus::Delivered]
        );
        svc.sweep(Instant::now() + Duration::from_secs(601));
        assert!(
            !svc.ledger_snapshot()
                .iter()
                .any(|(_, from, _, _)| from == NOTICE_SENDER_LABEL),
            "종결된 계약엔 기한 초과 통지가 나가지 않아야"
        );
    }

    #[test]
    fn a_reply_that_failed_for_a_transient_reason_keeps_the_contract_open_and_a_retry_closes_it() {
        let (svc, port, gate) = svc_gated();
        let (boss_from, boss) = live_sender("boss");
        let (w_id, worker) = live("worker");
        let boss_id = boss_from.peer_id;
        port.set_roster(vec![boss.clone(), worker.clone()]);

        svc.handle_send(
            "m-req",
            boss_from,
            "boss",
            &["worker".to_string()],
            "해줘",
            Entrance::Mcp,
            &req_meta("10m", 600),
        )
        .expect("배달");

        gate.set_busy(boss_id, 0);
        for i in 0..100 {
            send(&svc, &format!("fill{i}"), boss_from, "boss", &["boss"]).expect("파킹");
        }
        let worker_from = SenderIdentity {
            peer_id: w_id,
            epoch: 0,
        };
        let rows_out = svc
            .handle_send(
                "m-rep",
                worker_from,
                "worker",
                &["boss".to_string()],
                "했음",
                Entrance::Mcp,
                &reply_meta("m-req"),
            )
            .expect("행 응답");
        assert_eq!(
            rows_out[0].code,
            Some(FailCode::MailboxFull),
            "보관함이 가득이라 이 회신은 실패 행이다"
        );
        assert_eq!(
            svc.contract_outcome_for_test("m-req", tid("worker")),
            Some("awaiting_reply"),
            "일시 사유 실패는 **무동작** — 계약은 오픈 유지(ADR-0118 결정 2)"
        );

        svc.flush_for(boss_id);
        gate.clear();
        svc.flush_for(boss_id);
        let retry = svc
            .handle_send(
                "m-rep2",
                worker_from,
                "worker",
                &["boss".to_string()],
                "했음(재시도)",
                Entrance::Mcp,
                &reply_meta("m-req"),
            )
            .expect("행 응답");
        assert_eq!(
            retry[0].status,
            SendStatus::Delivered,
            "재시도는 배달된다: {retry:?}"
        );
        assert_eq!(
            svc.contract_outcome_for_test("m-req", tid("worker")),
            Some("replied"),
            "재시도 성공이 정상 경로로 계약을 닫는다(오픈 유지의 존재 이유)"
        );
    }

    #[test]
    fn an_ambiguous_reply_target_also_leaves_the_contract_open() {
        let (svc, port) = svc();
        let (boss_from, boss) = live_sender("boss");
        let (w_id, worker) = live("worker");
        port.set_roster(vec![boss.clone(), worker]);
        svc.handle_send(
            "m-req",
            boss_from,
            "boss",
            &["worker".to_string()],
            "해줘",
            Entrance::Mcp,
            &req_meta("10m", 600),
        )
        .expect("배달");

        let (_b2, boss_twin) = live_distinct("boss");
        let (_w2, worker_again) = live("worker");
        port.set_roster(vec![boss, boss_twin, worker_again]);
        let worker_from = SenderIdentity {
            peer_id: w_id,
            epoch: 0,
        };
        let rows_out = svc
            .handle_send(
                "m-rep",
                worker_from,
                "worker",
                &["boss".to_string()],
                "했음",
                Entrance::Mcp,
                &reply_meta("m-req"),
            )
            .expect("행 응답");
        assert_eq!(
            rows_out[0].code,
            Some(FailCode::RecipientAmbiguous),
            "동명 요청자에게는 배달할 수 없다: {rows_out:?}"
        );
        assert_eq!(
            svc.contract_outcome_for_test("m-req", tid("worker")),
            Some("awaiting_reply"),
            "동명 실패는 **무동작** — 계약 오픈 유지(사용자가 이름을 정리하면 재시도가 닫는다)"
        );
        assert_eq!(
            svc.open_request_count(),
            1,
            "512 계수에서 빠지지 않는다(종결이 아니다)"
        );
    }

    #[test]
    fn a_planned_retirement_that_vanished_before_commit_is_not_reported() {
        let (svc, port) = svc();
        let (from, me) = live_sender("boss");
        port.set_roster(vec![me]);
        fill_open_request_cap_evictable(&svc, from);
        let _ = retirement_reports::drain();

        let res = reserve_marked_contract(&svc, from, "m-phantom");
        let (vid, vto) = {
            let r = res.retired.as_ref().expect("전제: 희생자가 표시됐다");
            (r.request_id.clone(), r.recipient_id)
        };

        {
            let mut st = svc.state.lock().expect("lock");
            let now = Instant::now();
            // ① Push the victim's history row out of the ring (it is still open so it is not an evict cleanup target).
            //    The ring capacity constant is not duplicated — it looks directly at "has that row vanished" and stops.
            let mut i = 0usize;
            while !st.ledger.records_for(&vid).is_empty() {
                st.ledger.record(
                    &format!("flood{i}"),
                    "boss",
                    tid("boss"),
                    "sink",
                    Some(tid("sink")),
                    "x",
                    DeliveryStatus::Delivered,
                    now,
                );
                i += 1;
                assert!(i < 100_000, "링 회전이 관측되지 않는다 — 픽스처 전제 붕괴");
            }
            // ② Close and ③ one more row → the purge on the evict path cleans up that contract that is "finished and has no history".
            st.ledger.close_for_test(&vid, vto, now);
            st.ledger.record(
                "flood-last",
                "boss",
                tid("boss"),
                "sink",
                Some(tid("sink")),
                "x",
                DeliveryStatus::Delivered,
                now,
            );
        }
        assert!(
            !svc.contract_tracked_for_test(&vid, vto),
            "전제: 희생자가 커밋 전에 사라졌다(purge) — 이 전제가 깨지면 아래 단언이 무의미하다"
        );

        let mut log = RetirementLog::default();
        {
            let mut st = svc.state.lock().expect("lock");
            res.commit(&mut st, &mut log);
        }
        assert!(
            log.real.is_empty(),
            "커밋이 실제로 제거한 게 없으면 은퇴로 세지 않는다: {:?}",
            log.real
        );
        assert_eq!(log.phantom.len(), 1, "대신 이상(유령 계획)으로 남는다");
        log_contract_retirements("m-phantom", &log.real);
        let reports = retirement_reports::drain();
        assert!(
            reports.is_empty(),
            "커밋이 실제로 제거한 게 없으면 은퇴 보고도 없어야(유령 은퇴 금지): {reports:?}"
        );
        assert!(
            svc.contract_tracked_for_test("m-phantom", tid("ghost-worker")),
            "예약 자신의 계약은 살아 있어야"
        );
        assert_eq!(svc.marked_retirements_for_test(), 0, "표시가 남지 않는다");
    }

    #[test]
    fn a_retirement_is_actually_reported_and_only_when_it_happened() {
        let (svc, port) = svc();
        let (from, me) = live_sender("boss");
        let (_w, worker) = live("worker");
        port.set_roster(vec![me, worker]);
        fill_open_request_cap_evictable(&svc, from);
        let _ = retirement_reports::drain();

        // ① A send where retirement really happens (an idle recipient = the immediate-delivery branch).
        let out = svc
            .handle_send(
                "m-retire",
                from,
                "boss",
                &["worker".to_string()],
                "해줘",
                Entrance::Mcp,
                &SendMeta {
                    request: true,
                    ..SendMeta::default()
                },
            )
            .expect("행 응답");
        assert_eq!(out[0].status, SendStatus::Delivered, "전제: 즉시 배달 갈래");
        let reports = retirement_reports::drain();
        assert_eq!(
            reports.len(),
            1,
            "은퇴 1건은 정확히 1줄로 보고돼야 — 계측이 결말 루프 앞에 있으면 0줄이다: {reports:?}"
        );
        assert!(
            !reports[0].0.is_empty() && reports[0].0 != "m-retire",
            "보고는 **내보낸 옛 계약**을 지목해야(새 발송이 아니라): {reports:?}"
        );
        assert!(
            !svc.contract_tracked_for_test(&reports[0].0, reports[0].1),
            "보고된 계약은 실제로 사라졌어야(일어나지 않은 일을 보고하면 안 된다)"
        );

        // ② A send that ended in failure (fill the limit with **non-retirable** ones and end with RequestCapacity — it must start with a new service so
        //    the retirable contract of ① is not picked as a victim).
        let (svc, port) = super::tests::svc();
        let (from, me) = live_sender("boss");
        let (_w2, worker2) = live("worker");
        port.set_roster(vec![me, worker2]);
        fill_open_request_cap(&svc, from);
        let _ = retirement_reports::drain();
        let out2 = svc
            .handle_send(
                "m-nope",
                from,
                "boss",
                &["worker".to_string()],
                "해줘",
                Entrance::Mcp,
                &req_meta("10m", 600),
            )
            .expect("행 응답");
        assert_eq!(
            out2[0].code,
            Some(FailCode::RequestCapacity),
            "전제: 상한 포화로 실패 행: {out2:?}"
        );
        assert!(
            retirement_reports::drain().is_empty(),
            "일어나지 않은 은퇴를 보고하면 운영자가 유령 은퇴를 쫓는다"
        );
    }

    #[test]
    fn a_request_that_ends_as_a_failed_row_retires_nobody() {
        {
            let (svc, port, gate) = svc_gated();
            let (from, me) = live_sender("boss");
            let (t_id, target) = live("worker");
            let (_l, lead) = live("lead");
            port.set_roster(vec![me, target, lead]);
            // Fills only the worker queue to the cap — it is a notification so it is unrelated to the contract axis.
            gate.set_busy(t_id, 0);
            for i in 0..100 {
                send(&svc, &format!("filler{i}"), from, "boss", &["worker"]).expect("파킹");
            }
            fill_open_request_cap_evictable(&svc, from);
            let occupied_before = svc.occupied_slots_for_test();
            let tracking_before = svc.tracking_len_for_test();
            assert_eq!(svc.marked_retirements_for_test(), 0, "전제: 남은 표시 없음");
            let _ = retirement_reports::drain();

            let out = svc
                .handle_send(
                    "m-late",
                    from,
                    "boss",
                    &["lead".to_string(), "worker".to_string()],
                    "해줘",
                    Entrance::Mcp,
                    &req_meta("10m", 600),
                )
                .expect("행 응답");
            let worker_row = out.iter().find(|r| r.to == "worker").expect("worker 행");
            assert_eq!(
                worker_row.code,
                Some(FailCode::MailboxFull),
                "보관함이 가득한 수신자는 cap 게이트에서 실패 행으로 끝나야: {out:?}"
            );
            let lead_row = out.iter().find(|r| r.to == "lead").expect("lead 행");
            assert_eq!(
                lead_row.status,
                SendStatus::Delivered,
                "전제: 앞 수신자는 정상 수용돼 자기 희생자를 실제로 은퇴시킨다: {out:?}"
            );
            assert_eq!(
                svc.marked_retirements_for_test(),
                0,
                "실패로 끝난 request 는 **아무도 은퇴시키지 않는다** — 표시가 남으면 안 된다"
            );
            assert_eq!(
                svc.occupied_slots_for_test(),
                occupied_before,
                "cap 분모가 그대로여야(성공분은 −희생자 +신규로 상쇄) — 실패분 표시가 남으면 분모가 영구히 준다"
            );
            assert_eq!(
                svc.tracking_len_for_test(),
                tracking_before,
                "잠정 계약도 남지 않아야 — 남으면 추적이 무계로 자란다"
            );
            assert!(!svc.contract_tracked_for_test("m-late", tid("worker")));
            assert!(
                svc.contract_tracked_for_test("m-late", tid("lead")),
                "실패한 수신자의 롤백이 같은 발송의 성공한 수신자 계약까지 걷어가면 안 된다"
            );
            let reports = retirement_reports::drain();
            assert_eq!(
                reports.len(),
                1,
                "은퇴는 수용된 수신자 몫 1건뿐: {reports:?}"
            );
            assert_ne!(
                reports[0].1,
                tid("worker"),
                "실패 행은 은퇴의 주체가 아니다"
            );
        }

        // ── pass A branch (separate assertion) — when the queue is already full at the decision stage ─────────────────────
        let (svc, port, gate) = svc_gated();
        let (from, me) = live_sender("boss");
        let (f_id, full) = live("full");
        port.set_roster(vec![me, full]);
        gate.set_busy(f_id, 0);
        for i in 0..100 {
            send(&svc, &format!("p{i}"), from, "boss", &["full"]).expect("파킹");
        }
        fill_open_request_cap_evictable(&svc, from);
        let occupied_before = svc.occupied_slots_for_test();
        let out = svc
            .handle_send(
                "m-passa",
                from,
                "boss",
                &["full".to_string()],
                "해줘",
                Entrance::Mcp,
                &req_meta("10m", 600),
            )
            .expect("행 응답");
        assert_eq!(out[0].code, Some(FailCode::MailboxFull));
        assert_eq!(
            svc.marked_retirements_for_test(),
            0,
            "pass A 실패도 은퇴를 성립시키지 않는다"
        );
        assert_eq!(svc.occupied_slots_for_test(), occupied_before);
    }

    #[test]
    fn a_send_never_cannibalizes_a_contract_it_opened_itself() {
        // ★The fixture shape is load-bearing★: if the limit is filled only with contracts **older** than this send, victim selection
        //   (`min_by_key(created_at)`) cannot pick this send's own contract in the first place and the test becomes vacuous.
        let (svc, port) = svc();
        let (from, me) = live_sender("boss");
        let (_a, w1) = live("w1");
        let (_b, w2) = live("w2");
        port.set_roster(vec![me, w1, w2]);

        fill_open_request_cap_leaving_one_free(&svc, from);
        svc.park_absent_for_test(
            "spare",
            from,
            "boss",
            "spare-victim",
            "q",
            Entrance::Mcp,
            &SendMeta {
                request: true,
                ..SendMeta::default()
            },
        )
        .expect("여유 1자리를 은퇴 가능 계약으로 메운다");

        let out = svc
            .handle_send(
                "m-two",
                from,
                "boss",
                &["w1".to_string(), "w2".to_string()],
                "해줘",
                Entrance::Mcp,
                &SendMeta {
                    request: true,
                    ..SendMeta::default()
                },
            )
            .expect("행 응답");
        assert_eq!(out.len(), 2);
        assert_eq!(
            out[0].status,
            SendStatus::Delivered,
            "첫 수신자는 여유 희생자를 얻어 접수된다: {out:?}"
        );
        assert!(
            svc.contract_tracked_for_test("m-two", tid("w1")),
            "★핵심★: 첫 수신자의 계약이 살아 있어야 한다 — 같은 발송의 두 번째 수신자가 잡아먹으면 안 된다"
        );
        match out[1].status {
            SendStatus::Failed => assert_eq!(
                out[1].code,
                Some(FailCode::RequestCapacity),
                "두 번째 수신자의 실패 사유는 상한뿐(자기 발송을 먹지 못하므로): {out:?}"
            ),
            _ => assert!(
                svc.contract_tracked_for_test("m-two", tid("w2")),
                "접수된 수신자는 자기 계약을 가져야: {out:?}"
            ),
        }
        // Cross-check by sender attribution — number of accepted recipients == number of contracts opened by this send (attribution instead of the total).
        let admitted = out
            .iter()
            .filter(|r| r.status != SendStatus::Failed)
            .count();
        let mine = svc
            .open_items_for(from.peer_id, Instant::now())
            .into_iter()
            .filter(|i| i.direction == Direction::AwaitingTheirReply && i.id == "m-two")
            .count();
        assert_eq!(
            mine, admitted,
            "접수된 수신자마다 계약이 하나씩 있어야(자기잠식이면 모자란다)"
        );
    }

    #[test]
    fn an_unsettled_reservation_panics_in_debug_and_rolls_itself_back() {
        let (svc, port) = svc();
        let (from, me) = live_sender("boss");
        port.set_roster(vec![me]);
        fill_open_request_cap_evictable(&svc, from);
        let occupied_before = svc.occupied_slots_for_test();
        let tracking_before = svc.tracking_len_for_test();

        {
            let mut st = svc.state.lock().expect("lock");
            let retired = match st.ledger.open_request(
                "m-drop",
                "boss",
                from.peer_id,
                "ghost-worker",
                tid("ghost-worker"),
                None,
                Instant::now(),
            ) {
                OpenOutcome::OpenedAfterMarking(rc) => Some(rc),
                other => panic!("전제: 상한 압력으로 표시가 붙어야 — {other:?}"),
            };
            assert_eq!(st.ledger.marked_retirement_count_for_test(), 1, "표시 1건");
            let res = open_reservation(&mut st, &svc, "m-drop", tid("ghost-worker"), retired);
            drop(st); // Release the lock so that Drop's try_lock succeeds.
            let hook = std::panic::take_hook();
            std::panic::set_hook(Box::new(|_| {}));
            let hit = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
                let _unsettled = res;
            }));
            std::panic::set_hook(hook);
            assert!(
                hit.is_err(),
                "정산 없이 소멸하면 debug 빌드에서 즉시 터져야(H1 — 잊은 갈래를 테스트가 잡는다)"
            );
        }

        assert!(
            !svc.contract_tracked_for_test("m-drop", tid("ghost-worker")),
            "Drop 이 잠정 계약을 제거해야"
        );
        assert_eq!(
            svc.marked_retirements_for_test(),
            0,
            "Drop 이 희생자 표시도 풀어야 — 남으면 cap 분모가 영구히 줄어든다"
        );
        assert_eq!(svc.occupied_slots_for_test(), occupied_before);
        assert_eq!(svc.tracking_len_for_test(), tracking_before);
    }

    #[test]
    fn a_parked_recipient_keeps_the_contract_its_own_send_opened() {
        let (svc, port, gate) = svc_gated();
        let (from, me) = live_sender("boss");
        let (w1_id, w1) = live("w1");
        let (_b, w2) = live("w2");
        port.set_roster(vec![me, w1, w2]);
        gate.set_busy(w1_id, 0);

        fill_open_request_cap_leaving_one_free(&svc, from);
        svc.park_absent_for_test(
            "spare",
            from,
            "boss",
            "spare-victim",
            "q",
            Entrance::Mcp,
            &SendMeta {
                request: true,
                ..SendMeta::default()
            },
        )
        .expect("여유 1자리를 은퇴 가능 계약으로 메운다");

        let out = svc
            .handle_send(
                "m-park",
                from,
                "boss",
                &["w1".to_string(), "w2".to_string()],
                "해줘",
                Entrance::Mcp,
                &SendMeta {
                    request: true,
                    ..SendMeta::default()
                },
            )
            .expect("행 응답");
        assert_eq!(out.len(), 2);
        assert_eq!(
            out[0].status,
            SendStatus::Pending,
            "전제: 첫 수신자는 턴 중이라 파킹된다(이 갈래를 태우는 게 이 테스트의 목적): {out:?}"
        );
        assert_eq!(svc.parked_len(tid("w1")), 1, "실제로 큐에 실렸다");
        assert!(
            svc.contract_tracked_for_test("m-park", tid("w1")),
            "★핵심★: 파킹된 수신자의 계약이 살아 있어야 — 같은 발송의 두 번째 수신자가 잡아먹으면 안 된다"
        );
        assert!(
            !svc.contract_tracked_for_test("spare", tid("spare-victim")),
            "파킹 갈래도 **커밋**으로 정산돼 표시된 희생자를 물리 제거해야(롤백/미정산이면 표시가 남아 cap 분모가 영구히 준다)"
        );
        assert_eq!(
            svc.marked_retirements_for_test(),
            0,
            "정산이 끝나면 표시가 남아 있을 수 없다"
        );
        match out[1].status {
            SendStatus::Failed => assert_eq!(
                out[1].code,
                Some(FailCode::RequestCapacity),
                "두 번째 수신자의 실패 사유는 상한뿐(자기 발송을 먹지 못하므로): {out:?}"
            ),
            _ => assert!(
                svc.contract_tracked_for_test("m-park", tid("w2")),
                "접수된 수신자는 자기 계약을 가져야: {out:?}"
            ),
        }
        let admitted = out
            .iter()
            .filter(|r| r.status != SendStatus::Failed)
            .count();
        let mine = svc
            .open_items_for(from.peer_id, Instant::now())
            .into_iter()
            .filter(|i| i.direction == Direction::AwaitingTheirReply && i.id == "m-park")
            .count();
        assert_eq!(
            mine, admitted,
            "접수된 수신자마다 계약이 하나씩 — 파킹된 계약이 잠정으로 남으면 여기서 모자란다"
        );
    }

    /// Makes one provisional reservation **with the mark attached** under limit pressure (the caller settles it / does not).
    ///
    /// The two hard paths of F1 (`try_lock` failure · unwinding) assume "a state where the reservation already exists", so only that
    /// premise is made separately — going through the send path would finish settlement inside it and make reproduction impossible.
    fn reserve_marked_contract<'a>(
        svc: &'a Arc<MessagingService>,
        from: SenderIdentity,
        msg_id: &str,
    ) -> Reservation<'a> {
        let mut st = svc.state.lock().expect("lock");
        let retired = match st.ledger.open_request(
            msg_id,
            "boss",
            from.peer_id,
            "ghost-worker",
            tid("ghost-worker"),
            None,
            Instant::now(),
        ) {
            OpenOutcome::OpenedAfterMarking(rc) => Some(rc),
            other => panic!("전제: 상한 압력으로 표시가 붙어야 — {other:?}"),
        };
        assert_eq!(st.ledger.marked_retirement_count_for_test(), 1, "표시 1건");
        let res = open_reservation(&mut st, svc, msg_id, tid("ghost-worker"), retired);
        drop(st);
        res
    }

    fn catch_quietly(f: impl FnOnce()) -> std::thread::Result<()> {
        let hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
        std::panic::set_hook(hook);
        r
    }

    #[test]
    fn a_sweep_during_a_slow_injection_never_touches_the_live_reservation() {
        let (svc, port) = svc();
        let (from, me) = live_sender("boss");
        let (w_id, worker) = live("worker");
        port.set_roster(vec![me, worker]);
        fill_open_request_cap_evictable(&svc, from);
        let occupied_before = svc.occupied_slots_for_test();
        let _ = retirement_reports::drain();

        {
            let svc2 = svc.clone();
            port.set_on_inject(Arc::new(move |idx| {
                if idx != 0 {
                    return;
                }
                // Why the time is given as **one hour later**: it means no time threshold could have saved this send.
                svc2.sweep(Instant::now() + Duration::from_secs(3600));
            }));
        }
        let out = svc
            .handle_send(
                "m-slow",
                from,
                "boss",
                &["worker".to_string()],
                "해줘",
                Entrance::Mcp,
                &req_meta("10m", 600),
            )
            .expect("행 응답");
        assert_eq!(
            out[0].status,
            SendStatus::Delivered,
            "전제: 주입 갈래를 탔다: {out:?}"
        );

        assert!(
            svc.contract_tracked_for_test("m-slow", tid("worker")),
            "소유자가 살아 있는 예약을 sweep 이 회수하면 안 된다(R1)"
        );
        let sender_side: Vec<String> = svc
            .open_items_for(from.peer_id, Instant::now())
            .into_iter()
            .filter(|i| i.id == "m-slow")
            .map(|i| i.direction.as_str().to_string())
            .collect();
        assert_eq!(
            sender_side,
            vec!["awaiting_their_reply".to_string()],
            "발신자는 회신을 기다려야: {sender_side:?}"
        );
        let recipient_side: Vec<String> = svc
            .open_items_for(w_id, Instant::now())
            .into_iter()
            .filter(|i| i.id == "m-slow")
            .map(|i| i.direction.as_str().to_string())
            .collect();
        assert_eq!(
            recipient_side,
            vec!["reply_owed_by_me".to_string()],
            "수신자는 답할 의무를 봐야: {recipient_side:?}"
        );

        let reports = retirement_reports::drain();
        assert_eq!(reports.len(), 1, "은퇴 1건 = 보고 1줄: {reports:?}");
        assert_eq!(svc.marked_retirements_for_test(), 0, "표시가 남지 않는다");
        assert_eq!(
            svc.occupied_slots_for_test(),
            occupied_before,
            "cap 산술도 그대로(은퇴 1 − 신규 1)"
        );

        svc.handle_send(
            "m-reply",
            reply_from(w_id),
            "worker",
            &["boss".to_string()],
            "다 했다",
            Entrance::Mcp,
            &reply_meta("m-slow"),
        )
        .expect("회신 발송");
        // The observation surface of closing = **the outstanding lookup** (the tracking item itself remains until cleaned up together with the history).
        assert!(
            svc.open_items_for(from.peer_id, Instant::now())
                .iter()
                .all(|i| i.id != "m-slow"),
            "회신이 계약을 닫아야 — 발신자의 미결에서 사라진다(옛 구멍에선 NoMatch 라 영원히 남았다)"
        );
        assert!(
            svc.open_items_for(w_id, Instant::now())
                .iter()
                .all(|i| i.id != "m-slow"),
            "회신자의 의무도 사라져야"
        );
        assert_eq!(
            svc.ledger_statuses("m-slow"),
            vec![DeliveryStatus::Replied],
            "이력도 `Replied` 로 닫혀야 — 프로브가 본 영구 `Delivered`(감사 기록의 거짓말)의 반증"
        );
    }

    #[test]
    fn a_reservation_dropped_while_another_thread_holds_the_lock_is_reclaimed_by_the_sweep() {
        let (svc, port) = svc();
        let (from, me) = live_sender("boss");
        port.set_roster(vec![me]);
        fill_open_request_cap_evictable(&svc, from);
        let occupied_before = svc.occupied_slots_for_test();
        let tracking_before = svc.tracking_len_for_test();

        let guard = reserve_marked_contract(&svc, from, "m-locked");

        // Another thread **holds the lock while waiting** → drop the guard in that window (reproduces production lock contention).
        let (locked_tx, locked_rx) = std::sync::mpsc::channel::<()>();
        let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
        let holder = {
            let svc2 = svc.clone();
            std::thread::spawn(move || {
                let _st = svc2.state.lock().expect("lock");
                locked_tx.send(()).expect("신호");
                release_rx.recv().expect("해제 신호");
            })
        };
        locked_rx.recv().expect("락 점유 확인");
        let hit = catch_quietly(|| drop(guard));
        if cfg!(debug_assertions) {
            assert!(hit.is_err(), "정산 누락은 debug 에서 red 여야(H1)");
        }
        release_tx.send(()).expect("해제");
        holder.join().expect("holder");

        assert!(
            svc.contract_tracked_for_test("m-locked", tid("ghost-worker")),
            "try_lock 이 실패했으니 Drop 은 아무 것도 못 했다 — 이 전제가 깨지면 아래 보증 단언이 무의미해진다"
        );
        assert_eq!(svc.marked_retirements_for_test(), 1, "표시도 남아 있다");

        svc.sweep(Instant::now() + Duration::from_secs(30));
        assert!(
            !svc.contract_tracked_for_test("m-locked", tid("ghost-worker")),
            "sweep 이 잠정 계약을 회수해야(F1 보증 계층)"
        );
        assert_eq!(
            svc.marked_retirements_for_test(),
            0,
            "희생자 표시도 풀려야 — 남으면 cap 분모가 영구히 준다"
        );
        assert_eq!(svc.occupied_slots_for_test(), occupied_before);
        assert_eq!(svc.tracking_len_for_test(), tracking_before);
    }

    #[test]
    fn a_reservation_dropped_while_unwinding_neither_aborts_nor_leaks() {
        // There is no assertion that abort did not happen — the very fact that this test runs to the end is the evidence.
        let (svc, port) = svc();
        let (from, me) = live_sender("boss");
        port.set_roster(vec![me]);
        fill_open_request_cap_evictable(&svc, from);
        let occupied_before = svc.occupied_slots_for_test();
        let tracking_before = svc.tracking_len_for_test();

        let hit = catch_quietly(|| {
            let _guard = reserve_marked_contract(&svc, from, "m-unwind");
            panic!("boom");
        });
        let payload = *hit
            .expect_err("패닉이 전파돼야")
            .downcast::<&str>()
            .expect("페이로드는 원래 패닉 그대로");
        assert_eq!(
            payload, "boom",
            "Drop 이 원래 패닉을 자기 패닉으로 갈아치우면 원인이 사라진다"
        );

        assert!(
            !svc.contract_tracked_for_test("m-unwind", tid("ghost-worker")),
            "언와인딩 중에도 롤백은 최선 노력으로 수행된다"
        );
        assert_eq!(svc.marked_retirements_for_test(), 0);
        assert_eq!(svc.occupied_slots_for_test(), occupied_before);
        assert_eq!(svc.tracking_len_for_test(), tracking_before);
    }

    #[test]
    fn a_parked_reply_still_closes_the_contract() {
        let (svc, port, gate) = svc_gated();
        let (boss_from, boss) = live_sender("boss");
        let boss_id = boss.id;
        let (w_id, worker) = live("worker");
        port.set_roster(vec![boss, worker]);

        svc.handle_send(
            "m-req",
            boss_from,
            "boss",
            &["worker".to_string()],
            "해줘",
            Entrance::Mcp,
            &req_meta("10m", 600),
        )
        .expect("배달");
        assert_eq!(svc.open_request_count(), 1);

        gate.set_busy(boss_id, 0);
        let rows_out = svc
            .handle_send(
                "m-rep",
                reply_from(w_id),
                "worker",
                &["boss".to_string()],
                "했음",
                Entrance::Mcp,
                &reply_meta("m-req"),
            )
            .expect("행 응답");
        assert_eq!(
            rows_out[0].status,
            SendStatus::Pending,
            "턴 중 요청자에게는 파킹된다: {rows_out:?}"
        );
        assert_eq!(
            svc.open_request_count(),
            0,
            "★파킹된 회신도 계약을 닫는다(M5)★ — 접수됐으므로 곧 배달된다"
        );
        svc.sweep(Instant::now() + Duration::from_secs(601));
        assert!(
            !svc.ledger_snapshot()
                .iter()
                .any(|(_, from, _, _)| from == NOTICE_SENDER_LABEL),
            "닫힌 계약엔 거짓 기한 통지가 없다"
        );
    }

    #[test]
    fn the_to_attribute_rules_hold_for_every_token_shape() {
        // ★Why white-box★: "the expansion of an `@` token is **all failures**" cannot be made black-box in v1 where `@all` is the only source —
        //   the key of `@all` is the live roster itself, so it is incompatible with the condition under which an envelope goes out (2 or more accepted).
        //   A future source (folder group) will use this rule soon, so the decision function is pinned directly.
        let tok_name = |row: usize| AddressToken::Name { row };
        let tok_group = |l: &str, rows: &[usize]| AddressToken::Group {
            label: l.to_string(),
            rows: rows.to_vec(),
        };
        // This function looks at acceptance only via `admitted`, so the resolution result carries only the name (an unresolved row = the notation).
        let rcpt = |k: &str| ResolvedRecipient {
            display: k.to_string(),
            to: Resolution::NotFound,
        };

        // ① Name token: carries acceptance and drops failure.
        let addr = Addressing {
            recipients: vec![rcpt("bob"), rcpt("ghost")],
            tokens: vec![tok_name(0), tok_name(1)],
        };
        assert_eq!(build_to_attr(&addr, &[true, false]), "bob");

        // ② M4: a notation duplicate is written **only once** (the position that appeared first is kept).
        let addr = Addressing {
            recipients: vec![rcpt("bob"), rcpt("carol")],
            tokens: vec![tok_name(0), tok_name(0), tok_name(1)],
        };
        assert_eq!(
            build_to_attr(&addr, &[true, true]),
            "bob,carol",
            "같은 값을 두 번 적으면 수신 LLM 이 인원수를 오독한다(M4)"
        );

        // ③ M6: the expansion of an `@` token is **all failures**.
        let addr = Addressing {
            recipients: vec![rcpt("carol"), rcpt("dup")],
            tokens: vec![tok_name(0), tok_group("@all", &[1])],
        };
        assert_eq!(
            build_to_attr(&addr, &[true, false]),
            "carol",
            "펼침이 전부 실패한 @토큰은 이 발송에 아무 것도 기여하지 않았다(M6)"
        );

        // ④ It stays if even one is accepted (A5 — including absorption).
        let addr = Addressing {
            recipients: vec![rcpt("bob"), rcpt("carol")],
            tokens: vec![tok_name(0), tok_name(1), tok_group("@all", &[0, 1])],
        };
        assert_eq!(
            build_to_attr(&addr, &[true, true]),
            "bob,carol,@all",
            "펼침이 앞선 명시 지목에 완전히 흡수돼도 실패는 아니다(A5)"
        );

        // ⑤ M4 (group axis): the same label is not written twice.
        let addr = Addressing {
            recipients: vec![rcpt("bob")],
            tokens: vec![tok_group("@all", &[0]), tok_group("@all", &[0])],
        };
        assert_eq!(build_to_attr(&addr, &[true]), "@all");

        // ⑥ Even when targeted by id, the registry name is written in the envelope (ADR-0309 decision 2 — the edge is names).
        let (_b, bob) = live("bob");
        let addr = Addressing {
            recipients: vec![
                ResolvedRecipient {
                    display: bob.id.to_string(),
                    to: Resolution::Live(bob),
                },
                rcpt("carol"),
            ],
            tokens: vec![tok_name(0), tok_name(1)],
        };
        assert_eq!(build_to_attr(&addr, &[true, true]), "bob,carol");
    }

    #[test]
    fn a_duplicate_name_token_is_written_once_in_the_envelope() {
        let (svc, port) = svc();
        let (from, me) = live_sender("alice");
        let (_b, bob) = live("bob");
        let (_c, carol) = live("carol");
        port.set_roster(vec![me, bob, carol]);
        let out = send(&svc, "m1", from, "alice", &["bob", "bob", "carol"]).expect("ok");
        assert_eq!(out.len(), 2, "표기 중복은 행을 만들지 않는다");
        assert!(
            port.injected_bodies()[0].contains(r#"to="bob,carol""#),
            "봉투 to 도 중복 없이: {:?}",
            port.injected_bodies()
        );
    }

    #[test]
    fn two_exact_ids_of_same_named_twins_both_get_a_row() {
        let (svc, port) = svc();
        let (from, me) = live_sender("alice");
        let (d1, dup1) = live_distinct("dup");
        let (d2, dup2) = live_distinct("dup");
        port.set_roster(vec![me, dup1, dup2]);

        // ① Two exact ids — the row key is the id so different entities do not contend for a row (ADR-0309 · TRD S29 §4 X2).
        let (id1, id2) = (d1.to_string(), d2.to_string());
        let out = send(&svc, "m1", from, "alice", &[&id1, &id2]).expect("ok");
        assert_eq!(
            rows(&out),
            vec![
                (id1.clone(), SendStatus::Delivered, None),
                (id2.clone(), SendStatus::Delivered, None),
            ],
            "토큰마다 자기 행 · 둘 다 배달: {out:?}"
        );
        assert_eq!(
            port.injected_targets(),
            vec![d1, d2],
            "쌍둥이 각자에게 한 통씩"
        );

        // ② A name + two exact ids (3 tokens) — the name row is ambiguous by the name resolution rule as is, and the id rows are each delivered.
        let (svc2, port2) = super::tests::svc();
        let (from2, me2) = live_sender("alice");
        let (e1, t1) = live_distinct("dup");
        let (e2, t2) = live_distinct("dup");
        port2.set_roster(vec![me2, t1, t2]);
        let out2 = send(
            &svc2,
            "m2",
            from2,
            "alice",
            &["dup", &e1.to_string(), &e2.to_string()],
        )
        .expect("ok");
        assert_eq!(
            rows(&out2),
            vec![
                (
                    "dup".to_string(),
                    SendStatus::Failed,
                    Some(FailCode::RecipientAmbiguous)
                ),
                (e1.to_string(), SendStatus::Delivered, None),
                (e2.to_string(), SendStatus::Delivered, None),
            ],
            "{out2:?}"
        );
        assert_eq!(port2.injected_targets(), vec![e1, e2]);
    }

    #[test]
    fn a_repeated_identical_id_token_yields_exactly_one_row() {
        let (svc, port) = svc();
        let (from, me) = live_sender("alice");
        let (d1, dup1) = live_distinct("dup");
        let (d2, dup2) = live_distinct("dup");
        port.set_roster(vec![me, dup1, dup2]);
        let (id1, id2) = (d1.to_string(), d2.to_string());

        let out = send(&svc, "m1", from, "alice", &[&id1, &id2, &id2, &id1, &id2]).expect("ok");
        assert_eq!(
            rows(&out),
            vec![
                (id1.clone(), SendStatus::Delivered, None),
                (id2.clone(), SendStatus::Delivered, None),
            ],
            "반복 토큰은 접히고 행은 쌍둥이마다 하나: {out:?}"
        );
        // Ledger transitions are found by `(msg_id, 받는 이 id)` (msg_id, receiving id) — if two rows used one key, one would remain in pending.
        assert_eq!(
            svc.ledger_statuses("m1"),
            vec![DeliveryStatus::Delivered, DeliveryStatus::Delivered]
        );
        assert_eq!(
            port.injected_targets(),
            vec![d1, d2],
            "배달은 쌍둥이마다 한 통"
        );
    }

    /// ★A repeat of an unresolved token also folds into one row★ — when the row key has no id it is the notation (`same_row`), so a not-found ·
    ///   ambiguous token with the same notation leaves just one failure row and one ledger row.
    #[test]
    fn a_repeated_unresolved_token_yields_exactly_one_failed_row() {
        let (svc, port) = svc();
        let (from, me) = live_sender("alice");
        port.set_roster(vec![me]);
        let out = send(&svc, "m1", from, "alice", &["ghost", "ghost"]).expect("ok");
        assert_eq!(
            rows(&out),
            vec![(
                "ghost".to_string(),
                SendStatus::Failed,
                Some(FailCode::RecipientNotFound)
            )],
            "없는 이름의 반복은 실패 행 하나: {out:?}"
        );
        assert_eq!(svc.ledger_statuses("m1"), vec![DeliveryStatus::Failed]);

        let (svc2, port2) = super::tests::svc();
        let (from2, me2) = live_sender("alice");
        let (_d1, dup1) = live_distinct("dup");
        let (_d2, dup2) = live_distinct("dup");
        port2.set_roster(vec![me2, dup1, dup2]);
        let out2 = send(&svc2, "m2", from2, "alice", &["dup", "dup"]).expect("ok");
        assert_eq!(
            rows(&out2),
            vec![(
                "dup".to_string(),
                SendStatus::Failed,
                Some(FailCode::RecipientAmbiguous)
            )],
            "모호한 이름의 반복은 실패 행 하나: {out2:?}"
        );
        assert_eq!(svc2.ledger_statuses("m2"), vec![DeliveryStatus::Failed]);
        assert!(
            port2.injected_targets().is_empty(),
            "아무에게도 주입하지 않는다"
        );
    }

    /// ★The id first, then the name (the same order as the control entrance `resolve_in`)★ — a UUID-shaped name cannot hijack
    ///   someone else's id target, and if it is an id shape not in the registry it resolves as that name.
    #[test]
    fn a_uuid_shaped_token_is_read_as_an_id_first_and_as_a_name_otherwise() {
        let (svc, port) = svc();
        let (from, me) = live_sender("alice");
        let (bob_id, bob) = live("bob");
        let (_i, mut impostor) = live_distinct("placeholder");
        impostor.name = bob_id.to_string();
        let free_uuid = PeerId::new_v4().to_string();
        let (named_id, mut named) = live_distinct("placeholder");
        named.name = free_uuid.clone();
        port.set_roster(vec![me, bob, impostor, named]);

        let bob_token = bob_id.to_string();
        let out = send(&svc, "m1", from, "alice", &[&bob_token, &free_uuid]).expect("ok");
        assert_eq!(
            rows(&out),
            vec![
                (bob_token.clone(), SendStatus::Delivered, None),
                (free_uuid.clone(), SendStatus::Delivered, None),
            ],
            "{out:?}"
        );
        assert_eq!(
            port.injected_targets(),
            vec![bob_id, named_id],
            "bob 의 id 는 bob 에게(이름이 같은 impostor 가 아니라) · 명부에 없는 UUID 는 그 이름의 주인에게"
        );
    }

    #[test]
    fn a_cap_saturated_request_fails_that_row_and_delivers_nothing_to_it() {
        let (svc, port) = svc();
        let (from, me) = live_sender("boss");
        let (_w, worker) = live("worker");
        port.set_roster(vec![me, worker]);
        fill_open_request_cap(&svc, from);
        let injected_before = port.injected_bodies().len();

        let out = svc
            .handle_send(
                "m-over",
                from,
                "boss",
                &["worker".to_string()],
                "해줘",
                Entrance::Mcp,
                &req_meta("10m", 600),
            )
            .expect("행 응답(전체 반려 아님 — 행 실패)");
        assert_eq!(
            rows(&out),
            vec![(
                "worker".to_string(),
                SendStatus::Failed,
                Some(FailCode::RequestCapacity)
            )]
        );
        assert_eq!(
            port.injected_bodies().len(),
            injected_before,
            "계약을 못 연 수신자에겐 **배달도 하지 않는다**(추적 없는 request 금지)"
        );
        assert_eq!(
            svc.ledger_statuses("m-over"),
            vec![DeliveryStatus::Failed],
            "장부에 delivered 가 아니라 종점 failed 만 남는다"
        );
        assert_eq!(svc.parked_len(tid("worker")), 0, "파킹도 없다");
    }

    #[test]
    fn the_to_attribute_keeps_a_token_whose_expansion_was_fully_absorbed_by_explicit_names() {
        let (svc, port) = svc();
        let (from, me) = live_sender("alice");
        let (_b, bob) = live("bob");
        let (_c, carol) = live("carol");
        port.set_roster(vec![me, bob, carol]);

        let out = send(&svc, "m1", from, "alice", &["bob", "carol", "@all"]).expect("ok");
        assert_eq!(out.len(), 2, "행은 흡수돼 2개");
        assert!(
            port.injected_bodies()[0].contains(r#"to="bob,carol,@all""#),
            "흡수된 @all 토큰도 입력 표기 순으로 남아야: {:?}",
            port.injected_bodies()
        );
    }

    #[test]
    fn an_exact_id_token_and_an_ambiguous_name_in_the_same_to_list_get_separate_rows() {
        // ★A name row is not absorbed into an id row (ADR-0309 · TRD S29 §4 X2)★ — an unresolved name does not point at any twin,
        //   so it is not the same row as a row resolved by id. Even if the order is reversed the outcome is the same.
        for order in 0..2 {
            let (svc, port) = svc();
            let (from, me) = live_sender("alice");
            let (d1, dup1) = live_distinct("dup");
            let (_d2, dup2) = live_distinct("dup");
            port.set_roster(vec![me, dup1, dup2]);
            let id = d1.to_string();
            let list: Vec<&str> = if order == 0 {
                vec!["dup", id.as_str()]
            } else {
                vec![id.as_str(), "dup"]
            };
            let out = send(&svc, "m1", from, "alice", &list).expect("ok");
            let ambiguous = (
                "dup".to_string(),
                SendStatus::Failed,
                Some(FailCode::RecipientAmbiguous),
            );
            let delivered = (id.clone(), SendStatus::Delivered, None);
            let expected = if order == 0 {
                vec![ambiguous, delivered]
            } else {
                vec![delivered, ambiguous]
            };
            assert_eq!(rows(&out), expected, "order={order}");
            assert_eq!(port.injected_targets(), vec![d1], "order={order}");
        }
    }

    #[test]
    fn can_admits_prediction_always_matches_what_park_actually_does() {
        let (svc, port, gate) = svc_gated();
        let (from, me) = live_sender("alice");
        let (r_id, recv) = live("recv");
        port.set_roster(vec![me, recv]);
        gate.set_busy(r_id, 0);
        for i in 0..100 {
            assert_eq!(
                svc.can_admit_for_test(tid("recv")),
                true,
                "{i}번째 예측: 아직 여유"
            );
            send(&svc, &format!("p{i}"), from, "alice", &["recv"]).expect("파킹");
        }
        assert!(!svc.can_admit_for_test(tid("recv")), "cap 도달 예측");
        assert_eq!(
            send(&svc, "over", from, "alice", &["recv"]).expect("행")[0].code,
            Some(FailCode::MailboxFull),
            "실제 park 도 반려 — 예측과 일치"
        );

        gate.clear();
        let (svc_h, from_h) = (svc.clone(), from);
        let seen: Arc<StdMutex<Vec<(bool, bool)>>> = Arc::new(StdMutex::new(Vec::new()));
        let seen_h = seen.clone();
        port.set_on_inject(Arc::new(move |idx| {
            if idx != 0 {
                return;
            }
            let predicted = svc_h.can_admit_for_test(tid("recv"));
            let actual = svc_h
                .handle_send(
                    "during",
                    from_h,
                    "alice",
                    &["recv".to_string()],
                    "b",
                    Entrance::Mcp,
                    &SendMeta::default(),
                )
                .expect("행")[0]
                .code
                .is_none();
            seen_h.lock().unwrap().push((predicted, actual));
        }));
        svc.flush_for(r_id);
        let obs = seen.lock().unwrap().clone();
        assert_eq!(obs.len(), 1);
        assert_eq!(
            obs[0].0, obs[0].1,
            "in-flight 창에서도 예측 == 실제(분모가 갈리면 여기서 터진다)"
        );
    }

    // ══════════════════════════════════════════════════════════════════════════════════════════
    // S18.6 entrance 3 branches — asleep parking · immediate injection with no turn signal · delete cleanup (spec §7 · ADR-0116/0117/0118)
    // ══════════════════════════════════════════════════════════════════════════════════════════

    #[test]
    fn a_dormant_recipient_is_parked_without_a_wake_and_flushed_on_restore() {
        let (svc, port) = svc();
        let (from, me) = live_sender("alice");
        port.set_roster(vec![me]);
        port.set_dormant(&["sleepy"]);

        let out = send(&svc, "m-dorm", from, "alice", &["sleepy"]).expect("반려 아님");
        assert_eq!(
            (out[0].status, out[0].code),
            (SendStatus::Pending, None),
            "잠듦은 수용(파킹)이다 — 실패 행이 아니다: {out:?}"
        );
        assert!(
            out[0].hint.as_deref().unwrap_or("").contains("saved agent"),
            "힌트가 '저장된 에이전트라 복원 시 배달' 을 말해야: {:?}",
            out[0].hint
        );
        assert_eq!(
            svc.ledger_statuses("m-dorm"),
            vec![DeliveryStatus::Pending],
            "파킹 레코드가 장부에 남는다(조용한 유실 금지)"
        );
        assert_eq!(svc.parked_len(tid("sleepy")), 1, "그 id 큐에 파킹된다");
        assert!(
            port.injected_bodies().is_empty(),
            "잠든 상대에게 주입을 시도하지 않는다(wake 미발동)"
        );

        let (late_id, late) = live("sleepy");
        port.set_roster(vec![late]);
        port.set_dormant(&[]);
        svc.flush_for(late_id);
        assert_eq!(
            port.injected_bodies(),
            vec![r#"<message from="alice">body</message>"#.to_string()],
            "복원 시 flush 로 배달된다"
        );
        assert_eq!(
            svc.ledger_statuses("m-dorm"),
            vec![DeliveryStatus::Delivered]
        );
        assert_eq!(svc.parked_len(tid("sleepy")), 0);
    }

    #[test]
    fn at_here_never_expands_to_a_dormant_name_but_direct_naming_parks_it() {
        let (svc, port) = svc();
        let (from, me) = live_sender("alice");
        let (_b, bob) = live("bob");
        port.set_roster(vec![me, bob]);
        port.set_dormant(&["sleepy"]);

        let here = send(&svc, "m-here", from, "alice", &["@here"]).expect("반려 아님");
        assert_eq!(
            here.iter().map(|r| r.to.clone()).collect::<Vec<_>>(),
            vec!["bob".to_string()],
            "@here 펼침에 잠든 이름이 끼면 안 된다: {here:?}"
        );
        assert_eq!(
            svc.parked_len(tid("sleepy")),
            0,
            "@here 는 잠든 큐를 만들지 않는다"
        );

        let mixed = send(&svc, "m-mix", from, "alice", &["sleepy", "@here"]).expect("반려 아님");
        assert_eq!(
            rows(&mixed),
            vec![
                ("sleepy".to_string(), SendStatus::Pending, None),
                ("bob".to_string(), SendStatus::Delivered, None),
            ],
            "잠든 행은 pending, 산 행은 배달: {mixed:?}"
        );
    }

    #[test]
    fn at_all_expands_to_the_whole_roster_including_dormant_names() {
        let (svc, port) = svc();
        let (from, me) = live_sender("alice");
        let (_b, bob) = live("bob");
        port.set_roster(vec![me, bob]);
        port.set_dormant(&["sleepy"]);

        let all = send(&svc, "m-all", from, "alice", &["@all"]).expect("반려 아님");
        assert_eq!(
            rows(&all),
            vec![
                ("bob".to_string(), SendStatus::Delivered, None),
                ("sleepy".to_string(), SendStatus::Pending, None),
            ],
            "산 멤버는 배달, 잠든 멤버는 파킹(수용): {all:?}"
        );
        assert_eq!(
            svc.parked_len(tid("sleepy")),
            1,
            "잠든 몫은 그 id 큐에 쌓인다"
        );

        let (late_id, late) = live("sleepy");
        port.set_roster(vec![late]);
        port.set_dormant(&[]);
        svc.flush_for(late_id);
        assert_eq!(
            svc.ledger_statuses("m-all"),
            vec![DeliveryStatus::Delivered, DeliveryStatus::Delivered],
            "잠든 몫도 등장하면 배달된다"
        );
    }

    #[test]
    fn neither_broadcast_vocabulary_ever_reaches_the_sender() {
        // ★The sender is removed by id only (ADR-0309 · TRD S29 §4 X3)★ — a **different** agent that merely shares the sender's name (a live twin ·
        //   an asleep same-name profile) receives the broadcast.
        let (svc, port) = svc();
        let (from, me) = live_sender("alice");
        let (twin_id, twin) = live_distinct("alice");
        let (bob_id, bob) = live("bob");
        port.set_roster(vec![me, twin, bob]);
        let sleeper = PeerId::new_v4();
        port.set_dormant_agents(&[(sleeper, "alice"), (tid("sleepy"), "sleepy")]);

        let all = send(&svc, "m-all", from, "alice", &["@all"]).expect("반려 아님");
        assert_eq!(
            rows(&all),
            vec![
                ("alice".to_string(), SendStatus::Delivered, None),
                ("alice".to_string(), SendStatus::Pending, None),
                ("bob".to_string(), SendStatus::Delivered, None),
                ("sleepy".to_string(), SendStatus::Pending, None),
            ],
            "산 쌍둥이는 배달 · 잠든 동명은 그 id 로 파킹 · 발신자 행은 없다: {all:?}"
        );
        assert_eq!(svc.parked_len(sleeper), 1, "잠든 동명 몫은 그 id 큐에");

        let here = send(&svc, "m-here", from, "alice", &["@here"]).expect("반려 아님");
        assert_eq!(
            rows(&here),
            vec![
                ("alice".to_string(), SendStatus::Delivered, None),
                ("bob".to_string(), SendStatus::Delivered, None),
            ],
            "{here:?}"
        );

        assert_eq!(
            port.injected_targets(),
            vec![twin_id, bob_id, twin_id, bob_id],
            "발신자 stdin 에는 아무것도 쓰이지 않는다"
        );
        assert_eq!(
            svc.parked_len(from.peer_id),
            0,
            "발신자 id 앞으로 파킹도 생기지 않는다"
        );
    }

    #[test]
    fn a_dormant_request_opens_a_contract_whose_deadline_notice_still_fires() {
        let (svc, port) = svc();
        let (from, me) = live_sender("alice");
        port.set_roster(vec![me]);
        port.set_dormant(&["sleepy"]);

        let out = svc
            .handle_send(
                "m-req",
                from,
                "alice",
                &["sleepy".to_string()],
                "해줘",
                Entrance::Mcp,
                &req_meta("1m", 60),
            )
            .expect("반려 아님");
        assert_eq!(out[0].status, SendStatus::Pending);
        assert_eq!(svc.open_request_count(), 1, "잠듦 파킹도 계약을 연다");
        assert_eq!(
            svc.contract_outcome_for_test("m-req", tid("sleepy")),
            Some("awaiting_reply")
        );

        svc.sweep(Instant::now() + Duration::from_secs(61));
        assert!(
            svc.ledger_snapshot()
                .iter()
                .any(|(_, f, to, _)| f == NOTICE_SENDER_LABEL && to == "alice"),
            "잠듦 계약도 기한 통지가 나가야: {:?}",
            svc.ledger_snapshot()
        );
    }

    #[test]
    fn two_dormant_profiles_sharing_a_name_block_the_park_as_ambiguous() {
        let (svc, port) = svc();
        let (from, me) = live_sender("alice");
        port.set_roster(vec![me]);
        let (t1, t2) = (PeerId::new_v4(), PeerId::new_v4());
        port.set_dormant_agents(&[(t1, "twin"), (t2, "twin")]);

        let out = send(&svc, "m1", from, "alice", &["twin"]).expect("행 응답");
        assert_eq!(
            (out[0].status, out[0].code),
            (SendStatus::Failed, Some(FailCode::RecipientAmbiguous)),
            "잠듦 층 동명은 파킹이 아니라 실패 행: {out:?}"
        );
        assert_eq!(svc.parked_total_for_test(), 0, "조용히 받는 경로가 없어야");
        assert_eq!(
            svc.ledger_statuses("m1"),
            vec![DeliveryStatus::Failed],
            "실패 행도 장부 종점으로 남는다"
        );

        // `@all` expands by id, not by name — it parks for each of the two profiles (ADR-0309 · TRD S29 §4 X1).
        let all = send(&svc, "m2", from, "alice", &["@all"]).expect("행 응답");
        assert_eq!(
            rows(&all),
            vec![
                ("twin".to_string(), SendStatus::Pending, None),
                ("twin".to_string(), SendStatus::Pending, None),
            ],
            "{all:?}"
        );
        assert_eq!((svc.parked_len(t1), svc.parked_len(t2)), (1, 1));
    }

    #[test]
    fn a_live_agent_wins_over_a_dormant_namesake() {
        let (svc, port) = svc();
        let (from, me) = live_sender("alice");
        let (_d, dup) = live("dup");
        port.set_roster(vec![me, dup]);
        // An asleep same-name is a **different** agent from the live dup.
        let sleeper = PeerId::new_v4();
        port.set_dormant_agents(&[(sleeper, "dup")]);

        let out = send(&svc, "m1", from, "alice", &["dup"]).expect("행 응답");
        assert_eq!(
            out[0].status,
            SendStatus::Delivered,
            "산 쪽이 이긴다: {out:?}"
        );
        assert_eq!(port.injected_bodies().len(), 1);
        assert_eq!(svc.parked_total_for_test(), 0);

        // That is only a rule of name targeting — `@all` expands by id so an asleep same-name also gets its share (ADR-0309 · TRD S29 §4 X1).
        let all = send(&svc, "m2", from, "alice", &["@all"]).expect("행 응답");
        assert_eq!(
            rows(&all),
            vec![
                ("dup".to_string(), SendStatus::Delivered, None),
                ("dup".to_string(), SendStatus::Pending, None),
            ],
            "{all:?}"
        );
        assert_eq!(svc.parked_len(sleeper), 1);
    }

    /// ★An asleep agent can also be called by its exact id (ADR-0309 decision 3 · TRD S29 §4 B6)★ — what stays in the ledger is the registry name,
    ///   not the notation (id).
    #[test]
    fn a_dormant_agent_addressed_by_its_exact_id_is_parked_and_delivered_on_wake() {
        let (svc, port) = svc();
        let (from, me) = live_sender("alice");
        port.set_roster(vec![me.clone()]);
        port.set_dormant(&["sleepy"]);
        let token = tid("sleepy").to_string();

        let out = send(&svc, "m1", from, "alice", &[&token]).expect("행 응답");
        assert_eq!(
            rows(&out),
            vec![(token.clone(), SendStatus::Pending, None)],
            "{out:?}"
        );
        assert_eq!(svc.parked_len(tid("sleepy")), 1);
        assert_eq!(
            svc.ledger_endpoint_keys("m1"),
            vec!["sleepy".to_string()],
            "장부의 받는 이 = 명부 이름"
        );

        let (late_id, late) = live("sleepy");
        port.set_roster(vec![me, late]);
        port.set_dormant(&[]);
        svc.flush_for(late_id);
        assert_eq!(svc.ledger_statuses("m1"), vec![DeliveryStatus::Delivered]);
        assert_eq!(port.injected_targets(), vec![late_id]);
    }

    /// ★The id spelling is read the same as the control entrance (`resolve_in`) (TRD S29 §4 B7)★ — uppercase · braces · no hyphens ·
    ///   `urn:uuid:` are ids too. Writing the same agent as a name and an id in a different spelling is one row.
    #[test]
    fn every_uuid_spelling_addresses_live_and_dormant_recipients() {
        let spellings = |id: PeerId| {
            vec![
                id.to_string().to_uppercase(),
                id.braced().to_string(),
                id.simple().to_string(),
                id.urn().to_string(),
            ]
        };
        let (svc, port) = svc();
        let (from, me) = live_sender("alice");
        let (bob_id, bob) = live("bob");
        port.set_roster(vec![me, bob]);
        port.set_dormant(&["sleepy"]);
        let sleepy_id = tid("sleepy");

        for (i, (live_token, dormant_token)) in spellings(bob_id)
            .into_iter()
            .zip(spellings(sleepy_id))
            .enumerate()
        {
            let out = send(
                &svc,
                &format!("m{i}"),
                from,
                "alice",
                &[&live_token, &dormant_token, "bob"],
            )
            .expect("행 응답");
            assert_eq!(
                rows(&out),
                vec![
                    (live_token.clone(), SendStatus::Delivered, None),
                    (dormant_token.clone(), SendStatus::Pending, None),
                ],
                "{out:?}"
            );
        }
        assert_eq!(port.injected_targets(), vec![bob_id; 4]);
        assert_eq!(svc.parked_len(sleepy_id), 4);
    }

    #[test]
    fn a_live_agent_without_a_turn_signal_is_injected_with_no_gate() {
        let (svc, port, gate) = svc_gated();
        let (from, me) = live_sender("alice");
        let (tui_id, tui) = live_no_turn_signal("tui");
        port.set_roster(vec![me.clone(), tui]);
        gate.set_busy(tui_id, 0); // This value must not be visible to the class with no turn signal.

        let out = svc
            .handle_send(
                "m1",
                from,
                "alice",
                &["tui".to_string()],
                "해줘",
                Entrance::Mcp,
                &req_meta("10m", 600),
            )
            .expect("행 응답");
        assert_eq!(
            (out[0].status, out[0].code),
            (SendStatus::Delivered, None),
            "턴 신호가 없어도 배달 대상이다(게이트 없이 즉시 주입): {out:?}"
        );
        assert_eq!(port.injected_bodies().len(), 1, "실제로 주입됐다");
        assert_eq!(svc.parked_len(tid("tui")), 0, "busy 파킹이 없는 부류다");
        assert_eq!(
            svc.ledger_statuses("m1"),
            vec![DeliveryStatus::Delivered],
            "delivered = 실제 주입 시점"
        );
        assert_eq!(
            svc.contract_outcome_for_test("m1", tid("tui")),
            Some("awaiting_reply"),
            "배달됐으므로 계약도 정상 오픈"
        );

        let second = send(&svc, "m2", from, "alice", &["tui"]).expect("행 응답");
        assert_eq!(second[0].status, SendStatus::Delivered);
        assert_eq!(port.injected_bodies().len(), 2, "둘 다 주입된다");

        let all = send(&svc, "m3", from, "alice", &["@all"]).expect("행 응답");
        assert_eq!(
            all.iter().map(|r| r.to.clone()).collect::<Vec<_>>(),
            vec!["tui".to_string()],
            "@all 은 턴 신호 없는 산 세션도 펼친다: {all:?}"
        );

        port.set_roster(vec![me]);
        let gone = send(&svc, "m4", from, "alice", &["tui"]).expect("행 응답");
        assert_eq!(
            gone[0].code,
            Some(FailCode::RecipientNotFound),
            "로스터·프로필에 없으면 그때는 없음 코드다: {gone:?}"
        );
    }

    #[test]
    fn the_ambiguous_hint_escalates_to_the_user_instead_of_teaching_exact_ids() {
        let (svc, port) = svc();
        let (from, me) = live_sender("alice");
        let (_a, dup1) = live_distinct("dup");
        let (_b, dup2) = live_distinct("dup");
        port.set_roster(vec![me, dup1, dup2]);
        port.set_dormant_agents(&[(PeerId::new_v4(), "twin"), (PeerId::new_v4(), "twin")]);

        let live_row = send(&svc, "m1", from, "alice", &["dup"]).expect("행")[0]
            .hint
            .clone()
            .unwrap_or_default();
        let dormant_row = send(&svc, "m2", from, "alice", &["twin"]).expect("행")[0]
            .hint
            .clone()
            .unwrap_or_default();
        for (layer, hint) in [("live", &live_row), ("dormant", &dormant_row)] {
            assert!(
                hint.contains("not supported")
                    && hint.contains("do NOT resend")
                    && hint.contains("tell the user"),
                "{layer} 힌트가 사용자 에스컬레이션을 말해야: {hint}"
            );
            assert!(
                !hint.contains("exact agent id") && !hint.contains("exact id"),
                "{layer} 힌트에 exact-id 재발송 안내가 남아 있다: {hint}"
            );
        }
    }

    #[test]
    fn a_reply_to_a_dormant_requester_is_accepted_and_stays_replied_after_the_deletion() {
        let (svc, port) = svc();
        let (boss_from, boss) = live_sender("boss");
        let (w_id, worker) = live("worker");
        port.set_roster(vec![boss, worker.clone()]);

        svc.handle_send(
            "m-req",
            boss_from,
            "boss",
            &["worker".to_string()],
            "해줘",
            Entrance::Mcp,
            &req_meta("10m", 600),
        )
        .expect("배달");

        port.set_roster(vec![worker.clone()]);
        port.set_dormant(&["boss"]);
        let worker_from = SenderIdentity {
            peer_id: w_id,
            epoch: 0,
        };
        let rep = svc
            .handle_send(
                "m-rep",
                worker_from,
                "worker",
                &["boss".to_string()],
                "했음",
                Entrance::Mcp,
                &reply_meta("m-req"),
            )
            .expect("행 응답");
        assert_eq!(
            rep[0].status,
            SendStatus::Pending,
            "잠든 요청자에게 회신은 파킹된다: {rep:?}"
        );
        assert_eq!(
            svc.contract_outcome_for_test("m-req", tid("worker")),
            Some("replied"),
            "파킹도 수용이다 — 꽂으면 계약 완료(ADR-0116 결정 2)"
        );

        port.set_dormant(&[]);
        let cleanup = svc.handle_profile_deleted(boss_from.peer_id, "boss");
        assert_eq!(cleanup.failed_parked, 1, "파킹된 회신이 종결된다");
        assert_eq!(
            svc.ledger_statuses("m-rep"),
            vec![DeliveryStatus::Failed],
            "배달 축은 사실대로 실패를 적는다"
        );
        assert_eq!(
            svc.contract_outcome_for_test("m-req", tid("worker")),
            Some("replied"),
            "★되돌리지 않는다★ — 계약은 replied 로 남는다(ADR-0116 결정 2 · 종점 되돌림 금지)"
        );
    }

    #[test]
    fn deleting_a_profile_fails_its_parked_mail_and_the_contracts_it_was_waiting_on() {
        let (svc, port) = svc();
        let (gone_from, _gone_live) = live_sender("gone");
        let (w_id, worker) = live("worker");
        port.set_roster(vec![worker.clone()]);
        port.set_dormant(&["gone"]);
        let worker_from = SenderIdentity {
            peer_id: w_id,
            epoch: 0,
        };

        svc.handle_send(
            "m-out",
            gone_from,
            "gone",
            &["worker".to_string()],
            "해줘",
            Entrance::Mcp,
            &req_meta("10m", 600),
        )
        .expect("배달");
        svc.handle_send(
            "m-in",
            worker_from,
            "worker",
            &["gone".to_string()],
            "해줘",
            Entrance::Mcp,
            &req_meta("10m", 600),
        )
        .expect("반려 아님");
        assert_eq!(svc.parked_len(tid("gone")), 1);

        port.set_dormant(&[]);
        let out = svc.handle_profile_deleted(gone_from.peer_id, "gone");
        assert!(!out.skipped_live);
        assert_eq!(out.failed_parked, 1, "파킹분 종결");
        assert_eq!(out.failed_contracts, 1, "요청자 쪽 계약만 종결");
        assert_eq!(svc.parked_len(tid("gone")), 0, "대기열에서 사라진다");

        assert_eq!(svc.ledger_statuses("m-in"), vec![DeliveryStatus::Failed]);
        let view = svc
            .message_state("m-in", Instant::now())
            .expect("조회 가능");
        assert_eq!(view.rows[0].code, Some("RECIPIENT_DELETED"));
        assert!(
            view.rows[0]
                .hint
                .as_deref()
                .unwrap_or("")
                .contains("do not resend automatically"),
            "조회 힌트가 다음 행동을 말해야: {:?}",
            view.rows[0].hint
        );

        assert_eq!(
            svc.contract_outcome_for_test("m-out", tid("worker")),
            Some("reply_failed"),
            "그 이름이 요청자인 계약은 실패 종결"
        );
        assert_eq!(
            svc.contract_outcome_for_test("m-in", tid("gone")),
            Some("awaiting_reply"),
            "회신자 쪽이 삭제된 계약은 유지(기한 통지 경로 존속)"
        );
    }

    #[test]
    fn deleting_a_profile_whose_session_is_still_live_cleans_nothing() {
        let (svc, port, gate) = svc_gated();
        let (from, me) = live_sender("alice");
        let (r_id, recv) = live("recv");
        port.set_roster(vec![me, recv]);
        gate.set_busy(r_id, 0);
        svc.handle_send(
            "m-req",
            from,
            "alice",
            &["recv".to_string()],
            "해줘",
            Entrance::Mcp,
            &req_meta("10m", 600),
        )
        .expect("파킹");
        assert_eq!(svc.parked_len(tid("recv")), 1);

        port.set_roster(vec![
            live_sender("alice").1,
            LiveAgent {
                id: r_id,
                name: "recv-after-delete".to_string(),
                epoch: 0,
                turn_signal: true,
            },
        ]);
        port.set_live_ids(&[r_id]);
        let out = svc.handle_profile_deleted(r_id, "recv");
        assert!(out.skipped_live, "그 프로필의 세션이 살아 있으면 건너뛴다");
        assert_eq!(out.failed_parked, 0);
        assert_eq!(svc.parked_len(tid("recv")), 1, "파킹분 보존");
        assert_eq!(
            svc.contract_outcome_for_test("m-req", tid("recv")),
            Some("awaiting_reply"),
            "계약 오픈 유지"
        );

        gate.clear();
        svc.flush_for(r_id);
        assert_eq!(
            svc.ledger_statuses("m-req"),
            vec![DeliveryStatus::Delivered]
        );
    }

    #[test]
    fn deleting_a_profile_whose_live_session_has_no_turn_signal_cleans_nothing() {
        let (svc, port) = svc();
        let (from, me) = live_sender("alice");
        let (tui_id, tui) = live_no_turn_signal("tui");
        port.set_roster(vec![me, tui]);
        port.fail_at(&[0]);

        let out = svc
            .handle_send(
                "m-req",
                from,
                "alice",
                &["tui".to_string()],
                "해줘",
                Entrance::Mcp,
                &req_meta("10m", 600),
            )
            .expect("행 응답");
        assert_eq!(
            out[0].status,
            SendStatus::Pending,
            "주입 실패는 파킹으로 흡수된다(조용한 유실 금지): {out:?}"
        );
        assert_eq!(svc.parked_len(tid("tui")), 1);

        port.set_live_ids(&[tui_id]);
        let cleaned = svc.handle_profile_deleted(tui_id, "tui");
        assert!(
            cleaned.skipped_live,
            "턴 신호가 없어도 산 세션이다 — 게이트가 놓치면 배달될 메일이 죽는다"
        );
        assert_eq!(svc.parked_len(tid("tui")), 1, "파킹분 보존");
        assert_eq!(
            svc.contract_outcome_for_test("m-req", tid("tui")),
            Some("awaiting_reply"),
            "계약 오픈 유지"
        );
    }

    /// ★Even with a live same-name, the deleted profile's share is cleaned up and the same-name's share is left as is (ADR-0309 decision 6 · TRD S29 B4)★ — the cleanup
    ///   sweeps by id, so the fact that the names are the same enters neither the decision nor the target selection. Reviving the name-axis gate makes
    ///   the cleanup skipped wholesale, and the deleted side's parked items · contracts remain until TTL.
    #[test]
    fn deleting_a_dormant_profile_cleans_its_share_even_with_a_live_namesake() {
        let (svc, port, gate) = svc_gated();
        let (alice_from, alice) = live_sender("alice");
        // The profile P to delete (name boss) — while awake it placed a request to alice and went asleep.
        let p = PeerId::new_v4();
        let (_, p_live) = live_with_id(p, "boss");
        port.set_roster(vec![alice.clone(), p_live]);
        svc.handle_send(
            "m-p-req",
            SenderIdentity {
                peer_id: p,
                epoch: 0,
            },
            "boss",
            &["alice".to_string()],
            "해줘",
            Entrance::Mcp,
            &req_meta("10m", 600),
        )
        .expect("배달");
        port.set_roster(vec![alice.clone()]);
        port.set_dormant_agents(&[(p, "boss")]);
        svc.handle_send(
            "m-to-p",
            alice_from,
            "alice",
            &["boss".to_string()],
            "잠든 P 에게",
            Entrance::Mcp,
            &SendMeta::default(),
        )
        .expect("파킹");
        assert_eq!(svc.parked_len(p), 1, "전제: 잠든 P 앞 파킹");

        // A different agent Q with the same name is up (the live layer beats the asleep layer) — Q also places a request, and parking
        //   piles up for the busy Q as well.
        let (q, q_live) = live_distinct("boss");
        port.set_roster(vec![alice.clone(), q_live]);
        svc.handle_send(
            "m-q-req",
            SenderIdentity {
                peer_id: q,
                epoch: 0,
            },
            "boss",
            &["alice".to_string()],
            "해줘",
            Entrance::Mcp,
            &req_meta("10m", 600),
        )
        .expect("배달");
        gate.set_busy(q, 0);
        svc.handle_send(
            "m-to-q",
            alice_from,
            "alice",
            &["boss".to_string()],
            "산 Q 에게",
            Entrance::Mcp,
            &SendMeta::default(),
        )
        .expect("파킹");
        assert_eq!(svc.parked_len(q), 1, "전제: 바쁜 Q 앞 파킹");

        // Delete P — the name passed in is the same as the live same-name's name.
        port.set_dormant_agents(&[]);
        port.set_live_ids(&[q]);
        let out = svc.handle_profile_deleted(p, "boss");

        assert!(!out.skipped_live, "산 동명은 정리를 막지 않는다: {out:?}");
        assert_eq!(out.failed_parked, 1, "P 앞 파킹분만 종결: {out:?}");
        assert_eq!(
            out.failed_contracts, 1,
            "P 가 요청자인 계약만 종결: {out:?}"
        );
        assert_eq!(svc.parked_len(p), 0);
        assert_eq!(svc.ledger_statuses("m-to-p"), vec![DeliveryStatus::Failed]);
        assert_eq!(
            svc.contract_outcome_for_test("m-p-req", tid("alice")),
            Some("reply_failed")
        );

        assert_eq!(svc.parked_len(q), 1, "산 동명의 파킹분은 그대로");
        assert_eq!(
            svc.ledger_statuses("m-to-q"),
            vec![DeliveryStatus::Pending],
            "★거짓 사유 금지★ — 삭제되지 않은 Q 의 메일이 RECIPIENT_DELETED 로 죽으면 안 된다"
        );
        assert_eq!(
            svc.contract_outcome_for_test("m-q-req", tid("alice")),
            Some("awaiting_reply"),
            "산 동명이 요청자인 오픈 계약은 유지된다(ADR-0118 결정 2)"
        );

        gate.clear();
        svc.flush_for(q);
        assert_eq!(
            svc.ledger_statuses("m-to-q"),
            vec![DeliveryStatus::Delivered]
        );
    }

    /// ★Even when a profile renamed while asleep is deleted, the share accumulated under the old name is swept by id, and the share of a live
    ///   agent that newly took that old name is not touched (ADR-0309 decision 6 · TRD S29 B4)★. The name passed in is the new name just before removal, so
    ///   with that name a contract opened under the old name cannot be found — if the branch that picks by name revives, X's contract remains,
    ///   and picking by the old name kills Y's share.
    #[test]
    fn deleting_a_renamed_dormant_profile_cleans_by_id_and_spares_the_new_holder_of_its_old_name() {
        let (svc, port, gate) = svc_gated();
        let (carol_from, carol) = live_sender("carol");
        // X (name a) — while awake it placed a request to carol and went asleep, and then parking piles up for the asleep X.
        let x = PeerId::new_v4();
        let (_, x_live) = live_with_id(x, "a");
        port.set_roster(vec![carol.clone(), x_live]);
        svc.handle_send(
            "m-x-req",
            SenderIdentity {
                peer_id: x,
                epoch: 0,
            },
            "a",
            &["carol".to_string()],
            "해줘",
            Entrance::Mcp,
            &req_meta("10m", 600),
        )
        .expect("배달");
        port.set_roster(vec![carol.clone()]);
        port.set_dormant_agents(&[(x, "a")]);
        svc.handle_send(
            "m-to-x",
            carol_from,
            "carol",
            &["a".to_string()],
            "잠든 X 에게",
            Entrance::Mcp,
            &SendMeta::default(),
        )
        .expect("파킹");
        assert_eq!(svc.parked_len(x), 1, "전제: 잠든 X 앞 파킹");

        // X renames to b while asleep, and a different agent Y is up as a — Y also places a request and parking piles up for the busy Y.
        let (y, y_live) = live_distinct("a");
        port.set_dormant_agents(&[(x, "b")]);
        port.set_roster(vec![carol.clone(), y_live]);
        svc.handle_send(
            "m-y-req",
            SenderIdentity {
                peer_id: y,
                epoch: 0,
            },
            "a",
            &["carol".to_string()],
            "해줘",
            Entrance::Mcp,
            &req_meta("10m", 600),
        )
        .expect("배달");
        gate.set_busy(y, 0);
        svc.handle_send(
            "m-to-y",
            carol_from,
            "carol",
            &["a".to_string()],
            "산 Y 에게",
            Entrance::Mcp,
            &SendMeta::default(),
        )
        .expect("파킹");
        assert_eq!(svc.parked_len(y), 1, "전제: 바쁜 Y 앞 파킹");

        // Delete X — the name passed in is the pre-removal name b.
        port.set_dormant_agents(&[]);
        port.set_live_ids(&[y]);
        let out = svc.handle_profile_deleted(x, "b");

        assert!(!out.skipped_live, "{out:?}");
        assert_eq!(
            out.failed_parked, 1,
            "옛 이름 앞으로 쌓인 X 의 파킹분: {out:?}"
        );
        assert_eq!(out.failed_contracts, 1, "옛 이름으로 연 X 의 계약: {out:?}");
        assert_eq!(svc.parked_len(x), 0);
        let view = svc
            .message_state("m-to-x", Instant::now())
            .expect("조회 가능");
        assert_eq!(view.rows[0].code, Some("RECIPIENT_DELETED"));
        assert_eq!(view.rows[0].to, "a", "행은 보낼 때 이름을 보인다(결정 4)");
        assert_eq!(
            svc.contract_outcome_for_test("m-x-req", tid("carol")),
            Some("reply_failed")
        );

        assert_eq!(
            svc.parked_len(y),
            1,
            "옛 이름의 새 주인 Y 의 파킹분은 그대로"
        );
        assert_eq!(svc.ledger_statuses("m-to-y"), vec![DeliveryStatus::Pending]);
        assert_eq!(
            svc.contract_outcome_for_test("m-y-req", tid("carol")),
            Some("awaiting_reply"),
            "Y 가 요청자인 오픈 계약은 유지된다"
        );

        gate.clear();
        svc.flush_for(y);
        assert_eq!(
            svc.ledger_statuses("m-to-y"),
            vec![DeliveryStatus::Delivered]
        );
    }

    #[test]
    fn a_reply_accepted_as_a_park_closes_the_contract_in_the_same_critical_section() {
        let (svc, port) = svc();
        let (boss_from, boss) = live_sender("boss");
        let (w_id, worker) = live("worker");
        port.set_roster(vec![boss, worker.clone()]);
        svc.handle_send(
            "m-req",
            boss_from,
            "boss",
            &["worker".to_string()],
            "해줘",
            Entrance::Mcp,
            &req_meta("10m", 600),
        )
        .expect("배달");

        port.set_roster(vec![worker]);
        port.set_dormant(&["boss"]);

        let in_lock = Arc::new(StdMutex::new(Vec::<Option<&'static str>>::new()));
        let seen = in_lock.clone();
        svc.set_accept_hook_for_test(Arc::new(move |ledger| {
            seen.lock()
                .unwrap()
                .push(ledger.contract_outcome_for_test("m-req", tid("worker")));
        }));

        let rep = svc
            .handle_send(
                "m-rep",
                SenderIdentity {
                    peer_id: w_id,
                    epoch: 0,
                },
                "worker",
                &["boss".to_string()],
                "했음",
                Entrance::Mcp,
                &reply_meta("m-req"),
            )
            .expect("행 응답");
        assert_eq!(rep[0].status, SendStatus::Pending, "회신이 파킹 수용됐다");
        assert_eq!(
            *in_lock.lock().unwrap(),
            vec![Some("replied")],
            "★수용을 기록한 그 락 안에서 이미 닫혀 있어야 한다★ — 여기가 `awaiting_reply` 면 닫기가 다른 락으로 빠졌다는 뜻이고, 그 창에 삭제 정리가 끼면 '수용됐는데 회신 실패' 가 된다(리뷰 fix D2)"
        );
        assert_eq!(
            svc.contract_outcome_for_test("m-req", tid("worker")),
            Some("replied"),
            "수용 = 완료 — 계약은 그 임계구역에서 닫혔다"
        );

        port.set_dormant(&[]);
        let cleaned = svc.handle_profile_deleted(boss_from.peer_id, "boss");
        assert_eq!(cleaned.failed_parked, 1, "파킹된 회신은 사실대로 실패 종결");
        assert_eq!(
            svc.contract_outcome_for_test("m-req", tid("worker")),
            Some("replied"),
            "계약 축은 되돌리지 않는다(ADR-0116 결정 2)"
        );
    }

    #[test]
    fn a_reply_that_goes_out_in_its_own_call_closes_the_contract_in_the_enqueue_lock() {
        let (svc, port) = svc();
        let (boss_from, boss) = live_sender("boss");
        let (w_id, worker) = live("worker");
        port.set_roster(vec![boss, worker]);
        svc.handle_send(
            "m-req",
            boss_from,
            "boss",
            &["worker".to_string()],
            "해줘",
            Entrance::Mcp,
            &req_meta("10m", 600),
        )
        .expect("배달");

        let in_lock = Arc::new(StdMutex::new(Vec::<Option<&'static str>>::new()));
        let seen = in_lock.clone();
        svc.set_accept_hook_for_test(Arc::new(move |ledger| {
            seen.lock()
                .unwrap()
                .push(ledger.contract_outcome_for_test("m-req", tid("worker")));
        }));

        let rep = svc
            .handle_send(
                "m-rep",
                SenderIdentity {
                    peer_id: w_id,
                    epoch: 0,
                },
                "worker",
                &["boss".to_string()],
                "했음",
                Entrance::Mcp,
                &reply_meta("m-req"),
            )
            .expect("행 응답");
        assert_eq!(
            rep[0].status,
            SendStatus::Delivered,
            "그 호출의 드레인이 냈으므로 delivered(경로는 적재-후-드레인)"
        );
        assert_eq!(
            *in_lock.lock().unwrap(),
            vec![Some("replied")],
            "★적재 락 안에서 이미 닫혀 있어야 한다★ — `awaiting_reply` 로 관측되면 닫기가 그 락 밖으로 빠졌다는 뜻이다(리뷰 fix D2 · ADR-0125로 확정 지점이 하나가 됐다)"
        );
    }

    /// ★A contract addressed to an asleep recipient holds that profile id from the moment it opens (ADR-0309 decision 1 · TRD S29 B5)★ — so even if it renames
    ///   while asleep and wakes with the new name, 「내가 답할 의무」 (my duty to answer) remains, and that reply closes the contract.
    #[test]
    fn a_dormant_contract_carries_the_profile_id_from_the_moment_it_opens() {
        let (svc, port) = svc();
        let (from, me) = live_sender("alice");
        port.set_roster(vec![me.clone()]);
        // An id not derived from the name — checks that the contract holds the entrance source's id as is.
        let sleepy_id = PeerId::new_v4();
        port.set_dormant_agents(&[(sleepy_id, "sleepy")]);
        svc.handle_send(
            "m-req",
            from,
            "alice",
            &["sleepy".to_string()],
            "해줘",
            Entrance::Mcp,
            &req_meta("10m", 600),
        )
        .expect("파킹");
        assert_eq!(
            svc.contract_outcome_for_test("m-req", sleepy_id),
            Some("awaiting_reply"),
            "잠든 수신자 앞 계약의 키 = 그 프로필 id"
        );

        let (_, renamed) = live_with_id(sleepy_id, "renamed");
        port.set_roster(vec![me, renamed]);
        port.set_dormant(&[]);
        let items = svc.open_items_for(sleepy_id, Instant::now());
        assert_eq!(
            items
                .iter()
                .map(|i| (i.direction.as_str(), i.id.as_str()))
                .collect::<Vec<_>>(),
            vec![("reply_owed_by_me", "m-req")],
            "개명해도 의무는 그 id 에 남는다: {items:?}"
        );
        let rep = svc
            .handle_send(
                "m-rep",
                reply_from(sleepy_id),
                "renamed",
                &["alice".to_string()],
                "했음",
                Entrance::Mcp,
                &reply_meta("m-req"),
            )
            .expect("행 응답");
        assert_eq!(rep[0].status, SendStatus::Delivered, "{rep:?}");
        assert_eq!(
            svc.contract_outcome_for_test("m-req", sleepy_id),
            Some("replied"),
            "개명 뒤 회신도 그 계약을 닫는다"
        );
    }

    #[test]
    fn a_namesake_cannot_close_a_dormant_contract_it_never_received() {
        let (svc, port) = svc();
        let (boss_from, boss) = live_sender("boss");
        port.set_roster(vec![boss.clone()]);
        port.set_dormant(&["twin"]);
        svc.handle_send(
            "m-req",
            boss_from,
            "boss",
            &["twin".to_string()],
            "해줘",
            Entrance::Mcp,
            &req_meta("10m", 600),
        )
        .expect("파킹");
        assert_eq!(
            svc.contract_outcome_for_test("m-req", tid("twin")),
            Some("awaiting_reply")
        );

        let (twin_a, a) = live_distinct("twin");
        let (_twin_b, b) = live_distinct("twin");
        port.set_roster(vec![boss.clone(), a, b]);
        port.set_dormant(&[]);
        let rep = svc
            .handle_send(
                "m-rep",
                SenderIdentity {
                    peer_id: twin_a,
                    epoch: 0,
                },
                "twin",
                &["boss".to_string()],
                "했음",
                Entrance::Mcp,
                &reply_meta("m-req"),
            )
            .expect("행 응답");
        assert_eq!(
            rep[0].status,
            SendStatus::Delivered,
            "회신 **배달**은 막지 않는다(계약 쪽만 무동작 — spec §3 항목 7-②): {rep:?}"
        );
        assert_eq!(
            svc.contract_outcome_for_test("m-req", tid("twin")),
            Some("awaiting_reply"),
            "받은 적 없는 동명은 계약을 닫지 못한다(이름 폴백 대조 없음 — ADR-0309 결정 7)"
        );

        // ★Even if only one agent has that name, a different agent cannot close it (ADR-0309 decision 5)★ — reviving the name fallback would close it here.
        let (other, other_agent) = live_distinct("twin");
        port.set_roster(vec![boss.clone(), other_agent]);
        svc.handle_send(
            "m-rep-other",
            reply_from(other),
            "twin",
            &["boss".to_string()],
            "했음",
            Entrance::Mcp,
            &reply_meta("m-req"),
        )
        .expect("행 응답");
        assert_eq!(
            svc.contract_outcome_for_test("m-req", tid("twin")),
            Some("awaiting_reply"),
            "같은 이름의 다른 에이전트는 의무의 주체가 아니다"
        );

        let (solo, solo_agent) = live("twin");
        port.set_roster(vec![boss, solo_agent]);
        svc.handle_send(
            "m-rep2",
            SenderIdentity {
                peer_id: solo,
                epoch: 0,
            },
            "twin",
            &["boss".to_string()],
            "했음",
            Entrance::Mcp,
            &reply_meta("m-req"),
        )
        .expect("행 응답");
        assert_eq!(
            svc.contract_outcome_for_test("m-req", tid("twin")),
            Some("replied"),
            "잠들어 있던 바로 그 에이전트(같은 id)가 깨어나 답하면 닫힌다"
        );
    }

    #[test]
    fn every_fail_code_has_a_frozen_wire_string() {
        // ★Mutation probe D9-d★: even **swapping the strings** of `FailCode::as_str` left every messaging test green
        //   (only 2 daemon tests caught it) — so it is sealed directly in the kernel.
        for (code, wire) in [
            (FailCode::RecipientNotFound, "RECIPIENT_NOT_FOUND"),
            (FailCode::RecipientAmbiguous, "RECIPIENT_AMBIGUOUS"),
            (FailCode::MailboxFull, "MAILBOX_FULL"),
            (FailCode::RequestCapacity, "REQUEST_CAPACITY"),
            (FailCode::RecipientDeleted, "RECIPIENT_DELETED"),
        ] {
            assert_eq!(code.as_str(), wire, "{code:?} 의 wire 문자열이 바뀌었다");
            // Lets the compiler catch a missing variant (when a new code comes, this match breaks).
            match code {
                FailCode::RecipientNotFound
                | FailCode::RecipientAmbiguous
                | FailCode::MailboxFull
                | FailCode::RequestCapacity
                | FailCode::RecipientDeleted => {}
            }
        }
    }
}
