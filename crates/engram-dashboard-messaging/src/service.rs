//! service — MessagingService: wires the pure structures (mailbox, ledger) into the send pipeline on tokio
//! (ADR-0103/0104, ADR-0111/0112/0114 send overhaul, ADR-0125 enqueue-then-drain).
//!
//! ★Role★: multi-recipient sending (spec §5), appearance/idle flush (ADR-0104), TTL sweep, the idle gate, and the reply contract (open the
//! ledger entry, close on reply, overdue notice). `handle_send` is the **one** send entry point — individual, multi-recipient and `@all` share it.
//!
//! ★Enqueue → drain (ADR-0125, spec §5)★: a send is enqueued at the queue tail without exception; within the same call that recipient's queue is
//! drained from the front (enqueue inside the lock, drain outside it).
//!
//! ★Per-recipient decision (spec §5, ADR-0116 decision 1)★ looks at the identity axis only; "can they receive now" is the drain's job. Rule:
//! send if alive, wait if it may come alive later, fail loudly if it does not exist.
//!   1. Entrance bounce → per-recipient failure row: in neither roster nor profiles → `RECIPIENT_NOT_FOUND`; a duplicated name in any layer →
//!      `RECIPIENT_AMBIGUOUS` (for a name spanning alive and asleep the alive side wins).
//!   2. Alive — enqueue, then the same-call drain decides the outcome. Capability only decides "when to put it in" (ADR-0116 decision 7).
//!   3. Asleep (profile exists, not in the roster) — enqueue and mark `pending`; it waits for the re-appearance flush. There is no wake (spec §8 v2).
//! Mailbox overflow in any branch is `failed` + `MAILBOX_FULL` for that recipient only (spec §5 partial progress).
//!
//! ★One decision-source snapshot per send (ADR-0111 decision 2, ADR-0116)★: two physical lookups (session snapshot + profile list); re-taking it
//! per recipient would bring back half-decisions. The profile-list race is bounded and accepted (spec §5); TTL and delete cleanup collect the residue.
//!
//! ★Ordering guarantee (ADR-0125, load-bearing)★: a recipient sees **enqueue order** — no seat reservation, no "is anything ahead of me" joining
//! decision (needing one again means the send path split in two). Accepted residue: the global order *between* recipients; closing it needs inject inside the lock.
//!
//! ★Single lock (ADR-0006 spirit)★: Mailbox+Ledger sit behind one `Mutex<MessagingState>`. ★Never call `DeliveryPort` (inject/roster) while holding it★ —
//! collect parking/inject targets under the lock, release, then call out; inject may take other locks (sessions RwLock), so otherwise lock-order inversion.
//!
//! ★Envelope = assembled at injection time (single wrap point, ADR-0096)★: parking stores the unwrapped body + sender name + `SendMeta` (`ParkPayload`);
//! `wrap_message`/`wrap_notice` (`envelope.rs`) build it at injection so a flush uses the *current* format (colon/xml switch); immediate and late delivery must match.
//!
//! ★Remote rows (ADR-0391, ADR-0393)★: a token resolved to an agent on another PC is recorded `RemotePending` (no contract here) and handed to
//! `RemoteDeliveryPort::enqueue` after the lock is released, one mail per PC; its outcome arrives later through `handle_remote_verdict`, the
//! only path out of `RemotePending`. The control entrance answers the agent through `await_remote_answer`, which seals the answer under the
//! lock so each row is reported exactly once — in the answer, or by a later notice. Inbound mail enters through `handle_remote_mail` and is
//! resolved against local agents only (`ResolveScope::LocalOnly` — no forwarding to a third PC).
//!
//! Workspace crate imports: 0 (ADR-0110 — enforced by the compiler).
// ADR-0103, ADR-0104, ADR-0111, ADR-0112, ADR-0114, ADR-0116, ADR-0118, ADR-0121, ADR-0125, ADR-0309
// ADR-0390, ADR-0391, ADR-0393

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Condvar, Mutex};
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
    DeliveryStatus, DueTimeout, Ledger, OpenOutcome, RemoteRowView, RemoteSend, RemoteSettleError,
    RemoteSettlement, ReplyFailOutcome, ReplyOutcome, ReservationLiveness, RetiredContract,
    TransitionError,
};
use super::mailbox::{FlightTicket, Mailbox, ParkError, ParkKind, ParkedMessage};
use crate::{NodeRef, PeerId, SenderIdentity};

/// Ledger sender label for a notice — a notice has no real sender but the ledger record requires `from`. ★Not an address★: nothing can be sent to it
/// and it is never rendered in an envelope (a notice envelope has no from attribute — ADR-0103).
const NOTICE_SENDER_LABEL: &str = "engram";

// ★Do not revive `STALE_RESERVATION_AFTER` (age-based reservation reclaim)★: no age threshold is safe — the owner's out-of-lock inject is a child stdin write
// that can block without bound, so "older than N s" misjudges a live owner and delivers a request without a contract (`ledger::ReservationLiveness`).
// Reclaim is by owner-guard liveness (`ledger::reclaim_abandoned_reservations`). ADR-0125 moved the reservation window inside the enqueue lock, but the
// reason stands: no threshold can stand in for owner liveness.
// ADR-0108, ADR-0125

/// ★Carries both the raw notation and the parsed value★: the envelope `reply-by` must show the sender's notation (`"10m"`, spec §1), while the ledger
/// needs a `Duration` for the absolute deadline (spec §3); carrying one forces re-rendering and the notation drifts (`60m` → `1h`).
/// ★`Default` = notification★: all fields inactive.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SendMeta {
    pub request: bool,
    /// The deadline notation as the sender wrote it (`"10m"`); the envelope `reply-by` value. Only meaningful for `request`.
    pub reply_by_raw: Option<String>,
    /// The parsed deadline — absolute deadline = send time + this. If raw is Some, this must be Some.
    pub reply_by: Option<Duration>,
    pub reply_to: Option<String>,
    /// ★The envelope `to` attribute (spec §1, ADR-0111)★: if `Some`, used as is — the canonical names of accepted explicit targets + `@`address tokens
    /// (not expanded), joined in input order (`"@all,carol"`).
    /// ★Exposed only when 2 or more recipients were accepted★ (counted at acceptance, not by `delivered` — that is fixed later at injection).
    /// ★Rides on `SendMeta`★ because a parked item must carry the same envelope when delivered late (assembled at injection, same reason as
    /// request/reply_to); a separate axis would make two flush paths.
    /// ★The entrance does not fill it★: `handle_send` fixes it once after every acceptance decision (debug_assert) so all envelopes of a send carry the same `to`.
    // ADR-0111
    pub to_attr: Option<String>,
}

impl SendMeta {
    /// The attributes this send puts on the envelope (spec §1 — only fields that change behavior). Send argument `reply_to` appears as `in-reply-to`.
    /// ★`id` only on a request★: an id on a notification/reply makes the receiving LLM wonder whether to reply (spec §1, ADR-0103 decision 1).
    /// `pub` so host entrance tests assert "validated arguments → envelope" without re-implementing the assembly rules.
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

/// The send result for one recipient (an element of spec §6 `results[]`; the entrance converts it to wire JSON). One recipient = one row — duplicates
/// were folded at resolution (spec §5 ④).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecipientResult {
    /// Recipient notation — the token the sender wrote (trimmed only, ADR-0101); an `@all` expansion yields the roster name.
    pub to: String,
    pub status: SendStatus,
    /// The failure code, required when `Failed` (spec §6). `None` otherwise.
    pub code: Option<FailCode>,
    /// Self-correction hint (parking reason · failure reason). `None` for `Delivered`.
    pub hint: Option<String>,
}

/// The outcome vocabulary for one recipient (spec §6 — `delivered|pending|failed`). `skipped` left this vocabulary (ADR-0111 decision 3); the ledger's
/// terminal `skipped` is a different axis and remains.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendStatus {
    /// Actually injected — ledger `delivered` (ADR-0104 actual injection time).
    Delivered,
    /// Parked — ledger `pending`.
    Pending,
    /// ★Failure for that recipient only (ADR-0111 decision 3)★ — the others are still delivered (partial progress).
    Failed,
}

/// Per-recipient failure code (spec §6 row code — a **different axis** from the send-level bounce code).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailCode {
    /// ★A name in neither the roster nor the profiles★ (ADR-0116 decision 1). An asleep session is `pending`, not this; a live peer with no structured output
    /// is injected immediately (decision 7). ★Do not revive `RecipientUnreachable`★ — no state for it to point at; needs a new decision first.
    // ADR-0116
    RecipientNotFound,
    /// Two or more agents share the name (ADR-0114 decision 4). ★Same code in any layer★: duplicate roster names and the asleep layer (two profiles
    /// deriving one name) alike (ADR-0116 decision 1). ADR-0115 (spawn name uniqueness) will make it a dead letter.
    RecipientAmbiguous,
    /// The recipient's message-lane mailbox (100 items) is full — immediately, no reclaim attempt (ADR-0114 decision 1).
    MailboxFull,
    /// The recipient's reply contract cannot be opened (open limit 512, ADR-0108). Nothing is delivered either — tracking replies is the point of a request (ADR-0114).
    RequestCapacity,
    /// ★The recipient profile was deleted while the item waited parked★ (ADR-0116 decision 3). ★Never appears in the send response★ (spec §6): at send time it was
    /// `pending`; delete cleanup stamps it afterward, so the sender first sees it in an `eg_messages{id}` lookup. Differs from `RecipientNotFound` in timing —
    /// the sender must tell "wrote the name wrong" from "waited and the peer was deleted" (different resend decision).
    // ADR-0116
    RecipientDeleted,
    /// ★The recipient's PC has no live link★ (also: the link dropped before the mail was handed to it) — nothing was sent; there is no outbox,
    ///   so the sender resends when the PC is back.
    // ADR-0390, ADR-0393
    PeerOffline,
    /// A limit on the way to another PC: the receiver's per-PC backstop or body re-check, or this PC's in-flight cap to that PC.
    // ADR-0390
    RemoteLimit,
    /// The other PC refused the mail (validation, permission, id collision) — also any verdict code outside the closed set.
    // ADR-0390
    RemoteRefused,
    /// ★The sending agent's own name breaks the remote name rule★ (`remote_name_ok`) — the receiver would refuse it, so nothing is sent.
    // ADR-0390
    NameTooLong,
    /// The encoded mail to that PC would exceed the link's frame cap — that PC's rows fail, the link stays up.
    // ADR-0390
    BodyTooLarge,
}

impl FailCode {
    const ALL: [FailCode; 10] = [
        FailCode::RecipientNotFound,
        FailCode::RecipientAmbiguous,
        FailCode::MailboxFull,
        FailCode::RequestCapacity,
        FailCode::RecipientDeleted,
        FailCode::PeerOffline,
        FailCode::RemoteLimit,
        FailCode::RemoteRefused,
        FailCode::NameTooLong,
        FailCode::BodyTooLarge,
    ];

    /// The wire code string (stable contract — the sending LLM branches on it, spec §6).
    pub fn as_str(self) -> &'static str {
        match self {
            FailCode::RecipientNotFound => "RECIPIENT_NOT_FOUND",
            FailCode::RecipientAmbiguous => "RECIPIENT_AMBIGUOUS",
            FailCode::MailboxFull => "MAILBOX_FULL",
            FailCode::RequestCapacity => "REQUEST_CAPACITY",
            FailCode::RecipientDeleted => "RECIPIENT_DELETED",
            FailCode::PeerOffline => "PEER_OFFLINE",
            FailCode::RemoteLimit => "REMOTE_LIMIT",
            FailCode::RemoteRefused => "REMOTE_REFUSED",
            FailCode::NameTooLong => "NAME_TOO_LONG",
            FailCode::BodyTooLarge => "BODY_TOO_LARGE",
        }
    }

    fn from_wire(code: &str) -> Option<FailCode> {
        Self::ALL.into_iter().find(|c| c.as_str() == code)
    }

    /// ★A refused row's code from another PC, mapped onto the closed set★ — anything else (or none) becomes `REMOTE_REFUSED`, so a peer
    ///   cannot make this daemon report a code it never defined for that case.
    // ADR-0390
    pub fn from_remote_code(code: Option<&str>) -> FailCode {
        match code.and_then(Self::from_wire) {
            Some(
                c @ (FailCode::RemoteLimit
                | FailCode::RecipientAmbiguous
                | FailCode::MailboxFull
                | FailCode::RequestCapacity
                | FailCode::RemoteRefused),
            ) => c,
            _ => FailCode::RemoteRefused,
        }
    }
}

/// ★Remote name rule★ — an agent name that may travel to another PC: non-empty, at most `REMOTE_NAME_MAX_BYTES`, no control characters.
///   Agent names have no local cap, so the sender checks its own name (`NAME_TOO_LONG`), the receiver checks the frame's, and the host's
/// directory publisher omits names that break it.
// ADR-0390
pub fn remote_name_ok(name: &str) -> bool {
    !name.is_empty() && name.len() <= REMOTE_NAME_MAX_BYTES && !name.chars().any(char::is_control)
}

pub const REMOTE_NAME_MAX_BYTES: usize = 256;

/// Rows per inbound mail (`InboundMail::rows`).
// ADR-0390
pub const MAX_REMOTE_ROWS: usize = 64;

/// Cap of a peer's refusal reason as this PC shows it.
// ADR-0390
pub const REMOTE_REASON_MAX_BYTES: usize = 512;

/// ★Which addresses a send may resolve to★ — `All` for this PC's own entrances; `LocalOnly` for mail that came from another PC, so the
///   listed PCs and directory copies are ignored and nothing is forwarded to a third PC (a name not here is `RECIPIENT_NOT_FOUND`).
///   A named argument because `send_pipeline` takes its own snapshot — the caller cannot hand it an emptied one.
// ADR-0390
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolveScope {
    All,
    LocalOnly,
}

/// ★One PC this daemon lists (trusted)★, as the host's peer module sees it at snapshot time.
// ADR-0391
#[derive(Debug, Clone)]
pub struct RemotePc {
    pub node: NodeRef,
    /// This PC's name for it — unique ignoring case, no `@`; matched by `name@pc`.
    pub name: String,
    pub link: PcLink,
    /// ★The read-only copy of its agent directory★ — `None` = unknown this run (no copy since this daemon started): such a PC is not
    ///   counted for bare names (write `name@pc` to reach it). `Some` (possibly empty) = known, connected or not.
    pub directory: Option<Vec<DirectoryEntry>>,
}

/// One row of a directory copy (live or asleep — resolution treats them alike; the PC resolves its own layers).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectoryEntry {
    pub id: PeerId,
    pub name: String,
}

/// The link state of a listed PC — anything but `Connected` makes its rows `PEER_OFFLINE` at once, with a hint naming the cause.
// ADR-0390
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PcLink {
    Connected,
    Offline,
    /// That PC refuses this PC's link — it does not list this PC's ID.
    Refused,
    VersionMismatch,
}

/// ★Host port for mail to and from other PCs (the daemon's peer module)★. ★Out-of-lock call contract★ like `DeliveryPort`.
// ADR-0390, ADR-0393
pub trait RemoteDeliveryPort: Send + Sync {
    /// ★Non-blocking★: hands one mail to one PC (its rows only) and returns. ★Verdict contract★: exactly one final
    ///   `MessagingService::handle_remote_verdict` call per mail — including "not sent" (`RemoteOutcome::NotSent`) and "no answer"
    /// (`RemoteOutcome::Unconfirmed`); tags that call leaves out are settled `Unconfirmed` by the kernel, and a mail with no call at all stays
    /// unconfirmed until its remote entry ages out (`ledger::REMOTE_VERDICT_HORIZON`). The host's transport deadline is
    /// `ledger::REMOTE_TRANSPORT_DEADLINE`; on the receiving side it drops an inbound mail past that deadline instead of running it late,
    /// counted from the mail's arrival on the receiving PC (the PCs share no clock; transit sits inside the overdue grace margin).
    fn enqueue(&self, mail: OutboundMail);
    /// Best-effort and ★non-blocking★ (called from the sweep task; a blocking adapter stalls the TTL / BUSY sweep): tells `node` that its
    /// request `request_id` is overdue here (lost if the link is down).
    fn send_reply_overdue(&self, node: NodeRef, request_id: &str);
    /// ★Is this id an agent of this PC★ — every profile and live session, live shells included (not the mail-eligible set): an inbound
    ///   sender id that is also local is refused (a copied data folder).
    fn is_local_agent_id(&self, id: PeerId) -> bool;
}

/// One mail to one PC — what `RemoteDeliveryPort::enqueue` receives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboundMail {
    pub id: String,
    pub node: NodeRef,
    pub from: SenderIdentity,
    pub sender_name: String,
    pub rows: Vec<OutboundRow>,
    pub body: String,
    /// `request` / `reply_by_raw` / `reply_to` as validated by the sender's entrance (`to_attr` is never set).
    pub meta: SendMeta,
}

/// ★`tag` = the row index of the send, frozen on the ledger row★ — the verdict is matched by it, never by the notation. `to` = the name or
///   id on that PC (`@pc` stripped).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboundRow {
    pub tag: u32,
    pub to: String,
}

/// One row's outcome reported by the host for a mail it was handed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteRowVerdict {
    pub tag: u32,
    pub outcome: RemoteOutcome,
}

// ADR-0393
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemoteOutcome {
    Delivered {
        recipient_id: PeerId,
    },
    /// Parked on that PC (recipient asleep or busy).
    Held {
        recipient_id: PeerId,
    },
    /// Refused by that PC; `code` is mapped onto the closed set (`FailCode::from_remote_code`), `reason` is capped.
    Refused {
        code: Option<String>,
        reason: Option<String>,
    },
    UnknownRecipient,
    /// ★Not handed to that PC★ — `PEER_OFFLINE` (no live link, or dropped before sending), `REMOTE_LIMIT` (in-flight cap), `BODY_TOO_LARGE`.
    NotSent(FailCode),
    /// ★No outcome will come★ — the transport deadline passed, or the link dropped after the mail was handed over (it may have arrived).
    Unconfirmed,
}

/// One inbound mail, decoded by the host (the sender's node is the link's, not a frame field).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InboundMail {
    pub id: String,
    pub from_agent_id: PeerId,
    /// The sending daemon's claim — the link authenticates the PC, not the agent name.
    pub from_name: String,
    pub rows: Vec<InboundRow>,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InboundRow {
    pub tag: u32,
    pub to: String,
}

/// ★Per-row result of the remote entrance★ — `tag` echoes the request row; two rows that resolve to one local agent (an id and a name)
///   share one delivery and the same outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteRowResult {
    pub tag: u32,
    pub outcome: InboundOutcome,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InboundOutcome {
    Delivered {
        recipient_id: PeerId,
    },
    Held {
        recipient_id: PeerId,
    },
    /// `RECIPIENT_AMBIGUOUS` · `MAILBOX_FULL` · `REQUEST_CAPACITY` (the receiver's own row codes).
    Refused(FailCode),
    UnknownRecipient,
}

/// ★Whole-frame refusal by the remote entrance★ — `code` is `REMOTE_REFUSED` (validation, a local sender id, id collision) or `REMOTE_LIMIT`
///   (backstop, body cap); `reason` is this daemon's text, at most `REMOTE_REASON_MAX_BYTES`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteRefusal {
    pub code: FailCode,
    pub reason: String,
}

/// ★Inbound limits, one value injected by the host★ (ready for a later settings system): the per-PC backstop against a broken peer daemon
///   and the body re-check. No per-agent or per-pair limit.
// ADR-0390
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RemoteLimits {
    pub mails_per_minute: usize,
    pub max_body_bytes: usize,
}

struct RemoteWiring {
    port: Arc<dyn RemoteDeliveryPort>,
    limits: RemoteLimits,
}

/// ★Send-level bounce (spec §5·§6 `{status:"error", code, hint}`)★ — only problems that cannot be split per recipient. ★Layering (ADR-0114 decision 3)★:
/// absence of a name = runtime state → per-recipient failure row; an `@`address error (nonexistent syntax, mistyped group) = something the sender must
/// fix → whole bounce. `INVALID_SEND_ARGS` and `BODY_TOO_LARGE` are bounced by the entrance before reaching the service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SendReject {
    /// An `@address` other than `@all` (or an `@` rule violation) → `GROUP_NOT_FOUND`; a whole bounce even with a mixed `to` (no half-success).
    GroupNotFound { name: String },
    /// The final recipient set (expansion ∪ explicit) is empty → `GROUP_EMPTY` (ADR-0114 decision 3); e.g. `["@all"]` with the sender as sole survivor.
    GroupEmpty,
    /// ★The message id already exists in the ledger★ (history or request tracking; practically impossible, 36^8 space). ★No-side-effect guarantee★: the id check
    /// is the first side-effect point, so on this value nothing was delivered, parked or recorded — the caller can retry with a new id.
    IdCollision,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ProfileDeletedOutcome {
    /// ★Condition not met★ — that profile id's session is still alive, so nothing was done; every other field is 0.
    pub skipped_live: bool,
    /// Parked items closed as `failed` + `RECIPIENT_DELETED` (any lane, including notice).
    pub failed_parked: usize,
    /// Open contracts closed as `reply_failed` (only those where that profile id is the requester).
    pub failed_contracts: usize,
    /// Contracts skipped because of the guard (provisional, slated for retirement).
    pub guard_held_contracts: usize,
}

/// ★Injection receipt (ADR-0110 decision 2)★ — the 4-field value `DeliveryPort` returns to prove the envelope bytes were written; pure scalars so this
/// crate need not know the host type. `msg_uuid` = replay-dedup key the recipient session assigned to this user turn; `epoch` = the incarnation that
/// **executed** the write (not a resolution-time snapshot, so a mid-flight epoch race is visible from the record). ★`bytes_written` is not a short-write
/// detector★: on success it equals `bytes_requested`; completeness is Ok/Err (`DeliveryObservation::is_delivered`). ADR-0088 correlation.
// ADR-0110
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InjectReceipt {
    /// Byte count of the logical message the injection requested (envelope string length).
    pub bytes_requested: usize,
    pub bytes_written: usize,
    pub msg_uuid: uuid::Uuid,
    pub epoch: u32,
}

/// ★Control-plane port (ADR-0110 decision 3; the port lives in the lib, the adapter in the host)★ — the two things the service asks the host control
/// plane: ① the envelope format to use now ② recording a delivery observation. ★Fail-open★: both are optional — the format has a default (Xml) and
/// observation is a no-op in the production daemon; delivery goes through even if the implementation does nothing. ★Out-of-lock call contract★: same as
/// `DeliveryPort` — called only with the messaging lock released.
// ADR-0110
pub trait ControlPlanePort: Send + Sync {
    /// The envelope format to assemble with now (host global state, switchable at runtime, ADR-0096).
    fn envelope_format(&self) -> EnvelopeFormat;
    /// Records one delivery-boundary observation to the host sink (ADR-0088).
    fn record_delivery(&self, obs: DeliveryObservation);
}

/// The delivery-boundary seam (ADR-0012) — the service crosses this instead of calling the host agent directly, so headless tests verify branches, flush and
/// sweep deterministically (FakeDeliveryPort); production = host-owned `messaging_host::ManagerDeliveryPort` (ADR-0110 decision 3).
/// ★Out-of-lock call contract★: methods are called only with the messaging lock **released** (module header), so implementations may take other locks.
pub trait DeliveryPort: Send + Sync {
    /// Injects the finished envelope bytes into the recipient (a write to its stdin). `Err` = unreachable or write failure; the upper layer parks.
    /// ★Contract = the recipient receives this and starts processing it★: what submits it (some recipients buffer input and wait for a submit signal) is
    /// host-adapter knowledge — branching on recipient kind here would make the kernel know the host (ADR-0110). ★Do not append a submit character to the
    /// envelope★ (content vs channel act).
    /// ★Unconditional-incarnation injection★: it writes to whatever session that PeerId points at now, even after an epoch bump — correct, since the address is
    /// the name and re-spawn takeover is a feature (ADR-0101). ★Do not revive `inject_if_epoch`★ — the send-time binding was abolished (ADR-0111 decision 6);
    /// reopening it means a v2 personal-mail option (spec §8).
    // ADR-0111
    fn inject(&self, to_id: PeerId, bytes: &[u8]) -> Result<InjectReceipt, String>;

    /// ★Roster = every agent that currently has a process attached (`Running`|`Exiting`)★ — a snapshot of (name, id, epoch, turn_signal), shared by resolve,
    /// flush (appearance/epoch-change detection), `@`address expansion and the entrance decision.
    /// ★Turn observability is not an eligibility condition (ADR-0116 decisions 1·7)★: terminal/console-mode sessions stay in. ★One more axis — what the input is
    /// interpreted as★: the host removes the class that executes letters without reading them (shells). ★Do not merge the two and revert to "list = every live
    /// session"★ — a shell would execute the envelope as a command.
    /// ★Not "is in the session list"★: a session stays in the map until the reaper collects it, so existence alone misclassifies a just-exited agent as a
    /// delivery target instead of asleep; the real-adapter sealing test is canonical (mutation measurement found it undefended).
    // ADR-0116
    fn live_agents(&self) -> Vec<LiveAgent>;

    /// ★Entrance decision source: a one-shot snapshot★ (spec §5, ADR-0116): called exactly once per send. ★One verb, not two lookups★: splitting lets the list
    /// change between them and each recipient see a different world (ADR-0111 decision 2); one verb makes the adapter guarantee "1 session snapshot + 1 profile"
    /// structurally. ★Adapter contract (breaking it misclassifies silently)★:
    ///   - `roster` = the same predicate as `live_agents()` (`Running|Exiting` + readers of letters; no capability condition), from the same single
    ///     `list_agents()` snapshot — taking it twice revives TOCTOU and per-session locks.
    ///   - `dormant` = (id, canonical name) of profiles whose session is not alive; id = profile id (= session id when relaunched); name derived by the same rule
    ///     as a live session or a name-addressed recipient cannot be found. Duplicates are not folded (two of one name stay two).
    // ADR-0116, ADR-0309
    fn addressing_sources(&self) -> AddressingSources;

    /// ★Is this id's session alive right now★ (`Running`|`Exiting`, the roster's state predicate; mail eligibility not checked). Exists for the delete-cleanup
    /// gate (spec §5, ADR-0116 decision 3): id axis for rename immunity — rationale canonical in the `handle_profile_deleted` doc; profile id = session id.
    /// ★Not `addressing_sources().roster`★ (send-purpose snapshot that also drags in the profile list and name derivation). ★Not `live_agents()`★: that is the recipient
    /// list, missing alive sessions filtered out by mail eligibility (shells) — their pending contracts (a CLI request from a shell) would be closed as failures.
    // ADR-0116 (decision 3), review fix D1
    fn is_agent_live(&self, id: PeerId) -> bool;
}

/// ★The entrance decision source snapshot★ (spec §5, ADR-0116) — the output of `DeliveryPort::addressing_sources`. Both fields are facts at one moment and
/// must be consistent: `dormant` = profiles with no session in `roster` (empty intersection); the adapter guarantees it, the service assumes it.
#[derive(Debug, Default, Clone)]
pub struct AddressingSources {
    /// **The only source `@here` looks at** (spec §4); `@all` looks here plus `dormant` (ADR-0121 decision 1).
    pub roster: Vec<LiveAgent>,
    /// ★Duplicates are not folded★ — input of the duplicate-name decision (a shared name → `RECIPIENT_AMBIGUOUS`). `@all` expands by id, so each gets its own row
    /// (ADR-0309, TRD S29 §4 X1).
    pub dormant: Vec<DormantAgent>,
    /// ★The listed PCs and their directory copies, from the host's peer module in the same snapshot call★ — read only by `ResolveScope::All`.
    ///   Empty = no other PC (every resolution stays local). `@all` / `@here` never expand to these.
    // ADR-0391
    pub pcs: Vec<RemotePc>,
}

/// ★Flush doorbell seam (C2 review fix 11)★ — hands "flush this agent's parking queue now" to another thread. Production = a non-blocking enqueue onto the
/// flush channel (host `messaging_host.rs` `ChannelIdleNotifier`).
/// ★Why (ADR-0125)★: inject is a blocking child-stdin write; making the send path (a tokio worker handling an MCP/HTTP request) also push other people's queues
/// lets one blocked pipe eat request handling. The sender draining its own target's queue is the normal path (ADR-0125 decision 1), so the doorbell only carries
/// queues unrelated to the send: a backed-off drain's deferral marker, idle transition, re-appearance. ★Contract★: `request_flush` is non-blocking.
/// ★Unwired fallback★: an assembly without the doorbell (experiment bin, unit tests) flushes **inline** (fail-open); production always plugs it in (lib.rs).
pub trait FlushTrigger: Send + Sync {
    /// Duplicate requests are folded by the consumer (coalescing).
    fn request_flush(&self, id: PeerId);
}

#[derive(Debug, Clone)]
pub struct LiveAgent {
    pub id: PeerId,
    pub name: String,
    pub epoch: u32,
    /// ★Can the turn boundary be observed — does the idle gate hold for this recipient (ADR-0116 decision 7, load-bearing)★.
    /// `true` (structured-output backend) → ask `BusyGate` before injecting and park if busy. `false` (terminal/console) → skip the gate: that CLI has its own input
    /// queue, so this value decides **only timing, not membership**. ★It does not decide order★ (ADR-0121 decision 2, ADR-0125): "ready to receive" is not "in what order".
    // ADR-0116, ADR-0121
    pub turn_signal: bool,
}

/// One asleep agent — a profile with no live session (spec §5 branch 3). `id` = profile id = the session id when relaunched; a reply contract to an asleep
/// recipient carries it from the moment it is opened (ADR-0309 decision 1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DormantAgent {
    pub id: PeerId,
    pub name: String,
}

struct MessagingState {
    mailbox: Mailbox,
    ledger: Ledger,
    /// ★Deferred-flush re-doorbell ledger (load-bearing)★ — key = park queue (recipient id), value = every doorbell id that backed off (de-duplicated set).
    /// `drain_queue` does not overlap for one recipient (a later batch would overtake the earlier one's remainder); a later drain backs off without touching the queue,
    /// and backing off alone loses the wakeup (as C1 finding 3). So the drain in progress takes the set at settle and presses the doorbell again on the way out. It also
    /// opens a concurrent send's letter — the backed-off sender answered `pending`, and the winning batch or this echo opens it (ADR-0125).
    /// The id held is always that queue's id (`drain_queue` step 0, ADR-0309); shape cleanup goes with the ADR-0142 target axis (TRD S29 §3). ★Bounded★: no duplicates.
    /// ★Not "re-doorbell whenever non-empty"★: that would restart flush endlessly while parking persists because delivery is impossible (busy, no target); it echoes
    /// once per id that deferred.
    // ADR-0107
    deferred_flush: HashMap<PeerId, Vec<PeerId>>,
    /// ★Target-axis in-flight★ — the resolved target PeerIds being injected into outside the lock. Origin (ADR-0142): a per-item target could make two queues of
    /// different names converge on one session, and overlap merged two envelopes into one turn (body write → wait submit → CR). Since the queue key became the id
    /// (ADR-0309) that convergence is gone; kept as harmless (TRD S29 §3 · §7 Q2 — amend ADR-0142 before removing). The invariant must hold while it remains.
    /// A set suffices: one drain registers once per target, and two drains on one target is what this guard prevents.
    /// ★`FlightSettle` alone releases★ — a leak means that session never receives mail again. ★No fail-open, no sweep (intended)★: getting trapped here is silently
    /// permanent. ★Invariant★: on a path where `insert` ran, until the owning `FlightSettle` is constructed nothing may fail (early return, `?`, panic);
    /// after construction Drop repays on any exit. Only failures that unwind count (abort leaves no process).
    // ADR-0142
    in_flight_targets: std::collections::HashSet<PeerId>,
    /// ★Target-axis deferral waiters★ — key = the target id being injected, value = doorbell ids that backed off because of it. ★Not writable into `deferred_flush`★
    /// (ADR-0142): that key is the park queue, but a target overlap happens between different queues, so the winner would not see the loser's marker (lost wakeup).
    /// Same wake path: merged with the `deferred_flush` share after settle (step 6 tail). ★Residue★: if the winner unwinds mid-injection the echo (on the stack above
    /// `FlightSettle`) is not pressed; the backed-off letter stays queued until the next appearance, turn-end or TTL (not lost). Production's hook ends an uncontained
    /// panic before unwind, so this is reachable only in test builds without it (ADR-0292).
    // ADR-0142
    deferred_by_target: HashMap<PeerId, Vec<PeerId>>,
    /// ★Per-PC backstop window★ — arrival instants of the inbound mails accepted in the last minute, per node (at most the limit each).
    // ADR-0390
    backstop: HashMap<NodeRef, VecDeque<Instant>>,
}

/// ★Shared★: built as an `Arc<MessagingService>` at daemon boot; ingress (handle_send), the flush observer and the sweep task share one instance (lib.rs run()).
pub struct MessagingService {
    state: Mutex<MessagingState>,
    /// `@`address resolution source (ADR-0104 decision 1, ADR-0112 decision 1). v1 has only the stateless built-in `@all`, so it is an out-of-lock field (groups.rs header).
    groups: BuiltinGroups,
    port: Arc<dyn DeliveryPort>,
    registry: Arc<dyn ControlPlanePort>,
    /// Production = `BusyPolicy`; unwired/unobservable = `AlwaysIdleGate` (immediate-injection fallback — busy.rs header).
    busy: Arc<dyn BusyGate>,
    /// `None` = unwired → inline drain fallback (FlushTrigger comment).
    trigger: Option<Arc<dyn FlushTrigger>>,
    /// `None` = no remote mail: remote rows fail `PEER_OFFLINE`, inbound mail is refused.
    // ADR-0390
    remote: Option<RemoteWiring>,
    /// ★Woken on every verdict★ — `await_remote_answer` waits on it (with the messaging lock) until its rows are settled or its deadline.
    // ADR-0393
    remote_settled: Condvar,
    /// ★Test-only acceptance critical-section hook (review fix D2)★: shows the ledger **while still holding the lock that recorded the acceptance**, fired once just
    /// before leaving the `handle_send` main critical section. Needed because "acceptance and contract closing in one critical section" cannot be asserted from final
    /// state (two locks give the same result without a race); if closing slips to another lock the observation reads `awaiting_reply` and goes red (no sleep, no thread).
    /// ★Contract★: reads only the ledger handed to it — does not re-take the service lock (non-reentrant std Mutex). ★`OnceLock`★: reads need no lock.
    #[cfg(any(test, feature = "test-harness"))]
    accept_hook: std::sync::OnceLock<Arc<dyn Fn(&Ledger) + Send + Sync>>,
    /// ★Test-only seam: fired inside `await_remote_answer` right before each wait★ — while the messaging lock is held, so it must not take it;
    ///   a test uses it to send a verdict only after the wait has started.
    #[cfg(test)]
    wait_hook: std::sync::OnceLock<Box<dyn Fn() + Send + Sync>>,
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
                backstop: HashMap::new(),
            }),
            groups: BuiltinGroups,
            port,
            registry,
            busy,
            trigger: None,
            remote: None,
            remote_settled: Condvar::new(),
            #[cfg(any(test, feature = "test-harness"))]
            accept_hook: std::sync::OnceLock::new(),
            #[cfg(test)]
            wait_hook: std::sync::OnceLock::new(),
        }
    }

    /// Installed once (idempotent; later calls ignored).
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

    /// Not a constructor argument: the doorbell is the flush channel, wired after the service, and assemblies without a gate (experiment bin, unit tests) must work without it
    /// (fallback = inline flush).
    pub fn with_flush_trigger(mut self, trigger: Arc<dyn FlushTrigger>) -> Self {
        self.trigger = Some(trigger);
        self
    }

    /// Wires mail to and from other PCs. Not a constructor argument for the same reason as the doorbell: assemblies without peers work as before.
    // ADR-0390
    pub fn with_remote(mut self, port: Arc<dyn RemoteDeliveryPort>, limits: RemoteLimits) -> Self {
        self.remote = Some(RemoteWiring { port, limits });
        self
    }

    /// ★The single query point of "is this recipient in a turn right now" (ADR-0121 §Impact, load-bearing)★. The idle gate applies only to backends whose turn boundary
    /// is observable (ADR-0116 decision 7); the class decision (`turn_signal`) and the busy lookup live in one function so decision points cannot use different predicates
    /// (the 2026-07-30 review defect class). ★Not asking the gate for the no-turn-signal class does not cover order★ — order is the enqueue order (ADR-0125), so this
    /// function does not look at the queue.
    // ADR-0121, ADR-0116
    fn gate_busy_reason(&self, target: &LiveAgent) -> Option<BusyReason> {
        if !target.turn_signal {
            return None;
        }
        self.busy.busy_reason(target.id, target.epoch)
    }

    /// The only send function the entrance (ingress) calls after argument validation and auth.
    ///   - `msg_id`: logical message id from the upper layer (ledger correlation, response `id`).
    ///   - `from`: token-derived sender identity (ADR-0086); used for observation and `@all` sender exclusion.
    ///   - `sender_name`: the envelope sender name, already canonical (ADR-0101).
    ///   - `to`: recipient tokens (agent name, exact PeerId string, or `@`address, mixable). Comma splitting is CLI-entrance only and done before here (MCP elements
    ///     never split — spec §6). Non-empty (an empty list is bounced as `INVALID_SEND_ARGS`).
    ///   - `body`: the unwrapped body.
    /// ★Return★: N per-recipient rows (spec §6) or a send-level bounce (`SendReject` — only address-space errors and id collisions; zero side effects). Even if every
    /// recipient fails it is `Ok(all rows failed)` (spec §5 partial progress). `delivered` if the drain actually injected the letter, else `pending` (spec §6).
    /// ★Two passes (load-bearing)★: parking stores envelope materials but `to` is unknown until every recipient is decided; one pass would fix an incomplete `to` into the
    /// first parked item. Per-recipient branches: module header.
    /// ★Reply contract (spec §3, ADR-0111 decision 5)★: with `meta.request`, a contract opens for each accepted recipient (key `(msg_id, recipient id)`; asleep uses its profile
    /// id, ADR-0309) **before enqueue** — deferring would deliver an untracked request (spec §3 item 5). With `meta.reply_to`, the contract is closed by strict replier match
    /// once accepted (delivery unaffected if that fails).
    /// ★Remote rows (ADR-0393)★: a row for an agent on another PC comes back `pending` with an "awaiting" hint; the entrance then calls
    ///   `await_remote_answer` to replace it with the outcome or "unconfirmed". This entrance resolves with `ResolveScope::All` and is for this
    /// PC's own agents only — never `Entrance::Remote` (mail from another PC goes through `handle_remote_mail`).
    // ADR-0111, ADR-0114, ADR-0125
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
        debug_assert!(
            entrance != Entrance::Remote,
            "mail from another PC enters only through handle_remote_mail (validation, backstop, local-only resolution — ADR-0390)"
        );
        self.send_pipeline(
            msg_id,
            from,
            sender_name,
            to,
            body,
            entrance,
            meta,
            ResolveScope::All,
        )
        .map(|o| o.results)
    }

    /// The send pipeline behind `handle_send` and the remote entrance — `scope` is the only difference (no forwarding, ADR-0390).
    // ADR-0111, ADR-0114, ADR-0125, ADR-0390, ADR-0393
    #[allow(clippy::too_many_arguments)]
    fn send_pipeline(
        &self,
        msg_id: &str,
        from: SenderIdentity,
        sender_name: &str,
        to: &[String],
        body: &str,
        entrance: Entrance,
        meta: &SendMeta,
        scope: ResolveScope,
    ) -> Result<SendOutcome, SendReject> {
        // ★Ingress is the only validator★: this function and `SendMeta` are pub, so another assembly could pass request+reply_to together (opens one contract while closing
        //   another's). Local validation is not duplicated so bounce wording does not diverge per entrance; a debug build blows up wiring mistakes
        //   instead. Mail from another PC is the exception: `validate_inbound` re-checks the pure contract rules before it gets here.
        debug_assert!(
            !(meta.request && meta.reply_to.is_some()),
            "ingress is the only validator — request and reply_to are mutually exclusive (spec §6)"
        );
        debug_assert_eq!(
            meta.reply_by.is_some(),
            meta.reply_by_raw.is_some(),
            "reply_by carries the parsed value and the original notation as a pair (SendMeta)"
        );
        debug_assert!(
            meta.to_attr.is_none(),
            "the envelope to is fixed once by handle_send after the acceptance decision of every recipient (spec §1)"
        );
        debug_assert!(
            !to.is_empty(),
            "an empty recipient list is bounced by the entrance with INVALID_SEND_ARGS (spec §6)"
        );

        // 1) One decision-source snapshot (outside the lock).
        let sources = self.port.addressing_sources();
        let mut closing = ReplyClosing {
            in_reply_to: meta.reply_to.as_deref(),
            replier_id: from.peer_id,
            done: None,
        };

        // 2) Address resolution (pure).
        let addressing = resolve_addressing(&self.groups, to, from, &sources, scope)?;
        // Delivery records this send leaves = recipient count including failure rows (spec §5); the lookup's `may_be_truncated` compares it with remaining rows.
        let expected_rows = u16::try_from(addressing.recipients.len()).unwrap_or(u16::MAX);
        let roster_names = live_names_hint(&sources.roster);
        let unknown_pcs = match scope {
            ResolveScope::All => unknown_directory_pcs(&sources.pcs),
            ResolveScope::LocalOnly => Vec::new(),
        };
        // ADR-0390: the receiver refuses a whole mail whose contract fields break its rules, so remote rows of such a send fail here instead.
        let remote_meta_refusal = validate_inbound_contract(meta).err().map(|r| r.reason);

        let now = Instant::now();
        let mut plans: Vec<RecipientPlan> = Vec::with_capacity(addressing.recipients.len());
        let mut park_effects = ParkSideEffects::default();
        let mut retirements = RetirementLog::default();
        let mut duplicate_contracts: Vec<String> = Vec::new();
        // One mail per PC, rows in send order (ADR-0393).
        let mut outbound: Vec<OutboundMail> = Vec::new();
        let mut remote_capped = 0usize;

        {
            let mut st = self.state.lock().expect("messaging state poisoned");
            // ★The id collision check is the first side-effect point★ (zero-side-effect bounce — `SendReject::IdCollision`). It catches only ids already in the ledger; two sends
            // drawing the same value at once can both pass (36^8 space, damage = history aliasing, keys are `(msg_id, recipient)`), so no reservation is made.
            if st.ledger.msg_id_in_use(msg_id) {
                return Err(SendReject::IdCollision);
            }

            // ── pass A: per-recipient acceptance decision ────────────────
            for (i, r) in addressing.recipients.iter().enumerate() {
                // 2-a) Entrance branches (spec §5, ADR-0116 decision 1). ★Asleep = accepted★ — no doorbell; the re-appearance flush picks it up.
                let (recipient_id, live) = match admission(r, &roster_names, &unknown_pcs) {
                    Admission::Local { id, live } => (id, live),
                    Admission::Remote {
                        node,
                        pc_name,
                        name,
                        link,
                    } => {
                        // ★Remote row (ADR-0393 decision 1)★: no contract here, recorded `RemotePending` with its tag (the row index), handed to the port after the lock.
                        //   Not sent: a PC without a live link (`PEER_OFFLINE`, ADR-0391 decision 1-5), a sender name the receiver would refuse (`NAME_TOO_LONG`), a token or
                        //   contract fields the receiver would refuse — which would sink the other rows of that PC's mail (ADR-0390), no remote wiring.
                        let refusal = match link {
                            PcLink::Connected => None,
                            other => Some((
                                FailCode::PeerOffline,
                                peer_offline_hint(&r.display, pc_name, other),
                            )),
                        }
                        .or_else(|| {
                            (!remote_name_ok(sender_name))
                                .then(|| (FailCode::NameTooLong, name_too_long_hint(&r.display)))
                        })
                        .or_else(|| {
                            (name.len() > REMOTE_NAME_MAX_BYTES).then(|| {
                                (
                                    FailCode::RecipientNotFound,
                                    remote_token_too_long_hint(&r.display, pc_name),
                                )
                            })
                        })
                        .or_else(|| {
                            remote_meta_refusal.as_deref().map(|why| {
                                (
                                    FailCode::RemoteRefused,
                                    remote_meta_hint(&r.display, pc_name, why),
                                )
                            })
                        })
                        .or_else(|| {
                            self.remote.is_none().then(|| {
                                (
                                    FailCode::PeerOffline,
                                    peer_offline_hint(&r.display, pc_name, PcLink::Offline),
                                )
                            })
                        });
                        if let Some((code, hint)) = refusal {
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
                        // The row index fits: a send's rows are bounded by its request size, far below `u32::MAX`.
                        let tag = i as u32;
                        st.ledger.record_remote(
                            msg_id,
                            sender_name,
                            from.peer_id,
                            r.name(),
                            body,
                            now,
                            expected_rows,
                            RemoteSend {
                                node,
                                pc_name: pc_name.to_string(),
                                tag,
                                sent_to: name.to_string(),
                                reply_by: if meta.request {
                                    meta.reply_by.zip(meta.reply_by_raw.clone())
                                } else {
                                    None
                                },
                                closes: meta.reply_to.clone(),
                            },
                        );
                        // ADR-0393 decision 5: a reply to a remote requester closes its contract at verdict time; until then no overdue report.
                        if let Some(in_reply_to) = &meta.reply_to {
                            st.ledger.hold_reply(in_reply_to, from.peer_id, now);
                        }
                        let row = OutboundRow {
                            tag,
                            to: name.to_string(),
                        };
                        match outbound.iter_mut().find(|m| m.node == node) {
                            Some(m) => m.rows.push(row),
                            None => outbound.push(OutboundMail {
                                id: msg_id.to_string(),
                                node,
                                from,
                                sender_name: sender_name.to_string(),
                                rows: vec![row],
                                body: body.to_string(),
                                meta: meta.clone(),
                            }),
                        }
                        plans.push(RecipientPlan::Remote {
                            pc_name: pc_name.to_string(),
                        });
                        continue;
                    }
                    Admission::Reject(code, hint) => {
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

                // 2-b) ★Parking opens a contract too — asleep included (spec §3 item 2, ADR-0116 decision 1)★: it will be injected someday, so the reply obligation and deadline sweep apply.
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
                        // Under cap pressure the oldest retirable contract got a retirement-pending mark (mark-and-sweep, ADR-0108); removal happens in the commit below, reverted if
                        //   this recipient ends in failure.
                        OpenOutcome::OpenedAfterMarking(rc) => {
                            contract = Some(open_reservation(
                                &mut st,
                                self,
                                msg_id,
                                recipient_id,
                                Some(rc),
                            ))
                        }
                        // Recipients were already de-duplicated in the resolution step, so a duplicate `(id, recipient)` is unreachable.
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
                    // ADR-0393 decision 4: a request from another PC keeps the requester's node (from the link) for the overdue report.
                    if let (Some(node), Some(_)) = (from.node, &contract) {
                        st.ledger.attach_requester_node(msg_id, recipient_id, node);
                    }
                }

                // 2-c) ★Enqueue is unconditional (ADR-0125, spec §5 — load-bearing)★: the entrance checks neither busy nor what is ahead; it puts the item on the queue tail.
                //      A gate here revives direct send, loosens order from enqueue order and brings back seat reservation and joining — running the overhaul backwards.
                //      ★The only axis is the mailbox limit★, so every send passes the cap (`MAILBOX_FULL` increasing is a design consequence — spec §5).
                if st.mailbox.can_admit(recipient_id, ParkKind::Message) {
                    plans.push(RecipientPlan::Park {
                        recipient_id,
                        contract,
                        live,
                    });
                    continue;
                }
                // Revert the contract just opened (no ghost overdue notice for a request never delivered).
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

            // ADR-0393: the remote store is pruned once per send that added to it.
            if !outbound.is_empty() {
                remote_capped = st.ledger.prune_remote(now);
            }

            // ── Envelope `to` freeze (spec §1) ─────────────────────
            let admitted: Vec<bool> = plans
                .iter()
                .map(|p| !matches!(p, RecipientPlan::Failed { .. }))
                .collect();
            let admitted_count = admitted.iter().filter(|ok| **ok).count();
            // ★Freeze `to` **before injection**★: the envelope is assembled at injection time with this material. ★Do not "wrap after the outcome"★ — it would flatten the drain's
            //   `delivered` accounting to pending.
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
                    _ => unreachable!("park_targets holds only Park"),
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
                    // ★Unreachable★: `can_admit` passed under the same lock. Not swallowed — it means store accounting went off, so it degrades to a failure row.
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
            log_remote_capped(remote_capped);
            for name in &duplicate_contracts {
                tracing::error!(
                    msg_id = %msg_id,
                    recipient = %name,
                    "reply contract key (msg_id, recipient) duplicated — a wiring defect signal that recipient de-duplication went wrong (ADR-0111 decision 5)"
                );
            }

            // 2-d) Remote mail goes out before the local drains (outside the lock; the port is non-blocking — ADR-0393). A verdict the port reports at once
            //      lands in the ledger and is picked up by `await_remote_answer`.
            if let Some(wiring) = &self.remote {
                for mail in outbound {
                    wiring.port.enqueue(mail);
                }
            }

            // 3) Synchronous drain (outside the lock — ADR-0125 decision 1).
            let mut deferred_inline: Vec<PeerId> = Vec::new();
            let mut results: Vec<RecipientResult> = Vec::with_capacity(plans.len());
            let mut remote_rows: Vec<bool> = Vec::with_capacity(plans.len());
            for (i, plan) in plans.into_iter().enumerate() {
                let display = addressing.recipients[i].display.clone();
                remote_rows.push(matches!(plan, RecipientPlan::Remote { .. }));
                let result = match plan {
                    RecipientPlan::Failed { code, hint } => RecipientResult {
                        to: display,
                        status: SendStatus::Failed,
                        code: Some(code),
                        hint: Some(hint),
                    },
                    // Placeholder until `await_remote_answer` seals the answer.
                    RecipientPlan::Remote { pc_name } => RecipientResult {
                        hint: Some(awaiting_hint(&display, &pc_name)),
                        to: display,
                        status: SendStatus::Pending,
                        code: None,
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
                            // ★A share that could not go out must keep an occasion to be opened (spec §5)★, but press the doorbell narrowly: in the backed-off case the winner
                            //   echoes it while settling (`deferred_flush`), and pressing because of one's own injection failure is forbidden (re-injection to an unreachable
                            //   recipient — spec §5). Only the gate-caught and not-in-roster-at-drain branches press.
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

            // ★Retirement metering is recorded **after the outcome loop** (M1)★: the commit moved into pass B and the injection loop, so recording before it would leave the
            //   main-path retirement with no log — under ADR-0108 decision 2 that info log is the only evidence of retirement.
            log_contract_retirements(msg_id, &retirements.real);
            // ★A retirement planned but not done (R2)★ is not counted; left as an anomaly.
            for r in &retirements.phantom {
                tracing::warn!(
                    planned_msg_id = %r.request_id,
                    to = %r.recipient,
                    new_msg_id = %msg_id,
                    "a contract marked for retirement was already gone at commit time — no retirement happened (cap pressure was relieved one time less)"
                );
            }

            // 4) The inline drain of the doorbell-unwired assembly (the `ring_or_defer` division of labor). ★Not a direct send★ (spec §5 exception): the sending thread runs the
            //    shared drain routine on the spot (also what makes kernel unit tests deterministic); ripping it out makes the harness nondeterministic and cuts the normal path.
            for id in deferred_inline {
                self.flush_for_agent(id);
            }

            // 5) Reply contract handling (spec §3 item 7-④, ADR-0116 decision 2, ADR-0118).
            // ★Why three branches (load-bearing — the ledger must not lie in either direction)★:
            //   - Unconditional close claimed a message that never arrived as `replied` (requester died → the contract closed, `reply_owed_by_me` vanished, no deadline notice).
            //   - Keeping it open regardless of reason left a contract with nowhere to go forever.
            //   - Ratified: **parking is also acceptance** — replies piling up for an asleep requester fulfil the replier's obligation; if the requester is deleted later and the parked
            //     item is cleared as `RECIPIENT_DELETED` the contract stays `replied` (contract axis = "did they answer", delivery axis = "did it arrive", spec §6). Only when the
            //     requester name is nowhere (`RECIPIENT_NOT_FOUND`) does it close as a failure. Mailbox full, duplicate name and unreachable are the environment of that moment
            //     and a retry may succeed, so no-op (closing would leave "reply failed" forever even if the retry delivered).
            // ★Codes that cannot occur on the reply path★: `REQUEST_CAPACITY` (reply opens no contract; `reply_to`↔`request` are exclusive) and `RECIPIENT_DELETED` (lookup-time only) — no branch.
            // ★Remote rows are left out★: their outcome is not known yet, so the verdict entry closes by the same rule (ADR-0393 decision 5). A reply whose only
            //   row is remote has nothing to decide here.
            // ADR-0116, ADR-0118, ADR-0393
            let local_results: Vec<RecipientResult> = results
                .iter()
                .zip(&remote_rows)
                .filter(|(_, remote)| !**remote)
                .map(|(r, _)| r.clone())
                .collect();
            if let (Some(in_reply_to), false) = (&meta.reply_to, local_results.is_empty()) {
                match (closing.done, reply_disposition(&local_results)) {
                    // Accepted → already closed in-lock. Only the result is logged outside the lock.
                    (Some(outcome), _) => log_reply_close(outcome, in_reply_to, sender_name),
                    // ADR-0391: a requester on another PC whose directory is unknown this run was not searched — "not found" is not "gone".
                    (None, ReplyDisposition::RequesterGone)
                        if self.requester_pc_unsearched(in_reply_to, from.peer_id, &sources) =>
                    {
                        tracing::debug!(
                            in_reply_to,
                            "reply not delivered — the requester's PC has an unknown directory this run, so the contract stays open (no-op)"
                        )
                    }
                    (None, ReplyDisposition::RequesterGone) => {
                        self.fail_reply_contract(in_reply_to, sender_name, from.peer_id)
                    }
                    (None, ReplyDisposition::NoOp) => tracing::debug!(
                        in_reply_to,
                        "reply failed for a temporary reason (mailbox full · same name) — the contract **stays open** (no-op). A retry closes it on the normal path (spec §3 item 7-④)"
                    ),
                    // ★Accepted row without the in-lock close = a wiring defect★: every accept branch goes through `close_in_lock`, so unreachable; recorded rather than passed over
                    //   (a deadline notice would go out with the contract unclosed).
                    (None, ReplyDisposition::Accepted) => tracing::error!(
                        in_reply_to,
                        replier = %sender_name,
                        "the reply was accepted but closing the contract did not happen in the critical section — accept-branch wiring missing (review fix D2)"
                    ),
                }
            }
            Ok(SendOutcome {
                results,
                token_rows: addressing
                    .tokens
                    .iter()
                    .map(|t| match t {
                        AddressToken::Name { row } => Some(*row),
                        AddressToken::Group { .. } => None,
                    })
                    .collect(),
                row_ids: addressing.recipients.iter().map(|r| r.id()).collect(),
            })
        }
    }

    /// ★Reply contract close★ — strict matching, contract chosen on the replier basis (ADR-0111 decision 5). The result only changes **logging** (no effect on delivery or
    /// the response — spec §3 item 7-②). No new error for `NoMatch`/`AlreadyClosed`: unfixable, and a bounce would retry an already-delivered message (duplicate).
    /// ★Test-seam only — production does not use this verb (review fix D2)★: production closes in the very lock span that recorded the acceptance via
    /// `ReplyClosing::close_in_lock` (canonical rationale in that struct header); split across two locks a delete cleanup in between would close the contract as
    /// `reply_failed` ("accepted but reply failed"). Hence `cfg(test)` alone (the `test-harness` feature alone has no caller) — in a production build the
    /// "simpler" verb would revive that race.
    #[cfg(test)]
    fn close_reply_contract(&self, in_reply_to: &str, replier_name: &str, replier_id: PeerId) {
        let outcome = {
            let mut st = self.state.lock().expect("messaging state poisoned");
            st.ledger
                .close_on_reply(in_reply_to, replier_id, Instant::now())
        };
        log_reply_close(outcome, in_reply_to, replier_name);
    }

    /// ★Reply contract **failure close** (spec §3 item 7-④, ADR-0116 decision 2, ADR-0118 decision 1)★ — only when the reply row was `RECIPIENT_NOT_FOUND` (requester in
    /// neither roster nor profiles). Closes the contract axis only: the reply's failure row is already recorded and the original request stays `delivered` (spec §6; no
    /// `delivered → reply_failed` transition). ★Unlike the accept branch, splitting the lock is safe (D2)★: no acceptance here, so a delete cleanup closing the same
    /// contract in between gives the same outcome. `replier_name` is logging only — the contract is chosen by `replier_id` (ADR-0309).
    // ADR-0116, ADR-0118
    fn fail_reply_contract(&self, in_reply_to: &str, replier_name: &str, replier_id: PeerId) {
        let outcome = {
            let mut st = self.state.lock().expect("messaging state poisoned");
            st.ledger
                .fail_on_undeliverable_reply(in_reply_to, replier_id)
        };
        log_reply_fail(outcome, in_reply_to, replier_name);
    }

    /// Is the replier's contract owed to a requester on a listed PC whose directory is unknown this run (so a bare-name reply could not find it).
    // ADR-0391
    fn requester_pc_unsearched(
        &self,
        in_reply_to: &str,
        replier_id: PeerId,
        sources: &AddressingSources,
    ) -> bool {
        let node = {
            let st = self.state.lock().expect("messaging state poisoned");
            st.ledger.requester_node(in_reply_to, replier_id)
        };
        node.is_some_and(|n| {
            sources
                .pcs
                .iter()
                .any(|pc| pc.node == n && pc.directory.is_none())
        })
    }

    /// ★Bulk cleanup on profile deletion (spec §5, ADR-0116 decision 3, ADR-0309 decision 6)★ — called by the host right after it deletes a profile.
    /// ★Trigger = deleted ∧ no live session for that profile id (review fix D1, load-bearing)★: the profile-delete command does not kill a running session, so "tree entry
    /// deleted, process alive" exists; that peer can receive once idle, so cleaning up would kill deliverable mail and mark a held contract failed. When the session
    /// disappears it converges to the absent class.
    /// ★The gate is on the id axis (`is_agent_live(profile_id)`) — rename immunity★: after deletion the live session's canonical name changes from `display_name` to
    /// `basename(session.cwd)`, so a pre-deletion name never matches and cleanup would fire on a single `RenameProfile`. The predicate is liveness (`Running|Exiting`),
    /// not the recipient list (`is_agent_live` doc).
    /// ★Sweeps by id, so there is no name-axis gate (ADR-0309 decision 6)★: parked items to that id and open contracts requested by that id are found by id; a different agent
    /// with the same name has a different key and is untouched. ★Do not revive the second "live session holding the name" gate (N1)★ — a live duplicate would skip the whole share.
    /// ★Two things★: ① close all parked items to that profile id as `failed` + `RECIPIENT_DELETED` (ledger terminal + lookup hint; history stays, so the sender sees the
    /// reason via `eg_messages{id}` — "never silently discard live mail") ② close open contracts where that id is the requester as `reply_failed`. A `replied` contract
    /// is not reverted; a contract whose replier side was deleted is kept (the sender learns via the deadline notice).
    /// ★Deletion time is one-shot (spec §5)★: the trigger is evaluated only then, so ⓐ an item parked after the decision misses this scan and ⓑ a live session whose profile alone
    /// was deleted and dies later has no event to collect it — that residue ends as TTL 24h `expired`, not `RECIPIENT_DELETED` (accepted). A symmetric trigger and an
    /// insert-time tombstone are not adopted (rejected in ADR-0116); reviving them needs a new decision.
    /// ★Caller duty (ADR-0006)★: call after releasing the profile registry lock (else lock-order inversion — spec §5). `name` is logging only.
    // ADR-0116, ADR-0118, ADR-0309
    pub fn handle_profile_deleted(&self, profile_id: PeerId, name: &str) -> ProfileDeletedOutcome {
        // The gate = the id axis alone (doc above).
        if self.port.is_agent_live(profile_id) {
            tracing::debug!(
                deleted = %name,
                profile = %profile_id,
                "profile deletion cleanup skipped — that profile's session is still alive (only the tree item was removed). Parked items·contracts are left as is and delivered normally on the idle transition (spec §5)"
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
                "deletion cleanup: the ledger state of a queued item was not pending — queue↔ledger accounting mismatch (wiring defect signal)"
            );
        }
        if !purged.is_empty() || !contracts.failed.is_empty() {
            tracing::info!(
                deleted = %name,
                parked_failed = purged.len(),
                contracts_failed = contracts.failed.len(),
                guard_held = contracts.guard_held,
                "profile deletion cleanup — terminated parked items with RECIPIENT_DELETED and terminated open contracts whose requester is that profile with reply_failed (ADR-0116 decision 3 · ADR-0309 decision 6)"
            );
        }
        if contracts.guard_held > 0 {
            tracing::debug!(
                deleted = %name,
                held = contracts.guard_held,
                "deletion cleanup: contracts marked provisional·retiring were not touched (lifetime = owned by the guard, ADR-0118 decision 3). No re-firing — the remainder is TTL's job (spec §5)"
            );
        }
        ProfileDeletedOutcome {
            skipped_live: false,
            failed_parked: purged.len(),
            failed_contracts: contracts.failed.len(),
            guard_held_contracts: contracts.guard_held,
        }
    }

    /// ★Doorbell division of labor★ — wired: enqueue immediately (non-blocking); unwired: put it in `deferred` and the caller runs it inline after the response is finalized.
    /// ★Why split★: the wired branch is a channel send (deferring pushes the wake behind the remaining blocking writes; a panic loses it), while the unwired branch runs the
    /// batch write on this thread — calling it mid-injection-loop would cause flush re-entry and accounting skew (response `pending` vs ledger `delivered`).
    fn ring_or_defer(&self, id: PeerId, deferred: &mut Vec<PeerId>) {
        match &self.trigger {
            Some(t) => t.request_flush(id),
            None => deferred.push(id),
        }
    }

    /// ★The inline fallback must also be called outside the messaging lock★ — every call site is after park returns.
    fn request_flush(&self, id: PeerId) {
        match &self.trigger {
            Some(t) => t.request_flush(id),
            None => self.flush_for_agent(id),
        }
    }

    /// ★`<notice>` parking + recording ledger `pending`★ — wrapper for daemon notices only (the send path uses `park_into` directly). ★Never bounced★: `ParkKind::Notice` has
    /// its own lane (`mailbox::NOTICE_CAP` = 64) and on overflow only reclaims the oldest notice, so no return value (a bounce would be silent loss).
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

    /// ★Thin entrance for async occasions (appearance/epoch/idle/doorbell — ADR-0104)★: calls the shared drain routine and discards only the report (no sender to answer).
    /// `recipient` = agent id = the queue key (ADR-0309).
    pub fn flush_for(&self, recipient: PeerId) {
        let _ = self.drain_queue(recipient, None);
    }

    /// ★The one injection code (ADR-0125 §Impact, load-bearing)★: injects the parked items addressed to a recipient id in bulk, oldest first. A send-only injection code
    /// would be the "two paths" defect 0124/0125 removed, and seat reservation and the joining decision would become necessary again.
    /// `snapshot`: `Some` = the caller reuses the roster it just took (synchronous drain — one snapshot per send, ADR-0111 decision 2); `None` = take it here (async occasion —
    /// the recipient may have died or reappeared). "Oldest first" holds inside the batch; re-parking is a sequence-number merge, so carried-over older items keep priority.
    ///
    /// ★In-flight accounting (F1, load-bearing)★: at the `take_in_flight` registration point (just before releasing the lock) the outgoing batch is not in the queue but is still
    /// outstanding mail to that recipient. Leaving it out of the cap denominator lets the queue grow without bound (drain empties it → during the out-of-lock inject k concurrent
    /// sends see an empty queue and pass → inject fails and N items are restored → N + k, next batch also N + k, +k every cycle). So the batch stays in the denominator via
    /// `take_in_flight` and is settled per terminal; a missed settlement is covered by `FlightSettle` (Drop). Cost: while flush runs, a new send to that recipient can
    /// bounce as `MAILBOX_FULL` (spec §5 chose a visible bounce over silent growth).
    ///
    /// ★★Drains for one recipient must not overlap — do not remove (the premise of ADR-0125)★★: while batch A injects outside the lock, a batch B draining the queue would reach
    /// the recipient before A's remainder (order inversion), so step 0 checks in-flight and backs off. Since the drain runs inside the send call, two sending threads draining
    /// one recipient is routine; ripping the guard out as "surplus" breaks "delivery order = enqueue order". Two drains exist at once in production too (sync drain on the send
    /// path, the unwired inline fallback, the host delivery lane running recipients in parallel) — not a never-reached line of defense.
    /// ★Exclusion has two axes (do not remove)★: **Queue axis** (step 0 · `mailbox` in-flight, key = park queue/recipient id) blocks the order inversion and carries the cap
    /// denominator accounting. **Target axis** (`in_flight_targets`, ADR-0142; key = session id) sees the same thing today and only catches the gap from the batch's last
    /// settlement to the target release (why kept: `in_flight_targets` comment).
    /// ★The backed-off letter is not lost (both axes)★: it rides the winner's batch, and if it could not go out the receipt holder presses the doorbell again while settling
    /// (backing off alone is a lost wakeup, stuck until the next idle/appearance). Markers: `deferred_flush` (queue axis), `deferred_by_target` (target axis); one pressing verb.
    /// Remove the guard and batches tangle; remove the echo and the backed-off letter sticks.
    ///
    /// ★Recipient = that one id in the current roster (ADR-0309 decisions 1·5, load-bearing)★: the queue key is the receiving agent; an async snapshot may be stale, so look the id
    /// up again in the current roster just before drain. Absent (death, asleep) → skip; parking stays until the same id returns (new incarnation) or TTL. ★Do not look it up by
    /// name★ — a different agent that took the name after a rename/deletion would receive someone else's mail.
    ///
    /// ★The gate is looked at exactly once per drain (C2 review fix 4, load-bearing)★: one check before the first injection — in a turn now → return the batch in original order
    /// (that turn's MessageDone fires the trigger again); this blocks injection mid-turn (e.g. an appearance flush inside a priming turn). ★No mid-batch recheck★ (invariant):
    /// the first injection immediately triggers the recipient's input-time user echo (`Structured` = observed as turn progress), so a per-item look would stop the batch after 1 item
    /// (dribble injection, the rejected ADR-0104 alternative). The gate is safe because observation starts only from live output (a resumed transcript is not observed — agent
    /// `OutputCore::seed`), so busy = a turn in progress whose end notice must come; a lost notice is woken fail-open by the `BUSY_MAX_TURN` cap sweep (busy.rs).
    /// ★Undelivered items never leave the queue (lock atomicity, load-bearing)★: drain, gate and restoring the skipped items finish in one lock span and only the items to deliver
    /// leave it; releasing between drain and restore made a window where the queue looked empty and concurrent sends wobbled.
    /// ★Restore order★: new items may enter while returning, so they return via `restore_ordered` (admission-sequence merge) — the queue keeps global oldest-first.
    /// ★Accepted residual★: if the recipient starts a new turn on its own mid-batch, the remaining injections go into the CLI's stdin queue (nothing lost, only read time blurs — spec §7 unverified).
    // ADR-0125, ADR-0142, ADR-0309
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
        // Count backed off on the target axis — logged outside the lock.
        let mut target_deferred = 0usize;
        let mut evicted_transitions: Vec<EvictedTransition> = Vec::new();
        // 1~4) Drain + record expiry in the ledger + gate + immediate restore of undelivered items.
        // ★No tracing while holding the lock — collect, then log outside it (finding 3)★: a synchronously-formatting subscriber can block on the stdout lock and spread that into
        //   messaging lock waits.
        // ★Calling the gate under the lock is not a violation★: `BusyGate` is a pure lookup + short lock whose "safe under the messaging lock" is a contract (busy.rs `BusyGate`);
        //   the reverse (busy lock → messaging lock) does not exist (notice is a non-blocking channel send). Forbidden is **DeliveryPort (inject/roster)** — outside the lock (step 5).
        // ★Labeled block `'drained`★: the expiry evict log must be outside the lock; an early `return` on "nothing to deliver" would drop it, so the block exits by value (empty
        //   batch) and logging is on the common path after release.
        /// The receipt + the batch to deliver (recipient, items). `None` = **deferral** (backed off at step 0; touched neither queue nor receipt) — distinct from an empty batch `Some`
        ///   ("nothing to look at" vs "not its turn to look").
        type Drained = Option<(FlightTicket, Option<(LiveAgent, Vec<ParkedMessage>)>)>;
        let drained_or_deferred: Drained = 'drained: {
            let mut st = self.state.lock().expect("messaging state poisoned");
            // 0) Block duplicate entry into flush for the same recipient (why: doc above). Decision and record are in the same lock span, so no race with the receipt holder's
            //    settlement: if the holder is first, in-flight is 0 here; if here is first, the holder sees this marker.
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
            // `drain` yields ascending admission sequence, so the wholesale restore below meets `restore_ordered`'s caller contract.
            let deliverable = drained.deliverable;
            if deliverable.is_empty() {
                break 'drained Some((FlightTicket::default(), None));
            }
            let Some(target) = target else {
                no_target_kept = deliverable.len();
                st.mailbox.restore_ordered(recipient, deliverable);
                break 'drained Some((FlightTicket::default(), None));
            };

            // ★Target-axis deferral (ADR-0142) — back off if a target is still held on this session★. Looked at before the gate because the busy branch's wake hangs on the other
            //   side's turn-end notice (+ cap sweep), while a share backed off here is always echoed by the receipt holder (`deferred_by_target`) — spend the surer axis first.
            // ★Neither drop nor forget★: return the batch in original order and register a waiter — missing either strands the letter until TTL.
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
            // ★Known residual (not fixed)★: if the batch that left here panics in the out-of-lock inject, `FlightSettle` (Drop) repays the in-flight accounting but the items do not
            //   return to the queue (ownership was on the stack). Pre-v1 condition, not a guarded path; production ends that panic via the hook, so only the hookless test assembly is
            //   exposed (ADR-0292).
            // ★In-flight registration — only the share leaving the lock (F1, load-bearing)★: the cap-denominator hole exists only while the lock is released. Shares returned above (busy,
            //   no target, target-axis deferral) or terminal (expiry) are outside it; counting them inflates the denominator and bounces normal inflow as `MailboxFull`. The receipt is
            //   always returned by `FlightSettle` below (a miss blocks that recipient's lane permanently).
            let flight = st.mailbox.take_in_flight(recipient, deliverable.iter());
            // Target-axis registration is in the same lock span (split from the decision above, two batches could enter one session). Release by `FlightSettle` alone (a leak blocks it permanently).
            st.in_flight_targets.insert(target.id);
            Some((flight, Some((target, deliverable))))
        };
        // Deferral (step 0) touched neither queue nor receipt: nothing to log, settle or echo (echo is the receipt holder's job — tail below); one debug line.
        let Some((flight, batch)) = drained_or_deferred else {
            tracing::debug!(
                recipient = %recipient,
                "drain deferred: a previous batch to the same recipient is still being injected (in-flight) — backing off without draining, doorbell again at settlement (round-7 · ADR-0125)"
            );
            return report;
        };
        // ★Single-exit settlement guard (F1)★ — the delivery loop has an early break and out-of-lock calls (inject); scattered settlement missing one place permanently inflates that
        //   recipient's denominator, so the remaining receipt is returned by Drop (normal exit and unwinding).
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
                "drain deferred (target axis): a previous batch of this session still holds the target — batch put back, doorbell again at settlement"
            );
        }
        if no_target_kept > 0 {
            tracing::debug!(
                recipient = %recipient,
                keeping = no_target_kept,
                "flush skip: no agent with that id is in the roster now (asleep·dead) — kept parked, the appearance flush opens it when the same id comes up again (ADR-0309)"
            );
        }
        if let Some((epoch, parked)) = busy_skipped {
            tracing::debug!(
                recipient = %recipient,
                epoch,
                parked,
                "flush skip: the recipient is in a turn — batch not started (kept parked, the turn-end notice retries)"
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
                        // This item hit its terminal — detach it from the in-flight receipt now (F1); holding it would bounce normal inflow while a long batch runs.
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
                        // Observation `to_name` = the roster name at injection (the queue key is an id).
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
                        // ★Lossless on partial failure (load-bearing — ADR-0103/0104)★: the recipient died between drain and inject — return the remaining share including the failed part.
                        //   `restore_ordered`, not `park`: ① cap bypass — a concurrent park may have refilled the queue to the cap, park would bounce `MailboxFull` and silently lose an
                        //   admitted message (ledger `pending`, not in the queue: ghost pending); restore_ordered does not count the cap (cap = inflow control, not retention).
                        //   ② admission-sequence merge — the older re-parked part must go ahead of concurrently parked newer items. `parked_at` keeps its value via clone (no TTL extension).
                        //   Ledger is already pending, so no transition. Retry at the next appearance.
                        // ★Cutting the batch here is correct★: the recipient restarted or became unreachable, so the failed part and the remainder must be returned to pass the gate again
                        //   on the new incarnation (pushing on could inject mid-turn).
                        let remaining: Vec<ParkedMessage> = items[n..].to_vec();
                        let remaining_count = remaining.len();
                        // ★Restore and settlement are paired in one lock span (F1 invariant)★: restore moves items from in-flight to the queue; releasing between the two double-counts
                        //   them (denominator inflates), settling first reopens the window. `queue + in_flight` is invariant thanks to the pairing.
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
                            "inject failed/refused during message drain — re-parking the rest of the batch (lossless restore_ordered, ADR-0103/0104): {e}"
                        );
                        report.caller_inject_error = Some(e);
                        break;
                    }
                }
            }
        }

        // 6) Echo the deferred drain — wakes the caller that backed off at step 0 (`deferred_flush` comment).
        // ★Empty the receipt first★: the echoed flush hits step 0 again (on this thread in the inline fallback); our remaining receipt would defer it on the spot and the wake only
        //   shuttles as a marker — hence dropped explicitly rather than left to Drop. ★Release the target axis before echoing too★ (same reason); the ids it returns key the waiter lookup below.
        let released_targets = flight.release_targets();
        drop(flight);
        // ★Take the set out whole inside the lock (load-bearing)★: in the inline fallback the echo re-enters this thread's flush, which may newly insert an id under the same key;
        //   iterating the held map would mix that in and risk a re-borrow. Owning the snapshot makes us responsible for it only; deferrals accumulated meanwhile belong to the then
        //   receipt holder's step 6.
        // ★Merge the waiters of both axes into one list, pressed by one loop★: two wake paths make the outcome depend on which pressed and let a one-sided regression hide.
        //   Two axes only means two marker keys; the waking verb is `request_flush`.
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
        // ★Press them all — do not pick★: a surplus doorbell costs ~nothing (early return on an empty queue in `flush_for_agent`); missing one binds live mail until TTL.
        for id in deferred {
            self.request_flush(id);
        }
        report
    }

    /// ★Turn end (idle transition) · doorbell flush (ADR-0104 decision 3)★: a flush targeted **by id** (queue key = agent id, ADR-0309 — no name resolution, so a rename mid-turn
    /// does not matter). ★Early return on an empty queue★: the idle notice comes every turn (busy.rs `IdleNotifier` — surplus over omission) and most find nothing parked, so check the
    /// queue under a short lock **before** the roster snapshot (list_agents walks all sessions).
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

    /// ★TTL sweep + overdue-reply notice (spec §5·§3)★: the single maintenance entry called by the periodic task (lib.rs). ① expired parked items become `pending→expired` in the ledger
    /// ② overdue unanswered requests get a `<notice>` delivered **to the sender** (spec §3 step 4) — nudge or give up is the sending LLM's call (ADR-0103 decision 3), the daemon only
    /// reports. ★Exactly once (spec §7)★: `due_timeouts` sets `notified` as it returns; the ledger prevents double notice. ★Non-blocking (protects the sweep task)★: it does not
    /// inject — a notice goes only to parking + doorbell, the flush lane writes (rationale: `deliver_notice`).
    /// ★A `NotFound` on the expiry transition is not swallowed (C4 review fix J)★: a broadcast writes N records per send, so the history ring (`ledger::HISTORY_CAPACITY`) rotates fast and the
    /// record may already be pushed out, leaving "it expired" nowhere. Still best-effort (no resurrection), but logged at debug (outside the lock).
    pub fn sweep(&self, now: Instant) {
        // ★F1 guarantee layer — reclaim **abandoned** contract reservations★: RAII Drop does nothing when `try_lock` fails (`Reservation` header), so this point redoes the job.
        // ★Not reached on the normal path since ADR-0125★ (the reservation window sits inside the enqueue lock); it catches forgotten-branch / unwinding debris. Do not delete
        //   as "finds nothing" — removing it reopens ADR-0108 and drops the guarantee on `Reservation`'s Drop-failure branch.
        let reclaimed = {
            let mut st = self.state.lock().expect("messaging state poisoned");
            st.ledger.reclaim_abandoned_reservations()
        };
        for (msg_id, recipient_id) in &reclaimed {
            tracing::error!(
                msg_id = %msg_id,
                recipient_id = %recipient_id,
                "sweep reclaimed an unsettled contract reservation — the send path is missing a commit/rollback (in a test build without the hook it may be that thread's panic — F1 guarantee layer)"
            );
        }

        let mut evicted_transitions: Vec<EvictedTransition> = Vec::new();
        let remote_capped: usize;
        let due: Vec<DueTimeout> = {
            let mut st = self.state.lock().expect("messaging state poisoned");
            let expired = st.mailbox.sweep_expired(now);
            for ex in expired {
                // ★Target exactly by (msg_id, recipient) (C4, ADR-0104)★: the ledger holds 1 msg_id : N delivery records, so msg_id alone cannot name the member. The mailbox returns its own
                //   queue key per expired item (`ExpiredParked`); the old "first pending record" reverse lookup could expire the wrong member.
                transition_or_collect_evicted(
                    &mut st.ledger,
                    &ex.msg.msg_id,
                    ex.recipient,
                    DeliveryStatus::Expired,
                    now,
                    &mut evicted_transitions,
                );
            }
            remote_capped = st.ledger.prune_remote(now);
            st.ledger.due_timeouts(now)
        };
        log_remote_capped(remote_capped);
        // Logged first, regardless of whether anything is due.
        log_evicted_transitions(&evicted_transitions);
        if due.is_empty() {
            return;
        }
        for d in due {
            if let Some(node) = d.sender_node {
                self.report_remote_overdue(&d, node);
                continue;
            }
            // The spec §1 notice template (English since ADR-0383; the S18 spec example keeps the old wording). The deadline notation is the sender's original (fix 6 — matches envelope `reply-by`).
            // ★The `[engram]` prefix marks the sender origin★: `<notice>` has no `from`, which alone can leave the receiving LLM unsure who sent it (priming unread or envelope summarized), so the
            //   first token states system origin. ★Not a parsing contract★: a readability label; machine decisions use the tag shape (`<notice>`, no `from`) and never read it back.
            let body = format!(
                "[engram] request {} deadline ({}) exceeded — no reply from {}",
                d.request_id, d.reply_by_raw, d.recipient
            );
            self.deliver_notice(&d, &body);
        }
    }

    /// ★`<notice>` delivery (C3)★ — sends a daemon notice to one recipient; **always parking + doorbell**. ★Never injects here (load-bearing — protects the sweep task)★: the only caller is
    /// `sweep`, a 60 s tokio task that lives as long as the daemon (lib.rs); `inject` is a blocking child-stdin write, so one blocked pipe would stall TTL expiry and the `BUSY_MAX_TURN`
    /// cap sweep (fail-open wake of a stalled turn). Same discipline as `FlushTrigger` (fix 11). ★FIFO★: earlier parked items go first; a notice never overtakes mail.
    /// ★Queue key = the request sender's id (review fix 2, ADR-0309)★: a notice is `notified` and never fires again, so missing it strands it forever; an id key reaches the sender even
    /// after a rename. The ledger row's recipient name is the send-time name (`due.sender`). ★Doorbell always pressed after parking★: else a lost wakeup (an asleep sender's doorbell is
    /// empty-handed; the appearance flush opens it).
    // ADR-0103, ADR-0309
    fn deliver_notice(&self, due: &DueTimeout, body: &str) {
        // ★Narrowing the timeout↔reply race (review fix 5)★: a reply may have closed the contract between the due computation (lock released) and here — the sender would get the reply and
        //   a "no reply" notice. Re-check the ledger just before parking and drop the notice if closed. Residue: check and park are separate lock spans (microsecond window); tying them
        //   needs park inside the ledger (mixed concerns), so only narrowed.
        let closed_since_collection = {
            let st = self.state.lock().expect("messaging state poisoned");
            st.ledger
                .is_request_closed(&due.request_id, due.recipient_id)
        };
        if closed_since_collection {
            tracing::debug!(
                request_id = %due.request_id,
                "deadline notice cancelled — a reply arrived after computation and before parking, closing the contract (spec §3)"
            );
            return;
        }
        self.send_daemon_notice(due.sender_id, &due.sender, body);
    }

    /// Draws a notice id, parks the `<notice>` for `recipient` and rings its doorbell — shared by the overdue, late-verdict and remote-overdue notices.
    fn send_daemon_notice(&self, recipient: PeerId, recipient_name: &str, body: &str) {
        let drawn = self.draw_daemon_msg_id();
        if let Some(collided) = &drawn.collided {
            tracing::error!(
                collided = %collided,
                replacement = %drawn.id,
                still_colliding = drawn.still_colliding,
                "notice id collision — retrying once with a new id (ADR-0103 · a practically impossible path, so suspect the random/ledger wiring)"
            );
        }
        let notice_id = drawn.id;
        self.park_notice(&notice_id, recipient, recipient_name, body);
        self.request_flush(recipient);
    }

    /// ★Overdue report to a requester on another PC (ADR-0393 decision 6)★ — sent to the node recorded at request time, best-effort; never a local
    ///   notice (the requester is not here). Same "closed meanwhile" re-check as `deliver_notice`.
    // ADR-0393
    fn report_remote_overdue(&self, due: &DueTimeout, node: NodeRef) {
        let closed_since_collection = {
            let st = self.state.lock().expect("messaging state poisoned");
            st.ledger
                .is_request_closed(&due.request_id, due.recipient_id)
        };
        if closed_since_collection {
            return;
        }
        match &self.remote {
            Some(wiring) => wiring.port.send_reply_overdue(node, &due.request_id),
            None => tracing::warn!(
                request_id = %due.request_id,
                "a request from another PC is overdue but remote mail is not wired — the requester is not told (ADR-0393)"
            ),
        }
    }

    /// ★Draws the msg_id of a daemon-sent (`<notice>`) message with the same collision check as a send (round-final fix 2)★. A notice is checked too (load-bearing): msg_id is the history key
    /// `(msg_id, to)` and the observation correlation key, so an overlapping notice id makes `transition` pick up someone else's record. One lock-span check + **one** re-draw.
    /// ★If the second collides too, use it as is (notice-only rule)★: an agent send can bounce `IdCollision` and retry, but a notice cannot be discarded (a lost reply-contract notice leaves the
    /// contract half-done; same reason as the mailbox cap exception); history aliasing beats losing it. Exposed as `still_colliding` so the caller logs an error outside the lock.
    /// ★Known residual (not fixed)★: no reservation window between check and record — two sends drawing one fresh id can both pass (pre-v1; judgment: `handle_send` reservation comment).
    fn draw_daemon_msg_id(&self) -> DrawnMsgId {
        self.draw_daemon_msg_id_with(new_msg_id)
    }

    /// **Test seam** of the above — injects the id generator so the collision branch is asserted deterministically (1-in-36^8 otherwise). The closure runs under the lock, so it must be pure.
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

    /// ★`in_reply_to` = the structured value the caller passed (F1 review fix, load-bearing — security)★: deriving it by searching the rendered envelope for `in-reply-to="…"` let a sender
    /// forge the observation by mimicking that string in the body (`escape_xml_text` does not escape quotes — `DeliveryObservation.in_reply_to` in envelope.rs). `wrapped` is not parsed;
    /// the call site passes the `SendMeta.reply_to` it used to assemble the envelope (validated by ingress `validate_contract`).
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
            "messaging delivered (delivered, ADR-0103/0104)"
        );
    }

    /// Evidence that a failure is not swallowed as a success (ADR-0088).
    ///
    /// ★`in_reply_to` = a structured value (F1 — see `observe_success`)★ carried as is. A failed write still records what it replied to (a fact fixed in the send arguments), but
    /// `is_delivered()==false`, so a completeness-deciding consumer must not take it as evidence the reply went out.
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
            "messaging inject failed — switching to parking (lossless): {err}"
        );
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

    /// ★Contract-axis terminal vocabulary★: `awaiting_reply` | `replied` | `reply_failed` (`None` if not tracked). `replied` and `reply_failed` leave the open list, deadline sweep and limit
    /// count identically, so asserting "acceptance counts as completion, only confirmed unreachability is failure" (spec §3 item 7-④) or "delete cleanup does not revert `replied`" needs this reason.
    #[cfg(any(test, feature = "test-harness"))]
    pub fn contract_outcome_for_test(
        &self,
        msg_id: &str,
        recipient_id: PeerId,
    ) -> Option<&'static str> {
        let st = self.state.lock().expect("messaging state poisoned");
        st.ledger.contract_outcome_for_test(msg_id, recipient_id)
    }

    /// Is that contract key still in the tracking list (regardless of closed / retirement mark). Retirement is only a mark and removal happens at commit, so this observes the provisional
    /// window directly (`msg_id_in_use` stays true after it because of the 4096 history ring).
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
    /// The delivery-record recipient names (`to`) for a msg_id, oldest first. ★Not the terminal key★ (`(msg_id, to_id)`, ADR-0309): same-name twins share `to`, so duplicates here are not a
    /// key collision — check each row has exactly one terminal via `ledger_statuses`.
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

    /// The predicted value of `Mailbox::can_admit`. The envelope `to` freeze leans on it (cap result known before park), so the test pins that prediction and admission do not diverge.
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

    /// Sum of all recipient queue items — asserts a failure row created no ghost queue.
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

    /// The number of sessions (targets) currently held as being injected — target-axis counterpart of the value above. ★Failing to return to 0 is this subsystem's worst failure mode★: that
    /// session backs off from every later drain and its mail is cut off silently (`in_flight_targets` — no fail-open), so tests assert the reclaim, not just delivery.
    #[cfg(any(test, feature = "test-harness"))]
    pub fn in_flight_target_count(&self) -> usize {
        let st = self.state.lock().expect("messaging state poisoned");
        st.in_flight_targets.len()
    }

    /// **Deliberately corrupts** one parked item's payload — asserts via the real path that a broken item does not abort the flush batch and only degrades to a fallback envelope (C3 review fix 4).
    #[cfg(any(test, feature = "test-harness"))]
    pub fn corrupt_parked_payload_for_test(&self, recipient: PeerId, idx: usize) {
        let mut st = self.state.lock().expect("messaging state poisoned");
        st.mailbox
            .corrupt_envelope_for_test(recipient, idx, "CORRUPT-PAYLOAD".to_string());
    }

    // ── Ledger lookup surface (D · spec §6 `messages { id? }`) ──────────────────────────────────────

    /// ★`messages { id }` — the delivery ledger of that message (spec §6)★. `None` if absent (the entrance translates it to a bounce).
    /// ★Time is exposed as elapsed seconds (`age_secs`)★: ledger time is a monotonic `Instant`, and absolute time would need a `SystemTime` axis outside v1's in-memory scope (spec §5);
    /// elapsed from `now` is also more useful to the receiving LLM. ★`now` is an argument★ (injected clock, ledger.rs header — deterministic tests).
    /// ★An open contract whose history was pushed out answers with the contract view★: after B3 an unanswered contract outlives the history, and `eg_messages{}` would list the id as outstanding
    /// while `eg_messages{id}` said `MESSAGE_NOT_FOUND` (readers take that as "the list is lying"). So with no rows but a live contract it answers **0 rows + `awaiting_reply=true` + a truncation marker**.
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
                // No row left = the history was pushed out entirely, so truncation is certain.
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

    /// ★`eg_messages` with no argument — the caller's "outstanding" (spec §6)★: one list, oldest first, merging ① what I sent that is not yet plugged in (`from=me` + `pending`)
    /// ② the reply wait of a request I made (`from=me` + awaiting_reply) ③ requests I received and have not answered (`to=me` + awaiting_reply).
    /// ★The caller is one PeerId — all three branches compare ids (ADR-0309 decisions 4·5)★: rows and contracts hold ids, so same-name twins do not mix and a rename keeps waiting items,
    /// contracts and the duty to answer in the agent's list. `from`·`to` shown are the send-time names (decision 4). Epoch is not looked at (a restart is the same party — ADR-0007).
    /// ★Oldest first★: what aged longest should be handled first (same direction as the mailbox flush rule).
    // Review B1, ADR-0309
    pub fn open_items_for(&self, me_id: PeerId, now: Instant) -> Vec<OpenItemView> {
        let st = self.state.lock().expect("messaging state poisoned");
        let mut items: Vec<OpenItemView> = Vec::new();
        // ① My undelivered items — by sender id (`from_id`); a daemon notice row has a nil `from_id`, so no agent catches it.
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
        // ②·③ Unanswered contracts. The same contract can also appear in ① (a parked request) — not a duplicate but a different fact ("not plugged in" vs "reply not come"; different prescriptions).
        for r in st.ledger.open_requests() {
            // ★Look at the receive duty (③) first★: for a self-send both branches are true, and the fact to report is "you must answer" (just waiting would never close it).
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

    // ── Remote mail (ADR-0390 · ADR-0391 · ADR-0393) ──────────────────────────────────────

    /// ★The agent's answer for the remote rows of a send (ADR-0393 decision 3)★ — waits (blocking, on the messaging lock's condition variable) until
    ///   every remote row of `msg_id` has a verdict or `deadline` passes, then **seals** the answer under the lock and writes each remote row into
    /// `results` at its tag (the row index `handle_send` returned it at). A row still without a verdict is reported `pending` + unconfirmed; a
    /// verdict after the seal goes to the sender as one notice instead. A send with no remote row returns at once.
    /// ★Caller duty★: call once per send that `handle_send` answered, with the deadline counted from the control request's arrival (the host's
    ///   agent-facing wait); not under the messaging lock. Local rows are not touched.
    // ADR-0393
    pub fn await_remote_answer(
        &self,
        msg_id: &str,
        results: &mut [RecipientResult],
        deadline: Instant,
    ) {
        let mut st = self.state.lock().expect("messaging state poisoned");
        loop {
            match st.ledger.remote_rows_settled(msg_id) {
                None => return,
                Some(true) => break,
                Some(false) => {}
            }
            let now = Instant::now();
            if now >= deadline {
                break;
            }
            #[cfg(test)]
            if let Some(hook) = self.wait_hook.get() {
                hook();
            }
            st = self
                .remote_settled
                .wait_timeout(st, deadline - now)
                .expect("messaging state poisoned")
                .0;
        }
        let rows = st.ledger.seal_remote_answer(msg_id);
        drop(st);
        for row in rows {
            if let Some(slot) = results.get_mut(row.tag as usize) {
                *slot = remote_answer_row(&row);
            }
        }
    }

    /// ★The verdict entry — the only path out of `RemotePending` (ADR-0393 decisions 1 · 3 · 5)★. Applies each row's outcome by (mail id, tag) for a
    ///   mail sent to `node`; a reply to a remote requester closes its contract here by ADR-0116 decision 2's rule (delivered or held → replied,
    /// `RECIPIENT_NOT_FOUND` → reply_failed, anything else → stays open). Rows already in the agent's answer get one `<notice>` with the outcome and
    /// code only — a peer's reason never enters the daemon's notice voice. A second verdict for a row, an unknown tag or another node's row is
    /// ignored (logged).
    /// ★One final call per mail★ (`msg_id` to `node`): every tag of that mail still unsettled after `rows` is settled `Unconfirmed` here, so a
    ///   verdict that leaves a tag out cannot strand it.
    // ADR-0393
    pub fn handle_remote_verdict(&self, msg_id: &str, node: NodeRef, rows: &[RemoteRowVerdict]) {
        let now = Instant::now();
        let mut acc = VerdictEffects::default();
        {
            let mut st = self.state.lock().expect("messaging state poisoned");
            for v in rows {
                settle_one(
                    &mut st.ledger,
                    msg_id,
                    node,
                    v.tag,
                    &v.outcome,
                    now,
                    &mut acc,
                );
            }
            for tag in st.ledger.unsettled_remote_tags(msg_id, node) {
                settle_one(
                    &mut st.ledger,
                    msg_id,
                    node,
                    tag,
                    &RemoteOutcome::Unconfirmed,
                    now,
                    &mut acc,
                );
            }
        }
        self.remote_settled.notify_all();

        if !acc.second_call.is_empty() {
            tracing::warn!(
                msg_id,
                tags = ?acc.second_call,
                "a second verdict call for one mail — those tags were already settled and this outcome is dropped; the host must make one final call per mail (ADR-0393)"
            );
        }
        for (tag, e) in acc.ignored {
            tracing::debug!(msg_id, tag, error = ?e, "remote verdict row ignored — no such remote row of this PC's mail, or already settled (ADR-0393)");
        }
        for (in_reply_to, replier, close) in acc.closes {
            match close {
                ReplyClose::Replied(outcome) => log_reply_close(outcome, &in_reply_to, &replier),
                ReplyClose::Failed(outcome) => log_reply_fail(outcome, &in_reply_to, &replier),
            }
        }
        if let Some((sender_id, sender)) = acc.notice_to {
            // ★`[engram]` origin label, outcome + code only★ (same lane and shape as the overdue notice — ADR-0104).
            let body = format!("[engram] mail {msg_id}: {}", acc.late.join("; "));
            self.send_daemon_notice(sender_id, &sender, &body);
        }
    }

    /// ★The remote entrance (ADR-0390)★ — mail from an agent on PC `node` (this PC's name for it: `pc_name`), decoded by the host. Order: validation
    ///   (`REMOTE_REFUSED`, `validate_inbound`: the mail id, 1–`MAX_REMOTE_ROWS` rows with unique tags, no empty, oversized or `@` token, a reply names
    /// one row, the sender name within the remote name rule, no nil id, and the pure contract rules on `meta`) → a sender id that is also local is
    /// refused (copied data folder) → backstop (`REMOTE_LIMIT`: body over the cap, or over the per-PC mails per minute) → the send pipeline with
    /// `ResolveScope::LocalOnly`, `Entrance::Remote`, the sender shown as `"<name>@<pc_name>"` and its node from the link. Per-row results answer
    /// every tag. An accepted reply to one of this PC's requests (delivered, parked, or its requester gone — the rule the replier's PC closes its
    /// contract by) also marks that request's rows to `node` for the replier as answered (no overdue report names them); a refused one does not.
    /// ★Caller duty★: run this PC's own contract checks on `meta` too (reply_by notation, envelope support) — the kernel re-checks only the pure
    ///   rules, as defence in depth. Drop a mail whose transport deadline (`ledger::REMOTE_TRANSPORT_DEADLINE`) has passed instead of running it
    /// late — the overdue grace on the sending PC relies on it. `now` = the arrival instant (backstop clock).
    // ADR-0390, ADR-0393
    pub fn handle_remote_mail(
        &self,
        node: NodeRef,
        pc_name: &str,
        mail: &InboundMail,
        meta: &SendMeta,
        now: Instant,
    ) -> Result<Vec<RemoteRowResult>, RemoteRefusal> {
        let Some(wiring) = &self.remote else {
            return Err(remote_refused("remote mail is not enabled on this PC"));
        };
        validate_inbound(mail, meta)?;
        if wiring.port.is_local_agent_id(mail.from_agent_id) {
            return Err(remote_refused(
                "this agent id also exists on this PC — copied data folder?",
            ));
        }
        if mail.body.len() > wiring.limits.max_body_bytes {
            return Err(RemoteRefusal {
                code: FailCode::RemoteLimit,
                reason: format!("body over the {}-byte limit", wiring.limits.max_body_bytes),
            });
        }
        {
            let mut st = self.state.lock().expect("messaging state poisoned");
            let window = st.backstop.entry(node).or_default();
            while window
                .front()
                .is_some_and(|t| now.saturating_duration_since(*t) >= BACKSTOP_WINDOW)
            {
                window.pop_front();
            }
            if window.len() >= wiring.limits.mails_per_minute {
                return Err(RemoteRefusal {
                    code: FailCode::RemoteLimit,
                    reason: format!(
                        "over {} mails per minute from this PC",
                        wiring.limits.mails_per_minute
                    ),
                });
            }
            window.push_back(now);
        }

        let from = SenderIdentity {
            peer_id: mail.from_agent_id,
            epoch: 0,
            node: Some(node),
        };
        let sender_name = format!("{}@{pc_name}", mail.from_name);
        let tokens: Vec<String> = mail.rows.iter().map(|r| r.to.clone()).collect();
        let outcome = self
            .send_pipeline(
                &mail.id,
                from,
                &sender_name,
                &tokens,
                &mail.body,
                Entrance::Remote,
                meta,
                ResolveScope::LocalOnly,
            )
            .map_err(|e| match e {
                SendReject::IdCollision => {
                    remote_refused("this mail id is already used on this PC — send it again")
                }
                // Unreachable after validation (no `@` token) — still a refusal, never a panic.
                SendReject::GroupNotFound { .. } | SendReject::GroupEmpty => {
                    remote_refused("group addresses are local only")
                }
            })?;
        // ADR-0393 decision 5: the replier's PC closes its contract by ADR-0116 decision 2's rule on this verdict — accepted → replied, requester
        //   gone → reply_failed, else open (it may still report overdue). Mark the rows by the same rule.
        if let (Some(request_id), false) = (
            &meta.reply_to,
            reply_disposition(&outcome.results) == ReplyDisposition::NoOp,
        ) {
            let mut st = self.state.lock().expect("messaging state poisoned");
            st.ledger
                .mark_remote_replied(request_id, node, mail.from_agent_id, &mail.from_name);
        }

        Ok(mail
            .rows
            .iter()
            .enumerate()
            .map(|(k, row)| {
                let at = outcome.token_rows.get(k).copied().flatten();
                let result = at.and_then(|i| outcome.results.get(i));
                let id = at.and_then(|i| outcome.row_ids.get(i).copied().flatten());
                let outcome = match (result, id) {
                    (Some(r), Some(recipient_id)) if r.status == SendStatus::Delivered => {
                        InboundOutcome::Delivered { recipient_id }
                    }
                    (Some(r), Some(recipient_id)) if r.status == SendStatus::Pending => {
                        InboundOutcome::Held { recipient_id }
                    }
                    (Some(r), _) if r.code == Some(FailCode::RecipientNotFound) => {
                        InboundOutcome::UnknownRecipient
                    }
                    (Some(r), _) => {
                        InboundOutcome::Refused(r.code.unwrap_or(FailCode::RemoteRefused))
                    }
                    (None, _) => InboundOutcome::Refused(FailCode::RemoteRefused),
                };
                RemoteRowResult {
                    tag: row.tag,
                    outcome,
                }
            })
            .collect())
    }

    /// ★`ReplyOverdue` from PC `node` for this PC's request `request_id` (ADR-0393 decision 6)★ — covers that request's deadline-bearing rows to `node`
    ///   that are not failed, not answered by a reply from their recipient, and not reported yet; refused before the deadline on this PC's clock
    /// (`now`). One notice per accepted report, from this daemon's own template: one such row → it names that agent; several → it names the PC and
    /// lists them (the report carries no recipient). `false` = not accepted. Never peer text.
    // ADR-0393
    pub fn handle_reply_overdue(&self, node: NodeRef, request_id: &str, now: Instant) -> bool {
        let report = {
            let mut st = self.state.lock().expect("messaging state poisoned");
            st.ledger.accept_reply_overdue(request_id, node, now)
        };
        let report = match report {
            Ok(r) => r,
            Err(why) => {
                tracing::debug!(
                    request_id,
                    refusal = ?why,
                    "ReplyOverdue not accepted — no unanswered, unreported request row of this PC to that node, or before its deadline (ADR-0393)"
                );
                return false;
            }
        };
        let who = match report.agents.as_slice() {
            [one] => format!("no reply from {one}"),
            many => format!(
                "an agent on {} has not replied — not answered yet: {}",
                report.pc_name,
                many.join(", ")
            ),
        };
        let body = format!(
            "[engram] request {request_id} deadline ({}) exceeded — {who}",
            report.reply_by_raw
        );
        self.send_daemon_notice(report.from_id, &report.from, &body);
        true
    }
}

/// The per-PC backstop window (mails per minute).
// ADR-0390
const BACKSTOP_WINDOW: Duration = Duration::from_secs(60);

/// What the send pipeline knows beyond the response rows — the remote entrance maps its rows back through it (the per-row kernel result:
///   each input token's row index and the row's resolved id).
// ADR-0393
struct SendOutcome {
    results: Vec<RecipientResult>,
    /// Per input token: its row (`None` for an `@` token).
    token_rows: Vec<Option<usize>>,
    /// Per row: the resolved agent id (`None` = unresolved or remote).
    row_ids: Vec<Option<PeerId>>,
}

enum ReplyClose {
    Replied(ReplyOutcome),
    Failed(ReplyFailOutcome),
}

/// What one verdict call collected under the lock, for logging and the late notice outside it.
#[derive(Default)]
struct VerdictEffects {
    late: Vec<String>,
    notice_to: Option<(PeerId, String)>,
    closes: Vec<(String, String, ReplyClose)>,
    ignored: Vec<(u32, RemoteSettleError)>,
    /// Tags settled by this call — an `AlreadySettled` tag outside it was settled by an earlier call for the same mail.
    settled_here: Vec<u32>,
    /// Tags an earlier call had already settled (one final call per mail is the contract).
    second_call: Vec<u32>,
}

/// Settles one remote row and applies the reply close of a reply row (ADR-0116 decision 2's rule at verdict time) — under the messaging lock.
// ADR-0393
fn settle_one(
    ledger: &mut Ledger,
    msg_id: &str,
    node: NodeRef,
    tag: u32,
    outcome: &RemoteOutcome,
    now: Instant,
    acc: &mut VerdictEffects,
) {
    let settlement = settlement_of(outcome);
    let row = match ledger.settle_remote(msg_id, node, tag, settlement.clone(), now) {
        Ok(row) => row,
        Err(RemoteSettleError::AlreadySettled) if !acc.settled_here.contains(&tag) => {
            acc.second_call.push(tag);
            return;
        }
        Err(e) => {
            acc.ignored.push((tag, e));
            return;
        }
    };
    acc.settled_here.push(tag);
    if let Some(in_reply_to) = &row.closes {
        let close = match outcome {
            RemoteOutcome::Delivered { .. } | RemoteOutcome::Held { .. } => Some(
                ReplyClose::Replied(ledger.close_on_reply(in_reply_to, row.from_id, now)),
            ),
            RemoteOutcome::UnknownRecipient => Some(ReplyClose::Failed(
                ledger.fail_on_undeliverable_reply(in_reply_to, row.from_id),
            )),
            _ => None,
        };
        if let Some(close) = close {
            acc.closes
                .push((in_reply_to.clone(), row.from.clone(), close));
        }
    }
    if row.answered {
        acc.late.push(format!(
            "{} — {}",
            row.to,
            notice_phrase(&settlement, &row.pc_name)
        ));
        acc.notice_to = Some((row.from_id, row.from.clone()));
    }
}

fn log_remote_capped(count: usize) {
    if count > 0 {
        tracing::warn!(
            count,
            "remote store over its cap (MAX_REMOTE_ENTRIES) — dropped the oldest entries; their late verdicts or overdue reports will be ignored (ADR-0393)"
        );
    }
}

/// ★A mail id another PC may use★ — 1–`MAX_REMOTE_MSG_ID_BYTES` bytes of `[A-Za-z0-9_-]` (the `new_msg_id` family, `m-` + base36). It is logged and
///   stored per row, so its size and characters are bounded.
// ADR-0390
pub fn remote_msg_id_ok(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_REMOTE_MSG_ID_BYTES
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

pub const MAX_REMOTE_MSG_ID_BYTES: usize = 64;

/// The longest reply deadline accepted from another PC — the same 30 days as this PC's own entrance (`created_at + reply_by` must not overflow).
// ADR-0390
pub const MAX_REMOTE_REPLY_BY: Duration = Duration::from_secs(30 * 24 * 60 * 60);

fn remote_refused(reason: &str) -> RemoteRefusal {
    RemoteRefusal {
        code: FailCode::RemoteRefused,
        reason: reason.to_string(),
    }
}

/// Validation of an inbound mail before anything is recorded (`handle_remote_mail` doc lists the rules).
// ADR-0390
fn validate_inbound(mail: &InboundMail, meta: &SendMeta) -> Result<(), RemoteRefusal> {
    if !remote_msg_id_ok(&mail.id) {
        return Err(remote_refused(
            "mail id is empty, over 64 bytes, or not made of [A-Za-z0-9_-]",
        ));
    }
    if mail.rows.is_empty() {
        return Err(remote_refused("no recipients"));
    }
    if mail.rows.len() > MAX_REMOTE_ROWS {
        return Err(remote_refused("more than 64 recipients in one mail"));
    }
    let mut tags = std::collections::HashSet::new();
    for row in &mail.rows {
        if !tags.insert(row.tag) {
            return Err(remote_refused("duplicate row tag"));
        }
        let to = row.to.trim();
        if to.is_empty() {
            return Err(remote_refused("empty recipient"));
        }
        if row.to.len() > REMOTE_NAME_MAX_BYTES {
            return Err(remote_refused("recipient over 256 bytes"));
        }
        if to.starts_with('@') {
            return Err(remote_refused(
                "group addresses (@all, @here) reach only the sender's own PC",
            ));
        }
    }
    if meta.reply_to.is_some() && mail.rows.len() != 1 {
        return Err(remote_refused("a reply names exactly one recipient"));
    }
    if !remote_name_ok(&mail.from_name) {
        return Err(remote_refused(
            "sender name is empty, over 256 bytes, or has control characters",
        ));
    }
    if mail.from_agent_id.is_nil() {
        return Err(remote_refused("nil sender id"));
    }
    validate_inbound_contract(meta)
}

/// ★The pure contract rules, re-checked on mail from another PC (defence in depth)★ — the host runs its own entrance checks too, but no peer
///   value may reach the pipeline's debug asserts or an unbounded `created_at + reply_by`.
// ADR-0390
fn validate_inbound_contract(meta: &SendMeta) -> Result<(), RemoteRefusal> {
    if meta.request && meta.reply_to.is_some() {
        return Err(remote_refused("request and reply_to together"));
    }
    if meta.reply_by.is_some() != meta.reply_by_raw.is_some() {
        return Err(remote_refused("reply_by without its notation"));
    }
    if meta.reply_by.is_some() && !meta.request {
        return Err(remote_refused("reply_by without request"));
    }
    if meta.reply_by.is_some_and(|d| d > MAX_REMOTE_REPLY_BY) {
        return Err(remote_refused("reply_by over 30 days"));
    }
    if meta.reply_by_raw.as_deref().is_some_and(|raw| {
        raw.is_empty() || raw.len() > 16 || !raw.bytes().all(|b| b.is_ascii_alphanumeric())
    }) {
        return Err(remote_refused("reply_by notation is not a short duration"));
    }
    if meta
        .reply_to
        .as_deref()
        .is_some_and(|r| !remote_msg_id_ok(r))
    {
        return Err(remote_refused("reply_to is not a mail id"));
    }
    if meta.to_attr.is_some() {
        return Err(remote_refused(
            "to_attr is set by the receiver, not the sender",
        ));
    }
    Ok(())
}

/// A verdict outcome → the ledger settlement (codes mapped onto the closed set, the peer's reason capped and escaped).
// ADR-0390, ADR-0393
fn settlement_of(outcome: &RemoteOutcome) -> RemoteSettlement {
    match outcome {
        RemoteOutcome::Delivered { recipient_id } => RemoteSettlement::Delivered {
            to_id: *recipient_id,
        },
        RemoteOutcome::Held { recipient_id } => RemoteSettlement::Held {
            to_id: *recipient_id,
        },
        RemoteOutcome::Refused { code, reason } => RemoteSettlement::Failed {
            code: FailCode::from_remote_code(code.as_deref()).as_str(),
            reason: reason.as_deref().map(clip_reason),
        },
        RemoteOutcome::UnknownRecipient => RemoteSettlement::Failed {
            code: FailCode::RecipientNotFound.as_str(),
            reason: None,
        },
        RemoteOutcome::NotSent(code) => RemoteSettlement::Failed {
            code: code.as_str(),
            reason: None,
        },
        RemoteOutcome::Unconfirmed => RemoteSettlement::Unconfirmed,
    }
}

/// ★Peer text shown to the sending agent★ — control characters escaped, then cut to `REMOTE_REASON_MAX_BYTES` on a character boundary.
// ADR-0390
fn clip_reason(raw: &str) -> String {
    let mut out = String::new();
    for c in raw.chars() {
        let piece: String = if c.is_control() {
            c.escape_default().collect()
        } else {
            c.to_string()
        };
        if out.len() + piece.len() > REMOTE_REASON_MAX_BYTES {
            break;
        }
        out.push_str(&piece);
    }
    out
}

/// A late verdict as one notice line — outcome and code only (ADR-0393 decision 3).
fn notice_phrase(settlement: &RemoteSettlement, pc: &str) -> String {
    match settlement {
        RemoteSettlement::Delivered { .. } => "delivered".to_string(),
        RemoteSettlement::Held { .. } => {
            format!("held on {pc} (recipient asleep or busy)")
        }
        RemoteSettlement::Failed { code, .. } => format!("failed ({code})"),
        RemoteSettlement::Unconfirmed => {
            format!("outcome unknown — no answer from {pc}; it may or may not have arrived")
        }
    }
}

/// One sealed remote row → the agent's answer row: delivered → `delivered`, held → `pending`, refused / not found / not sent → `failed` + code,
///   unconfirmed → `pending`, never `failed` (ADR-0393 decision 2).
// ADR-0393
fn remote_answer_row(row: &RemoteRowView) -> RecipientResult {
    let pending = |hint: String| RecipientResult {
        to: row.to.clone(),
        status: SendStatus::Pending,
        code: None,
        hint: Some(hint),
    };
    match row.status {
        DeliveryStatus::Delivered => RecipientResult {
            to: row.to.clone(),
            status: SendStatus::Delivered,
            code: None,
            hint: None,
        },
        DeliveryStatus::RemoteHeld => pending(held_hint(&row.to, &row.pc_name)),
        DeliveryStatus::Failed => {
            let code = row
                .fail_code
                .and_then(FailCode::from_wire)
                .unwrap_or(FailCode::RemoteRefused);
            RecipientResult {
                to: row.to.clone(),
                status: SendStatus::Failed,
                code: Some(code),
                hint: Some(remote_failure_hint(
                    &row.to,
                    &row.pc_name,
                    code,
                    row.reason.as_deref(),
                )),
            }
        }
        _ => pending(unconfirmed_hint(&row.to, &row.pc_name)),
    }
}

/// The elapsed seconds between two `Instant`s (never negative; 0 if `now` is in the past — a test pushing `now` by hand would otherwise panic).
fn secs_since(then: Instant, now: Instant) -> u64 {
    now.saturating_duration_since(then).as_secs()
}

/// Ledger state → wire vocabulary (spec §5). Must be **the same vocabulary** as the send response `results[].status`, so the sending LLM reads both by one rule.
fn status_label(s: DeliveryStatus) -> &'static str {
    match s {
        DeliveryStatus::Pending => "pending",
        DeliveryStatus::Delivered => "delivered",
        DeliveryStatus::Replied => "replied",
        DeliveryStatus::Expired => "expired",
        DeliveryStatus::Skipped => "skipped",
        DeliveryStatus::Failed => "failed",
        // ADR-0393: held on the other PC and unconfirmed both read `pending` (the three-word vocabulary stays).
        DeliveryStatus::RemotePending | DeliveryStatus::RemoteHeld => "pending",
    }
}

/// The `messages { id }` lookup result (the entrance converts it to JSON — canonical shape: ingress `handle_messages` comment).
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

/// The lookup row's code → hint wording (**single point**). ★The wording is not stored★: a string per record grows the history ring (4096) without basis, and old stored wording would
/// coexist with fixed wording; the code is the stable contract, the wording is expression made at lookup time.
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
    /// I sent it and it is not yet plugged into the recipient (parked, busy wait). To do: wait (or check the peer's state).
    OutboundPending,
    /// Waiting for the reply to a request I made. To do: wait.
    AwaitingTheirReply,
    /// I received a request and have not yet answered it. **To do: reply now.**
    ReplyOwedByMe,
}

impl Direction {
    /// The wire token (stable contract — the receiving LLM branches on it).
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
    /// For a request, the deadline notation the sender wrote (`"10m"`). None for a notification or a request with no deadline.
    pub reply_by: Option<String>,
    /// Has the overdue notice already gone out (the contract is still open — ledger `OpenRequestView` comment).
    pub timed_out: bool,
}

/// The **decision result** for one recipient (decided inside the lock, carried out outside). ★Why split decision and execution★: enqueue and the ledger finish atomically in the lock
/// (no "looks-empty" queue window — `drain_queue`), and injection must never run in the lock (a port call). ★Only two branches (ADR-0125)★: identity holds → unconditional enqueue
/// (`Park`), else a failure row — the "inject right now" branch (old `Deliver`) left with the direct-send shortcut; reviving it skips enqueue and loosens order.
enum RecipientPlan<'a> {
    Park {
        /// Queue key = the receiving agent id (ADR-0309 — the session id if alive, the profile id if asleep).
        recipient_id: PeerId,
        /// ★`false` = asleep (ADR-0116 decision 1)★: no live entity to drain to; delivery happens in the appearance drain the roster diff opens when that id returns.
        live: bool,
        contract: PendingContract<'a>,
    },
    /// Failure for this recipient only (ledger terminal recorded) — nothing to do outside the lock; the others go through.
    Failed { code: FailCode, hint: String },
    /// Recorded `RemotePending` and handed to the remote port — its answer row comes from `await_remote_answer` (ADR-0393).
    Remote { pc_name: String },
}

/// ★What one drain did (ADR-0125)★ — how a synchronous drain's caller (`handle_send`) reads **the outcome of its own letter**; an async occasion (`flush_for`) discards it.
/// ★Why "what was injected"★: response `delivered` means "this call's drain actually injected it" (spec §6); inferring from queue/ledger later would credit another drain's injection
/// (false `delivered`). ★`retreated` is a separate axis★: "cannot know" (spec §6 ㉯) — the winner's batch may have sent it — not one of the three reasons (㉮).
/// ★The reason fields are the caller's share (ADR-0309)★: a drain's recipient is one queue id, so the drain's outcome is that recipient's.
// ADR-0142
#[derive(Debug, Default)]
struct DrainReport {
    injected: Vec<String>,
    /// **This drain backed off** — both axes (step 0 queue axis, target axis). ★The caller need not press its own doorbell★: on backing off both axes register the queue id as waiter, which is
    /// exactly what `ring_or_defer` would press (`handle_send` drains `drain_queue(recipient_id, _)`), so the winner's settlement reissues the identical press. ★Exception★: if the winner unwinds
    /// mid-injection the reissue does not happen — the backed-off side waits for the next doorbell or TTL (see `deferred_by_target` residue; production's hook ends it — ADR-0292).
    retreated: bool,
    /// If deferred by the idle gate, `Some(busy reason)` (turn-signal backend busy — in a turn, awaiting user input, stopped after an error), else `None`; `pending_hint` picks wording from it (ADR-0231).
    gated: Option<BusyReason>,
    /// ★Injection failure reason★ — basis for the no-own-doorbell decision and the response reason wording.
    caller_inject_error: Option<String>,
}

/// ★The contract reservation's **single-exit RAII guard** (H1; guarantee owner of ADR-0108 decision 3)★ — while alive the contract is **provisional** and must pass through `commit` or
/// `rollback` to disappear.
///
/// ★Commit is deferred until after the outcome (A2/A3)★:
///   - **A2 (early loss of the victim)**: commit physically removes the marked victim. Committing at contract open (pass A) and then having the same pass's cap gate drop this recipient
///     to a failure row would erase someone else's unanswered contract for nobody's seat (ADR-0108 round-3 failure). ADR-0125 narrowed the window (outcome fixed inside the enqueue
///     lock) but the pass-A failure branch remains; moving commit back regresses it.
///   - **A3 (self-erosion in one send)**: a provisional contract is excluded from victim candidates (`!r.provisional` in `ledger::open_request`), so deferring commit makes it impossible for
///     recipient 1's contract to be eaten by recipient 2 of the same send.
///
/// ★Why a **type** (RAII) (H1)★: an ordinary `Option<Option<_>>` let a forgotten branch pass neither compiler nor tests (a prober changed the re-park branch to `let _ = contract.take();`
/// and the suite stayed green), yet forgetting is permanent: ① the contract stays `provisional` and `due_timeouts` skips it forever (silently half-done) ② the marked victim stays
/// `pending_retirement`, drops out of `occupies_slot()`, the cap denominator shrinks permanently and `requests` grows without bound. So dropping unsettled `debug_assert`s (red in tests/debug)
/// and in release attempts a rollback with an error log. ★In production a panic on this path is uncontained and the hook ends it (ADR-0292)★, so the motive is not panic recovery but ① the
/// forgotten-branch class ② unwinding in hookless test builds.
/// ★Drop is the fast path, not the guarantee (F1, load-bearing)★: settlement happens **holding the lock** (`commit`/`rollback` take `&mut MessagingState`), so `lock()` in Drop would
/// self-deadlock; it uses `try_lock` and skips the rollback on failure (contention, same thread holding, poison).
/// ★Guarantee = the periodic sweep reclaiming **abandoned** reservations (`ledger::reclaim_abandoned_reservations`)★ — a lock-owning maintenance point rolls back reservations whose owner is gone,
/// so the guarantee does not depend on Drop timing.
/// ★Since ADR-0125 the window sits inside one enqueue-lock span★, so `try_lock` failure narrowed to unwinding-while-the-same-thread-holds-the-lock (where the sweep's poison `expect` cannot
/// pass either, so the sweep fallback does not help). ★Do not read it as "no out-of-lock injection, machinery obsolete"★ — the guard blocks an uncaught settlement omission (forgotten branch),
/// independent of window length. Removal reopens ADR-0108 and brings back the `STALE_RESERVATION_AFTER` class.
/// ★The reclaim looks at this guard's liveness (R1)★: rationale = `ledger::ReservationLiveness` header; deleting the field or bypassing with `mem::forget` breaks it.
/// ★During unwinding `debug_assert` is skipped★: Drop also runs while panicking, and a second panic aborts (test process dies, cause unseen); if `thread::panicking()` it only records and leaves reclaim to the sweep.
// ADR-0108
struct Reservation<'a> {
    svc: &'a MessagingService,
    msg_id: String,
    // ADR-0309
    recipient_id: PeerId,
    /// The victim **marked pending retirement** by limit pressure (if any). Commit = physical removal + metering, rollback = unmark.
    retired: Option<RetiredContract>,
    /// Settlement-complete marker — set by `commit`/`rollback`; Drop intervenes only when false.
    settled: bool,
    /// ★Owner liveness token (R1)★ — while alive the sweep does not reclaim this reservation. Only **held**, never read (the ledger observes it from the weak side).
    _liveness: ReservationLiveness,
}

/// ★Bundles guard creation + liveness-token attachment into one verb (R1, load-bearing)★: done separately, a "made the guard, forgot to attach" branch makes the reservation look abandoned to
/// the sweep while the owner works (the round-4 failure chain). Do not add a path calling `Reservation::new` directly. ★Call site = the same lock span as `open_request`★ (takes `st`).
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
    /// ★Caller duty (R1)★: attach `liveness.watch()` to the ledger entry in the same lock span (`Ledger::attach_reservation_liveness`).
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
        // ★Metering reports only what happened (R2)★: the planned victim may have vanished (closed by a reply with its history row pushed out — `purge_finished_without_history`; known residual of
        //   `ledger::rollback_open`). Reporting the plan as fact pollutes the only evidence of retirement (ADR-0108 decision 2), so count it only when the ledger confirms removal; a mismatch goes to an anomaly record.
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
                    "rolled back an unsettled contract reservation in Drop — the call path is missing a commit/rollback (H1)"
                );
            }
            Err(_) => tracing::error!(
                msg_id = %self.msg_id,
                recipient_id = %self.recipient_id,
                "could not roll back an unsettled contract reservation (lock held or poison) — the provisional contract and the retirement mark remain (H1)"
            ),
        }
        // ★Blow up **after** the rollback★: the other order lets the panic block recovery, so a test could not observe "state intact when settlement is forgotten" (release has no debug_assert, only recovery).
        debug_assert!(
            std::thread::panicking(),
            "a reply contract reservation vanished without settlement — it must go through either commit or rollback (H1)"
        );
    }
}

/// The core of reservation cancellation (shared by the guard's `rollback` and Drop so the rule does not diverge).
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

/// The resolution result for one recipient — resolved only once from the send-time snapshot (ADR-0111 decision 2); re-resolving by name in the decision step would demote an id target to
/// name resolution and diverge within one snapshot.
// ADR-0309
enum Resolution {
    Live(LiveAgent),
    /// One asleep profile — parking (spec §5 branch 3, ADR-0116 decision 1).
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
    /// ★An agent on another PC★ — `name` = what goes to that PC (the token with `@pc` stripped, or the id). `link` decides at admission
    ///   whether it is sent or fails `PEER_OFFLINE`.
    // ADR-0391
    Remote {
        node: NodeRef,
        pc_name: String,
        name: String,
        link: PcLink,
    },
    /// ★Not local, and on two or more PCs★ (a bare name in 2+ copies, or an id listed by 2+ copies) → `RECIPIENT_AMBIGUOUS` with the
    ///   `name@pc` candidates, sorted.
    // ADR-0391
    RemoteAmbiguous {
        candidates: Vec<String>,
    },
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
    /// The notation as the sender wrote it, for the response row (trimmed only, ADR-0101); an `@` expansion result is the registry name.
    display: String,
    to: Resolution,
}

impl ResolvedRecipient {
    /// The receiving id — session id if alive, profile id if asleep. `None` if unresolved or on another PC (its id comes with the verdict).
    fn id(&self) -> Option<PeerId> {
        match &self.to {
            Resolution::Live(a) => Some(a.id),
            Resolution::Dormant { id, .. } => Some(*id),
            Resolution::Ambiguous { .. }
            | Resolution::NotFound
            | Resolution::Remote { .. }
            | Resolution::RemoteAmbiguous { .. } => None,
        }
    }

    /// The name for the ledger row, parked item and envelope `to` — the registry name if resolved locally (kept even when targeted by id), else the notation.
    fn name(&self) -> &str {
        match &self.to {
            Resolution::Live(a) => &a.name,
            Resolution::Dormant { name, .. } => name,
            Resolution::Ambiguous { .. }
            | Resolution::NotFound
            | Resolution::Remote { .. }
            | Resolution::RemoteAmbiguous { .. } => &self.display,
        }
    }

    /// Is it the same row — the same id if resolved, the same (PC, stripped token) if remote (ADR-0393: one row per PC and token), the same notation if
    ///   unresolved (`push_recipient`).
    fn same_row(&self, other: &ResolvedRecipient) -> bool {
        match (&self.to, &other.to) {
            (
                Resolution::Remote {
                    node: a, name: an, ..
                },
                Resolution::Remote {
                    node: b, name: bn, ..
                },
            ) => a == b && an == bn,
            (Resolution::Remote { .. }, _) | (_, Resolution::Remote { .. }) => false,
            _ => match (self.id(), other.id()) {
                (Some(a), Some(b)) => a == b,
                (None, None) => self.display == other.display,
                _ => false,
            },
        }
    }
}

/// One input token (input order preserved) — the order axis of envelope `to` assembly and the source of the rows it contributed (a slot in `Addressing.recipients`).
/// ★Why it holds "the rows it contributed" (A5 regression, load-bearing)★: deciding `to` inclusion by "did the token create a row after dedup" drops `@all` wholesale when earlier explicit targets absorb
/// its whole expansion (`["bob","carol","@all"]` → `to="bob,carol"`). spec §1: the value lists the tokens the sender wrote and only **failed** tokens drop; absorption is not failure. So each token records
/// all rows it resolved, and inclusion = any of them was decided accepted.
enum AddressToken {
    Name { row: usize },
    Group { label: String, rows: Vec<usize> },
}

struct Addressing {
    recipients: Vec<ResolvedRecipient>,
    tokens: Vec<AddressToken>,
}

/// ★Recipient resolution (spec §5 order; pure — looks only at the entrance snapshot)★. Order: ① trim (comma splitting is CLI-entrance only) → ② expand `@` elements (excluding the sender) →
/// ③ join with names → ④ resolve per element (expanded ones are already ids) → ⑤ dedup (fold key = resolved id, or the notation if unresolved; one recipient = one delivery = one row). The 3-branch
/// decision is the caller's (`admission`).
/// ★Row order is deterministic (pinned by tests — spec §5)★: explicit tokens in written order, then `@`expansion results in name lexicographic order; dedup keeps the first appearance (explicit
/// precede expansion, so an overlapping recipient's position follows its explicit one).
/// ★`@`address error = whole send-level bounce (ADR-0114 decision 3)★: `@name` other than `@all` or a rule violation → `GROUP_NOT_FOUND`; an empty final set → `GROUP_EMPTY` (judged on
/// expansion ∪ explicit, not per element).
/// ★Sender exclusion in expansion (spec §4 — the spec is canonical, ADR-0111 decision 4 has no such wording)★: omit it and the sender receives its own broadcast. A direct-target self-send is delivered
/// (exclusion applies only to expansion), so `["@all", "<own-name>"]` sends 1 row to oneself.
// ADR-0111, ADR-0114, ADR-0121
fn resolve_addressing(
    groups: &dyn GroupSource,
    to: &[String],
    from: SenderIdentity,
    sources: &AddressingSources,
    scope: ResolveScope,
) -> Result<Addressing, SendReject> {
    // ★Which vocabulary reads which pool is decided by the source (`groups::MemberPools`)★ — here only build the pool and remove the sender (ADR-0121 decision 1).
    // ★The sender is removed by id only (ADR-0309, TRD S29 §4 X3)★: by name would also drop another agent sharing the name (live twin, asleep profile), and an asleep same-name could
    //   otherwise resolve to the sender's own live session and receive its broadcast.
    // ★A live session with no turn signal enters the live pool★ (roster has no capability condition — ADR-0116 decision 7): injected immediately, no gate.
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
                    to: resolve_token(token, sources, scope),
                },
            );
            tokens.push(AddressToken::Name { row });
        }
    }
    expanded.sort_by(|a, b| a.1.name.cmp(&b.1.name).then(a.0.cmp(&b.0)));
    for (i, member) in expanded {
        // ★Expanded members resolve directly by id (ADR-0309 decision 3)★ — by name would fold same-name twins into one `RECIPIENT_AMBIGUOUS` row (TRD S29 §4 X1). An id not in the pool is a
        //   source defect, surfaced as not found.
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
        // Unreachable with any explicit target (a name token makes a row even if resolution fails) — so only "all tokens were `@`addresses expanding to 0 people" (e.g. `["@all"]`, sender alone).
        return Err(SendReject::GroupEmpty);
    }
    Ok(Addressing { recipients, tokens })
}

/// ★One token → the recipient (spec §5)★ — the **same rule** as the control entrance's `resolve_in` (agent crate).
/// ① If it reads as a UUID (`PeerId::parse_str` — uppercase, braces, no hyphens, `urn:uuid:`) look it up as live id → asleep id, else fall through to the name. Id first so a UUID-shaped **name**
///    cannot hijack someone else's id target.
/// ② Name (exact, case-sensitive, no prefix): live 1 = live · live 2+ = ambiguous (live) · live 0 and asleep 1 = asleep · asleep 2+ = ambiguous (stored) · 0 = not found. ★A hit in the live layer
///    means the asleep layer is not looked at (spec §5 branch order)★ — "1 live + 1 asleep same name → the live one".
/// ★If the two entrances diverge, the agent that got the letter and the one the control command reached differ★ — guarded by the daemon crate's `tests/control_agent.rs` `resolver_alignment`,
///   which runs this function through `handle_send`. Domains differ: control sees live and asleep as one registry, mail has live beat asleep.
/// ★Across PCs (`ResolveScope::All` only — ADR-0391 decision 1)★: ⓪ before everything, `name@pc` (the text after the last `@` is a listed PC name,
///   ignoring case) goes to that PC without consulting its copy — it wins over a local agent literally named the whole token; then ①② as above —
///   anything found here ends here (a local agent always wins); ③ a UUID-shaped token listed by a copy → that PC (2+ copies → ambiguous);
///   ④ a bare name in the copies (connected or not, live or asleep) counted per PC: one → that PC, 2+ → ambiguous with `name@pc` candidates; a PC
///   whose directory is unknown this run is not counted. Whether the chosen PC is reachable is admission's (`PEER_OFFLINE`).
// ADR-0116, ADR-0120, ADR-0309, ADR-0391
fn resolve_token(token: &str, sources: &AddressingSources, scope: ResolveScope) -> Resolution {
    let remote = scope == ResolveScope::All;
    if remote {
        if let Some(found) = resolve_at_pc(token, sources) {
            return found;
        }
    }
    let as_id = PeerId::parse_str(token).ok();
    if let Some(found) = as_id.and_then(|id| resolve_id(id, sources)) {
        return found;
    }
    let local = resolve_local_name(token, sources);
    if !remote || !matches!(local, Resolution::NotFound) {
        return local;
    }
    if let Some(found) = as_id.and_then(|id| resolve_remote_id(id, token, sources)) {
        return found;
    }
    resolve_remote_name(token, sources)
}

/// Step ⓪ of `resolve_token` — `name@pc`. `None` = the text after the last `@` names no listed PC (the token is then an ordinary name).
// ADR-0391
fn resolve_at_pc(token: &str, sources: &AddressingSources) -> Option<Resolution> {
    let (name, pc) = token.rsplit_once('@')?;
    let pc = pc.to_lowercase();
    let target = sources.pcs.iter().find(|p| p.name.to_lowercase() == pc)?;
    Some(remote_to(target, name))
}

/// Step ③ — an id no local agent has, looked up in the known copies.
// ADR-0391
fn resolve_remote_id(id: PeerId, token: &str, sources: &AddressingSources) -> Option<Resolution> {
    let hits: Vec<(&RemotePc, &str)> = sources
        .pcs
        .iter()
        .filter_map(|pc| {
            let entry = pc.directory.as_ref()?.iter().find(|e| e.id == id)?;
            Some((pc, entry.name.as_str()))
        })
        .collect();
    match hits.as_slice() {
        [] => None,
        [(pc, _)] => Some(remote_to(pc, token)),
        many => Some(Resolution::RemoteAmbiguous {
            candidates: sorted_candidates(many.iter().map(|(pc, name)| (*name, *pc))),
        }),
    }
}

/// Step ④ — a bare name, exact and case-sensitive, counted per PC over the known copies only (ADR-0391 decision 1-4).
// ADR-0391
fn resolve_remote_name(token: &str, sources: &AddressingSources) -> Resolution {
    let hits: Vec<&RemotePc> = sources
        .pcs
        .iter()
        .filter(|pc| {
            pc.directory
                .as_ref()
                .is_some_and(|d| d.iter().any(|e| e.name == token))
        })
        .collect();
    match hits.as_slice() {
        [] => Resolution::NotFound,
        [pc] => remote_to(pc, token),
        many => Resolution::RemoteAmbiguous {
            candidates: sorted_candidates(many.iter().map(|pc| (token, *pc))),
        },
    }
}

fn remote_to(pc: &RemotePc, name: &str) -> Resolution {
    Resolution::Remote {
        node: pc.node,
        pc_name: pc.name.clone(),
        name: name.to_string(),
        link: pc.link,
    }
}

fn sorted_candidates<'a>(hits: impl Iterator<Item = (&'a str, &'a RemotePc)>) -> Vec<String> {
    let mut out: Vec<String> = hits
        .map(|(name, pc)| format!("{name}@{}", pc.name))
        .collect();
    out.sort();
    out
}

/// The listed PCs whose directory is unknown this run — named in the not-found hint (they were not searched).
// ADR-0391
fn unknown_directory_pcs(pcs: &[RemotePc]) -> Vec<String> {
    pcs.iter()
        .filter(|p| p.directory.is_none())
        .map(|p| p.name.clone())
        .collect()
}

/// Step ② of `resolve_token` — the local name rule, unchanged by the remote branch.
fn resolve_local_name(token: &str, sources: &AddressingSources) -> Resolution {
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

/// One registry id → the recipient. `None` if neither live nor asleep.
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

/// Puts one recipient into the row list and returns that row's position (the existing one if already there). ★Dedup keeps the first appearance (spec §5)★: an overlapping recipient's row position and
/// notation follow the first-written token (response `to` = WYSIWYA). ★Same row = same id if resolved, same notation if unresolved (ADR-0309)★: one agent by overlapping name/id/`@all` is one row,
/// same-name twins targeted each by id are two rows (TRD S29 §4 X2). ★Do not fold by name★ — a twin would vanish without a row (violates spec §6 "one recipient = 1 row"). An unresolved notation
/// does not mix with resolved rows: `["dup", "<dup1 id>"]` = a name row `RECIPIENT_AMBIGUOUS` + an id row delivered.
fn push_recipient(recipients: &mut Vec<ResolvedRecipient>, candidate: ResolvedRecipient) -> usize {
    if let Some(pos) = recipients.iter().position(|r| r.same_row(&candidate)) {
        return pos;
    }
    recipients.push(candidate);
    recipients.len() - 1
}

/// An `@`address resolution error → send-level bounce. A rule violation also folds into "no such address" (fits the sender; no new code outside the spec §6 vocabulary — the entrance hint says how to fix).
fn group_reject(e: GroupError, raw: &str) -> SendReject {
    match e {
        GroupError::NotFound { name } => SendReject::GroupNotFound { name },
        GroupError::InvalidName { .. } => SendReject::GroupNotFound {
            name: raw.to_string(),
        },
    }
}

/// ★Envelope `to` assembly (spec §1 — freeze target)★: accepted explicit targets by canonical name, `@`addresses kept as the token (not expanded), failed tokens excluded, in input order (`"@all,carol"`).
/// ★Inclusion = the token's resolved recipients are not all failures (A5 regression, load-bearing)★: not "did it create a row" — a token absorbed by dedup (`@all` in `["bob","carol","@all"]`) makes
/// no row but is not a failure either (spec §1: only failed tokens drop). ★Written once (M4)★: `["bob","bob"]` or name+id of one agent must not give `to="bob,bob"` (false headcount to the
/// receiving LLM); keep the first, fold later duplicates, compared by value (same-name twins accepted → name once). ★Expanded list not carried instead★ — disclosing recipients/roles (cc) is v2 (spec §8).
fn build_to_attr(addr: &Addressing, admitted: &[bool]) -> String {
    // The index of `admitted` matches the `recipients` order.
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

/// ★The single decision point of the entrance 3 branches (spec §5, ADR-0116 decision 1)★ — accepted → (receiving id, alive?), failed → (code, hint). Alive = delivery target; asleep = parking
/// (accepted); duplicate names = `RECIPIENT_AMBIGUOUS`; not found = `RECIPIENT_NOT_FOUND` (no parking). ★A live session is a delivery target even with no turn signal (decision 7)★ — do not revive `RECIPIENT_UNREACHABLE`.
/// ★Remote (ADR-0391)★: an agent on another PC is a fourth branch; whether it is sent is the caller's (link, sender name). Not found also names the
///   listed PCs whose directory is unknown this run (`unknown_pcs`), since those were not searched.
// ADR-0116, ADR-0391
fn admission<'r>(
    r: &'r ResolvedRecipient,
    roster_names: &str,
    unknown_pcs: &[String],
) -> Admission<'r> {
    match &r.to {
        Resolution::Live(a) => Admission::Local {
            id: a.id,
            live: true,
        },
        Resolution::Dormant { id, .. } => Admission::Local {
            id: *id,
            live: false,
        },
        Resolution::Remote {
            node,
            pc_name,
            name,
            link,
        } => Admission::Remote {
            node: *node,
            pc_name,
            name,
            link: *link,
        },
        Resolution::Ambiguous { count, layer } => Admission::Reject(
            FailCode::RecipientAmbiguous,
            ambiguous_hint(&r.display, *count, layer.label()),
        ),
        Resolution::RemoteAmbiguous { candidates } => Admission::Reject(
            FailCode::RecipientAmbiguous,
            cross_pc_ambiguous_hint(&r.display, candidates),
        ),
        Resolution::NotFound => {
            let mut hint = format!(
                "No agent named '{}' exists right now — not running and no saved agent by that name, so nothing was queued for it (fix the name, or create/spawn it and send again). Live agents: {roster_names}.",
                r.display
            );
            if !unknown_pcs.is_empty() {
                hint.push_str(&format!(
                    " Not searched — PCs whose agent list is not known yet (not linked since this daemon started): {}; to reach an agent there, write name@pc.",
                    unknown_pcs.join(", ")
                ));
            }
            Admission::Reject(FailCode::RecipientNotFound, hint)
        }
    }
}

enum Admission<'r> {
    Local {
        id: PeerId,
        live: bool,
    },
    Remote {
        node: NodeRef,
        pc_name: &'r str,
        name: &'r str,
        link: PcLink,
    },
    Reject(FailCode, String),
}

/// ★The contract-handling branch of a reply send (spec §3 item 7-④)★ — `handle_send` step 5 branches on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReplyDisposition {
    /// Accepted (`delivered`|`pending`) → `replied`. **Parking is acceptance too.**
    Accepted,
    /// The requester name is nowhere (`RECIPIENT_NOT_FOUND`) → `reply_failed` failure close.
    RequesterGone,
    /// Any other failure (`MAILBOX_FULL`·`RECIPIENT_AMBIGUOUS`, and the remote codes that fail at send time: `PEER_OFFLINE`·`NAME_TOO_LONG`·
    /// `REMOTE_REFUSED` for contract fields refused before sending) → **no-op** (stays open). An over-long remote token fails at send time as
    /// `RECIPIENT_NOT_FOUND` and so counts as `RequesterGone`.
    NoOp,
}

/// ★Reply outcome decision (spec §3 item 7-④, ADR-0116 decision 2)★ — from the response rows only (ledger not re-read). A reply has exactly 1 recipient (spec §3 item 7-①, ingress-enforced), yet this
/// decides from the row list: `handle_send` is pub and another assembly may pass several rows, where "accepted if any accepted" is the only safe reading (one reply arrived = duty fulfilled).
/// ★`RECIPIENT_NOT_FOUND` counts only when every row failed★.
// ADR-0116
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

/// ★Common tail of same-name failure hints (ADR-0116 decision 5)★. ★No exact-id resend guidance★: same-name agents are unsupported (root removal = ADR-0115), so the id-rescue flow must not be taught
/// as policy; the sending LLM should tell the user — resending repeats the result and burns context.
const AMBIGUOUS_ESCALATION: &str = "Duplicate agent names are not supported: do NOT resend this message — tell the user, who can rename or retire one of them.";

/// The hint of a duplicate-names failure row (live and asleep layers share it; `layer` is the label a human reads). ★Count and label must point at the same layer (review fix D6-a)★: live = roster count +
/// `live` (the roster is every live session regardless of turn signal, so the label matches the facts); asleep = profile count + `saved`. One side only makes the sender read "it says two but I see one" (spec §5).
fn ambiguous_hint(display: &str, count: usize, layer: &str) -> String {
    format!("'{display}' matches {count} {layer} agents, so the broker cannot tell which one you mean — nothing was queued. {AMBIGUOUS_ESCALATION}")
}

/// ★The lookup hint of a parked item closed by delete cleanup (spec §6 `RECIPIENT_DELETED`)★ — next action beside the code.
fn deleted_hint() -> String {
    "The recipient was deleted while this message was still parked, so it was closed as undelivered. The name may now belong to a different agent — do not resend automatically; tell the user.".to_string()
}

/// Asleep-parking hint (spec §5 branch 3, ADR-0116 decision 1): why it is pending. ★Expiry is quiet (accepted)★: unrestored items become `expired` at the 24h TTL with **no active notice to the sender**
/// (only the response `pending` and a later `eg_messages` lookup), so the hint says so — the sending LLM must not read it as "will surely arrive".
fn park_hint_dormant(display: &str) -> String {
    format!(
        "'{display}' is not running right now but it is a saved agent — parked; it will be delivered as one batch when that agent is restored. Nobody is notified if it expires first (24h TTL), so check with `eg_messages` if it matters."
    )
}

/// Busy hint (spec §6 ㉮①) — states the real busy reason; calling a recipient stopped after an error "in a turn" makes the sending LLM wait for a turn about to end.
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

/// ★Backed off from an overlapping drain — the "cannot confirm" hint (spec §6 ㉯, load-bearing)★. ★It must not read as "it did not go"★: the letter may already have been delivered in the winner's batch;
/// the sending LLM reporting "the peer did not receive it" is the misreading the spec §7 priming blocks — the real outcome is an `eg_messages{id}` lookup.
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

/// ★Choosing the `pending` hint when the drain could not send this letter (spec §6)★ — backing off (㉯) has top priority (mixed with other reasons it misleads the sender; `park_hint_overlapping`).
/// The reason is that of **this letter** (a drain's recipient is one queue id — `DrainReport`).
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

/// ★Cross-PC duplicate (ADR-0391 decision 1-4)★ — unlike the local duplicate hint, the sender can fix this in one resend, so it lists the candidates.
fn cross_pc_ambiguous_hint(display: &str, candidates: &[String]) -> String {
    format!(
        "'{display}' matches agents on several PCs ({}) — nothing was sent. Resend to exactly one of these addresses.",
        candidates.join(", ")
    )
}

/// `PEER_OFFLINE` — names the PC and the cause; there is no outbox (ADR-0390), so nothing waits to be resent.
fn peer_offline_hint(display: &str, pc: &str, link: PcLink) -> String {
    match link {
        PcLink::Refused => format!(
            "PC '{pc}' does not list this PC's ID, so it refuses the link — nothing was sent to '{display}'. Add this PC's ID on '{pc}' (peer.add there), then send again."
        ),
        PcLink::VersionMismatch => format!(
            "PC '{pc}' runs an incompatible Engram version, so there is no link — nothing was sent to '{display}'. Update Engram on one side, then send again."
        ),
        PcLink::Connected | PcLink::Offline => format!(
            "PC '{pc}' is offline (no live link) — nothing was sent to '{display}' and nothing is queued. Send again when it is back (check peer.list)."
        ),
    }
}

fn name_too_long_hint(display: &str) -> String {
    format!(
        "Your agent name cannot be used for mail to another PC (empty, over {REMOTE_NAME_MAX_BYTES} bytes, or with control characters) — nothing was sent to '{display}'. Rename the agent, then send again."
    )
}

/// A remote token no agent can carry — the other PC publishes and accepts names of at most `REMOTE_NAME_MAX_BYTES`.
fn remote_token_too_long_hint(display: &str, pc: &str) -> String {
    format!(
        "No agent on PC '{pc}' can be addressed as '{display}' (over {REMOTE_NAME_MAX_BYTES} bytes) — nothing was sent for it."
    )
}

/// Contract fields the other PC would refuse — refused here, per row, so the other rows to that PC still go.
fn remote_meta_hint(display: &str, pc: &str, why: &str) -> String {
    format!(
        "This mail cannot go to PC '{pc}' ({why}) — nothing was sent to '{display}'. Fix the request / reply fields and send again."
    )
}

/// The placeholder `handle_send` returns for a remote row before `await_remote_answer` seals the answer.
fn awaiting_hint(display: &str, pc: &str) -> String {
    format!("'{display}' is on PC '{pc}' — waiting for that PC's answer.")
}

fn held_hint(display: &str, pc: &str) -> String {
    format!(
        "Held on PC '{pc}': '{display}' is asleep or busy there — parked on that PC, to be delivered when it can take it. Nobody is notified if it expires there first (24h TTL) and no receipt follows. Ask later if it matters; resend only once it could have expired (24h), or the agent gets it twice."
    )
}

/// ★Unconfirmed (ADR-0393 decision 2)★ — must not read as "failed": the mail may have arrived.
fn unconfirmed_hint(display: &str, pc: &str) -> String {
    format!(
        "Unconfirmed: no answer from PC '{pc}' yet for '{display}' — it may or may not have arrived. Do not resend blindly; a later notice reports the outcome if one comes, or check `eg_messages`."
    )
}

/// A failed remote row — the code's own wording; a peer's reason is shown labelled with its PC (never in a notice).
// ADR-0390
fn remote_failure_hint(display: &str, pc: &str, code: FailCode, reason: Option<&str>) -> String {
    let said = reason
        .map(|r| format!(" PC '{pc}' says: \"{r}\"."))
        .unwrap_or_default();
    match code {
        FailCode::RecipientNotFound => format!(
            "No agent named '{display}' on PC '{pc}' — nothing was delivered. Its agent list here may be stale; check peer.list."
        ),
        FailCode::PeerOffline => peer_offline_hint(display, pc, PcLink::Offline),
        FailCode::BodyTooLarge => format!(
            "The message is too large to send to PC '{pc}' — nothing was sent to '{display}'. Shorten it and send again."
        ),
        FailCode::RemoteLimit => format!(
            "A limit on the way to PC '{pc}' was reached — '{display}' was not delivered. Retry later.{said}"
        ),
        _ => format!(
            "PC '{pc}' refused the mail for '{display}' ({}).{said}",
            code.as_str()
        ),
    }
}

/// ★The retirement facts one send gathered (R2)★ — what happened and what was planned but did not happen, separately (one bin pollutes metering — `Reservation::commit`); logged at different levels outside the lock.
#[derive(Default)]
struct RetirementLog {
    real: Vec<RetiredContract>,
    phantom: Vec<RetiredContract>,
}

/// ★Test observation surface for retirement metering (F5)★ — the `(contract msg_id, recipient id)` that `log_contract_retirements` actually reported. Without a log-capture harness it observes
/// only **whether the call happened** — whether it remains after the outcome loop (the M1 regression axis), not the wording.
#[cfg(any(test, feature = "test-harness"))]
pub(crate) mod retirement_reports {
    use std::cell::RefCell;

    thread_local! {
        static REPORTS: RefCell<Vec<(String, crate::PeerId)>> = const { RefCell::new(Vec::new()) };
    }

    pub(crate) fn push(request_id: &str, recipient_id: crate::PeerId) {
        REPORTS.with(|r| r.borrow_mut().push((request_id.to_string(), recipient_id)));
    }

    /// **Takes out and empties** the reports so far (per-section assertions). A lib build with only `test-harness` has no caller and a dead_code warning — normal (test-only surface).
    #[allow(dead_code)]
    pub(crate) fn drain() -> Vec<(String, crate::PeerId)> {
        REPORTS.with(|r| std::mem::take(&mut *r.borrow_mut()))
    }
}

/// Meters contracts actually retired by limit pressure **outside the lock** (logging before commit reports something that did not happen). ★Call site = after the outcome loop (M1)★: commit moved into
/// pass B (A2), so logging before the loop leaves the main-path retirement unlogged — the only evidence of retirement (ADR-0108 decision 2). ★M2 — do not revert to "commit where the contract is opened"★:
/// pass A's cap gate can still drop the recipient to a failure row and needs the rollback (`Reservation` header) — an A2/A3 regression.
fn log_contract_retirements(new_msg_id: &str, retired: &[RetiredContract]) {
    for r in retired {
        // ★Thread-local (F5)★: a free function with no service handle (signature kept for observation), and the send path runs to the end on the calling thread.
        #[cfg(any(test, feature = "test-harness"))]
        retirement_reports::push(&r.request_id, r.recipient_id);
        // Field-only metering — no body or token. A retirement-eligible contract owes no notice to the sender (only notified items or contracts with no deadline — ledger `open_request`).
        tracing::info!(
            retired_msg_id = %r.request_id,
            from = %r.sender,
            to = %r.recipient,
            to_id = %r.recipient_id,
            age_secs = r.age.as_secs(),
            new_msg_id = %new_msg_id,
            "unreplied contract cap pressure — evicting the oldest retirable contract and accepting the new request (ADR-0108)"
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
    /// The recipient name for the ledger row (send-time snapshot — ADR-0309 decision 4).
    recipient_name: &'a str,
    body: &'a str,
    kind: ParkKind,
    meta: &'a SendMeta,
    /// ★Total delivery records this logical message will leave (round-2 review F3)★ — notice = 1, send = recipient count **including failure rows**; parking also leaves a history row (`Pending`),
    /// so the same expected count is stamped there for the lookup's truncation decision.
    expected_rows: u16,
}

/// ★Finishes the reply contract close **in the same critical section as the acceptance record** (review fix D2, load-bearing)★. ★Blocks the reversal "accepted but contract `reply_failed`"★ (violates the
/// ratified "acceptance = completion, no reversal"): the old wiring parked the reply under lock A, released, then closed `replied` under lock B; a **delete cleanup** (`handle_profile_deleted`) between the
/// locks closes the contract as `reply_failed` (requester deleted) and the late close backs off with `AlreadyClosed` — "reply received as `pending`, contract reply-failed". Parking does not call inject, so there is
/// no reason to split the locks. ★Since ADR-0125 the acceptance branch is enqueue alone★: the single enqueue lock is the acceptance point and closing inside it makes atomicity hold on every branch; the
/// following drain's promotion to delivery is the delivery axis. (Rejected alternative: a mid-flight marker blocking cleanup — new state that, if leaked, leaves the contract never closed; this has zero new state.)
/// ★Idempotent★: once `done` is filled it does not close again (multi-recipient reply: the first acceptance closes, the rest are no-ops — same direction as `reply_disposition`).
/// ★Caveat — idempotency does not distinguish outcome kinds (harmless now, future trap; review fix N-nit)★: `NoMatch` is also recorded as "closed". Harmless today because the entrance forces exactly 1 recipient
/// on a reply (`control/ingress.rs` rule 2, spec §3 item 7-①); but `handle_send` is pub, so a caller skipping that enforcement could have an earlier row's `NoMatch` hide a later row's real match. To allow
/// multi-row replies narrow this to success outcomes (`Closed`/`ClosedHistoryAnomaly`), leaving `NoMatch` as "not yet closed". ★Logging is outside the lock★: the caller logs via `log_reply_close`.
// ADR-0116, ADR-0118, review fix D2
struct ReplyClosing<'a> {
    /// `Some` = this send is a reply (that contract id). `None` = this device is a complete no-op.
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

/// ★Logging the reply contract close result (**only outside the lock**)★ — the result only changes logging (no effect on delivery or the response, spec §3 item 7-②).
/// - `Closed` = normal (that replier's first valid reply); other recipients' contracts stay open (no reply-all).
/// - `ClosedHistoryAnomaly` = contract closed but history could not take `Delivered → Replied` (e.g. the reply was processed before the original, still `pending`); tracking is canonical, so only **observed**.
/// - `NoMatch`/`AlreadyClosed` = wrong id, not my contract, or already closed — a **normal path** (the reply itself was already delivered/parked); no new error for the sender (unfixable; a bounce would retry a delivered message).
fn log_reply_close(outcome: ReplyOutcome, in_reply_to: &str, replier_name: &str) {
    match outcome {
        ReplyOutcome::Closed => {}
        ReplyOutcome::ClosedHistoryAnomaly { from } => tracing::warn!(
            in_reply_to,
            history_status = ?from,
            "the reply closed the contract but the history could not transition to Replied (illegal edge) — the contract's source of truth is the tracking entry; only the history is not updated (ADR-0103)"
        ),
        ReplyOutcome::NoMatch => tracing::debug!(
            in_reply_to,
            replier = %replier_name,
            "reply_to does not point to an open request of this replier — message delivered normally, no contract closed (spec §3 item 7-②. Contracts are picked only by replier id — ADR-0309)"
        ),
        ReplyOutcome::AlreadyClosed => tracing::debug!(
            in_reply_to,
            replier = %replier_name,
            "an additional reply to an already closed contract — message delivered normally, no-op (spec §3 item 7-③)"
        ),
    }
}

/// Logging a reply-failure close result (outside the lock) — `fail_reply_contract` and the verdict entry's close share it.
fn log_reply_fail(outcome: ReplyFailOutcome, in_reply_to: &str, replier_name: &str) {
    match outcome {
        ReplyFailOutcome::Failed => tracing::info!(
            in_reply_to,
            replier = %replier_name,
            "reply confirmed unreachable (the requester name exists nowhere) — contract terminated as failed with reply_failed (removed from the open list · deadline sweep · 512 count, history kept — ADR-0116 decision 2)"
        ),
        ReplyFailOutcome::NoMatch => tracing::debug!(
            in_reply_to,
            replier = %replier_name,
            "reply_to does not point to an open contract of this replier — no contract to terminate (normal path, spec §3 item 7-②)"
        ),
        ReplyFailOutcome::AlreadyClosed => tracing::debug!(
            in_reply_to,
            replier = %replier_name,
            "contract already terminated — not reverted (no-op, spec §3 items 7-③/7-④)"
        ),
        ReplyFailOutcome::GuardHeld => tracing::debug!(
            in_reply_to,
            replier = %replier_name,
            "contract marked provisional·retiring, so failure termination is not applied — its lifetime is owned by that guard (ADR-0118 decision 3, no-op)"
        ),
    }
}

/// ★One terminal transition that **could not be left in the ledger** (C4 review fix J, round-5 finding 2)★ — the record was already pushed out of the history ring (`TransitionError::NotFound`).
/// ★Carries `intended` (load-bearing)★: the collector began expiry-only, but `skipped` (notice lane retirement) joined the same bin; without the state the log flattens to "expired" and the only audit
/// evidence left would state a false cause. The transition and the log use one value.
#[derive(Debug, Clone, PartialEq, Eq)]
struct EvictedTransition {
    msg_id: String,
    recipient: PeerId,
    intended: DeliveryStatus,
}

/// One terminal transition + collecting the evict fact (**single point** — the intended state cannot diverge between transition and log). `Illegal` is ignored as best-effort (an upper-layer bug, no reason to
/// block delivery); only `NotFound` is collected and logged **outside the lock**.
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

/// Logs the collected evict facts — **outside the lock only**.
fn log_evicted_transitions(evicted: &[EvictedTransition]) {
    for e in evicted {
        tracing::debug!(
            msg_id = %e.msg_id,
            recipient = %e.recipient,
            intended_status = ?e.intended,
            "record evicted before terminal transition — the terminal (expiry/reclaim) really happened but the record was already pushed out of the history ring buffer, so there is nothing to leave in the ledger (ledger HISTORY_CAPACITY)"
        );
    }
}

/// ★The single-exit settlement guard of the in-flight receipt (F1, load-bearing)★ — lowers the cap denominator raised by `Mailbox::take_in_flight` **whichever path flush exits by**.
/// ★Why a stack guard★: the delivery loop has per-target early breaks and external calls (inject); scattered settlement missing one place inflates that lane's denominator **permanently** (mail to that id never
/// gets in again — harder to fix than silent loss). So Drop repays the remainder and per-branch settlement is only an optimization (fitting the denominator to the real count), not a correctness condition.
/// ★A lock failure is swallowed in Drop★: it may be unwinding with the messaging lock poisoned; panicking again is a double-panic abort, and a poisoned lock means the service is already dead (every path
/// `expect`s), so restoring the denominator has no benefit. ★Temporary over-counting is explicit contract★: between detach points the denominator is higher by the number of items out — the window F1 aims to block.
struct FlightSettle<'a> {
    svc: &'a MessagingService,
    recipient: PeerId,
    owed: FlightTicket,
    /// The **resolved target** id this batch holds — released by the same guard as the recipient-id receipt. ★Leaked = that session never receives mail again★ (every later drain backs off on the target axis).
    // ADR-0142
    targets: Vec<PeerId>,
}

impl FlightSettle<'_> {
    /// Releases the held targets and returns those ids — **idempotent** (a second call returns an empty Vec so Drop does not release twice). Why it is called before echoing: `drain_queue` step 6.
    fn release_targets(&mut self) -> Vec<PeerId> {
        if self.targets.is_empty() {
            return Vec::new();
        }
        // ★Take the list out **after** taking the lock (load-bearing order)★: taking it first, a failed lock acquisition leaves the id gone from `self` and Drop's belt cannot see what to recover — that
        //   session would never receive mail again. Poison is not swallowed here (normal path = `expect`); the swallowing idiom belongs to `Drop`, which may be unwinding.
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
        // ★Target release comes before receipt settlement★: when unwinding this is the only release point, and both finish with one lock acquisition.
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

/// ★The out-of-lock facts one park left (round-6)★ — pressure reclaim happens in the lock but logging must be outside (module header), so the caller carries this accumulator and calls `log` after release.
/// ★Count the branches separately★: reclaim reasons differ (notice pushed out by a newer notice / TTL exceeded — the clock decided first); merged, the log blurs the reason, and the TTL branch even has a
/// different ledger vocabulary (`expired` — F3).
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
                "park retire: TTL (mailbox PARK_TTL) already exceeded at reclaim time — the sweep (60s period) just has not collected it yet, so the ledger word is expired, not skipped (spec §5 · F3)"
            );
        }
        if !self.retired_notices.is_empty() {
            tracing::debug!(
                recipient,
                retired = self.retired_notices.len(),
                msg_ids = ?self.retired_notices,
                "park retire: notice lane cap (mailbox NOTICE_CAP) exceeded — reclaiming the oldest notice and ledger skipped (new notices are always accepted — round-6)"
            );
        }
        log_evicted_transitions(&self.evicted);
    }
}

/// `ParkRequest` → one stored item. One place assembles it so the envelope payload and TTL reference time cannot diverge per insert path (caller is just `park_into` — ADR-0125).
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

/// ★The in-lock core of parking + recording ledger `pending` (shared by single send and group fan-out)★. ★Extracted as a free function★: fan-out must finish N members' decision and parking in one lock
/// span, and the lock-taking wrapper (`park_notice`) per recipient would open a "queue looks empty" window; `park_notice` stays a thin wrapper so one set of parking rules (no silent loss, payload encoding,
/// ledger recording) serves both paths.
/// ★No silent loss (ADR-0103)★: a successful park always leaves `pending` in the ledger; a cap-exceeded bounce stores nothing and leaves nothing (the caller shows it as a bounce/skip).
/// ★Ledgering of pressure reclaim (round-6, F3)★: the item this park swept out (`ParkAdmitted.retired` — message-lane debris or an old notice) moves **here** to a ledger terminal — the store is pure and this boundary
/// is the only place that knows the fact; without it a ghost pending remains (not in the queue, lookup says "waiting"). Logging material per reason goes to `effects`, logged outside the lock.
/// ★Vocabulary differs per item — TTL beats `skipped` (F3)★: default `skipped`, but because the sweep period (60 s) is shorter than the TTL (24 h) only by a gap, an item past TTL but uncollected may be
/// reclaimed; per spec §5 that is `expired`. The decision reuses mailbox's `ParkedMessage::is_expired(now)` so the TTL constant and `>=` boundary live in one place.
// ADR-0107
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
    // The admission sequence number is assigned by `park` (the store is the only assigner — mailbox comment); the value here is an ignored placeholder.
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
            // ADR-0114: message-lane pressure reclaim was abolished — only notice retirement comes here. The branch stays so a future reclaim reason is not silently flattened.
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

/// ★Encoding (version header since C3 review fix 4)★:
///   `<ver>\n<sender_len>\n<reply_by_len>\n<reply_to_len>\n<group_len>\n<from_agent_id>\n<from_epoch>\n<from_node>\n`
///   `<entrance>\n<flags>\n<sender><reply_by><reply_to><group><body>`
///   `<from_node>` = 64 hex digits, or `-` for this PC (ADR-0393 — the observation keeps the sender's node across parking).
///   The first 10 lines are newline-free fields (numbers/uuid/hex/short literals); the first 4 variable strings are bounded by a **length prefix** (body = the remainder, safe with newlines in body or in `reply_to`,
///   which is agent input). Length 0 = `None` (`Some("")` is already bounced by entrance validation).
/// ★Why a version tag★: used only inside the process (parking is in-memory), but an old payload meeting a changed decode could be silently misread (length field shifted, body cut). With the 1-character tag an
/// unknown version = immediate fallback → a visible degradation ("attributes lost, body only"), not misinterpretation. If v2 persists parking to disk the tag is the migration branch point.
/// ★If you change the layout, bump this value★: same-layout-different-version collapses the tag's meaning (= the layout contract of this string).
const PARK_PAYLOAD_VERSION: &str = "4";

/// Parking payload — the minimum meta at flush injection time to assemble the envelope and emit the observation record (sender name, sender identity, entrance, raw body): an internal encoding carrying raw
/// values around the `ParkedMessage.envelope` contract. At flush it decodes → `wrap_now` assembles in the **current** format (single wrap point) and emits the observation with the sender identity (ADR-0088).
/// ★Why from/entrance are carried★: parking→flush auto-delivery is observed too (the harness must recover "who was delivered to whom" on the flush path, spec §7), so the original `SenderIdentity` and entrance
/// survive until flush. ★Why meta (reply contract) is carried (C3, load-bearing)★: a late-delivered request/reply must carry the same envelope attributes (id/type/reply-by/in-reply-to) as immediate delivery;
/// otherwise a parked request arrives plain, the receiving LLM does not know the id to reply to and only the sender gets the overdue notice (contract silently broken).
struct ParkPayload {
    sender_name: String,
    from: SenderIdentity,
    entrance: Entrance,
    body: String,
    /// C3 reply-contract meta + C4 group label. The parsed Duration is **not restored** (the contract was opened in the ledger at send time; flush needs only the envelope attributes), so only `reply_by`'s raw notation survives.
    meta: SendMeta,
}

impl ParkPayload {
    fn encode(&self) -> String {
        let ent = match self.entrance {
            Entrance::Mcp => "mcp",
            Entrance::Cli => "cli",
            Entrance::Daemon => "daemon",
            Entrance::Remote => "remote",
        };
        let reply_by = self.meta.reply_by_raw.as_deref().unwrap_or("");
        let reply_to = self.meta.reply_to.as_deref().unwrap_or("");
        // The envelope `to` (frozen value) — carried so a late-delivered parked item keeps the send-moment notation (spec §1).
        let group = self.meta.to_attr.as_deref().unwrap_or("");
        let flags = if self.meta.request { "r" } else { "-" };
        let node = self
            .from
            .node
            .map_or_else(|| "-".to_string(), NodeRef::to_hex);
        format!(
            "{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}{}{}{}{}",
            PARK_PAYLOAD_VERSION,
            self.sender_name.len(),
            reply_by.len(),
            reply_to.len(),
            group.len(),
            self.from.peer_id,
            self.from.epoch,
            node,
            ent,
            flags,
            self.sender_name,
            reply_by,
            reply_to,
            group,
            self.body
        )
    }

    /// The inverse of encode. A broken format (impossible — our own encoding) falls back to treating everything as body (better than silent loss).
    /// ★Never panics (fix 4, load-bearing)★: called per item inside the flush batch loop; a panic drops the rest of the batch (production ends the daemon via the hook — ADR-0292) and one broken item would drag down
    /// healthy messages. Every failure path ends in an **item-level fallback**: a false length field (sum exceeds the remainder), a cut **not on a char boundary** (where `split_at` panics), or an unknown version
    /// loses the envelope attributes but sends the original as body; the batch's other items deliver normally (test: one corrupt item does not abort the batch).
    fn decode(s: &str) -> Self {
        // splitn(11) — the last piece is not cut, so newlines inside body are preserved.
        let mut it = s.splitn(11, '\n');
        let fallback = || Self {
            sender_name: String::new(),
            from: SenderIdentity {
                peer_id: PeerId::nil(),
                epoch: 0,
                node: None,
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
            Some(node_str),
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
            it.next(),
        )
        else {
            return fallback();
        };
        if ver != PARK_PAYLOAD_VERSION {
            return fallback();
        }
        let node = match node_str {
            "-" => None,
            hex => match NodeRef::from_hex(hex) {
                Some(n) => Some(n),
                None => return fallback(),
            },
        };
        // ★Enums are parsed **strictly** (round-final fix 3, load-bearing)★: an unknown entrance string used to fall silently to `Cli` and an unknown flag to "not a request" — arbitrary reinterpretation (not a fallback):
        //   the observation records a different entrance, or a request arrives plain without id/reply-by (contract break). A value outside the vocabulary is evidence of damage → the same item-level fallback as length damage.
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
                node,
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

/// Pairs with the literal in `encode`; when entrance kinds increase fix both (if they diverge a normal payload falls back — the round-trip test catches it).
fn parse_entrance(s: &str) -> Option<Entrance> {
    match s {
        "mcp" => Some(Entrance::Mcp),
        "cli" => Some(Entrance::Cli),
        "daemon" => Some(Entrance::Daemon),
        "remote" => Some(Entrance::Remote),
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

/// ★The identity of a daemon self-send (C3)★ — a `<notice>` has no sending agent but the observation record (`DeliveryObservation.from`) requires an identity, so the **nil PeerId** marks daemon origin (no real
/// agent overlaps); with `Entrance::Daemon` it tells an infrastructure notice from the record alone.
fn daemon_identity() -> SenderIdentity {
    SenderIdentity {
        peer_id: PeerId::nil(),
        epoch: 0,
        node: None,
    }
}

/// The `draw_daemon_msg_id` result — the selected id + a collision trace (logged by the caller **outside the lock**).
struct DrawnMsgId {
    /// The id actually to use (passed the check, or the last value even after the re-draw).
    id: String,
    /// If the first draw collided and was discarded, that value — the investigation clue is the side that collided (as the ingress retry log; keeping only the replacement leaves nothing to trace).
    collided: Option<String>,
    /// Did even the re-drawn id collide. true = suspect random-number/ledger wiring; the notice is sent anyway.
    still_colliding: bool,
}

// ★Do not revive `duration_notation` (Duration → notation reverse map; C3 review fix 6)★: normalization announced `"60m"` as `"1h"` and diverged from the envelope `reply-by="60m"`. The ledger keeps the sender
//   notation as the original (`ledger.rs` `RequestEntry.reply_by`) and the notice uses it as is.

// ★Do not revive `transition_expired_by_msg_id` (C4, ADR-0104)★: looking up the "first pending record" by msg_id alone is right only for a single recipient; a group broadcast has N records per msg_id (wrong member)
//   and its correctness leaned on a coincidence between history insertion order and expiry order. The mailbox now returns the queue key per expired item (`ExpiredParked`) so `sweep` targets `(msg_id, recipient)`;
//   a msg_id-only reverse lookup is structurally incompatible with a 1:N ledger.
// ADR-0104

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tid;
    use std::sync::Mutex as StdMutex;
    use std::time::Duration;

    /// Delivery observation is a no-op here — production has no observer installed either; asserting observation records is the daemon integration harness's job (ADR-0088).
    struct FakeControlPlane;
    impl ControlPlanePort for FakeControlPlane {
        fn envelope_format(&self) -> EnvelopeFormat {
            EnvelopeFormat::default()
        }
        fn record_delivery(&self, _obs: DeliveryObservation) {}
    }

    struct FakeDeliveryPort {
        /// ★Why an Arc★: the test clones the handle and swaps the roster **during inject** (inside `on_inject`) to reproduce the TOCTOU "recipient restarted after the snapshot, before the write"
        ///   deterministically (capturing the port in the hook would be a self-referential cycle).
        roster: Arc<StdMutex<Vec<LiveAgent>>>,
        injected: StdMutex<Vec<(PeerId, Vec<u8>)>>,
        call_count: StdMutex<usize>,
        fail_at_call: StdMutex<Vec<usize>>,
        /// ★Why an Arc, not a Box★: injection can happen again inside the hook (e.g. a deferred flush echoes and delivers on the spot); calling a Box under a lock guard would re-enter this
        ///   non-reentrant mutex and **deadlock**, so it is a cloneable Arc called with the lock released.
        on_inject: StdMutex<Option<Arc<dyn Fn(usize) + Send + Sync>>>,
        /// The live_agents lookup count — the axis the "one snapshot per send" (ADR-0111 decision 2) regression counts (fix 8).
        roster_calls: StdMutex<usize>,
        /// Once set, it returns this roster from **after the first lookup** — for reproducing finding 3 TOCTOU self-heal.
        roster_after_first: StdMutex<Option<Vec<LiveAgent>>>,
        /// ★One-shot roster lookup hook (reproduces the fix 5 race)★ — used once and emptied, so a send made inside the hook does not recurse.
        on_roster: StdMutex<Option<Box<dyn Fn() + Send>>>,
        /// ★The asleep (id, name) list (profile exists, no session — spec §5 branch 3)★. Duplicates are not folded (two of one name = same name in the asleep layer).
        dormant: StdMutex<Vec<DormantAgent>>,
        /// ★The ids for which `is_agent_live` returns true (delete-cleanup gate scenario — review fix D1)★: simulates cases not caught by name (terminal-mode session, renamed profile).
        /// ★Scripted separately from the roster (intended)★: the gate asks with that narrow verb; deriving it from the roster would make the fake build the wiring itself and hide the real one.
        live_ids: StdMutex<Vec<PeerId>>,
        /// The listed PCs and their directory copies handed out in the snapshot (S37 remote branch).
        pcs: StdMutex<Vec<RemotePc>>,
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
                pcs: StdMutex::new(Vec::new()),
            }
        }
        fn set_pcs(&self, pcs: Vec<RemotePc>) {
            *self.pcs.lock().unwrap() = pcs;
        }
        /// Puts `tid(name)` as the id per name — launching `live` with the same name means that agent woke up.
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
        /// Unlike `live_agents` it does not touch the call counter or hook — an injection-boundary decision, not a roster **lookup** (production also reads the session map directly).
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
        /// ★The entrance decision-source snapshot (ADR-0116)★ — mirrors the production adapter: `roster` is the same value as `live_agents()` (predicate = state only), `dormant` a separate source.
        ///   Reading `self.roster` directly would not raise the lookup counter and blind the "one snapshot per send" regression.
        fn addressing_sources(&self) -> AddressingSources {
            AddressingSources {
                roster: self.live_agents(),
                dormant: self.dormant.lock().unwrap().clone(),
                pcs: self.pcs.lock().unwrap().clone(),
            }
        }

        fn is_agent_live(&self, id: PeerId) -> bool {
            self.live_ids.lock().unwrap().contains(&id)
        }
    }

    /// ★Replier identity (A6)★ — a reply must be sent with that recipient's own identity to close its own contract (id-first matching, ledger `close_on_reply`); a fixed `ident()` leaned on
    ///   name matching, which was a path that closed someone else's contract.
    fn reply_from(id: PeerId) -> SenderIdentity {
        SenderIdentity {
            peer_id: id,
            epoch: 0,
            node: None,
        }
    }

    /// The sender must also be in the roster for the `@all` exclusion rule to apply.
    fn live_sender(name: &str) -> (SenderIdentity, LiveAgent) {
        let (id, agent) = live(name);
        (
            SenderIdentity {
                peer_id: id,
                epoch: agent.epoch,
                node: None,
            },
            agent,
        )
    }

    /// ★Test-only single-recipient shim★ — folds the 1-row `handle_send` response into the old outcome vocabulary so the existing "1 recipient" regression tests (parking, flush, gate, contract) survive the
    ///   ADR-0111 overhaul. ★It does not hide★: absence, duplicate name and mailbox full are now row failures → `Err(SendFail::…)`, so a test expecting "park if absent" still fails.
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
            assert_eq!(rows.len(), 1, "a single-recipient send has exactly 1 row");
            let row = &rows[0];
            match row.status {
                SendStatus::Delivered => Ok(SendOutcome::Delivered),
                SendStatus::Pending => Ok(SendOutcome::Parked {
                    hint: row.hint.clone().unwrap_or_default(),
                }),
                SendStatus::Failed => Err(SendFail::Row(
                    row.code.expect("a failure row requires code"),
                )),
            }
        }
    }

    impl MessagingService {
        /// ★Legacy parking seam (test-only)★ — creates the **state** "mail is queued for that name". Absence is now a failure row (ADR-0111 decision 1), so the old absent-parking entry is gone; those tests
        ///   subject the queue's behavior (flush order, gate, TTL, re-parking, in-flight accounting), so only the entry is replaced and everything after it runs the real code.
        /// ★It does not obscure "absence is failure"★: that contract is covered by the tests calling `handle_send` directly (absence → failure row + no parking record).
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
            // First through the same wiring guard as production (the bypass does not loosen that contract).
            debug_assert!(
                !(meta.request && meta.reply_to.is_some()),
                "ingress is the only validator — request and reply_to are mutually exclusive (spec §6)"
            );
            debug_assert!(
                meta.to_attr.is_none(),
                "the envelope to is fixed once by handle_send after the acceptance decision of every recipient (spec §1)"
            );
            // ★Look at the roster **first** (load-bearing)★: running the production path then reverting on the failure row would leave the bounce's ledger terminal (`failed`) overlapping the queue item
            //   (a lookup would return 2 rows). Bypass the entry only when absence is certain.
            let live_only = AddressingSources {
                roster: self.port.live_agents(),
                dormant: Vec::new(),
                pcs: Vec::new(),
            };
            let absent = !matches!(
                resolve_token(to, &live_only, ResolveScope::All),
                Resolution::Live(_)
            );
            match absent {
                true => {
                    // The id collision check is at the production place (first side-effect point) — the bypass is also a zero-side-effect bounce.
                    {
                        let st = self.state.lock().expect("messaging state poisoned");
                        if st.ledger.msg_id_in_use(msg_id) {
                            return Err(SendFail::Reject(SendReject::IdCollision));
                        }
                    }
                    // The old absent parking opened a contract too (spec §3); opened here to keep the tests on that axis (overdue notice, outstanding lookup) alive.
                    if meta.request {
                        let mut st = self.state.lock().expect("messaging state poisoned");
                        let reply_by = meta.reply_by.zip(meta.reply_by_raw.clone());
                        // ★Always hand the retirement mark to commit★: otherwise the marked victim stays in the list (ledger `occupies_slot`) and the limit is never reached — the cap fixture loops forever (as `commit_contract`).
                        // ★Receiving id = `tid(to)`★ — an agent later launched as `live(to)` with the same name is that absent recipient itself (`live` doc; same as the asleep id in production).
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
            node: None,
        }
    }

    /// A live agent — id is `tid(name)`, so the same name again is the **same agent** (restarted, woke from asleep). For distinct same-name agents use `live_distinct` (ADR-0309 — the mail key is an id; mixing
    ///   the two folds the test into one entity and it passes vacuously).
    fn live(name: &str) -> (PeerId, LiveAgent) {
        live_with_id(tid(name), name)
    }

    /// A live agent — random id. For same-name twins · 「a different agent with the same name」 scenarios.
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
                node: None,
            },
            agent,
        )
    }

    /// ★A live agent with no turn signal (terminal/console — ADR-0116 decision 7)★ — **in** the roster (membership is not a condition), injected immediately with no gate.
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

    /// ★Doorbell unwired (C2)★ = the inline flush fallback (FlushTrigger comment) — synchronous self-healing keeps assertions deterministic; the doorbell wiring itself is `svc_gated_with_doorbell`.
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

    /// Simulates the production shape that **hands flush to another thread** — nobody consumes here, so "no injection on the sending thread" can be asserted.
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
        /// Per-section assertions — take out and empty so earlier sections' doorbells do not mix in.
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
            "immediate inject = delivered (ADR-0104)"
        );
        assert_eq!(
            port.injected_bodies(),
            vec![r#"<message from="bob">hi</message>"#.to_string()],
            "inject = envelope in the current format (xml)"
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
            .expect("parking is not a bounce");
        assert!(
            matches!(out, SendOutcome::Parked { .. }),
            "inject failure → parking"
        );
        assert_eq!(
            svc.ledger_statuses("m1"),
            vec![DeliveryStatus::Pending],
            "an inject failure is a pending ledger entry too (no loss)"
        );
        assert_eq!(svc.parked_len(tid("alice")), 1);
    }

    #[test]
    fn inject_failure_parks_pending_without_a_turn_signal() {
        // ★Blocks 2 regressions★: ① turning this class's injection failure into a failure row/loss (ADR-0116 decision 7) ② a new send overtaking the backlog so built (ADR-0121 decision 2). ② holds because of the
        //   **enqueue order** (ADR-0125), not sealed decision points.
        // ★No-isolation occasion ㉠ (spec §7 ④)★: what opens m1's injection-failed share is the synchronous drain of the **following send** — not the doorbell, not its own retry (pressing its own doorbell because
        //   of its own injection failure is forbidden — spec §5); m2 alone must make m1 go out.
        let (svc, port) = svc();
        let (from, me) = live_sender("alice");
        let (_tui_id, tui) = live_no_turn_signal("tui");
        port.set_roster(vec![me, tui]);
        port.fail_at(&[0]);

        let out = send_body(&svc, "m1", from, "alice", &["tui"], "old letter")
            .expect("parking is not a bounce");
        assert_eq!(
            (out[0].status, out[0].code),
            (SendStatus::Pending, None),
            "inject failure → parking (neither a failure row nor a silent loss): {out:?}"
        );
        assert_eq!(svc.parked_len(tid("tui")), 1);
        assert_eq!(
            svc.ledger_statuses("m1"),
            vec![DeliveryStatus::Pending],
            "the ledger must stay pending too (no loss)"
        );

        let second =
            send_body(&svc, "m2", from, "alice", &["tui"], "new letter").expect("row response");
        assert_eq!(
            second[0].status,
            SendStatus::Delivered,
            "the drain after queueing sent it together with the old one = delivered (the result of going through the queue, not a direct send): {second:?}"
        );
        assert_eq!(
            port.injected_bodies(),
            vec![
                r#"<message from="alice">old letter</message>"#.to_string(),
                r#"<message from="alice">new letter</message>"#.to_string()
            ],
            "went out in one batch in the order old (m1) → new (m2) (oldest first — the drain of the following send opened it)"
        );
        assert_eq!(
            svc.parked_len(tid("tui")),
            0,
            "nothing stranded — the batch emptied the queue"
        );
        assert_eq!(
            (svc.ledger_statuses("m1"), svc.ledger_statuses("m2")),
            (
                vec![DeliveryStatus::Delivered],
                vec![DeliveryStatus::Delivered]
            ),
            "both delivered at the actual inject time"
        );
    }

    #[test]
    fn a_mid_batch_injection_failure_stops_there_and_leaves_the_rest_pending() {
        // ★Latter half of ADR-0121 decision 2★: partial success is not reverted to total failure — `delivered → failed` is illegal on the ledger graph (`fail_pending` guard), so this assertion observes
        //   "the ledger rows keep the graph".
        // ★Why the backlog is made with the gate (ADR-0125)★: every send drains in its own call, so an unconsumed doorbell alone does not pile up the queue; the drain must be unable to send — formally the
        //   idle gate (a recipient with a turn signal set busy).
        let (svc, port, gate) = svc_gated();
        let (from, me) = live_sender("alice");
        let (recv_id, recv) = live("recv"); // Has a turn signal = a gate target.
        port.set_roster(vec![me, recv]);
        gate.set_busy(recv_id, 0);

        for (id, body) in [("m1", "one"), ("m2", "two"), ("m3", "three")] {
            let out = send_body(&svc, id, from, "alice", &["recv"], body).expect("row response");
            assert_eq!(
                out[0].status,
                SendStatus::Pending,
                "{id} is queued and the drain is caught by the gate: {out:?}"
            );
        }
        assert_eq!(
            svc.parked_msg_ids(tid("recv")),
            vec!["m1", "m2", "m3"],
            "queueing order = queue order (oldest first)"
        );

        gate.clear();
        port.fail_at(&[1]);
        svc.flush_for(recv_id);

        assert_eq!(
            port.injected_bodies(),
            vec![r#"<message from="alice">one</message>"#.to_string()],
            "the batch stops at the failure point — the items after it were not written"
        );
        assert_eq!(
            svc.parked_msg_ids(tid("recv")),
            vec!["m2", "m3"],
            "everything after the failure point returns to the queue keeping its age order (no TTL extension)"
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
            "the front part stays delivered (not reverted) and the rest stays pending"
        );
    }

    #[test]
    fn an_overlapping_drain_retreats_with_pending_and_the_letter_still_goes_out_in_order() {
        // ★spec §7 ⑦, ADR-0125 §Impact unverified ②★ — seat reservation is gone but the two properties it sealed remain: **no order inversion** and **no isolation (`parked_len == 0`)**.
        // Interleaving (routine since the drain runs inside the send call): ① m1 enqueues, drains and stops inside inject (batch in flight) ② m2 arrives for the same recipient → enqueued, its drain **backs off at step 0**
        // ③ m1's inject fails, the batch is re-parked, settlement echoes the backed-off side's doorbell. Without the guard ② would drain again and reach the recipient before m1's remainder.
        // ★Determinism★: the hook runs on the same thread inside the fake port's `inject` — no sleep or thread timing.
        let (svc, port) = svc();
        let (from, me) = live_sender("alice");
        let (_tui_id, tui) = live_no_turn_signal("tui");
        port.set_roster(vec![me, tui]);

        // An assertion inside the hook would leak a panic by unwinding and blur the cause, so only the result is brought out.
        let observed = Arc::new(StdMutex::new(None::<RecipientResult>));
        let observed_h = observed.clone();
        let svc_h = svc.clone();
        let armed = Arc::new(StdMutex::new(true));
        let armed_h = armed.clone();
        port.set_on_inject(Arc::new(move |idx| {
            if idx != 0 || !std::mem::replace(&mut *armed_h.lock().unwrap(), false) {
                return;
            }
            let second = send_body(&svc_h, "m2", from, "alice", &["tui"], "new letter")
                .expect("row response");
            *observed_h.lock().unwrap() = Some(second[0].clone());
        }));
        port.fail_at(&[0]);

        let first = send_body(&svc, "m1", from, "alice", &["tui"], "old letter")
            .expect("parking is not a bounce");
        assert_eq!(first[0].status, SendStatus::Pending, "{first:?}");
        let loser = observed
            .lock()
            .unwrap()
            .clone()
            .expect("concurrent send row");
        assert_eq!(
            loser.status,
            SendStatus::Pending,
            "the side that backed off from an overlapping drain answers pending **conservatively** (it does not know whether its letter was injected)"
        );
        assert!(
            loser.hint.unwrap_or_default().contains("at the same time"),
            "that pending must be ㉯ (cannot confirm) — answering with the ㉮ (left in the queue) reason is misread as 'did not go' (spec §6)"
        );
        // Reaching here without a manual flush is because the receipt holder echoes the backed-off side's doorbell (spec §7 ④ ㉡).
        assert_eq!(
            port.injected_bodies(),
            vec![
                r#"<message from="alice">old letter</message>"#.to_string(),
                r#"<message from="alice">new letter</message>"#.to_string()
            ],
            "delivery order = queueing order (m1 → m2). Without the overlap guard, m2 overtakes m1"
        );
        assert_eq!(
            svc.parked_len(tid("tui")),
            0,
            "nothing stranded — the queue was emptied"
        );
        assert_eq!(
            (svc.ledger_statuses("m1"), svc.ledger_statuses("m2")),
            (
                vec![DeliveryStatus::Delivered],
                vec![DeliveryStatus::Delivered]
            ),
            "even if the response is pending, the ledger is delivered at the actual inject time (the two axes differ — spec §6)"
        );
    }

    #[test]
    fn a_concurrent_burst_loses_nothing_and_keeps_each_senders_own_order() {
        // ★A **soak test**, not a seal of the overlap guard★: it asserts nothing about global delivery order matching enqueue order (the cross-thread enqueue order is the scheduler's and unobservable), and deleting the
        //   guard turns it red only probabilistically. The deterministic seal is `an_overlapping_drain_...` (hook) and the two deferral-echo hook tests — do not delete those and rely on this. Several threads draining one
        //   recipient is routine, so it checks the assertions below survive real contention.
        // ★The cap is deliberately left alone★ — the total stays under 100 so contention does not leak into `MAILBOX_FULL` (that axis is `the_cap_gate_applies_to_a_live_idle_recipient_too`).
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
                                .expect("row response");
                            out[0].status
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        let statuses: Vec<SendStatus> = handles
            .into_iter()
            .flat_map(|h| h.join().expect("sender thread"))
            .collect();

        assert!(
            statuses
                .iter()
                .all(|s| matches!(s, SendStatus::Delivered | SendStatus::Pending)),
            "a race does not create failure rows (the identity axis is unrelated to races): {statuses:?}"
        );
        assert_eq!(
            svc.parked_len(tid("tui")),
            0,
            "nothing stranded — the queue was emptied in the end"
        );

        let bodies = port.injected_bodies();
        assert_eq!(
            bodies.len(),
            SENDERS * PER_SENDER,
            "0 lost · 0 duplicated — every letter went out exactly once"
        );
        let order: Vec<(usize, usize)> = bodies
            .iter()
            .map(|b| {
                let start = b.find('>').expect("opening tag") + 1;
                let end = b.rfind('<').expect("closing tag");
                let (k, n) = b[start..end]
                    .trim_start_matches('t')
                    .split_once('-')
                    .expect("tag notation");
                (
                    k.parse().expect("sender"),
                    n.parse().expect("sequence number"),
                )
            })
            .collect();
        for k in 0..SENDERS {
            let seen: Vec<usize> = order
                .iter()
                .filter(|(s, _)| *s == k)
                .map(|(_, n)| *n)
                .collect();
            assert_eq!(
                seen.len(),
                PER_SENDER,
                "all of sender {k}'s letters must go out"
            );
            assert!(
                seen.windows(2).all(|w| w[0] < w[1]),
                "the order sender {k} queued was reversed in delivery — the batch skipped the head of the queue: {seen:?}"
            );
        }
    }

    #[test]
    fn the_cap_gate_applies_to_a_live_idle_recipient_too() {
        // ★ADR-0125 §Impact, spec §5 — wider cap surface★: every send passes the enqueue gate, so even a live recipient can get `MAILBOX_FULL` — a design consequence, not a regression. Do not revive the old
        //   contract (empty queue + idle → even the 101st injected immediately).
        // ★The narrow span is pinned too (spec §5 correction)★: an idle recipient's drain empties the queue, so the cap is never hit (①); only when the queue exceeds the cap before the drain catches up does it differ —
        //   made deterministic here with the idle gate (②③).
        let (svc, port, gate) = svc_gated();
        let (from, me) = live_sender("boss");
        let (r_id, recv) = live("recv");
        port.set_roster(vec![me, recv]);

        // ① An idle recipient — the drain empties the queue every time.
        for i in 0..200 {
            let out =
                send(&svc, &format!("idle{i}"), from, "boss", &["recv"]).expect("row response");
            assert_eq!(
                out[0].status,
                SendStatus::Delivered,
                "the cap does not apply to an idle recipient (#{i}): {out:?}"
            );
        }
        assert_eq!(
            svc.parked_len(tid("recv")),
            0,
            "the queue is empty, so the cap denominator is 0 too"
        );

        // ② Only in the span where the drain cannot run (in a turn) does the queue fill up to the cap.
        gate.set_busy(r_id, 0);
        for i in 0..100 {
            let out =
                send(&svc, &format!("busy{i}"), from, "boss", &["recv"]).expect("row response");
            assert_eq!(
                out[0].status,
                SendStatus::Pending,
                "in a turn = stays in the queue (#{i})"
            );
        }
        assert_eq!(svc.parked_len(tid("recv")), 100);

        // ③ The next send is caught by the cap gate **even though the recipient is live**.
        let over = send(&svc, "over", from, "boss", &["recv"]).expect("row response");
        assert_eq!(
            (over[0].status, over[0].code),
            (SendStatus::Failed, Some(FailCode::MailboxFull)),
            "a live recipient also passes the cap gate (direct-send bypass removed — ADR-0125): {over:?}"
        );
        assert_eq!(
            svc.parked_len(tid("recv")),
            100,
            "bounced items do not enter the queue"
        );
        assert_eq!(svc.ledger_statuses("over"), vec![DeliveryStatus::Failed]);
    }

    #[test]
    fn a_drain_that_deferred_a_flush_repays_it_even_when_its_own_inject_fails() {
        // ★The receipt holder repays the deferral wake-up (spec §5 invariant, ADR-0121, ADR-0125)★: unless it echoes the `deferred_flush` marker while settling, the wake-up evaporates and the re-parked share is
        //   bound until the next event or TTL. Holding the receipt is routine (drain runs on the sending thread), so this is the main path.
        // ★Not a self-retry★: what is repaid is a wake-up another caller requested; a lone failure with no marker still presses no doorbell (guarded by `inject_failure_parks_pending_without_a_turn_signal`'s
        //   `parked_len == 1`).
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

        send_body(&svc, "m1", from, "alice", &["tui"], "old letter").expect("park");

        assert_eq!(
            port.injected_bodies(),
            vec![r#"<message from="alice">old letter</message>"#.to_string()],
            "the flush that backed off must be rung again after settlement and pick up the re-parked items"
        );
        assert_eq!(svc.parked_len(tid("tui")), 0);
        assert_eq!(svc.ledger_statuses("m1"), vec![DeliveryStatus::Delivered]);
    }

    #[test]
    fn a_successful_drain_also_repays_the_flush_it_deferred() {
        // ★The success branch of the same contract★: a batch receipt is in flight until settlement even on success, so the duty to repay a backed-off flush is the same; dropping only this branch makes delivery depend
        //   on another doorbell happening (a recipient with no turn signal has no turn-end doorbell — `deferred_flush`).
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
            // Parks by a **path that does not press the doorbell**, so the following flush backs off.
            svc_h
                .park_absent_for_test(
                    "m0",
                    ident(),
                    "s",
                    "tui",
                    "extra",
                    Entrance::Mcp,
                    &SendMeta::default(),
                )
                .expect("park");
            svc_h.flush_for(tui_id);
        }));

        send_body(&svc, "m1", from, "alice", &["tui"], "my letter").expect("row response");

        assert_eq!(
            svc.parked_len(tid("tui")),
            0,
            "the success branch must also pay back the deferral mark — otherwise m0 stays stuck in the queue until the next event"
        );
        assert_eq!(svc.ledger_statuses("m0"), vec![DeliveryStatus::Delivered]);
    }

    #[test]
    fn the_flush_gate_asks_the_same_predicate_the_send_path_asks() {
        // ★ADR-0121 §Impact — the gate predicate is defined in one place★: if the send side skips the gate for this class while the flush side backs off on it, the order guarantee means no one opens the queued letter
        //   (delivery stops). Sealed here: both call the same predicate (`gate_busy_reason`).
        // ★Assert with the gate set busy★: the no-turn-signal class must not look at it — if `flush_for` reverts to calling the gate directly (`busy.busy_reason`/`busy.is_busy`) the batch is skipped and this goes red.
        let (svc, port, gate) = svc_gated();
        let (from, me) = live_sender("alice");
        let (tui_id, tui) = live_no_turn_signal("tui");
        port.set_roster(vec![me, tui]);
        gate.set_busy(tui_id, 0);

        port.fail_at(&[0]);
        let out = send(&svc, "m1", from, "alice", &["tui"]).expect("row response");
        assert_eq!(out[0].status, SendStatus::Pending, "{out:?}");
        assert_eq!(svc.parked_len(tid("tui")), 1);

        port.fail_at(&[]);
        svc.flush_for(tui_id);
        assert_eq!(
            svc.ledger_statuses("m1"),
            vec![DeliveryStatus::Delivered],
            "the batch of a recipient without a turn signal goes out even if the gate says busy (the same predicate as the send side)"
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
            "flush = bulk inject of individual envelopes, oldest first (ADR-0104)"
        );
        assert_eq!(svc.ledger_statuses("m0"), vec![DeliveryStatus::Delivered]);
        assert_eq!(svc.ledger_statuses("m2"), vec![DeliveryStatus::Delivered]);
        assert_eq!(svc.parked_len(tid("late")), 0, "queue empty after flush");
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
            "only the one before the failure (b0) injected"
        );
        assert_eq!(
            svc.ledger_statuses("m0"),
            vec![DeliveryStatus::Delivered],
            "b0 delivered"
        );
        assert_eq!(
            svc.parked_len(tid("late")),
            2,
            "failed + unattempted items re-parked (lossless)"
        );
        assert_eq!(
            svc.ledger_statuses("m1"),
            vec![DeliveryStatus::Pending],
            "b1 re-parked pending"
        );
        assert_eq!(
            svc.ledger_statuses("m2"),
            vec![DeliveryStatus::Pending],
            "b2 re-parked pending"
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
            "re-flush of re-parked items also keeps oldest-first (b1→b2)"
        );
        assert_eq!(svc.parked_len(tid("late")), 0, "queue empty after re-flush");
    }

    #[test]
    fn flush_repark_with_full_queue_loses_nothing_and_preserves_order() {
        // ★finding 1 (BLOCK) regression★: re-parking after a flush inject failure must not lose an admitted item even if a **concurrent park** between drain and inject refilled the queue to the cap (no silent loss),
        //   and the older re-parked share must go ahead of the new one (no FIFO inversion). Park-based re-parking was lost to MailboxFull.
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
        // The hook's concurrent sends must be **absent** to be parked, so at idx 0 it briefly empties the roster — `flush_for`'s reachability re-verification has already passed.
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
            "only the one before the failure (b0) injected"
        );
        // ★Zero loss — but not above the cap either (F1 expected value 102 → 99)★: the old expectation (concurrent park fills to 100 and the 2 re-parked sit on top) is the hole that grew the queue every cycle.
        //   Now the drained batch's 3 items stay in the denominator as in-flight, so concurrent park is accepted 97 times and the 2 re-parked come back: 97 + 2 = 99 ≤ cap.
        assert_eq!(
            svc.parked_len(tid("late")),
            MAILBOX_CAP_FOR_TEST - 1,
            "the re-parked items (b1,b2) are not lost, and the total does not exceed the cap (97 concurrent park + 2 re-parked)"
        );
        assert!(
            svc.ledger_statuses("c99").is_empty(),
            "a concurrent send bounced because of in-flight does not remain in the ledger either (bounce = not stored at all)"
        );
        assert_eq!(
            svc.ledger_statuses("m1"),
            vec![DeliveryStatus::Pending],
            "b1 re-parked pending (not a ghost)"
        );
        assert_eq!(
            svc.ledger_statuses("m2"),
            vec![DeliveryStatus::Pending],
            "b2 re-parked pending (not a ghost)"
        );
        // ★Order★
        port.set_on_inject(Arc::new(|_| {}));
        port.fail_at(&[]);
        svc.flush_for(late_id);
        let bodies = port.injected_bodies();
        assert_eq!(bodies[0], r#"<message from="s">b0</message>"#);
        assert_eq!(
            bodies[1], r#"<message from="s">b1</message>"#,
            "re-parked items must come before new concurrent park items (prevents FIFO inversion)"
        );
        assert_eq!(bodies[2], r#"<message from="s">b2</message>"#);
        assert_eq!(
            bodies[3], r#"<message from="s">c0</message>"#,
            "new concurrent park items come after the re-parked items"
        );
        assert_eq!(svc.parked_len(tid("late")), 0, "queue empty after re-flush");
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
            .expect("parking within the cap");
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
            .expect_err("exceeding the cap is bounced");
        assert_eq!(rej, SendFail::Row(FailCode::MailboxFull));
        assert_eq!(
            svc.parked_len(tid("full")),
            100,
            "a bounced one does not enter the queue"
        );
        // It is made visible to the sender at once as a bounce, so it is not left in the ledger.
        assert!(
            svc.ledger_statuses("over").is_empty(),
            "a cap bounce is not recorded in the ledger"
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
        // The parking time is `Instant::now()` inside handle_single_send, so give a now well in the future of that.
        let now = Instant::now() + PARK_TTL_FOR_TEST + Duration::from_secs(1);
        svc.sweep(now);
        assert_eq!(
            svc.ledger_statuses("m1"),
            vec![DeliveryStatus::Expired],
            "sweep = pending→expired ledger transition"
        );
        assert_eq!(
            svc.parked_len(tid("ghost")),
            0,
            "expired items removed from the queue"
        );
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
            "absent at execution time → flush skip (kept parked)"
        );
        assert!(
            port.injected_bodies().is_empty(),
            "no inject to a dead target (finding 2)"
        );
    }

    /// ★A new incarnation of the same id receives it (ADR-0111 decision 6 → ADR-0309 decision 1)★: a letter parked for a busy incarnation (epoch 0) goes to the new incarnation (same id, epoch 1); the gate is
    ///   asked by (id, epoch), so the old incarnation's busy mark does not block it.
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

        assert_eq!(
            svc.parked_len(id),
            0,
            "queue emptied by delivery to the new incarnation"
        );
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
            node: None,
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
        assert_eq!(
            d.meta,
            SendMeta::default(),
            "a notice must have empty contract meta"
        );
    }

    #[test]
    fn park_payload_roundtrip_preserves_contract_meta_with_newlines() {
        // ★C3★: reply_to is **agent input**, so newlines and multibyte characters can be mixed in.
        let from = SenderIdentity {
            peer_id: PeerId::new_v4(),
            epoch: 7,
            node: None,
        };
        let meta = SendMeta {
            request: true,
            reply_by_raw: Some("10m".to_string()),
            reply_by: Some(Duration::from_secs(600)),
            reply_to: None,
            to_attr: None,
        };
        let p = ParkPayload {
            sender_name: "발신자-한글".to_string(), // hangul-ok
            from,
            entrance: Entrance::Daemon,
            body: "본문\n둘째 줄".to_string(), // hangul-ok
            meta: meta.clone(),
        };
        let d = ParkPayload::decode(&p.encode());
        assert_eq!(
            d.sender_name,
            "발신자-한글", // hangul-ok
            "multi-byte sender boundary (byte-length prefix)"
        );
        assert_eq!(d.body, "본문\n둘째 줄"); // hangul-ok
        assert!(matches!(d.entrance, Entrance::Daemon));
        assert!(d.meta.request, "request flag restored");
        assert_eq!(
            d.meta.reply_by_raw.as_deref(),
            Some("10m"),
            "notation restored"
        );
        assert_eq!(
            d.meta.reply_by, None,
            "the parsed value is intentionally not restored"
        );
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
        // The state vocabulary is shared with injection-failure parking; only the hint distinguishes the reason.
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
            .expect("busy parking is not a bounce");
        match out {
            SendOutcome::Parked { hint } => {
                assert!(
                    hint.contains("mid-turn"),
                    "the hint states the busy reason: {hint}"
                )
            }
            other => panic!("a busy recipient must be parked: {other:?}"),
        }
        assert_eq!(
            svc.ledger_statuses("m1"),
            vec![DeliveryStatus::Pending],
            "busy parking = pending ledger (delivered only at the actual inject time — ADR-0104)"
        );
        assert_eq!(
            svc.parked_len(tid("alice")),
            1,
            "waiting in that id's queue"
        );
        assert!(
            port.injected_bodies().is_empty(),
            "does not push to stdin during a turn (no bypassing the CLI queue)"
        );
    }

    // ── ADR-0231: the parking hint states the real busy reason (production gate `BusyPolicy` + planted facts) ─────────────

    struct SilentNotifier;
    impl super::super::busy::IdleNotifier for SilentNotifier {
        fn notify_idle(&self, _id: PeerId) {}
    }

    /// The same `BusyPolicy` as production on planted facts — a reason cannot be produced with `FakeGate` (bool only).
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
            .expect("busy parking is not a bounce");
        assert!(
            port.injected_bodies().is_empty(),
            "does not push to a busy recipient"
        );
        match out {
            SendOutcome::Parked { hint } => hint,
            other => panic!("a busy recipient must be parked: {other:?}"),
        }
    }

    #[test]
    fn a_recipient_halted_after_a_failed_turn_is_not_called_mid_turn() {
        // QA: a "mid-turn" hint went to a recipient stopped after an error turn (not in a turn).
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
        assert_eq!(
            svc.parked_len(tid("alice")),
            0,
            "the drain emptied the queue"
        );
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
            "unobserved (id, epoch) falls back to idle → immediate inject"
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
        assert!(port.injected_bodies().is_empty(), "0 injects during a turn");

        gate.clear();
        svc.flush_for_agent(recv_id);
        assert_eq!(
            port.injected_bodies(),
            vec![
                r#"<message from="s">b0</message>"#.to_string(),
                r#"<message from="s">b1</message>"#.to_string(),
                r#"<message from="s">b2</message>"#.to_string(),
            ],
            "bulk inject of individual envelopes, oldest first, on entering idle (ADR-0104)"
        );
        assert_eq!(svc.ledger_statuses("m0"), vec![DeliveryStatus::Delivered]);
        assert_eq!(svc.ledger_statuses("m2"), vec![DeliveryStatus::Delivered]);
        assert_eq!(svc.parked_len(tid("recv")), 0);
    }

    #[test]
    fn busy_park_self_heals_when_turn_ends_during_park() {
        // ★busy-park TOCTOU self-healing (as C1 finding 3)★: if the turn ends between the gate check (busy) and park, the idle trigger already passed the empty queue (lost wakeup).
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
            "the send response is parking (pending) — self-heal is the delivery after that"
        );
        assert_eq!(gate.call_count(), 2, "check + recheck after park = 2 times");
        assert_eq!(
            svc.ledger_statuses("m1"),
            vec![DeliveryStatus::Delivered],
            "if idle right after park, delivered immediately by the self-heal flush (pending→delivered)"
        );
        assert_eq!(
            svc.parked_len(tid("recv")),
            0,
            "queue emptied by the self-heal flush"
        );
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
        assert_eq!(svc.parked_len(tid("recv")), 1, "still busy → kept parked");
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
            "a flush during a turn does not start a batch (pre-drain gate)"
        );
        assert_eq!(svc.parked_len(tid("recv")), 3, "kept parked");
        assert_eq!(
            gate.call_count() - calls_before,
            1,
            "the gate is queried once per batch (pre-drain)"
        );
        gate.clear();
        let calls_before = gate.call_count();
        svc.flush_for(recv_id);
        assert_eq!(
            port.injected_bodies().len(),
            3,
            "if idle, the batch runs to completion (no mid-batch recheck — no dribbling)"
        );
        assert_eq!(
            gate.call_count() - calls_before,
            1,
            "even a 3-item batch queries the gate only once"
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
            "a skip does not touch the queue"
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

    /// Targeting one twin by exact id parks it in that id's queue and it goes only to that agent — overlapping names do not mix queues (ADR-0309 decision 1); twin B's turn end does not open A's letter.
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
        assert_eq!(svc.parked_len(id_a), 1, "queue key = the targeted id");
        assert_eq!(svc.parked_len(id_b), 0, "twin B's queue is empty");

        svc.flush_for_agent(id_b);
        assert!(
            port.injected_bodies().is_empty(),
            "B's turn end does not release A's letter"
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
            "delivered to that targeted agent"
        );
        assert_eq!(svc.ledger_statuses("m1"), vec![DeliveryStatus::Delivered]);
        assert_eq!(svc.parked_len(id_a), 0);
    }

    /// ★B2 — the busy-parking side (ADR-0309 decisions 1·5)★: a letter parked for busy X does not go to Y if X disappears and a **different agent with the same name** Y (different id) comes up; if X (same id) returns
    ///   as a new incarnation, X receives it even with Y up.
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
            "while X is absent, same-named Y does not receive X's letter (B2)"
        );
        assert_eq!(svc.parked_len(x_id), 1, "stays in X's queue");
        assert_eq!(svc.ledger_statuses("m1"), vec![DeliveryStatus::Pending]);

        port.set_roster(vec![y, LiveAgent { epoch: 1, ..x }]);
        svc.flush_for(x_id);
        assert_eq!(
            port.injected_targets(),
            vec![x_id],
            "X, back up, receives it"
        );
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
            "after queueing, that call's drain sent it together with the old one = delivered: {out:?}"
        );
        assert_eq!(
            port.injected_bodies(),
            vec![
                r#"<message from="s">b0</message>"#.to_string(),
                r#"<message from="s">b1</message>"#.to_string(),
                r#"<message from="s">b2</message>"#.to_string(),
            ],
            "the order the recipient sees = queueing order (a new send does not overtake the queue)"
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
            "even with an empty queue + idle it goes through queueing, and if the same call's drain sends it, delivered"
        );
        assert_eq!(port.injected_bodies().len(), 1);
        assert_eq!(
            svc.parked_len(tid("recv")),
            0,
            "the drain emptied that item"
        );
        assert_eq!(
            svc.in_flight_len(tid("recv")),
            0,
            "no missed receipt settlement — if one remains, that recipient lane is blocked forever"
        );
    }

    #[test]
    fn an_idle_sends_letter_is_observably_taken_out_of_the_queue_not_bypassed() {
        // ★The **counterfactual seal** of spec §7 ① (review blind MEDIUM)★ — kills "with an empty queue + idle, a shortcut skipping enqueue goes straight to `port.inject` and answers `Delivered`".
        //   `an_empty_queue_and_an_idle_recipient_still_ends_as_delivered` and `an_idle_recipient_gets_it_within_the_same_send_call` see only the outcome, which the shortcut also produces, so they cannot guard this.
        // ★Observation point★: the `on_inject` hook = outside the lock, drain holding the receipt, **mid-injection**; the ledger row being `pending` is also where unspecified (ii) "does the row pass through pending" is settled.
        let (svc, port, _gate) = svc_gated();
        let (from, me) = live_sender("alice");
        let (_r, recv) = live("recv");
        port.set_roster(vec![me, recv]);

        // A panic inside the hook would leak by unwinding and blur the cause, so only the observation is brought out.
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

        let out = send(&svc, "m1", from, "alice", &["recv"]).expect("row response");
        assert_eq!(
            out[0].status,
            SendStatus::Delivered,
            "premise: this counterfactual means something only on the empty queue + idle branch: {out:?}"
        );

        let (in_flight, queued, statuses) = seen.lock().unwrap().clone().expect("the hook ran");
        assert_eq!(
            in_flight, 1,
            "★if the direct-send branch revives, this becomes 0★ — a letter being injected must be accounted for as a share taken out of the queue"
        );
        assert_eq!(
            queued, 0,
            "it was taken out, so it is not in the queue (pins that ① is not 'not taken out yet')"
        );
        assert_eq!(
            statuses,
            vec![DeliveryStatus::Pending],
            "★the row queueing left inside the lock★ — with a direct send, at this point there would be no row at all or it would already be delivered"
        );

        assert_eq!(svc.ledger_statuses("m1"), vec![DeliveryStatus::Delivered]);
        assert_eq!(svc.parked_len(tid("recv")), 0);
        assert_eq!(
            svc.in_flight_len(tid("recv")),
            0,
            "no missed receipt settlement"
        );
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
            "if a batch is being injected, a new send is queued at the tail and its drain backs off (response pending): {:?}",
            outcome.lock().unwrap()
        );
        assert_eq!(
            port.injected_bodies(),
            vec![
                r#"<message from="s">b0</message>"#.to_string(),
                r#"<message from="s">b1</message>"#.to_string(),
                r#"<message from="s">b2</message>"#.to_string(),
            ],
            "the order the recipient sees = queueing order (a new send does not overtake the batch in progress)"
        );
        assert_eq!(
            svc.parked_len(tid("recv")),
            0,
            "the share that backed off is eventually delivered too (nothing stranded)"
        );
        assert_eq!(
            svc.in_flight_len(tid("recv")),
            0,
            "no receipt settlement leak"
        );
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
            "an overlapping flush neither drains nor injects (deferral)"
        );
        assert_eq!(
            port.injected_bodies(),
            vec![
                r#"<message from="s">b0</message>"#.to_string(),
                r#"<message from="s">b1</message>"#.to_string(),
            ],
            "this assembly has no doorbell consumer, so the joined items are still in the queue"
        );
        assert_eq!(
            svc.parked_msg_ids(tid("recv")),
            vec!["m2".to_string()],
            "deferred items do not vanish (lossless)"
        );
        assert_eq!(
            svc.in_flight_len(tid("recv")),
            0,
            "no receipt settlement leak"
        );
        assert_eq!(
            bell.seen().len(),
            bells_at_hook + 1,
            "finishing settlement rings the deferred flush again exactly once (no lost wakeup)"
        );
        // ★Only one id deferred here, so the echo is also 1 (as in the single-slot days)★ — that the marker is a set is enforced by the stale-id test below (round-8).
        assert_eq!(
            bell.seen().last().copied(),
            Some(recv_id),
            "re-ring target = the doorbell id of the queue the deferring side was about to open"
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
            "1 doorbell request (unconditional enqueue)"
        );
        assert!(
            port.injected_bodies().is_empty(),
            "does not inject on the sender thread (consumption is the flush lane)"
        );
        assert_eq!(svc.parked_len(tid("recv")), 1, "kept parked");
        gate.clear();
        svc.flush_for_agent(recv_id);
        assert_eq!(svc.ledger_statuses("m1"), vec![DeliveryStatus::Delivered]);
    }

    // ── Even after a rename, that id's letter goes to that agent (ADR-0309 decisions 1 · 5) ────────────────────────
    /// Park for busy live X → X renames mid-turn → the turn-end flush opens that id's queue; the queue key is an id, so a rename cannot orphan it (round-3 finding 2 — formerly blocked by sweeping the name queue with an id hint).
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
        assert_eq!(svc.parked_len(id), 1, "parked in that agent id's queue");

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
            "after a rename, the turn-end flush opens that id's queue"
        );
        assert_eq!(port.injected_targets(), vec![id]);
        assert_eq!(svc.parked_len(id), 0, "queue empty");
        assert_eq!(svc.ledger_statuses("m1"), vec![DeliveryStatus::Delivered]);
    }

    /// ★B1★ A letter entrusted to asleep X (name a) still goes to X after it renames to b while asleep and wakes as b (a name queue stranded it in queue a until TTL `expired`).
    #[test]
    fn mail_parked_for_a_dormant_agent_reaches_it_after_a_rename_while_asleep() {
        let (svc, port) = svc();
        let (from, me) = live_sender("alice");
        let x_id = PeerId::new_v4();
        port.set_roster(vec![me.clone()]);
        port.set_dormant_agents(&[(x_id, "a")]);
        let out = send_body(&svc, "m1", from, "alice", &["a"], "hi").expect("row response");
        assert_eq!(
            out[0].status,
            SendStatus::Pending,
            "asleep recipient → parking: {out:?}"
        );
        assert_eq!(svc.parked_len(x_id), 1, "queue key = the id of asleep X");

        // Rename to b while asleep → wake up as b and appearance flush.
        port.set_dormant_agents(&[]);
        port.set_roster(vec![me, live_with_id(x_id, "b").1]);
        svc.flush_for(x_id);

        assert_eq!(port.injected_targets(), vec![x_id]);
        assert_eq!(
            port.injected_bodies(),
            vec![r#"<message from="alice">hi</message>"#.to_string()],
            "the renamed X receives it (B1)"
        );
        assert_eq!(svc.parked_len(x_id), 0);
        assert_eq!(svc.ledger_statuses("m1"), vec![DeliveryStatus::Delivered]);
    }

    /// ★B2★ After X renames a → b while asleep, another profile Y waking as a and flushing does not receive X's letter; X receives it when it wakes. A name-lookup branch would hand Y someone else's letter.
    #[test]
    fn a_new_holder_of_the_old_name_does_not_take_mail_parked_for_the_renamed_agent() {
        let (svc, port) = svc();
        let (from, me) = live_sender("alice");
        let x_id = PeerId::new_v4();
        port.set_roster(vec![me.clone()]);
        port.set_dormant_agents(&[(x_id, "a")]);
        let out = send_body(&svc, "m1", from, "alice", &["a"], "hi").expect("row response");
        assert_eq!(
            out[0].status,
            SendStatus::Pending,
            "asleep recipient → parking: {out:?}"
        );

        port.set_dormant_agents(&[(x_id, "b")]);
        let (y_id, y) = live_distinct("a");
        port.set_roster(vec![me.clone(), y.clone()]);
        svc.flush_for(y_id);
        svc.flush_for_agent(y_id);
        svc.flush_for(x_id);
        assert!(
            port.injected_bodies().is_empty(),
            "Y, who newly has the old name, does not receive X's letter (B2)"
        );
        assert_eq!(svc.parked_len(x_id), 1, "stays in X's queue");
        assert_eq!(svc.parked_len(y_id), 0);
        assert_eq!(svc.ledger_statuses("m1"), vec![DeliveryStatus::Pending]);

        port.set_dormant_agents(&[]);
        port.set_roster(vec![me, y, live_with_id(x_id, "b").1]);
        svc.flush_for(x_id);
        assert_eq!(
            port.injected_targets(),
            vec![x_id],
            "when X wakes, X receives it"
        );
        assert_eq!(svc.ledger_statuses("m1"), vec![DeliveryStatus::Delivered]);
    }

    // ── Same-name twins have separate queues — one side's busy · injection failure does not hold the other's letter (ADR-0309 decision 1) ──
    /// Name-queue days: letters targeting twins by exact id mixed into one queue and a batch needed a per-target gate and restore (round-3 finding 3). The queue key is an id, so a batch's target is always one;
    ///   pinned: each twin has its own queue and order.
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
            "accumulates only in the targeted id's queue"
        );
        assert_eq!(svc.parked_msg_ids(id_b), vec!["m1", "m3"]);

        // A is free · B is busy. A's first injection fails.
        gate.clear();
        gate.set_busy(id_b, 0);
        port.fail_at(&[0]);
        svc.flush_for(id_a);
        assert!(
            port.injected_bodies().is_empty(),
            "A's batch stopped at the first item"
        );
        assert_eq!(
            svc.parked_msg_ids(id_a),
            vec!["m0", "m2"],
            "failed items return to A's queue, oldest first as they were"
        );
        assert_eq!(
            svc.parked_msg_ids(id_b),
            vec!["m1", "m3"],
            "A's failure does not touch B's queue"
        );

        port.fail_at(&[]);
        svc.flush_for(id_a);
        assert_eq!(
            port.injected_bodies(),
            vec![
                r#"<message from="s">body-m0</message>"#.to_string(),
                r#"<message from="s">body-m2</message>"#.to_string(),
            ],
            "A's share goes out even if B is busy"
        );
        assert_eq!(port.injected_targets(), vec![id_a, id_a]);
        assert_eq!(svc.ledger_statuses("m1"), vec![DeliveryStatus::Pending]);
        assert_eq!(svc.ledger_statuses("m3"), vec![DeliveryStatus::Pending]);

        svc.flush_for(id_b);
        assert_eq!(
            port.injected_bodies().len(),
            2,
            "does not go into B, which is in a turn"
        );

        gate.clear();
        svc.flush_for(id_b);
        assert_eq!(
            port.injected_bodies()[2..],
            [
                r#"<message from="s">body-m1</message>"#.to_string(),
                r#"<message from="s">body-m3</message>"#.to_string(),
            ],
            "B's share is oldest-first too"
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
        assert_eq!(svc.parked_len(tid("recv")), 1, "parked because busy");
        assert!(
            port.injected_bodies().is_empty(),
            "no inject since in a turn (normal)"
        );

        assert_eq!(
            gate.sweep_stale_busy(t0 + BUSY_MAX_TURN + Duration::from_secs(1)),
            1,
            "wakes the leftovers past the cap"
        );
        assert_eq!(
            notifier.seen.lock().unwrap().clone(),
            vec![id],
            "waiting mail goes out only if woken (flipping only the decision leaves no next trigger)"
        );
        // In production the flush lane consumes that doorbell — here the consumption is performed directly.
        svc.flush_for_agent(id);
        assert_eq!(
            port.injected_bodies(),
            vec![r#"<message from="s">hi</message>"#.to_string()],
            "delivered after the cap sweep (TTL blackhole resolved)"
        );
        assert_eq!(svc.ledger_statuses("m1"), vec![DeliveryStatus::Delivered]);
    }

    /// A doorbell with an empty list, or a turn end whose stall was released — either way the flush lane asks about busy again and drains the parking; until then even the 30-minute cap does not release it.
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
        assert_eq!(
            svc.parked_len(tid("recv")),
            1,
            "parked because of the list·stop"
        );

        assert_eq!(
            gate.sweep_stale_busy(t0 + BUSY_MAX_TURN + Duration::from_secs(1)),
            0,
            "the cap does not wake the list·stop"
        );
        svc.flush_for_agent(id);
        assert!(
            port.injected_bodies().is_empty(),
            "does not go in while the list remains"
        );

        facts.set_inputs_pending(id, 0, false, t0);
        svc.flush_for_agent(id);
        assert!(
            port.injected_bodies().is_empty(),
            "even with an empty list, does not go in if stopped after an error"
        );

        facts.set_last_end_failed(id, 0, false, t0);
        svc.flush_for_agent(id);
        assert_eq!(
            port.injected_bodies(),
            vec![r#"<message from="s">hi</message>"#.to_string()],
            "the doorbell after release lets the parked items flow"
        );
    }

    #[test]
    fn flush_for_agent_is_noop_without_parked_and_opens_its_own_id_queue() {
        let (svc, port, _gate) = svc_gated();
        let (recv_id, recv) = live("recv");
        port.set_roster(vec![recv]);
        svc.flush_for_agent(recv_id);
        assert!(port.injected_bodies().is_empty(), "no-op without parking");
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
            "flushing a nonexistent id is a harmless no-op"
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
            "do it",
            Entrance::Mcp,
            &req_meta("60m", 3600),
        )
        .expect("parked");
        svc.sweep(Instant::now() + Duration::from_secs(3601));
        assert_eq!(
            port.injected_bodies(),
            vec![
                "<notice>[engram] request m-60m deadline (60m) exceeded — no reply from ghost</notice>".to_string()
            ],
            "the notation written in the envelope as is (no normalizing to 1h)"
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
                "write the code and reply",
                Entrance::Mcp,
                &req_meta("10m", 600),
            )
            .expect("delivered");
        assert_eq!(out, SendOutcome::Delivered);
        assert_eq!(
            port.injected_bodies(),
            vec![concat!(
                r#"<message from="qa-alpha" id="m-req1" type="request" reply-by="10m">"#,
                "write the code and reply</message>"
            )
            .to_string()],
            "request envelope golden (attribute order·kebab-case)"
        );
        assert_eq!(
            svc.open_request_count(),
            1,
            "a delivered request opens a contract"
        );
        assert_eq!(
            svc.ledger_statuses("m-req1"),
            vec![DeliveryStatus::Delivered]
        );
    }

    #[test]
    fn parked_request_opens_contract_too_and_keeps_envelope_on_flush() {
        // ★spec §3 step 2★: envelope attributes must be the same when delivered late — else the receiver does not know the id to reply to and only the sender gets a timeout notice.
        let (svc, port) = svc();
        port.set_roster(vec![]);
        let out = svc
            .park_absent_for_test(
                "m-req2",
                ident(),
                "qa-alpha",
                "bob",
                "do it and let me know",
                Entrance::Mcp,
                &req_meta("1h", 3600),
            )
            .expect("parked");
        assert!(matches!(out, SendOutcome::Parked { .. }));
        assert_eq!(
            svc.open_request_count(),
            1,
            "a parked request opens a contract too"
        );
        assert_eq!(svc.ledger_statuses("m-req2"), vec![DeliveryStatus::Pending]);

        let (bob_id, bob) = live("bob");
        port.set_roster(vec![bob]);
        svc.flush_for(bob_id);
        assert_eq!(
            port.injected_bodies(),
            vec![concat!(
                r#"<message from="qa-alpha" id="m-req2" type="request" reply-by="1h">"#,
                "do it and let me know</message>"
            )
            .to_string()],
            "park→flush delivery is the same request envelope too (C3 ParkPayload meta preserved)"
        );
        assert_eq!(
            svc.ledger_statuses("m-req2"),
            vec![DeliveryStatus::Delivered]
        );
    }

    #[test]
    fn reply_envelope_carries_in_reply_to_immediate_and_flushed() {
        // ★Golden (reply)★: send argument reply_to → receive attribute in-reply-to (spec §1); no id/type (a reply is not something to reply to).
        let (svc, port) = svc();
        let (_id, alice) = live("alice");
        port.set_roster(vec![alice]);
        svc.park_absent_for_test(
            "m-rep1",
            ident(),
            "qa-bravo",
            "alice",
            "all written",
            Entrance::Mcp,
            &reply_meta("m-7f3k"),
        )
        .expect("delivered");
        assert_eq!(
            port.injected_bodies(),
            vec![
                r#"<message from="qa-bravo" in-reply-to="m-7f3k">all written</message>"#
                    .to_string()
            ],
        );

        let (svc2, port2) = super::tests::svc();
        port2.set_roster(vec![]);
        svc2.park_absent_for_test(
            "m-rep2",
            ident(),
            "qa-bravo",
            "carol",
            "late reply",
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
                r#"<message from="qa-bravo" in-reply-to="m-7f3k">late reply</message>"#.to_string()
            ],
            "a park→flush reply keeps in-reply-to too"
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
        assert_eq!(
            svc.open_request_count(),
            0,
            "a notice does not open a contract"
        );
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
            "do it",
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
            "done",
            Entrance::Mcp,
            &reply_meta("m-req"),
        )
        .expect("delivered");
        assert_eq!(
            svc.open_request_count(),
            0,
            "a reply with the exact id closes the contract"
        );
        assert_eq!(
            svc.ledger_statuses("m-req"),
            vec![DeliveryStatus::Replied],
            "the original record transitions to Replied"
        );
    }

    #[test]
    fn wrong_reply_id_still_delivers_but_closes_nothing() {
        // ★spec §2 strict matching★: bouncing a wrong id would retry an already-delivered message (duplicate) — so no close, delivery and response shape stay.
        let (svc, port) = svc();
        let (_a, alice) = live("alice");
        let (_b, bob) = live("bob");
        port.set_roster(vec![alice, bob]);
        svc.park_absent_for_test(
            "m-req",
            ident(),
            "alice",
            "bob",
            "do it",
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
                "reply with a wrong id",
                Entrance::Mcp,
                &reply_meta("m-nope"),
            )
            .expect("the reply must be delivered normally");
        assert_eq!(out, SendOutcome::Delivered);
        assert_eq!(
            svc.open_request_count(),
            1,
            "a wrong id cannot close the contract"
        );
        assert_eq!(
            svc.ledger_statuses("m-req"),
            vec![DeliveryStatus::Delivered],
            "the original is still unreplied"
        );
        assert_eq!(
            port.injected_bodies().len(),
            2,
            "both the request + the reply were actually injected"
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
            "do it",
            Entrance::Mcp,
            &req_meta("10m", 600),
        )
        .expect("delivered");
        svc.park_absent_for_test(
            "m-r1",
            reply_from(b_id),
            "bob",
            "alice",
            "first",
            Entrance::Mcp,
            &reply_meta("m-req"),
        )
        .expect("delivered");
        svc.park_absent_for_test(
            "m-r2",
            reply_from(b_id),
            "bob",
            "alice",
            "second",
            Entrance::Mcp,
            &reply_meta("m-req"),
        )
        .expect("the second reply must be delivered too");
        assert_eq!(svc.open_request_count(), 0);
        assert_eq!(port.injected_bodies().len(), 3, "all 3 injected");
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
            "do it",
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
            "done",
            Entrance::Mcp,
            &reply_meta("m-req"),
        )
        .expect("delivered");
        assert_eq!(svc.open_request_count(), 0, "the contract closes");
        assert_eq!(
            svc.ledger_statuses("m-req"),
            vec![DeliveryStatus::Pending],
            "the history stays not-injected (illegal edge — observed only as an anomaly)"
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
            "do it",
            Entrance::Mcp,
            &req_meta("10m", 600),
        )
        .expect("recipient absent → parking (the contract opens)");
        assert!(
            port.injected_bodies().is_empty(),
            "absent, so no inject yet"
        );

        svc.sweep(Instant::now() + Duration::from_secs(601));
        assert_eq!(
            port.injected_bodies(),
            vec![
                "<notice>[engram] request m-7f3k deadline (10m) exceeded — no reply from qa-bravo</notice>"
                    .to_string()
            ],
            "notice golden (no from attribute = not something to reply to)"
        );

        svc.sweep(Instant::now() + Duration::from_secs(1200));
        assert_eq!(port.injected_bodies().len(), 1, "the notice exactly once");
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
            "do it",
            Entrance::Mcp,
            &req_meta("1m", 60),
        )
        .expect("delivered");
        svc.park_absent_for_test(
            "m-rep",
            reply_from(b_id),
            "bob",
            "alice",
            "done",
            Entrance::Mcp,
            &reply_meta("m-req"),
        )
        .expect("delivered");
        svc.sweep(Instant::now() + Duration::from_secs(600));
        assert_eq!(
            port.injected_bodies().len(),
            2,
            "a replied contract has no notice (only request + reply, 2 items)"
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
            "do it",
            Entrance::Mcp,
            &meta,
        )
        .expect("parked");
        svc.sweep(Instant::now() + Duration::from_secs(100_000));
        assert!(
            port.injected_bodies().is_empty(),
            "a request without a deadline has no timeout"
        );
        assert_eq!(svc.open_request_count(), 1, "the contract stays open");
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
            .expect("within the cap");
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
            "a message is bounced at the cap (control group)"
        );

        svc.park_absent_for_test(
            "m-req",
            reply_from(tid("alice")),
            "alice",
            "bob",
            "do it",
            Entrance::Mcp,
            &req_meta("1m", 60),
        )
        .expect("parked");
        svc.sweep(Instant::now() + Duration::from_secs(61));

        assert_eq!(
            svc.parked_len(tid("alice")),
            101,
            "a message backlog does not block notices (100 messages + 1 notice — different lanes)"
        );
        let noticed: Vec<_> = svc
            .ledger_snapshot()
            .into_iter()
            .filter(|(_, from, _, _)| from == NOTICE_SENDER_LABEL)
            .collect();
        assert_eq!(
            noticed.len(),
            1,
            "a notice remains in the ledger too (no silent loss)"
        );
        assert_eq!(noticed[0].2, "alice", "notice recipient = request sender");
        assert_eq!(
            noticed[0].3,
            DeliveryStatus::Pending,
            "pending since parked"
        );
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
                "do it",
                Entrance::Mcp,
                &req_meta("1m", 60),
            )
            .expect("parked");
        }
        svc.sweep(Instant::now() + Duration::from_secs(61));

        assert_eq!(
            svc.parked_len(tid("alice")),
            NOTICE_CAP_FOR_TEST,
            "notices stop at their own lane cap (old implementation = cap+1 — unbounded)"
        );
        let noticed: Vec<_> = svc
            .ledger_snapshot()
            .into_iter()
            .filter(|(_, from, _, _)| from == NOTICE_SENDER_LABEL)
            .collect();
        assert_eq!(
            noticed.len(),
            NOTICE_CAP_FOR_TEST + 1,
            "a pushed-out notice remains in the ledger too (0 silent loss — reclaim, not bounce)"
        );
        assert_eq!(
            noticed[0].3,
            DeliveryStatus::Skipped,
            "the oldest notice is reclaimed as skipped (new notices are never bounced)"
        );
        assert!(
            noticed[1..]
                .iter()
                .all(|(_, _, _, s)| *s == DeliveryStatus::Pending),
            "the rest stay in the queue as pending"
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
            "do it",
            Entrance::Mcp,
            &req_meta("1m", 60),
        )
        .expect("parked");
        svc.sweep(Instant::now() + Duration::from_secs(61));
        assert!(
            port.injected_bodies().is_empty(),
            "sender absent too → notice parked"
        );

        let (alice_id, alice) = live("alice");
        port.set_roster(vec![alice]);
        svc.flush_for(alice_id);
        assert_eq!(
            port.injected_bodies(),
            vec![
                "<notice>[engram] request m-9x deadline (1m) exceeded — no reply from bob</notice>"
                    .to_string()
            ],
            "flush delivery is a notice tag too (no from)"
        );
        let noticed: Vec<_> = svc
            .ledger_snapshot()
            .into_iter()
            .filter(|(_, from, _, _)| from == NOTICE_SENDER_LABEL)
            .collect();
        assert_eq!(
            noticed[0].3,
            DeliveryStatus::Delivered,
            "delivered at the actual inject time"
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
            "do it",
            Entrance::Mcp,
            &req_meta("1m", 60),
        )
        .expect("parked");
        svc.sweep(Instant::now() + Duration::from_secs(61));
        assert!(
            port.injected_bodies().is_empty(),
            "if the sender is in a turn, the notice is not injected either"
        );
        assert_eq!(
            svc.parked_len(tid("alice")),
            1,
            "the notice parked addressed to alice"
        );

        gate.clear();
        svc.flush_for(alice_id);
        assert_eq!(
            port.injected_bodies(),
            vec!["<notice>[engram] request m-b1 deadline (1m) exceeded — no reply from ghost</notice>".to_string()],
            "delivered by the turn-end flush"
        );
    }

    // ── C3 review round fix ────────────────────────────────────────────────────────────

    #[test]
    fn timeout_notice_reaches_a_renamed_sender_by_its_id() {
        // ★fix 2 regression★: a notice is notified and never fires again — unable to reach a renamed sender it would be stranded forever; the queue key is the sender id, so it arrives regardless of rename (ADR-0309).
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
                node: None,
            },
            "alice",
            "ghost",
            "do it",
            Entrance::Mcp,
            &req_meta("1m", 60),
        )
        .expect("parked (recipient absent)");

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
                "<notice>[engram] request m-req deadline (1m) exceeded — no reply from ghost</notice>".to_string()
            ],
            "must be delivered to a renamed sender too (queue key = sender id)"
        );
        assert_eq!(port.injected_targets(), vec![alice_id]);
        assert_eq!(
            svc.parked_len(alice_id),
            0,
            "the queue is empty = it actually went out"
        );
    }

    /// ★The sweep does not read the roster (ADR-0309 — protects the sweep task)★: a notice parks in the sender id's queue and presses the doorbell by that id; injection belongs to the flush lane (`deliver_notice`).
    ///   (Name-resolving days took one roster snapshot per tick — the predecessor of the test counting that cost.)
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
                "do it",
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
            "the sweep does not read the roster regardless of the due count"
        );
        let mut rung = bell.take();
        rung.sort();
        let mut expected: Vec<PeerId> = senders.iter().map(|s| s.peer_id).collect();
        expected.sort();
        assert_eq!(
            rung, expected,
            "one doorbell with that sender id per notice"
        );
        for s in &senders {
            assert_eq!(
                svc.parked_len(s.peer_id),
                1,
                "the notice is parked in the sender id's queue"
            );
        }
        assert!(
            port.injected_bodies().is_empty(),
            "the sweep does not inject"
        );

        // The lane consumes that doorbell.
        port.set_roster(vec![live_with_id(senders[0].peer_id, "sender0").1]);
        svc.flush_for_agent(senders[0].peer_id);
        assert_eq!(
            port.injected_bodies(),
            vec!["<notice>[engram] request m-r0 deadline (1m) exceeded — no reply from ghost</notice>".to_string()],
        );
    }

    /// ★A reply arriving after the due computation and before notice parking drops that notice (review fix 5)★. The race point = the earlier notice's inline flush taking the roster (`arm_on_next_roster`) — both dues
    ///   are already `notified` and the second notice is not yet parked. The sweep itself does not read the roster, so the gap exists only with two dues.
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
                "do it",
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
                "done",
                Entrance::Mcp,
                &reply_meta("m-req1"),
            )
            .expect("parked (carol absent)");
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
            "the notice of m-req1, whose reply arrived first, is cancelled; the earlier m-req0 notice stays"
        );
        assert_eq!(
            svc.contract_outcome_for_test("m-req1", tid("bob")),
            Some("replied"),
            "m-req1 was closed by the reply"
        );
        assert_eq!(svc.open_request_count(), 1, "only m-req0 is open");
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
            "a notice is bounced on an id collision too"
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
            "a reply is bounced on an id collision too"
        );
        assert_eq!(
            svc.ledger_snapshot().len(),
            before,
            "no ledger record increase"
        );
        assert_eq!(
            port.injected_bodies().len(),
            injected_before,
            "no inject (0 side effects)"
        );
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "ingress is the only validator")]
    fn mutually_exclusive_contract_fields_trip_the_debug_assert() {
        // ★fix 11★: this function is pub, so a wiring that violates mutual exclusion blows up in debug (the bounce wording is canonical in ingress, so validation is not duplicated).
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
                node: None,
            },
            entrance: Entrance::Mcp,
            body: "hi\nsecond line".to_string(),
            meta: req_meta("10m", 600),
        };
        let encoded = good.encode();
        // Control: a normal payload.
        let ok = ParkPayload::decode(&encoded);
        assert_eq!(ok.body, "hi\nsecond line");
        assert_eq!(ok.sender_name, "qa");
        assert!(ok.meta.request);

        // The tag is always at the very front so it is cut by position — the old `replacen('1', …)` hit a wrong spot when the version char changed and silently became a valid payload.
        let unknown_version = {
            let mut s = encoded.clone();
            s.replace_range(0..PARK_PAYLOAD_VERSION.len(), "9");
            s
        };
        // v2 layout: ver·sender_len·rb_len·rt_len·grp_len·uuid·epoch·entrance·flags 9 lines + the rest.
        for corrupt in [
            // No header at all.
            "just a body".to_string(),
            // The version tag is an unknown value (format drift).
            unknown_version,
            // A length field is not a number.
            "2\nx\n0\n0\n0\n00000000-0000-0000-0000-000000000000\n0\nmcp\n-\nbody".to_string(),
            // The sum of lengths is larger than the remaining string (truncation).
            "2\n99\n0\n0\n0\n00000000-0000-0000-0000-000000000000\n0\nmcp\n-\nshort".to_string(),
            // ★Cut mid char boundary★: the first Hangul syllable U+D55C is 3 bytes in UTF-8 but sender_len=1, so it splits the character.
            "2\n1\n0\n0\n0\n00000000-0000-0000-0000-000000000000\n0\nmcp\n-\n한글".to_string(), // hangul-ok
            // A case where the reply_to length makes the boundary misaligned (the second cut point).
            "2\n0\n1\n0\n0\n00000000-0000-0000-0000-000000000000\n0\nmcp\n-\n한글".to_string(), // hangul-ok
            // ★The group label cut point (C4 — the fourth cut point)★.
            "2\n0\n0\n0\n1\n00000000-0000-0000-0000-000000000000\n0\nmcp\n-\n한글".to_string(), // hangul-ok
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
                "a broken payload falls back to the whole as body (no silent loss): {corrupt:?}"
            );
            assert!(
                !p.meta.request,
                "the fallback does not claim contract attributes"
            );
        }
    }

    #[test]
    fn one_corrupt_parked_item_does_not_abort_the_flush_batch() {
        let (svc, port) = svc();
        port.set_roster(vec![]);
        for (id, body) in [("m-a", "first"), ("m-b", "second"), ("m-c", "third")] {
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
        assert_eq!(bodies.len(), 3, "one broken item does not cut the batch");
        assert!(bodies[0].contains("first") && bodies[2].contains("third"));
        assert!(
            bodies[1].contains("CORRUPT"),
            "a broken item goes out even as a fallback envelope (no silent loss): {}",
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
        assert_eq!(
            drawn.id, "m-fresh",
            "the colliding id is discarded and a new id is used"
        );
        assert_eq!(
            drawn.collided.as_deref(),
            Some("m-taken"),
            "the colliding one is the investigation clue — the log prints that value"
        );
        assert!(!drawn.still_colliding);

        // ② The second one collides too.
        let mut both = vec!["m-taken".to_string(), "m-taken".to_string()];
        let drawn = svc.draw_daemon_msg_id_with(|| both.pop().expect("draw"));
        assert_eq!(
            drawn.id, "m-taken",
            "observational pollution is better than losing a notice"
        );
        assert!(
            drawn.still_colliding,
            "if even the re-draw collides, a signal (log target)"
        );

        // ③ Control: no collision.
        let mut once = vec!["m-new".to_string()];
        let drawn = svc.draw_daemon_msg_id_with(|| once.pop().expect("draw must happen only once"));
        assert_eq!(drawn.id, "m-new");
        assert!(drawn.collided.is_none());
    }

    /// ★H3★: a victim under provisional retirement may be in neither tracking nor history (when the history was already pushed out), and if
    /// the same id is newly issued in that window, the restore loses the original. So the reservation set must take part in the issuance check.
    #[test]
    fn the_mint_path_regenerates_when_it_draws_a_provisionally_retired_id() {
        // ★After ADR-0125 the window opens differently★: the reservation window is inside the enqueue lock and `draw_daemon_msg_id_with` takes the same lock, so the `on_inject` hook cannot see it; the reservation guard
        //   is opened with the production verb and the test runs outside it.
        let (svc, port) = svc();
        let (boss_from, boss) = live_sender("boss");
        port.set_roster(vec![boss]);
        fill_open_request_cap_evictable(&svc, boss_from);
        let cap = svc.occupied_slots_for_test();
        assert!(cap > 0, "filled up to the cap");
        assert!(
            svc.msg_id_in_use_for_test("cap0"),
            "premise: a fixture contract to become the victim exists"
        );

        // After the reservation (retirement mark) · before commit — while this guard is alive is the provisional window.
        let res = reserve_marked_contract(&svc, boss_from, "m-provisional");
        // ★C2★: the old `msg_id_in_use("cap0")` assertion also held because of a history-ring row, so it stayed green with reservation visibility removed — the window is observed directly by the tracking count (cap + 1 → cap).
        assert_eq!(
            svc.open_request_count(),
            cap + 1,
            "provisional window: the marked victim and the new contract must be tracked at the same time"
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
            "after commit (outcome settled), the victim is physically removed and it returns to the cap"
        );
        assert!(
            in_use,
            "cap0, provisionally retiring, must be 'in use' in the issuance check (H3)"
        );
        assert_eq!(
            drawn_id, "m-fresh",
            "so the issuance path discards that id and switches to a new id"
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
        assert_eq!(
            svc.ledger_snapshot().len(),
            before,
            "no ledger record increase"
        );
        assert_eq!(svc.parked_len(tid("bob")), 1, "no parking increase");
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
            "do it",
            Entrance::Mcp,
            &req_meta("10m", 600),
        )
        .expect("request");
        let view = svc.message_state("m-req", t0).expect("query");
        assert!(view.awaiting_reply, "waiting before the reply");
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
            "done",
            Entrance::Mcp,
            &reply_meta("m-req"),
        )
        .expect("reply");
        let view = svc.message_state("m-req", t0).expect("query");
        assert!(
            !view.awaiting_reply,
            "no longer waiting after the reply arrives"
        );
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
        .expect("send");
        let view = svc
            .message_state("m1", t0 + Duration::from_secs(90))
            .expect("query");
        assert!(
            (85..=95).contains(&view.rows[0].age_secs),
            "elapsed since send ≈ 90 seconds: {}",
            view.rows[0].age_secs
        );
        let view = svc
            .message_state("m1", t0 - Duration::from_secs(10))
            .expect("query");
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
        .expect("park");
        svc.park_absent_for_test(
            "m-out",
            alice_from,
            "alice",
            "bob",
            "do it",
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
            "do this",
            Entrance::Mcp,
            &req_meta("30m", 1800),
        )
        .expect("received request");
        port.set_roster(vec![bob_agent]);
        svc.park_absent_for_test(
            "m-other",
            carol_from,
            "carol",
            "bob",
            "you do it",
            Entrance::Mcp,
            &req_meta("10m", 600),
        )
        .expect("someone else's request");

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
            "the three directions are told apart by tag and others' contracts are left out: {items:?}"
        );
        assert_eq!(
            items[0].reply_by.as_deref(),
            Some("10m"),
            "original deadline notation"
        );
        assert_eq!(items[2].from, "carol", "the one I must answer");
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
            "bob only the two he must answer: {bob_items:?}"
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
            "done",
            Entrance::Mcp,
            &reply_meta("m-a"),
        )
        .expect("reply");
        svc.sweep(t0 + Duration::from_secs(601));

        let items = svc.open_items_for(alice_from.peer_id, t0 + Duration::from_secs(601));
        let ids: Vec<&str> = items.iter().map(|i| i.id.as_str()).collect();
        assert_eq!(
            ids,
            vec!["m-b"],
            "only the replied contract is left out: {items:?}"
        );
        assert!(
            items[0].timed_out,
            "the deadline-exceeded fact shows as a flag but it stays in the list"
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
            "do it",
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
            "targeted twin A bears the duty: {a_items:?}"
        );
        let b_items = svc.open_items_for(twin_b_from.peer_id, t0);
        assert!(
            b_items.is_empty(),
            "twin B, who never received it, must not get the duty (B1): {b_items:?}"
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

    /// ★Sent waiting items are found by sender id — they stay after a rename and the row shows the send-time name (ADR-0309 decisions 4·5, TRD S29 B3)★. By-name comparison lost the pre-rename items and showed them to an agent that took the old name.
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
        .expect("park");
        assert_eq!(
            svc.parked_len(tid("bob")),
            1,
            "premise: parked addressed to asleep bob"
        );

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
            "the waiting item sent before the rename remains and the row has the name at send time: {items:?}"
        );
        assert!(
            svc.open_items_for(other_id, Instant::now()).is_empty(),
            "not visible to another agent that newly has the old name"
        );
    }

    /// ★Fills the open-contract limit with **non-retirable** contracts (A7 — reproduces `REQUEST_CAPACITY`)★.
    fn fill_open_request_cap(svc: &Arc<MessagingService>, from: SenderIdentity) {
        fill_open_request_cap_with(svc, from, &req_meta("10m", 600));
    }

    /// ★Fills to the limit **leaving one slot** with non-retirable contracts (H2 — A3's real window)★: if the caller fills it with 1 retirable contract, the only retirable candidate the second recipient of the next send
    ///   can see is the first recipient's contract that send just opened (the only batch where self-erosion occurs).
    fn fill_open_request_cap_leaving_one_free(svc: &Arc<MessagingService>, from: SenderIdentity) {
        fill_open_request_cap(svc, from);
        let last = svc
            .open_items_for(from.peer_id, Instant::now())
            .into_iter()
            .filter(|i| i.direction == Direction::AwaitingTheirReply)
            .last()
            .expect("there are filled contracts");
        svc.close_contract_for_test(&last.id, tid(&last.to));
        assert!(
            svc.occupied_slots_for_test() > 0,
            "premise: filled up near the cap"
        );
    }

    /// ★Fills the limit with **retirable** contracts (A2/A3 — reproduces victim exchange)★: with no deadline (the daemon owes no notice) a contract is a retirement candidate.
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

    /// ★Do not duplicate the constant (A7)★ — growth stops at the limit, so the fixture follows a changed limit. ★Not `open_request_count` (= `!closed`)★: a victim marked for retirement frees a slot but stays in that
    ///   count and grows without bound (the loop never ends).
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
            assert!(
                i < 100_000,
                "the cap is not observed — fixture premise collapsed"
            );
        }
    }

    /// ★The reply duty stays with the agent (id) that received the request — a different agent with the same name moves neither the letter nor the duty (ADR-0309 decisions 1·5·7)★. A device re-stamping the duty onto the
    ///   landing side or a name-lookup branch lets B take over A's duty and A sees "nothing to answer".
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
            "do it",
            Entrance::Mcp,
            &req_meta("10m", 600),
        )
        .expect("busy recipient → parked request");
        assert_eq!(svc.parked_len(a_agent.id), 1, "premise: parked");

        let (b_from, b_agent) = live_sender_distinct("worker");
        port.set_roster(vec![boss, b_agent.clone()]);
        gate.clear();
        svc.flush_for(b_agent.id);
        assert!(
            port.injected_bodies().is_empty(),
            "same-named B does not receive a letter addressed to A (B2)"
        );
        assert_eq!(svc.parked_len(a_agent.id), 1, "stays in A's queue");

        let b_items = svc.open_items_for(b_from.peer_id, t0);
        assert!(
            b_items.is_empty(),
            "same-named B, who never received the request, must not get the duty: {b_items:?}"
        );
        let a_items = svc.open_items_for(a_from.peer_id, t0);
        assert_eq!(
            a_items
                .iter()
                .map(|i| (i.direction.as_str(), i.id.as_str()))
                .collect::<Vec<_>>(),
            vec![("reply_owed_by_me", "m-req")],
            "the duty stays on targeted A's id: {a_items:?}"
        );
        svc.handle_send(
            "m-rep",
            b_from,
            "worker",
            &["boss".to_string()],
            "done",
            Entrance::Mcp,
            &reply_meta("m-req"),
        )
        .expect("row response");
        assert_eq!(
            svc.contract_outcome_for_test("m-req", a_agent.id),
            Some("awaiting_reply"),
            "B's reply cannot close A's contract"
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
            "boss waits for the reply, and the letter, not yet received by A, also shows as waiting: {boss_items:?}"
        );
    }

    #[test]
    fn a_reply_racing_its_own_injection_still_finds_the_contract() {
        // ★A reply arriving concurrently with injection also finds the contract★ — the contract holds the receiving id from the moment it opens, independent of injection order (ADR-0309); the receiver is the same agent
        //   restarted (`live_sender("worker")` twice = same id). ★Deterministic race★: `on_inject` is called right before the write, so sending a reply inside it exactly simulates a concurrent reply (no sleep or thread).
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
            "do it",
            Entrance::Mcp,
            &req_meta("10m", 600),
        )
        .expect("busy recipient → parked request");
        assert_eq!(
            svc.parked_len(tid("worker")),
            1,
            "premise: parked in that id's queue"
        );
        assert_eq!(svc.open_request_count(), 1, "premise: 1 contract open");

        // A new incarnation of the same agent (same id).
        let b_agent = LiveAgent {
            epoch: 1,
            ..a_agent.clone()
        };
        let b_id = b_agent.id;
        port.set_roster(vec![boss.clone(), b_agent.clone()]);
        gate.clear();

        // idx 0 = flush's injection; the reply's own is idx 1, so it is filtered out — re-entering would recurse infinitely.
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
                    "all done",
                    Entrance::Mcp,
                    &reply_meta("m-req"),
                )
                .expect("the reply send must succeed");
            }));
        }
        svc.flush_for(b_id);

        let bodies = port.injected_bodies();
        assert_eq!(
            bodies.len(),
            2,
            "1 flush delivery + 1 reply (premise — the reply actually went out): {bodies:?}"
        );
        // The reply is recorded before the outer write, so the order assertion is by presence, not by index.
        assert!(
            bodies.iter().any(|b| b.contains(r#"in-reply-to="m-req""#)),
            "the reply envelope must have actually gone out: {bodies:?}"
        );
        assert!(
            bodies.iter().any(|b| b.contains(r#"type="request""#)),
            "the parked request must have been delivered too: {bodies:?}"
        );

        assert_eq!(
            svc.open_request_count(),
            0,
            "the reply that arrived during the inject must have closed the contract"
        );
        assert!(
            svc.open_items_for(b_id, t0).is_empty(),
            "the replying side has no remaining duty either"
        );
        assert!(
            svc.open_items_for(boss_from.peer_id, t0).is_empty(),
            "the sender no longer waits either"
        );

        svc.sweep(t0 + Duration::from_secs(700));
        assert_eq!(
            port.injected_bodies().len(),
            2,
            "a closed contract has no deadline notice: {:?}",
            port.injected_bodies()
        );
        assert_eq!(
            svc.parked_len(tid("boss")),
            0,
            "the notice does not leak into parking either"
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
        .expect("send");
        let view = svc.message_state("m1", t0).expect("query");
        assert!(
            !view.may_be_truncated,
            "before evict, rows can be asserted to be everything"
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
        .expect("delivery complete");
        assert!(
            svc.open_items_for(from.peer_id, Instant::now()).is_empty(),
            "a delivered notice is not outstanding"
        );
    }

    // ══════════════════════════════════════════════════════════════════════════════════════════
    // S18 send overhaul acceptance criteria (spec §7) — ADR-0111/0112/0114
    // ★Canonical for★: absence · duplicate name = per-recipient failure row · multi-recipient fan-out (one snapshot, one path) · `@all` = everyone but me · partial progress · envelope `to` freeze · multi-request N contracts ·
    //   binding/reclaim abolished. Loosening an assertion here is a **contract change** (needs user ratification).
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

    /// A notification send with a given body — for asserting **injection order in bytes** (identical bodies cannot be told apart in a batch).
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

        let out =
            send(&svc, "m1", from, "alice", &["ghost"]).expect("row response (not a whole bounce)");
        assert_eq!(
            rows(&out),
            vec![(
                "ghost".to_string(),
                SendStatus::Failed,
                Some(FailCode::RecipientNotFound)
            )],
            "the answer to a send to 1 absent recipient is 1 failed row, not an error (one path — spec §5)"
        );
        assert!(
            out[0].hint.is_some(),
            "a failure row requires code + hint (spec §6)"
        );

        assert_eq!(
            svc.parked_total_for_test(),
            0,
            "not put in the parking queue"
        );
        assert!(
            svc.ledger_statuses("m1")
                .iter()
                .all(|s| *s != DeliveryStatus::Pending),
            "the ledger must have no pending record (never parked)"
        );
        assert_eq!(
            svc.ledger_statuses("m1"),
            vec![DeliveryStatus::Failed],
            "only 1 terminal `failed` row remains"
        );

        let view = svc
            .message_state("m1", Instant::now())
            .expect("a failure row can be queried too");
        assert_eq!(view.rows.len(), out.len());
        assert!(
            !view.may_be_truncated,
            "the expected row count equals the remaining row count"
        );
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
            "the `@all` expansion result and direct targeting must have **the same outcome** (one path — ADR-0111 decision 4)"
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
            "the roster is queried once per send regardless of the recipient count (ADR-0111 decision 2)"
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
            "if only the sender is alive, @all has a final set of 0 → whole bounce"
        );
        assert!(
            svc.ledger_snapshot().is_empty(),
            "a whole bounce has 0 side effects (no ledger record)"
        );

        let out = send(&svc, "m2", from, "alice", &["@all", "alice"]).expect("not a bounce");
        assert_eq!(
            rows(&out),
            vec![("alice".to_string(), SendStatus::Delivered, None)],
            "a directly targeted self-send survives (the exclusion applies **only to expansion** — spec §4)"
        );

        port.set_dormant(&["sleepy"]);
        let all = send(&svc, "m3", from, "alice", &["@all"])
            .expect("not a bounce if there is an asleep name");
        assert_eq!(
            rows(&all),
            vec![("sleepy".to_string(), SendStatus::Pending, None)],
            "@all expands asleep names and parks them (the sender is still excluded): {all:?}"
        );
        assert_eq!(
            send(&svc, "m4", from, "alice", &["@here"]),
            Err(SendReject::GroupEmpty),
            "@here does not see asleep names, so still a whole bounce"
        );
    }

    #[test]
    fn an_explicit_dormant_token_and_at_all_fold_into_one_row() {
        let (svc, port) = svc();
        let (from, me) = live_sender("alice");
        let (_b, bob) = live("bob");
        port.set_roster(vec![me, bob]);
        port.set_dormant(&["sleepy"]);

        let out = send(&svc, "m1", from, "alice", &["sleepy", "@all"]).expect("not a bounce");
        assert_eq!(
            rows(&out),
            vec![
                ("sleepy".to_string(), SendStatus::Pending, None),
                ("bob".to_string(), SendStatus::Delivered, None),
            ],
            "an asleep name leaves **1 row** only, at the explicit targeting spot (the expanded part is absorbed): {out:?}"
        );
        assert_eq!(
            svc.parked_len(tid("sleepy")),
            1,
            "parking must be 1 item too (no double delivery)"
        );
        assert_eq!(
            svc.ledger_statuses("m1"),
            vec![DeliveryStatus::Pending, DeliveryStatus::Delivered],
            "ledger row count = response row count (2 rows)"
        );
    }

    #[test]
    fn at_all_delivers_to_each_live_twin_in_its_own_row() {
        // ★Expansion resolves directly by registry id (ADR-0309 decision 3, TRD S29 §4 X1)★ — two live sessions with one name are different recipients; re-resolving by name folds them into one `RECIPIENT_AMBIGUOUS` row and neither receives it.
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
            "twins each get a row · both delivered"
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
            "the explicit token (zed) first → then the expansion results alphabetically (bob, carol). zed is only one line"
        );
        assert_eq!(
            out.len(),
            3,
            "writing a name already in @all again does not add rows"
        );
        assert_eq!(
            port.injected_bodies().len(),
            3,
            "1 delivery to an overlapping recipient"
        );
    }

    #[test]
    fn a_name_typo_fails_only_its_own_row_while_live_recipients_still_receive() {
        let (svc, port) = svc();
        let (from, me) = live_sender("alice");
        let (_b, bob) = live("bob");
        port.set_roster(vec![me, bob]);

        let out = send(&svc, "m1", from, "alice", &["nonexistent-name", "bob"])
            .expect("partial progress");
        assert_eq!(
            rows(&out),
            vec![
                (
                    "nonexistent-name".to_string(),
                    SendStatus::Failed,
                    Some(FailCode::RecipientNotFound)
                ),
                ("bob".to_string(), SendStatus::Delivered, None),
            ]
        );
        assert_eq!(
            port.injected_bodies().len(),
            1,
            "goes to the live recipient as is"
        );

        let before = port.injected_bodies().len();
        assert!(matches!(
            send(&svc, "m2", from, "alice", &["@typo", "bob"]),
            Err(SendReject::GroupNotFound { .. })
        ));
        assert_eq!(
            port.injected_bodies().len(),
            before,
            "a whole bounce has 0 deliveries"
        );
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
            .expect("partial progress is Ok");
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
            "all rows failed"
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
            let out =
                send(&svc, &format!("f{i}"), from, "alice", &["full"]).expect("within the cap");
            assert_eq!(out[0].status, SendStatus::Pending, "#{i}");
        }
        assert_eq!(svc.parked_len(tid("full")), 100);
        let oldest_before = svc.parked_msg_ids(tid("full"))[0].clone();

        let out = send(&svc, "over", from, "alice", &["full", "carol"]).expect("partial progress");
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
            "one full recipient does not block the rest of the delivery (no escalation to a whole bounce)"
        );
        assert_eq!(svc.parked_len(tid("full")), 100, "queue length unchanged");
        assert_eq!(
            svc.parked_msg_ids(tid("full"))[0],
            oldest_before,
            "the oldest item is still there = **no reclaim attempt** (ADR-0114 decision 1)"
        );
        assert_eq!(
            svc.ledger_statuses("over"),
            vec![DeliveryStatus::Failed, DeliveryStatus::Delivered],
            "a failed recipient leaves a terminal ledger row too"
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
                "do it",
                Entrance::Mcp,
                &req_meta("10m", 600),
            )
            .expect("ok");
        assert!(out.iter().all(|r| r.status == SendStatus::Delivered));
        assert_eq!(svc.open_request_count(), 2, "1 contract per recipient");

        let bob_from = SenderIdentity {
            peer_id: b_id,
            epoch: 0,
            node: None,
        };
        svc.handle_send(
            "m-rep",
            bob_from,
            "bob",
            &["alice".to_string()],
            "done",
            Entrance::Mcp,
            &reply_meta("m-req"),
        )
        .expect("reply delivery");
        assert_eq!(
            svc.open_request_count(),
            1,
            "no reply-all — carol's contract stays open"
        );

        svc.sweep(Instant::now() + Duration::from_secs(601));
        let notices: Vec<_> = svc
            .ledger_snapshot()
            .into_iter()
            .filter(|(_, from, _, _)| from == NOTICE_SENDER_LABEL)
            .collect();
        assert_eq!(
            notices.len(),
            1,
            "a replied contract has no notice: {notices:?}"
        );
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
                "everyone stand by",
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
            vec![r#"<message from="alice" to="@all">everyone stand by</message>"#.to_string()],
            "2 accepted → to exposed · @addresses kept as tokens without expanding · failed tokens excluded"
        );

        gate.clear();
        svc.flush_for(b_id);
        assert_eq!(
            port.injected_bodies().len(),
            2,
            "the parked item was delivered by the flush"
        );
        assert_eq!(
            port.injected_bodies()[1],
            r#"<message from="alice" to="@all">everyone stand by</message>"#,
            "the notation at send time as is (the park payload carries the frozen value up to the flush)"
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
            "1 accepted → to attribute omitted"
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
            "in the order the sender wrote (@all first, then the explicit name): {:?}",
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
            send(&svc, &format!("p{i}"), from, "alice", &["recv"]).expect("parking within the cap");
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
                .expect("row response");
            seen_h.lock().unwrap().push(out[0].status);
        }));
        svc.flush_for(r_id);

        assert_eq!(
            during.lock().unwrap().as_slice(),
            &[SendStatus::Failed],
            "new inflow while a batch is out is caught by the denominator (queue + in-flight) and bounced (F1)"
        );
        assert_eq!(
            svc.ledger_statuses("concurrent"),
            vec![DeliveryStatus::Failed],
            "a bounce also remains in the ledger as a failure row (ADR-0111 — the reverse of the old 'bounces are not recorded in the ledger')"
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
            send(&svc, &format!("p{i}"), from, "alice", &["recv"]).expect("park");
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
            "no overlapping runs, so the batch runs only once, yet everything goes out"
        );
        assert!(
            bell.take().contains(&r_id),
            "a deferred wake-up **must be rung again** at settlement (no lost wakeup)"
        );
    }

    // ══════════════════════════════════════════════════════════════════════════════════════════
    // /review code deep — fix round 1 regression net (A2·A3·A5·A6·A7·A8 · C2·C4)
    //   ★A1 (reply contract handling) was replaced by the ratified rule★ (ADR-0116 decision 2, ADR-0118) — guarded by the two tests below (confirmed unreachable → `reply_failed` / transient → stays open + retry closes it).
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
            "do it",
            Entrance::Mcp,
            &req_meta("10m", 600),
        )
        .expect("delivery");
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
            node: None,
        };
        let rows_out = svc
            .handle_send(
                "m-rep",
                worker_from,
                "worker",
                &["boss".to_string()],
                "done",
                Entrance::Mcp,
                &reply_meta("m-req"),
            )
            .expect("row response");
        assert_eq!(
            rows_out[0].code,
            Some(FailCode::RecipientNotFound),
            "a reply does not go to a nonexistent requester"
        );
        assert_eq!(
            svc.open_request_count(),
            0,
            "a reply confirmed unreachable **terminates** the contract as reply_failed (ADR-0116 decision 2)"
        );
        assert_eq!(
            svc.contract_outcome_for_test("m-req", tid("worker")),
            Some("reply_failed"),
            "the terminal word is not replied — it does not claim the reply went through (spec §6 axis distinction)"
        );
        assert_eq!(
            svc.occupied_slots_for_test(),
            slots_before - 1,
            "left out of the 512 count (ADR-0118 decision 4 — so zombies do not eat the cap)"
        );
        let owed = svc.open_items_for(w_id, Instant::now());
        assert!(
            !owed.iter().any(|i| i.id == "m-req"),
            "a terminated contract must not be in the outstanding list: {owed:?}"
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
            "no deadline-exceeded notice must go out for a terminated contract"
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
            "do it",
            Entrance::Mcp,
            &req_meta("10m", 600),
        )
        .expect("delivery");

        gate.set_busy(boss_id, 0);
        for i in 0..100 {
            send(&svc, &format!("fill{i}"), boss_from, "boss", &["boss"]).expect("park");
        }
        let worker_from = SenderIdentity {
            peer_id: w_id,
            epoch: 0,
            node: None,
        };
        let rows_out = svc
            .handle_send(
                "m-rep",
                worker_from,
                "worker",
                &["boss".to_string()],
                "done",
                Entrance::Mcp,
                &reply_meta("m-req"),
            )
            .expect("row response");
        assert_eq!(
            rows_out[0].code,
            Some(FailCode::MailboxFull),
            "the mailbox is full, so this reply is a failure row"
        );
        assert_eq!(
            svc.contract_outcome_for_test("m-req", tid("worker")),
            Some("awaiting_reply"),
            "a failure for a temporary reason is a **no-op** — the contract stays open (ADR-0118 decision 2)"
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
                "done (retry)",
                Entrance::Mcp,
                &reply_meta("m-req"),
            )
            .expect("row response");
        assert_eq!(
            retry[0].status,
            SendStatus::Delivered,
            "the retry is delivered: {retry:?}"
        );
        assert_eq!(
            svc.contract_outcome_for_test("m-req", tid("worker")),
            Some("replied"),
            "a successful retry closes the contract on the normal path (the reason for staying open)"
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
            "do it",
            Entrance::Mcp,
            &req_meta("10m", 600),
        )
        .expect("delivery");

        let (_b2, boss_twin) = live_distinct("boss");
        let (_w2, worker_again) = live("worker");
        port.set_roster(vec![boss, boss_twin, worker_again]);
        let worker_from = SenderIdentity {
            peer_id: w_id,
            epoch: 0,
            node: None,
        };
        let rows_out = svc
            .handle_send(
                "m-rep",
                worker_from,
                "worker",
                &["boss".to_string()],
                "done",
                Entrance::Mcp,
                &reply_meta("m-req"),
            )
            .expect("row response");
        assert_eq!(
            rows_out[0].code,
            Some(FailCode::RecipientAmbiguous),
            "cannot deliver to a same-named requester: {rows_out:?}"
        );
        assert_eq!(
            svc.contract_outcome_for_test("m-req", tid("worker")),
            Some("awaiting_reply"),
            "a same-name failure is a **no-op** — the contract stays open (once the user sorts out the names, a retry closes it)"
        );
        assert_eq!(
            svc.open_request_count(),
            1,
            "not left out of the 512 count (not a termination)"
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
            let r = res
                .retired
                .as_ref()
                .expect("premise: the victim was marked");
            (r.request_id.clone(), r.recipient_id)
        };

        {
            let mut st = svc.state.lock().expect("lock");
            let now = Instant::now();
            // ① Push the victim's history row out of the ring (still open, so not an evict cleanup target); the ring capacity constant is not duplicated — it loops until that row vanishes.
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
                assert!(
                    i < 100_000,
                    "the ring rotation is not observed — fixture premise collapsed"
                );
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
            "premise: the victim vanished before commit (purge) — if this premise breaks, the assertions below are meaningless"
        );

        let mut log = RetirementLog::default();
        {
            let mut st = svc.state.lock().expect("lock");
            res.commit(&mut st, &mut log);
        }
        assert!(
            log.real.is_empty(),
            "if the commit actually removed nothing, it is not counted as a retirement: {:?}",
            log.real
        );
        assert_eq!(
            log.phantom.len(),
            1,
            "it remains as an anomaly (ghost plan) instead"
        );
        log_contract_retirements("m-phantom", &log.real);
        let reports = retirement_reports::drain();
        assert!(
            reports.is_empty(),
            "if the commit actually removed nothing, there must be no retirement report either (no ghost retirement): {reports:?}"
        );
        assert!(
            svc.contract_tracked_for_test("m-phantom", tid("ghost-worker")),
            "the reservation's own contract must be alive"
        );
        assert_eq!(svc.marked_retirements_for_test(), 0, "no mark remains");
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
                "do it",
                Entrance::Mcp,
                &SendMeta {
                    request: true,
                    ..SendMeta::default()
                },
            )
            .expect("row response");
        assert_eq!(
            out[0].status,
            SendStatus::Delivered,
            "premise: the immediate delivery branch"
        );
        let reports = retirement_reports::drain();
        assert_eq!(
            reports.len(),
            1,
            "1 retirement must be reported in exactly 1 line — if the metering is before the outcome loop, it is 0 lines: {reports:?}"
        );
        assert!(
            !reports[0].0.is_empty() && reports[0].0 != "m-retire",
            "the report must point at **the old contract that was evicted** (not the new send): {reports:?}"
        );
        assert!(
            !svc.contract_tracked_for_test(&reports[0].0, reports[0].1),
            "the reported contract must have actually vanished (must not report something that did not happen)"
        );

        // ② A send that ends in failure (fill the limit with **non-retirable** ones → RequestCapacity — needs a new service so ①'s retirable contract is not picked as a victim).
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
                "do it",
                Entrance::Mcp,
                &req_meta("10m", 600),
            )
            .expect("row response");
        assert_eq!(
            out2[0].code,
            Some(FailCode::RequestCapacity),
            "premise: failure row due to cap saturation: {out2:?}"
        );
        assert!(
            retirement_reports::drain().is_empty(),
            "reporting a retirement that did not happen sends the operator chasing a ghost retirement"
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
                send(&svc, &format!("filler{i}"), from, "boss", &["worker"]).expect("park");
            }
            fill_open_request_cap_evictable(&svc, from);
            let occupied_before = svc.occupied_slots_for_test();
            let tracking_before = svc.tracking_len_for_test();
            assert_eq!(
                svc.marked_retirements_for_test(),
                0,
                "premise: no mark remains"
            );
            let _ = retirement_reports::drain();

            let out = svc
                .handle_send(
                    "m-late",
                    from,
                    "boss",
                    &["lead".to_string(), "worker".to_string()],
                    "do it",
                    Entrance::Mcp,
                    &req_meta("10m", 600),
                )
                .expect("row response");
            let worker_row = out.iter().find(|r| r.to == "worker").expect("worker row");
            assert_eq!(
                worker_row.code,
                Some(FailCode::MailboxFull),
                "a recipient with a full mailbox must end as a failure row at the cap gate: {out:?}"
            );
            let lead_row = out.iter().find(|r| r.to == "lead").expect("lead row");
            assert_eq!(
                lead_row.status,
                SendStatus::Delivered,
                "premise: the earlier recipient is accepted normally and actually retires its own victim: {out:?}"
            );
            assert_eq!(
                svc.marked_retirements_for_test(),
                0,
                "a request that ended in failure **retires nobody** — no mark may remain"
            );
            assert_eq!(
                svc.occupied_slots_for_test(),
                occupied_before,
                "the cap denominator must be unchanged (the success offsets −victim +new) — if the failure's mark remains, the denominator shrinks forever"
            );
            assert_eq!(
                svc.tracking_len_for_test(),
                tracking_before,
                "no provisional contract may remain either — if one does, tracking grows unbounded"
            );
            assert!(!svc.contract_tracked_for_test("m-late", tid("worker")));
            assert!(
                svc.contract_tracked_for_test("m-late", tid("lead")),
                "a failed recipient's rollback must not take away the contract of a successful recipient of the same send"
            );
            let reports = retirement_reports::drain();
            assert_eq!(
                reports.len(),
                1,
                "only 1 retirement, for the accepted recipient: {reports:?}"
            );
            assert_ne!(
                reports[0].1,
                tid("worker"),
                "a failure row is not the subject of a retirement"
            );
        }

        // ── pass A branch (separate assertion) — when the queue is already full at the decision stage ─────────────────────
        let (svc, port, gate) = svc_gated();
        let (from, me) = live_sender("boss");
        let (f_id, full) = live("full");
        port.set_roster(vec![me, full]);
        gate.set_busy(f_id, 0);
        for i in 0..100 {
            send(&svc, &format!("p{i}"), from, "boss", &["full"]).expect("park");
        }
        fill_open_request_cap_evictable(&svc, from);
        let occupied_before = svc.occupied_slots_for_test();
        let out = svc
            .handle_send(
                "m-passa",
                from,
                "boss",
                &["full".to_string()],
                "do it",
                Entrance::Mcp,
                &req_meta("10m", 600),
            )
            .expect("row response");
        assert_eq!(out[0].code, Some(FailCode::MailboxFull));
        assert_eq!(
            svc.marked_retirements_for_test(),
            0,
            "a pass A failure does not establish a retirement either"
        );
        assert_eq!(svc.occupied_slots_for_test(), occupied_before);
    }

    #[test]
    fn a_send_never_cannibalizes_a_contract_it_opened_itself() {
        // ★The fixture shape is load-bearing★: filling the limit only with contracts **older** than this send means victim selection (`min_by_key(created_at)`) cannot pick this send's own contract and the test is vacuous.
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
        .expect("fills the 1 spare spot with a retirable contract");

        let out = svc
            .handle_send(
                "m-two",
                from,
                "boss",
                &["w1".to_string(), "w2".to_string()],
                "do it",
                Entrance::Mcp,
                &SendMeta {
                    request: true,
                    ..SendMeta::default()
                },
            )
            .expect("row response");
        assert_eq!(out.len(), 2);
        assert_eq!(
            out[0].status,
            SendStatus::Delivered,
            "the first recipient gets the spare victim and is accepted: {out:?}"
        );
        assert!(
            svc.contract_tracked_for_test("m-two", tid("w1")),
            "★key★: the first recipient's contract must be alive — the second recipient of the same send must not eat it"
        );
        match out[1].status {
            SendStatus::Failed => assert_eq!(
                out[1].code,
                Some(FailCode::RequestCapacity),
                "the second recipient's failure reason is only the cap (since it cannot eat its own send): {out:?}"
            ),
            _ => assert!(
                svc.contract_tracked_for_test("m-two", tid("w2")),
                "an accepted recipient must have its own contract: {out:?}"
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
            "each accepted recipient must have one contract (self-cannibalization would leave it short)"
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
                other => panic!("premise: a mark must be attached by cap pressure — {other:?}"),
            };
            assert_eq!(st.ledger.marked_retirement_count_for_test(), 1, "1 mark");
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
                "vanishing without settlement must blow up immediately in a debug build (H1 — the test catches the forgotten branch)"
            );
        }

        assert!(
            !svc.contract_tracked_for_test("m-drop", tid("ghost-worker")),
            "Drop must remove the provisional contract"
        );
        assert_eq!(
            svc.marked_retirements_for_test(),
            0,
            "Drop must release the victim mark too — if it remains, the cap denominator shrinks forever"
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
        .expect("fills the 1 spare spot with a retirable contract");

        let out = svc
            .handle_send(
                "m-park",
                from,
                "boss",
                &["w1".to_string(), "w2".to_string()],
                "do it",
                Entrance::Mcp,
                &SendMeta {
                    request: true,
                    ..SendMeta::default()
                },
            )
            .expect("row response");
        assert_eq!(out.len(), 2);
        assert_eq!(
            out[0].status,
            SendStatus::Pending,
            "premise: the first recipient is in a turn, so it is parked (running this branch is the purpose of this test): {out:?}"
        );
        assert_eq!(svc.parked_len(tid("w1")), 1, "actually put in the queue");
        assert!(
            svc.contract_tracked_for_test("m-park", tid("w1")),
            "★key★: the parked recipient's contract must be alive — the second recipient of the same send must not eat it"
        );
        assert!(
            !svc.contract_tracked_for_test("spare", tid("spare-victim")),
            "the parking branch must also settle with a **commit** and physically remove the marked victim (with rollback/no settlement the mark remains and the cap denominator shrinks forever)"
        );
        assert_eq!(
            svc.marked_retirements_for_test(),
            0,
            "once settlement is done, no mark can remain"
        );
        match out[1].status {
            SendStatus::Failed => assert_eq!(
                out[1].code,
                Some(FailCode::RequestCapacity),
                "the second recipient's failure reason is only the cap (since it cannot eat its own send): {out:?}"
            ),
            _ => assert!(
                svc.contract_tracked_for_test("m-park", tid("w2")),
                "an accepted recipient must have its own contract: {out:?}"
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
            "one contract per accepted recipient — if the parked contract stays provisional, it comes up short here"
        );
    }

    /// Makes one provisional reservation **with the mark attached** under limit pressure (the caller settles it or not). F1's two hard paths (`try_lock` failure, unwinding) assume the reservation already exists;
    /// going through the send path would finish settlement inside it and make reproduction impossible.
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
            other => panic!("premise: a mark must be attached by cap pressure — {other:?}"),
        };
        assert_eq!(st.ledger.marked_retirement_count_for_test(), 1, "1 mark");
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
                "do it",
                Entrance::Mcp,
                &req_meta("10m", 600),
            )
            .expect("row response");
        assert_eq!(
            out[0].status,
            SendStatus::Delivered,
            "premise: took the inject branch: {out:?}"
        );

        assert!(
            svc.contract_tracked_for_test("m-slow", tid("worker")),
            "the sweep must not reclaim a reservation whose owner is alive (R1)"
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
            "the sender must wait for the reply: {sender_side:?}"
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
            "the recipient must see the duty to answer: {recipient_side:?}"
        );

        let reports = retirement_reports::drain();
        assert_eq!(
            reports.len(),
            1,
            "1 retirement = 1 report line: {reports:?}"
        );
        assert_eq!(svc.marked_retirements_for_test(), 0, "no mark remains");
        assert_eq!(
            svc.occupied_slots_for_test(),
            occupied_before,
            "the cap arithmetic is unchanged too (1 retired − 1 new)"
        );

        svc.handle_send(
            "m-reply",
            reply_from(w_id),
            "worker",
            &["boss".to_string()],
            "all done",
            Entrance::Mcp,
            &reply_meta("m-slow"),
        )
        .expect("reply send");
        // The observation surface of closing = **the outstanding lookup** (the tracking item itself remains until cleaned up together with the history).
        assert!(
            svc.open_items_for(from.peer_id, Instant::now())
                .iter()
                .all(|i| i.id != "m-slow"),
            "the reply must close the contract — it disappears from the sender's outstanding items (with the old hole it was NoMatch and stayed forever)"
        );
        assert!(
            svc.open_items_for(w_id, Instant::now())
                .iter()
                .all(|i| i.id != "m-slow"),
            "the replier's duty must disappear too"
        );
        assert_eq!(
            svc.ledger_statuses("m-slow"),
            vec![DeliveryStatus::Replied],
            "the history must close as `Replied` too — the refutation of the permanent `Delivered` the probe saw (a lie in the audit record)"
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
                locked_tx.send(()).expect("signal");
                release_rx.recv().expect("release signal");
            })
        };
        locked_rx.recv().expect("lock held confirmed");
        let hit = catch_quietly(|| drop(guard));
        if cfg!(debug_assertions) {
            assert!(
                hit.is_err(),
                "a missed settlement must be red in debug (H1)"
            );
        }
        release_tx.send(()).expect("release");
        holder.join().expect("holder");

        assert!(
            svc.contract_tracked_for_test("m-locked", tid("ghost-worker")),
            "try_lock failed, so Drop could do nothing — if this premise breaks, the guarantee assertions below become meaningless"
        );
        assert_eq!(svc.marked_retirements_for_test(), 1, "the mark remains too");

        svc.sweep(Instant::now() + Duration::from_secs(30));
        assert!(
            !svc.contract_tracked_for_test("m-locked", tid("ghost-worker")),
            "the sweep must reclaim the provisional contract (F1 guarantee layer)"
        );
        assert_eq!(
            svc.marked_retirements_for_test(),
            0,
            "the victim mark must be released too — if it remains, the cap denominator shrinks forever"
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
            .expect_err("the panic must propagate")
            .downcast::<&str>()
            .expect("the payload is the original panic as is");
        assert_eq!(
            payload, "boom",
            "if Drop replaced the original panic with its own panic, the cause would vanish"
        );

        assert!(
            !svc.contract_tracked_for_test("m-unwind", tid("ghost-worker")),
            "even during unwinding, the rollback is performed on a best-effort basis"
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
            "do it",
            Entrance::Mcp,
            &req_meta("10m", 600),
        )
        .expect("delivery");
        assert_eq!(svc.open_request_count(), 1);

        gate.set_busy(boss_id, 0);
        let rows_out = svc
            .handle_send(
                "m-rep",
                reply_from(w_id),
                "worker",
                &["boss".to_string()],
                "done",
                Entrance::Mcp,
                &reply_meta("m-req"),
            )
            .expect("row response");
        assert_eq!(
            rows_out[0].status,
            SendStatus::Pending,
            "parked for a requester in a turn: {rows_out:?}"
        );
        assert_eq!(
            svc.open_request_count(),
            0,
            "★a parked reply closes the contract too (M5)★ — it was accepted, so it will be delivered soon"
        );
        svc.sweep(Instant::now() + Duration::from_secs(601));
        assert!(
            !svc.ledger_snapshot()
                .iter()
                .any(|(_, from, _, _)| from == NOTICE_SENDER_LABEL),
            "a closed contract has no false deadline notice"
        );
    }

    #[test]
    fn the_to_attribute_rules_hold_for_every_token_shape() {
        // ★Why white-box★: "an `@` token's expansion is **all failures**" cannot be made black-box while `@all` is the only source (its key is the live roster, incompatible with the 2-or-more-accepted condition for an
        //   envelope); a future source (folder group) will use the rule, so the decision function is pinned directly.
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
            "writing the same value twice makes the receiving LLM misread the head count (M4)"
        );

        // ③ M6: the expansion of an `@` token is **all failures**.
        let addr = Addressing {
            recipients: vec![rcpt("carol"), rcpt("dup")],
            tokens: vec![tok_name(0), tok_group("@all", &[1])],
        };
        assert_eq!(
            build_to_attr(&addr, &[true, false]),
            "carol",
            "an @token whose expansion entirely failed contributed nothing to this send (M6)"
        );

        // ④ It stays if even one is accepted (A5 — including absorption).
        let addr = Addressing {
            recipients: vec![rcpt("bob"), rcpt("carol")],
            tokens: vec![tok_name(0), tok_name(1), tok_group("@all", &[0, 1])],
        };
        assert_eq!(
            build_to_attr(&addr, &[true, true]),
            "bob,carol,@all",
            "even if the expansion is entirely absorbed by an earlier explicit target, it is not a failure (A5)"
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
        assert_eq!(out.len(), 2, "a duplicated notation does not create rows");
        assert!(
            port.injected_bodies()[0].contains(r#"to="bob,carol""#),
            "the envelope to without duplicates too: {:?}",
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
            "each token its own row · both delivered: {out:?}"
        );
        assert_eq!(
            port.injected_targets(),
            vec![d1, d2],
            "one letter to each twin"
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
            "repeated tokens fold and there is one row per twin: {out:?}"
        );
        // Ledger transitions are found by `(msg_id, receiving id)` — if two rows used one key, one would remain in pending.
        assert_eq!(
            svc.ledger_statuses("m1"),
            vec![DeliveryStatus::Delivered, DeliveryStatus::Delivered]
        );
        assert_eq!(
            port.injected_targets(),
            vec![d1, d2],
            "delivery is one letter per twin"
        );
    }

    /// ★A repeat of an unresolved token also folds into one row★ — with no id the row key is the notation (`same_row`), so a not-found or ambiguous token with one notation leaves one failure row and one ledger row.
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
            "a repeated nonexistent name is one failure row: {out:?}"
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
            "a repeated ambiguous name is one failure row: {out2:?}"
        );
        assert_eq!(svc2.ledger_statuses("m2"), vec![DeliveryStatus::Failed]);
        assert!(port2.injected_targets().is_empty(), "injects to no one");
    }

    /// ★The id first, then the name (same order as the control entrance `resolve_in`)★ — a UUID-shaped name cannot hijack someone else's id target; an id shape not in the registry resolves as that name.
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
            "bob's id to bob (not to an impostor with the same name) · a UUID not in the registry to the owner of that name"
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
                "do it",
                Entrance::Mcp,
                &req_meta("10m", 600),
            )
            .expect("row response (not a whole bounce — a row failure)");
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
            "a recipient whose contract could not be opened **is not delivered to either** (no untracked request)"
        );
        assert_eq!(
            svc.ledger_statuses("m-over"),
            vec![DeliveryStatus::Failed],
            "only a terminal failed remains in the ledger, not delivered"
        );
        assert_eq!(svc.parked_len(tid("worker")), 0, "no parking either");
    }

    #[test]
    fn the_to_attribute_keeps_a_token_whose_expansion_was_fully_absorbed_by_explicit_names() {
        let (svc, port) = svc();
        let (from, me) = live_sender("alice");
        let (_b, bob) = live("bob");
        let (_c, carol) = live("carol");
        port.set_roster(vec![me, bob, carol]);

        let out = send(&svc, "m1", from, "alice", &["bob", "carol", "@all"]).expect("ok");
        assert_eq!(out.len(), 2, "rows absorbed to 2");
        assert!(
            port.injected_bodies()[0].contains(r#"to="bob,carol,@all""#),
            "an absorbed @all token must also remain in input notation order: {:?}",
            port.injected_bodies()
        );
    }

    #[test]
    fn an_exact_id_token_and_an_ambiguous_name_in_the_same_to_list_get_separate_rows() {
        // ★A name row is not absorbed into an id row (ADR-0309, TRD S29 §4 X2)★ — an unresolved name points at no twin, so it is not the same row as an id-resolved one; reversed order gives the same outcome.
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
                "prediction #{i}: still room"
            );
            send(&svc, &format!("p{i}"), from, "alice", &["recv"]).expect("park");
        }
        assert!(
            !svc.can_admit_for_test(tid("recv")),
            "cap reached prediction"
        );
        assert_eq!(
            send(&svc, "over", from, "alice", &["recv"]).expect("row")[0].code,
            Some(FailCode::MailboxFull),
            "the actual park is bounced too — matches the prediction"
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
                .expect("row")[0]
                .code
                .is_none();
            seen_h.lock().unwrap().push((predicted, actual));
        }));
        svc.flush_for(r_id);
        let obs = seen.lock().unwrap().clone();
        assert_eq!(obs.len(), 1);
        assert_eq!(
            obs[0].0, obs[0].1,
            "prediction == actual in the in-flight window too (if the denominators diverge, it blows up here)"
        );
    }

    // ══════════════════════════════════════════════════════════════════════════════════════════
    // S18.6 entrance 3 branches — asleep parking · immediate injection with no turn signal · delete cleanup (spec §7, ADR-0116/0117/0118)
    // ══════════════════════════════════════════════════════════════════════════════════════════

    #[test]
    fn a_dormant_recipient_is_parked_without_a_wake_and_flushed_on_restore() {
        let (svc, port) = svc();
        let (from, me) = live_sender("alice");
        port.set_roster(vec![me]);
        port.set_dormant(&["sleepy"]);

        let out = send(&svc, "m-dorm", from, "alice", &["sleepy"]).expect("not a bounce");
        assert_eq!(
            (out[0].status, out[0].code),
            (SendStatus::Pending, None),
            "asleep is acceptance (parking) — not a failure row: {out:?}"
        );
        assert!(
            out[0].hint.as_deref().unwrap_or("").contains("saved agent"),
            "the hint must say 'a saved agent, delivered on restore': {:?}",
            out[0].hint
        );
        assert_eq!(
            svc.ledger_statuses("m-dorm"),
            vec![DeliveryStatus::Pending],
            "the parking record remains in the ledger (no silent loss)"
        );
        assert_eq!(
            svc.parked_len(tid("sleepy")),
            1,
            "parked in that id's queue"
        );
        assert!(
            port.injected_bodies().is_empty(),
            "does not try to inject into an asleep counterpart (no wake triggered)"
        );

        let (late_id, late) = live("sleepy");
        port.set_roster(vec![late]);
        port.set_dormant(&[]);
        svc.flush_for(late_id);
        assert_eq!(
            port.injected_bodies(),
            vec![r#"<message from="alice">body</message>"#.to_string()],
            "delivered by flush on restore"
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

        let here = send(&svc, "m-here", from, "alice", &["@here"]).expect("not a bounce");
        assert_eq!(
            here.iter().map(|r| r.to.clone()).collect::<Vec<_>>(),
            vec!["bob".to_string()],
            "an asleep name must not be in the @here expansion: {here:?}"
        );
        assert_eq!(
            svc.parked_len(tid("sleepy")),
            0,
            "@here does not create asleep queues"
        );

        let mixed = send(&svc, "m-mix", from, "alice", &["sleepy", "@here"]).expect("not a bounce");
        assert_eq!(
            rows(&mixed),
            vec![
                ("sleepy".to_string(), SendStatus::Pending, None),
                ("bob".to_string(), SendStatus::Delivered, None),
            ],
            "the asleep row is pending, the live row delivered: {mixed:?}"
        );
    }

    #[test]
    fn at_all_expands_to_the_whole_roster_including_dormant_names() {
        let (svc, port) = svc();
        let (from, me) = live_sender("alice");
        let (_b, bob) = live("bob");
        port.set_roster(vec![me, bob]);
        port.set_dormant(&["sleepy"]);

        let all = send(&svc, "m-all", from, "alice", &["@all"]).expect("not a bounce");
        assert_eq!(
            rows(&all),
            vec![
                ("bob".to_string(), SendStatus::Delivered, None),
                ("sleepy".to_string(), SendStatus::Pending, None),
            ],
            "live members delivered, asleep members parked (accepted): {all:?}"
        );
        assert_eq!(
            svc.parked_len(tid("sleepy")),
            1,
            "the asleep share accumulates in that id's queue"
        );

        let (late_id, late) = live("sleepy");
        port.set_roster(vec![late]);
        port.set_dormant(&[]);
        svc.flush_for(late_id);
        assert_eq!(
            svc.ledger_statuses("m-all"),
            vec![DeliveryStatus::Delivered, DeliveryStatus::Delivered],
            "the asleep share is delivered too when it appears"
        );
    }

    #[test]
    fn neither_broadcast_vocabulary_ever_reaches_the_sender() {
        // ★The sender is removed by id only (ADR-0309, TRD S29 §4 X3)★ — a different agent merely sharing the sender's name (live twin, asleep same-name profile) receives the broadcast.
        let (svc, port) = svc();
        let (from, me) = live_sender("alice");
        let (twin_id, twin) = live_distinct("alice");
        let (bob_id, bob) = live("bob");
        port.set_roster(vec![me, twin, bob]);
        let sleeper = PeerId::new_v4();
        port.set_dormant_agents(&[(sleeper, "alice"), (tid("sleepy"), "sleepy")]);

        let all = send(&svc, "m-all", from, "alice", &["@all"]).expect("not a bounce");
        assert_eq!(
            rows(&all),
            vec![
                ("alice".to_string(), SendStatus::Delivered, None),
                ("alice".to_string(), SendStatus::Pending, None),
                ("bob".to_string(), SendStatus::Delivered, None),
                ("sleepy".to_string(), SendStatus::Pending, None),
            ],
            "the live twin delivered · the asleep same-named one parked under that id · no sender row: {all:?}"
        );
        assert_eq!(
            svc.parked_len(sleeper),
            1,
            "the asleep same-named share in that id's queue"
        );

        let here = send(&svc, "m-here", from, "alice", &["@here"]).expect("not a bounce");
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
            "nothing is written to the sender's stdin"
        );
        assert_eq!(
            svc.parked_len(from.peer_id),
            0,
            "no parking addressed to the sender id either"
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
                "do it",
                Entrance::Mcp,
                &req_meta("1m", 60),
            )
            .expect("not a bounce");
        assert_eq!(out[0].status, SendStatus::Pending);
        assert_eq!(
            svc.open_request_count(),
            1,
            "asleep parking opens a contract too"
        );
        assert_eq!(
            svc.contract_outcome_for_test("m-req", tid("sleepy")),
            Some("awaiting_reply")
        );

        svc.sweep(Instant::now() + Duration::from_secs(61));
        assert!(
            svc.ledger_snapshot()
                .iter()
                .any(|(_, f, to, _)| f == NOTICE_SENDER_LABEL && to == "alice"),
            "an asleep contract must get a deadline notice too: {:?}",
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

        let out = send(&svc, "m1", from, "alice", &["twin"]).expect("row response");
        assert_eq!(
            (out[0].status, out[0].code),
            (SendStatus::Failed, Some(FailCode::RecipientAmbiguous)),
            "a same name in the asleep layer is a failure row, not parking: {out:?}"
        );
        assert_eq!(
            svc.parked_total_for_test(),
            0,
            "there must be no path that silently accepts"
        );
        assert_eq!(
            svc.ledger_statuses("m1"),
            vec![DeliveryStatus::Failed],
            "a failure row remains as a ledger terminal too"
        );

        // `@all` expands by id, not by name — it parks for each of the two profiles (ADR-0309 · TRD S29 §4 X1).
        let all = send(&svc, "m2", from, "alice", &["@all"]).expect("row response");
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

        let out = send(&svc, "m1", from, "alice", &["dup"]).expect("row response");
        assert_eq!(
            out[0].status,
            SendStatus::Delivered,
            "the live one wins: {out:?}"
        );
        assert_eq!(port.injected_bodies().len(), 1);
        assert_eq!(svc.parked_total_for_test(), 0);

        // That is only a rule of name targeting — `@all` expands by id so an asleep same-name also gets its share (ADR-0309 · TRD S29 §4 X1).
        let all = send(&svc, "m2", from, "alice", &["@all"]).expect("row response");
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

    /// ★An asleep agent can also be called by its exact id (ADR-0309 decision 3, TRD S29 §4 B6)★ — the ledger keeps the registry name, not the notation (id).
    #[test]
    fn a_dormant_agent_addressed_by_its_exact_id_is_parked_and_delivered_on_wake() {
        let (svc, port) = svc();
        let (from, me) = live_sender("alice");
        port.set_roster(vec![me.clone()]);
        port.set_dormant(&["sleepy"]);
        let token = tid("sleepy").to_string();

        let out = send(&svc, "m1", from, "alice", &[&token]).expect("row response");
        assert_eq!(
            rows(&out),
            vec![(token.clone(), SendStatus::Pending, None)],
            "{out:?}"
        );
        assert_eq!(svc.parked_len(tid("sleepy")), 1);
        assert_eq!(
            svc.ledger_endpoint_keys("m1"),
            vec!["sleepy".to_string()],
            "the ledger's recipient = the registry name"
        );

        let (late_id, late) = live("sleepy");
        port.set_roster(vec![me, late]);
        port.set_dormant(&[]);
        svc.flush_for(late_id);
        assert_eq!(svc.ledger_statuses("m1"), vec![DeliveryStatus::Delivered]);
        assert_eq!(port.injected_targets(), vec![late_id]);
    }

    /// ★The id spelling is read like the control entrance (`resolve_in`) (TRD S29 §4 B7)★ — uppercase, braces, no hyphens, `urn:uuid:` are ids too; one agent written as name and as a differently-spelled id is one row.
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
            .expect("row response");
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
                "do it",
                Entrance::Mcp,
                &req_meta("10m", 600),
            )
            .expect("row response");
        assert_eq!(
            (out[0].status, out[0].code),
            (SendStatus::Delivered, None),
            "a delivery target even without a turn signal (immediate inject without the gate): {out:?}"
        );
        assert_eq!(port.injected_bodies().len(), 1, "actually injected");
        assert_eq!(
            svc.parked_len(tid("tui")),
            0,
            "a kind that has no busy parking"
        );
        assert_eq!(
            svc.ledger_statuses("m1"),
            vec![DeliveryStatus::Delivered],
            "delivered = the actual inject time"
        );
        assert_eq!(
            svc.contract_outcome_for_test("m1", tid("tui")),
            Some("awaiting_reply"),
            "delivered, so the contract opens normally too"
        );

        let second = send(&svc, "m2", from, "alice", &["tui"]).expect("row response");
        assert_eq!(second[0].status, SendStatus::Delivered);
        assert_eq!(port.injected_bodies().len(), 2, "both are injected");

        let all = send(&svc, "m3", from, "alice", &["@all"]).expect("row response");
        assert_eq!(
            all.iter().map(|r| r.to.clone()).collect::<Vec<_>>(),
            vec!["tui".to_string()],
            "@all also expands live sessions without a turn signal: {all:?}"
        );

        port.set_roster(vec![me]);
        let gone = send(&svc, "m4", from, "alice", &["tui"]).expect("row response");
        assert_eq!(
            gone[0].code,
            Some(FailCode::RecipientNotFound),
            "if it is in neither the roster nor the profiles, then it is the not-found code: {gone:?}"
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

        let live_row = send(&svc, "m1", from, "alice", &["dup"]).expect("row")[0]
            .hint
            .clone()
            .unwrap_or_default();
        let dormant_row = send(&svc, "m2", from, "alice", &["twin"]).expect("row")[0]
            .hint
            .clone()
            .unwrap_or_default();
        for (layer, hint) in [("live", &live_row), ("dormant", &dormant_row)] {
            assert!(
                hint.contains("not supported")
                    && hint.contains("do NOT resend")
                    && hint.contains("tell the user"),
                "the {layer} hint must speak of user escalation: {hint}"
            );
            assert!(
                !hint.contains("exact agent id") && !hint.contains("exact id"),
                "the {layer} hint still has exact-id resend guidance: {hint}"
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
            "do it",
            Entrance::Mcp,
            &req_meta("10m", 600),
        )
        .expect("delivery");

        port.set_roster(vec![worker.clone()]);
        port.set_dormant(&["boss"]);
        let worker_from = SenderIdentity {
            peer_id: w_id,
            epoch: 0,
            node: None,
        };
        let rep = svc
            .handle_send(
                "m-rep",
                worker_from,
                "worker",
                &["boss".to_string()],
                "done",
                Entrance::Mcp,
                &reply_meta("m-req"),
            )
            .expect("row response");
        assert_eq!(
            rep[0].status,
            SendStatus::Pending,
            "the reply to an asleep requester is parked: {rep:?}"
        );
        assert_eq!(
            svc.contract_outcome_for_test("m-req", tid("worker")),
            Some("replied"),
            "parking is acceptance too — once stuck in, the contract is complete (ADR-0116 decision 2)"
        );

        port.set_dormant(&[]);
        let cleanup = svc.handle_profile_deleted(boss_from.peer_id, "boss");
        assert_eq!(cleanup.failed_parked, 1, "the parked reply is terminated");
        assert_eq!(
            svc.ledger_statuses("m-rep"),
            vec![DeliveryStatus::Failed],
            "the delivery axis records the failure truthfully"
        );
        assert_eq!(
            svc.contract_outcome_for_test("m-req", tid("worker")),
            Some("replied"),
            "★not reverted★ — the contract stays replied (ADR-0116 decision 2 · no reverting terminals)"
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
            node: None,
        };

        svc.handle_send(
            "m-out",
            gone_from,
            "gone",
            &["worker".to_string()],
            "do it",
            Entrance::Mcp,
            &req_meta("10m", 600),
        )
        .expect("delivery");
        svc.handle_send(
            "m-in",
            worker_from,
            "worker",
            &["gone".to_string()],
            "do it",
            Entrance::Mcp,
            &req_meta("10m", 600),
        )
        .expect("not a bounce");
        assert_eq!(svc.parked_len(tid("gone")), 1);

        port.set_dormant(&[]);
        let out = svc.handle_profile_deleted(gone_from.peer_id, "gone");
        assert!(!out.skipped_live);
        assert_eq!(out.failed_parked, 1, "parked items terminated");
        assert_eq!(
            out.failed_contracts, 1,
            "only the requester-side contract terminated"
        );
        assert_eq!(svc.parked_len(tid("gone")), 0, "disappears from the queue");

        assert_eq!(svc.ledger_statuses("m-in"), vec![DeliveryStatus::Failed]);
        let view = svc
            .message_state("m-in", Instant::now())
            .expect("queryable");
        assert_eq!(view.rows[0].code, Some("RECIPIENT_DELETED"));
        assert!(
            view.rows[0]
                .hint
                .as_deref()
                .unwrap_or("")
                .contains("do not resend automatically"),
            "the query hint must state the next action: {:?}",
            view.rows[0].hint
        );

        assert_eq!(
            svc.contract_outcome_for_test("m-out", tid("worker")),
            Some("reply_failed"),
            "contracts whose requester is that name are terminated as failed"
        );
        assert_eq!(
            svc.contract_outcome_for_test("m-in", tid("gone")),
            Some("awaiting_reply"),
            "contracts whose replier side was deleted are kept (the deadline notice path survives)"
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
            "do it",
            Entrance::Mcp,
            &req_meta("10m", 600),
        )
        .expect("park");
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
        assert!(
            out.skipped_live,
            "skipped if that profile's session is alive"
        );
        assert_eq!(out.failed_parked, 0);
        assert_eq!(svc.parked_len(tid("recv")), 1, "parked items preserved");
        assert_eq!(
            svc.contract_outcome_for_test("m-req", tid("recv")),
            Some("awaiting_reply"),
            "contract stays open"
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
                "do it",
                Entrance::Mcp,
                &req_meta("10m", 600),
            )
            .expect("row response");
        assert_eq!(
            out[0].status,
            SendStatus::Pending,
            "an inject failure is absorbed into parking (no silent loss): {out:?}"
        );
        assert_eq!(svc.parked_len(tid("tui")), 1);

        port.set_live_ids(&[tui_id]);
        let cleaned = svc.handle_profile_deleted(tui_id, "tui");
        assert!(
            cleaned.skipped_live,
            "a live session even without a turn signal — if the gate misses it, mail that would be delivered dies"
        );
        assert_eq!(svc.parked_len(tid("tui")), 1, "parked items preserved");
        assert_eq!(
            svc.contract_outcome_for_test("m-req", tid("tui")),
            Some("awaiting_reply"),
            "contract stays open"
        );
    }

    /// ★With a live same-name, the deleted profile's share is cleaned and the same-name's share left (ADR-0309 decision 6, TRD S29 B4)★ — cleanup sweeps by id, so equal names enter neither decision nor target selection;
    ///   reviving the name-axis gate would skip the cleanup wholesale and leave the deleted side's parked items and contracts until TTL.
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
                node: None,
            },
            "boss",
            &["alice".to_string()],
            "do it",
            Entrance::Mcp,
            &req_meta("10m", 600),
        )
        .expect("delivery");
        port.set_roster(vec![alice.clone()]);
        port.set_dormant_agents(&[(p, "boss")]);
        svc.handle_send(
            "m-to-p",
            alice_from,
            "alice",
            &["boss".to_string()],
            "to asleep P",
            Entrance::Mcp,
            &SendMeta::default(),
        )
        .expect("park");
        assert_eq!(
            svc.parked_len(p),
            1,
            "premise: parked addressed to asleep P"
        );

        // A different agent Q with the same name is up (the live layer beats the asleep layer) — Q also places a request, and parking
        //   piles up for the busy Q as well.
        let (q, q_live) = live_distinct("boss");
        port.set_roster(vec![alice.clone(), q_live]);
        svc.handle_send(
            "m-q-req",
            SenderIdentity {
                peer_id: q,
                epoch: 0,
                node: None,
            },
            "boss",
            &["alice".to_string()],
            "do it",
            Entrance::Mcp,
            &req_meta("10m", 600),
        )
        .expect("delivery");
        gate.set_busy(q, 0);
        svc.handle_send(
            "m-to-q",
            alice_from,
            "alice",
            &["boss".to_string()],
            "to live Q",
            Entrance::Mcp,
            &SendMeta::default(),
        )
        .expect("park");
        assert_eq!(svc.parked_len(q), 1, "premise: parked addressed to busy Q");

        // Delete P — the name passed in is the same as the live same-name's name.
        port.set_dormant_agents(&[]);
        port.set_live_ids(&[q]);
        let out = svc.handle_profile_deleted(p, "boss");

        assert!(
            !out.skipped_live,
            "a live same-named one does not block the cleanup: {out:?}"
        );
        assert_eq!(
            out.failed_parked, 1,
            "only items parked for P terminated: {out:?}"
        );
        assert_eq!(
            out.failed_contracts, 1,
            "only contracts whose requester is P terminated: {out:?}"
        );
        assert_eq!(svc.parked_len(p), 0);
        assert_eq!(svc.ledger_statuses("m-to-p"), vec![DeliveryStatus::Failed]);
        assert_eq!(
            svc.contract_outcome_for_test("m-p-req", tid("alice")),
            Some("reply_failed")
        );

        assert_eq!(
            svc.parked_len(q),
            1,
            "the live same-named one's parked items stay"
        );
        assert_eq!(
            svc.ledger_statuses("m-to-q"),
            vec![DeliveryStatus::Pending],
            "★no false reasons★ — mail of Q, which was not deleted, must not die as RECIPIENT_DELETED"
        );
        assert_eq!(
            svc.contract_outcome_for_test("m-q-req", tid("alice")),
            Some("awaiting_reply"),
            "open contracts whose requester is the live same-named one are kept (ADR-0118 decision 2)"
        );

        gate.clear();
        svc.flush_for(q);
        assert_eq!(
            svc.ledger_statuses("m-to-q"),
            vec![DeliveryStatus::Delivered]
        );
    }

    /// ★A profile renamed while asleep and then deleted: the share under the old name is swept by id, and a live agent that newly took the old name is untouched (ADR-0309 decision 6, TRD S29 B4)★. The name passed in is the
    ///   new name just before removal, so a name pick would miss X's old-name contract and kill Y's share.
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
                node: None,
            },
            "a",
            &["carol".to_string()],
            "do it",
            Entrance::Mcp,
            &req_meta("10m", 600),
        )
        .expect("delivery");
        port.set_roster(vec![carol.clone()]);
        port.set_dormant_agents(&[(x, "a")]);
        svc.handle_send(
            "m-to-x",
            carol_from,
            "carol",
            &["a".to_string()],
            "to asleep X",
            Entrance::Mcp,
            &SendMeta::default(),
        )
        .expect("park");
        assert_eq!(
            svc.parked_len(x),
            1,
            "premise: parked addressed to asleep X"
        );

        // X renames to b while asleep, and a different agent Y is up as a — Y also places a request and parking piles up for the busy Y.
        let (y, y_live) = live_distinct("a");
        port.set_dormant_agents(&[(x, "b")]);
        port.set_roster(vec![carol.clone(), y_live]);
        svc.handle_send(
            "m-y-req",
            SenderIdentity {
                peer_id: y,
                epoch: 0,
                node: None,
            },
            "a",
            &["carol".to_string()],
            "do it",
            Entrance::Mcp,
            &req_meta("10m", 600),
        )
        .expect("delivery");
        gate.set_busy(y, 0);
        svc.handle_send(
            "m-to-y",
            carol_from,
            "carol",
            &["a".to_string()],
            "to live Y",
            Entrance::Mcp,
            &SendMeta::default(),
        )
        .expect("park");
        assert_eq!(svc.parked_len(y), 1, "premise: parked addressed to busy Y");

        // Delete X — the name passed in is the pre-removal name b.
        port.set_dormant_agents(&[]);
        port.set_live_ids(&[y]);
        let out = svc.handle_profile_deleted(x, "b");

        assert!(!out.skipped_live, "{out:?}");
        assert_eq!(
            out.failed_parked, 1,
            "X's parked items accumulated under the old name: {out:?}"
        );
        assert_eq!(
            out.failed_contracts, 1,
            "X's contract opened under the old name: {out:?}"
        );
        assert_eq!(svc.parked_len(x), 0);
        let view = svc
            .message_state("m-to-x", Instant::now())
            .expect("queryable");
        assert_eq!(view.rows[0].code, Some("RECIPIENT_DELETED"));
        assert_eq!(
            view.rows[0].to, "a",
            "the row shows the name at send time (decision 4)"
        );
        assert_eq!(
            svc.contract_outcome_for_test("m-x-req", tid("carol")),
            Some("reply_failed")
        );

        assert_eq!(
            svc.parked_len(y),
            1,
            "the parked items of Y, the new owner of the old name, stay"
        );
        assert_eq!(svc.ledger_statuses("m-to-y"), vec![DeliveryStatus::Pending]);
        assert_eq!(
            svc.contract_outcome_for_test("m-y-req", tid("carol")),
            Some("awaiting_reply"),
            "open contracts whose requester is Y are kept"
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
            "do it",
            Entrance::Mcp,
            &req_meta("10m", 600),
        )
        .expect("delivery");

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
                    node: None,
                },
                "worker",
                &["boss".to_string()],
                "done",
                Entrance::Mcp,
                &reply_meta("m-req"),
            )
            .expect("row response");
        assert_eq!(
            rep[0].status,
            SendStatus::Pending,
            "the reply was accepted by parking"
        );
        assert_eq!(
            *in_lock.lock().unwrap(),
            vec![Some("replied")],
            "★it must already be closed within the very lock that recorded the acceptance★ — if this is `awaiting_reply`, the close slipped into another lock, and if a deletion cleanup gets into that window it becomes 'accepted but reply failed' (review fix D2)"
        );
        assert_eq!(
            svc.contract_outcome_for_test("m-req", tid("worker")),
            Some("replied"),
            "accepted = complete — the contract closed in that critical section"
        );

        port.set_dormant(&[]);
        let cleaned = svc.handle_profile_deleted(boss_from.peer_id, "boss");
        assert_eq!(
            cleaned.failed_parked, 1,
            "the parked reply is truthfully terminated as failed"
        );
        assert_eq!(
            svc.contract_outcome_for_test("m-req", tid("worker")),
            Some("replied"),
            "the contract axis is not reverted (ADR-0116 decision 2)"
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
            "do it",
            Entrance::Mcp,
            &req_meta("10m", 600),
        )
        .expect("delivery");

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
                    node: None,
                },
                "worker",
                &["boss".to_string()],
                "done",
                Entrance::Mcp,
                &reply_meta("m-req"),
            )
            .expect("row response");
        assert_eq!(
            rep[0].status,
            SendStatus::Delivered,
            "that call's drain sent it, so delivered (the path is queue-then-drain)"
        );
        assert_eq!(
            *in_lock.lock().unwrap(),
            vec![Some("replied")],
            "★it must already be closed within the queueing lock★ — if observed as `awaiting_reply`, the close slipped outside that lock (review fix D2 · ADR-0125 made the settlement point a single one)"
        );
    }

    /// ★A contract addressed to an asleep recipient holds that profile id from the moment it opens (ADR-0309 decision 1, TRD S29 B5)★ — so after a rename and wake under the new name "my duty to answer" remains and that reply closes it.
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
            "do it",
            Entrance::Mcp,
            &req_meta("10m", 600),
        )
        .expect("park");
        assert_eq!(
            svc.contract_outcome_for_test("m-req", sleepy_id),
            Some("awaiting_reply"),
            "the key of a contract addressed to an asleep recipient = that profile id"
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
            "the duty stays on that id even after a rename: {items:?}"
        );
        let rep = svc
            .handle_send(
                "m-rep",
                reply_from(sleepy_id),
                "renamed",
                &["alice".to_string()],
                "done",
                Entrance::Mcp,
                &reply_meta("m-req"),
            )
            .expect("row response");
        assert_eq!(rep[0].status, SendStatus::Delivered, "{rep:?}");
        assert_eq!(
            svc.contract_outcome_for_test("m-req", sleepy_id),
            Some("replied"),
            "a reply after the rename also closes that contract"
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
            "do it",
            Entrance::Mcp,
            &req_meta("10m", 600),
        )
        .expect("park");
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
                    node: None,
                },
                "twin",
                &["boss".to_string()],
                "done",
                Entrance::Mcp,
                &reply_meta("m-req"),
            )
            .expect("row response");
        assert_eq!(
            rep[0].status,
            SendStatus::Delivered,
            "reply **delivery** is not blocked (only the contract side is a no-op — spec §3 item 7-②): {rep:?}"
        );
        assert_eq!(
            svc.contract_outcome_for_test("m-req", tid("twin")),
            Some("awaiting_reply"),
            "a same-named one that never received it cannot close the contract (no name fallback matching — ADR-0309 decision 7)"
        );

        // ★Even if only one agent has that name, a different agent cannot close it (ADR-0309 decision 5)★ — reviving the name fallback would close it here.
        let (other, other_agent) = live_distinct("twin");
        port.set_roster(vec![boss.clone(), other_agent]);
        svc.handle_send(
            "m-rep-other",
            reply_from(other),
            "twin",
            &["boss".to_string()],
            "done",
            Entrance::Mcp,
            &reply_meta("m-req"),
        )
        .expect("row response");
        assert_eq!(
            svc.contract_outcome_for_test("m-req", tid("twin")),
            Some("awaiting_reply"),
            "another agent with the same name is not the subject of the duty"
        );

        let (solo, solo_agent) = live("twin");
        port.set_roster(vec![boss, solo_agent]);
        svc.handle_send(
            "m-rep2",
            SenderIdentity {
                peer_id: solo,
                epoch: 0,
                node: None,
            },
            "twin",
            &["boss".to_string()],
            "done",
            Entrance::Mcp,
            &reply_meta("m-req"),
        )
        .expect("row response");
        assert_eq!(
            svc.contract_outcome_for_test("m-req", tid("twin")),
            Some("replied"),
            "closed when the very agent that was asleep (same id) wakes and answers"
        );
    }

    #[test]
    fn every_fail_code_has_a_frozen_wire_string() {
        // ★Mutation probe D9-d★: swapping the strings of `FailCode::as_str` left every messaging test green (only 2 daemon tests caught it) — sealed directly in the kernel.
        for (code, wire) in [
            (FailCode::RecipientNotFound, "RECIPIENT_NOT_FOUND"),
            (FailCode::RecipientAmbiguous, "RECIPIENT_AMBIGUOUS"),
            (FailCode::MailboxFull, "MAILBOX_FULL"),
            (FailCode::RequestCapacity, "REQUEST_CAPACITY"),
            (FailCode::RecipientDeleted, "RECIPIENT_DELETED"),
            (FailCode::PeerOffline, "PEER_OFFLINE"),
            (FailCode::RemoteLimit, "REMOTE_LIMIT"),
            (FailCode::RemoteRefused, "REMOTE_REFUSED"),
            (FailCode::NameTooLong, "NAME_TOO_LONG"),
            (FailCode::BodyTooLarge, "BODY_TOO_LARGE"),
        ] {
            assert_eq!(code.as_str(), wire, "the wire string of {code:?} changed");
            // Lets the compiler catch a missing variant (when a new code comes, this match breaks).
            match code {
                FailCode::RecipientNotFound
                | FailCode::RecipientAmbiguous
                | FailCode::MailboxFull
                | FailCode::RequestCapacity
                | FailCode::RecipientDeleted
                | FailCode::PeerOffline
                | FailCode::RemoteLimit
                | FailCode::RemoteRefused
                | FailCode::NameTooLong
                | FailCode::BodyTooLarge => {}
            }
        }
        // `FailCode::ALL` (used to read a stored code back) must list every variant once.
        assert_eq!(FailCode::ALL.len(), 10);
        for c in FailCode::ALL {
            assert_eq!(FailCode::from_wire(c.as_str()), Some(c));
        }
    }

    // ── Remote mail (S37 U3a — ADR-0390 · ADR-0391 · ADR-0393) ──────────────────────────────

    #[derive(Default)]
    struct FakeRemote {
        sent: StdMutex<Vec<OutboundMail>>,
        overdue: StdMutex<Vec<(NodeRef, String)>>,
        local_ids: StdMutex<Vec<PeerId>>,
    }

    impl RemoteDeliveryPort for FakeRemote {
        fn enqueue(&self, mail: OutboundMail) {
            self.sent.lock().unwrap().push(mail);
        }
        fn send_reply_overdue(&self, node: NodeRef, request_id: &str) {
            self.overdue
                .lock()
                .unwrap()
                .push((node, request_id.to_string()));
        }
        fn is_local_agent_id(&self, id: PeerId) -> bool {
            self.local_ids.lock().unwrap().contains(&id)
        }
    }

    impl FakeRemote {
        fn sent(&self) -> Vec<OutboundMail> {
            self.sent.lock().unwrap().clone()
        }
    }

    const LIMITS: RemoteLimits = RemoteLimits {
        mails_per_minute: 600,
        max_body_bytes: 64 * 1024,
    };

    fn svc_remote_with(
        limits: RemoteLimits,
    ) -> (
        Arc<MessagingService>,
        Arc<FakeDeliveryPort>,
        Arc<FakeRemote>,
    ) {
        let port = Arc::new(FakeDeliveryPort::new());
        let remote = Arc::new(FakeRemote::default());
        let svc = Arc::new(
            MessagingService::new(port.clone(), Arc::new(FakeControlPlane))
                .with_remote(remote.clone(), limits),
        );
        (svc, port, remote)
    }

    fn svc_remote() -> (
        Arc<MessagingService>,
        Arc<FakeDeliveryPort>,
        Arc<FakeRemote>,
    ) {
        svc_remote_with(LIMITS)
    }

    fn node(b: u8) -> NodeRef {
        NodeRef::from_bytes([b; 32])
    }

    /// The id an agent `agent` on PC `pc` carries in that PC's directory copy.
    fn rid(pc: &str, agent: &str) -> PeerId {
        tid(&format!("{pc}/{agent}"))
    }

    /// A listed PC; `dir = None` = its directory is unknown this run.
    fn pc(b: u8, name: &str, link: PcLink, dir: Option<&[&str]>) -> RemotePc {
        RemotePc {
            node: node(b),
            name: name.to_string(),
            link,
            directory: dir.map(|agents| {
                agents
                    .iter()
                    .map(|a| DirectoryEntry {
                        id: rid(name, a),
                        name: a.to_string(),
                    })
                    .collect()
            }),
        }
    }

    fn send_as(
        svc: &MessagingService,
        from: SenderIdentity,
        sender: &str,
        id: &str,
        to: &[&str],
        meta: &SendMeta,
    ) -> Vec<RecipientResult> {
        let to: Vec<String> = to.iter().map(|s| s.to_string()).collect();
        svc.handle_send(id, from, sender, &to, "hi", Entrance::Mcp, meta)
            .expect("no bounce")
    }

    /// A live sender `alice` in the roster plus `extra` live agents; returns alice's identity.
    fn with_alice(port: &FakeDeliveryPort, extra: Vec<LiveAgent>) -> SenderIdentity {
        let (alice, alice_agent) = live_sender("alice");
        let mut roster = vec![alice_agent];
        roster.extend(extra);
        port.set_roster(roster);
        alice
    }

    fn inbound(id: &str, from_name: &str, rows: &[(u32, &str)]) -> InboundMail {
        InboundMail {
            id: id.to_string(),
            from_agent_id: rid("away", from_name),
            from_name: from_name.to_string(),
            rows: rows
                .iter()
                .map(|(tag, to)| InboundRow {
                    tag: *tag,
                    to: to.to_string(),
                })
                .collect(),
            body: "hello from away".to_string(),
        }
    }

    fn refusal_code(r: Result<Vec<RemoteRowResult>, RemoteRefusal>) -> (FailCode, String) {
        let e = r.expect_err("the frame must be refused");
        (e.code, e.reason)
    }

    #[test]
    fn name_at_pc_wins_over_a_local_agent_literally_named_that_token_and_does_not_consult_the_copy()
    {
        let (svc, port, remote) = svc_remote();
        let (_, literal) = live("bob@home");
        let alice = with_alice(&port, vec![literal]);
        // The copy of "Home" does not list bob — `name@pc` goes there anyway (the receiver answers authoritatively); the PC name matches ignoring case.
        port.set_pcs(vec![pc(1, "Home", PcLink::Connected, Some(&[]))]);

        let rows = send_as(
            &svc,
            alice,
            "alice",
            "m-1",
            &["bob@home"],
            &SendMeta::default(),
        );

        assert_eq!(rows[0].status, SendStatus::Pending);
        assert!(
            port.injected_bodies().is_empty(),
            "the local literal namesake must not get it"
        );
        let sent = remote.sent();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].node, node(1));
        assert_eq!(
            sent[0].rows,
            vec![OutboundRow {
                tag: 0,
                to: "bob".to_string()
            }]
        );
        assert_eq!(
            svc.ledger_statuses("m-1"),
            vec![DeliveryStatus::RemotePending]
        );
    }

    #[test]
    fn a_local_name_or_id_wins_over_a_remote_agent_of_the_same_name() {
        let (svc, port, remote) = svc_remote();
        let (bob_id, bob) = live("bob");
        let alice = with_alice(&port, vec![bob]);
        // The copy lists a "bob" and even lists the local bob's id.
        let mut home = pc(1, "home", PcLink::Connected, Some(&["bob"]));
        home.directory.as_mut().unwrap().push(DirectoryEntry {
            id: bob_id,
            name: "bob".to_string(),
        });
        port.set_pcs(vec![home]);

        let by_name = send_as(&svc, alice, "alice", "m-1", &["bob"], &SendMeta::default());
        let by_id = send_as(
            &svc,
            alice,
            "alice",
            "m-2",
            &[&bob_id.to_string()],
            &SendMeta::default(),
        );

        assert_eq!(by_name[0].status, SendStatus::Delivered);
        assert_eq!(by_id[0].status, SendStatus::Delivered);
        assert!(remote.sent().is_empty());
    }

    #[test]
    fn a_remote_id_goes_to_the_copy_that_lists_it_and_two_copies_listing_it_are_ambiguous() {
        let (svc, port, remote) = svc_remote();
        let alice = with_alice(&port, vec![]);
        let shared = PeerId::new_v4();
        let mut work = pc(2, "work", PcLink::Connected, Some(&["carol"]));
        let mut lab = pc(3, "lab", PcLink::Connected, Some(&[]));
        port.set_pcs(vec![
            pc(1, "home", PcLink::Connected, Some(&["bob"])),
            work.clone(),
            lab.clone(),
        ]);

        let one = send_as(
            &svc,
            alice,
            "alice",
            "m-1",
            &[&rid("home", "bob").to_string()],
            &SendMeta::default(),
        );
        assert_eq!(one[0].status, SendStatus::Pending);
        assert_eq!(remote.sent()[0].node, node(1));
        assert_eq!(remote.sent()[0].rows[0].to, rid("home", "bob").to_string());

        for (copy, name) in [(&mut work, "dave"), (&mut lab, "eve")] {
            copy.directory.as_mut().unwrap().push(DirectoryEntry {
                id: shared,
                name: name.to_string(),
            });
        }
        port.set_pcs(vec![work, lab]);
        let two = send_as(
            &svc,
            alice,
            "alice",
            "m-2",
            &[&shared.to_string()],
            &SendMeta::default(),
        );
        assert_eq!(two[0].code, Some(FailCode::RecipientAmbiguous));
        assert!(two[0]
            .hint
            .as_deref()
            .unwrap()
            .contains("dave@work, eve@lab"));
        assert_eq!(remote.sent().len(), 1, "an ambiguous id sends nothing");
    }

    #[test]
    fn a_bare_name_in_one_connected_copy_goes_to_that_pc() {
        let (svc, port, remote) = svc_remote();
        let alice = with_alice(&port, vec![]);
        port.set_pcs(vec![
            pc(1, "home", PcLink::Connected, Some(&["bob"])),
            pc(2, "work", PcLink::Connected, Some(&["carol"])),
        ]);

        let rows = send_as(&svc, alice, "alice", "m-1", &["bob"], &SendMeta::default());

        assert_eq!(rows[0].status, SendStatus::Pending);
        let sent = remote.sent();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].node, node(1));
        assert_eq!(sent[0].rows[0].to, "bob");
        assert_eq!(sent[0].sender_name, "alice");
        assert_eq!(sent[0].from, alice);
    }

    #[test]
    fn a_pc_without_a_live_link_fails_peer_offline_at_once_with_the_cause_and_sends_nothing() {
        let (svc, port, remote) = svc_remote();
        let alice = with_alice(&port, vec![]);
        port.set_pcs(vec![
            pc(1, "home", PcLink::Offline, Some(&["bob"])),
            pc(2, "work", PcLink::Refused, Some(&[])),
            pc(3, "lab", PcLink::VersionMismatch, None),
        ]);

        let rows = send_as(
            &svc,
            alice,
            "alice",
            "m-1",
            &["bob", "carol@work", "eve@lab"],
            &SendMeta::default(),
        );

        assert!(rows.iter().all(|r| r.code == Some(FailCode::PeerOffline)));
        assert!(rows[0]
            .hint
            .as_deref()
            .unwrap()
            .contains("'home' is offline"));
        assert!(rows[1]
            .hint
            .as_deref()
            .unwrap()
            .contains("does not list this PC's ID"));
        assert!(rows[2]
            .hint
            .as_deref()
            .unwrap()
            .contains("incompatible Engram version"));
        assert!(remote.sent().is_empty());
        assert_eq!(svc.ledger_statuses("m-1"), vec![DeliveryStatus::Failed; 3]);
    }

    #[test]
    fn a_bare_name_on_two_pcs_is_ambiguous_with_sorted_candidates_even_when_one_is_disconnected() {
        let (svc, port, remote) = svc_remote();
        let alice = with_alice(&port, vec![]);
        port.set_pcs(vec![
            pc(1, "work", PcLink::Connected, Some(&["bob"])),
            pc(2, "home", PcLink::Offline, Some(&["bob"])),
        ]);

        let rows = send_as(&svc, alice, "alice", "m-1", &["bob"], &SendMeta::default());

        assert_eq!(rows[0].status, SendStatus::Failed);
        assert_eq!(rows[0].code, Some(FailCode::RecipientAmbiguous));
        assert!(rows[0]
            .hint
            .as_deref()
            .unwrap()
            .contains("(bob@home, bob@work)"));
        assert!(remote.sent().is_empty());
    }

    #[test]
    fn a_pc_whose_directory_is_unknown_is_not_counted_so_the_bare_name_goes_to_the_known_pc() {
        let (svc, port, remote) = svc_remote();
        let alice = with_alice(&port, vec![]);
        port.set_pcs(vec![
            pc(1, "home", PcLink::Connected, Some(&["bob"])),
            pc(2, "work", PcLink::Connected, None),
        ]);

        let rows = send_as(&svc, alice, "alice", "m-1", &["bob"], &SendMeta::default());

        assert_eq!(rows[0].status, SendStatus::Pending);
        assert_eq!(remote.sent()[0].node, node(1));
    }

    #[test]
    fn not_found_anywhere_names_the_pcs_whose_directory_is_unknown() {
        let (svc, port, remote) = svc_remote();
        let alice = with_alice(&port, vec![]);
        port.set_pcs(vec![
            pc(1, "home", PcLink::Connected, Some(&["bob"])),
            pc(2, "work", PcLink::Offline, None),
        ]);

        let rows = send_as(&svc, alice, "alice", "m-1", &["zed"], &SendMeta::default());

        assert_eq!(rows[0].code, Some(FailCode::RecipientNotFound));
        let hint = rows[0].hint.as_deref().unwrap();
        assert!(
            hint.contains("not known yet") && hint.contains("work"),
            "{hint}"
        );
        assert!(!hint.contains("home"), "a known PC was searched: {hint}");
        assert!(remote.sent().is_empty());
    }

    #[test]
    fn group_addresses_never_expand_to_remote_agents() {
        let (svc, port, remote) = svc_remote();
        let (_, carol) = live("carol");
        let alice = with_alice(&port, vec![carol]);
        port.set_pcs(vec![pc(1, "home", PcLink::Connected, Some(&["bob"]))]);

        let rows = send_as(&svc, alice, "alice", "m-1", &["@all"], &SendMeta::default());

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].to, "carol");
        assert!(remote.sent().is_empty());
    }

    #[test]
    fn rows_naming_one_pc_and_token_fold_into_one_and_one_mail_goes_per_pc() {
        let (svc, port, remote) = svc_remote();
        let alice = with_alice(&port, vec![]);
        port.set_pcs(vec![
            pc(1, "home", PcLink::Connected, Some(&["bob", "dan"])),
            pc(2, "work", PcLink::Connected, Some(&["carol"])),
        ]);

        let rows = send_as(
            &svc,
            alice,
            "alice",
            "m-1",
            &["bob", "carol", "bob@home", "dan"],
            &SendMeta::default(),
        );

        assert_eq!(rows.len(), 3, "bob and bob@home are one row");
        let sent = remote.sent();
        assert_eq!(sent.len(), 2);
        let home = sent.iter().find(|m| m.node == node(1)).unwrap();
        assert_eq!(
            home.rows,
            vec![
                OutboundRow {
                    tag: 0,
                    to: "bob".to_string()
                },
                OutboundRow {
                    tag: 2,
                    to: "dan".to_string()
                },
            ]
        );
        let work = sent.iter().find(|m| m.node == node(2)).unwrap();
        assert_eq!(work.rows[0].tag, 1);
    }

    #[test]
    fn a_sender_whose_name_breaks_the_remote_name_rule_fails_name_too_long_and_sends_nothing() {
        let (svc, port, remote) = svc_remote();
        let alice = with_alice(&port, vec![]);
        port.set_pcs(vec![pc(1, "home", PcLink::Connected, Some(&["bob"]))]);
        let long = "a".repeat(REMOTE_NAME_MAX_BYTES + 1);

        let rows = send_as(&svc, alice, &long, "m-1", &["bob"], &SendMeta::default());
        let ctrl = send_as(
            &svc,
            alice,
            "al\u{7}ice",
            "m-2",
            &["bob"],
            &SendMeta::default(),
        );

        assert_eq!(rows[0].code, Some(FailCode::NameTooLong));
        assert_eq!(ctrl[0].code, Some(FailCode::NameTooLong));
        assert!(remote.sent().is_empty());
        assert!(remote_name_ok(&"a".repeat(REMOTE_NAME_MAX_BYTES)));
    }

    #[test]
    fn verdicts_move_remote_pending_rows_only_by_tag_and_node() {
        let (svc, port, remote) = svc_remote();
        let alice = with_alice(&port, vec![]);
        port.set_pcs(vec![pc(
            1,
            "home",
            PcLink::Connected,
            Some(&["bob", "dan", "eve", "fay"]),
        )]);
        send_as(
            &svc,
            alice,
            "alice",
            "m-1",
            &["bob", "dan", "eve", "fay"],
            &SendMeta::default(),
        );
        assert_eq!(
            svc.ledger_statuses("m-1"),
            vec![DeliveryStatus::RemotePending; 4]
        );
        // The local sweep never moves a remote row (swept inside the verdict horizon, so the rows can still take their verdict).
        svc.sweep(Instant::now() + Duration::from_secs(5 * 60));
        assert_eq!(
            svc.ledger_statuses("m-1"),
            vec![DeliveryStatus::RemotePending; 4]
        );

        // Another node's verdict for the same tag is ignored.
        svc.handle_remote_verdict(
            "m-1",
            node(9),
            &[RemoteRowVerdict {
                tag: 0,
                outcome: RemoteOutcome::UnknownRecipient,
            }],
        );
        assert_eq!(svc.ledger_statuses("m-1")[0], DeliveryStatus::RemotePending);

        svc.handle_remote_verdict(
            "m-1",
            node(1),
            &[
                RemoteRowVerdict {
                    tag: 0,
                    outcome: RemoteOutcome::Delivered {
                        recipient_id: rid("home", "bob"),
                    },
                },
                RemoteRowVerdict {
                    tag: 1,
                    outcome: RemoteOutcome::Held {
                        recipient_id: rid("home", "dan"),
                    },
                },
                RemoteRowVerdict {
                    tag: 2,
                    outcome: RemoteOutcome::Refused {
                        code: Some("NOT_A_CODE".to_string()),
                        reason: None,
                    },
                },
                RemoteRowVerdict {
                    tag: 3,
                    outcome: RemoteOutcome::Unconfirmed,
                },
            ],
        );
        assert_eq!(
            svc.ledger_statuses("m-1"),
            vec![
                DeliveryStatus::Delivered,
                DeliveryStatus::RemoteHeld,
                DeliveryStatus::Failed,
                DeliveryStatus::RemotePending,
            ]
        );
        let view = svc.message_state("m-1", Instant::now()).unwrap();
        assert_eq!(
            view.rows[2].code,
            Some("REMOTE_REFUSED"),
            "an unknown verdict code maps to REMOTE_REFUSED"
        );
        assert_eq!(view.rows[1].status, "pending");
        assert_eq!(view.rows[3].status, "pending");

        // A second verdict for a settled row changes nothing.
        svc.handle_remote_verdict(
            "m-1",
            node(1),
            &[RemoteRowVerdict {
                tag: 0,
                outcome: RemoteOutcome::UnknownRecipient,
            }],
        );
        assert_eq!(svc.ledger_statuses("m-1")[0], DeliveryStatus::Delivered);
        assert!(remote.sent().len() == 1);
    }

    #[test]
    fn a_verdict_is_matched_by_tag_after_the_pc_is_renamed_mid_request() {
        let (svc, port, _remote) = svc_remote();
        let alice = with_alice(&port, vec![]);
        port.set_pcs(vec![pc(1, "home", PcLink::Connected, Some(&["bob"]))]);
        send_as(
            &svc,
            alice,
            "alice",
            "m-1",
            &["bob@home"],
            &SendMeta::default(),
        );
        port.set_pcs(vec![pc(1, "attic", PcLink::Connected, Some(&["bob"]))]);

        svc.handle_remote_verdict(
            "m-1",
            node(1),
            &[RemoteRowVerdict {
                tag: 0,
                outcome: RemoteOutcome::Delivered {
                    recipient_id: rid("home", "bob"),
                },
            }],
        );

        assert_eq!(svc.ledger_statuses("m-1"), vec![DeliveryStatus::Delivered]);
    }

    #[test]
    fn a_verdict_before_the_seal_is_in_the_answer_and_gives_no_notice() {
        let (svc, port, _remote) = svc_remote();
        let alice = with_alice(&port, vec![]);
        port.set_pcs(vec![pc(
            1,
            "home",
            PcLink::Connected,
            Some(&["bob", "dan", "eve"]),
        )]);
        let mut rows = send_as(
            &svc,
            alice,
            "alice",
            "m-1",
            &["bob", "dan", "eve"],
            &SendMeta::default(),
        );
        svc.handle_remote_verdict(
            "m-1",
            node(1),
            &[
                RemoteRowVerdict {
                    tag: 0,
                    outcome: RemoteOutcome::Held {
                        recipient_id: rid("home", "bob"),
                    },
                },
                RemoteRowVerdict {
                    tag: 1,
                    outcome: RemoteOutcome::Refused {
                        code: Some("REMOTE_LIMIT".to_string()),
                        reason: Some("too\nmany".to_string()),
                    },
                },
                RemoteRowVerdict {
                    tag: 2,
                    outcome: RemoteOutcome::NotSent(FailCode::PeerOffline),
                },
            ],
        );

        // Every row settled: returns without waiting even with a far deadline.
        svc.await_remote_answer("m-1", &mut rows, Instant::now() + Duration::from_secs(3600));

        assert_eq!(rows[0].status, SendStatus::Pending);
        assert!(rows[0]
            .hint
            .as_deref()
            .unwrap()
            .contains("Held on PC 'home'"));
        assert_eq!(rows[1].code, Some(FailCode::RemoteLimit));
        let hint = rows[1].hint.as_deref().unwrap();
        assert!(
            hint.contains("PC 'home' says: \"too\\nmany\""),
            "reason labelled with the PC, control characters escaped: {hint}"
        );
        assert_eq!(rows[2].code, Some(FailCode::PeerOffline));
        assert!(
            port.injected_bodies().is_empty(),
            "no notice for rows already in the answer"
        );
        assert_eq!(svc.parked_len(alice.peer_id), 0);
    }

    #[test]
    fn a_row_without_a_verdict_at_the_deadline_is_unconfirmed_and_a_later_verdict_comes_as_exactly_one_notice(
    ) {
        let (svc, port, _remote) = svc_remote();
        let alice = with_alice(&port, vec![]);
        port.set_pcs(vec![pc(
            1,
            "home",
            PcLink::Connected,
            Some(&["bob", "dan"]),
        )]);
        let mut rows = send_as(
            &svc,
            alice,
            "alice",
            "m-1",
            &["bob", "dan"],
            &SendMeta::default(),
        );

        // The deadline has passed: seal at once.
        svc.await_remote_answer("m-1", &mut rows, Instant::now());
        for r in &rows {
            assert_eq!(r.status, SendStatus::Pending, "unconfirmed is never failed");
            assert!(r
                .hint
                .as_deref()
                .unwrap()
                .contains("may or may not have arrived"));
        }

        svc.handle_remote_verdict(
            "m-1",
            node(1),
            &[
                RemoteRowVerdict {
                    tag: 0,
                    outcome: RemoteOutcome::Refused {
                        code: Some("MAILBOX_FULL".to_string()),
                        reason: Some("secret peer words".to_string()),
                    },
                },
                RemoteRowVerdict {
                    tag: 1,
                    outcome: RemoteOutcome::Unconfirmed,
                },
            ],
        );

        assert_eq!(
            svc.ledger_statuses("m-1"),
            vec![DeliveryStatus::Failed, DeliveryStatus::RemotePending]
        );
        let notices = port.injected_bodies();
        assert_eq!(
            notices.len(),
            1,
            "one notice for the late verdict: {notices:?}"
        );
        assert!(notices[0].contains("[engram] mail m-1"));
        assert!(notices[0].contains("bob — failed (MAILBOX_FULL)"));
        assert!(notices[0].contains("dan — outcome unknown"));
        assert!(
            !notices[0].contains("secret peer words"),
            "a peer's reason never enters a notice"
        );

        // A repeated verdict produces no second notice.
        svc.handle_remote_verdict(
            "m-1",
            node(1),
            &[RemoteRowVerdict {
                tag: 0,
                outcome: RemoteOutcome::Delivered {
                    recipient_id: rid("home", "bob"),
                },
            }],
        );
        assert_eq!(port.injected_bodies().len(), 1);
    }

    #[test]
    fn the_answer_waits_for_a_verdict_that_arrives_after_the_wait_started() {
        let (svc, port, _remote) = svc_remote();
        let alice = with_alice(&port, vec![]);
        port.set_pcs(vec![pc(1, "home", PcLink::Connected, Some(&["bob"]))]);
        let mut rows = send_as(&svc, alice, "alice", "m-1", &["bob"], &SendMeta::default());
        // The hook fires inside the wait (lock held) — the verdict thread starts only then, so it can only be seen through the wake-up.
        let (started_tx, started_rx) = std::sync::mpsc::channel::<()>();
        let started_tx = StdMutex::new(started_tx);
        assert!(svc
            .wait_hook
            .set(Box::new(move || {
                let _ = started_tx.lock().unwrap().send(());
            }))
            .is_ok());

        let svc2 = svc.clone();
        let verdict = std::thread::spawn(move || {
            started_rx.recv().unwrap();
            svc2.handle_remote_verdict(
                "m-1",
                node(1),
                &[RemoteRowVerdict {
                    tag: 0,
                    outcome: RemoteOutcome::Delivered {
                        recipient_id: rid("home", "bob"),
                    },
                }],
            );
        });
        let started = Instant::now();
        svc.await_remote_answer("m-1", &mut rows, started + Duration::from_secs(30));
        let waited = started.elapsed();
        verdict.join().unwrap();

        assert_eq!(rows[0].status, SendStatus::Delivered);
        assert!(
            waited < Duration::from_secs(10),
            "woken by the verdict, not by the deadline: {waited:?}"
        );
        assert!(port.injected_bodies().is_empty());
    }

    #[test]
    fn a_send_without_remote_rows_is_left_alone_by_the_answer_wait() {
        let (svc, port, _remote) = svc_remote();
        let (_, bob) = live("bob");
        let alice = with_alice(&port, vec![bob]);
        let mut rows = send_as(&svc, alice, "alice", "m-1", &["bob"], &SendMeta::default());
        let before = rows.clone();

        svc.await_remote_answer("m-1", &mut rows, Instant::now() + Duration::from_secs(600));

        assert_eq!(rows, before);
    }

    #[test]
    fn remote_rows_without_remote_wiring_fail_peer_offline() {
        let (svc, port) = svc();
        let alice = with_alice(&port, vec![]);
        port.set_pcs(vec![pc(1, "home", PcLink::Connected, Some(&["bob"]))]);

        let rows = send_as(&svc, alice, "alice", "m-1", &["bob"], &SendMeta::default());

        assert_eq!(rows[0].code, Some(FailCode::PeerOffline));
    }

    #[test]
    fn remote_codes_map_onto_the_closed_set() {
        for (wire, code) in [
            (Some("REMOTE_LIMIT"), FailCode::RemoteLimit),
            (Some("RECIPIENT_AMBIGUOUS"), FailCode::RecipientAmbiguous),
            (Some("MAILBOX_FULL"), FailCode::MailboxFull),
            (Some("REQUEST_CAPACITY"), FailCode::RequestCapacity),
            (Some("REMOTE_REFUSED"), FailCode::RemoteRefused),
            (Some("RECIPIENT_DELETED"), FailCode::RemoteRefused),
            (Some("PEER_OFFLINE"), FailCode::RemoteRefused),
            (Some("whatever"), FailCode::RemoteRefused),
            (None, FailCode::RemoteRefused),
        ] {
            assert_eq!(FailCode::from_remote_code(wire), code, "{wire:?}");
        }
    }

    #[test]
    fn the_peer_reason_is_capped_on_a_character_boundary() {
        let long = "é".repeat(REMOTE_REASON_MAX_BYTES);
        let clipped = clip_reason(&long);
        assert!(clipped.len() <= REMOTE_REASON_MAX_BYTES);
        assert_eq!(clipped.len(), REMOTE_REASON_MAX_BYTES);
        assert_eq!(clip_reason("a\u{1b}b"), "a\\u{1b}b");
    }

    // ── Remote entrance (receiving side) ──

    #[test]
    fn the_remote_entrance_answers_every_tag_and_folds_an_id_and_a_name_of_one_agent() {
        let (svc, port, _remote) = svc_remote();
        let (bob_id, bob) = live("bob");
        port.set_roster(vec![bob]);
        port.set_dormant(&["sleepy"]);
        let mail = inbound(
            "r-1",
            "alice",
            &[
                (7, "bob"),
                (3, &bob_id.to_string()),
                (5, "sleepy"),
                (9, "nobody"),
            ],
        );

        let rows = svc
            .handle_remote_mail(node(4), "away", &mail, &SendMeta::default(), Instant::now())
            .expect("accepted");

        assert_eq!(
            rows,
            vec![
                RemoteRowResult {
                    tag: 7,
                    outcome: InboundOutcome::Delivered {
                        recipient_id: bob_id
                    }
                },
                RemoteRowResult {
                    tag: 3,
                    outcome: InboundOutcome::Delivered {
                        recipient_id: bob_id
                    }
                },
                RemoteRowResult {
                    tag: 5,
                    outcome: InboundOutcome::Held {
                        recipient_id: tid("sleepy")
                    }
                },
                RemoteRowResult {
                    tag: 9,
                    outcome: InboundOutcome::UnknownRecipient
                },
            ]
        );
        let bodies = port.injected_bodies();
        assert_eq!(bodies.len(), 1, "two rows naming one agent deliver once");
        assert!(
            bodies[0].contains("alice@away"),
            "the sender shows as name@pc: {}",
            bodies[0]
        );
    }

    #[test]
    fn the_remote_entrance_resolves_locally_only_and_never_forwards() {
        let (svc, port, remote) = svc_remote();
        port.set_pcs(vec![pc(1, "home", PcLink::Connected, Some(&["bob"]))]);
        let mail = inbound(
            "r-1",
            "alice",
            &[
                (0, "bob"),
                (1, "bob@home"),
                (2, &rid("home", "bob").to_string()),
            ],
        );

        let rows = svc
            .handle_remote_mail(node(4), "away", &mail, &SendMeta::default(), Instant::now())
            .expect("accepted");

        assert!(rows
            .iter()
            .all(|r| r.outcome == InboundOutcome::UnknownRecipient));
        assert!(remote.sent().is_empty());
    }

    #[test]
    fn the_remote_entrance_refuses_an_invalid_frame_whole() {
        let (svc, port, remote) = svc_remote();
        let (bob_id, bob) = live("bob");
        port.set_roster(vec![bob]);
        let now = Instant::now();
        let ok_meta = SendMeta::default();
        let check = |mail: InboundMail, meta: &SendMeta, needle: &str| {
            let (code, reason) =
                refusal_code(svc.handle_remote_mail(node(4), "away", &mail, meta, now));
            assert_eq!(code, FailCode::RemoteRefused, "{needle}");
            assert!(reason.contains(needle), "{reason} should mention {needle}");
        };

        check(inbound("r-1", "alice", &[]), &ok_meta, "no recipients");
        let many: Vec<(u32, &str)> = (0..=MAX_REMOTE_ROWS as u32).map(|t| (t, "bob")).collect();
        check(inbound("r-2", "alice", &many), &ok_meta, "more than 64");
        check(
            inbound("r-3", "alice", &[(1, "bob"), (1, "bob")]),
            &ok_meta,
            "duplicate row tag",
        );
        check(
            inbound("r-4", "alice", &[(1, " @all")]),
            &ok_meta,
            "group addresses",
        );
        check(
            inbound("r-5", "alice", &[(1, "@here")]),
            &ok_meta,
            "group addresses",
        );
        check(
            inbound("r-6", "alice", &[(1, "  ")]),
            &ok_meta,
            "empty recipient",
        );
        check(
            inbound("r-7", "alice", &[(1, "bob"), (2, &bob_id.to_string())]),
            &reply_meta("m-x"),
            "exactly one recipient",
        );
        check(inbound("r-8", "", &[(1, "bob")]), &ok_meta, "sender name");
        check(
            inbound("r-9", &"a".repeat(REMOTE_NAME_MAX_BYTES + 1), &[(1, "bob")]),
            &ok_meta,
            "sender name",
        );
        check(
            inbound("r-10", "al\nice", &[(1, "bob")]),
            &ok_meta,
            "sender name",
        );
        let mut nil = inbound("r-11", "alice", &[(1, "bob")]);
        nil.from_agent_id = PeerId::nil();
        check(nil, &ok_meta, "nil sender id");

        // R8: a sender id that is also an agent of this PC.
        let copied = inbound("r-12", "alice", &[(1, "bob")]);
        remote.local_ids.lock().unwrap().push(copied.from_agent_id);
        check(copied, &ok_meta, "copied data folder");

        assert!(
            port.injected_bodies().is_empty(),
            "a refused frame delivers nothing"
        );
        assert!(
            svc.ledger_snapshot().is_empty(),
            "a refused frame records nothing"
        );
    }

    #[test]
    fn a_colliding_mail_id_is_refused_by_the_remote_entrance() {
        let (svc, port, _remote) = svc_remote();
        let (_, bob) = live("bob");
        let alice = with_alice(&port, vec![bob]);
        send_as(
            &svc,
            alice,
            "alice",
            "same-id",
            &["bob"],
            &SendMeta::default(),
        );

        let (code, _) = refusal_code(svc.handle_remote_mail(
            node(4),
            "away",
            &inbound("same-id", "carl", &[(0, "bob")]),
            &SendMeta::default(),
            Instant::now(),
        ));

        assert_eq!(code, FailCode::RemoteRefused);
    }

    #[test]
    fn the_per_pc_backstop_refuses_over_the_limit_and_the_body_cap_with_remote_limit() {
        let (svc, port, _remote) = svc_remote_with(RemoteLimits {
            mails_per_minute: 3,
            max_body_bytes: 10,
        });
        let (_, bob) = live("bob");
        port.set_roster(vec![bob]);
        let t0 = Instant::now();
        let mail = |i: usize, from: &str| {
            let mut m = inbound(&format!("r-{i}"), from, &[(0, "bob")]);
            m.body = "short".to_string();
            m
        };
        // Different senders on the same PC share the PC's budget — no per-agent or per-pair limit below it.
        for (i, from) in ["alice", "carl", "dora"].iter().enumerate() {
            assert!(svc
                .handle_remote_mail(node(4), "away", &mail(i, from), &SendMeta::default(), t0)
                .is_ok());
        }
        let (code, reason) = refusal_code(svc.handle_remote_mail(
            node(4),
            "away",
            &mail(3, "alice"),
            &SendMeta::default(),
            t0 + Duration::from_secs(59),
        ));
        assert_eq!(code, FailCode::RemoteLimit);
        assert!(reason.contains("3 mails per minute"));
        // Another PC has its own budget; the window slides.
        assert!(svc
            .handle_remote_mail(node(5), "lab", &mail(4, "alice"), &SendMeta::default(), t0)
            .is_ok());
        assert!(svc
            .handle_remote_mail(
                node(4),
                "away",
                &mail(5, "alice"),
                &SendMeta::default(),
                t0 + Duration::from_secs(60)
            )
            .is_ok());

        let mut big = mail(6, "alice");
        big.body = "x".repeat(11);
        let (code, _) =
            refusal_code(svc.handle_remote_mail(node(6), "far", &big, &SendMeta::default(), t0));
        assert_eq!(code, FailCode::RemoteLimit);
    }

    #[test]
    fn a_request_from_another_pc_keeps_the_requester_node_and_reports_overdue_to_it() {
        let (svc, port, remote) = svc_remote();
        let (bob_id, bob) = live("bob");
        port.set_roster(vec![bob]);
        let mail = inbound("r-1", "alice", &[(0, "bob")]);

        svc.handle_remote_mail(node(4), "away", &mail, &req_meta("5m", 300), Instant::now())
            .expect("accepted");
        assert_eq!(
            svc.contract_outcome_for_test("r-1", bob_id),
            Some("awaiting_reply")
        );
        svc.sweep(Instant::now() + Duration::from_secs(301));

        assert_eq!(
            *remote.overdue.lock().unwrap(),
            vec![(node(4), "r-1".to_string())]
        );
        // No local notice: the requester is not on this PC (only bob's request was injected).
        assert_eq!(port.injected_bodies().len(), 1);
    }

    /// B's side of a reply to a request from PC A: the contract closes at verdict time by ADR-0116 decision 2's rule, and no overdue report goes out
    ///   while the reply's verdict is in flight.
    fn reply_to_remote_requester(
        outcome: RemoteOutcome,
    ) -> (Arc<MessagingService>, Arc<FakeRemote>, PeerId) {
        let (svc, port, remote) = svc_remote();
        let (bob, bob_agent) = live_sender("bob");
        port.set_roster(vec![bob_agent]);
        port.set_pcs(vec![pc(4, "away", PcLink::Connected, Some(&["alice"]))]);
        svc.handle_remote_mail(
            node(4),
            "away",
            &inbound("r-1", "alice", &[(0, "bob")]),
            &req_meta("1m", 60),
            Instant::now(),
        )
        .expect("accepted");

        let rows = send_as(
            &svc,
            bob,
            "bob",
            "m-reply",
            &["alice@away"],
            &reply_meta("r-1"),
        );
        assert_eq!(rows[0].status, SendStatus::Pending);
        assert_eq!(remote.sent()[0].meta.reply_to.as_deref(), Some("r-1"));
        assert_eq!(
            svc.contract_outcome_for_test("r-1", bob.peer_id),
            Some("awaiting_reply")
        );

        // Past the deadline while the reply's verdict is in flight: no overdue report.
        svc.sweep(Instant::now() + Duration::from_secs(61));
        assert!(remote.overdue.lock().unwrap().is_empty());

        svc.handle_remote_verdict("m-reply", node(4), &[RemoteRowVerdict { tag: 0, outcome }]);
        (svc, remote, bob.peer_id)
    }

    #[test]
    fn a_reply_to_a_remote_requester_closes_as_replied_when_delivered_or_held() {
        let (svc, _, bob) = reply_to_remote_requester(RemoteOutcome::Delivered {
            recipient_id: rid("away", "alice"),
        });
        assert_eq!(svc.contract_outcome_for_test("r-1", bob), Some("replied"));
        let (svc, _, bob) = reply_to_remote_requester(RemoteOutcome::Held {
            recipient_id: rid("away", "alice"),
        });
        assert_eq!(svc.contract_outcome_for_test("r-1", bob), Some("replied"));
    }

    #[test]
    fn a_reply_to_a_remote_requester_closes_as_reply_failed_on_unknown_recipient() {
        let (svc, _, bob) = reply_to_remote_requester(RemoteOutcome::UnknownRecipient);
        assert_eq!(
            svc.contract_outcome_for_test("r-1", bob),
            Some("reply_failed")
        );
    }

    #[test]
    fn a_reply_to_a_remote_requester_stays_open_on_any_other_outcome_and_the_overdue_report_resumes(
    ) {
        let (svc, remote, bob) = reply_to_remote_requester(RemoteOutcome::Refused {
            code: Some("MAILBOX_FULL".to_string()),
            reason: None,
        });
        assert_eq!(
            svc.contract_outcome_for_test("r-1", bob),
            Some("awaiting_reply")
        );
        svc.sweep(Instant::now() + Duration::from_secs(62));
        assert_eq!(
            *remote.overdue.lock().unwrap(),
            vec![(node(4), "r-1".to_string())]
        );
    }

    #[test]
    fn reply_overdue_is_rendered_by_the_requester_naming_the_pc_once_and_only_for_its_own_request_to_that_node(
    ) {
        let (svc, port, _remote) = svc_remote();
        let alice = with_alice(&port, vec![]);
        port.set_pcs(vec![
            pc(1, "home", PcLink::Connected, Some(&["bob"])),
            pc(2, "work", PcLink::Connected, Some(&["carol"])),
        ]);
        send_as(
            &svc,
            alice,
            "alice",
            "m-req",
            &["bob"],
            &req_meta("10m", 600),
        );
        send_as(
            &svc,
            alice,
            "alice",
            "m-plain",
            &["bob"],
            &SendMeta::default(),
        );
        svc.handle_remote_verdict(
            "m-req",
            node(1),
            &[RemoteRowVerdict {
                tag: 0,
                outcome: RemoteOutcome::Delivered {
                    recipient_id: rid("home", "bob"),
                },
            }],
        );

        let late = Instant::now() + Duration::from_secs(601);
        assert!(
            !svc.handle_reply_overdue(node(2), "m-req", late),
            "another node's report"
        );
        assert!(
            !svc.handle_reply_overdue(node(1), "m-plain", late),
            "not a request"
        );
        assert!(!svc.handle_reply_overdue(node(1), "m-unknown", late));
        assert!(svc.handle_reply_overdue(node(1), "m-req", late));
        assert!(
            !svc.handle_reply_overdue(node(1), "m-req", late),
            "only once"
        );

        let notices = port.injected_bodies();
        assert_eq!(notices.len(), 1);
        assert!(
            notices[0].contains("request m-req deadline (10m) exceeded — no reply from bob@home"),
            "{}",
            notices[0]
        );
    }

    #[test]
    fn park_payload_roundtrip_keeps_the_remote_sender_node_and_entrance() {
        let from = SenderIdentity {
            peer_id: PeerId::new_v4(),
            epoch: 0,
            node: Some(node(0xab)),
        };
        let p = ParkPayload {
            sender_name: "alice@away".to_string(),
            from,
            entrance: Entrance::Remote,
            body: "b\nody".to_string(),
            meta: SendMeta::default(),
        };
        let back = ParkPayload::decode(&p.encode());
        assert_eq!(back.from, from);
        assert_eq!(back.entrance, Entrance::Remote);
        assert_eq!(back.sender_name, "alice@away");
        assert_eq!(back.body, "b\nody");
        assert_eq!(NodeRef::from_hex(&node(0xab).to_hex()), Some(node(0xab)));
        assert_eq!(NodeRef::from_hex("zz"), None);
    }

    // ── Review round 1 (U3a) ──

    #[test]
    fn a_settled_remote_row_never_takes_a_local_rows_transition() {
        let (svc, port, _remote) = svc_remote();
        let (carol_id, carol) = live("carol");
        let alice = with_alice(&port, vec![carol]);
        port.set_pcs(vec![pc(2, "pc2", PcLink::Connected, Some(&["carol"]))]);
        // carol's first injection fails, so her local row stays pending in the queue.
        port.fail_at(&[0]);
        let rows = send_as(
            &svc,
            alice,
            "alice",
            "m-1",
            &["carol", "carol@pc2"],
            &SendMeta::default(),
        );
        assert_eq!(rows[0].status, SendStatus::Pending);
        // PC2 reports the same agent id (a copied data folder, or a lying peer).
        svc.handle_remote_verdict(
            "m-1",
            node(2),
            &[RemoteRowVerdict {
                tag: 1,
                outcome: RemoteOutcome::Held {
                    recipient_id: carol_id,
                },
            }],
        );

        svc.flush_for(carol_id);

        assert_eq!(
            svc.ledger_statuses("m-1"),
            vec![DeliveryStatus::RemoteHeld, DeliveryStatus::Delivered],
            "carol's local row reaches delivered; the remote row is untouched"
        );
    }

    fn reply_from_home(svc: &MessagingService, agent: &str, request_id: &str) {
        let mail = InboundMail {
            id: format!("m-re-{agent}"),
            from_agent_id: rid("home", agent),
            from_name: agent.to_string(),
            rows: vec![InboundRow {
                tag: 0,
                to: "alice".to_string(),
            }],
            body: "answer".to_string(),
        };
        svc.handle_remote_mail(
            node(1),
            "home",
            &mail,
            &reply_meta(request_id),
            Instant::now(),
        )
        .expect("accepted");
    }

    fn overdue_notices(port: &FakeDeliveryPort) -> Vec<String> {
        port.injected_bodies()
            .into_iter()
            .filter(|b| b.contains("[engram] request"))
            .collect()
    }

    /// alice requests from bob@home and dan@home (10 min deadline).
    fn request_to_bob_and_dan() -> (Arc<MessagingService>, Arc<FakeDeliveryPort>) {
        let (svc, port, _remote) = svc_remote();
        let alice = with_alice(&port, vec![]);
        port.set_pcs(vec![pc(
            1,
            "home",
            PcLink::Connected,
            Some(&["bob", "dan"]),
        )]);
        send_as(
            &svc,
            alice,
            "alice",
            "m-req",
            &["bob@home", "dan@home"],
            &req_meta("10m", 600),
        );
        (svc, port)
    }

    #[test]
    fn an_overdue_report_names_only_the_agent_that_did_not_reply_once_and_not_before_the_deadline()
    {
        let (svc, port) = request_to_bob_and_dan();
        svc.handle_remote_verdict(
            "m-req",
            node(1),
            &[
                RemoteRowVerdict {
                    tag: 0,
                    outcome: RemoteOutcome::Delivered {
                        recipient_id: rid("home", "bob"),
                    },
                },
                RemoteRowVerdict {
                    tag: 1,
                    outcome: RemoteOutcome::Delivered {
                        recipient_id: rid("home", "dan"),
                    },
                },
            ],
        );
        reply_from_home(&svc, "bob", "m-req");
        let t = Instant::now();

        assert!(
            !svc.handle_reply_overdue(node(1), "m-req", t + Duration::from_secs(590)),
            "before the deadline"
        );
        assert!(svc.handle_reply_overdue(node(1), "m-req", t + Duration::from_secs(601)));
        assert!(
            !svc.handle_reply_overdue(node(1), "m-req", t + Duration::from_secs(601)),
            "a repeated report gives no second notice"
        );

        let notices = overdue_notices(&port);
        assert_eq!(notices.len(), 1);
        assert!(
            notices[0].contains("no reply from dan@home"),
            "{}",
            notices[0]
        );
        assert!(!notices[0].contains("bob"), "bob replied: {}", notices[0]);
    }

    #[test]
    fn an_overdue_report_with_several_unanswered_rows_names_the_pc_and_lists_them() {
        let (svc, port) = request_to_bob_and_dan();

        assert!(svc.handle_reply_overdue(
            node(1),
            "m-req",
            Instant::now() + Duration::from_secs(601)
        ));

        let notices = overdue_notices(&port);
        assert_eq!(notices.len(), 1);
        assert!(
            notices[0].contains(
                "an agent on home has not replied — not answered yet: bob@home, dan@home"
            ),
            "{}",
            notices[0]
        );
    }

    #[test]
    fn a_reply_that_arrived_for_an_unconfirmed_row_refuses_a_later_overdue_report() {
        let (svc, port, _remote) = svc_remote();
        let alice = with_alice(&port, vec![]);
        port.set_pcs(vec![pc(1, "home", PcLink::Connected, Some(&["bob"]))]);
        send_as(
            &svc,
            alice,
            "alice",
            "m-req",
            &["bob@home"],
            &req_meta("10m", 600),
        );
        // No recipient id comes back: the final verdict call settles the row unconfirmed.
        svc.handle_remote_verdict("m-req", node(1), &[]);
        reply_from_home(&svc, "bob", "m-req");

        assert!(!svc.handle_reply_overdue(
            node(1),
            "m-req",
            Instant::now() + Duration::from_secs(601)
        ));
        assert!(overdue_notices(&port).is_empty());
    }

    #[test]
    fn a_late_verdict_still_gives_its_notice_after_the_history_ring_rolled_over() {
        let (svc, port, _remote) = svc_remote();
        let alice = with_alice(&port, vec![]);
        port.set_pcs(vec![pc(1, "home", PcLink::Connected, Some(&["bob"]))]);
        let mut rows = send_as(&svc, alice, "alice", "m-1", &["bob"], &SendMeta::default());
        svc.await_remote_answer("m-1", &mut rows, Instant::now());
        // More rows than the history ring holds (unknown names — one failed row each).
        for i in 0..4100 {
            send_as(
                &svc,
                alice,
                "alice",
                &format!("m-f{i}"),
                &["zed"],
                &SendMeta::default(),
            );
        }
        assert!(
            svc.ledger_statuses("m-1").is_empty(),
            "the row left the ring"
        );

        svc.handle_remote_verdict(
            "m-1",
            node(1),
            &[RemoteRowVerdict {
                tag: 0,
                outcome: RemoteOutcome::Delivered {
                    recipient_id: rid("home", "bob"),
                },
            }],
        );

        let notices = port.injected_bodies();
        assert_eq!(notices.len(), 1);
        assert!(notices[0].contains("bob — delivered"), "{}", notices[0]);
    }

    #[test]
    fn the_final_verdict_call_settles_every_tag_it_left_out_as_unconfirmed() {
        let (svc, port, _remote) = svc_remote();
        let alice = with_alice(&port, vec![]);
        port.set_pcs(vec![pc(
            1,
            "home",
            PcLink::Connected,
            Some(&["bob", "dan"]),
        )]);
        let mut rows = send_as(
            &svc,
            alice,
            "alice",
            "m-1",
            &["bob", "dan"],
            &SendMeta::default(),
        );

        svc.handle_remote_verdict(
            "m-1",
            node(1),
            &[RemoteRowVerdict {
                tag: 0,
                outcome: RemoteOutcome::Delivered {
                    recipient_id: rid("home", "bob"),
                },
            }],
        );
        // Every row is settled, so the answer does not wait for the far deadline.
        let started = Instant::now();
        svc.await_remote_answer("m-1", &mut rows, started + Duration::from_secs(30));

        assert!(started.elapsed() < Duration::from_secs(10));
        assert_eq!(rows[0].status, SendStatus::Delivered);
        assert_eq!(rows[1].status, SendStatus::Pending);
        assert!(rows[1]
            .hint
            .as_deref()
            .unwrap()
            .contains("may or may not have arrived"));
        // A verdict for the left-out tag after that is ignored.
        svc.handle_remote_verdict(
            "m-1",
            node(1),
            &[RemoteRowVerdict {
                tag: 1,
                outcome: RemoteOutcome::UnknownRecipient,
            }],
        );
        assert_eq!(svc.ledger_statuses("m-1")[1], DeliveryStatus::RemotePending);
    }

    #[test]
    fn a_mixed_send_keeps_the_local_row_and_tags_the_remote_one_by_its_row_index() {
        let (svc, port, remote) = svc_remote();
        let (_, carol) = live("carol");
        let alice = with_alice(&port, vec![carol]);
        port.set_pcs(vec![pc(1, "home", PcLink::Connected, Some(&["bob"]))]);
        let mut rows = send_as(
            &svc,
            alice,
            "alice",
            "m-1",
            &["carol", "bob"],
            &SendMeta::default(),
        );
        let carol_row = rows[0].clone();
        assert_eq!(remote.sent()[0].rows[0].tag, 1);

        svc.handle_remote_verdict(
            "m-1",
            node(1),
            &[RemoteRowVerdict {
                tag: 1,
                outcome: RemoteOutcome::Held {
                    recipient_id: rid("home", "bob"),
                },
            }],
        );
        svc.await_remote_answer("m-1", &mut rows, Instant::now());

        assert_eq!(rows[0], carol_row);
        assert_eq!(rows[0].status, SendStatus::Delivered);
        assert_eq!(rows[1].to, "bob");
        assert!(rows[1]
            .hint
            .as_deref()
            .unwrap()
            .contains("Held on PC 'home'"));
    }

    /// bob owes alice@away an answer and replies with the bare name "alice" while away's directory is `dir`.
    fn bare_reply_to_remote_requester(dir: Option<&[&str]>) -> Option<&'static str> {
        let (svc, port, _remote) = svc_remote();
        let (bob, bob_agent) = live_sender("bob");
        port.set_roster(vec![bob_agent]);
        port.set_pcs(vec![pc(4, "away", PcLink::Connected, dir)]);
        svc.handle_remote_mail(
            node(4),
            "away",
            &inbound("r-1", "alice", &[(0, "bob")]),
            &req_meta("10m", 600),
            Instant::now(),
        )
        .expect("accepted");

        let rows = send_as(&svc, bob, "bob", "m-reply", &["alice"], &reply_meta("r-1"));
        assert_eq!(rows[0].code, Some(FailCode::RecipientNotFound));
        svc.contract_outcome_for_test("r-1", bob.peer_id)
    }

    #[test]
    fn a_bare_name_reply_to_a_requester_on_a_pc_with_an_unknown_directory_leaves_the_contract_open()
    {
        assert_eq!(bare_reply_to_remote_requester(None), Some("awaiting_reply"));
        // With that PC's directory known, "not found" is real.
        assert_eq!(
            bare_reply_to_remote_requester(Some(&[])),
            Some("reply_failed")
        );
    }

    #[test]
    fn the_remote_entrance_refuses_a_bad_mail_id_an_oversized_token_and_broken_contract_fields() {
        let (svc, port, _remote) = svc_remote();
        let (_, bob) = live("bob");
        port.set_roster(vec![bob]);
        let now = Instant::now();
        let refused = |mail: InboundMail, meta: SendMeta, needle: &str| {
            let (code, reason) =
                refusal_code(svc.handle_remote_mail(node(4), "away", &mail, &meta, now));
            assert_eq!(code, FailCode::RemoteRefused, "{needle}");
            assert!(reason.contains(needle), "{reason} should mention {needle}");
        };
        let ok = SendMeta::default();

        refused(inbound("", "alice", &[(0, "bob")]), ok.clone(), "mail id");
        let long_id = "m".repeat(MAX_REMOTE_MSG_ID_BYTES + 1);
        refused(
            inbound(&long_id, "alice", &[(0, "bob")]),
            ok.clone(),
            "mail id",
        );
        refused(
            inbound("m-1\nfake log line", "alice", &[(0, "bob")]),
            ok.clone(),
            "mail id",
        );
        let long_to = "b".repeat(REMOTE_NAME_MAX_BYTES + 1);
        refused(
            inbound("m-2", "alice", &[(0, &long_to)]),
            ok.clone(),
            "over 256 bytes",
        );

        let row = &[(0u32, "bob")][..];
        let mut both = req_meta("10m", 600);
        both.reply_to = Some("m-x".to_string());
        refused(inbound("m-3", "alice", row), both, "request and reply_to");
        let mut no_raw = req_meta("10m", 600);
        no_raw.reply_by_raw = None;
        refused(inbound("m-4", "alice", row), no_raw, "without its notation");
        let mut no_request = req_meta("10m", 600);
        no_request.request = false;
        refused(inbound("m-5", "alice", row), no_request, "without request");
        let too_long = req_meta("31d", 31 * 24 * 3600);
        refused(inbound("m-6", "alice", row), too_long, "over 30 days");
        let odd_raw = req_meta("1 0m\n", 600);
        refused(inbound("m-7", "alice", row), odd_raw, "short duration");
        refused(
            inbound("m-8", "alice", row),
            reply_meta("not an id!"),
            "reply_to",
        );
        let mut with_to = SendMeta::default();
        with_to.to_attr = Some("bob,carol".to_string());
        refused(inbound("m-9", "alice", row), with_to, "to_attr");

        assert!(port.injected_bodies().is_empty());
        assert!(svc.ledger_snapshot().is_empty());
        // The ordinary id family passes.
        assert!(remote_msg_id_ok("m-7f3k9q2d") && remote_msg_id_ok("r_1-X"));
    }

    // ── Review round 2 (U3a) ──

    #[test]
    fn a_reply_refused_here_does_not_count_as_answered_so_the_later_overdue_names_its_agent() {
        let (svc, port, _remote) = svc_remote();
        // alice is asleep, so mail parks for her; her mailbox is then filled.
        port.set_dormant(&["alice"]);
        let alice = SenderIdentity {
            peer_id: tid("alice"),
            epoch: 0,
            node: None,
        };
        port.set_pcs(vec![pc(1, "home", PcLink::Connected, Some(&["bob"]))]);
        send_as(
            &svc,
            alice,
            "alice",
            "m-req",
            &["bob@home"],
            &req_meta("10m", 600),
        );
        svc.handle_remote_verdict(
            "m-req",
            node(1),
            &[RemoteRowVerdict {
                tag: 0,
                outcome: RemoteOutcome::Delivered {
                    recipient_id: rid("home", "bob"),
                },
            }],
        );
        let carl = ident();
        for i in 0..100 {
            send_as(
                &svc,
                carl,
                "carl",
                &format!("m-fill{i}"),
                &["alice"],
                &SendMeta::default(),
            );
        }

        let mail = InboundMail {
            id: "m-re-bob".to_string(),
            from_agent_id: rid("home", "bob"),
            from_name: "bob".to_string(),
            rows: vec![InboundRow {
                tag: 0,
                to: "alice".to_string(),
            }],
            body: "answer".to_string(),
        };
        let rows = svc
            .handle_remote_mail(node(1), "home", &mail, &reply_meta("m-req"), Instant::now())
            .expect("frame accepted");
        assert_eq!(
            rows[0].outcome,
            InboundOutcome::Refused(FailCode::MailboxFull)
        );

        // bob's PC keeps its contract open and reports overdue: accepted, and it can only be about bob.
        assert!(svc.handle_reply_overdue(
            node(1),
            "m-req",
            Instant::now() + Duration::from_secs(601)
        ));
    }

    #[test]
    fn the_local_sweep_never_moves_a_remote_history_row_even_past_the_park_ttl() {
        let (svc, port, _remote) = svc_remote();
        let alice = with_alice(&port, vec![]);
        port.set_pcs(vec![pc(1, "home", PcLink::Connected, Some(&["bob"]))]);
        send_as(&svc, alice, "alice", "m-1", &["bob"], &SendMeta::default());

        svc.sweep(Instant::now() + Duration::from_secs(48 * 3600));

        assert_eq!(
            svc.ledger_statuses("m-1"),
            vec![DeliveryStatus::RemotePending]
        );
    }

    #[test]
    fn a_reply_from_an_agent_addressed_by_a_braced_id_is_recognised_and_aliases_are_listed_once() {
        let (svc, port, _remote) = svc_remote();
        let alice = with_alice(&port, vec![]);
        port.set_pcs(vec![pc(
            1,
            "home",
            PcLink::Connected,
            Some(&["bob", "dan"]),
        )]);
        let bob = rid("home", "bob");
        let braced = format!("{{{bob}}}");
        let dan = rid("home", "dan");
        send_as(
            &svc,
            alice,
            "alice",
            "m-req",
            &[&braced, "dan@home", &format!("{dan}@home")],
            &req_meta("10m", 600),
        );
        svc.handle_remote_verdict(
            "m-req",
            node(1),
            &[
                RemoteRowVerdict {
                    tag: 1,
                    outcome: RemoteOutcome::Delivered { recipient_id: dan },
                },
                RemoteRowVerdict {
                    tag: 2,
                    outcome: RemoteOutcome::Delivered { recipient_id: dan },
                },
            ],
        );
        // bob's row stays unconfirmed (no recipient id): his reply is matched by the braced id token.
        let mail = InboundMail {
            id: "m-re-bob".to_string(),
            from_agent_id: bob,
            from_name: "bob".to_string(),
            rows: vec![InboundRow {
                tag: 0,
                to: "alice".to_string(),
            }],
            body: "answer".to_string(),
        };
        svc.handle_remote_mail(node(1), "home", &mail, &reply_meta("m-req"), Instant::now())
            .expect("accepted");

        assert!(svc.handle_reply_overdue(
            node(1),
            "m-req",
            Instant::now() + Duration::from_secs(601)
        ));
        let notices = overdue_notices(&port);
        assert_eq!(notices.len(), 1);
        assert!(
            notices[0].contains("no reply from dan@home"),
            "{}",
            notices[0]
        );
        assert!(
            !notices[0].contains(&bob.to_string()),
            "bob replied: {}",
            notices[0]
        );
    }

    #[test]
    fn a_row_the_receiver_would_refuse_fails_alone_and_the_other_rows_to_that_pc_still_go() {
        let (svc, port, remote) = svc_remote();
        let alice = with_alice(&port, vec![]);
        port.set_pcs(vec![pc(1, "home", PcLink::Connected, Some(&["bob"]))]);
        let long = format!("{}@home", "x".repeat(REMOTE_NAME_MAX_BYTES + 1));

        let rows = send_as(
            &svc,
            alice,
            "alice",
            "m-1",
            &["bob@home", &long],
            &SendMeta::default(),
        );

        assert_eq!(rows[0].status, SendStatus::Pending);
        assert_eq!(rows[1].code, Some(FailCode::RecipientNotFound));
        let sent = remote.sent();
        assert_eq!(sent.len(), 1);
        assert_eq!(
            sent[0].rows,
            vec![OutboundRow {
                tag: 0,
                to: "bob".to_string()
            }]
        );
    }

    #[test]
    fn contract_fields_the_receiver_would_refuse_fail_the_remote_rows_here_and_leave_local_rows_alone(
    ) {
        let (svc, port, remote) = svc_remote();
        let (_, carol) = live("carol");
        let alice = with_alice(&port, vec![carol]);
        port.set_pcs(vec![pc(1, "home", PcLink::Connected, Some(&["bob"]))]);
        // A notation the local entrance accepts (leading zeros) but over the remote cap.
        let odd = req_meta("0000000000000000010m", 600);

        let rows = send_as(&svc, alice, "alice", "m-1", &["carol", "bob@home"], &odd);

        assert_eq!(rows[0].status, SendStatus::Delivered);
        assert_eq!(rows[1].code, Some(FailCode::RemoteRefused));
        assert!(rows[1]
            .hint
            .as_deref()
            .unwrap()
            .contains("cannot go to PC 'home'"));
        assert!(remote.sent().is_empty());
    }

    #[test]
    fn an_inbound_mail_reusing_a_remote_request_id_after_its_history_left_the_ring_is_refused() {
        let (svc, port, _remote) = svc_remote();
        let alice = with_alice(&port, vec![]);
        port.set_pcs(vec![pc(1, "home", PcLink::Connected, Some(&["bob"]))]);
        send_as(
            &svc,
            alice,
            "alice",
            "m-req",
            &["bob"],
            &req_meta("10m", 600),
        );
        for i in 0..4100 {
            send_as(
                &svc,
                alice,
                "alice",
                &format!("m-f{i}"),
                &["zed"],
                &SendMeta::default(),
            );
        }
        assert!(svc.ledger_statuses("m-req").is_empty());

        let (code, _) = refusal_code(svc.handle_remote_mail(
            node(1),
            "home",
            &inbound("m-req", "carl", &[(0, "alice")]),
            &SendMeta::default(),
            Instant::now(),
        ));

        assert_eq!(code, FailCode::RemoteRefused);
    }
}
