//! AgentManager — binds backend/transport/output_core/session together to manage the agent lifecycle. Zero tauri imports: upper-layer
//! status notifications are injected through the StatusSink trait.
//!
//! ★Single owner of the roster (ADR-0119)★: "all agents + each one alive/asleep" is built only in `roster()`. The profile registry is
//! **inside** this type and no handle goes out; the outside uses narrow verbs (create/delete/rename/reparent/set-auto-restore/snapshot).
//! Canonical names are unique across the roster (ADR-0120), enforced at creation, new registration (spawn) and rename.
//!
//! Lock order (LLD §10 rule 1): `sessions` is lookup-only — clone the `Arc<AgentSession>`, release the lock, only then take
//! session-internal locks (core/transport). Taking them while holding `sessions` is forbidden (deadlock).

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, RwLock};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use engram_dashboard_base::sync;

use crate::backend;
use crate::failure::AgentFailureKind;
use crate::inputs_pending::InputsPendingTable;
use crate::output_core::{OutputCore, QueuedWiring, TurnWiring};
use crate::preset::PresetRegistry;
use crate::profile::{
    AgentCommand, AgentProfile, ProfileRegistry, RestoreOutcome, RestoreReport, SpawnMode,
    StoreError, StoreStatus,
};
use crate::queued_input::{QueuedInputs, QueuedListing};
use crate::reaper::{self, ReaperCmd, ReaperDeps};
use crate::session::AgentSession;
use crate::session_id_latch::SessionIdLatch;
use crate::session_tracker::SessionTracker;
use crate::transport::{LinkResolution, LinkSink};
use crate::turn::TurnObservations;
use crate::types::{
    AgentId, AgentInfo, AgentStatus, CancelError, CancelOutcome, CommandSpec, ControlChannel,
    InputOrigin, NoopControlChannel, OutputChunk, OutputEvent, OutputSink, PtyError, ReapMsg,
    SinkId, StatusSink, SubscribeReply, TerminalReason, TerminationIntent,
};

const DEFAULT_COLS: u16 = 80;
const DEFAULT_ROWS: u16 = 24;

/// Within this time after a resume spawn, an abnormal exit (code≠0/Failed/Killed) **or a diagnostic stream saying it failed** decides a
/// resume failure (H-1.7 "early exit window"); a successful resume is a TUI and stays up.
///
/// ★Do not enlarge it to wait for death★ (measured: claude emits the diagnostic at +2.2s, exits at +6.4s): activation **blocks** for this
/// window, so every healthy resume would slow down. The decision ground is 「death or evidence」 instead (`EarlyVerdict`).
// ADR-0172
const EARLY_EXIT_WINDOW: Duration = Duration::from_secs(3);

/// A liveness backstop used only when the link **never produces the connection outcome**.
///
/// ★Not a decision window — the decision is the connection itself★: resume success is 「the agent signaled it is ready to take input」
/// (codex: handshake completion, `Link::Up`) and a refusal is answered at once, so [`AgentManager::early_activation_verdict`] returns the
/// moment the outcome exists. Do not bring back the old 15-second connection decision window.
/// ★The one reason this value remains★: the link's handshake cap ([`crate::backend::codex`]) bounds waiting for **responses**, not
/// **writes** — a peer that does not read our stdin hangs `write_all` and leaves the link in `Connecting`, so `activate_profile` would block
/// **forever** (one daemon dispatch worker). So this is 「giving up」, not 「deciding」: reaching it is a failure and **triggers the teardown
/// below** (that kill also releases the hung write).
/// ★It must be larger than the link's handshake cap★ — else a reasonless give-up covers a refusal with a reason (measured by the test rig).
pub(crate) const LINK_RESOLUTION_BACKSTOP: Duration = Duration::from_secs(15);
/// Spawn interval between agents during restore (stagger to prevent concurrent bursts).
const RESTORE_STAGGER: Duration = Duration::from_millis(200);

/// The outcome of one spawn request. ★The caller **must be able to tell** "launched" from "there was nothing to do"★, so registration,
/// epoch and recording cleanup is not done on a session it did not create.
///
/// ★A duplicate request is an outcome, not an error — do not revert★: if that entry is already up or launching the request is **moot**. As
/// an `Err` every consumer had to know "this error is not a real failure" and one was missed each time; the frontend's in-flight guard is the
/// same shape. ★moot changes nothing★ — no failure to record, no ground to clear (`AgentManager::note_spawn_result`).
// ADR-0172
#[derive(Debug, Clone)]
pub enum SpawnOutcome {
    /// This call launched an incarnation.
    Started(AgentInfo),
    /// **This request had nothing to do** — the entry was already up or launching. `Some` = the live session seen at that moment · `None` = the
    /// winner had not yet put it on the roster. Either way **this call created nothing**.
    Moot(Option<AgentInfo>),
}

impl SpawnOutcome {
    /// Whether this call actually launched — the decision of the **eligibility** to do cleanup.
    pub fn started(&self) -> bool {
        matches!(self, SpawnOutcome::Started(_))
    }

    /// "That agent now" information regardless of the outcome. Moot is a session someone else launched, so it can be `None`.
    pub fn into_info(self) -> Option<AgentInfo> {
        match self {
            SpawnOutcome::Started(info) => Some(info),
            SpawnOutcome::Moot(info) => info,
        }
    }

    /// Only the incarnation **this call launched**. `None` if moot — for callers that want to do cleanup only on "what I made".
    pub fn into_started(self) -> Option<AgentInfo> {
        match self {
            SpawnOutcome::Started(info) => Some(info),
            SpawnOutcome::Moot(_) => None,
        }
    }
}

/// Hard cap on the output tail the failure classification looks into (`OutputCore::terminal_tail` keeps it exactly); the exit text is in the last few lines. Classification input only — not
/// shown on screen or logs.
// ADR-0172
const FAILURE_TAIL_BYTES: usize = 4096;

/// The early decision outcome of a resume attempt — split by 「what evidence stood」, not 「did it die」.
///
/// ★Why `Diagnosed` exists (measured — do not revert)★: real claude, for `--resume <empty sid>`, **emits the diagnostic at +2.2s and dies
/// at +6.4s**; waiting only for death passes the early-exit window (3s) and **decides the failure a success, clearing even the earlier
/// record**. With the evidence in hand it does not wait for the corpse. Enlarging the window is not the answer (activation **blocks** for it).
// ADR-0172
#[derive(Debug)]
enum EarlyVerdict {
    /// It entered a terminal state within the window. `evidence` = the console tail + diagnostic text captured at that moment (best-effort,
    /// an empty value is normal).
    Terminal {
        status: AgentStatus,
        evidence: String,
    },
    /// **Still alive**, but the diagnostic stream already said a known failure — settled without waiting for death.
    Diagnosed(AgentFailureKind),
    /// It passed the window and no evidence stood = activation holds. ★**Weak** success evidence★ — only 「did not die within N seconds」;
    /// links without a connection axis (claude · shell · stdio) have only this.
    Alive,
    /// The agent **signaled it is ready to take input** = activation holds. Split from `Alive` because ① this is a positive signal (codex
    /// handshake completion) with nothing to wait for, and ② this optimistic success **does not clear** the last-failure record (the clearing
    /// rule is in `AgentManager::resume_no_fallback`).
    Ready,
    /// The link **could not set up the connection** — the process may be alive. Split from `Terminal` (died): same failure disposition but a
    /// different reason sentence and classification input, and **no corpse** — console and diagnostic tails are empty, the only evidence is `reason`.
    LinkFailed { reason: String },
}

/// One roster entry = **one agent** (ADR-0119 decision 1); it removes consumers each merging "the live list" and "the profile list".
/// ★A corpse left in the map in a terminal state is not an entry★: with a profile it is asleep, ad-hoc it is not listed.
#[derive(Debug, Clone)]
pub struct RosterEntry {
    pub id: AgentId,
    pub canonical_name: String,
    /// `Some` = alive (a Running|Exiting session attached) · `None` = asleep (profile only).
    pub live: Option<AgentInfo>,
    /// Working folder: the session cwd for live entries (canonicalized at spawn), the stored raw cwd for asleep ones — **the same material the name derivation reads** (`roster()`'s "live/asleep sources
    /// differ"). Empty if neither a session nor a profile exists.
    pub cwd: String,
    /// Tree parent. The stored hierarchy is the only source, so live entries also read it from the profile — `None` = top level.
    pub parent: Option<AgentId>,
}

/// **Today the only path carrying an override is rename** (`RenameProfile`); the create/spawn side has no such field, so calls from
/// `try_create_agent` · `register_for_spawn` are a public-API defense line.
///
/// ★Stored name · displayed name · mail address must be **the same string**★: uniqueness (ADR-0120) is a string comparison, so `bob` and
/// `" bob "` would both pass yet look identical in the tree, and the messaging entry trims the recipient, making `" bob "` visible but
/// unaddressable. ★Do not delete that entry trim (`messaging` recipient matching)★: the CLI splits `--to a,b` by commas without removing
/// spaces (`"alice, bob"` → `["alice", " bob"]`) and the trim rescues later recipients.
/// ★Applied **before** the uniqueness decision★ (decision and storage see the same value); ★inner whitespace is part of the name★ (only
/// both ends are trimmed). ★Residue is a different kind★: `str::trim` leaves zero-width characters (U+200B) — stored == displayed == address
/// still holds, only visual confusion remains, and NFKC/confusable folding is a policy decision, not an improvised filter here.
/// ★Already stored names are not fixed★ (no migration).
fn normalize_display_name(display_name: Option<String>) -> Option<String> {
    let trimmed = display_name?.trim().to_string();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

/// The form of `name` in the `base` family. ★**Never mix** `Exact` and `Suffixed(0)`★: a literal `bob(0)` would look like it occupied the unsuffixed `bob`, so a free `bob` would
/// still get `bob(1)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NameKind {
    /// `name == base` — no suffix at all (= that name itself is in use).
    Exact,
    /// `name == base(n)` — a literal `base(0)` also comes here.
    Suffixed(u32),
}

/// The parser of the ADR-0115 notation (`name(N)`) — it must be **the exact inverse** of the issuer (`format!("{base}({n})")` of `decide_name_with_roster`).
/// If the notation changes, change both.
fn classify_name(base: &str, name: &str) -> Option<NameKind> {
    if name == base {
        return Some(NameKind::Exact);
    }
    let rest = name.strip_prefix(base)?;
    let digits = rest.strip_prefix('(')?.strip_suffix(')')?;
    // Empty parentheses, signs and whitespace are not our notation (`base()`·`base(-1)` are unrelated).
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    // Leading zeros (`base(01)`) are not our notation (the issuer never writes it); leaving them out of the family creates no duplicate.
    if digits.len() > 1 && digits.starts_with('0') {
        return None;
    }
    digits.parse::<u32>().ok().map(NameKind::Suffixed)
}

/// ★The total roster cap = **a runaway backstop**, not a product limit★ (ADR-0132).
///
/// The control plane is open to all agents (ADR-0132 decision 5); a loop calling "create agent" would grow the roster and `agents.json`
/// **without bound** — only that is cut.
/// ★Why the core, not the entry★: an entry cap needs a copy per entry and missed the desktop (`CreateProfile`) and ad-hoc spawn. Where the
/// registration is **committed**, inside the name allocation gate, it holds for every entry and is **atomic** (no window of several requests
/// all seeing 511). ★A count check, not rate limiting★ — no timers or per-caller state.
/// ★The number is not to be tuned★: it only has to sit where legitimate use (tens of agents) cannot reach. ★Do not argue "resources run
/// out first"★ — that fails for **registering asleep agents**, which launch nothing and have no physical brake.
/// ★The existing roster is not a hostage★: only **new registrations** are checked — restore/respawn of a roster over the cap run as is
/// (the existing-id branch of `register_for_spawn` skips it).
// ADR-0132
pub const MAX_ROSTER_SIZE: usize = 512;

/// The total check before a new registration — call **only while holding the name allocation gate** (atomic). ★Not atomic with deletion
/// (intended)★: deletion only decreases the count, so the worst case is a refusal just after a slot frees; the next call passes.
fn check_roster_capacity(roster: &[RosterEntry]) -> Result<(), PtyError> {
    if roster.len() >= MAX_ROSTER_SIZE {
        return Err(PtyError::RosterFull {
            current: roster.len(),
            limit: MAX_ROSTER_SIZE,
        });
    }
    Ok(())
}

/// The single error text for suffix space exhaustion — refuses rather than issue a duplicate name (name = address, so a duplicate
/// misdelivers mail, ADR-0116).
///
/// ★Unreachable in reality★: `pick_suffix` returns `None` only when all of 1..=u32::MAX are occupied; saturation is absorbed earlier by
/// "the lowest free number". This path and `RenameOutcome::Exhausted` exist to keep the decision table total — not a living policy.
/// ★No dedicated error variant★: the caller only needs "this verb could not be performed now" (same as `write_stdin_observed_if_epoch`).
fn name_space_exhausted(base: &str) -> PtyError {
    PtyError::Unsupported(format!(
        "name suffix space exhausted for base {base:?} — refusing rather than minting a duplicate name"
    ))
}

/// One outcome of the name decision table (ADR-0120 uniqueness · ADR-0123 numbering rule). Computed while holding the allocation gate.
#[derive(Debug, Clone, PartialEq, Eq)]
enum NameDecision {
    /// Nobody has the requested name → settled as is without a suffix.
    Free,
    /// The requested name belongs to someone else, and it already holds the unique name of that family → nothing changes.
    KeepCurrent,
    /// The requested name belongs to someone else → settled with a suffixed name.
    Suffixed(String),
    /// Every number of the family (1..=u32::MAX) is occupied → cannot issue.
    Exhausted,
}

/// Rename outcome — the caller must **tell apart** failure reasons; a bool would merge "no such agent" and "cannot issue a name" and the wire would state a false cause.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RenameOutcome {
    /// The name was settled (the requested name as is or a suffixed form). Value = the settled canonical name.
    Renamed(String),
    /// It already holds the unique name of that family, so nothing changed. Value = the name kept.
    Unchanged(String),
    /// An agent of that id is not in the roster.
    NotFound,
    /// Suffix space exhausted — the rename was refused because no name could be issued (state unchanged).
    Exhausted,
}

/// `used` = the numbers the family currently occupies. Rule (ADR-0115): **observed max + 1**, so live numbers do not overlap; no high-water
/// state, so an emptied family restarts from 1 (ADR-0123).
/// ★Saturation escape only★: if the max is `u32::MAX` it goes to **the lowest free number** (else one `name(4294967295)` would block that
/// family's free numbers forever). `None` = all of 1..=MAX occupied (unreachable; keeps the function total).
// ADR-0123
fn pick_suffix(used: &std::collections::BTreeSet<u32>) -> Option<u32> {
    if let Some(&max) = used.iter().next_back() {
        if let Some(next) = max.checked_add(1) {
            return Some(next);
        }
    } else {
        return Some(1);
    }
    // Saturation — walks in ascending order to find the first hole (`used` is bounded by the roster size, so it ends quickly).
    let mut candidate: u32 = 1;
    for &n in used.iter() {
        match n.cmp(&candidate) {
            std::cmp::Ordering::Less => continue,
            std::cmp::Ordering::Equal => candidate = candidate.checked_add(1)?,
            std::cmp::Ordering::Greater => break,
        }
    }
    Some(candidate)
}

/// One connection outcome the link delivered — ★carries **together** which incarnation it belongs to★: sent separately, the outcome of a
/// collected incarnation would be applied to the successor just launched (that cleanup **kills a live session**). Consumers compare the
/// marker **only for match/mismatch** — "newer" is never derived from ordering (ADR-0163/0164).
#[derive(Debug, Clone)]
pub(crate) struct LinkVerdict {
    pub(crate) epoch: u32,
    pub(crate) resolution: LinkResolution,
}

/// The set a caller waiting for the connection outcome holds — **the mailbox and the spawn reservation are one body**.
///
/// ★Binding the two fields is the point★: on a link declaring a connection there is a one-handshake gap between 「on the roster」 and 「input
/// can be sent」; a second activation looking only at the roster would answer 「it is up」 and then the first request's failure would collect
/// that incarnation under it. Holding the reservation until the outcome makes the answer 「already launching」.
/// ★Dropping this value releases the reservation at once★ ([`AgentManager::spawn_agent`] does, as it does not wait). ★Links without a
/// connection axis never get it★ (`Option` is `None`).
struct LinkWatch {
    rx: Receiver<LinkVerdict>,
    /// Not released until the outcome happens — not reading it is normal (a lifetime is all this field is).
    _reservation: SpawnReservation,
}

/// Builds the delivery port handed to the link — the incarnation marker is stamped **here** and passed into the channel. ★The link does not
/// know the incarnation★ (as [`session_id_sink`]; ADR-0004 keeps that out of the backend). ★Swallowing a send failure is intended★ — the
/// receiver already left (decision finished, or a Fresh spawn nobody waits on).
fn link_verdict_sink(tx: Sender<LinkVerdict>, incarnation: u32) -> LinkSink {
    Arc::new(move |resolution: LinkResolution| {
        let _ = tx.send(LinkVerdict {
            epoch: incarnation,
            resolution,
        });
    })
}

/// The profile vanished during spawn — **the spawn is aborted**. `at` = the stage noticed: `spawn_agent` decides this in **three** places
/// (incarnation marker · Fresh's session id clearing · Resume's reading of the handle), told apart in logs by it. ★All three share one
/// disposition★ — a swallowed `None` would launch with a deleted profile or silently turn a resume into a new conversation.
fn profile_vanished_mid_spawn(id: AgentId, at: &str) -> PtyError {
    PtyError::SpawnFailed(format!(
        "profile {id} vanished mid-spawn at [{at}] (concurrent delete) — spawn aborted"
    ))
}

/// The kind written into 「the last failure」 when the launch ended with `Err` before producing a process. ★A save `Err` is 「other」, not
/// `SpawnFailed`★ — the store refused the pre-launch change, no process was attempted, and `SpawnFailed` would make the screen and the LLM
/// suspect the execution environment.
// ADR-0291 R17
fn spawn_error_kind(e: &PtyError) -> AgentFailureKind {
    match e {
        PtyError::Store(_) => AgentFailureKind::Other,
        _ => AgentFailureKind::SpawnFailed,
    }
}

/// Which registry change entry to change through — decided by the caller; the verbs below only forward to that entry's registry verb, and
/// `Internal` emits no `Err` (a save failure still commits + dirty).
/// ★The launch path carries this one value to the shared function — no per-entry launch copies★: boot restore and manual wake-up using the
/// same verb closes 「the decision splits per entry and one falls behind」 (ADR-0082 · ADR-0201 · ADR-0202).
// ADR-0291 R17 (E3)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Entry {
    /// A change with a caller (bus · WS · launches those two called) — if the save is `Err`, memory is left as is and it errors.
    Caller,
    /// A change without a caller (boot restore) — applied to memory + dirty on any save `Err`. There is no request to revert, and
    /// it keeps one momentary lock from blocking restore.
    Internal,
}

impl Entry {
    fn update_preserving_hierarchy(
        self,
        profiles: &ProfileRegistry,
        profile: AgentProfile,
    ) -> Result<bool, StoreError> {
        match self {
            Entry::Caller => profiles.try_update_preserving_hierarchy(profile),
            Entry::Internal => Ok(profiles.update_preserving_hierarchy(profile)),
        }
    }

    fn upsert_preserving_hierarchy(
        self,
        profiles: &ProfileRegistry,
        profile: AgentProfile,
    ) -> Result<(), StoreError> {
        match self {
            Entry::Caller => profiles.try_upsert_preserving_hierarchy(profile),
            Entry::Internal => {
                profiles.upsert_preserving_hierarchy(profile);
                Ok(())
            }
        }
    }

    fn release_session_id(
        self,
        profiles: &ProfileRegistry,
        id: AgentId,
    ) -> Result<Option<bool>, StoreError> {
        match self {
            Entry::Caller => profiles.try_release_session_id(id),
            Entry::Internal => Ok(profiles.release_session_id(id)),
        }
    }
}

/// Whether a Fresh spawn **must start by clearing the stored session id**. Both required: **1. It is Fresh** (clearing on Resume loses the
/// resume handle — ADR-0083); **2. ★this incarnation has a path to fill that field★** (else a hand-planted value vanishes with nobody
/// refilling it).
///
/// ★The enumeration of 2 is **managed by hand**★: first-submission draw ([`backend::assigns_session_id`]) · link connection
/// ([`backend::declares_link`] — the handshake response feeds the latch) · control plane ([`backend::supports_control_channel`]). A fourth
/// path must be added here — the compiler cannot catch the omission. Today: claude · gemini · codex true, shell false.
/// ★codex's terminal mode is true **by chance**★ — its field is on for MCP mail (ADR-0214) while the real fill path is the child's writer
/// lock (ADR-0218); turning that field off for mail reasons would **silently** flip this to false. App-server mode stands on `declares_link`.
/// ★Without it (do not bring back)★: the previous incarnation's value stays in the field `resume_session_id` reads, so **the resume rewinds
/// to the previous conversation**; issuing backends write a new value only at the first submission, so an incarnation ending earlier leaves the old.
// ADR-0185
// ADR-0083
// ADR-0208
// ADR-0216
// ADR-0217
// ADR-0218
// ADR-0226
pub(crate) fn fresh_spawn_release_session_id(command: &AgentCommand, mode: SpawnMode) -> bool {
    matches!(mode, SpawnMode::Fresh)
        && (backend::assigns_session_id(command)
            || backend::supports_control_channel(command)
            || backend::declares_link(command))
}

/// The **resume handle** this spawn hands to the backend — ★`None` on Fresh even if a value is stored★. This line makes 「`resume_session_id`
/// exists ⟺ resuming incarnation」 hold for two consumers that look at different arguments (argv assembly looks at `(mode, handle)`, the
/// reclamation decision at the handle only); only this place could produce "Fresh with a handle". Not passing the mode to the backend is the
/// `AgentBackend::open_spawn` contract, and this function is what makes that inexpressibility real.
// ADR-0185
// ADR-0218
fn resume_handle_for(mode: SpawnMode, stored: Option<uuid::Uuid>) -> Option<uuid::Uuid> {
    match mode {
        SpawnMode::Resume => stored,
        SpawnMode::Fresh => None,
    }
}

/// **Our side's session id** handed to the backend and the first-submission latch — `Some` only for issuing-axis backends (Fresh = newly
/// drawn · Resume = the stored value).
/// ★Decided by the issuing axis (`assigns_session_id`) — not the resume axis (ADR-0185)★: handing a value to a backend that issues its own
/// id (codex) would put it in the latch, the backend would drop it from argv and the real id would be rejected as the latch's second offer —
/// **every later resume silently fails** (ADR-0226).
// ADR-0185
// ADR-0226
fn session_id_to_hand_over(
    command: &AgentCommand,
    mode: SpawnMode,
    stored_handle: Option<uuid::Uuid>,
) -> Option<uuid::Uuid> {
    if !backend::assigns_session_id(command) {
        return None;
    }
    match mode {
        SpawnMode::Resume => stored_handle,
        SpawnMode::Fresh => Some(ProfileRegistry::mint_session_id()),
    }
}

/// Builds the **commit port** writing this incarnation's session id into the roster — called by the first-submission latch
/// ([`SessionIdLatch`]) at most once per incarnation.
///
/// ★Compare-and-swap ([`ProfileRegistry::commit_session_id`])★: writes only if the field is `expected` (what this incarnation saw at start —
/// Fresh `None` after clearing · Resume the stored value) or already that value; ★not the overwrite `observe_session_id`★ (a claude file
/// watcher change before the first submission would be rewound). On success `expected` moves (defensive; production calls once).
/// The `String` → `Uuid` decoding is here (profile schema knowledge, not `backend/` — ADR-0004). ★Do not assume v4★: codex gives UUIDv7
/// (`01a0a08f-…`) while claude's is a drawn v4, in the same `backend_session_id` field; ★no version validation★ (`Uuid::parse_str` accepts both).
/// ★Unreadable → logged and **dropped**, never a panic★: this call runs outside the containment marker, so a panic would let the hook end the
/// daemon over a format change (ADR-0292); a failed record just makes the resume fall to Fresh, honestly reported. ★The value is not logged★.
/// ★Pins the incarnation marker (ADR-0007/0163)★: bound to one spawn, a record arriving after **a newer incarnation stood** is refused; a
/// session that **only ended** keeps the marker, so this is no late-record block (that window: the link's `record_session_id` doc).
/// ★Three log outcomes★ (`docs/reference/logging-conventions.md`): recorded (info) · refused (debug) · decode failure (warn); not-received
/// is the link's handshake log, held-for-lack-of-submission the latch's.
/// ★`true` ≠ persisted★ — only the in-memory roster changed; a failed disk save keeps it (the store logs). Lock order `latch → expected →
/// profiles → store-state`, one-way; only this port takes `expected`.
// ADR-0007
// ADR-0185
// ADR-0226
fn session_id_sink(
    profiles: Arc<ProfileRegistry>,
    id: AgentId,
    incarnation: u32,
    expected: Option<uuid::Uuid>,
) -> backend::SessionIdSink {
    let expected = Mutex::new(expected);
    Arc::new(move |raw: &str| {
        // ★Do not put the record call in a match guard★ — a side-effecting guard runs 0 or 2 times if an arm slips in front; keep decision and branching apart.
        let sid = match uuid::Uuid::parse_str(raw) {
            Ok(sid) => sid,
            Err(e) => {
                tracing::warn!(
                    agent = %id,
                    epoch = incarnation,
                    "could not read the session id the backend gave as a uuid, dropping it: {e}"
                );
                return;
            }
        };
        let mut seen = sync::lock(&expected);
        let written = profiles.commit_session_id(id, incarnation, *seen, sid);
        if written {
            *seen = Some(sid);
        }
        drop(seen);
        if written {
            tracing::info!(
                agent = %id,
                epoch = incarnation,
                "reflected the session id the backend gave into the profile roster"
            );
        } else {
            // ★The reason cannot be told apart — one `bool`★: marker mismatch · another writer changed it first · profile absent · already the same value all give `false`.
            tracing::debug!(
                agent = %id,
                epoch = incarnation,
                "the session id the backend gave was not reflected into the roster — one of incarnation marker mismatch · another writer · profile absent · same value"
            );
        }
    })
}

/// The incarnation facts spawn carries into the session — a **required** argument of [`AgentManager::spawn_session`].
/// ★Never `Option`, no default★: a latch not carried into the session counts no submission, so no incarnation persists its session id and
/// **every resume silently disappears**; a wiring structure test measures the carrying. `continues_conversation` = this spawn carried a
/// resume handle (not that the resume succeeded).
// ADR-0226
struct SpawnIncarnation {
    continues_conversation: bool,
    latch: Arc<SessionIdLatch>,
}

pub struct AgentManager {
    sessions: Arc<RwLock<HashMap<AgentId, Arc<AgentSession>>>>,
    status_sink: Arc<dyn StatusSink>,
    // Single owner of profiles.
    profiles: Arc<ProfileRegistry>,
    // ADR-0061: single owner of presets (cwd bookmarks), held by the daemon like profiles (ADR-0029). The reaper does not read presets (unrelated to agent lifetimes), so it is only a manager field.
    presets: Arc<PresetRegistry>,
    tracker: Arc<SessionTracker>,

    // ── ADR-0019 reaper ──────────────────────────────────────
    /// Daemon/app shutdown global flag. shutdown_all sets it **before** each kill, so the finish hook of a session ending in between snapshots true and the reaper skips the disposition (boot
    /// restore kept).
    shutting_down: Arc<AtomicBool>,
    /// The channel through which the session/pump finish hook sends ReapMsg (consumed by the single supervisor).
    reaper_tx: Sender<ReaperCmd>,
    /// Reaper thread handle. On Drop, join (wait after sending Stop) — prevents test leaks.
    reaper_handle: Option<JoinHandle<()>>,

    /// ADR-0086 control channel provisioning seam: provision (token + mcp-config) on spawn, the reaper revokes (discard + delete file) at terminal. Only the daemon injects the real one
    /// (`DaemonControlChannel`); the default is `NoopControlChannel` (headless tests · shell-only paths). An `Arc`, shared with the reaper.
    control: Arc<dyn ControlChannel>,

    /// ADR-0086 provision race guard (FIX 6) — the AgentIds being spawned (reservations). Without it, another connection spawning the same id in
    /// the TOCTOU window between the `contains_key` guard and `sessions.insert` would provision the same (AgentId,epoch) path and one reaper
    /// could delete the other's live session. Entry reserves atomically (immediate `Err` if present); `SpawnReservation` releases on drop.
    /// ★A leaf lock separate from the sessions map★: no sessions/status lock is taken while holding it (ADR-0006).
    spawning: Arc<Mutex<HashSet<AgentId>>>,

    /// The name allocation gate — a stateless serializing lock guarding one invariant: **between observing the roster and committing a name
    /// from it, no other allocation slips in** (else two creations both take the same free name). Derivation · observation · commit all happen inside.
    ///
    /// ★Lock order = name_allocation → sessions/profiles, one-way★ — `try_create_agent` · `try_rename_agent` · `register_for_spawn` touch the
    /// roster only **after** taking it; no reverse order exists (ADR-0006).
    /// ★The critical section is not cheap★: the `roster()` observation pays **one `dunce::canonicalize` per asleep agent without an override** and
    /// the commit writes the whole `agents.json` (ADR-0071), so creation, rename and new-registration spawn serialize globally — a dead network
    /// share stalls all three. ★Mail delivery must stay unblocked★: `DeliveryPort` never takes this lock (`roster()` works without it).
    // ADR-0006
    name_allocation: Arc<Mutex<()>>,

    /// The turn observation table (the ADR-0113 fact layer) — shared by every `OutputCore` this manager launched. Written by `OutputCore::emit`
    /// (★two calling threads★: the output pump and the injection thread emitting the input echo — turn.rs header), cleaned by
    /// `OutputCore::finish` (★not the reaper★: the late-emit race must close at the finalize flag — ADR-0127 decision 5), read by consumers (the
    /// mail idle gate). ★A leaf★: nothing takes sessions/profiles while holding it (ADR-0006).
    // ADR-0113
    // ADR-0127
    turns: Arc<TurnObservations>,

    /// The waiting-list table (「this incarnation's user waiting list is not empty」) — a leaf like the turn observation table; read by the mail busy adapter. ★Unrelated to the sessions lock★.
    // ADR-0231
    inputs_pending: Arc<InputsPendingTable>,
}

/// The RAII guard taking the AgentId reservation during spawn and releasing it on drop (ADR-0086 FIX 6), so no early return of `spawn_agent`
/// leaks it. `reserve` returns `None` for an already-reserved id.
///
/// ★Two other safeties rest on this guard's **lifetime** — shortening it opens both together★: on a link declaring a connection, [`LinkWatch`]
/// holds it **until the connection outcome**, so no new incarnation can stand with the same id. Hence ① the mailbox's incarnation-marker
/// comparison never fires (the `as_verdict` mismatch branch of [`AgentManager::link_activation_verdict`] is dead in production) and
/// ② the supervisor may pick the session by id alone (`get_session(id)` without a marker comparison). A change narrowing this lifetime
/// **must re-examine both**; the dependency is written here, at the definition.
struct SpawnReservation {
    spawning: Arc<Mutex<HashSet<AgentId>>>,
    id: AgentId,
}

impl SpawnReservation {
    fn reserve(spawning: Arc<Mutex<HashSet<AgentId>>>, id: AgentId) -> Option<Self> {
        {
            let mut set = spawning.lock().expect("spawning set poisoned");
            if !set.insert(id) {
                return None;
            }
        }
        Some(Self { spawning, id })
    }
}

impl Drop for SpawnReservation {
    fn drop(&mut self) {
        let _ = self
            .spawning
            .lock()
            .expect("spawning set poisoned")
            .remove(&self.id);
    }
}

/// The RAII guard for a failure after provision succeeds and **before** session registration (exe/PTY error · `?`): the issued token + config
/// file would leak forever (no session, so the reaper never revokes) (ADR-0086 FIX 3). Armed only when provision returned a real endpoint,
/// disarmed by `disarm()` once the session is registered; on drop while armed it calls revoke. ★No lock held (ADR-0006)★: it drops only
/// where the sessions/status locks are not held.
struct ProvisionGuard {
    control: Arc<dyn ControlChannel>,
    id: AgentId,
    epoch: u32,
    /// The revoke of a session registered after disarm is the job of kill_agent/reaper (prevents double revoke).
    armed: bool,
}

impl ProvisionGuard {
    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for ProvisionGuard {
    fn drop(&mut self) {
        if self.armed {
            tracing::warn!(
                agent = %self.id,
                epoch = self.epoch,
                "ADR-0086: spawn failed (before session registration) — reclaiming (revoking) the issued control channel token/config"
            );
            self.control.revoke(self.id, self.epoch);
        }
    }
}

impl AgentManager {
    /// Default constructor — no control channel (NoopControlChannel). For headless tests and paths not using a control channel.
    pub fn new(
        status_sink: Arc<dyn StatusSink>,
        profiles: Arc<ProfileRegistry>,
        presets: Arc<PresetRegistry>,
        tracker: Arc<SessionTracker>,
    ) -> Self {
        Self::new_with_control(
            status_sink,
            profiles,
            presets,
            tracker,
            Arc::new(NoopControlChannel),
        )
    }

    /// The control-channel-injected form (ADR-0086) — the daemon plugs in `DaemonControlChannel`; the reaper shares the same Arc and revokes at terminal (spawn=provision / terminal=revoke symmetry).
    pub fn new_with_control(
        status_sink: Arc<dyn StatusSink>,
        profiles: Arc<ProfileRegistry>,
        presets: Arc<PresetRegistry>,
        tracker: Arc<SessionTracker>,
        control: Arc<dyn ControlChannel>,
    ) -> Self {
        let sessions = Arc::new(RwLock::new(HashMap::new()));
        let turns = Arc::new(TurnObservations::new());
        let inputs_pending = Arc::new(InputsPendingTable::new());

        let deps = ReaperDeps {
            sessions: sessions.clone(),
            profiles: profiles.clone(),
            status_sink: status_sink.clone(),
            control: control.clone(),
        };
        let (reaper_tx, reaper_handle) = reaper::spawn_reaper(deps);

        Self {
            sessions,
            status_sink,
            profiles,
            presets,
            tracker,
            shutting_down: Arc::new(AtomicBool::new(false)),
            reaper_tx,
            reaper_handle: Some(reaper_handle),
            control,
            spawning: Arc::new(Mutex::new(HashSet::new())),
            name_allocation: Arc::new(Mutex::new(())),
            turns,
            inputs_pending,
        }
    }

    pub fn turns(&self) -> Arc<TurnObservations> {
        self.turns.clone()
    }

    pub fn inputs_pending(&self) -> Arc<InputsPendingTable> {
        self.inputs_pending.clone()
    }

    pub fn presets(&self) -> &Arc<PresetRegistry> {
        &self.presets
    }

    /// The current state of the agent roster store (agents.json). The preset side is seen through [`AgentManager::presets`].
    // ADR-0291 R17 (D6)
    pub fn agents_store_status(&self) -> StoreStatus {
        self.profiles.store_status()
    }

    // ★No mail-eligibility query verb here (do not revert)★: consumers read `AgentInfo::reads_messages` of the list snapshot; an ask-back verb by id
    //   would revive TOCTOU and per-session locks. The source is the value the session received from the backend at spawn
    //   (`AgentSession::reads_messages`), **not the profile** — `DeleteProfile` does not kill a live session, so the profile axis fails open
    //   (that hole put a shell back on the list).

    // ── Roster — the single entry ────────────────────────────────────────────

    /// **The only place** building "all agents + each one alive/asleep" (ADR-0119 decision 2).
    ///
    /// ★One snapshot★: the roster and the live session id set come from **the same single `list_agents()` sheet** — two takes would let a
    /// spawn/exit/deletion slip in and two recipients of one send see different worlds (ADR-0111 decision 2).
    /// ★Asleep = the difference on **the id axis**★ (a profile's session launches with its profile id); subtracting by name would let one live
    /// same-name entry hide another asleep profile (sealed by the `messaging_host` tests). ★Asleep names are not folded★ (same-name decision axis).
    /// ★Live and asleep name sources **differ** (do not merge)★: live = `AgentInfo.name` from `list_agents()` (`resolve_canonical_name`, based on
    /// the canonicalized `session.cwd`), asleep = `AgentProfile::canonical_name_when_live()` (raw `profile.cwd` + normalization) — merging
    /// changes the parking key behavior.
    /// ★fs access only for asleep profiles without an override★: do not defeat the `canonical_name_when_live()` shortcut (0 syscalls when
    /// `display_name` is non-blank) — this is the send critical path and a dead network share blocks one canonicalize for tens of seconds.
    /// ★Lock discipline (ADR-0006 · ADR-0071)★: `sessions` (RwLock) and the profile map (Mutex) are independent — `list_agents()` clones the Arcs
    /// and releases, then `profiles.list()` takes the profile lock, **sequential, not nested** (one lock would let profile saves block the
    /// envelope injection path — ADR-0119). ★Not atomic, not to be made atomic★: the residue between the two lookups was accepted by ADR-0116
    /// (bounded by a 24h TTL).
    // ADR-0119
    pub fn roster(&self) -> Vec<RosterEntry> {
        // ★The profile snapshot is **one sheet**; live entries' names and hierarchy come from it too (load-bearing)★ — taken separately, a rename +
        //   hierarchy move committed in between would put a combination that never existed (old name + new parent) in one row. The sessions↔profiles
        //   non-atomicity remains (ADR-0116); what is closed is profile↔profile. Side effect: no per-session `profiles.get` clone on the mail-send
        //   critical path (ADR-0119 · messaging_host `addressing_sources`).
        let sessions: Vec<Arc<AgentSession>> = {
            let guard = self.sessions.read().expect("sessions poisoned");
            guard.values().cloned().collect()
        };
        let mut profiles: HashMap<AgentId, AgentProfile> = self
            .profiles
            .list()
            .into_iter()
            .map(|p| (p.id, p))
            .collect();
        let mut live_ids: HashSet<AgentId> = HashSet::with_capacity(sessions.len());
        let mut entries: Vec<RosterEntry> = Vec::with_capacity(sessions.len() + profiles.len());
        for session in &sessions {
            let info = self.agent_info_with(
                session,
                profiles
                    .get(&session.id)
                    .and_then(|p| p.display_name.as_deref()),
            );
            // A corpse (terminal) stays in the map until reaped — split by state, not existence (the ADR-0116 predicate). ★The profile is left in the map★: the corpse's profile must come up as an **asleep**
            //   entry in the loop below.
            if !info.status.is_live() {
                continue;
            }
            live_ids.insert(info.id);
            entries.push(RosterEntry {
                id: info.id,
                canonical_name: info.name.clone(),
                cwd: info.cwd.clone(),
                parent: profiles.get(&info.id).and_then(|p| p.parent_id),
                live: Some(info),
            });
        }
        for (_, p) in profiles.drain() {
            if live_ids.contains(&p.id) {
                continue;
            }
            entries.push(RosterEntry {
                id: p.id,
                canonical_name: p.canonical_name_when_live(),
                // It is an owned value, so it is **moved** — if valid UTF-8 the buffer is reused, otherwise a lossy copy only then.
                cwd: p
                    .cwd
                    .into_os_string()
                    .into_string()
                    .unwrap_or_else(|os| os.to_string_lossy().to_string()),
                parent: p.parent_id,
                live: None,
            });
        }
        entries
    }

    pub fn agent_snapshot(&self, id: AgentId) -> Option<AgentProfile> {
        self.profiles.get(id)
    }

    /// The wire `ProfileList` still carries the profile type as is, so the data crosses the boundary, but the registry
    /// handle does not.
    pub fn agent_snapshots(&self) -> Vec<AgentProfile> {
        self.profiles.list()
    }

    pub fn agent_backend_session_id(&self, id: AgentId) -> Option<uuid::Uuid> {
        self.profiles.get(id).and_then(|p| p.backend_session_id)
    }

    /// New agent registration (tree "create"). Enforces global roster name uniqueness before registering (ADR-0120).
    ///
    /// Returns **the profile this call registered** (assigned name reflected) — the caller's response must carry that name, not the
    /// pre-registration snapshot. ★Not the stored value read back★: `ProfileRegistry::mutate` runs `normalize_hierarchy` before saving, but a new
    /// profile has `parent_id == None`, so it does not diverge (re-query if a read-back is ever needed).
    /// ★The suffix is pinned as `display_name`★: without an override the name derives from the cwd basename, so two on one folder collide
    /// without any rename (ADR-0120 §Impact); the override is the only place that can resolve it.
    /// ★A change with a caller★ — `Err` = roster cap · suffix space exhausted · save refusal/failure ([`PtyError::Store`]); no registration then.
    // ADR-0291 R17 (E4)
    pub fn try_create_agent(&self, mut profile: AgentProfile) -> Result<AgentProfile, PtyError> {
        profile.display_name = normalize_display_name(profile.display_name.take());
        // ★The derivation is also inside the gate★: a read outside would open a window **between derivation and observation** (someone commits the same name in between, both see it free).
        let _gate = self.lock_name_allocation();
        // The cap and the name are decided **together** with one roster sheet — taking it twice would make the gap a window again, and the lookup cost
        //   would double.
        let roster = self.roster();
        check_roster_capacity(&roster)?;
        let desired = profile.canonical_name_when_live();
        match self.decide_name_with_roster(&roster, profile.id, &desired, None) {
            NameDecision::Free => {}
            NameDecision::Suffixed(assigned) => profile.display_name = Some(assigned),
            NameDecision::Exhausted => return Err(name_space_exhausted(&desired)),
            NameDecision::KeepCurrent => {
                unreachable!("decide_name(current=None) cannot produce KeepCurrent")
            }
        }
        self.profiles
            .try_upsert(profile.clone())
            .map_err(PtyError::Store)?;
        Ok(profile)
    }

    /// Agent deletion (tree "delete") — a change with a caller: a save `Err` does not delete ([`PtyError::Store`]); a missing id also attempts a save, so in a refusal state it is `Err`.
    // ADR-0291 R17 (E4)
    pub fn try_delete_agent(&self, id: AgentId) -> Result<(), PtyError> {
        self.profiles.try_remove(id).map_err(PtyError::Store)
    }

    /// Display name override set/clear (tree "rename").
    ///
    /// ★Rename is also a uniqueness check point (ADR-0120 decision 2)★; `None` (clearing the override) is checked too — it makes the name the cwd
    /// basename, **changing it**.
    /// ★★`decide_name` decides everything — no shortcut before the gate★★: filtering "already holds a name of the requested family → no-op" before
    /// it made `bob(1)` stay `bob(1)` although `bob` was free (reported success while the caller sends Ack + list broadcast), and made clearing an
    /// override impossible (`shared(1)` of `C:/shared` always hit). "Give it if free" is knowable only from the roster, inside the gate.
    /// ★A change with a caller★ — a save `Err` leaves the name and is [`PtyError::Store`]; outcomes not attempting a save (`Unchanged` ·
    /// `Exhausted` · `NotFound` at lookup) are `Ok` whatever the store state.
    // ADR-0120
    // ADR-0291 R17 (E4)
    pub fn try_rename_agent(
        &self,
        id: AgentId,
        display_name: Option<String>,
    ) -> Result<RenameOutcome, PtyError> {
        let display_name = normalize_display_name(display_name);
        let _gate = self.lock_name_allocation();
        let Some(profile) = self.profiles.get(id) else {
            return Ok(RenameOutcome::NotFound);
        };
        // ★Its own current name must be **the value the roster says**★: a live agent's roster name derives from session.cwd, the profile derivation from profile.cwd, and they can diverge (`roster()`);
        //   deriving separately could mistake a name someone else holds for its own. For a profile-only (asleep) case they are the same.
        let roster = self.roster();
        let current = roster
            .iter()
            .find(|e| e.id == id)
            .map(|e| e.canonical_name.clone())
            .unwrap_or_else(|| profile.canonical_name_when_live());
        // ★The existing derivation matching liveness is chosen, not one duplicated rule★: live axis (`resolve_canonical_name`) =
        //   `canonical_name_or_id_fallback(override, **raw** session.cwd)` (no canonicalize); asleep axis (`canonical_name_when_live`) =
        //   canonicalize, then basename. Predicting a live agent through the asleep function assumes `basename(canonicalize(cwd)) == basename(cwd)`,
        //   which fails when spawn's canonicalize failed (raw `profile.cwd` kept) or a harness seam plugs an arbitrary cwd — then a name someone else
        //   holds is misjudged `Free` and ADR-0120 uniqueness breaks. ★Adding canonicalize to the live axis is forbidden★ (mail delivery critical path,
        //   ADR-0119). For a non-blank override both functions are the same; whitespace-only counts as "no override" on both.
        let self_live_cwd = roster
            .iter()
            .find(|e| e.id == id)
            .and_then(|e| e.live.as_ref())
            .map(|info| info.cwd.clone());
        let desired = match self_live_cwd {
            Some(live_cwd) => {
                crate::name::canonical_name_or_id_fallback(display_name.as_deref(), &live_cwd, id)
            }
            None => {
                let mut probe = profile;
                probe.display_name = display_name.clone();
                probe.canonical_name_when_live()
            }
        };
        let outcome = match self.decide_name_with_roster(&roster, id, &desired, Some(&current)) {
            // ★The commit result is not swallowed★: `get` · `roster()` · `rename` take the profile lock **separately**, so another connection's
            //   `DeleteProfile` slipping in makes the commit return false; reporting success would send Ack + list broadcast for a nonexistent agent
            //   (the gate serializes only name allocations — deletion does not take it).
            NameDecision::Free => {
                if self
                    .profiles
                    .try_rename(id, display_name)
                    .map_err(PtyError::Store)?
                {
                    RenameOutcome::Renamed(desired)
                } else {
                    RenameOutcome::NotFound
                }
            }
            NameDecision::KeepCurrent => RenameOutcome::Unchanged(current),
            NameDecision::Suffixed(assigned) => {
                if self
                    .profiles
                    .try_rename(id, Some(assigned.clone()))
                    .map_err(PtyError::Store)?
                {
                    RenameOutcome::Renamed(assigned)
                } else {
                    RenameOutcome::NotFound
                }
            }
            NameDecision::Exhausted => {
                tracing::warn!(
                    agent = %id,
                    base = %desired,
                    "suffix space exhausted — rename refused (name unchanged)"
                );
                RenameOutcome::Exhausted
            }
        };
        Ok(outcome)
    }

    /// Tree hierarchy move (set/clear parent) — a change with a caller: a save `Err` leaves the hierarchy as is ([`PtyError::Store`]); a rule violation (`Ok(false)`) also attempts a save, so in a
    /// refusal state it is `Err`.
    // ADR-0291 R17 (E4)
    pub fn try_reparent_agent(
        &self,
        child_id: AgentId,
        parent_id: Option<AgentId>,
    ) -> Result<bool, PtyError> {
        self.profiles
            .try_reparent(child_id, parent_id)
            .map_err(PtyError::Store)
    }

    /// Toggles the boot auto-restore target — a change with a caller: a save `Err` leaves it as is ([`PtyError::Store`]); a missing id (`Ok(false)`) also attempts a save.
    // ADR-0291 R17 (E4)
    pub fn try_set_agent_auto_restore(
        &self,
        id: AgentId,
        auto_restore: bool,
    ) -> Result<bool, PtyError> {
        self.profiles
            .try_update_with(id, |p| p.auto_restore = auto_restore)
            .map_err(PtyError::Store)
    }

    /// ★Harness-only roster injection seam (ADR-0012, same gate shape as `insert_test_session`)★ — plants a profile **bypassing** name uniqueness
    /// (ADR-0120), for tests reproducing states uniqueness forbids. Not compiled into production (feature OFF) — a production call would
    /// silently break uniqueness.
    #[cfg(feature = "test-harness")]
    #[doc(hidden)]
    pub fn seed_agent_bypassing_uniqueness(&self, profile: AgentProfile) {
        self.profiles.upsert(profile);
    }

    /// Acquires the allocation gate. ★Ignores poison★: this Mutex wraps `()` and **protects no state**, and `expect` would let one unrelated
    /// panic block creation, rename and spawn until restart. Holders are outside the containment marker, so production ends that panic first —
    /// only hook-less test builds use this tolerance (ADR-0292).
    fn lock_name_allocation(&self) -> std::sync::MutexGuard<'_, ()> {
        sync::lock(&self.name_allocation)
    }

    /// ★The name decision table (ADR-0120 uniqueness · ADR-0123 numbering) — call only while holding the gate★.
    ///
    /// `self_id` is **excluded** from observation; `current` = the roster's current canonical name for this agent (`None` for a new registration).
    /// The order is the contract: 3) **nobody** has `desired` → `Free` (family is irrelevant; checking family first would silently not give a free
    /// requested name); 4a) someone else has it and `current` is in the `desired` family → `KeepCurrent` (a re-request must not burn a number or
    /// drop to a lower one — name = mail address, ADR-0116); 4b) otherwise → `Suffixed` with a free number of the family.
    /// ★It **receives** the roster snapshot★ — the caller also checks the total cap (and a rename reads its current name) with it in the same
    /// gate section; taking it twice splits worlds and doubles the lookup cost.
    // ADR-0120
    fn decide_name_with_roster(
        &self,
        roster: &[RosterEntry],
        self_id: AgentId,
        desired: &str,
        current: Option<&str>,
    ) -> NameDecision {
        // ★Two separate axes★: `taken_exact` (someone else uses that string — whether to suffix) and the family number set (which number). Mixed, `bob` looks taken when only `bob(1)` exists, or a literal
        //   `bob(0)` looks like it occupied `bob`.
        let mut taken_exact = false;
        let mut used: std::collections::BTreeSet<u32> = std::collections::BTreeSet::new();
        for entry in roster {
            if entry.id == self_id {
                continue;
            }
            match classify_name(desired, &entry.canonical_name) {
                Some(NameKind::Exact) => taken_exact = true,
                Some(NameKind::Suffixed(n)) => {
                    used.insert(n);
                }
                None => {}
            }
        }
        if !taken_exact {
            return NameDecision::Free;
        }
        if let Some(current) = current {
            if classify_name(desired, current).is_some() {
                return NameDecision::KeepCurrent;
            }
        }
        match pick_suffix(&used) {
            Some(n) => NameDecision::Suffixed(format!("{desired}({n})")),
            None => NameDecision::Exhausted,
        }
    }

    /// The point where spawn registers the profile in the roster (only `spawn_agent_watching_link` calls it); `entry` picks the registry entry.
    ///
    /// ★★Uniqueness applies **only to new registrations** — epoch replacement is not a rename (ADR-0115 §Impact · ADR-0007)★★: restart / restore
    /// (`restore_all` · `restore_one`) / reactivation (`activate_profile`) / `resume_no_fallback` replace the map entry of the same AgentId;
    /// whether the id is already in the roster tells them apart (an ad-hoc spawn has an id not in it → new registration).
    /// ★`upsert_preserving_hierarchy` keeps the live `display_name`★, so planting the override only on the new branch is safe; a leak needs both
    /// removed together (`epoch_replacement_never_renames_an_existing_agent`). ★Restore path cost★: boot restore calls this per agent; checking
    /// would pay a full scan + canonicalize per asleep agent inside the gate.
    /// ★`Err` = roster cap · suffix exhausted · deleted in the meantime · (`Entry::Caller`) save refusal/failure★ — the spawn aborts (no duplicate
    /// name; roster unchanged on save `Err`).
    // ADR-0115
    // ADR-0291 R17 (E3)
    fn register_for_spawn(&self, profile: &AgentProfile, entry: Entry) -> Result<(), PtyError> {
        // An existing id needs no name allocation, so the gate is not taken.
        //
        // ★「Is it there」 and 「write」 in **one critical section** — do not revert to two calls★: split, a `DeleteProfile` in between is ignored and
        //   **the profile comes back to life after the deletion was reported** (even its child launches). So
        //   [`ProfileRegistry::update_preserving_hierarchy`] decides existence inside the lock and this only branches on it.
        // ★`false` = deleted in the meantime → **cut the spawn**★ (falling to the new-registration branch would be that resurrection). Same
        //   disposition as the three `profile_vanished_mid_spawn` checks, which catch a deletion **after** the marker settles; neither replaces the other.
        // ★Not every resurrection path is closed★: a deletion finished **before** registration started reaches the new-registration (ad-hoc) branch;
        //   telling it apart needs the caller to say "this entry existed" — not in the `spawn_agent` signature (unresolved).
        if self.profiles.get(profile.id).is_some() {
            return if entry
                .update_preserving_hierarchy(&self.profiles, profile.clone())
                .map_err(PtyError::Store)?
            {
                Ok(())
            } else {
                Err(profile_vanished_mid_spawn(profile.id, "spawn registration"))
            };
        }
        let mut fresh = profile.clone();
        fresh.display_name = normalize_display_name(fresh.display_name.take());
        let _gate = self.lock_name_allocation();
        // ★The cap is checked here too★: this is **the new-registration point of ad-hoc spawn**; empty, the roster could grow without bound bypassing `try_create_agent` (the "total" cap would be false).
        let roster = self.roster();
        check_roster_capacity(&roster)?;
        let desired = fresh.canonical_name_when_live();
        match self.decide_name_with_roster(&roster, fresh.id, &desired, None) {
            NameDecision::Free => {}
            NameDecision::Suffixed(assigned) => fresh.display_name = Some(assigned),
            NameDecision::Exhausted => return Err(name_space_exhausted(&desired)),
            NameDecision::KeepCurrent => {
                unreachable!("decide_name(current=None) cannot produce KeepCurrent")
            }
        }
        entry
            .upsert_preserving_hierarchy(&self.profiles, fresh)
            .map_err(PtyError::Store)
    }

    // ── spawn ──────────────────────────────────────────────────────────────

    /// ★The two branches' 「nothing to do」 is `Ok(Moot)` — do not revert to `Err`★: an entry already up (double-spawn guard below) or already
    /// launching (reservation lost) is moot, not a failure (see `SpawnOutcome`).
    // ADR-0172
    /// ★A thin delegation that drops the delivery channel★ — callers waiting for the connection outcome use
    /// [`AgentManager::spawn_agent_watching_link`]. Here the link's delivery silently fails (by design) and nobody waits.
    /// ★So **the connection failure of an incarnation launched with this verb has no owner**★; the activation entries
    /// ([`AgentManager::activate_profile`] · [`AgentManager::restore_one`]) use the [`LinkWatch`] branch for Fresh and Resume. The only
    /// production caller is the daemon's **instant by-cwd creation** (no profile registered), and codex there has terminal output format, so no
    /// connection axis (canonical: the same-hole comment of `backend/codex/transport.rs`).
    /// ★The entry with a caller★ — a save `Err` of the roster registration or session id clearing does not launch ([`PtyError::Store`]; partial
    /// success = [`AgentManager::spawn_agent_watching_link`]).
    pub fn spawn_agent(
        &self,
        profile: &AgentProfile,
        mode: SpawnMode,
    ) -> Result<SpawnOutcome, PtyError> {
        self.spawn_agent_watching_link(profile, mode, Entry::Caller)
            .map(|(o, _)| o)
    }

    /// The shared body of launching — the three entries (`spawn_agent` · `activate_profile` · `restore_one`) reach it through two verbs.
    ///
    /// `entry` = the caller. Of the three roster changes before opening the process, ① registration and ③ Fresh's session id clearing use that
    /// entry; ② the incarnation marker is memory-only; the post-launch `auto_restore` raise is always internal.
    /// ★Partial success is not reverted (`Entry::Caller`)★: each change has its own save. If ③'s save is `Err` after ① saved, ① remains
    /// (on the roster · not launched, same shape as a process open failure), ②'s new marker stays in memory (no incarnation holds it) and the
    /// field keeps the old handle. If ① fails, ② · ③ are not reached.
    // ADR-0291 R17 (E3 · D8)
    fn spawn_agent_watching_link(
        &self,
        profile: &AgentProfile,
        mode: SpawnMode,
        entry: Entry,
    ) -> Result<(SpawnOutcome, Option<LinkWatch>), PtyError> {
        // ★The reservation is taken **before the roster lookup** — do not revert the order★: on a link declaring a connection it lives **until the
        //   connection outcome** ([`LinkWatch`]), while the session is on the roster but **not yet known to take input**. Looking up first would let a
        //   second request answer `Moot(Some(..))` 「it is up」, and when the first then fails and collects it the second caller **holds as alive an
        //   agent about to die**. Reservation first answers `Moot(None)` (「already launching」). ★On links without a connection axis the order changes
        //   nothing★ (the reservation ends with this function).
        let Some(reservation) = SpawnReservation::reserve(self.spawning.clone(), profile.id) else {
            // The winner has not yet put it on the roster, or even if it has, that connection has not produced its outcome yet — either way
            // we have no ground to answer 「it is up」.
            tracing::info!(
                agent = %profile.id,
                "spawn_agent: already launching — this request has nothing to do (moot)"
            );
            return Ok((SpawnOutcome::Moot(None), None));
        };

        // ★Residual race (ADR-0082 unresolved · follow-up)★: this guard reads `contains_key` under a read lock and releases it, while `sessions.insert`
        //   happens below under a separate write lock; the same id spawned concurrently can pass both this check and `activate_profile`'s pre-check
        //   (a double spawn). It predates ADR-0082. ★It is not only cross-connection★: the daemon runs a connection's activation commands as separate
        //   tasks off its arrival-order lane (`connection_core::dispatch_order`), so **two requests of one connection** can overlap too. The reservation
        //   above narrows the window but does not close it.
        if let Ok(session) = self.get_session(profile.id) {
            tracing::info!(
                agent = %profile.id,
                "spawn_agent: already running — this request has nothing to do (moot)"
            );
            return Ok((SpawnOutcome::Moot(Some(self.agent_info(&session))), None));
        }

        self.register_for_spawn(profile, entry)?;

        // ★Settling the incarnation marker — a new value per incarnation (ADR-0007)★
        //
        // ★Why here, not the caller★: the map replacement happens here and **every spawn passes this line regardless of mode**. Issuing used to be only
        //   in `activate_profile`'s Resume branch, so Fresh respawn paths (WS `Spawn` · Fresh `activate_profile` · boot restore of a profile without
        //   a sid) **reused the dead incarnation's marker**, collapsing every (AgentId, epoch)-keyed structure: the turn observation table (ADR-0113:
        //   a dead incarnation's late emit overwrites the live entry), the control channel token (ADR-0086), the reap epoch-guard (ADR-0084).
        // ★The mode is not looked at★ — a new process is a new incarnation. ★If the profile vanished, **abort the spawn**★ (cut with `?`).
        // ★**It must come right after `register_for_spawn` — nothing in between**★: [`ProfileRegistry::upsert_preserving_hierarchy`] **keeps** the live
        //   marker (ADR-0084), so until this line the roster holds **the previous incarnation's** marker, and a late record of it
        //   (`Some(epoch)` branch of [`ProfileRegistry::observe_session_id`]) **passes** (the guard blocks only 「a newer incarnation already stands」).
        //   Moved after the Fresh clearing, that late record would sit in the cleared field, this incarnation's first-submission commit would be refused
        //   (field ≠ `None`) and the next activation would open the dead incarnation's conversation (ADR-0226).
        // ★Hence three places check 「the profile vanished」, all cut with `?`★: with this line first, a deletion between here and the clearing / roster
        //   read would make them silently return `None` — a resume silently becoming a new conversation, or a session without a profile on the roster.
        // ADR-0007
        let epoch = self.profiles.epoch_for_spawn(profile.id).ok_or_else(|| {
            profile_vanished_mid_spawn(profile.id, "settling the incarnation marker")
        })?;

        // cwd normalization — fixes the notation of the claude session directory (avoids UNC). On failure the original is used (best-effort).
        let cwd = dunce::canonicalize(&profile.cwd).unwrap_or_else(|_| profile.cwd.clone());

        // ★Session id rule (ADR-0226 — 「a stored id exists ⟺ there is a conversation to resume」)★: **nothing new is persisted here**; the first-submission
        //   latch writes this incarnation's id right before the first user turn (`SessionIdLatch` below). Do not bring back issue-and-persist at spawn —
        //   every session closed without input would leave an id with no conversation.
        //   - Fresh: pushes the old handle into history and starts from an empty field ([`fresh_spawn_release_session_id`]). An issuing backend **must
        //     draw a new sid** — reusing the stored one collides with the disk session ("Session ID already in use", ADR-0076).
        //   - Resume: **only reads** the stored sid. Who gets our sid: [`session_id_to_hand_over`].
        // ★`None` = 「the profile vanished」 — cut, do not swallow★ (deleted after the marker settled).
        // ADR-0226
        if fresh_spawn_release_session_id(&profile.command, mode) {
            entry
                .release_session_id(&self.profiles, profile.id)
                .map_err(PtyError::Store)?
                .ok_or_else(|| profile_vanished_mid_spawn(profile.id, "clearing the session id"))?;
        }

        // ★**Read the roster, not the caller's snapshot**★ — the field is **written by the other side** (the record port stores codex's handshake thread
        //   id), so a handle arriving after the snapshot is not in the copy (reading it would resume an old thread). It has a value here only since
        //   [`ProfileRegistry::upsert_preserving_hierarchy`] keeps `backend_session_id`.
        // ★Reading **after** `epoch_for_spawn` is the grounds★: from the new marker until this link receives `offer_sink`, every marker-carrying record is
        //   refused. ★Not 「nobody can write」★: the `incarnation: None` branch of [`ProfileRegistry::observe_session_id`] **writes unconditionally**, and
        //   the daemon's [`SessionTracker`] callback (`engram-dashboard-daemon/src/lib.rs`) enters it. The value is frozen because that observer does
        //   not attach to codex profiles (`tracker.watch` sits behind the `assigns_sid` gate; codex has it off — ADR-0185) — switching that gate to the
        //   resume axis breaks this first.
        // ★`get` → `None` = deleted in the meantime → **cut**★ (`and_then` would silently make a new conversation and put a session without a profile
        //   on the roster).
        let stored_handle = match mode {
            SpawnMode::Resume => {
                self.profiles
                    .get(profile.id)
                    .ok_or_else(|| {
                        profile_vanished_mid_spawn(profile.id, "reading the resume handle")
                    })?
                    .backend_session_id
            }
            SpawnMode::Fresh => None,
        };
        // ★This read is authoritative — empty means refuse before launching★: a backend resuming with the stored handle **silently drops** the resume
        //   flag when there is none, opening an unknown new conversation reported as 「resumed」. The common empty case is turned into a new
        //   conversation at the head of `resume_no_fallback`; this is the back door of the race where the field empties between that decision and
        //   this read (same predicate). The next activation has no handle and goes to a new conversation.
        // ADR-0226
        if mode == SpawnMode::Resume
            && stored_handle.is_none()
            && backend::can_resume_stored_session(&profile.command)
        {
            return Err(PtyError::SpawnFailed(format!(
                "the resume handle vanished during spawn (agent {}) — not launching the process",
                profile.id
            )));
        }
        let resume_session_id = resume_handle_for(mode, stored_handle);

        let assigns_sid = backend::assigns_session_id(&profile.command);
        let sid = session_id_to_hand_over(&profile.command, mode, stored_handle);

        // ADR-0086 ★Called right before spec assembly★ — `build_command_spec` must inject the endpoint the backend's way (claude=`--mcp-config`); the
        //   marker is the value settled above, so a token is issued per incarnation (lifetime = (AgentId,epoch)).
        // ★Backend-conditional (round-2 F3)★: provision only for backends that **consume** the control channel (claude); shell has
        //   supports_control_channel=false and the registry is untouched, so a config-write failure never aborts a shell spawn. Decided by backend
        //   dispatch (ADR-0004), not `matches!` on the command.
        // ★Fail-closed (FIX 2)★: for a calling backend, `Err` (CSPRNG/file write failure) **aborts the spawn** (no agent running without a control
        //   channel). `Ok(None)` = legitimately not using one (Noop) → proceed. `Ok(Some)` → armed with ProvisionGuard below, so any failure before
        //   session registration reclaims the token/config; a non-calling backend (supports_control_channel=false) flows as `None`, outside fail-closed.
        let control_endpoint = if backend::supports_control_channel(&profile.command) {
            // ADR-0099: the backend's MCP capability goes to provision so channel wiring, priming (non-MCP branch gets no instructions — ADR-0209) and grants
            //   split at once (invariant: taught channels ⊆ laid channels — one-way per ADR-0126 decision 4). Backend dispatch (ADR-0004).
            // ADR-0126
            // ★The axes are asked **each separately**★ — folding one from the other silently opened send authorization for a backend 「using control but
            //   outside the mail plane」 ([`ControlChannelNeeds`]).
            // ADR-0133
            // ★The third axis too★: deriving 「does it read the file we wrote」 from 「uses mail through MCP」 would hand a backend that enables MCP another way
            //   a plaintext token file it never reads (ADR-0209 · `backend::writes_mcp_config_file`).
            let needs = crate::types::ControlChannelNeeds {
                accepts_mcp_config: backend::accepts_mcp_config(&profile.command),
                writes_mcp_config_file: backend::writes_mcp_config_file(&profile.command),
                uses_mail: backend::uses_mail(&profile.command),
            };
            self.control
                .provision(profile.id, epoch, needs)
                .map_err(|e| {
                    PtyError::SpawnFailed(format!(
                        "control channel provision failed (fail-closed): {e}"
                    ))
                })?
        } else {
            None
        };
        let mut provision_guard = control_endpoint.as_ref().map(|_| ProvisionGuard {
            control: self.control.clone(),
            id: profile.id,
            epoch,
            armed: true,
        });

        // ★The second fail-closed door — cuts failures the daemon cannot see★: provision's `?` catches only daemon-side failures, so 「could not build the
        //   attachment value」 of **a backend that writes no file** (codex `-c mcp_servers.…`) would pass; that agent would be on the delivery list with 0 send
        //   entries, every request an unanswerable contract. ★Placed **after** the guard is armed★ so the issued token and file are reclaimed. Backend
        //   dispatch (ADR-0004).
        // ADR-0209
        backend::precheck_control_endpoint(&profile.command, control_endpoint.as_ref()).map_err(
            |e| {
                PtyError::SpawnFailed(format!(
                    "backend cannot use the provisioned control channel (fail-closed): {e}"
                ))
            },
        )?;

        // ★`sid` is not passed as is — this field is **the stored backend sid** (ADR-0185 two axes)★: `sid` is filled only for issuing backends, so for codex
        //   it is always `None` even when a resumable value exists.
        // ★One value goes to **two places**★ — `build_command_spec` (backends resuming by command line, codex terminal `resume <id>`) and `open_spawn`
        //   (link-requested resume, codex app-server `thread/resume`); different values would split 「what this spawn resumes」. ★No assembly here
        //   (ADR-0004)★ — the backend decides subcommand/flag/none. ★Empty on Fresh★ (`resume_handle_for` above): the mode is not re-judged by the backend.
        let spec = backend::build_command_spec(
            &profile.command,
            mode,
            sid,
            resume_session_id,
            cwd.clone(),
            profile.env.clone(),
            control_endpoint.clone(),
        );

        // ADR-0079: only json-mode claude actually reads the transcript — the terminal is restored by TUI PTY repaint
        //   and shell has no conversation, so other backends return an empty Vec.
        let seed_events = match mode {
            SpawnMode::Resume => match sid {
                Some(s) => backend::resume_transcript_events(&profile.command, &cwd, s),
                None => Vec::new(),
            },
            SpawnMode::Fresh => Vec::new(),
        };

        // ADR-0191: the backend hands out the real link and the session-carried values (caps · encoder · turn classifier · mail eligibility) **in one
        //   dispatch**; this place does not know the link's real type. Mail eligibility rides the session so a live session keeps it after profile
        //   deletion (`AgentManager::reads_messages`).
        // ★This call launches the child — after the transcript read above★ (else the file is read while the program runs).
        // ADR-0185: where a backend that **fetches** the session id (codex) announces it — this incarnation's first-submission latch (ADR-0226), which writes
        //   the roster right before the first user turn. ★Handed unconditionally on purpose — no `receives_session_id()` axis★: it has no effect on
        //   backends that do not read it (unlike tokens/config files), so a declaration table would duplicate the impl's decision. The commit port binds the
        //   `epoch` above so a record after this spawn died cannot overwrite the next incarnation.
        // ★The delivery port only for backends declaring a connection★ — elsewhere the supervisor would wait on a channel nobody calls (claude · shell ·
        //   stdio · codex terminal get `None` and stay byte-identical). ★The marker is stamped **here**★ (`link_verdict_sink` has the reason).
        let (link_sink, link_rx) = if backend::declares_link(&profile.command) {
            let (tx, rx) = std::sync::mpsc::channel();
            (Some(link_verdict_sink(tx, epoch)), Some(rx))
        } else {
            (None, None)
        };
        // ★An issuing backend puts its id into the latch **here, before `open_spawn`**★: input is accepted from the roster registration below and the
        //   following announcement releases parked mail at once, so a later offer could let the first submission go out unknowing and the commit land after
        //   the turn. Backend-neutral: it keys on holding `sid` (Fresh = just drawn, Resume = stored). ★Commit compares against what this incarnation saw
        //   at start (`stored_handle`; Fresh `None`)★ — a concurrent writer (claude file watcher) is refused, not rewound.
        // ADR-0226
        let latch = SessionIdLatch::new(
            profile.id,
            epoch,
            session_id_sink(self.profiles.clone(), profile.id, epoch, stored_handle),
        );
        if let Some(s) = sid {
            latch.offer(&s.to_string());
        }
        let parts = backend::open_spawn(
            &profile.command,
            &spec,
            DEFAULT_COLS,
            DEFAULT_ROWS,
            Some(latch.offer_sink()),
            // ★A backend whose link owns the turn counts submissions here★ — that mode's session does not; whether to plug it in is the backend's decision (others drop it).
            // ADR-0226 · ADR-0231: persisted when the id became real (the peer's first user message echo).
            Some(latch.first_turn_sink()),
            resume_session_id,
            link_sink,
            // ★**The same endpoint** as `build_command_spec` above★ — handed once more for backends carrying it **in the link handshake** (codex app-server:
            //   priming in `thread/start`); which means applies is the backend's decision (ADR-0004).
            control_endpoint.as_ref(),
        )?;

        // ADR-0226: these incarnation facts are held only by the session (what carries them on the wire is the subscription response).
        let incarnation = SpawnIncarnation {
            continues_conversation: resume_session_id.is_some(),
            latch,
        };
        let (session, child_pid) =
            self.spawn_session(profile.id, spec, parts, epoch, seed_events, incarnation)?;

        if let Some(g) = provision_guard.as_mut() {
            g.disarm();
        }

        // sid drift observation (best-effort); making the observer or answering "nothing to make" is the backend's job (ADR-0004).
        // ★The gate is **the issuing axis** (ADR-0185)★: the observer takes the sid we handed over as `expected_sid`; a session we did not issue has no
        //   reference value, so the resume axis would attach it to one without.
        if let (Some(s), Some(pid)) = (sid, child_pid) {
            if assigns_sid {
                if let Some(source) =
                    backend::session_id_source(&profile.command, profile.id, pid, s)
                {
                    self.tracker.watch(profile.id, source);
                }
            }
        }

        tracing::info!(agent = %profile.id, epoch, ?mode, "agent spawned");

        let info = self.agent_info(&session);
        self.status_sink.agent_list_updated(self.list_agents());
        // ★The mailbox and the reservation are handed over **as one set**★ — a waiter for the outcome must not release the reservation in between ([`LinkWatch`]). Without a connection axis it is `None` and the
        //   reservation ends with this function (`reservation` dropped here).
        let watch = match link_rx {
            Some(rx) => Some(LinkWatch {
                rx,
                _reservation: reservation,
            }),
            None => None,
        };
        Ok((SpawnOutcome::Started(info), watch))
    }

    /// ★Manual activation entry point — resume only, fresh-fallback abolished (ADR-0082)★. The SpawnProfile handler calls this instead of `spawn_agent`.
    ///
    /// 1. **Already running (reactivation guard)** — returns that session's AgentInfo as is, **without killing or respawning** (a harmless "already
    ///    running" signal, epoch unchanged). ★Core fix of the a4aac1a regression★: the double-spawn guard's `Err` was mistaken by
    ///    `resume_with_fresh_fallback` for a resume failure and `fallback_fresh` **killed a healthy live agent** and replaced it with an empty one.
    ///    Now blocked in two layers: this lookup first, and the guard emits `SpawnOutcome::Moot` instead of an `Err`.
    /// 2. **Fresh (truly new — no session)** — `spawn_fresh_settled`; normal creation, not a failure fallback (ADR-0076).
    /// 3. **Resume** — `resume_no_fallback` only; its Failed outcome is exposed as `Err`.
    ///
    /// ★blocking★ is decided by the link's connection axis, not the mode: links not declaring a connection (claude · shell · stdio · codex terminal) —
    /// Resume polls up to EARLY_EXIT_WINDOW (3s), Fresh returns at once; links declaring one (codex app-server) — **both** wait for the connection
    /// outcome, a **signal** (ADR-0201) returning immediately normally (~130ms), capped by [`LINK_RESOLUTION_BACKSTOP`] (giving up, not deciding).
    /// The reactivation and in-flight guards return at once on either axis. A failure can return sooner when the diagnostic stream speaks first.
    /// **Contract: call only on a blocking pool** — worst case is the backstop (15 s) plus the teardown's `session.kill` (pump join ≤ 5 s) with **no
    /// yield point**, so an async worker would be tied up and N stuck activations stop other connections' commands. The four production callers keep it
    /// (command bus behind `blocking_handler` in `crate::commands::make_table`; WS `Spawn` · `SpawnProfile` behind `spawn_blocking` in `connection_core`).
    /// ★The entry with a caller (`Entry::Caller`)★ — a save `Err` of the pre-launch roster change does not launch ([`PtyError::Store`]; partial
    /// success = [`AgentManager::spawn_agent_watching_link`]); boot restore uses `Entry::Internal`.
    // ADR-0082
    // ADR-0076
    // ADR-0201
    // ADR-0291 R17 (E3)
    pub fn activate_profile(
        &self,
        profile: &AgentProfile,
        mode: SpawnMode,
    ) -> Result<AgentInfo, PtyError> {
        // ★Before answering 「it is up」, check 「an activation without an outcome yet」 — do not revert the order★: on a link declaring a connection the
        //   incarnation on the roster may not yet take input ([`LinkWatch`]) and the request opening that interval holds the reservation; answering `Ok`
        //   (running) to a second activation would let it **hold as alive an agent about to die** when the first fails and collects it.
        // ★`Err` is honest★: this request did nothing and has no live session to return (the Fresh branch below uses the same text) — extended here until the
        //   connection stands. ★Narrows the window, does not close it★: a few-command window remains before the reservation in
        //   `spawn_agent_watching_link` (ADR-0082 「Open items」 ③); what closed is the long one-handshake window.
        if self.activation_in_flight(profile.id) {
            tracing::info!(
                agent = %profile.id,
                "activate_profile: another request's activation has not produced its outcome yet — this request has nothing to do"
            );
            return Err(PtyError::SpawnFailed(format!(
                "another request is already activating agent {}; this one did nothing",
                profile.id
            )));
        }

        if let Ok(session) = self.get_session(profile.id) {
            tracing::info!(
                agent = %profile.id,
                "activate_profile: already running — reactivation ignored (live agent preserved, ADR-0082)"
            );
            // ★Writes nothing to 「the last failure」★: no activation was attempted — no failure to record, no ground to clear (as the moot branch of `spawn_agent`, ADR-0172).
            return Ok(self.agent_info(&session));
        }

        if mode == SpawnMode::Fresh {
            // ★The record is made inside it — writing here would make two clearing points★ (only the place that saw the outcome knows what happened; as the Resume branch leaving it to `resume_no_fallback`).
            let outcome = self.spawn_fresh_settled(profile, Entry::Caller);
            // If moot, return the session someone else launched (is launching); if not on the roster yet, look once more.
            // ★The text when still missing is not "a nonexistent agent"★: the profile exists and **another request is launching it**; `NotFound` would
            //   misidentify the cause to the caller and the LLM.
            // ADR-0172
            return match outcome?.into_info() {
                Some(info) => Ok(info),
                None => self.agent_info_by_id(profile.id).map_err(|e| match e {
                    PtyError::NotFound(id) => PtyError::SpawnFailed(format!(
                        "another request is already starting agent {id}; this one did nothing"
                    )),
                    other => other,
                }),
            };
        }

        // A save `Err` is returned in its original form — so it becomes the same answer as the `?` of the Fresh branch (ADR-0291 R17).
        let (outcome, spawned) = self
            .resume_no_fallback(profile, Entry::Caller)
            .map_err(PtyError::Store)?;
        match outcome {
            // Success and failure were already recorded inside `resume_no_fallback` (the only place knowing what happened); recording here would make two
            //   clearing points.
            // ★A lookup failure does not flip a success★: success was decided and the record **cleared** at the early-exit window; if the child dies right
            //   after and the reaper collects it, `Err` would contradict "failure while having cleared the record" — fall back to spawn-time info.
            // ★But not returned **as alive**★: the spawn-time snapshot says `Running` (`state_of` → "live", WS `Spawned` carries it, then mail goes to a
            //   corpse). The lookup missing is itself the observation 「collected in between」, so it is lowered to a terminal state, code unknown (`None`).
            // ADR-0172
            // ★`Started` = `Resumed`★ — both 「launched」, differing only in vocabulary (no handle → new conversation opened by `resume_no_fallback`); lowering it
            //   to `Err` would report a normal launch as failure.
            RestoreOutcome::Resumed | RestoreOutcome::Started => {
                match self.agent_info_by_id(profile.id) {
                    Ok(info) => Ok(info),
                    Err(e) => match spawned {
                        Some(mut info) => {
                            info.status = AgentStatus::Exited { code: None };
                            Ok(info)
                        }
                        None => Err(e),
                    },
                }
            }
            RestoreOutcome::Failed { reason } => Err(PtyError::SpawnFailed(reason)),
            // Only resumable profiles enter, so other outcomes are unreachable (defensive Err). ★The defensive branch does not touch state★ — it does not know what happened and an assertive failure could stamp an
            //   agent that just launched (with no epoch guard).
            other => Err(PtyError::SpawnFailed(format!(
                "activate_profile: unexpected outcome {other:?}"
            ))),
        }
    }

    /// ★**The only point** writing 「the last failure」 (ADR-0172 decision 4 — amended)★. `None` = the activation held → clear. `Some(kind)` = record now.
    ///
    /// ★The clearing condition is 「activation success」, not 「a conversation round trip held」 — do not revert★: a round trip is observed only by the turn-end
    /// signal (structured output only), while failure **classification** comes only from console bytes (structured sessions send diagnostics to stderr);
    /// the halves are mutually exclusive, so a terminal agent's record **would never clear**. That signal also has no epoch gate and a non-blocking contract
    /// (`StatusSink::turn_ended`), so taking the profile lock there was a contract violation.
    /// ★Success is an honest clearing ground★: a `--resume` passing the early-exit window means the conversation **really existed**; if closed without a
    /// word, **the next activation fails again and is recorded again**.
    /// ★Called on the activation thread★, not the output pump. ★`incarnation` is the late-write guard★ (contract: `ProfileRegistry::set_last_failure`): outcomes
    /// that made an incarnation send its epoch; only failures before one are `None`.
    // ADR-0172
    fn note_activation_result(
        &self,
        id: AgentId,
        incarnation: Option<u32>,
        failure: Option<AgentFailureKind>,
    ) {
        self.profiles.set_last_failure(id, incarnation, failure);
    }

    /// Whether this entry's activation is **still in progress without an outcome**. ★Reading the reservation set as is suffices — no second state★: its
    /// lifetime is already 「someone is launching this entry」 and [`LinkWatch`] extends it **until the connection outcome**; a separate 「establishing」
    /// table would add two set/clear places and revive the shape where the link kept its own state for the supervisor to peek at (root of four defects).
    /// One `contains` under the lock; no sessions/profiles lock is taken (ADR-0006).
    fn activation_in_flight(&self, id: AgentId) -> bool {
        self.spawning
            .lock()
            .expect("spawning set poisoned")
            .contains(&id)
    }

    /// The record of branches **settled by one spawn** — launched: clear with that incarnation's epoch; failed: record; **nothing to do: write nothing**.
    ///
    /// ★Only for [`AgentManager::spawn_fresh_settled`]'s **links without a connection axis**★: on a connection-declaring link `Started` returns before the
    /// outcome exists, and clearing then is the evidence destruction ADR-0202 prevents (that branch writes after seeing the decision).
    /// ★moot writing nothing is the definition★ (nothing done → no failure, no ground to clear; same as `activate_profile`'s "already running" branch).
    /// Resume does not use this — success holds only after the early-exit window, known to `resume_no_fallback`.
    // ADR-0172
    fn note_spawn_result(&self, id: AgentId, result: &Result<SpawnOutcome, PtyError>) {
        match result {
            Ok(SpawnOutcome::Started(info)) => {
                self.note_activation_result(id, Some(info.epoch), None)
            }
            Ok(SpawnOutcome::Moot(_)) => {}
            // There is no incarnation, so there is no generation to compare (the `None` branch of the `set_last_failure` contract).
            Err(e) => self.note_activation_result(id, None, Some(spawn_error_kind(e))),
        }
    }

    /// Fresh activation — ★does not claim success just because the process launched★.
    ///
    /// ★Why this exists (deleting it brings the hole back)★: `spawn_agent` returns `Started` **when the child launches** and drops the mailbox; on a
    /// connection-declaring link the handshake has not started, so a codex Fresh whose `thread/start` was refused was **seen by nobody** and: ① a peer
    /// holding stdout left the reader without EOF, [`OutputCore::finish`] and the reaper never ran — **the freeze** (Running, all input refused; ADR-0200's
    /// shape on the resume side); ② `note_spawn_result` read `Started` as success and **cleared 「the last failure」**, destroying the earlier evidence
    /// (the hole ADR-0202 wrote as 「not yet true」). Fresh is the everyday path of new codex agents.
    /// ★「Success」 means the same as Resume (ADR-0201)★: the agent **signaled it is ready to take input** (`thread/start` vs `thread/resume` is a split inside
    /// the link). ★Success **does not clear** 「the last failure」 (ADR-0202)★ — readiness says nothing about what follows and clearing is irreversible.
    /// ★Links without a connection axis change not at all★ — they fall out via the let-else below into the old `note_spawn_result` path (ADR-0201 scope).
    /// ★The failure kind stays [`AgentFailureKind::Other`]★: `resume_failure_kind` speaks **resume** vocabulary, false here (no resume was tried); 「other」 is
    /// retryable so the screen does not block the entry (`failureKinds.ts`). `entry` is handed to the shared body as received.
    // ADR-0201
    // ADR-0202
    fn spawn_fresh_settled(
        &self,
        profile: &AgentProfile,
        entry: Entry,
    ) -> Result<SpawnOutcome, PtyError> {
        let attempt = self.spawn_agent_watching_link(profile, SpawnMode::Fresh, entry);
        let (result, watch) = match attempt {
            Ok((outcome, watch)) => (Ok(outcome), watch),
            Err(e) => (Err(e), None),
        };

        // The three with no outcome to wait for fall out here — links without a connection axis · a request that made nothing (moot) · a failure before a process opened; all follow the old record discipline
        // (`note_spawn_result`).
        let (Some(watch), Ok(SpawnOutcome::Started(info))) = (watch.as_ref(), &result) else {
            self.note_spawn_result(profile.id, &result);
            return result;
        };
        // ★The marker is carried **out of** the outcome★ — cleanup and record below happen as late as the decision, and the incarnation may have been collected and the next launched; a carried value makes
        //   both reach only this incarnation (ADR-0163/0164, match/mismatch only).
        let incarnation = info.epoch;

        match self.link_activation_verdict(
            profile.id,
            incarnation,
            &watch.rx,
            LINK_RESOLUTION_BACKSTOP,
        ) {
            // ★The other side signaled ready = holds★; the record is untouched (ADR-0202).
            // ★★The snapshot is **taken again** — do not return `result` as is★★: its `info` was assembled right after launch with `Running`; now that we waited for the
            //   outcome it is stale by up to the backstop, and a child dying meanwhile would be reported alive (WS `Spawned` carries the status; **mail then goes to a
            //   corpse**). ★A lookup failure only lowers the state★ (observation 「collected in between」), the activation held so no `Err`; code unknown (`None`) — the
            //   same discipline as `activate_profile`'s sibling branch (canonical there).
            EarlyVerdict::Ready => match self.agent_info_by_id(profile.id) {
                Ok(fresh) => Ok(SpawnOutcome::Started(fresh)),
                Err(_) => {
                    let mut gone = info.clone();
                    gone.status = AgentStatus::Exited { code: None };
                    Ok(SpawnOutcome::Started(gone))
                }
            },
            // ★Cleanup **first**, record later★ — as `resume_no_fallback`: the record takes the profile mutex, and the reversed order **never kills the child** when it is held (the backstop exists for that stall).
            EarlyVerdict::LinkFailed { reason } => {
                self.tear_down_failed_activation(profile.id, incarnation);
                self.note_activation_result(
                    profile.id,
                    Some(incarnation),
                    Some(AgentFailureKind::Other),
                );
                tracing::warn!(
                    agent = %profile.id,
                    epoch = incarnation,
                    %reason,
                    "the new conversation's connection could not be set up → activation failed — a target for LLM escalation"
                );
                Err(PtyError::SpawnFailed(format!(
                    "new conversation connection failed: {reason}"
                )))
            }
            // The process died before the decision finished — already terminal, nothing to clean up (ADR-0082 「kill nothing」: an observed corpse is left).
            // ★What the user interrupted is not an activation failure★ — not recorded (as `resume_no_fallback`).
            EarlyVerdict::Terminal { status, evidence } => {
                if matches!(status, AgentStatus::Killed) {
                    tracing::info!(
                        agent = %profile.id,
                        epoch = incarnation,
                        "the user terminated while the new conversation was being set up — not recorded as an activation failure"
                    );
                } else {
                    self.note_activation_result(
                        profile.id,
                        Some(incarnation),
                        Some(AgentFailureKind::Other),
                    );
                    tracing::warn!(
                        agent = %profile.id,
                        epoch = incarnation,
                        ?status,
                        %evidence,
                        "exited before the new conversation was set up → activation failed — a target for LLM escalation"
                    );
                }
                // ★The tail is **not carried in the error text**★ — up to 4 KiB of console would go out as the command response; the log above carries it and the text keeps the shape of the `resume_no_fallback` sibling.
                Err(PtyError::SpawnFailed(format!(
                    "exited before the new conversation was set up ({status:?})"
                )))
            }
            // ★Unreachable — [`AgentManager::link_activation_verdict`] does not emit these two★. The defensive branch does not touch state (an assertive failure could stamp an agent that just launched).
            other => Err(PtyError::SpawnFailed(format!(
                "spawn_fresh_settled: unexpected outcome {other:?}"
            ))),
        }
    }

    /// ★The link is not made here (ADR-0191)★ — it arrives as `parts` and its real type is unknown; **the child process is already up** before the caller hands it over.
    fn spawn_session(
        &self,
        id: AgentId,
        spec: CommandSpec,
        parts: backend::SpawnParts,
        epoch: u32,
        seed_events: Vec<OutputEvent>,
        incarnation: SpawnIncarnation,
    ) -> Result<(Arc<AgentSession>, Option<u32>), PtyError> {
        // ★Destructured by name — never `{ latch: _, .. }`★: the required argument carries the latch only this far; the wiring is measured by a structure test (`SpawnIncarnation` doc).
        // ADR-0226
        let SpawnIncarnation {
            continues_conversation,
            latch,
        } = incarnation;
        let backend::SpawnParts {
            transport,
            child_pid,
            backend_caps,
            encoder,
            turn_classifier,
            reads_messages,
            mid_turn,
            delivery_ack,
        } = parts;

        // ADR-0113: plug in the shared turn observation table + this backend's signal classifier together — without them only this session falls out of observation.
        // ADR-0231: this incarnation's new registry + the shared waiting-list table go in the same place (the session takes the registry from the core) — omitted,
        //   the user list is invisible to mail.
        let core = Arc::new(
            OutputCore::new(
                id,
                epoch,
                self.status_sink.clone(),
                TurnWiring::new(self.turns.clone(), turn_classifier),
            )
            .with_queued(QueuedWiring {
                registry: Arc::new(QueuedInputs::new()),
                pending: self.inputs_pending.clone(),
            }),
        );

        // 2.1. ★ADR-0079 seed-before-publish (load-bearing order)★: seed the restored past events of resume into the core Ring **before the session becomes
        //      observable** (before the sessions map insert) — the core is still this function's local Arc, unreachable by subscribe/emit paths. Closes (a)
        //      empty-ring replay: a reconnecting subscription slipping in between would replay an empty Ring and, since seed does not fan out, lose the past
        //      permanently; (b) seq interleave: concurrent emits would break Ring order ([0,2,1]) and replay's `partition_point` premise. Still before
        //      `start_pump`. Empty `seed_events` (Fresh · non-json · no transcript) is a no-op.
        if !seed_events.is_empty() {
            tracing::info!(
                agent = %id,
                epoch,
                count = seed_events.len(),
                "ADR-0079: resume transcript seed (before publish)"
            );
            core.seed(seed_events);
        }

        // ★ADR-0019 finish-snapshot hook★: called once on the finalize winner path of core.finish; snapshots intent · shutting_down **at that moment** and sends ReapMsg (no live read at reap time — avoids the
        //   crash→user-kill misclassification race). A send failure (reaper ended) is ignored.
        let intent = Arc::new(AtomicU8::new(TerminationIntent::None as u8));
        {
            let intent_hook = intent.clone();
            let shutting_down_hook = self.shutting_down.clone();
            let reaper_tx = self.reaper_tx.clone();
            core.set_on_terminal(Box::new(move |reason: TerminalReason| {
                let msg = ReapMsg {
                    id,
                    epoch,
                    reason,
                    intent_at_finish: TerminationIntent::from_u8(
                        intent_hook.load(Ordering::SeqCst),
                    ),
                    shutting_down_at_finish: shutting_down_hook.load(Ordering::SeqCst),
                };
                let _ = reaper_tx.send(ReaperCmd::Reap(msg));
            }));
        }

        let session = Arc::new(
            AgentSession::new(
                id,
                spec.cwd.clone(),
                epoch,
                DEFAULT_COLS,
                DEFAULT_ROWS,
                intent,
                backend_caps,
                encoder,
                reads_messages,
                core,
                transport,
            )
            .with_incarnation(continues_conversation)
            .with_session_id_latch(latch)
            // ADR-0231: omitting this still compiles with no error — all of this incarnation's input goes to today's path.
            .with_mid_turn(mid_turn, delivery_ack),
        );

        // ★ADR-0113 claim the turn observation slot **before** the sessions map insert★: this incarnation takes that id's entry (replacing the previous one).
        //   Before insert nothing can reach this core, so it is **always before the first signal**; reversed, that signal would mismatch the previous marker and
        //   be dropped, and the incarnation would answer unobserved (not in a turn) — mail injection mid-turn. ★The old marker-ordering filter moved here★
        //   (random markers order correctly only half the time — `turn::TurnObservations::register`).
        // ADR-0113
        self.turns.register(id, epoch);
        // The waiting-list table goes in the same place for the same reason — it drops unregistered writes, so a late registration leaves that incarnation's list invisible to mail.
        // ADR-0231
        self.inputs_pending.register(id, epoch);

        // ★ADR-0019 — register in sessions **before** starting the pump★: the finish hook sends ReapMsg; on a fast exit EOF→finish must find the session in the map
        //    or reap is a no-op and the session becomes a zombie. `attach_pump` completes synchronously inside start, so `join_pump` is independent of insert order.
        self.sessions
            .write()
            .expect("sessions poisoned")
            .insert(id, session.clone());

        // 5.5. ★ADR-0019 activation — before start_pump★: a spawned (= up) profile is persisted `auto_restore=true`. Flipping before the pump closes the race where an
        //      immediate crash (`cmd /c exit 1`) makes the reaper lower it to false and a later flip overwrite it — the crashed session would stay a boot restore target
        //      (crash loop). Order "flip true → start_pump → (crash) reaper false" keeps the reaper's downgrade **last**; spawn only raises here (the reaper only lowers).
        // ★Always the internal entry whatever the entry (ADR-0291 E1 · not `try_`)★: the process is already up, so a save failure cannot revert it; `Err` still raises
        //   in memory + dirty and the next save carries it.
        self.profiles.update_with(id, |p| p.auto_restore = true);

        session.start_pump();

        Ok((session, child_pid))
    }

    // ── Restore ────────────────────────────────────────────────────────────────

    /// **Call from a background thread** (blocks on the stagger and early-exit window waits — no synchronous call from
    /// setup, H-1.8).
    pub fn restore_all(&self) -> Vec<RestoreReport> {
        let targets = self.profiles.restorable();
        tracing::info!(count = targets.len(), "restore_all started");

        let mut reports = Vec::with_capacity(targets.len());
        for profile in targets {
            let outcome = self.restore_one(&profile);
            // After a successful spawn the latest marker from `epoch_for_spawn` is in the roster, so read it; the profile is missing only if spawn failed (outcome Failed; the snapshot value is then report notation only).
            let epoch = self
                .profiles
                .get(profile.id)
                .map(|p| p.epoch)
                .unwrap_or(profile.epoch);
            let report = RestoreReport {
                agent_id: profile.id,
                epoch,
                outcome,
            };
            tracing::info!(agent = %report.agent_id, ?report.outcome, "restore result");
            self.status_sink.restore_result(report.clone());
            reports.push(report);
            std::thread::sleep(RESTORE_STAGGER);
        }
        reports
    }

    /// ★Boot restore is an internal entry (`Entry::Internal`)★ — nobody receives the result to revert, so a pre-launch save `Err` applies to memory + dirty and the
    /// launch continues; not overwriting a newer-edition or unreadable file is guarded by every save's re-decision.
    // ADR-0291 R17 (E1 · E3)
    fn restore_one(&self, profile: &AgentProfile) -> RestoreOutcome {
        // ADR-0185: the resume axis — **does not ask who issued** the stored sid (that axis is `assigns_session_id` at spawn). One rule with the other activation entries, held by dispatch.
        let resumable = backend::can_resume_profile(profile);

        if !resumable {
            // ADR-0172: boot restore follows the same discipline — clear if launched, record on the spot if failed.
            // ★The **same verb** as manual activation (ADR-0201/0202)★: a codex profile without a resume handle also lands here, and a refused `thread/start` would
            //   leave the unseen-outcome hole for **every profile** at once; per-entry decisions would let one fall behind.
            let outcome = self.spawn_fresh_settled(profile, Entry::Internal);
            return match outcome {
                // moot (already up) folds into the same report ("that entry is up") — the restore report vocabulary has no 「nothing to do」 (adding one is a wire change, separate).
                Ok(_) => RestoreOutcome::Started,
                Err(e) => RestoreOutcome::Failed {
                    reason: e.to_string(),
                },
            };
        }

        match self.resume_no_fallback(profile, Entry::Internal) {
            Ok((outcome, _)) => outcome,
            // The internal entry does not emit a save `Err` — if it comes, it is folded into the restore report text (defense).
            Err(store) => RestoreOutcome::Failed {
                reason: format!("resume spawn failed: {}", PtyError::Store(store)),
            },
        }
    }

    /// ★Shared discipline for resume only (ADR-0082 — boot restore and manual activation, fresh-fallback abolished)★. Premise: this profile was already decided
    /// resumable (claude + sid) — an explicit resume request without a handle is left to a new conversation at the head for stored-handle backends (ADR-0226).
    ///
    /// On a spawn failure, or within EARLY_EXIT_WINDOW an **abnormal exit or a diagnostic stream saying it failed** (claude: "No conversation found ..." to
    /// stderr), it goes straight to a terminal point (corpse) **without creating a new conversation**, leaving the reason in the log for the LLM.
    /// ★The terminal point is not `AgentStatus::Failed`★: state splits only by `TerminalReason` (`OutputCore::finish`); the resume child dies by itself →
    /// `Exited{code}` (usually ≠0). `RestoreOutcome::Failed` is **a different axis** (the activation result). ★Not "dying immediately"★ (measured): claude runs
    /// its exit hook a few more seconds, so the decision rests on evidence too (`EarlyVerdict::Diagnosed`).
    /// ★Nothing is killed or respawned★: the child's pump does EOF→finish, the reaper collects it and lowers `auto_restore=false`, leaving the corpse in its
    /// observed terminal state (the profile is not deleted: exit≠0/unknown is no deletion target). The old `fallback_fresh` (remove_session · marker reissue ·
    /// respawn) is gone — decided: kill nothing, create nothing new. `restore_one` and `activate_profile` reuse it **identically**.
    /// The return's second field = the incarnation this attempt **produced** (if any), so `activate_profile` does not flip a success when the lookup fails after
    /// it. `entry` is handed to the shared body as received.
    /// ★`Err` = the pre-launch roster save `Err`, not launched (only the caller entry)★ — do not fold into a `RestoreOutcome::Failed` text: `activate_profile`
    /// could then not return `PtyError::Store` and the bus could not tell a save refusal from other failures. Record and log stay as when folded.
    // ADR-0082
    // ADR-0172
    // ADR-0291 R17
    fn resume_no_fallback(
        &self,
        profile: &AgentProfile,
        entry: Entry,
    ) -> Result<(RestoreOutcome, Option<AgentInfo>), StoreError> {
        // ★A resume request without a handle **opens a new conversation** — consequence of D1 (「a stored id exists ⟺ there is a conversation to resume · no id → Fresh」)★. What reaches it is WS
        //   `SpawnProfile`'s explicit `resume: true` (it stays Resume even without a stored session). ★Handed entirely to the Fresh path, not launched as Resume with a changed report★: a stored-handle
        //   backend silently drops the resume flag without a handle, so Resume would report an unknown-id new conversation as 「resumed」; the outcome is `Started`, not `Resumed`.
        //   ★No clash with 「no new conversation」 (ADR-0082)★ — that guards requests **with a conversation to resume**; this is normal creation. This is the front door filtering the common case; the race where the
        //   field empties before spawn's handle read is refused by spawn before launching (same predicate). Backends not resuming with the stored handle (shell) are not caught and take the old way.
        // ADR-0226
        let opens_a_new_conversation = backend::can_resume_stored_session(&profile.command)
            && self
                .profiles
                .get(profile.id)
                .and_then(|p| p.backend_session_id)
                .is_none();
        if opens_a_new_conversation {
            // It is info because it is a normal lifecycle — the set outcome of a request with no conversation to resume (D1).
            tracing::info!(
                agent = %profile.id,
                "resume requested but there is no stored handle — opening a new conversation (the outcome is reported as `Started`, not `Resumed`)"
            );
            // The record is made inside the new-conversation path — writing here again would make two clearing points.
            return match self.spawn_fresh_settled(profile, entry) {
                Ok(outcome) => Ok((RestoreOutcome::Started, outcome.into_info())),
                Err(PtyError::Store(store)) => Err(store),
                Err(e) => Ok((
                    RestoreOutcome::Failed {
                        reason: format!("new conversation spawn failed: {e}"),
                    },
                    None,
                )),
            };
        }

        let attempt = self.spawn_agent_watching_link(profile, SpawnMode::Resume, entry);
        let (outcome, watch) = match attempt {
            Err(e) => {
                // ADR-0172: failure is recorded where attempted — the screen/API-side real thing of ADR-0082's "leave the cause for the control LLM to read" (logs are read only by people).
                self.note_activation_result(profile.id, None, Some(spawn_error_kind(&e)));
                // ★A save `Err` does not emit the warning below★ — it stopped before opening a process (no corpse), and the caller-error surface (bus · WS) leaves a line; emitting here makes two for one request.
                // ADR-0291 R17
                if let PtyError::Store(store) = e {
                    return Err(store);
                }
                let reason = format!("resume spawn failed: {e}");
                tracing::warn!(
                    agent = %profile.id,
                    %reason,
                    "ADR-0082: resume failed → terminal point (corpse), no fresh-fallback"
                );
                return Ok((RestoreOutcome::Failed { reason }, None));
            }
            Ok(pair) => pair,
        };
        let spawned = match outcome {
            // ★Nothing to do — no early-exit window to watch★: a session someone else launched (is launching) was not observed, so neither record nor clear
            //   (as `note_spawn_result`). Reported as 「it is up」 — the restore vocabulary has no 「nothing to do」.
            // ADR-0172
            SpawnOutcome::Moot(info) => {
                tracing::info!(
                    agent = %profile.id,
                    "resume: already up or launching — this request has nothing to do (moot)"
                );
                return Ok((RestoreOutcome::Resumed, info));
            }
            SpawnOutcome::Started(info) => info,
        };

        // ★fable M-1★: a successful claude resume is a TUI and does not exit within the window, so entering terminal in it is a resume failure whatever the code
        //   (a code==0 early exit judged Resumed would report an empty screen as "restore succeeded").
        // ★`spawned.epoch` is carried **out of** the window (late-write guard)★: the writes below happen up to EARLY_EXIT_WINDOW later with the reservation
        //   released; if another connection activated the profile meanwhile the epoch rose, so sending this value keeps the old observation from overwriting
        //   the new incarnation (`set_last_failure` contract).
        // ADR-0172
        // ★The decision path splits **by the link's axis**★: with a delivery channel it waits for the outcome, without one the old window decision (why not one
        //   loop: `link_activation_verdict`).
        let verdict = match watch.as_ref() {
            Some(w) => self.link_activation_verdict(
                profile.id,
                spawned.epoch,
                &w.rx,
                LINK_RESOLUTION_BACKSTOP,
            ),
            None => self.early_activation_verdict(
                profile.id,
                &profile.command,
                EARLY_EXIT_WINDOW,
                LINK_RESOLUTION_BACKSTOP,
            ),
        };
        Ok(match verdict {
            EarlyVerdict::Terminal { status, evidence } => {
                let reason = format!("resume exited early ({status:?})");
                // ★What the user interrupted is not an activation failure — not recorded★: a kill within the window (tree termination · WS `Kill` frame · daemon shutdown)
                //   is an explicit intervention; recording 「exited right after resuming」 would make an entry the user just turned off look broken. The result is still
                //   Failed (not up) and not cleared (no ground for success). Today the only kill surface is the WS frame — `agent.kill` does not exist (kill is
                //   intentionally absent from `agent.*`: `types::CLI_AGENT_VERBS` · ADR-0122 unresolved).
                // ADR-0172
                if matches!(status, AgentStatus::Killed) {
                    tracing::info!(
                        agent = %profile.id,
                        %reason,
                        "user exit within the resume window — not recorded as an activation failure"
                    );
                } else {
                    // If the kind cannot be recognized from the evidence, the contextual default — 「early exit right after resume」 (retryable).
                    let kind = backend::resume_failure_kind(&profile.command, &evidence)
                        .unwrap_or(AgentFailureKind::EarlyExitAfterResume);
                    self.note_activation_result(profile.id, Some(spawned.epoch), Some(kind));
                    tracing::warn!(
                        agent = %profile.id,
                        %reason,
                        ?kind,
                        "ADR-0082: resume early exit → terminal point (corpse), no fresh-fallback — a target for LLM escalation"
                    );
                }
                (RestoreOutcome::Failed { reason }, Some(spawned))
            }
            // ★Still alive but folded into failure — **nothing is killed**★: claude runs its exit hook a few more seconds after the diagnostic (+2.2s / +6.4s);
            //   waiting for the corpse would pass the window and decide success, so it settles here. Observation only — the child does EOF→finish and the reaper
            //   collects it (ADR-0082 "kill nothing"). ★「running + last failure」 for a few seconds is not a bug★: the axes are separate and the tree draws
            //   「running wins」 (ADR-0173).
            // ADR-0172
            EarlyVerdict::Diagnosed(kind) => {
                let reason = format!("resume failure diagnosed ({kind:?})");
                self.note_activation_result(profile.id, Some(spawned.epoch), Some(kind));
                tracing::warn!(
                    agent = %profile.id,
                    %reason,
                    ?kind,
                    "ADR-0172: the diagnostic stream settled the resume failure — not waiting for the exit"
                );
                (RestoreOutcome::Failed { reason }, Some(spawned))
            }
            // ★The process is alive but **unusable** — alive is not counted as success★. Without this branch (ADR-0082's feared outcome) a refused resume left the
            //   child fine, the window saw only `Running`, fell to `Alive`, **reported success and cleared the last failure**.
            // ★The evidence is `reason` alone★ — no corpse, console and diagnostic tails empty; passed as is as the classification input (backend knowledge — ADR-0004).
            // ★★Disposition **differs** from `Diagnosed` — this branch collects★★: the link only releases its stdin (ADR-0199's line) and if the peer keeps stdout the chain
            //   never runs, so `tear_down_failed_activation` below collects it with the existing 2 verbs (its doc: why the manager). It does not conflict with
            //   ADR-0082 「kill nothing」 (an observed corpse vs an incarnation whose terminal point never stood).
            EarlyVerdict::LinkFailed { reason } => {
                let kind = backend::resume_failure_kind(&profile.command, &reason)
                    .unwrap_or(AgentFailureKind::EarlyExitAfterResume);
                let reason = format!("resume connection failed: {reason}");
                // ★★Cleanup **first** — before the record★★: the record takes the profile mutex, which the session id record path may hold **across a disk write**; reversed,
                //   that stall would become 「never kills the child」 — the backstop exists for exactly that stall. (The old reason 「kill first sets `Killed`, indistinguishable
                //   from a user exit」 was wrong: that discipline lives in `early_activation_verdict`, already decided and returned.)
                self.tear_down_failed_activation(profile.id, spawned.epoch);
                self.note_activation_result(profile.id, Some(spawned.epoch), Some(kind));
                tracing::warn!(
                    agent = %profile.id,
                    %reason,
                    ?kind,
                    "ADR-0082: the link could not set up the connection → activation failed, no fresh-fallback — a target for LLM escalation"
                );
                (RestoreOutcome::Failed { reason }, Some(spawned))
            }
            // ★The other side **signaled ready** = holds★ — positive evidence, so this branch arrives **sooner** than `Alive` on **stronger grounds**.
            // ★★But 「the last failure」 is **not cleared**★★: this success is optimistic — it observes only that the agent can take input; what follows (turn errors ·
            //   connection drops) is **a different event on a different path**, and a success report erasing the failure evidence was half the original defect.
            // ★Nothing is cleared automatically★: the record answers 「how the last activation ended」; only **the next failure overwriting it**
            //   (`set_last_failure(Some(..))`) or **the profile disappearing** invalidates it. ★Cost★: a stale failure may sit next to a live agent — drawn with
            //   「running wins」 (ADR-0173); the opposite error (evidence loss) is irreversible.
            // ★claude does not come here★ (no connection axis → `Alive` below); do not widen the clearing rule to it.
            EarlyVerdict::Ready => {
                // A request without a handle already fell out to a new conversation at the head above, so what comes here is only resumes.
                (RestoreOutcome::Resumed, Some(spawned))
            }
            EarlyVerdict::Alive => {
                // ★Passed the early-exit window with silent diagnostics = the conversation to resume really existed★ — the ground of 「clearing」 (`note_activation_result`).
                // ★Only **links without a connection axis** reach here★ (claude · shell · stdio); connection-declaring links split at `Ready`/`LinkFailed` above.
                self.note_activation_result(profile.id, Some(spawned.epoch), None);
                (RestoreOutcome::Resumed, Some(spawned))
            }
        })
    }

    /// Ends a session whose activation is **settled as failed** — meaningful only while the process is still alive.
    ///
    /// ★Why the manager★: a refused resume makes the link release only its stdin and wait for the peer to end; if the peer keeps stdout, the reader never sees EOF,
    /// `OutputCore::finish` and the reaper never run, and the child, Job and threads stay for the daemon's lifetime. The owner is **the kill handler** (ADR-0001's
    /// 2 verbs), not the link — do not bring back the link writer's direct kill (crosses ADR-0199's line). This is an existing teardown, not a new verb.
    /// ★Not `kill_agent` as is★: it sets `TerminationIntent::UserKill`, but this is not the user turning it off; that marker is frozen into `ReapMsg`, so a false
    /// one falsifies the exit classification.
    /// ★No conflict with ADR-0082 「kill nothing」★: that guards an **observed terminal point (corpse)**; here the terminal point **never stood**, and killing
    /// **creates the exit transition** rather than erasing evidence (the failure reason is recorded **before** this call). So not called for `Terminal` ·
    /// `Diagnosed` (child already dead / claude in its exit hook).
    fn tear_down_failed_activation(&self, id: AgentId, incarnation: u32) {
        let Ok(session) = self.get_session(id) else {
            return;
        };
        // ★★A differing marker is **not touched**★★: this cleanup arrives as late as the decision and the incarnation may have been collected and **the next one launched**;
        //   killing by id alone would kill **the healthy successor** and `revoke` would erase its token. ★Match/mismatch only★ (ADR-0163/0164).
        if session.epoch != incarnation {
            tracing::info!(
                agent = %id,
                expected = incarnation,
                live = session.epoch,
                "skipping cleanup of the failed activation — another incarnation stood in between (not killing a live session)"
            );
            return;
        }
        // The retreat notice is the first step (reason: `kill_agent`). ★It must come **after** the marker comparison★ — before it, the live successor's link would receive an irreversible retreat marker.
        session.begin_retire();
        self.control.revoke(id, session.epoch);
        let _ = session.enter_exiting();
        session.kill(Duration::from_secs(5));
    }

    /// After spawn, decides the resume outcome within the window — **death or evidence** (`EarlyVerdict`).
    ///
    /// ★Evidence is not looked at before death (loop order — load-bearing)★: each poll checks state first, then diagnostics; reversed, a **user exit** inside the window
    /// would be caught as `Diagnosed`, bypassing "what the user interrupted is not a failure" (`resume_no_fallback`).
    /// ★Only the stderr stream settles by diagnostics — not the console ring★: on resume the ring receives **the redrawn conversation body**, so a conversation that once
    /// mentioned that text would **stamp a successful activation as failure** (this repository's sessions do). stderr is claude's own diagnostic channel; the console tail
    /// is evidence only **after death**.
    /// ★Take the session Arc **once outside the loop** (do not revert)★: `finish` wakes the reaper right after the terminal state and the reaper first removes the session
    /// from the roster, while the loop polls every 100ms — a per-iteration `get_session` misses the tail almost always and classification always falls to the contextual
    /// default. Holding the Arc keeps the same `OutputCore` readable after it leaves the roster.
    /// ★The first lookup failure is not "no session"★: the child may have died and been collected between `spawn_agent` returning and here; the tail is missed and the
    /// classification falls to the retryable contextual default (fail-open) — causes are not told apart.
    /// ★The two evidence kinds are exclusive, passed merged★: PTY sessions merge stderr into the console ring, pipe sessions keep it in the diagnostic buffer; both empty →
    /// contextual default. `command` is received because what the text means is backend knowledge (ADR-0004).
    // ADR-0172
    fn early_activation_verdict(
        &self,
        id: AgentId,
        command: &AgentCommand,
        window: Duration,
        // ★This branch is only for links without a connection axis, so it does not use the backstop★ — the argument keeps the two decisions' call shape the same; being unused is the contract.
        _link_backstop: Duration,
    ) -> EarlyVerdict {
        let session = match self.get_session(id) {
            Ok(s) => s,
            // Not in the roster = not on it yet or already collected → treated as exited.
            // ★This `AgentStatus::Failed` is **not published**★ — a local value never reaching `StatusSink`, used only in the reason string
            //   `format!("resume exited early ({status:?})")` and the `matches!(status, AgentStatus::Killed)` decision. "A published `Failed` is only for pump panics" holds;
            //   canonical: docs/reference/architecture.md 「Session restore · activation」.
            Err(_) => {
                return EarlyVerdict::Terminal {
                    status: AgentStatus::Failed {
                        message: "session gone".into(),
                    },
                    evidence: String::new(),
                }
            }
        };
        // ★Only links **without** a connection axis come here★ (claude · shell · stdio · codex terminal); the axis side waits for delivery in [`AgentManager::link_activation_verdict`]. **Byte for byte** as before
        //   round 2 (one window, outcome `Alive`).
        let deadline = Instant::now() + window;
        loop {
            let status = session.status();
            if matches!(
                status,
                AgentStatus::Exited { .. } | AgentStatus::Killed | AgentStatus::Failed { .. }
            ) {
                return EarlyVerdict::Terminal {
                    status,
                    evidence: Self::terminal_evidence(&session),
                };
            }
            if let Some(kind) = backend::resume_failure_kind(command, &session.diagnostic_tail()) {
                return EarlyVerdict::Diagnosed(kind);
            }
            if Instant::now() >= deadline {
                return EarlyVerdict::Alive;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    /// Moves the user's cancellation into the decision — ★`Killed` is **set here**★. ★Not published★: a local value never reaching `StatusSink`, used only in the caller's
    /// reason string and the `matches!(status, AgentStatus::Killed)` decision (same as `early_activation_verdict`'s `"session gone"`).
    /// ★It means 「the user decided to interrupt」, not 「already dead」★ (`session.kill()` may still be running); the caller only uses it to 「not record a failure」.
    /// ★The reason of the intercepted delivery is **not carried here — nowhere to carry it**★: `spawn_fresh_settled` · `resume_no_fallback` record nothing and do not read
    /// `evidence` in the `Killed` branch, and the text already remains (the link `tracing::warn!`s it in the failure branch — `writer_loop` of
    /// `backend/codex/transport.rs` — before `deliver_link`, not suppressed by the exit latch). ★Nor dump the tail to the log★: up to 4 KiB of possible conversation body
    /// per user turn-off, for nothing.
    fn cancelled_verdict(session: &AgentSession) -> EarlyVerdict {
        EarlyVerdict::Terminal {
            status: AgentStatus::Killed,
            evidence: Self::terminal_evidence(session),
        }
    }

    /// Terminal evidence — console tail + diagnostic (stderr) tail. The two streams fill exclusively depending on the link.
    fn terminal_evidence(session: &AgentSession) -> String {
        let tail = session.terminal_tail(FAILURE_TAIL_BYTES);
        let mut evidence = String::from_utf8_lossy(&tail).into_owned();
        let diagnostics = session.diagnostic_tail();
        if !diagnostics.is_empty() {
            evidence.push('\n');
            evidence.push_str(&diagnostics);
        }
        evidence
    }

    /// The activation decision of a link that sets up a connection — ★it does not **read** state but waits for the outcome to be **delivered**★.
    ///
    /// ★Why separate★: read-based and delivery-based decisions cannot share a loop — mixing revives 「the gap between reading and acting」 (the four defects this round
    /// closed) — and backends without the axis must not change by a line.
    /// It looks at four things — ★the first **before the other three**★: 1. **the user cancellation latch** (`kill_agent` sets it **before** bringing the link down;
    /// visible before delivery, channel loss and state — else cancellation masquerades as failure or success); 2. **the delivered outcome** (not withdrawn once sent;
    /// accepted only for this spawn's marker); 3. **the terminal state** (a latch, set once); 4. **the backstop** (liveness for paths where the link **cannot** produce an
    /// outcome — [`LINK_RESOLUTION_BACKSTOP`]).
    /// ★★The terminal **does not** beat delivery — only cancellation does★★: a delivered `Ready` means the peer **signaled** ready, and dying later is 「a separate event」
    /// (ADR-0201); the caller **re-takes the state** and returns it (`Ready` branch of `spawn_fresh_settled` · Resume branch of `activate_profile`). Cancellation is
    /// **a person deciding** the outcome.
    /// ★Accepts `epoch` **only on match**★ (why it never mismatches today and stays: `as_verdict`). ★Right before the backstop it **re-checks delivery**★ — on overlap delivery
    /// wins, else a connection that just held would be recorded as failure and die (defect ②).
    fn link_activation_verdict(
        &self,
        id: AgentId,
        epoch: u32,
        link_rx: &Receiver<LinkVerdict>,
        backstop: Duration,
    ) -> EarlyVerdict {
        /// Moves one delivery into a decision — a differing marker is **dropped** (another incarnation's outcome).
        ///
        /// ★This guard **does not fire in production today — kept knowingly**★: the mailbox is made per spawn ([`AgentManager::spawn_agent_watching_link`]) with the marker baked
        /// into the port closure, so every message in this `rx` carries the awaited marker; what blocks mismatches is **channel freshness**, not this `if`. It stays because a
        /// shared channel (one supervisor over several spawns, a reused port) would make it the only wall. Do not claim it prevents a collected incarnation's outcome reaching
        /// the successor — it only fires in the test rig (`a_verdict_from_another_incarnation_is_discarded`).
        fn as_verdict(v: LinkVerdict, epoch: u32) -> Option<EarlyVerdict> {
            if v.epoch != epoch {
                tracing::debug!(
                    delivered = v.epoch,
                    expected = epoch,
                    "another incarnation's connection outcome arrived — dropping it"
                );
                return None;
            }
            Some(match v.resolution {
                LinkResolution::Ready => EarlyVerdict::Ready,
                LinkResolution::Failed { reason } => EarlyVerdict::LinkFailed { reason },
            })
        }

        /// ★★Did the user interrupt this incarnation — **the latch looked at before delivery**★★.
        ///
        /// ★It cannot be seen through the terminal state★: `kill_agent` goes `set_intent(UserKill)` → `enter_exiting()` → `session.kill()` and `Killed` stands only when the
        /// pump wakes after; the in-between state is `Exiting` (**not terminal**), and a link disappearing then would be recorded by the `Disconnected` branch as 「connection
        /// failure」. The latch is **always visible before** those observations (`AgentSession::termination_intent`).
        fn user_cancelled(session: &AgentSession) -> bool {
            matches!(session.termination_intent(), TerminationIntent::UserKill)
        }

        let session = match self.get_session(id) {
            Ok(s) => s,
            Err(_) => {
                return EarlyVerdict::Terminal {
                    status: AgentStatus::Failed {
                        message: "session gone".into(),
                    },
                    evidence: String::new(),
                }
            }
        };
        let deadline = Instant::now() + backstop;
        loop {
            // ★The wait is sliced **to look at the terminal latch together**★ — delivery wakes it at once, so the slice adds no success latency.
            let slice = deadline
                .saturating_duration_since(Instant::now())
                .min(Duration::from_millis(100));
            match link_rx.recv_timeout(slice) {
                Ok(v) => {
                    // ★★User cancellation **beats delivery**★★: if the user interrupts right after the handshake and `Ready` is queued, taking that `Ready` would **report a dead agent as
                    //   success** (`restore_one` `Resumed`, `activate_profile` `Ok`); the latch first falls to 「what the user turned off」, so record and report stay honest.
                    // ★The dropped delivery's reason is **not carried**★ — the receiver does not read `evidence` in `Killed` and the link already logged it (`cancelled_verdict`).
                    if user_cancelled(&session) {
                        return Self::cancelled_verdict(&session);
                    }
                    if let Some(verdict) = as_verdict(v, epoch) {
                        return verdict;
                    }
                    continue;
                }
                Err(RecvTimeoutError::Disconnected) => {
                    // ★★The latch is looked at **before the state** — do not revert the order★★: a common cause of the link disappearing is **the user kill itself** — it suppresses delivery,
                    //   ends the writer, the sending end drops and this branch wakes while `session.kill()` is still in `child.kill()`/`wait()` and the state is `Exiting` (**not terminal**).
                    //   State-only reading recorded **the user's cancellation as a resume failure**, what the suppression meant to prevent.
                    // ★No grace period, intended★: on the kill path the latch stands **before** the link disappears, and for a non-kill disappearance a grace period gives the same failure.
                    if user_cancelled(&session) {
                        return Self::cancelled_verdict(&session);
                    }
                    // The link disappeared without delivery — the only fact left is the terminal state.
                    let status = session.status();
                    if matches!(
                        status,
                        AgentStatus::Exited { .. }
                            | AgentStatus::Killed
                            | AgentStatus::Failed { .. }
                    ) {
                        return EarlyVerdict::Terminal {
                            status,
                            evidence: Self::terminal_evidence(&session),
                        };
                    }
                    return EarlyVerdict::LinkFailed {
                        reason: "the link disappeared without delivering the connection outcome"
                            .to_string(),
                    };
                }
                Err(RecvTimeoutError::Timeout) => {}
            }

            // ★A user kill also comes here★ — the same discipline as the two branches above, and it does not wait for the terminal transition.
            if user_cancelled(&session) {
                return Self::cancelled_verdict(&session);
            }

            // ★The terminal is a latch, so reading it is safe★ — `OutputCore` sets it only once and does not revert it.
            let status = session.status();
            if matches!(
                status,
                AgentStatus::Exited { .. } | AgentStatus::Killed | AgentStatus::Failed { .. }
            ) {
                let mut evidence = Self::terminal_evidence(&session);
                // Even if the terminal outran delivery, the reason is kept — that text is in neither tail.
                if let Ok(v) = link_rx.try_recv() {
                    if v.epoch == epoch {
                        if let LinkResolution::Failed { reason } = v.resolution {
                            if !evidence.is_empty() {
                                evidence.push('\n');
                            }
                            evidence.push_str(&reason);
                        }
                    }
                }
                return EarlyVerdict::Terminal { status, evidence };
            }

            if Instant::now() >= deadline {
                // ★Before giving up, look at the mailbox **one last time**★ — defect ② closes here.
                if let Ok(v) = link_rx.try_recv() {
                    if let Some(verdict) = as_verdict(v, epoch) {
                        return verdict;
                    }
                }
                return EarlyVerdict::LinkFailed {
                    reason: format!(
                        "the link did not produce the connection outcome within {} seconds — the other side neither answers nor accepts our writes (the handshake cap applies only to waiting for responses)",
                        backstop.as_secs()
                    ),
                };
            }
        }
    }

    // ── Subscribe/IO ────────────────────────────────────────────────────────

    pub fn subscribe(
        &self,
        agent_id: AgentId,
        sink: Arc<dyn OutputSink>,
    ) -> Result<SinkId, PtyError> {
        let session = self.get_session(agent_id)?;
        Ok(session.subscribe(sink))
    }

    /// `requested_epoch` = the incarnation marker the subscriber last saw; the session compares it with its own.
    /// ★One session lookup — do not add a second★: the response's incarnation marker, resume marker and replay must come from the same session.
    ///
    /// ## ★Contract: `Err` ⟹ `on_ready` is never called (load-bearing — breaking it kills output)★
    /// The only failure is **the one session lookup**, structurally *before* `on_ready`. The daemon turns `Err` into `AgentEvent::SubscribeFailed`, advertising "neither
    /// `SubscribeAck` (= `on_ready`) nor `ReplayComplete` follows", and the client (src-tauri) releases its single-flight slot **immediately**. A failure branch added
    /// *after* `on_ready` would send refusal and Ack together and a late Ack/Complete would arrive over the released slot, attaching **a success marker to a generation
    /// where the replay did not run** (gen fence collapse). A new failure branch goes before `on_ready`, or fix the refusal notice contract together. Regression net =
    /// `subscribe_from_err_never_invokes_on_ready`.
    // ADR-0226
    pub fn subscribe_from(
        &self,
        agent_id: AgentId,
        sink: Arc<dyn OutputSink>,
        after_seq: Option<u64>,
        requested_epoch: Option<u32>,
        on_ready: impl FnOnce(&SubscribeReply),
    ) -> Result<SubscribeReply, PtyError> {
        let session = self.get_session(agent_id)?;
        Ok(session.subscribe_from(sink, after_seq, requested_epoch, on_ready))
    }

    pub fn unsubscribe(&self, agent_id: AgentId, sink_id: SinkId) -> Result<(), PtyError> {
        let session = self.get_session(agent_id)?;
        session.unsubscribe(sink_id);
        Ok(())
    }

    /// Keystroke and turn writes carrying the origin — human input (WS `WriteStdin`) is `User`, non-human callers `Mail`. ★The `*_observed` verbs take no origin — all `Mail`★ (the mail delivery path).
    // ADR-0231
    pub fn write_stdin(
        &self,
        agent_id: AgentId,
        data: &[u8],
        origin: InputOrigin,
    ) -> Result<(), PtyError> {
        self.get_session(agent_id)?
            .write_input_from(data, origin)
            .map(|_| ())
    }

    pub fn write_stdin_observed(
        &self,
        agent_id: AgentId,
        data: &[u8],
    ) -> Result<crate::types::WriteOutcome, PtyError> {
        self.get_session(agent_id)?.write_input_observed(data)
    }

    /// The **submit-including** version of `write_stdin_observed` — if the backend requires submit bytes (terminal) they go out again as a separate write after the body
    /// (`AgentSession::submit_input_observed`). ★Which to call★: this for "one completed message = one turn" (mail delivery); `write_stdin`/`write_stdin_observed` for
    /// keystroke streaming where a person types Enter.
    pub fn submit_stdin_observed(
        &self,
        agent_id: AgentId,
        data: &[u8],
    ) -> Result<crate::types::WriteOutcome, PtyError> {
        self.get_session(agent_id)?.submit_input_observed(data)
    }

    /// ★Incarnation-conditional write★ — writes only when `expected_epoch` equals the epoch of the session the AgentId points at **now**; otherwise it touches nothing and
    /// returns `Err`.
    ///
    /// ★Why (check-then-write TOCTOU — load-bearing)★: deciding the recipient by `(id, epoch)` then calling `write_stdin_observed(id, ..)` lets a restart in between land the
    /// write on **the new incarnation**; the window closes only by pulling the decision into **the same unit as the write**.
    /// ★★No consumers (since ADR-0111 decision 6)★★: the only caller was the daemon messaging group broadcast ("only to the incarnation alive at sending"), **abolished** —
    /// parked mail also goes to a new incarnation of the same name. ★Kept · revival condition★: if "only the incarnation at sending" returns it is to be added as a v2
    /// **personal mail option** (spec §8), needing exactly this write; reviving it as a group-only rule without reopening violates ADR-0111.
    /// ★Why it really closes the window (ADR-0006)★: `get_session` clones the `Arc<AgentSession>` and **releases**; comparison and write use **the same Arc** and
    /// `AgentSession.epoch` is immutable, so `Ok` means "wrote to exactly the session with epoch == expected".
    /// ★Mismatch = `PtyError::Unsupported` (no dedicated variant)★: the caller only needs "could not be performed **on this target now**, nothing written"; the message carries
    /// requested/current epoch; a new variant would widen every caller's match.
    // ADR-0088
    // ADR-0111
    pub fn write_stdin_observed_if_epoch(
        &self,
        agent_id: AgentId,
        expected_epoch: u32,
        data: &[u8],
    ) -> Result<crate::types::WriteOutcome, PtyError> {
        let session = self.get_session(agent_id)?;
        // ★Zero side-effect guarantee★: a mismatch leaves **before** touching the transport, so the caller can settle this `Err` as "not sent" (reparking and skip decisions rely on it).
        if session.epoch != expected_epoch {
            return Err(PtyError::Unsupported(format!(
                "epoch mismatch: agent {agent_id} is now at epoch {}, caller required {expected_epoch} — nothing was written",
                session.epoch
            )));
        }
        session.write_input_observed(data)
    }

    /// The waiting list of the live incarnation — the bus `agent.listQueuedInputs` and the WS list query carry this value. `Err(NotFound)` = no live session (an asleep agent holds
    /// no list). ★Locks do not overlap★: registry (snapshot) → release → link (acceptance unknown) → turn observation table, one at a time.
    // ADR-0006
    // ADR-0231
    pub fn list_queued_inputs(&self, agent_id: AgentId) -> Result<QueuedListing, PtyError> {
        let session = self.get_session(agent_id)?;
        let (rows, as_of_seq) = session.list_queued_inputs();
        let stopped_after_error = self
            .turns
            .get(agent_id, session.epoch)
            .is_some_and(|observed| observed.last_end_failed);
        Ok(QueuedListing {
            rows,
            as_of_seq,
            epoch: session.epoch,
            stopped_after_error,
        })
    }

    /// Cancels one queued input of the live incarnation (outcome translation in `AgentSession::cancel_queued_input`). `NotFound` = no live session (an asleep agent has nothing to cancel).
    // ADR-0231
    pub fn cancel_queued_input(
        &self,
        agent_id: AgentId,
        input_id: &str,
    ) -> Result<CancelOutcome, CancelError> {
        self.get_session(agent_id)
            .map_err(|_| CancelError::NotFound)?
            .cancel_queued_input(input_id)
    }

    /// ★Harness-only session injection seam (ADR-0088 / ADR-0012)★ — registers a pre-assembled `AgentSession` (incl. a test transport) in the sessions map, to drive delivery-boundary
    /// observation tests **without binaries** (no PTY / claude). Gated by the `test-harness` feature, turned on only in the daemon's `[dev-dependencies]`; it must not compile into
    /// production (an arbitrary session bypasses spawn reservation, profile+epoch coordination, token issuing, pump/reaper wiring and tracker lifetime). Only `spawn_session` is the
    /// regular registration point.
    ///
    /// ★Invariants★: (a) no reaper — no pump, no finish hook, no `ReapMsg`; it stays in the map until the manager Drop (`kill_agent` does not remove it either; tests use a fresh manager).
    /// (b) ADR-0006 — takes the sessions write lock, inserts, releases, takes no internal lock. (c) profiles untouched (no auto_restore flip or persist). Reinjecting the same id replaces it.
    #[cfg(feature = "test-harness")]
    #[doc(hidden)]
    pub fn insert_test_session(&self, session: Arc<AgentSession>) {
        self.sessions
            .write()
            .expect("sessions poisoned")
            .insert(session.id, session);
    }

    /// ★Harness only★ — the pair of `insert_test_session`: builds an `OutputCore` **connected to this manager's notification path and turn observation table** (spawn_session's wiring).
    /// ★Why★: an `OutputCore::new` session's emits would not reach the manager's table, making gate/doorbell wiring tests "pass because there is no observation"; tests needing none skip it.
    /// ★The waiting-list table is registered here (the turn observation table is not)★ — the observation table accepts unregistered writes, the waiting-list table drops them; unregistered,
    /// this core's list would never be visible to the manager's mail busy.
    #[cfg(feature = "test-harness")]
    #[doc(hidden)]
    pub fn wired_test_core(
        &self,
        id: AgentId,
        epoch: u32,
        classify: crate::backend::TurnClassifier,
    ) -> Arc<OutputCore> {
        self.inputs_pending.register(id, epoch);
        Arc::new(
            OutputCore::new(
                id,
                epoch,
                self.status_sink.clone(),
                TurnWiring::new(self.turns.clone(), classify),
            )
            .with_queued(QueuedWiring {
                registry: Arc::new(QueuedInputs::new()),
                pending: self.inputs_pending.clone(),
            }),
        )
    }

    pub fn resize(&self, agent_id: AgentId, cols: u16, rows: u16) -> Result<(), PtyError> {
        self.get_session(agent_id)?.resize(cols, rows)
    }

    pub fn interrupt(&self, agent_id: AgentId) -> Result<(), PtyError> {
        self.get_session(agent_id)?.interrupt()
    }

    // ── kill (LLD §6 absolute order) ───────────────────────────────────────────────

    /// Agent termination (delegated to the ADR-0019 reaper). **No map removal, disposition or notification here** — the reaper singly consumes the pump's ReapMsg, so the session may remain in the map right
    /// after return; a caller asserting "gone" must poll (headless tests do).
    pub fn kill_agent(&self, agent_id: AgentId) -> Result<(), PtyError> {
        let session = self.get_session(agent_id)?;
        // ★The retreat notice is the first step — before the permission revoke · intent · `Exiting` · `session.kill`★: it narrows the window where the link's background work overlaps the termination. It collects no
        //   resources, so the ADR-0001 causality is unchanged; the roster lock was released by `get_session` (ADR-0006).
        session.begin_retire();
        let epoch = session.epoch;

        // 0. ★Discard the control channel token immediately — **before** the blocking kill (FIX 4)★: after `session.kill` (up to a 5s join) the dying agent's token would stay valid for that window
        //    and it could issue commands (TOCTOU). No lock held (registry is a leaf lock, §10 · ADR-0006); revoke is idempotent, so overlap with the reaper's terminal revoke is harmless (backstop).
        // ADR-0086
        self.control.revoke(agent_id, epoch);

        session.set_intent(TerminationIntent::UserKill);

        let _ = session.enter_exiting();

        // 1~6. ★The revoke placement does not touch this causality (ADR-0001)★ — it touches only registry/files, not the shutdown chain, so it neither blocks nor reorders the kill; a join timeout just proceeds.
        session.kill(Duration::from_secs(5));

        // 7. Release session tracking (S9 — prevents zombie watcher entries). The reaper does not know the tracker.
        self.tracker.unwatch(agent_id);

        Ok(())
    }

    // ── Lookup/termination ─────────────────────────────────────────────────────────────

    pub fn list_agents(&self) -> Vec<AgentInfo> {
        let sessions: Vec<Arc<AgentSession>> = {
            let guard = self.sessions.read().expect("sessions poisoned");
            guard.values().cloned().collect()
        };
        sessions.iter().map(|s| self.agent_info(s)).collect()
    }

    pub fn get_snapshot(&self, agent_id: AgentId) -> Result<Vec<OutputChunk>, PtyError> {
        let session = self.get_session(agent_id)?;
        Ok(session.snapshot())
    }

    /// A lightweight sibling looking only at the epoch (no full `list_agents` / AgentInfo assembly under the profiles lock). ★No production caller★ — the subscription response marker comes from the
    /// `subscribe_from` reply (ADR-0226); the remaining consumer is the daemon integration test (`tests/ws_e2e.rs`).
    pub fn agent_epoch(&self, agent_id: AgentId) -> Option<u32> {
        self.sessions
            .read()
            .expect("sessions poisoned")
            .get(&agent_id)
            .map(|s| s.epoch)
    }

    pub fn shutdown_all(&self) {
        // ★ADR-0019★: set shutting_down **before** each kill — later, a session exiting in the gap would snapshot shutting_down=false in its finish hook and get KeepDisableAutoRestore
        //   (dropped from boot restore). Set first, every later finish snapshots true → the reaper does KeepAsIs.
        self.shutting_down.store(true, Ordering::SeqCst);

        // S9: stop the session tracking thread first (so polling does not touch sessions being cleaned up).
        self.tracker.stop();

        let ids: Vec<AgentId> = {
            let guard = self.sessions.read().expect("sessions poisoned");
            guard.keys().copied().collect()
        };
        std::thread::scope(|s| {
            for id in ids {
                s.spawn(move || {
                    let _ = self.kill_agent(id);
                });
            }
        });
    }

    // ── Internal helpers ─────────────────────────────────────────────

    /// §10 rule 1 — releases the read lock immediately and returns an Arc clone (the caller continues without holding the lock).
    fn get_session(&self, agent_id: AgentId) -> Result<Arc<AgentSession>, PtyError> {
        self.sessions
            .read()
            .expect("sessions poisoned")
            .get(&agent_id)
            .cloned()
            .ok_or(PtyError::NotFound(agent_id))
    }

    /// Used by `activate_profile` for the live session's info after a resume succeeds — `resume_no_fallback` returns only `RestoreOutcome`, so it looks up again by id.
    fn agent_info_by_id(&self, id: AgentId) -> Result<AgentInfo, PtyError> {
        let session = self.get_session(id)?;
        Ok(self.agent_info(&session))
    }

    /// The single source for callers not needing the whole AgentInfo (e.g. the envelope sender) to get a name **byte-identical to agent_info** — the session.cwd-based resolve lives here once.
    pub fn canonical_name(&self, id: AgentId) -> Option<String> {
        let session = self.get_session(id).ok()?;
        Some(self.resolve_canonical_name(&session))
    }

    /// The shared core of agent_info · canonical_name — one place for name derivation so it does not diverge from reaper/ingress/cli.
    ///
    /// ADR-0101 (WYSIWYA): AgentInfo.name is "the name a person sees in the tree"; routing (resolve_recipient) · roster · envelope sender · frontend tree must use **the same string**.
    /// ★cwd source = session.cwd (not profile.cwd)★: the tree draws `display_name ?? basename(AgentInfo.cwd)` and AgentInfo.cwd = session.cwd (canonicalized at spawn), while profile.cwd is
    /// raw ("." · ".." · symlinks) and would split basename from the routing address.
    // ADR-0101
    fn resolve_canonical_name(&self, session: &Arc<AgentSession>) -> String {
        let cwd = session.cwd.to_string_lossy();
        // get() takes the profiles lock, clones and releases — this function is called without the sessions lock held, so the two are never held together (§10 lock order).
        let display_name = self.profiles.get(session.id).and_then(|p| p.display_name);
        crate::name::canonical_name_or_id_fallback(display_name.as_deref(), &cwd, session.id)
    }

    /// Called only without holding the sessions lock.
    fn agent_info(&self, session: &Arc<AgentSession>) -> AgentInfo {
        // get() takes the profiles lock, clones and releases — called without the sessions lock held, so the two are never held together (§10 lock order).
        let display_name = self.profiles.get(session.id).and_then(|p| p.display_name);
        self.agent_info_with(session, display_name.as_deref())
    }

    /// The **display-name-injected form** of `agent_info` — lets a caller already holding the profile snapshot (`roster`) skip a per-session profile fetch. The derivation rule exists only here
    /// (`canonical_name_or_id_fallback`, as `resolve_canonical_name`).
    fn agent_info_with(
        &self,
        session: &Arc<AgentSession>,
        display_name: Option<&str>,
    ) -> AgentInfo {
        let cwd = session.cwd.to_string_lossy().to_string();
        let name = crate::name::canonical_name_or_id_fallback(display_name, &cwd, session.id);
        AgentInfo {
            id: session.id,
            name,
            cwd,
            status: session.status(),
            cols: session.cols.load(Ordering::Relaxed),
            rows: session.rows.load(Ordering::Relaxed),
            epoch: session.epoch,
            capabilities: session.capabilities(),
            // Drawn from the same session Arc — so consumers do not have to ask back later (that field's doc).
            reads_messages: session.reads_messages(),
        }
    }
}

impl Drop for AgentManager {
    /// ★Why an explicit Stop★: dropping reaper_tx closes the channel, but the sessions' finish hook closures hold reaper_tx clones, so recv may not end at once. A send failure (reaper already ended) is ignored.
    fn drop(&mut self) {
        let _ = self.reaper_tx.send(ReaperCmd::Stop);
        if let Some(handle) = self.reaper_handle.take() {
            let _ = handle.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    fn probe_spec() -> CommandSpec {
        CommandSpec {
            program: "cmd.exe".into(),
            args: vec!["/c".into(), "echo select-probe".into()],
            env: vec![],
            cwd: std::path::PathBuf::from("."),
        }
    }

    // ── ADR-0185: the record verb for 「the fetched session id」 ────────────────────────────────────────────
    //
    // Measured here: the one verb `session_id_sink` builds; that it is carried on spawn is measured by `backend/codex/mod.rs`'s app-server end-to-end entry.

    /// ★The store being **memory** is intended★ — these entries assert only the registry's decision; file storage (as `bare_manager`) leaves a temp folder per call (9 piled up in one regression run).
    fn sink_fixture() -> (Arc<crate::profile::ProfileRegistry>, AgentId, u32) {
        #[derive(Default)]
        struct MemStore(Mutex<Vec<AgentProfile>>);
        impl crate::profile::ProfileStore for MemStore {
            fn save(&self, profiles: &[AgentProfile]) -> Result<(), crate::profile::StoreError> {
                *self.0.lock().expect("mem store poisoned") = profiles.to_vec();
                Ok(())
            }
            fn load(&self) -> Vec<AgentProfile> {
                self.0.lock().expect("mem store poisoned").clone()
            }
        }
        let profiles = Arc::new(crate::profile::ProfileRegistry::new(Arc::new(
            MemStore::default(),
        )));
        let p = AgentProfile::new(
            "raw".into(),
            AgentCommand::Codex {
                extra_args: vec![],
                output_format: crate::profile::AgentOutputFormat::StreamJson,
            },
            std::path::PathBuf::from("."),
            vec![],
            true,
        );
        let id = p.id;
        profiles.upsert(p);
        let epoch = profiles.epoch_for_spawn(id).expect("a profile just put in");
        (profiles, id, epoch)
    }

    // ── The Fresh spawn returning its handle (ADR-0185) ──────────────────────────────────────

    use uuid::Uuid;

    fn codex(output_format: crate::profile::AgentOutputFormat) -> AgentCommand {
        AgentCommand::Codex {
            extra_args: vec![],
            output_format,
        }
    }

    /// ★Decision table — split by **reading** the backend declarations (not a literal copy)★. Not measured: that this predicate is carried on `spawn_agent` (needs a real process).
    #[test]
    fn fresh_spawn_releases_the_handle_only_where_something_can_refill_it() {
        use crate::profile::AgentOutputFormat as Fmt;
        let shell = AgentCommand::Shell {
            program: "cmd.exe".into(),
            args: vec![],
        };
        let claude = AgentCommand::Claude {
            extra_args: vec![],
            output_format: Fmt::Terminal,
        };

        // The two shapes with the issuing axis off and a report path — they clear.
        for c in [codex(Fmt::Terminal), codex(Fmt::StreamJson)] {
            assert!(
                fresh_spawn_release_session_id(&c, SpawnMode::Fresh),
                "{c:?}: Fresh must start from an empty field"
            );
            // ★Resume **never** clears★ — that field is the handle to resume (ADR-0083).
            assert!(
                !fresh_spawn_release_session_id(&c, SpawnMode::Resume),
                "{c:?}: Resume erased the handle"
            );
        }

        // There is no report path at all — clearing would make a hand-planted value disappear with nobody filling it.
        assert!(!fresh_spawn_release_session_id(&shell, SpawnMode::Fresh));
        assert!(!fresh_spawn_release_session_id(&shell, SpawnMode::Resume));

        // The side we issue for — the new value is written only at the first submission, so Fresh also starts from an empty field (ADR-0226).
        assert!(fresh_spawn_release_session_id(&claude, SpawnMode::Fresh));
        assert!(!fresh_spawn_release_session_id(&claude, SpawnMode::Resume));
    }

    /// The cleared field is filled right away by the commit port — the value the Fresh commit compares against is the `None` after clearing.
    #[test]
    fn releasing_the_handle_does_not_disturb_a_transport_that_refills_it() {
        let (profiles, id, epoch) = sink_fixture();
        profiles.observe_session_id(id, Some(epoch), Uuid::new_v4());
        let next_epoch = profiles.epoch_for_spawn(id).expect("profile");
        assert_eq!(profiles.release_session_id(id), Some(true));

        // The link writes in that place the value it received in the handshake response (the same verb as production).
        let from_transport = Uuid::new_v4();
        session_id_sink(profiles.clone(), id, next_epoch, None)(&from_transport.to_string());

        assert_eq!(
            profiles.get(id).and_then(|p| p.backend_session_id),
            Some(from_transport)
        );
    }

    /// ★The commit port is compare-and-swap — it does not rewind a field another writer changed before the first submission (ADR-0226)★: with Y written by the claude file watcher, a
    /// commit with the start value S must be refused (an overwrite would rewind Y to S and push Y into history).
    #[test]
    fn the_commit_port_does_not_rewind_a_value_another_writer_set() {
        let (profiles, id, epoch) = sink_fixture();
        let started_with = Uuid::new_v4();
        assert!(profiles.observe_session_id(id, Some(epoch), started_with));
        let tracker_wrote = Uuid::new_v4();
        assert!(profiles.observe_session_id(id, None, tracker_wrote));
        let history = profiles.get(id).expect("profile").old_session_ids;

        session_id_sink(profiles.clone(), id, epoch, Some(started_with))(&started_with.to_string());

        let p = profiles.get(id).expect("profile");
        assert_eq!(
            p.backend_session_id,
            Some(tracker_wrote),
            "the port rewound another writer's value to the start value — it changed to an overwrite verb"
        );
        assert_eq!(
            p.old_session_ids, history,
            "a refusal but the history moved"
        );
    }

    /// (f) ★The first echo of a resumed incarnation has nothing to write★ — if the handshake id equals the stored value, neither field nor history moves even if the link calls the first-turn port and the
    ///   commit runs (compare-and-swap's 「already the same value」 · ADR-0226 amendment).
    #[test]
    fn a_resumed_thread_s_first_echo_changes_nothing() {
        let (profiles, id, epoch) = sink_fixture();
        let stored = Uuid::new_v4();
        assert!(profiles.observe_session_id(id, Some(epoch), stored));
        let history = profiles.get(id).expect("profile").old_session_ids;
        let latch = SessionIdLatch::new(
            id,
            epoch,
            session_id_sink(profiles.clone(), id, epoch, Some(stored)),
        );

        (latch.offer_sink())(&stored.to_string());
        (latch.first_turn_sink())();

        let p = profiles.get(id).expect("profile");
        assert_eq!(p.backend_session_id, Some(stored));
        assert_eq!(
            p.old_session_ids, history,
            "the same value but the history moved"
        );
    }

    /// ★Our sid goes out only to issuing-axis backends (ADR-0185 · ADR-0226)★, measured by **reading** the declaration table: a value sent to codex would enter the latch, be dropped from
    /// argv and the real id rejected — every later resume silently fails. Not even **with** a stored value (a resume-axis decision is caught here).
    #[test]
    fn only_a_backend_that_assigns_ids_gets_one_handed_over() {
        use crate::profile::AgentOutputFormat as Fmt;
        let stored = Some(Uuid::new_v4());
        let shell = AgentCommand::Shell {
            program: "cmd.exe".into(),
            args: vec![],
        };
        for c in [codex(Fmt::Terminal), codex(Fmt::StreamJson), shell] {
            for mode in [SpawnMode::Fresh, SpawnMode::Resume] {
                assert_eq!(
                    session_id_to_hand_over(&c, mode, stored),
                    None,
                    "{c:?} {mode:?}: a sid went out to a backend we do not issue for"
                );
            }
        }

        for format in [Fmt::Terminal, Fmt::StreamJson] {
            let claude = AgentCommand::Claude {
                extra_args: vec![],
                output_format: format,
            };
            assert_eq!(
                session_id_to_hand_over(&claude, SpawnMode::Resume, stored),
                stored,
                "{claude:?}: Resume hands over the stored value read"
            );
            let minted = session_id_to_hand_over(&claude, SpawnMode::Fresh, stored);
            assert!(
                minted.is_some() && minted != stored,
                "{claude:?}: Fresh must be a newly drawn value (no reuse of the stored value — ADR-0076): {minted:?}"
            );
        }
    }

    /// ★The defensive clause where the port moves `expected`★ — after a successful commit a second call would take the first value as 「seen at start」; the latch calls once per incarnation, so production never reaches it.
    #[test]
    fn a_commit_port_moves_its_expected_value_after_a_successful_commit() {
        let (profiles, id, epoch) = sink_fixture();
        let port = session_id_sink(profiles.clone(), id, epoch, None);
        let first = Uuid::new_v4();
        let second = Uuid::new_v4();

        port(&first.to_string());
        port(&second.to_string());

        let p = profiles.get(id).expect("profile");
        assert_eq!(p.backend_session_id, Some(second));
        assert_eq!(p.old_session_ids, vec![first]);
    }

    /// Clearing an empty field again is a no-op (it does not save either).
    #[test]
    fn releasing_an_empty_handle_changes_nothing() {
        let (profiles, id, _) = sink_fixture();
        assert_eq!(profiles.release_session_id(id), Some(false));
        assert_eq!(profiles.get(id).and_then(|p| p.backend_session_id), None);
        assert!(profiles
            .get(id)
            .expect("profile")
            .old_session_ids
            .is_empty());
    }

    /// ★Fresh **does not send the stored handle out to the backend**★ — it makes 「`resume_session_id` exists ⟺ resuming incarnation」 hold, which the codex terminal reclamation relies on (it
    /// decides fresh from the handle alone — ADR-0218 decision 5); a leak makes a resumed incarnation reclaim again or a new conversation skip reclaiming.
    #[test]
    fn fresh_never_carries_a_resume_handle() {
        let stored = uuid::Uuid::new_v4();
        assert_eq!(
            resume_handle_for(SpawnMode::Resume, Some(stored)),
            Some(stored)
        );
        assert_eq!(resume_handle_for(SpawnMode::Resume, None), None);
        assert_eq!(
            resume_handle_for(SpawnMode::Fresh, Some(stored)),
            None,
            "a handle was carried out on Fresh — the moment that combination becomes expressible, the two consumers' decisions split"
        );
        assert_eq!(resume_handle_for(SpawnMode::Fresh, None), None);
    }

    #[test]
    fn the_session_id_sink_records_the_id_it_is_handed() {
        let (profiles, id, epoch) = sink_fixture();
        let sid = uuid::Uuid::new_v4();

        session_id_sink(profiles.clone(), id, epoch, None)(&sid.to_string());

        assert_eq!(profiles.get(id).unwrap().backend_session_id, Some(sid));
    }

    /// ★A dead incarnation's late record cannot overwrite the live one's value★ — this verb holds its own spawn's marker, which goes stale when the next spawn draws a new one.
    #[test]
    fn a_dead_incarnations_sink_cannot_overwrite_a_live_value() {
        let (profiles, id, dead_epoch) = sink_fixture();
        let dead_sink = session_id_sink(profiles.clone(), id, dead_epoch, None);

        // A new incarnation stands and writes its own value.
        let live_epoch = profiles
            .epoch_for_spawn(id)
            .expect("the profile is unchanged");
        let live_sid = uuid::Uuid::new_v4();
        session_id_sink(profiles.clone(), id, live_epoch, None)(&live_sid.to_string());

        // The dead incarnation's link finally comes back with its own thread id.
        dead_sink(&uuid::Uuid::new_v4().to_string());

        assert_eq!(
            profiles.get(id).unwrap().backend_session_id,
            Some(live_sid),
            "the dead incarnation's record overwrote the live incarnation's value"
        );
    }

    /// A value that cannot be read as a uuid is **dropped** — a panic would end the daemon, being outside the marker ([`session_id_sink`] doc).
    #[test]
    fn a_non_uuid_session_id_is_dropped_without_panicking() {
        let (profiles, id, epoch) = sink_fixture();
        let sink = session_id_sink(profiles.clone(), id, epoch, None);

        sink("not-a-uuid");
        sink("");

        assert_eq!(
            profiles.get(id).unwrap().backend_session_id,
            None,
            "a value that failed decoding was written into the profile"
        );
    }

    /// ★Measures the **cost** of dropping — 「diverged」, not 「not written」★: with a stored handle **already present** and a non-uuid thread id from the peer, the link keeps sending with that string
    /// while the disk stays old — the next resume opens a thread this incarnation never talked in, silently. Pair: `backend/codex/mod.rs`
    /// `tests::a_resume_target_makes_the_handshake_issue_thread_resume` (its fake returns `resumed-<uuid>`). ★Not about flipping 「must drop」★ (reason: [`session_id_sink`]); it pins that the
    /// outcome **has a cost**, so silence is a decision, not a regression.
    #[test]
    fn a_non_uuid_thread_id_leaves_the_stored_one_diverged() {
        let (profiles, id, epoch) = sink_fixture();
        let stored = Uuid::new_v4();
        assert!(profiles.observe_session_id(id, Some(epoch), stored));

        session_id_sink(profiles.clone(), id, epoch, Some(stored))("resumed-abc");

        assert_eq!(
            profiles.get(id).unwrap().backend_session_id,
            Some(stored),
            "a value that failed decoding touched the stored handle — the decision to drop was broken"
        );
    }

    // ── ADR-0044/0191: what the link the backend handed over carries ──────────────────────────────
    //
    // ★Measured by the caps the link reports, not its **type**★ (the building code lives in `backend/<name>/`, ADR-0191). `probe_spec` launches an immediately-ending probe through that
    //   backend's link choice and looks only at which implementation was chosen (ADR-0012, no real claude).
    #[cfg(windows)]
    #[test]
    fn stream_json_backend_hands_over_structured_pipe() {
        let parts = backend::open_spawn(
            &claude_stream_json_command(),
            &probe_spec(),
            DEFAULT_COLS,
            DEFAULT_ROWS,
            None,
            None,
            None,
            None,
            None,
        )
        .expect("open_spawn");
        let caps = parts.transport.capabilities();
        assert!(
            caps.output.structured && !caps.output.terminal_bytes,
            "stream-json → structured pipe (structured output, not terminal bytes)"
        );
        assert!(!caps.control.resize, "a pipe cannot resize");
        parts.transport.shutdown();
    }

    // ── Regression ──
    #[cfg(windows)]
    #[test]
    fn terminal_backend_hands_over_pty() {
        let parts = backend::open_spawn(
            &claude_terminal_command(),
            &probe_spec(),
            DEFAULT_COLS,
            DEFAULT_ROWS,
            None,
            None,
            None,
            None,
            None,
        )
        .expect("open_spawn");
        let caps = parts.transport.capabilities();
        assert!(
            caps.output.terminal_bytes && !caps.output.structured,
            "terminal mode → PTY (terminal bytes, not structured)"
        );
        assert!(caps.control.resize, "PTY can resize");
        parts.transport.shutdown();
    }

    // ── write_stdin_observed_if_epoch ──
    //
    // ★The session is plugged into the map directly, no real spawn★: only "compares the mapped session's epoch with the requested one and executes/refuses the write" is verified (ADR-0012);
    //   in-crate, so the private `sessions` is reachable without `insert_test_session`.

    use crate::backend::InputEncoder;
    use crate::persistence::{FilePresetStore, FileProfileStore};
    use crate::transport::AgentTransport;
    use crate::types::{
        BackendCaps, ControlCaps, InputCaps, InputEvent, ModelCaps, OutputCaps, SessionCaps,
        TransportCaps,
    };

    struct RecordingTransport {
        written: Arc<Mutex<Vec<Vec<u8>>>>,
    }
    impl AgentTransport for RecordingTransport {
        fn start(&self, _core: Arc<OutputCore>) {}
        fn send_input(&self, input: InputEvent) -> Result<(), PtyError> {
            let InputEvent::Raw(bytes) = input;
            self.written.lock().expect("written poisoned").push(bytes);
            Ok(())
        }
        fn resize(&self, _c: u16, _r: u16) -> Result<(), PtyError> {
            Ok(())
        }
        fn interrupt(&self) -> Result<(), PtyError> {
            Ok(())
        }
        fn shutdown(&self) {}
        fn capabilities(&self) -> TransportCaps {
            TransportCaps {
                input: InputCaps {
                    raw: true,
                    message: false,
                    attachment: false,
                },
                output: OutputCaps {
                    terminal_bytes: false,
                    structured: true,
                    markdown: false,
                    tool_events: false,
                    usage: false,
                },
                control: ControlCaps {
                    resize: false,
                    interrupt: false,
                    cancel: false,
                    graceful_shutdown: false,
                },
            }
        }
    }

    /// A stand-in for a link that **must set up** a connection — `link_state` can be swapped from outside. Measured without a real codex (needs a binary + a refusing thread id; ADR-0012).
    struct LinkedTransport {
        /// How many times `shutdown()` was called — measures whether the teardown of a failed activation actually reaches the link.
        shutdowns: Arc<Mutex<usize>>,
    }
    impl AgentTransport for LinkedTransport {
        fn start(&self, _core: Arc<OutputCore>) {}
        fn send_input(&self, _input: InputEvent) -> Result<(), PtyError> {
            Ok(())
        }
        fn resize(&self, _c: u16, _r: u16) -> Result<(), PtyError> {
            Ok(())
        }
        fn interrupt(&self) -> Result<(), PtyError> {
            Ok(())
        }
        fn shutdown(&self) {
            *self.shutdowns.lock().expect("shutdowns poisoned") += 1;
        }
        fn capabilities(&self) -> TransportCaps {
            TransportCaps {
                input: InputCaps {
                    raw: false,
                    message: true,
                    attachment: false,
                },
                output: OutputCaps {
                    terminal_bytes: false,
                    structured: true,
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

    struct NoopStatus;
    impl StatusSink for NoopStatus {
        fn status_changed(&self, _id: AgentId, _s: AgentStatus, _e: u32) {}
        fn agent_list_updated(&self, _a: Vec<AgentInfo>) {}
    }

    fn bare_manager() -> AgentManager {
        bare_manager_with_control(Arc::new(NoopControlChannel))
    }

    fn bare_manager_with_control(control: Arc<dyn ControlChannel>) -> AgentManager {
        let tag = uuid::Uuid::new_v4();
        let profiles = Arc::new(crate::profile::ProfileRegistry::new(Arc::new(
            FileProfileStore::new(std::env::temp_dir().join(format!("engram-epoch-w-{tag}"))),
        )));
        let presets = Arc::new(PresetRegistry::new(Arc::new(FilePresetStore::new(
            std::env::temp_dir().join(format!("engram-epoch-w-preset-{tag}")),
        ))));
        let tracker = Arc::new(SessionTracker::new(
            crate::session_tracker::TrackerConfig {
                enabled: false,
                poll_interval: Duration::from_secs(1),
            },
            Arc::new(|_, _| {}),
        ));
        AgentManager::new_with_control(Arc::new(NoopStatus), profiles, presets, tracker, control)
    }

    /// Reinserting the same id = imitating a restart (incarnation replacement).
    fn put_session(manager: &AgentManager, id: AgentId, epoch: u32) -> Arc<Mutex<Vec<Vec<u8>>>> {
        let written = Arc::new(Mutex::new(Vec::new()));
        let core = Arc::new(OutputCore::new(
            id,
            epoch,
            Arc::new(NoopStatus),
            TurnWiring::detached(),
        ));
        let session = Arc::new(AgentSession::new(
            id,
            std::path::PathBuf::from("."),
            epoch,
            80,
            24,
            Arc::new(AtomicU8::new(0)),
            BackendCaps {
                session: SessionCaps {
                    resume: false,
                    snapshot: false,
                    cwd_env: false,
                },
                model: ModelCaps {
                    select: false,
                    temperature: false,
                    max_tokens: false,
                },
            },
            InputEncoder::Raw,
            true,
            core,
            Box::new(RecordingTransport {
                written: written.clone(),
            }),
        ));
        manager
            .sessions
            .write()
            .expect("sessions poisoned")
            .insert(id, session);
        written
    }

    /// Plugs one **live** (non-terminal) session into the roster and returns its `OutputCore`. `RecordingTransport` launches nothing, so it **never dies by itself** — needed to measure "it does not
    /// wait for death" (a real claude cannot hold that state deterministically; ADR-0012).
    // ADR-0172
    fn put_live_session(manager: &AgentManager, id: AgentId, epoch: u32) -> Arc<OutputCore> {
        let core = Arc::new(OutputCore::new(
            id,
            epoch,
            Arc::new(NoopStatus),
            TurnWiring::detached(),
        ));
        let session = Arc::new(AgentSession::new(
            id,
            std::path::PathBuf::from("."),
            epoch,
            80,
            24,
            Arc::new(AtomicU8::new(0)),
            BackendCaps {
                session: SessionCaps {
                    resume: true,
                    snapshot: false,
                    cwd_env: false,
                },
                model: ModelCaps {
                    select: false,
                    temperature: false,
                    max_tokens: false,
                },
            },
            InputEncoder::Raw,
            true,
            core.clone(),
            Box::new(RecordingTransport {
                written: Arc::new(Mutex::new(Vec::new())),
            }),
        ));
        manager
            .sessions
            .write()
            .expect("sessions poisoned")
            .insert(id, session);
        core
    }

    /// Plugs a live session carrying a connection-axis link into the roster — returns the core and the kill counter. ★The connection state is not passed in★ — the outcome is **delivered** and the rig holds
    /// the channel directly.
    fn put_session_with_link(
        manager: &AgentManager,
        id: AgentId,
        epoch: u32,
    ) -> (Arc<OutputCore>, Arc<Mutex<usize>>) {
        let core = Arc::new(OutputCore::new(
            id,
            epoch,
            Arc::new(NoopStatus),
            TurnWiring::detached(),
        ));
        let shutdowns = Arc::new(Mutex::new(0usize));
        let session = Arc::new(AgentSession::new(
            id,
            std::path::PathBuf::from("."),
            epoch,
            80,
            24,
            Arc::new(AtomicU8::new(0)),
            BackendCaps {
                session: SessionCaps {
                    resume: true,
                    snapshot: false,
                    cwd_env: false,
                },
                model: ModelCaps {
                    select: false,
                    temperature: false,
                    max_tokens: false,
                },
            },
            InputEncoder::Raw,
            true,
            core.clone(),
            Box::new(LinkedTransport {
                shutdowns: shutdowns.clone(),
            }),
        ));
        manager
            .sessions
            .write()
            .expect("sessions poisoned")
            .insert(id, session);
        (core, shutdowns)
    }

    fn link_epoch_fixture(manager: &AgentManager, epoch: u32) -> (AgentId, Arc<Mutex<usize>>) {
        let id = AgentId::new_v4();
        let (_core, kills) = put_session_with_link(manager, id, epoch);
        (id, kills)
    }

    /// ★A delivered `Ready` finishes the decision **on the spot**★ — no window wait. A large backstop is given and the elapsed time measured; a decision that waits for time again is caught here.
    #[test]
    fn a_delivered_ready_ends_the_verdict_at_once() {
        let manager = bare_manager();
        let (id, _kills) = link_epoch_fixture(&manager, 7);
        let (tx, rx) = std::sync::mpsc::channel();
        tx.send(LinkVerdict {
            epoch: 7,
            resolution: LinkResolution::Ready,
        })
        .expect("deliver");

        let started = Instant::now();
        let verdict = manager.link_activation_verdict(id, 7, &rx, Duration::from_secs(6));
        let elapsed = started.elapsed();

        assert!(
            matches!(verdict, EarlyVerdict::Ready),
            "the delivered ready signal was not read as success — got {verdict:?}"
        );
        assert!(
            elapsed < Duration::from_secs(1),
            "the decision waited ({elapsed:?}) with an outcome already delivered — the ground of success is the signal, not elapsed time"
        );
    }

    /// ★A delivered failure comes **carrying its reason**★ — that text is the only entry of the classification (it is in neither tail).
    #[test]
    fn a_delivered_failure_carries_the_reason_to_the_classifier() {
        let manager = bare_manager();
        let (id, _kills) = link_epoch_fixture(&manager, 3);
        let (tx, rx) = std::sync::mpsc::channel();
        tx.send(LinkVerdict {
            epoch: 3,
            resolution: LinkResolution::Failed {
                reason: "thread/resume: -32600 no rollout found for thread id".into(),
            },
        })
        .expect("deliver");

        let EarlyVerdict::LinkFailed { reason } =
            manager.link_activation_verdict(id, 3, &rx, Duration::from_secs(6))
        else {
            panic!("the delivered failure was not read as a failure");
        };
        assert_eq!(
            backend::resume_failure_kind(&codex_app_server_command(), &reason),
            Some(AgentFailureKind::NoConversationToResume),
            "the reason does not reach the classification — then the contextual default is stamped as the last failure instead of the cause"
        );
    }

    /// ★★Another incarnation's outcome is **dropped**★★ — the root of 「the cleanup of a collected incarnation kills the live successor」: without the marker the supervisor found it by id alone and an old
    /// outcome became the new session's decision. A mismatch is dropped silently and the decision goes to the backstop **as if nothing happened**.
    #[test]
    fn a_verdict_from_another_incarnation_is_discarded() {
        let manager = bare_manager();
        let (id, _kills) = link_epoch_fixture(&manager, 11);
        let (tx, rx) = std::sync::mpsc::channel();
        tx.send(LinkVerdict {
            epoch: 10,
            resolution: LinkResolution::Ready,
        })
        .expect("deliver");

        let verdict = manager.link_activation_verdict(id, 11, &rx, Duration::from_millis(200));
        assert!(
            matches!(verdict, EarlyVerdict::LinkFailed { .. }),
            "a dead incarnation's outcome was used as this incarnation's decision — the cleanup after it kills a live session: got {verdict:?}"
        );
    }

    /// ★A delivered value **does not lose to the time limit**★ — defect ②: even with the backstop already expired, a value in the channel is the decision (the old shape read state, then time, and
    /// **recorded a just-held connection as a failure**). The exact overlap cannot be made, so an expired limit stands in.
    #[test]
    fn a_delivered_resolution_outranks_an_expired_backstop() {
        let manager = bare_manager();
        let (id, _kills) = link_epoch_fixture(&manager, 5);
        let (tx, rx) = std::sync::mpsc::channel();
        tx.send(LinkVerdict {
            epoch: 5,
            resolution: LinkResolution::Ready,
        })
        .expect("deliver");

        let verdict = manager.link_activation_verdict(id, 5, &rx, Duration::ZERO);
        assert!(
            matches!(verdict, EarlyVerdict::Ready),
            "the time limit beat the delivery — a connection that just held is recorded as a failure and dies: got {verdict:?}"
        );
    }

    /// ★If nobody delivers, the backstop gives up★ — that outcome is not success.
    #[test]
    fn nothing_delivered_ends_at_the_backstop_as_a_failure() {
        let manager = bare_manager();
        let (id, _kills) = link_epoch_fixture(&manager, 1);
        let (_tx, rx) = std::sync::mpsc::channel();

        let verdict = manager.link_activation_verdict(id, 1, &rx, Duration::from_millis(150));
        assert!(
            matches!(verdict, EarlyVerdict::LinkFailed { .. }),
            "the connection produced no outcome but it fell to success — got {verdict:?}"
        );
    }

    /// ★A user kill reaches the terminal **without delivery** — not recorded as a resume failure★ (the link does not deliver the kill branch — `deliver_link`); measures the supervisor's terminal branch.
    #[test]
    fn a_user_kill_reaches_the_verdict_as_terminal_not_as_a_link_failure() {
        let manager = bare_manager();
        let id = AgentId::new_v4();
        let (core, _kills) = put_session_with_link(&manager, id, 2);
        let (_tx, rx) = std::sync::mpsc::channel();
        core.finish(crate::types::TerminalReason::Killed);

        let verdict = manager.link_activation_verdict(id, 2, &rx, Duration::from_secs(6));
        assert!(
            matches!(
                verdict,
                EarlyVerdict::Terminal {
                    status: AgentStatus::Killed,
                    ..
                }
            ),
            "a user exit was read as a connection failure — then the user's cancellation is recorded as a resume failure and cleanup runs once more on top of it: got {verdict:?}"
        );
    }

    /// ★★A user kill **suppresses** delivery and the channel closes — not read as a failure★★: `deliver_link` suppresses **the message** but not **the channel disappearing** (the writer ends, the sending end
    /// drops) while `session.kill()` is still in `child.kill()`/`wait()` and the state is `Exiting` (**not terminal**); state-only reading recorded the cancellation as a resume failure.
    /// ★Made **deterministically**★: set the intent latch, go `Exiting`, then drop the sending end — the real kill path's states.
    #[test]
    fn a_kill_that_suppresses_delivery_is_not_read_as_a_link_failure() {
        let manager = bare_manager();
        let id = AgentId::new_v4();
        let (_core, _kills) = put_session_with_link(&manager, id, 13);
        let session = manager
            .get_session(id)
            .expect("premise: the session is in the roster");

        // Exactly the two lines kill_agent passes before bringing the link down.
        session.set_intent(TerminationIntent::UserKill);
        let _ = session.enter_exiting();
        assert!(
            !matches!(
                session.status(),
                AgentStatus::Exited { .. } | AgentStatus::Killed | AgentStatus::Failed { .. }
            ),
            "premise: not terminal yet — that interval is exactly the window this entry measures"
        );

        // Suppressed delivery = the sending end drops without any message.
        let (tx, rx) = std::sync::mpsc::channel::<LinkVerdict>();
        drop(tx);

        let verdict = manager.link_activation_verdict(id, 13, &rx, Duration::from_secs(6));
        assert!(
            matches!(
                verdict,
                EarlyVerdict::Terminal {
                    status: AgentStatus::Killed,
                    ..
                }
            ),
            "a channel closed by the user's interrupt was read as a connection failure — then the user's cancellation is recorded as an activation failure and the suppression device does nothing: got {verdict:?}"
        );
    }

    /// ★★A queued `Ready` **cannot beat** the user's cancellation★★ (opposite sign of the sibling): else a dead agent is **reported as success** (`restore_one` `Resumed`, `activate_profile`
    /// `Ok(info)`). ★Not widened to 「the terminal beats delivery」★ — dying by itself is 「a separate event」 (ADR-0201); only **a person's decided outcome** wins.
    #[test]
    fn a_queued_ready_does_not_outrank_a_user_cancellation() {
        let manager = bare_manager();
        let id = AgentId::new_v4();
        let (_core, _kills) = put_session_with_link(&manager, id, 21);
        let session = manager
            .get_session(id)
            .expect("premise: the session is in the roster");

        let (tx, rx) = std::sync::mpsc::channel();
        tx.send(LinkVerdict {
            epoch: 21,
            resolution: LinkResolution::Ready,
        })
        .expect("deliver");
        // The user interrupts **after** the delivery stood in the queue.
        session.set_intent(TerminationIntent::UserKill);
        let _ = session.enter_exiting();

        let verdict = manager.link_activation_verdict(id, 21, &rx, Duration::from_secs(6));
        assert!(
            matches!(
                verdict,
                EarlyVerdict::Terminal {
                    status: AgentStatus::Killed,
                    ..
                }
            ),
            "a queued ready signal beat the user's cancellation — it reports an agent the user just turned off as success: got {verdict:?}"
        );
    }

    /// ★★Even if the cancellation intercepts the delivery, **its reason already remains in the link's log**★★ — the ground for not carrying it. Two properties: ① on entering the failure branch the reason is
    /// **logged** ② **before** `deliver_link` (after it, the reason vanishes when the exit latch suppresses delivery). ★Measured in the source★: the crate has no `tracing` collector, and what must be
    /// measured is **the order of two lines**.
    #[test]
    fn the_transport_logs_the_rejection_reason_before_it_may_be_suppressed() {
        // ★Comments are stripped before scanning — else **a comment line poses as a real call**★: that branch's comments quote the names they guard, so a `deliver_link(` mention before the log line would make this
        //   **green while measuring nothing** (two link-side order guards were empty this way). Another file's `writer_loop_code` is unreachable here, so the stripping is copied.
        let src = include_str!("backend/codex/transport.rs");
        let production = src.split("mod tests {").next().expect("production section");
        let failure_arm: String = production
            .split("fn writer_loop(")
            .nth(1)
            .expect("`writer_loop` body")
            .split("Err(failure) => {")
            .nth(1)
            .expect("handshake failure branch")
            .lines()
            .filter(|l| !l.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");

        let logged = failure_arm
            .find("tracing::warn!(\"{}: {reason}\", failure.headline());")
            .expect(
                "the link does not log the failure reason — the ground for the supervisor not carrying it disappears",
            );
        let delivered = failure_arm
            .find("deliver_link(")
            .expect("the failure branch does not deliver — this entry's premise is stale");
        assert!(
            logged < delivered,
            "the reason log is after the delivery — in the interleaving where the exit latch suppresses delivery, that text remains nowhere (what is suppressed is the delivery, not the log)"
        );
    }

    /// ★★A Fresh success returns **the state after waiting** — not the spawn-time snapshot★★: `info` is `Running` from right after launch; now stale by up to the backstop, so an agent dead in between would be told alive
    /// (WS `Spawned`) and **mail would go to a corpse**. The Resume sibling does the same in `activate_profile`. ★Measured in the source★: running needs a real codex handshake.
    #[test]
    fn a_fresh_success_reports_the_state_after_the_wait_not_the_spawn_snapshot() {
        let ready = fresh_arm("EarlyVerdict::Ready =>", "EarlyVerdict::LinkFailed");
        assert!(
            ready.contains("self.agent_info_by_id(profile.id)"),
            "the success branch returns the spawn-time snapshot as is — a `Running` stale by the backstop goes to the caller: {ready}"
        );
        assert!(
            ready.contains("AgentStatus::Exited { code: None }"),
            "it does not lower the state when the lookup missed — then a collected session is reported as alive: {ready}"
        );
    }

    /// ★★The cleanup of a failed activation reaches **only that incarnation**★★ — defect ①: it arrives as late as the decision, and without the marker it would kill **the healthy successor just launched**.
    #[test]
    fn a_stale_teardown_never_touches_a_live_successor() {
        let manager = bare_manager();
        let id = AgentId::new_v4();
        let (_core, kills) = put_session_with_link(&manager, id, 42);

        // The dead incarnation's (41) cleanup arrives late — what is in the roster now is 42.
        manager.tear_down_failed_activation(id, 41);
        assert_eq!(
            *kills.lock().expect("shutdowns poisoned"),
            0,
            "the dead incarnation's cleanup killed the live successor"
        );

        // Its own incarnation's cleanup runs as is (with only the assertion above, 「doing nothing」 would also pass).
        manager.tear_down_failed_activation(id, 42);
        assert_eq!(
            *kills.lock().expect("shutdowns poisoned"),
            1,
            "its own incarnation's cleanup did not run — the failed session remains with nobody collecting it"
        );
    }

    // ── The place of the retreat notice: the first step once ending is decided ──
    // One record in call order: control channel (permission revoke) · core status notification (`Exiting`) · link (notice · termination). The termination intent has no verb to write it, so the value at
    //   the notice moment is stamped separately.

    type CallLog = Arc<Mutex<Vec<&'static str>>>;

    struct LoggingControl(CallLog);
    impl ControlChannel for LoggingControl {
        fn provision(
            &self,
            _id: AgentId,
            _epoch: u32,
            _needs: crate::types::ControlChannelNeeds,
        ) -> Result<Option<crate::types::ControlEndpoint>, crate::types::ProvisionError> {
            Ok(None)
        }
        fn revoke(&self, _id: AgentId, _epoch: u32) {
            self.0.lock().expect("log poisoned").push("revoke");
        }
    }

    struct LoggingStatus(CallLog);
    impl StatusSink for LoggingStatus {
        fn status_changed(&self, _id: AgentId, s: AgentStatus, _e: u32) {
            if matches!(s, AgentStatus::Exiting) {
                self.0.lock().expect("log poisoned").push("exiting");
            }
        }
        fn agent_list_updated(&self, _a: Vec<AgentInfo>) {}
    }

    struct RetireProbe {
        log: CallLog,
        intent: Arc<AtomicU8>,
        intent_at_retire: Arc<Mutex<Option<u8>>>,
    }
    impl AgentTransport for RetireProbe {
        fn start(&self, _core: Arc<OutputCore>) {}
        fn send_input(&self, _input: InputEvent) -> Result<(), PtyError> {
            Ok(())
        }
        fn resize(&self, _c: u16, _r: u16) -> Result<(), PtyError> {
            Ok(())
        }
        fn interrupt(&self) -> Result<(), PtyError> {
            Ok(())
        }
        fn begin_retire(&self) {
            *self.intent_at_retire.lock().expect("intent poisoned") =
                Some(self.intent.load(Ordering::SeqCst));
            self.log.lock().expect("log poisoned").push("begin_retire");
        }
        fn shutdown(&self) {
            self.log.lock().expect("log poisoned").push("shutdown");
        }
        fn capabilities(&self) -> TransportCaps {
            TransportCaps {
                input: InputCaps {
                    raw: true,
                    message: false,
                    attachment: false,
                },
                output: OutputCaps {
                    terminal_bytes: false,
                    structured: true,
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

    /// Plugs one `epoch` incarnation into the roster — the core's status notification and the link write to `log`. Returns = the termination intent at the moment of the notice.
    fn put_retire_probe(
        manager: &AgentManager,
        id: AgentId,
        epoch: u32,
        log: &CallLog,
    ) -> Arc<Mutex<Option<u8>>> {
        let intent = Arc::new(AtomicU8::new(TerminationIntent::None as u8));
        let intent_at_retire = Arc::new(Mutex::new(None));
        let core = Arc::new(OutputCore::new(
            id,
            epoch,
            Arc::new(LoggingStatus(log.clone())),
            TurnWiring::detached(),
        ));
        let session = Arc::new(AgentSession::new(
            id,
            std::path::PathBuf::from("."),
            epoch,
            80,
            24,
            intent.clone(),
            BackendCaps {
                session: SessionCaps {
                    resume: false,
                    snapshot: false,
                    cwd_env: false,
                },
                model: ModelCaps {
                    select: false,
                    temperature: false,
                    max_tokens: false,
                },
            },
            InputEncoder::Raw,
            true,
            core,
            Box::new(RetireProbe {
                log: log.clone(),
                intent,
                intent_at_retire: intent_at_retire.clone(),
            }),
        ));
        manager
            .sessions
            .write()
            .expect("sessions poisoned")
            .insert(id, session);
        intent_at_retire
    }

    #[test]
    fn a_kill_announces_retirement_before_revocation_intent_exiting_and_shutdown() {
        let log: CallLog = Arc::new(Mutex::new(Vec::new()));
        let manager = bare_manager_with_control(Arc::new(LoggingControl(log.clone())));
        let id = AgentId::new_v4();
        let intent_at_retire = put_retire_probe(&manager, id, 7, &log);

        manager.kill_agent(id).expect("a session in the roster");

        assert_eq!(
            *log.lock().expect("log poisoned"),
            vec!["begin_retire", "revoke", "exiting", "shutdown"],
            "the retreat notice does not come before the permission revoke · `Exiting` · termination"
        );
        assert_eq!(
            *intent_at_retire.lock().expect("intent poisoned"),
            Some(TerminationIntent::None as u8),
            "at the notice the user termination intent already stands — the notice was pushed after the intent"
        );
    }

    #[test]
    fn a_failed_activation_teardown_announces_retirement_first_and_only_to_its_own_incarnation() {
        let log: CallLog = Arc::new(Mutex::new(Vec::new()));
        let manager = bare_manager_with_control(Arc::new(LoggingControl(log.clone())));
        let id = AgentId::new_v4();
        let _ = put_retire_probe(&manager, id, 42, &log);

        manager.tear_down_failed_activation(id, 41);
        assert!(
            log.lock().expect("log poisoned").is_empty(),
            "the dead incarnation's cleanup gave the live successor a retreat notice — that marker cannot be undone"
        );

        manager.tear_down_failed_activation(id, 42);
        assert_eq!(
            *log.lock().expect("log poisoned"),
            vec!["begin_retire", "revoke", "exiting", "shutdown"],
            "its own incarnation's cleanup does not start with the retreat notice"
        );
    }

    /// ★★An activation without an outcome yet is not answered 「running」★★ — where two callers split: on a connection-declaring link a second activation getting `Ok` would **hold different facts** from the
    /// first once it fails and collects the incarnation. ★The interval is made by holding the reservation directly★ (`LinkWatch` does exactly this). ★The control group is the point★: without a reservation the same
    /// combination must still answer 「running」, else 「always refuse」 passes and breaks ADR-0082's reactivation guard.
    #[test]
    fn an_unresolved_activation_is_not_reported_to_a_second_caller_as_running() {
        let manager = bare_manager();
        let profile = create(
            &manager,
            "C:/still-establishing",
            Some("still-establishing"),
        );
        let id = profile.id;
        // The incarnation the first request put on the roster.
        let (_core, _kills) = put_session_with_link(&manager, id, 4);

        let settled = manager.activate_profile(&profile, SpawnMode::Fresh);
        assert_eq!(
            settled.as_ref().ok().map(|i| i.id),
            Some(id),
            "premise: an incarnation with an outcome answers running as is (ADR-0082 reactivation guard)"
        );

        // That interval where the first request waits for the outcome — `LinkWatch` holds the reservation.
        let _held = SpawnReservation::reserve(manager.spawning.clone(), id).expect("reservation");
        let during = manager.activate_profile(&profile, SpawnMode::Fresh);
        assert!(
            during.is_err(),
            "answered running for an incarnation whose connection has not stood yet — if the first request decides it a failure and collects it, this caller ends up holding as alive an agent about to die under it: got {:?}",
            during.map(|i| i.id)
        );
    }

    /// ★In the same interval `spawn_agent` also does not answer 「it is up」★ — the order of looking at the reservation **before** the roster lookup
    /// is caught here.
    #[test]
    fn a_spawn_request_during_an_unresolved_activation_does_not_hand_back_the_live_session() {
        let manager = bare_manager();
        let profile = create(
            &manager,
            "C:/moot-vs-establishing",
            Some("moot-vs-establishing"),
        );
        let id = profile.id;
        let (_core, _kills) = put_session_with_link(&manager, id, 6);

        let settled = manager
            .spawn_agent(&profile, SpawnMode::Fresh)
            .expect("a duplicate request is not an error");
        assert_eq!(
            settled.into_info().map(|i| i.id),
            Some(id),
            "premise: an incarnation with an outcome returns that information even if moot"
        );

        let _held = SpawnReservation::reserve(manager.spawning.clone(), id).expect("reservation");
        let during = manager
            .spawn_agent(&profile, SpawnMode::Fresh)
            .expect("a duplicate request is not an error");
        assert!(
            during.into_info().is_none(),
            "returned an incarnation whose connection has not stood yet as up — this answer comes out if the roster lookup precedes the reservation"
        );
    }

    /// ★Measures **the half the two entries above cannot**★ — that the interval lasts **until the connection outcome**: they hold the reservation by hand and stay green even if production releases it where spawn
    /// ends. ★Measured in the source★ (needs a real app-server holding the handshake). ★Cannot catch★ a waiter taking only the mailbox from the value (e.g. `watch.map(|w| w.rx)`), silently releasing the
    /// reservation — only both callers holding `watch` whole prevents that.
    #[test]
    fn the_link_watch_holds_the_spawn_reservation_until_the_verdict() {
        let src = include_str!("manager.rs");
        let production = src.split("mod tests {").next().expect("production section");

        let declaration = production
            .split("struct LinkWatch {")
            .nth(1)
            .expect("`LinkWatch` declaration")
            .split('}')
            .next()
            .expect("declaration body");
        assert!(
            declaration.contains("_reservation: SpawnReservation"),
            "the mailbox no longer holds the reservation — then the reservation is released the moment the session goes onto the roster, and a second request coming in while the connection stands answers up again: {declaration}"
        );

        let spawning = production
            .split("fn spawn_agent_watching_link(")
            .nth(1)
            .expect("`spawn_agent_watching_link` body")
            .split("pub fn activate_profile(")
            .next()
            .expect("up to the next function");
        assert!(
            spawning.contains("_reservation: reservation"),
            "the reservation is not carried in the mailbox — it is released the moment this function returns: {spawning}"
        );
    }

    /// Cuts out from the production section only the body of one branch of `spawn_fresh_settled`.
    fn fresh_arm(start: &str, end: &str) -> String {
        let src = include_str!("manager.rs");
        let production = src.split("mod tests {").next().expect("production section");
        let body = production
            .split("fn spawn_fresh_settled(")
            .nth(1)
            .expect("`spawn_fresh_settled` body")
            .split("fn spawn_session(")
            .next()
            .expect("up to the next function");
        let i = body.find(start).unwrap_or_else(|| {
            panic!("the `{start}` branch is missing — this entry's premise is stale")
        });
        let rest = &body[i..];
        let j = rest
            .find(end)
            .unwrap_or_else(|| panic!("`{end}` is not after it — this entry's premise is stale"));
        rest[..j].to_string()
    }

    /// ★★A Fresh spawn also claims success only after seeing the connection outcome★★, in **both entries**. ★Measured in the source★: running needs a real app-server refusing `thread/start`; a regression is
    /// **a silent ownerless state** (Running, all input refused, nobody collecting). Both entries: a boot restore that falls behind opens the hole for every codex profile without a resume handle.
    #[test]
    fn both_fresh_entrances_settle_the_link_before_reporting_success() {
        let src = include_str!("manager.rs");
        let production = src.split("mod tests {").next().expect("production section");

        let settling = production
            .split("fn spawn_fresh_settled(")
            .nth(1)
            .expect("`spawn_fresh_settled` body")
            .split("fn spawn_session(")
            .next()
            .expect("up to the next function");
        assert!(
            settling.contains("self.link_activation_verdict("),
            "the Fresh branch does not look at the connection outcome — a refused handshake has no owner: {settling}"
        );
        assert!(
            settling.contains("self.note_spawn_result("),
            "the old record path of links without a connection axis (claude · shell) disappeared — that path is outside this change's scope"
        );

        let activate = production
            .split("pub fn activate_profile(")
            .nth(1)
            .expect("`activate_profile` body")
            .split("fn note_activation_result(")
            .next()
            .expect("up to the next function");
        assert!(
            activate.contains("self.spawn_fresh_settled(profile, Entry::Caller)"),
            "the Fresh branch of manual activation went back to a verb that does not run the decision, or does not carry the caller entry: {activate}"
        );

        let restore = production
            .split("fn restore_one(")
            .nth(1)
            .expect("`restore_one` body")
            .split("fn resume_no_fallback(")
            .next()
            .expect("up to the next function");
        assert!(
            restore.contains("self.spawn_fresh_settled(profile, Entry::Internal)"),
            "the non-resume branch of boot restore went back to a verb that does not run the decision, or does not carry the internal entry: {restore}"
        );

        // ★The two entries pass **different entry values** to **the same verb** (ADR-0291 E3)★ — per-entry launch copies would bring back 「the decision splits per entry and one falls behind」 (ADR-0082 · ADR-0201 · ADR-0202).
        assert!(
            activate.contains(".resume_no_fallback(profile, Entry::Caller)"),
            "the resume branch of manual activation does not carry the caller entry: {activate}"
        );
        assert!(
            restore.contains(".resume_no_fallback(profile, Entry::Internal)"),
            "the resume branch of boot restore does not carry the internal entry: {restore}"
        );
        let spawn_agent =
            squashed_production_body("pub fn spawn_agent(", "fn spawn_agent_watching_link(");
        assert!(
            spawn_agent.contains("self.spawn_agent_watching_link(profile,mode,Entry::Caller)"),
            "the instant creation entry does not carry the caller entry: {spawn_agent}"
        );
        for (name, body) in [
            ("activate_profile", activate),
            ("spawn_agent", spawn_agent.as_str()),
        ] {
            assert!(
                !body.contains("Entry::Internal"),
                "the entry with a caller `{name}` carries the internal entry — even on a save refusal memory changes and it answers that it changed"
            );
        }
        assert_eq!(
            production.matches("fn spawn_agent_watching_link(").count(),
            1,
            "there are two or more shared launch bodies — per-entry copies split the decision"
        );
        // The two verbs pass the received value as is, and the shared body uses that value to choose the entry of ① registration · ③ clearing.
        let fresh = squashed_production_body("fn spawn_fresh_settled(", "fn spawn_session(");
        assert!(
            fresh.contains("self.spawn_agent_watching_link(profile,SpawnMode::Fresh,entry)"),
            "the Fresh verb does not pass the received entry: {fresh}"
        );
        let resume =
            squashed_production_body("fn resume_no_fallback(", "fn tear_down_failed_activation(");
        for call in [
            "self.spawn_fresh_settled(profile,entry)",
            "self.spawn_agent_watching_link(profile,SpawnMode::Resume,entry)",
        ] {
            assert!(
                resume.contains(call),
                "the resume verb does not pass the received entry (`{call}`)"
            );
        }
        let body =
            squashed_production_body("fn spawn_agent_watching_link(", "pub fn activate_profile(");
        for call in [
            "self.register_for_spawn(profile,entry)?;",
            "entry.release_session_id(&self.profiles,profile.id)",
        ] {
            assert!(
                body.contains(call),
                "the shared body does not choose the registry entry by the entry value (`{call}`)"
            );
        }

        assert_eq!(
            production.matches("self.note_spawn_result(").count(),
            1,
            "the callers of the record settled by one spawn grew — that verb is only for links **without** a connection axis, and calling it where there is one is exactly the evidence destruction ADR-0202 blocks"
        );
    }

    /// ★Fresh's optimistic success does not clear 「the last failure」 either★ (ADR-0202) — as the Resume sibling; Fresh used to read launch as success and clear the earlier record.
    #[test]
    fn a_fresh_success_does_not_clear_the_failure_record() {
        let ready = fresh_arm("EarlyVerdict::Ready =>", "EarlyVerdict::LinkFailed");
        assert!(
            !ready.contains("note_activation_result"),
            "the Fresh success branch touches the last failure — this decision observes only up to being ready to take input, so erasing old failure evidence with it is an irreversible loss: {ready}"
        );
    }

    /// ★The Fresh failure branch also does **cleanup first, record later**★ (as the Resume sibling): the record takes the profile mutex, and the reversed order **never kills the child** if it is held; the cleanup must
    /// carry the incarnation marker or it kills the live successor.
    #[test]
    fn a_failed_fresh_link_tears_down_before_it_records() {
        let arm = fresh_arm(
            "EarlyVerdict::LinkFailed { reason } => {",
            "EarlyVerdict::Terminal {",
        );
        let teardown = arm
            .find("tear_down_failed_activation")
            .expect("the Fresh connection failure branch does not call the cleanup — nobody collects the failed session");
        let record = arm
            .find("note_activation_result")
            .expect("the Fresh connection failure branch does not record the failure — the evidence remains nowhere");
        assert!(
            teardown < record,
            "the record precedes the cleanup — if the profile mutex is held, the child never dies: {arm}"
        );
        assert!(
            arm.contains("tear_down_failed_activation(profile.id, incarnation)"),
            "the cleanup is called without the incarnation marker — if another incarnation stood in between, it kills the live successor: {arm}"
        );
    }

    /// Cuts out from the production section only the body of one branch of `resume_no_fallback`.
    fn resume_arm(start: &str, end: &str) -> String {
        let src = include_str!("manager.rs");
        let production = src.split("mod tests {").next().expect("production section");
        let body = production
            .split("fn resume_no_fallback(")
            .nth(1)
            .expect("`resume_no_fallback` body")
            .split("fn tear_down_failed_activation(")
            .next()
            .expect("up to the next function");
        let i = body.find(start).unwrap_or_else(|| {
            panic!("the `{start}` branch is missing — this entry's premise is stale")
        });
        let rest = &body[i..];
        let j = rest
            .find(end)
            .unwrap_or_else(|| panic!("`{end}` is not after it — this entry's premise is stale"));
        rest[..j].to_string()
    }

    /// ★An optimistic success **does not clear** 「the last failure」★ — half the original defect. ★Measured in the source★: the clearing is **one** `note_activation_result(.., None)` call, and running needs a real codex
    /// resume; a regression is **silent evidence loss**. ★The `Alive` branch must **keep** that call★ (claude's path, out of scope) — measuring both also catches overstepping.
    #[test]
    fn an_optimistic_success_does_not_clear_the_failure_record() {
        let ready = resume_arm("EarlyVerdict::Ready => {", "EarlyVerdict::Alive => {");
        assert!(
            !ready.contains("note_activation_result"),
            "the optimistic success branch touches the last failure — this decision observes only up to being ready to take input, so erasing old failure evidence with it is an irreversible loss: {ready}"
        );

        let src = include_str!("manager.rs");
        let production = src.split("mod tests {").next().expect("production section");
        let body = production
            .split("fn resume_no_fallback(")
            .nth(1)
            .expect("body");
        let alive = &body[body
            .find("EarlyVerdict::Alive => {")
            .expect("the `Alive` branch")..];
        assert!(
            alive.contains("note_activation_result(profile.id, Some(spawned.epoch), None)"),
            "the clearing of links without a connection axis (claude) was removed too — this decision's scope is only links declaring a connection"
        );
    }

    /// ★The failure branch does **cleanup first, record later**★ — reversed, the record's profile mutex (possibly held across the session id record's disk write) turns the stall into **「never kills the child」**,
    /// disabling the backstop. ★The cleanup must carry **the incarnation marker**★ or it kills the live successor (defect ①).
    #[test]
    fn the_failed_activation_arm_tears_down_before_it_records() {
        let arm = resume_arm(
            "EarlyVerdict::LinkFailed { reason } => {",
            "EarlyVerdict::Ready => {",
        );
        let teardown = arm
            .find("tear_down_failed_activation")
            .expect("the connection failure branch does not call the cleanup — nobody collects the failed session");
        let record = arm
            .find("note_activation_result")
            .expect("the connection failure branch does not record the failure");
        assert!(
            teardown < record,
            "the record precedes the cleanup — if the profile mutex is held, the child never dies: {arm}"
        );
        assert!(
            arm.contains("tear_down_failed_activation(profile.id, spawned.epoch)"),
            "the cleanup is called without the incarnation marker — if another incarnation stood in between, it kills the live successor: {arm}"
        );
    }

    /// ★Even if the terminal outruns delivery, **the reason survives**★ — this link's failure text is in neither tail, and the peer exits in 41–51ms on a real refusal; dropping it would stamp the contextual default (early exit)
    /// instead of the cause.
    #[test]
    fn a_terminal_verdict_still_carries_the_delivered_reason() {
        let manager = bare_manager();
        let id = AgentId::new_v4();
        let (core, _kills) = put_session_with_link(&manager, id, 9);
        let (tx, rx) = std::sync::mpsc::channel();
        tx.send(LinkVerdict {
            epoch: 9,
            resolution: LinkResolution::Failed {
                reason: "thread/resume: -32600 no rollout found for thread id".into(),
            },
        })
        .expect("deliver");
        // The two tails are left **empty** — the real shape of a structured link (the refusal comes over stdout).
        core.finish(crate::types::TerminalReason::Exited { code: Some(1) });

        // ★Whichever branch it falls to, **the reason must be alive**★ — delivery vs terminal observation order is timing and pinning it would flicker; what is measured is 「does the cause reach the classification」.
        let verdict = manager.link_activation_verdict(id, 9, &rx, Duration::from_millis(300));
        let carried = match &verdict {
            EarlyVerdict::LinkFailed { reason } => reason.clone(),
            EarlyVerdict::Terminal { evidence, .. } => evidence.clone(),
            other => panic!("premise: observed as a failure — got {other:?}"),
        };
        assert_eq!(
            backend::resume_failure_kind(&codex_app_server_command(), &carried),
            Some(AgentFailureKind::NoConversationToResume),
            "the delivered reason did not get out of the decision — the contextual default (early exit) is stamped as the last failure instead of the cause: verdict={verdict:?}"
        );
    }

    fn codex_app_server_command() -> crate::profile::AgentCommand {
        crate::profile::AgentCommand::Codex {
            extra_args: vec![],
            output_format: crate::profile::AgentOutputFormat::StreamJson,
        }
    }

    fn claude_terminal_command() -> crate::profile::AgentCommand {
        crate::profile::AgentCommand::Claude {
            extra_args: vec![],
            output_format: crate::profile::AgentOutputFormat::Terminal,
        }
    }

    #[cfg(windows)]
    fn claude_stream_json_command() -> crate::profile::AgentCommand {
        crate::profile::AgentCommand::Claude {
            extra_args: vec![],
            output_format: crate::profile::AgentOutputFormat::StreamJson,
        }
    }

    // ── ADR-0172: the decision also stands on **evidence**, not only death ────────────────────────────────

    /// ★Regression net of the headline case★ — diagnostics already saying it failed settle at once even with the process alive: real claude for `--resume <empty sid>` **emits the diagnostic at +2.2s and dies at +6.4s**, and
    /// the old death-only decision passed the 3s window, **decided success and cleared the earlier record**.
    /// ★The clock is not asserted (no flaky recurrence)★: a **very long window** is given and only the outcome is read — a death-waiting implementation (this session never dies) runs to the window's end and emits
    /// `Alive`, going red; no timing dependence.
    #[test]
    fn a_diagnosed_failure_settles_the_verdict_without_waiting_for_the_process_to_die() {
        let manager = bare_manager();
        let id = AgentId::new_v4();
        let core = put_live_session(&manager, id, 1);
        // The seeding is done outside the assertion (avoiding action-in-assertion).
        core.push_diagnostic("No conversation found with session ID: 8b1c");

        // ★The window is much longer than production (3s)★: this session never dies by itself, so a death-waiting implementation runs to the window's end, emits `Alive` and the assertion goes red; a large value rules
        //   out "passing by hitting the deadline" (cost: a slow red on regression).
        let verdict = manager.early_activation_verdict(
            id,
            &claude_terminal_command(),
            Duration::from_secs(30),
            LINK_RESOLUTION_BACKSTOP,
        );

        assert!(
            matches!(
                verdict,
                EarlyVerdict::Diagnosed(AgentFailureKind::NoConversationToResume)
            ),
            "if the diagnostics already said it, it does not wait for the corpse — got {:?}",
            match verdict {
                EarlyVerdict::Diagnosed(k) => format!("Diagnosed({k:?})"),
                EarlyVerdict::Alive =>
                    "Alive (the old decision waiting only for death = regression)".into(),
                EarlyVerdict::Terminal { ref status, .. } => format!("Terminal({status:?})"),
                EarlyVerdict::LinkFailed { ref reason } => format!("LinkFailed({reason})"),
                EarlyVerdict::Ready => "Ready".into(),
            }
        );
        assert!(
            !matches!(
                manager.get_session(id).expect("the session is in the roster").status(),
                AgentStatus::Exited { .. } | AgentStatus::Killed | AgentStatus::Failed { .. }
            ),
            "at decision time this session must still be alive — if it died, this test is measuring the old branch"
        );
    }

    /// ★False-hit defense★ — with silent diagnostics a live session passes as holding; else an implementation emitting `Diagnosed` for any text would pass the test above and **stamp a successful resume as failure**
    /// (ADR-0172 §Impact, fail-open).
    #[test]
    fn a_live_session_with_nothing_to_say_is_not_diagnosed() {
        let manager = bare_manager();
        let id = AgentId::new_v4();
        let core = put_live_session(&manager, id, 1);
        // The seeding is done outside the assertion — unknown text is not something to settle.
        core.push_diagnostic("[DEBUG] loading plugins…");

        let verdict = manager.early_activation_verdict(
            id,
            &claude_terminal_command(),
            Duration::from_millis(250),
            LINK_RESOLUTION_BACKSTOP,
        );

        assert!(
            matches!(verdict, EarlyVerdict::Alive),
            "unknown diagnostics do not block activation (fail-open) — got {}",
            match verdict {
                EarlyVerdict::Diagnosed(k) => format!("Diagnosed({k:?})"),
                EarlyVerdict::Alive => "Alive".into(),
                EarlyVerdict::Terminal { ref status, .. } => format!("Terminal({status:?})"),
                EarlyVerdict::LinkFailed { ref reason } => format!("LinkFailed({reason})"),
                EarlyVerdict::Ready => "Ready".into(),
            }
        );
    }

    /// ★A dead session's evidence includes **the diagnostic stream**★ — a structured session's console ring is empty and only stderr fills; missing it was defect (2) (stream-json is the default, so the classifier ran
    /// blind for most agents).
    #[test]
    fn a_dead_sessions_diagnostic_stream_counts_as_evidence_too() {
        let manager = bare_manager();
        let id = AgentId::new_v4();
        let core = put_live_session(&manager, id, 1);
        // The seeding is done outside the assertion — the console ring is left **empty** (the real shape of a structured session).
        core.push_diagnostic("No conversation found with session ID: 8b1c");
        core.finish(TerminalReason::Exited { code: Some(1) });

        let verdict = manager.early_activation_verdict(
            id,
            &claude_terminal_command(),
            Duration::from_secs(5),
            LINK_RESOLUTION_BACKSTOP,
        );

        let evidence = match verdict {
            EarlyVerdict::Terminal { evidence, .. } => evidence,
            other => panic!(
                "premise: observed in a terminal state — got {}",
                match other {
                    EarlyVerdict::Diagnosed(k) => format!("Diagnosed({k:?})"),
                    EarlyVerdict::Alive => "Alive".into(),
                    EarlyVerdict::LinkFailed { reason } => format!("LinkFailed({reason})"),
                    EarlyVerdict::Ready => "Ready".into(),
                    EarlyVerdict::Terminal { .. } => unreachable!(),
                }
            ),
        };
        assert_eq!(
            backend::resume_failure_kind(&claude_terminal_command(), &evidence),
            Some(AgentFailureKind::NoConversationToResume),
            "even with the console ring empty, stderr must set up the classification — got evidence {evidence:?}"
        );
    }

    // ── subscribe_from: Err comes before on_ready (the premise of the daemon's refusal notice contract) ──────────────
    //
    // ★What it guards★: the contract on `subscribe_from`; a branch failing after `on_ready` would send refusal and Ack together and a late Ack/Complete would attach a success marker to a generation where the replay did
    //   not run. Signal-less at runtime (output silently dies), so this assertion is the only detector.
    #[test]
    fn subscribe_from_err_never_invokes_on_ready() {
        struct NoopSink;
        impl OutputSink for NoopSink {
            fn send(
                &self,
                _frame: crate::types::OutputFrame<'_>,
            ) -> Result<(), crate::types::SinkError> {
                Ok(())
            }
            fn sink_id(&self) -> SinkId {
                SinkId::nil()
            }
        }

        let manager = bare_manager();
        let missing = AgentId::new_v4(); // an id not in the map — get_session fails.
        let mut ready_calls = 0usize;
        let res = manager.subscribe_from(missing, Arc::new(NoopSink), None, None, |_| {
            ready_calls += 1;
        });
        assert!(res.is_err(), "subscribing to a missing agent is Err");
        assert_eq!(
            ready_calls, 0,
            "0 on_ready (=SubscribeAck) emitted on the Err path"
        );
    }

    /// ★The incarnation facts of the subscription response come from the session that replayed★ — `on_ready` (the daemon's Ack) and the return (`ReplayComplete`) speak of the same incarnation, which also does the marker
    ///   comparison.
    // ADR-0226
    #[test]
    fn subscribe_from_reports_the_incarnation_it_replayed_from() {
        struct NoopSink;
        impl OutputSink for NoopSink {
            fn send(
                &self,
                _frame: crate::types::OutputFrame<'_>,
            ) -> Result<(), crate::types::SinkError> {
                Ok(())
            }
            fn sink_id(&self) -> SinkId {
                SinkId::nil()
            }
        }

        let manager = bare_manager();
        let id = AgentId::new_v4();
        put_session(&manager, id, 3);

        let mut at_ready = None;
        let reply = manager
            .subscribe_from(id, Arc::new(NoopSink), Some(0), Some(3), |r| {
                at_ready = Some(r.incarnation)
            })
            .expect("in the roster");
        let expected = crate::types::Incarnation {
            epoch: 3,
            continues_conversation: false,
        };
        assert_eq!(
            reply.incarnation, expected,
            "default session = not a resume"
        );
        assert_eq!(
            at_ready,
            Some(expected),
            "on_ready and the return must speak of the same incarnation"
        );
        assert_eq!(
            reply.outcome.kind,
            crate::types::ReplayKind::Resumed,
            "if the requested marker equals the session marker, continue by seq"
        );

        let stale = manager
            .subscribe_from(id, Arc::new(NoopSink), Some(0), Some(4), |_| {})
            .expect("in the roster");
        assert_eq!(
            stale.outcome.kind,
            crate::types::ReplayKind::FromOldest,
            "if the markers differ, from the beginning"
        );
    }

    #[test]
    fn write_stdin_observed_if_epoch_writes_when_the_incarnation_matches() {
        let manager = bare_manager();
        let id = AgentId::new_v4();
        let written = put_session(&manager, id, 3);

        let out = manager
            .write_stdin_observed_if_epoch(id, 3, b"hello")
            .expect("on a match, a normal write");
        assert_eq!(out.bytes_requested, 5);
        assert_eq!(
            out.epoch, 3,
            "WriteOutcome.epoch = the epoch of the session that executed the write"
        );
        assert_eq!(
            written.lock().unwrap().as_slice(),
            &[b"hello".to_vec()],
            "if the requested epoch equals the current incarnation, it writes as is"
        );
    }

    #[test]
    fn write_stdin_observed_if_epoch_refuses_a_replaced_incarnation_without_writing() {
        let manager = bare_manager();
        let id = AgentId::new_v4();
        let old_written = put_session(&manager, id, 0);
        let new_written = put_session(&manager, id, 1);

        let err = manager
            .write_stdin_observed_if_epoch(id, 0, b"broadcast")
            .expect_err("does not write to a replaced incarnation");
        assert!(
            matches!(err, PtyError::Unsupported(ref m) if m.contains("epoch mismatch")),
            "a mismatch is the unsupported signal + a cause message: {err}"
        );
        assert!(
            new_written.lock().unwrap().is_empty(),
            "not a single byte must go to the new incarnation (zero side effects)"
        );
        assert!(
            old_written.lock().unwrap().is_empty(),
            "the old session was pushed out of the map, so it is not written to either"
        );
        manager
            .write_stdin_observed_if_epoch(id, 1, b"broadcast")
            .expect("pointing at the current incarnation passes");
        assert_eq!(new_written.lock().unwrap().len(), 1);
    }

    // ── Waiting-list query and cancel (ADR-0231) ──

    #[test]
    fn the_queued_listing_carries_the_incarnation_and_the_halt_from_the_turn_table() {
        use crate::turn::{TurnEndKind, TurnSignal};
        let manager = bare_manager();
        let id = AgentId::new_v4();
        put_session(&manager, id, 3);
        let halted = |manager: &AgentManager| {
            manager
                .list_queued_inputs(id)
                .expect("live session")
                .stopped_after_error
        };

        assert_eq!(
            manager.list_queued_inputs(id).expect("live session"),
            QueuedListing {
                rows: vec![],
                as_of_seq: None,
                epoch: 3,
                stopped_after_error: false,
            },
            "without an entry in the turn observation table, the stop is false"
        );

        let turns = manager.turns();
        turns.register(id, 3);
        assert!(!halted(&manager), "right after registration = false");
        turns.observe(id, 3, 1, TurnSignal::Ended(TurnEndKind::Failed));
        assert!(
            halted(&manager),
            "after an error end = stop (the table's field as is)"
        );
        turns.observe(id, 3, 2, TurnSignal::Ended(TurnEndKind::Clean));
        assert!(!halted(&manager), "a clean end clears it");

        turns.register(id, 4);
        turns.observe(id, 4, 1, TurnSignal::Ended(TurnEndKind::Failed));
        assert!(
            !halted(&manager),
            "if the table's entry belongs to another incarnation, it is the same as no entry"
        );
    }

    #[test]
    fn listing_or_cancelling_without_a_live_session_is_not_found() {
        let manager = bare_manager();
        let id = AgentId::new_v4();
        assert!(matches!(
            manager.list_queued_inputs(id),
            Err(PtyError::NotFound(missing)) if missing == id
        ));
        assert!(matches!(
            manager.cancel_queued_input(id, "q1"),
            Err(CancelError::NotFound)
        ));

        put_session(&manager, id, 0);
        assert!(
            matches!(
                manager.cancel_queued_input(id, "q1"),
                Err(CancelError::NotFound)
            ),
            "a session that does not use the list (policy None) is also NotFound — today's answer of every backend"
        );
    }

    #[test]
    fn write_stdin_observed_if_epoch_reports_not_found_for_an_unknown_agent() {
        let manager = bare_manager();
        let err = manager
            .write_stdin_observed_if_epoch(AgentId::new_v4(), 0, b"x")
            .expect_err("a missing agent");
        assert!(
            matches!(err, PtyError::NotFound(_)),
            "absence is a different fact from an epoch mismatch: {err}"
        );
    }

    // ── Roster single entry (ADR-0119) · globally unique names (ADR-0120) ─────────────────────

    /// No profile = an ad-hoc live agent. `put_session` fixes the cwd at `"."`, so name-axis assertions
    /// cannot be made, hence a separate one.
    fn put_live_session_at(manager: &AgentManager, id: AgentId, cwd: &str) {
        let core = Arc::new(OutputCore::new(
            id,
            0,
            Arc::new(NoopStatus),
            TurnWiring::detached(),
        ));
        let session = Arc::new(AgentSession::new(
            id,
            std::path::PathBuf::from(cwd),
            0,
            80,
            24,
            Arc::new(AtomicU8::new(0)),
            BackendCaps {
                session: SessionCaps {
                    resume: false,
                    snapshot: false,
                    cwd_env: false,
                },
                model: ModelCaps {
                    select: false,
                    temperature: false,
                    max_tokens: false,
                },
            },
            InputEncoder::Raw,
            true,
            core,
            Box::new(RecordingTransport {
                written: Arc::new(Mutex::new(Vec::new())),
            }),
        ));
        manager
            .sessions
            .write()
            .expect("sessions poisoned")
            .insert(id, session);
    }

    fn agent_profile(cwd: &str, display_name: Option<&str>) -> AgentProfile {
        let mut p = AgentProfile::new(
            "raw".into(),
            crate::profile::AgentCommand::Shell {
                program: engram_dashboard_platform::shell::default_shell().to_string(),
                args: vec![],
            },
            std::path::PathBuf::from(cwd),
            vec![],
            false,
        );
        p.display_name = display_name.map(|s| s.to_string());
        p
    }

    fn create(manager: &AgentManager, cwd: &str, display_name: Option<&str>) -> AgentProfile {
        manager
            .try_create_agent(agent_profile(cwd, display_name))
            .expect("this fixture does not exhaust the suffix space")
    }

    /// ★A live entry's name and hierarchy come from **the same profile snapshot**★ (else a rename + hierarchy move in between gives a never-existed row: old name + new parent). ★Proves vs cannot★: the two facts are
    /// read together here; the absence of a window between reads holds by structure (one lookup), and a deterministic reproduction needs a seam between reads that no longer exist.
    #[test]
    fn a_live_roster_row_reads_its_name_and_hierarchy_from_one_profile_snapshot() {
        let manager = bare_manager();
        let lead = create(&manager, "C:/lead", Some("lead"));
        let helper = create(&manager, "C:/helper", Some("helper"));
        put_live_session_at(&manager, helper.id, "C:/live/helper");

        assert!(
            manager
                .try_reparent_agent(helper.id, Some(lead.id))
                .expect("save succeeds"),
            "premise"
        );
        assert!(
            renamed_ok(
                manager
                    .try_rename_agent(helper.id, Some("helper-renamed".into()))
                    .expect("save succeeds")
            ),
            "premise"
        );

        let entry = manager
            .roster()
            .into_iter()
            .find(|e| e.id == helper.id)
            .expect("must be in the roster");
        assert_eq!(entry.canonical_name, "helper-renamed", "new name");
        assert_eq!(
            entry.parent,
            Some(lead.id),
            "new parent — not paired with the old name"
        );
        let live = entry.live.as_ref().expect("must be alive");
        assert_eq!(
            live.name, entry.canonical_name,
            "values from one derivation, so they cannot diverge within the entry"
        );
        assert_eq!(
            entry.cwd, "C:/live/helper",
            "the cwd of a live entry is the session cwd (ADR-0101)"
        );
    }

    // ── Runaway backstop (total roster cap) ──────────────────────────────────────────────────

    /// A manager only for cap tests — **memory** profile storage: `bare_manager`'s file storage writes the whole roster per registration (save under lock), making 512 fills O(n²) disk I/O; only the count decision is looked at.
    fn capacity_manager() -> AgentManager {
        #[derive(Default)]
        struct MemStore(Mutex<Vec<AgentProfile>>);
        impl crate::profile::ProfileStore for MemStore {
            fn save(&self, profiles: &[AgentProfile]) -> Result<(), crate::profile::StoreError> {
                *self.0.lock().expect("mem store poisoned") = profiles.to_vec();
                Ok(())
            }
            fn load(&self) -> Vec<AgentProfile> {
                self.0.lock().expect("mem store poisoned").clone()
            }
        }
        let tag = uuid::Uuid::new_v4();
        let profiles = Arc::new(crate::profile::ProfileRegistry::new(Arc::new(
            MemStore::default(),
        )));
        let presets = Arc::new(PresetRegistry::new(Arc::new(FilePresetStore::new(
            std::env::temp_dir().join(format!("engram-cap-preset-{tag}")),
        ))));
        let tracker = Arc::new(SessionTracker::new(
            crate::session_tracker::TrackerConfig {
                enabled: false,
                poll_interval: Duration::from_secs(1),
            },
            Arc::new(|_, _| {}),
        ));
        AgentManager::new(Arc::new(NoopStatus), profiles, presets, tracker)
    }

    /// Fills up to just below the cap. The uniqueness-bypassing seam is used only for speed (the normal path scans the roster per registration, O(n²)).
    fn fill_roster_to(manager: &AgentManager, count: usize) {
        for i in 0..count {
            let mut p = agent_profile(&format!("C:/filler/{i}"), Some(&format!("filler-{i}")));
            p.id = AgentId::new_v4();
            manager.profiles.upsert(p);
        }
        assert_eq!(manager.roster().len(), count, "fill premise");
    }

    /// ★The cap is at **the place where registration commits**, not the entry★ — any entry gets the same answer; the two paths that leaked at the entry (desktop CreateProfile · ad-hoc spawn registration) are looked at together.
    #[test]
    fn both_registration_paths_refuse_a_new_agent_at_the_ceiling() {
        let manager = capacity_manager();
        fill_roster_to(&manager, MAX_ROSTER_SIZE - 1);

        // The last slot passes — the boundary closes not "near" but exactly at the cap.
        let last = manager
            .try_create_agent(agent_profile("C:/last", Some("last-one")))
            .expect("below the cap passes");
        assert_eq!(manager.roster().len(), MAX_ROSTER_SIZE);

        let err = manager
            .try_create_agent(agent_profile("C:/over", Some("one-too-many")))
            .expect_err("a registration over the cap is refused");
        assert!(
            matches!(
                err,
                PtyError::RosterFull {
                    current: MAX_ROSTER_SIZE,
                    limit: MAX_ROSTER_SIZE
                }
            ),
            "must be a dedicated signal **distinct** from name space exhaustion (the caller has different things to do): {err}"
        );

        // The new-registration path of ad-hoc spawn gives the same answer — if this side were open, "total" would be false.
        let err = manager
            .register_for_spawn(&agent_profile("C:/adhoc", Some("adhoc")), Entry::Caller)
            .expect_err("ad-hoc new registration is also refused");
        assert!(matches!(err, PtyError::RosterFull { .. }), "{err}");
        assert_eq!(
            manager.roster().len(),
            MAX_ROSTER_SIZE,
            "a refused registration does not grow the roster"
        );

        // ★Existing agents are not hostages★: re-registration of the same id (restore · respawn) and rename keep working even at the cap.
        manager
            .register_for_spawn(&last, Entry::Caller)
            .expect("re-registration of an existing id is unrelated to the cap");
        assert!(
            renamed_ok(
                manager
                    .try_rename_agent(last.id, Some("still-renameable".into()))
                    .expect("save succeeds")
            ),
            "if the cap froze the roster, recovery itself would become impossible"
        );
    }

    /// ★The check and the commit are the same critical section★ — no window where concurrent registrations each observe below the cap and all commit together.
    #[test]
    fn concurrent_registrations_cannot_all_slip_through_the_last_slot() {
        let manager = Arc::new(capacity_manager());
        fill_roster_to(&manager, MAX_ROSTER_SIZE - 1);

        // There is one slot left but eight try to register at the same time.
        let winners = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        std::thread::scope(|s| {
            for i in 0..8 {
                let manager = Arc::clone(&manager);
                let winners = Arc::clone(&winners);
                s.spawn(move || {
                    if manager
                        .try_create_agent(agent_profile(&format!("C:/racer/{i}"), Some("racer")))
                        .is_ok()
                    {
                        winners.fetch_add(1, Ordering::SeqCst);
                    }
                });
            }
        });
        assert_eq!(
            winners.load(Ordering::SeqCst),
            1,
            "with one free slot exactly one must pass"
        );
        assert_eq!(manager.roster().len(), MAX_ROSTER_SIZE);
    }

    fn renamed_ok(o: RenameOutcome) -> bool {
        matches!(o, RenameOutcome::Renamed(_) | RenameOutcome::Unchanged(_))
    }

    fn name_of_in_roster(manager: &AgentManager, id: AgentId) -> String {
        manager
            .roster()
            .into_iter()
            .find(|e| e.id == id)
            .expect("must be in the roster")
            .canonical_name
    }

    fn name_of(manager: &AgentManager, id: AgentId) -> String {
        manager
            .agent_snapshot(id)
            .expect("must be in the roster")
            .canonical_name_when_live()
    }

    #[test]
    fn roster_reports_live_and_dormant_agents_in_one_query() {
        let manager = bare_manager();
        let live_id = AgentId::new_v4();
        put_live_session_at(&manager, live_id, "C:/roster/alpha");
        let dormant = create(&manager, "C:/roster/beta", Some("beta"));

        let roster = manager.roster();
        assert_eq!(roster.len(), 2, "1 live + 1 asleep: {roster:?}");
        let live = roster
            .iter()
            .find(|e| e.id == live_id)
            .expect("the live agent must be in the roster");
        assert!(
            live.live.is_some(),
            "a live entry must have a session attached"
        );
        assert_eq!(
            live.canonical_name, "alpha",
            "the live name comes from AgentInfo (based on session.cwd)"
        );
        let asleep = roster
            .iter()
            .find(|e| e.id == dormant.id)
            .expect("the asleep agent must also be in the same roster");
        assert!(asleep.live.is_none(), "an asleep entry has no session");
        assert_eq!(
            asleep.canonical_name, "beta",
            "the asleep name comes from canonical_name_when_live()"
        );
    }

    #[test]
    fn a_live_namesake_does_not_hide_a_dormant_agent_with_the_same_name() {
        let manager = bare_manager();
        let live_id = AgentId::new_v4();
        put_live_session_at(&manager, live_id, "C:/roster/twin");
        // ★Planted directly, bypassing uniqueness★: the normal path (try_create_agent) would rename it to "twin(1)" under ADR-0120 and this state could not be made; what is looked at is **the axis of the difference**.
        let dormant = agent_profile("C:/elsewhere/quiet", Some("twin"));
        let dormant_id = dormant.id;
        manager.profiles.upsert(dormant);

        let roster = manager.roster();
        assert_eq!(
            roster
                .iter()
                .filter(|e| e.canonical_name == "twin")
                .count(),
            2,
            "both the live same-name and the asleep same-name must be present (subtracting by the name axis would make the asleep one disappear): {roster:?}"
        );
        assert!(
            roster
                .iter()
                .any(|e| e.id == dormant_id && e.live.is_none()),
            "the asleep one must remain since its id is not in the live set: {roster:?}"
        );
    }

    #[test]
    fn two_dormant_agents_sharing_a_name_are_both_reported() {
        let manager = bare_manager();
        manager.profiles.upsert(agent_profile("C:/a", Some("twin")));
        manager.profiles.upsert(agent_profile("C:/b", Some("twin")));

        let roster = manager.roster();
        assert_eq!(
            roster
                .iter()
                .filter(|e| e.canonical_name == "twin" && e.live.is_none())
                .count(),
            2,
            "two asleep entries with the same name stay two: {roster:?}"
        );
    }

    #[test]
    fn a_dormant_agent_with_a_display_name_does_not_depend_on_the_filesystem() {
        // ★Limit of the observation (stated honestly)★: the syscall cannot be counted — a nonexistent cwd makes "the result would differ had canonicalize run" and the unchanged result is asserted.
        let vanished = "C:/engram-does-not-exist-9f1c/never/created";
        assert!(
            dunce::canonicalize(vanished).is_err(),
            "the premise of this test — this path must not exist"
        );
        let manager = bare_manager();
        let p = create(&manager, vanished, Some("Named"));

        let roster = manager.roster();
        let entry = roster.iter().find(|e| e.id == p.id).expect("asleep entry");
        assert_eq!(
            entry.canonical_name, "Named",
            "with an override the cwd derivation (= fs access) is not taken — had the cwd been looked at it would have become 'created'"
        );
    }

    #[test]
    fn creating_a_colliding_name_gets_the_next_free_suffix() {
        let manager = bare_manager();
        let a = create(&manager, "C:/x", Some("bob"));
        assert_eq!(a.canonical_name_when_live(), "bob", "the first one as is");
        let b = create(&manager, "C:/x", Some("bob"));
        assert_eq!(b.canonical_name_when_live(), "bob(1)");
        let c = create(&manager, "C:/x", Some("bob"));
        assert_eq!(
            c.canonical_name_when_live(),
            "bob(2)",
            "the number is the current family max + 1"
        );
    }

    #[test]
    fn a_suffix_number_is_reused_once_nothing_holds_it() {
        // ★Safety ground★: profile deletion runs the messaging cleanup hook (connection_core DeleteProfile), so a reissued name does not inherit the old owner's parked mail or open contracts.
        let manager = bare_manager();
        create(&manager, "C:/x", Some("bob"));
        let one = create(&manager, "C:/x", Some("bob"));
        assert_eq!(one.canonical_name_when_live(), "bob(1)");
        manager.try_delete_agent(one.id).expect("save succeeds");
        let again = create(&manager, "C:/x", Some("bob"));
        assert_eq!(
            again.canonical_name_when_live(),
            "bob(1)",
            "if nobody holds that number, it is (1) again"
        );
    }

    #[test]
    fn renaming_into_an_existing_name_gets_a_suffix_but_self_rename_does_not() {
        let manager = bare_manager();
        let bob = create(&manager, "C:/x", Some("bob"));
        let alice = create(&manager, "C:/y", Some("alice"));

        assert!(renamed_ok(
            manager
                .try_rename_agent(alice.id, Some("bob".into()))
                .expect("save succeeds")
        ));
        assert_eq!(
            name_of(&manager, alice.id),
            "bob(1)",
            "renaming to someone else's name attaches a suffix"
        );
        assert!(renamed_ok(
            manager
                .try_rename_agent(bob.id, Some("bob".into()))
                .expect("save succeeds")
        ));
        assert_eq!(
            name_of(&manager, bob.id),
            "bob",
            "if re-settling its own name attached a suffix, the number would grow with every rename"
        );
    }

    #[test]
    fn repeating_a_rename_request_does_not_burn_a_new_number() {
        // ★No upstream guard★: the frontend's "value unchanged" guard does not catch the re-request because the current name is `bob(1)`,
        //   and the LLM `RenameProfile` has no guard.
        let manager = bare_manager();
        create(&manager, "C:/x", Some("bob"));
        let alice = create(&manager, "C:/y", Some("alice"));

        assert!(renamed_ok(
            manager
                .try_rename_agent(alice.id, Some("bob".into()))
                .expect("save succeeds")
        ));
        assert_eq!(name_of(&manager, alice.id), "bob(1)");
        assert!(
            renamed_ok(
                manager
                    .try_rename_agent(alice.id, Some("bob".into()))
                    .expect("save succeeds")
            ),
            "a re-request is reported as success (no-op), not a failure"
        );
        assert_eq!(
            name_of(&manager, alice.id),
            "bob(1)",
            "does not burn a number"
        );
        assert!(renamed_ok(
            manager
                .try_rename_agent(alice.id, Some("bob".into()))
                .expect("save succeeds")
        ));
        assert_eq!(name_of(&manager, alice.id), "bob(1)");
        assert!(
            !manager
                .roster()
                .iter()
                .any(|e| e.canonical_name == "bob(2)"),
            "a re-request must not create a new number: {:?}",
            manager.roster()
        );

        let filler = create(&manager, "C:/f", Some("bob"));
        assert_eq!(filler.canonical_name_when_live(), "bob(2)");
        let carol = create(&manager, "C:/c", Some("carol"));
        assert!(renamed_ok(
            manager
                .try_rename_agent(carol.id, Some("bob".into()))
                .expect("save succeeds")
        ));
        assert_eq!(name_of(&manager, carol.id), "bob(3)");
        manager.try_delete_agent(filler.id).expect("save succeeds");
        assert!(
            renamed_ok(
                manager
                    .try_rename_agent(carol.id, Some("bob".into()))
                    .expect("save succeeds")
            ),
            "the same request resubmitted"
        );
        assert_eq!(
            name_of(&manager, carol.id),
            "bob(3)",
            "a re-request does not drag it down to a free lower number (name = address would waver)"
        );
    }

    #[test]
    fn clearing_an_override_into_a_collision_suffixes_and_is_idempotent() {
        let manager = bare_manager();
        let a = create(&manager, "C:/shared", None);
        assert_eq!(
            a.canonical_name_when_live(),
            "shared",
            "without an override, derived from the cwd basename (the premise of this test)"
        );
        let b = create(&manager, "C:/shared", Some("bee"));
        assert_eq!(b.canonical_name_when_live(), "bee");

        assert!(renamed_ok(
            manager.try_rename_agent(b.id, None).expect("save succeeds")
        ));
        assert_eq!(name_of(&manager, b.id), "shared(1)");
        assert_eq!(
            manager.agent_snapshot(b.id).unwrap().display_name,
            Some("shared(1)".to_string()),
            "a conflicting clear does not remove the override (removing it would make same names)"
        );
        assert!(renamed_ok(
            manager.try_rename_agent(b.id, None).expect("save succeeds")
        ));
        assert_eq!(
            name_of(&manager, b.id),
            "shared(1)",
            "a clear re-request is idempotent too"
        );
        assert_eq!(name_of(&manager, a.id), "shared");
    }

    #[test]
    fn a_literal_zero_suffix_does_not_occupy_the_unsuffixed_name() {
        let manager = bare_manager();
        create(&manager, "C:/x", Some("bob(0)"));

        let plain = create(&manager, "C:/y", Some("bob"));
        assert_eq!(
            plain.canonical_name_when_live(),
            "bob",
            "a literal bob(0) does not occupy the unsuffixed bob"
        );
        let next = create(&manager, "C:/z", Some("bob"));
        assert_eq!(next.canonical_name_when_live(), "bob(1)");
    }

    #[test]
    fn the_requested_name_is_the_base_verbatim() {
        let manager = bare_manager();
        let one = create(&manager, "C:/x", Some("bob(1)"));
        assert_eq!(
            one.canonical_name_when_live(),
            "bob(1)",
            "if free, the requested name as is (no reinterpretation as a family)"
        );
        let plain = create(&manager, "C:/y", Some("bob"));
        assert_eq!(
            plain.canonical_name_when_live(),
            "bob",
            "if bob(1) existing made bob unusable, the path of reclaiming names by deletion would be blocked"
        );
        let nested = create(&manager, "C:/z", Some("bob(1)"));
        assert_eq!(nested.canonical_name_when_live(), "bob(1)(1)");
        let bob2 = create(&manager, "C:/w", Some("bob"));
        assert_eq!(
            bob2.canonical_name_when_live(),
            "bob(2)",
            "the bob family max is only the 1 of bob(1) (bob(1)(1) is not in the family)"
        );
        let nested2 = create(&manager, "C:/v", Some("bob(1)"));
        assert_eq!(nested2.canonical_name_when_live(), "bob(1)(2)");
    }

    #[test]
    fn a_saturated_family_falls_back_to_the_lowest_free_number() {
        // ★Saturation is not theory★: `name(4294967295)` is an ordinary state one UI rename can make.
        let manager = bare_manager();
        create(&manager, "C:/x", Some("bob"));
        create(&manager, "C:/y", Some("bob(4294967295)"));

        let first = create(&manager, "C:/z", Some("bob"));
        assert_eq!(
            first.canonical_name_when_live(),
            "bob(1)",
            "even when saturated it gives a free number (refusing would block the whole family permanently)"
        );
        let second = create(&manager, "C:/w", Some("bob"));
        assert_eq!(second.canonical_name_when_live(), "bob(2)");

        let carol = create(&manager, "C:/c", Some("carol"));
        assert!(renamed_ok(
            manager
                .try_rename_agent(carol.id, Some("bob".into()))
                .expect("save succeeds")
        ));
        assert_eq!(name_of(&manager, carol.id), "bob(3)");

        // ★The gate was not poisoned★ — unrelated name allocations keep working.
        let fine = create(&manager, "C:/ok", Some("dave"));
        assert_eq!(fine.canonical_name_when_live(), "dave");
    }

    #[test]
    fn pick_suffix_walks_up_then_fills_the_lowest_hole_only_when_saturated() {
        use std::collections::BTreeSet;
        let set = |v: &[u32]| -> BTreeSet<u32> { v.iter().copied().collect() };

        assert_eq!(pick_suffix(&set(&[])), Some(1));
        assert_eq!(pick_suffix(&set(&[1, 2])), Some(3));
        assert_eq!(
            pick_suffix(&set(&[2])),
            Some(3),
            "even with a hole (1), it does not go down unless saturated"
        );
        assert_eq!(pick_suffix(&set(&[u32::MAX])), Some(1));
        assert_eq!(pick_suffix(&set(&[1, u32::MAX])), Some(2));
        assert_eq!(pick_suffix(&set(&[1, 2, 3, u32::MAX])), Some(4));
        assert_eq!(
            pick_suffix(&set(&[2, u32::MAX])),
            Some(1),
            "the saturation branch uses the lowest hole"
        );
        // ★`None` (the whole family occupied) is not sealed★ — an input filling all of 1..=u32::MAX cannot be made.
    }
    #[test]
    fn epoch_replacement_never_renames_an_existing_agent() {
        let manager = bare_manager();
        let bob = create(&manager, "C:/x", Some("bob"));

        let newcomer = agent_profile("C:/x", Some("bob"));
        manager
            .register_for_spawn(&newcomer, Entry::Caller)
            .expect("new registration succeeds");
        assert_eq!(
            name_of(&manager, newcomer.id),
            "bob(1)",
            "an id not in the roster = new registration → suffix"
        );

        manager
            .register_for_spawn(&bob, Entry::Caller)
            .expect("re-registration succeeds");
        assert_eq!(
            name_of(&manager, bob.id),
            "bob",
            "a restart must not change the name"
        );
        manager
            .register_for_spawn(&newcomer, Entry::Caller)
            .expect("re-registration succeeds");
        assert_eq!(
            name_of(&manager, newcomer.id),
            "bob(1)",
            "re-registration does not accumulate suffixes"
        );

        let third = create(&manager, "C:/x", Some("bob"));
        assert_eq!(third.canonical_name_when_live(), "bob(2)");

        // ★★Stale snapshot re-registration — a real regression scenario★★: rename in the tree while live → a later restart re-registers with **the pre-rename snapshot** whose old name **another agent took** meanwhile; a
        //   new-registration check would call it a collision and rename the agent wrongly.
        let agent = create(&manager, "C:/stale", Some("was-here"));
        let stale_snapshot = agent.clone();
        assert!(renamed_ok(
            manager
                .try_rename_agent(agent.id, Some("renamed".into()))
                .expect("save succeeds")
        ));
        assert_eq!(name_of(&manager, agent.id), "renamed");
        let squatter = create(&manager, "C:/squat", Some("was-here"));
        assert_eq!(squatter.canonical_name_when_live(), "was-here");
        manager
            .register_for_spawn(&stale_snapshot, Entry::Caller)
            .expect("re-registration succeeds");
        assert_eq!(
            name_of(&manager, agent.id),
            "renamed",
            "a restart must not make a rename decision with the stale snapshot's old name"
        );
        assert_eq!(
            name_of(&manager, squatter.id),
            "was-here",
            "a restart does not touch someone else's name either"
        );
    }

    #[test]
    fn renaming_into_a_freed_name_actually_takes_it() {
        let manager = bare_manager();
        let first = create(&manager, "C:/x", Some("bob"));
        let second = create(&manager, "C:/y", Some("bob"));
        assert_eq!(second.canonical_name_when_live(), "bob(1)");

        manager.try_delete_agent(first.id).expect("save succeeds");
        let out = manager
            .try_rename_agent(second.id, Some("bob".into()))
            .expect("save succeeds");
        assert_eq!(
            out,
            RenameOutcome::Renamed("bob".to_string()),
            "a request for a free name must be settled (reporting unchanged success is false)"
        );
        assert_eq!(name_of(&manager, second.id), "bob");
    }

    #[test]
    fn clearing_an_override_works_once_the_derived_name_is_free() {
        let manager = bare_manager();
        let holder = create(&manager, "C:/shared", None);
        assert_eq!(holder.canonical_name_when_live(), "shared");
        let b = create(&manager, "C:/shared", Some("bee"));
        assert!(renamed_ok(
            manager.try_rename_agent(b.id, None).expect("save succeeds")
        ));
        assert_eq!(
            name_of(&manager, b.id),
            "shared(1)",
            "the suffix stays while in conflict"
        );

        manager.try_delete_agent(holder.id).expect("save succeeds");
        let out = manager.try_rename_agent(b.id, None).expect("save succeeds");
        assert_eq!(out, RenameOutcome::Renamed("shared".to_string()));
        assert_eq!(
            manager.agent_snapshot(b.id).unwrap().display_name,
            None,
            "once clearing became possible, the override must actually be removed"
        );
        assert_eq!(name_of(&manager, b.id), "shared");
    }

    #[test]
    fn rename_failures_are_distinguishable_from_each_other() {
        let manager = bare_manager();
        create(&manager, "C:/x", Some("bob"));
        let alice = create(&manager, "C:/y", Some("alice"));

        assert_eq!(
            manager
                .try_rename_agent(AgentId::new_v4(), Some("bob".into()))
                .expect("save succeeds"),
            RenameOutcome::NotFound,
            "a missing id is NotFound (not a name problem)"
        );
        assert_eq!(
            manager
                .try_rename_agent(alice.id, Some("bob".into()))
                .expect("save succeeds"),
            RenameOutcome::Renamed("bob(1)".to_string()),
            "settling reports the settled name with it"
        );
        assert_eq!(
            manager
                .try_rename_agent(alice.id, Some("bob".into()))
                .expect("save succeeds"),
            RenameOutcome::Unchanged("bob(1)".to_string()),
            "an idempotent re-request is told apart as unchanged"
        );
    }

    #[test]
    fn a_live_agents_own_name_is_read_from_the_roster_not_re_derived() {
        let manager = bare_manager();
        create(&manager, "C:/x", Some("bob"));
        let x = create(&manager, "C:/somewhere/zeta", None);
        assert_eq!(
            x.canonical_name_when_live(),
            "zeta",
            "profile derivation axis"
        );
        put_live_session_at(&manager, x.id, "C:/live/bob(1)");
        assert_eq!(name_of_in_roster(&manager, x.id), "bob(1)", "roster axis");

        let out = manager
            .try_rename_agent(x.id, Some("bob".into()))
            .expect("save succeeds");
        assert_eq!(
            out,
            RenameOutcome::Unchanged("bob(1)".to_string()),
            "it must decide with its own name as the roster says"
        );
        assert_eq!(
            manager.agent_snapshot(x.id).unwrap().display_name,
            None,
            "if unchanged, no new override is planted (planting one would be a silent rename)"
        );
    }

    #[test]
    fn every_name_gate_entrance_stores_the_override_without_edge_whitespace() {
        let manager = bare_manager();

        let created = create(&manager, "C:/x", Some("  bob  "));
        assert_eq!(
            created.display_name,
            Some("bob".to_string()),
            "normalized starting from the value the creation response carries"
        );
        assert_eq!(
            manager.agent_snapshot(created.id).unwrap().display_name,
            Some("bob".to_string()),
            "the value stored in the roster must also be the same"
        );

        let renamed = create(&manager, "C:/y", Some("carol"));
        assert!(renamed_ok(
            manager
                .try_rename_agent(renamed.id, Some("\tdave\n".into()))
                .expect("save succeeds")
        ));
        assert_eq!(
            manager.agent_snapshot(renamed.id).unwrap().display_name,
            Some("dave".to_string()),
            "rename path (including tabs · newlines)"
        );

        let spawned = agent_profile("C:/z", Some(" erin "));
        manager
            .register_for_spawn(&spawned, Entry::Caller)
            .expect("new registration succeeds");
        assert_eq!(
            manager.agent_snapshot(spawned.id).unwrap().display_name,
            Some("erin".to_string()),
            "new registration path"
        );
    }

    #[test]
    fn a_whitespace_only_override_is_stored_as_no_override() {
        // A whitespace-only override is already ignored by the derivation functions (no empty labels), which rescues only **the display**; a useless string remains in the roster.
        let manager = bare_manager();

        let blank = create(&manager, "C:/blankdir", Some("   "));
        assert_eq!(
            blank.display_name, None,
            "a request leaving only whitespace is stored as no override"
        );
        assert_eq!(
            name_of(&manager, blank.id),
            "blankdir",
            "no override, so a cwd-derived name"
        );

        let named = create(&manager, "C:/otherdir", Some("zoe"));
        assert!(renamed_ok(
            manager
                .try_rename_agent(named.id, Some(" ".into()))
                .expect("save succeeds")
        ));
        assert_eq!(
            manager.agent_snapshot(named.id).unwrap().display_name,
            None,
            "a whitespace-only request on the rename path = clearing the override"
        );
        assert_eq!(name_of(&manager, named.id), "otherdir");
    }

    #[test]
    fn interior_whitespace_is_part_of_the_name() {
        let manager = bare_manager();
        let a = create(&manager, "C:/x", Some("  bob smith  "));
        assert_eq!(a.display_name, Some("bob smith".to_string()));
        assert_eq!(name_of(&manager, a.id), "bob smith");
    }

    #[test]
    fn renaming_to_a_padded_form_of_the_current_name_burns_no_number() {
        let manager = bare_manager();
        let bob = create(&manager, "C:/a", Some("bob"));

        assert_eq!(
            manager
                .try_rename_agent(bob.id, Some("  bob  ".into()))
                .expect("save succeeds"),
            RenameOutcome::Renamed("bob".to_string()),
            "the request is a `bob` request, so the name does not change"
        );
        assert_eq!(name_of(&manager, bob.id), "bob");
        assert_eq!(
            manager.agent_snapshot(bob.id).unwrap().display_name,
            Some("bob".to_string())
        );
        let next = create(&manager, "C:/b", Some("bob"));
        assert_eq!(
            next.canonical_name_when_live(),
            "bob(1)",
            "if the padded rename had freed bob, two same-named entries would sit here"
        );
    }

    #[test]
    fn a_padded_request_for_a_taken_name_gets_the_suffixed_form() {
        let manager = bare_manager();
        create(&manager, "C:/h", Some("bob"));

        let other = create(&manager, "C:/o", Some("  bob  "));
        assert_eq!(
            other.display_name,
            Some("bob(1)".to_string()),
            "a padded request also goes into the suffix family"
        );
        assert_eq!(name_of(&manager, other.id), "bob(1)");

        assert_eq!(
            manager
                .try_rename_agent(other.id, Some(" bob ".into()))
                .expect("save succeeds"),
            RenameOutcome::Unchanged("bob(1)".to_string())
        );
        let third = create(&manager, "C:/t", Some("bob"));
        assert_eq!(
            third.canonical_name_when_live(),
            "bob(2)",
            "if the re-request had burned a number, this would become bob(3)"
        );
    }

    /// ★Why the hook point is `capabilities()`★: `roster()` → `list_agents()` → `agent_info()` calls it per session — the only injection point for arbitrary code **during the observation** of `try_rename_agent`, so
    ///   "the profile disappears right before commit" is reproduced deterministically without threads or timing.
    struct HookedTransport {
        hook: Mutex<Option<Box<dyn FnOnce() + Send>>>,
    }
    impl AgentTransport for HookedTransport {
        fn start(&self, _core: Arc<OutputCore>) {}
        fn send_input(&self, _input: InputEvent) -> Result<(), PtyError> {
            Ok(())
        }
        fn resize(&self, _c: u16, _r: u16) -> Result<(), PtyError> {
            Ok(())
        }
        fn interrupt(&self) -> Result<(), PtyError> {
            Ok(())
        }
        fn shutdown(&self) {}
        fn capabilities(&self) -> TransportCaps {
            if let Some(h) = self.hook.lock().expect("hook poisoned").take() {
                h();
            }
            TransportCaps {
                input: InputCaps {
                    raw: true,
                    message: false,
                    attachment: false,
                },
                output: OutputCaps {
                    terminal_bytes: false,
                    structured: true,
                    markdown: false,
                    tool_events: false,
                    usage: false,
                },
                control: ControlCaps {
                    resize: false,
                    interrupt: false,
                    cancel: false,
                    graceful_shutdown: false,
                },
            }
        }
    }

    fn put_live_session_with(
        manager: &AgentManager,
        id: AgentId,
        cwd: &str,
        transport: Box<dyn AgentTransport>,
    ) {
        let core = Arc::new(OutputCore::new(
            id,
            0,
            Arc::new(NoopStatus),
            TurnWiring::detached(),
        ));
        let session = Arc::new(AgentSession::new(
            id,
            std::path::PathBuf::from(cwd),
            0,
            80,
            24,
            Arc::new(AtomicU8::new(0)),
            BackendCaps {
                session: SessionCaps {
                    resume: false,
                    snapshot: false,
                    cwd_env: false,
                },
                model: ModelCaps {
                    select: false,
                    temperature: false,
                    max_tokens: false,
                },
            },
            InputEncoder::Raw,
            true,
            core,
            transport,
        ));
        manager
            .sessions
            .write()
            .expect("sessions poisoned")
            .insert(id, session);
    }

    #[test]
    fn a_delete_landing_mid_rename_reports_not_found_not_success() {
        let manager = bare_manager();
        let victim = create(&manager, "C:/victim", Some("before"));
        let vid = victim.id;

        let profiles = Arc::clone(&manager.profiles);
        put_live_session_with(
            &manager,
            vid,
            "C:/live/victim",
            Box::new(HookedTransport {
                hook: Mutex::new(Some(Box::new(move || {
                    profiles.try_remove(vid).expect("save succeeds")
                }))),
            }),
        );

        let out = manager
            .try_rename_agent(vid, Some("after".into()))
            .expect("save succeeds");
        assert_eq!(
            out,
            RenameOutcome::NotFound,
            "a commit that did not land must not be reported as success (Free branch)"
        );
        assert!(
            manager.agent_snapshot(vid).is_none(),
            "premise — the profile actually disappeared"
        );

        let holder = create(&manager, "C:/holder", Some("taken"));
        let victim2 = create(&manager, "C:/victim2", Some("other"));
        let vid2 = victim2.id;
        let profiles2 = Arc::clone(&manager.profiles);
        put_live_session_with(
            &manager,
            vid2,
            "C:/live/victim2",
            Box::new(HookedTransport {
                hook: Mutex::new(Some(Box::new(move || {
                    profiles2.try_remove(vid2).expect("save succeeds")
                }))),
            }),
        );
        let out2 = manager
            .try_rename_agent(vid2, Some("taken".into()))
            .expect("save succeeds");
        assert_eq!(
            out2,
            RenameOutcome::NotFound,
            "the commit result of the suffix allocation branch must not be swallowed either"
        );
        assert!(manager.agent_snapshot(vid2).is_none());
        assert_eq!(
            name_of(&manager, holder.id),
            "taken",
            "a failed rename does not touch someone else's name"
        );
    }

    #[test]
    fn clearing_an_override_predicts_the_name_from_the_live_axis() {
        let manager = bare_manager();
        let y = create(&manager, "C:/other", Some("q"));
        // X: override "x", profile-axis basename "p", live-axis basename "q" (= the side conflicting with Y).
        let x = create(&manager, "C:/prof/p", Some("x"));
        put_live_session_at(&manager, x.id, "C:/live/q");
        assert_eq!(
            name_of_in_roster(&manager, x.id),
            "x",
            "premise — the override is the name"
        );

        let out = manager.try_rename_agent(x.id, None).expect("save succeeds");
        assert_eq!(
            out,
            RenameOutcome::Renamed("q(1)".to_string()),
            "predicting with the profile axis ('p') misjudges Free and reports Renamed(\"p\")"
        );
        assert_eq!(
            name_of_in_roster(&manager, x.id),
            "q(1)",
            "it must be the suffixed name in the roster too"
        );
        assert_eq!(name_of_in_roster(&manager, y.id), "q", "Y's name unchanged");
        let roster = manager.roster();
        let unique: std::collections::BTreeSet<&String> =
            roster.iter().map(|e| &e.canonical_name).collect();
        assert_eq!(
            unique.len(),
            roster.len(),
            "no same-named entries must sit in the roster: {roster:?}"
        );
    }

    #[test]
    fn clearing_an_override_predicts_with_the_live_derivation_not_the_dormant_one() {
        // This fixture makes the split of the two axes directly with `<temp>/.` — the raw basename is `"."`, and canonicalized it is
        //   the last segment of `<temp>`.
        let temp = std::env::temp_dir();
        let dotted = format!("{}/.", temp.to_string_lossy());
        let raw_base = crate::name::cwd_basename(&dotted);
        let canon_base = {
            let c = dunce::canonicalize(&dotted).expect("the temp directory exists");
            crate::name::cwd_basename(&c.to_string_lossy())
        };
        assert_ne!(
            raw_base, canon_base,
            "the premise of this test — the two axes must actually split"
        );

        let manager = bare_manager();
        let y = create(&manager, "C:/other", Some(&raw_base));
        let x = create(&manager, "C:/prof/whatever", Some("x"));
        put_live_session_at(&manager, x.id, &dotted);
        assert_eq!(
            name_of_in_roster(&manager, x.id),
            "x",
            "premise — the override is the name"
        );

        let out = manager.try_rename_agent(x.id, None).expect("save succeeds");
        assert_eq!(
            out,
            RenameOutcome::Renamed(format!("{raw_base}(1)")),
            "predicting with the asleep function sees the canonicalized name (free) and misjudges Free"
        );
        assert_eq!(name_of_in_roster(&manager, x.id), format!("{raw_base}(1)"));
        assert_eq!(name_of_in_roster(&manager, y.id), raw_base, "Y unchanged");
        let roster = manager.roster();
        let unique: std::collections::BTreeSet<&String> =
            roster.iter().map(|e| &e.canonical_name).collect();
        assert_eq!(
            unique.len(),
            roster.len(),
            "no same-named entries must sit in the roster: {roster:?}"
        );
    }

    #[test]
    fn concurrent_creates_of_one_name_never_both_take_it() {
        // ★The overlap is made **by design** with a Barrier★ — without it threads flow sequentially and contention depends on disk latency. ★Still probabilistic★: the barrier aligns only the entry time; removing the gate
        //   fails almost always but may pass by luck. A deterministic seal needs a yield-seam at the allocation point (a separate decision).
        const THREADS: usize = 8;
        let manager = Arc::new(bare_manager());
        let start = Arc::new(std::sync::Barrier::new(THREADS));
        let names: Vec<String> = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..THREADS)
                .map(|_| {
                    let m = Arc::clone(&manager);
                    let start = Arc::clone(&start);
                    scope.spawn(move || {
                        // Profile assembly is finished **before** the barrier so that only the allocation starts at the same time.
                        let p = agent_profile("C:/race", Some("bob"));
                        start.wait();
                        m.try_create_agent(p)
                            .expect("allocation succeeds")
                            .canonical_name_when_live()
                    })
                })
                .collect();
            handles.into_iter().map(|h| h.join().unwrap()).collect()
        });

        let unique: std::collections::BTreeSet<&String> = names.iter().collect();
        assert_eq!(
            unique.len(),
            THREADS,
            "concurrent allocations must receive different names (duplicate = uniqueness collapse): {names:?}"
        );
        let roster = manager.roster();
        assert_eq!(roster.len(), THREADS);
        let roster_unique: std::collections::BTreeSet<&String> =
            roster.iter().map(|e| &e.canonical_name).collect();
        assert_eq!(
            roster_unique.len(),
            THREADS,
            "no same-named entries must sit in the roster: {roster:?}"
        );
    }

    // ── ADR-0172: a duplicate request is not an error but 「nothing to do」, and changes nothing ────────────

    /// ★A request to an entry already launching — made **deterministically** by holding the reservation★ (no thread race). Checks: an outcome, not an error · the caller can tell 「I did not launch it」 · **neither record
    /// nor clear** (the earlier value stays).
    #[test]
    fn a_request_that_arrives_while_another_spawn_is_in_flight_is_moot_and_changes_nothing() {
        use crate::failure::AgentFailureKind;

        let manager = bare_manager();
        let profile = create(&manager, "C:/moot-in-flight", Some("moot-in-flight"));
        let id = profile.id;
        // The seeding is done outside the assertion (avoiding action-in-assertion).
        let seeded = manager.profiles.set_last_failure(
            id,
            None,
            Some(AgentFailureKind::NoConversationToResume),
        );
        assert!(seeded, "premise: the seed actually changes the value");

        // The very situation of another request in progress — held without releasing.
        let _held = SpawnReservation::reserve(manager.spawning.clone(), id)
            .expect("the reservation can be taken");

        let outcome = manager
            .activate_profile(&profile, SpawnMode::Fresh)
            .err()
            .map(|e| e.to_string());
        assert!(
            outcome.is_some(),
            "the winner has not yet put it on the roster, so there is no session to return — NotFound is the honest answer"
        );
        assert!(
            manager.list_agents().is_empty(),
            "this request did not launch a process"
        );
        assert_eq!(
            manager.agent_snapshot(id).and_then(|p| p.last_failure),
            Some(AgentFailureKind::NoConversationToResume),
            "there was nothing to do, so it neither records nor clears"
        );
    }

    /// ★A request to an entry already up — with a real process★. It differs from the sibling above by having **a session to return**, so the caller gets that agent's information even on moot.
    #[cfg(windows)]
    #[test]
    fn a_request_for_an_already_running_agent_is_moot_and_returns_the_live_session() {
        use crate::failure::AgentFailureKind;

        let manager = bare_manager();
        let profile = create(
            &manager,
            &std::env::temp_dir().to_string_lossy(),
            Some("moot-live"),
        );
        let id = profile.id;
        let first = manager
            .spawn_agent(&profile, SpawnMode::Fresh)
            .expect("first spawn");
        assert!(
            first.started(),
            "premise: the first request actually launched"
        );

        // Plant a record on the live agent and see that the duplicate request does not touch it.
        // The seeding is done outside the assertion (avoiding action-in-assertion).
        let seeded = manager.profiles.set_last_failure(
            id,
            None,
            Some(AgentFailureKind::EarlyExitAfterResume),
        );
        assert!(seeded, "premise: the seed actually changes the value");

        let again = manager
            .spawn_agent(&profile, SpawnMode::Fresh)
            .expect("a duplicate request is not an error");
        assert!(!again.started(), "this call launched nothing");
        assert_eq!(
            again.into_info().map(|i| i.id),
            Some(id),
            "even moot returns the live session information"
        );
        assert_eq!(
            manager.agent_snapshot(id).and_then(|p| p.last_failure),
            Some(AgentFailureKind::EarlyExitAfterResume),
            "there was nothing to do, so neither record nor clear"
        );
        assert_eq!(
            manager.list_agents().len(),
            1,
            "there is still one incarnation"
        );

        manager.kill_agent(id).ok();
    }

    /// When activation holds it clears at once (ADR-0172 decision 4 amended); the axes stay separate — a record must be attachable again to a running entry so the screen's 「running wins」 has a place. ★In-crate★: seeding
    /// needs `set_last_failure`, which is `pub(crate)` (the compiler guards the single-write-point boundary).
    #[cfg(windows)]
    #[test]
    fn a_successful_activation_clears_the_last_failure_but_the_axis_stays_separate() {
        use crate::failure::AgentFailureKind;

        let manager = bare_manager();
        let profile = create(
            &manager,
            &std::env::temp_dir().to_string_lossy(),
            Some("clear-on-success"),
        );
        let id = profile.id;
        // The seeding is done outside the assertion (avoiding action-in-assertion).
        let seeded = manager.profiles.set_last_failure(
            id,
            None,
            Some(AgentFailureKind::NoConversationToResume),
        );
        assert!(seeded, "premise: the seed actually changes the value");

        let info = manager
            .activate_profile(&profile, SpawnMode::Fresh)
            .expect("activation succeeds");
        assert!(
            !matches!(
                info.status,
                AgentStatus::Failed { .. } | AgentStatus::Killed | AgentStatus::Exited { .. }
            ),
            "premise: this session is alive — got {:?}",
            info.status
        );
        assert_eq!(
            manager.agent_snapshot(id).and_then(|p| p.last_failure),
            None,
            "activation held, so it must clear"
        );

        // Axis independence — merged into the state enum, this combination could not be expressed. Seeding is outside the assertion (no action-in-assertion).
        let seeded = manager.profiles.set_last_failure(
            id,
            Some(info.epoch),
            Some(AgentFailureKind::SpawnFailed),
        );
        assert!(seeded, "premise: the seed actually changes the value");
        assert!(
            manager.list_agents().iter().any(|a| a.id == id),
            "still in the live roster"
        );
        assert_eq!(
            manager.agent_snapshot(id).and_then(|p| p.last_failure),
            Some(AgentFailureKind::SpawnFailed),
            "the running + last failure combination is expressed"
        );

        manager.kill_agent(id).ok();
    }

    /// ★Whether a dead session's output tail is readable — only a real process can say (ADR-0172)★: a session looked up again **inside** the polling loop is removed by the reaper right after `finish` (polling is 100ms), so
    /// the tail is empty, classification always falls to the contextual default and 「no conversation to resume」 becomes **dead code** — invisible to a pure dispatch test.
    #[cfg(windows)]
    #[test]
    fn early_verdict_captures_the_dead_sessions_output_tail() {
        use crate::failure::AgentFailureKind;

        // ★A batch file★: portable-pty `CommandBuilder` quotes `&`·`>` individually and breaks them through ConPTY (activation.rs fixture); cmd parses a batch directly.
        // ★It lives briefly before dying (ping)★: the subject is reading the ring **after** the reaper removed it; dying at once could lose the first `get_session` race and go red for an unrelated reason.
        let uniq = uuid::Uuid::new_v4();
        let batch = std::env::temp_dir().join(format!("engram-tail-probe-{uniq}.cmd"));
        std::fs::write(
            &batch,
            "@echo off\r\necho No conversation found with session ID: 8b1c\r\nping -n 2 127.0.0.1 >nul\r\nexit /b 1\r\n",
        )
        .expect("batch write");

        let manager = bare_manager();
        let mut draft = agent_profile(&std::env::temp_dir().to_string_lossy(), Some("tail-probe"));
        draft.command = AgentCommand::Shell {
            program: "cmd.exe".into(),
            args: vec!["/c".into(), batch.to_string_lossy().to_string()],
        };
        let profile = manager.try_create_agent(draft).expect("register");
        manager
            .spawn_agent(&profile, SpawnMode::Fresh)
            .expect("spawn")
            .into_started()
            .expect("this call actually launches (not a duplicate request)");

        // ★A shell profile, so the diagnostic buffer is empty (the PTY merges stderr into the console)★ — the evidence here is solely **the console tail**, which this test measures.
        let EarlyVerdict::Terminal { status, evidence } = manager.early_activation_verdict(
            profile.id,
            &claude_terminal_command(),
            Duration::from_secs(10),
            LINK_RESOLUTION_BACKSTOP,
        ) else {
            panic!("this batch dies immediately (premise)")
        };
        assert!(
            matches!(
                status,
                AgentStatus::Exited { .. } | AgentStatus::Failed { .. } | AgentStatus::Killed
            ),
            "premise: observed in a terminal state — got {status:?}"
        );
        assert!(
            evidence.contains("No conversation found"),
            "the tail must be readable even after the reaper collected the session — if empty, the classification is always the contextual default. got {evidence:?}"
        );
        // That tail actually continues into the classification (claude vocabulary — runs only dispatch without a real claude).
        assert_eq!(
            backend::resume_failure_kind(&claude_terminal_command(), &evidence),
            Some(AgentFailureKind::NoConversationToResume),
            "the tail read from the real session must be classified as no conversation to resume"
        );

        let _ = std::fs::remove_file(&batch);
    }

    /// ★Pins **the order of three verbs** inside `spawn_agent` — the race-freedom of the resume value★: 1. `epoch_for_spawn` **right after** `register_for_spawn` — the latter **keeps** the live marker (ADR-0084), so in
    /// between **the previous incarnation's marker** stands and a late record of it is **not refused**; cwd normalization (a syscall) or the Fresh clearing (whole `agents.json` write) in between would open the interval.
    /// 2. The resume handle read is **after** `epoch_for_spawn`: until this spawn hands over the record port no record passes, so the roster value is frozen and 「read = resumed」 holds.
    /// ★The read must be **from the roster**★ — the caller snapshot would miss the thread id the peer gave meanwhile. ★Measured in the source★: running needs a real codex and a window for a late record; a divergence is
    /// **a low-probability silent miswrite**. Precedent: `backend::codex::transport::tests::the_session_id_is_recorded_before_the_gate_opens`.
    #[test]
    fn the_resume_handle_is_read_from_the_roster_after_the_incarnation_tag_is_stamped() {
        let src = include_str!("manager.rs");
        let production = src.split("mod tests {").next().expect("production section");

        // ★Before cutting out the body, check **that what is cut out is right**★: the two boundary names are free text, and a function moved above `spawn_agent` would grow the lump so the order assertions go green over
        //   unrelated code; these two lines block that silent pass.
        assert_eq!(
            production.matches("pub fn spawn_agent(").count(),
            1,
            "the `spawn_agent` name appears twice or more in the production section — it is unknown which one the cut below caught"
        );
        let body = production
            .split("pub fn spawn_agent(")
            .nth(1)
            .expect("`spawn_agent` body")
            .split("pub fn activate_profile(")
            .next()
            .expect("up to the next function");
        assert!(
            !body.contains("pub fn "),
            "the cut lump contains another `pub fn` — the boundary function moved and the range overflowed"
        );

        // ★Looked at as **the list of executed lines**, not byte offsets★ — rustfmt splits chains, so a single-lump string search breaks on one formatting (it did).
        let code: Vec<&str> = body
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with("//"))
            .collect();
        let only = |needle: &str| -> usize {
            let hits: Vec<usize> = code
                .iter()
                .enumerate()
                .filter(|(_, l)| l.contains(needle))
                .map(|(i, _)| i)
                .collect();
            assert_eq!(
                hits.len(),
                1,
                "`{needle}` is caught {} times in `spawn_agent` — it must be exactly one for this entry to pin down the place",
                hits.len()
            );
            hits[0]
        };

        let registered = only("self.register_for_spawn(profile, entry)?;");
        let stamped = only(".epoch_for_spawn(profile.id)");
        // ★The anchor is **the line reading from the roster**★ — one more line follows (`resume_handle_for` clearing Fresh); the order assertion measures the place of 「the read」.
        let resume_read = only("let stored_handle = match mode {");
        let handle_gated = only("let resume_session_id = resume_handle_for(");
        let spec_built = only("let spec = backend::build_command_spec(");

        // ★The read must be **before** the spec assembly (ADR-0208)★ — a command-line-resuming backend (codex terminal `resume <id>`) takes the value at argv assembly; pushed after, the resume **silently becomes a new
        //   conversation** in that mode only.
        assert!(
            resume_read < spec_built,
            "the handle read is after the spec assembly (read {resume_read} · assembly {spec_built}) — the backend resuming by command \
             line loses the value it would receive"
        );
        // Reading but not passing would let a state where only the order above matches and the value is `None` pass green.
        let spec_call_end = code[spec_built..]
            .iter()
            .position(|l| l.starts_with(");"))
            .expect("end of the spec assembly call");
        let spec_call = &code[spec_built..spec_built + spec_call_end];
        assert!(
            spec_call.iter().any(|l| l.contains("resume_session_id")),
            "the spec assembly does not receive the resume handle — the terminal-mode codex cannot emit `resume <id>`: \
             {spec_call:?}"
        );

        assert!(
            registered < stamped && stamped < resume_read,
            "the order of the three verbs diverged (registration {registered} · marker {stamped} · handle read {resume_read}) — \
             if the marker precedes the registration, the registration revives the previous incarnation's marker, and if the handle read precedes the marker, the roster value of that \
             interval is open to the previous incarnation's late records"
        );

        // The **very next executed line** after registration must be the start of the marker binding (only comments and blank lines allowed in between).
        assert!(
            code[registered + 1].starts_with("let epoch"),
            "the executed line after `register_for_spawn` is not the marker binding — the interval where the previous incarnation's marker stands in the roster \
             widens by that much, and a late record arriving in that interval sits in the cleared field and makes this incarnation's first submission \
             commit refused: {:?}",
            code[registered + 1]
        );

        // ★The door clearing Fresh must be **between** the read and the spec assembly★ — without it a Fresh spawn carries the stored handle out and the consumer reading only that value (codex terminal session id
        //   reclamation — ADR-0218 decision 5) misjudges a new conversation as 「resuming」 and skips reclamation.
        assert!(
            resume_read < handle_gated && handle_gated < spec_built,
            "the place of the door clearing Fresh diverged (read {resume_read} · door {handle_gated} · assembly \n             {spec_built})"
        );

        let binding_end = code[resume_read..]
            .iter()
            .position(|l| *l == "};")
            .expect("end of the resume binding (`};`)");
        let binding = &code[resume_read..resume_read + binding_end];
        assert!(
            !binding.iter().any(|l| l.contains("profile.backend_session_id")),
            "it reads the resume handle from the caller snapshot — it misses the thread id the other side gave after the snapshot and \
             resumes a stale thread: {binding:?}"
        );
        assert!(
            binding.iter().any(|l| l.contains(".get(profile.id)")),
            "it does not read the resume handle from the roster: {binding:?}"
        );
        // ★Absence is not swallowed★ — a deleted profile with `None` flowing on would silently make the resume a new conversation (see the comment on that line above).
        assert!(
            binding.iter().any(|l| l.contains("profile_vanished_mid_spawn")),
            "it does not cut on profile absence — swallowed with `and_then`, the resume silently becomes a new conversation: {binding:?}"
        );
    }

    /// Gives the function body between `start` and `end` in the production section **with comment lines removed and all whitespace erased** — rustfmt line splits give the same string; trailing commas
    /// (`,}`·`,)`) are folded.
    fn squashed_production_body(start: &str, end: &str) -> String {
        let src = include_str!("manager.rs");
        let production = src.split("mod tests {").next().expect("production section");
        let body = production
            .split(start)
            .nth(1)
            .unwrap_or_else(|| {
                panic!("`{start}` is not in the production section — this entry's premise is stale")
            })
            .split(end)
            .next()
            .expect("up to the next function");
        body.lines()
            .map(str::trim)
            .filter(|l| !l.starts_with("//"))
            .collect::<String>()
            .split_whitespace()
            .collect::<String>()
            .replace(",}", "}")
            .replace(",)", ")")
    }

    /// ★The queued input registry is carried on the spawn path (ADR-0231)★ — wiring pinned in the source: the core gets this incarnation's new registry + the manager's waiting-list table, registered **before** the sessions
    /// map insert. Omitting either compiles; the symptom is the user list invisible to mail (mail reaches stdin before user text).
    #[test]
    fn the_queued_registry_and_pending_table_are_wired_through_the_spawn_path() {
        let session = squashed_production_body("fn spawn_session(", "pub fn restore_all(");
        assert!(
            session.contains(
                ".with_queued(QueuedWiring{registry:Arc::new(QueuedInputs::new()),pending:self.inputs_pending.clone()})"
            ),
            "the spawn core is not plugged with this incarnation's new registry + the manager's waiting-list table"
        );
        assert!(
            session.contains(".with_mid_turn(mid_turn,delivery_ack)"),
            "the mid-turn input policy and ack value the backend reported are not carried on the session — all input goes to today's path"
        );
        let register = session
            .find("self.inputs_pending.register(id,epoch);")
            .expect("this incarnation is not registered in the waiting-list table — the table drops all of that incarnation's writes");
        let insert = session
            .find("self.sessions.write()")
            .expect("sessions map insert");
        assert!(
            register < insert,
            "the registration is after the sessions insert — the filled of the first list event in between is dropped"
        );
    }

    /// ★The first-submission latch is carried on the spawn path (ADR-0226)★ — wiring pinned in the source: ① `open_spawn`'s record port slot is the latch's `offer_sink` (no old port writing the roster) ② the `offer` of the
    /// issued value is **before** `open_spawn` and went through `session_id_to_hand_over` (input is accepted from the roster registration) ③ the same latch goes to `spawn_session`, which destructures `SpawnIncarnation`
    /// **by name** and calls **both** `.with_incarnation(` and `.with_session_id_latch(`; no old issue-and-persist verb. ★Do not remove ③'s `.with_session_id_latch(` assertion★ — the required argument cannot force
    /// carrying the latch into the session (`{ latch: _, .. }` compiles); without it no submission is counted and **every resume silently disappears**. Real-process pair: `tests/activation.rs`
    /// `d1_a_fresh_claude_persists_its_minted_id_only_after_the_first_submission`.
    #[test]
    fn the_first_submission_latch_is_wired_through_the_spawn_path() {
        let spawning =
            squashed_production_body("fn spawn_agent_watching_link(", "pub fn activate_profile(");
        assert!(
            spawning.contains("Some(latch.offer_sink())"),
            "the record port of `open_spawn` is not the latch's receiving port — the fetched id bypasses the latch"
        );
        // ★Submissions of the mode where the link owns the turn (codex JSON) come only through this port★ — missing, no incarnation of that mode can persist its id (the session does not count there — ADR-0226 amendment).
        assert!(
            spawning.contains("Some(latch.first_turn_sink())"),
            "the latch's first-turn port is not carried on `open_spawn` — the id of the mode where the link owns the turn is never persisted"
        );
        assert!(
            !spawning.contains("Some(session_id_sink("),
            "the port writing straight to the roster is handed to the link again — the id of a 0-turn incarnation is persisted"
        );
        assert!(
            spawning.contains(
                "letlatch=SessionIdLatch::new(profile.id,epoch,session_id_sink(self.profiles.clone(),profile.id,epoch,stored_handle))"
            ),
            "the latch's commit port is not bound to this incarnation's marker and start-time value"
        );
        // The value put into the latch must have gone through the issuing-axis decision — that decision itself is
        //   measured by `only_a_backend_that_assigns_ids_gets_one_handed_over`.
        assert!(
            spawning
                .contains("letsid=session_id_to_hand_over(&profile.command,mode,stored_handle);"),
            "the sid to put into the latch does not go through the issuing-axis decision (`session_id_to_hand_over`)"
        );
        let offer = spawning.find("ifletSome(s)=sid{latch.offer(").expect(
            "there is no line putting the issued value into the latch — the id of an issuing backend is never committed",
        );
        let open = spawning
            .find("backend::open_spawn(")
            .expect("`open_spawn` call");
        assert!(
            offer < open,
            "the offer of the issued value is after `open_spawn` — the first submission may go out without knowing the id"
        );
        assert!(
            spawning.contains(
                "SpawnIncarnation{continues_conversation:resume_session_id.is_some(),latch}"
            ),
            "the latch spawn made is not carried in `SpawnIncarnation`"
        );
        assert!(
            spawning.contains(
                "self.spawn_session(profile.id,spec,parts,epoch,seed_events,incarnation)"
            ),
            "`SpawnIncarnation` does not go to `spawn_session`"
        );

        let session = squashed_production_body("fn spawn_session(", "pub fn restore_all(");
        let head = session
            .find("letSpawnIncarnation{continues_conversation,latch}=incarnation;")
            .expect("`spawn_session` does not destructure `SpawnIncarnation` by name");
        let parts = session
            .find("letbackend::SpawnParts{")
            .expect("`SpawnParts` destructuring");
        assert!(
            head < parts,
            "the `SpawnIncarnation` destructuring is not at the function head"
        );
        assert!(
            session.contains(".with_incarnation(continues_conversation)"),
            "the incarnation facts are not carried on the session"
        );
        assert!(
            session.contains(".with_session_id_latch(latch)"),
            "the latch is not carried on the session — no submission is ever counted, and every resume silently disappears"
        );

        let src = include_str!("manager.rs");
        let production = src.split("mod tests {").next().expect("production section");
        let code: String = production
            .lines()
            .map(str::trim)
            .filter(|l| !l.starts_with("//"))
            .collect();
        for verb in [".ensure_session_id(", ".new_session_id("] {
            assert!(
                !code.contains(verb),
                "the old verb `{verb}` issuing and persisting at spawn came back to the production section"
            );
        }
    }

    /// ★Why it is looked at with a real process★: marker issuing is inside `spawn_agent`, and only a real spawn says whether it
    /// passes that place.
    #[cfg(windows)]
    #[test]
    fn a_fresh_respawn_of_a_reaped_agent_never_reuses_the_prior_epoch() {
        let manager = bare_manager();
        let profile = create(
            &manager,
            &std::env::temp_dir().to_string_lossy(),
            Some("epoch-reuse"),
        );
        let first = manager
            .spawn_agent(&profile, SpawnMode::Fresh)
            .expect("first spawn")
            .into_started()
            .expect("this call actually launches (not a duplicate request)");
        manager.kill_agent(first.id).ok();
        // Until the reaper collects it from the map — only after that does the second spawn pass the double-spawn guard.
        let reaped = (0..200).any(|_| {
            if manager.list_agents().is_empty() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
            false
        });
        assert!(reaped, "the session must be collected after kill (premise)");

        let second = manager
            .spawn_agent(&profile, SpawnMode::Fresh)
            .expect("Fresh respawn")
            .into_started()
            .expect("this call actually launches (not a duplicate request)");
        assert_ne!(
            second.epoch, first.epoch,
            "a Fresh respawn must not reuse the dead incarnation's marker"
        );
        manager.kill_agent(second.id).ok();
    }

    /// Confirms with a real process that the real spawn path really takes `register_for_spawn` (the unit test above is not looking at dead code). Two `cmd.exe` are launched briefly and killed.
    #[cfg(windows)]
    #[test]
    fn spawning_a_brand_new_agent_with_a_taken_name_gets_a_suffix() {
        let manager = bare_manager();
        let existing = create(
            &manager,
            &std::env::temp_dir().to_string_lossy(),
            Some("bob"),
        );
        let first = manager
            .spawn_agent(&existing, SpawnMode::Fresh)
            .expect("spawn of an existing agent")
            .into_started()
            .expect("this call actually launches (not a duplicate request)");
        assert_eq!(
            first.name, "bob",
            "launching an agent already in the roster is not a rename target"
        );

        let adhoc = agent_profile(&std::env::temp_dir().to_string_lossy(), Some("bob"));
        let second = manager
            .spawn_agent(&adhoc, SpawnMode::Fresh)
            .expect("ad-hoc spawn")
            .into_started()
            .expect("this call actually launches (not a duplicate request)");
        assert_eq!(
            second.name, "bob(1)",
            "a new-registration spawn gets a suffix"
        );

        manager.kill_agent(first.id).ok();
        manager.kill_agent(second.id).ok();
    }

    // ── Save refusal · failure — the two registry entries (ADR-0291 R17 · D2 · E1 · E3 · E4 · D8) ─────────────

    use crate::profile::scripted_store::{Outcome, ScriptedStore, FAILURES};

    fn scripted_manager() -> (AgentManager, Arc<ScriptedStore>) {
        let store = ScriptedStore::new();
        let profiles = Arc::new(crate::profile::ProfileRegistry::new(store.clone()));
        let presets = Arc::new(PresetRegistry::new(Arc::new(FilePresetStore::new(
            std::env::temp_dir().join(format!("engram-store-refusal-preset-{}", Uuid::new_v4())),
        ))));
        let tracker = Arc::new(SessionTracker::new(
            crate::session_tracker::TrackerConfig {
                enabled: false,
                poll_interval: Duration::from_secs(1),
            },
            Arc::new(|_, _| {}),
        ));
        let manager = AgentManager::new(Arc::new(NoopStatus), profiles, presets, tracker);
        (manager, store)
    }

    // E4 — the five versions with a caller: save `Err` → `PtyError::Store` · in-memory roster unchanged · dirty not set.

    #[test]
    fn try_create_agent_registers_nothing_when_the_store_does_not_save() {
        for failure in FAILURES {
            let (manager, store) = scripted_manager();
            store.set(failure);
            let profile = agent_profile("C:/store/create", Some("create"));

            let err = manager
                .try_create_agent(profile.clone())
                .expect_err("a save failure is an error");

            assert!(matches!(err, PtyError::Store(_)), "{failure:?}: {err}");
            assert!(manager.agent_snapshot(profile.id).is_none(), "{failure:?}");
            assert!(manager.roster().is_empty(), "{failure:?}");
            assert!(!manager.profiles.is_dirty(), "{failure:?}");
        }
    }

    #[test]
    fn try_delete_agent_keeps_the_agent_when_the_store_does_not_save() {
        for failure in FAILURES {
            let (manager, store) = scripted_manager();
            let agent = create(&manager, "C:/store/delete", Some("delete"));
            store.set(failure);

            let err = manager
                .try_delete_agent(agent.id)
                .expect_err("a save failure is an error");

            assert!(matches!(err, PtyError::Store(_)), "{failure:?}: {err}");
            assert!(manager.agent_snapshot(agent.id).is_some(), "{failure:?}");
            assert!(!manager.profiles.is_dirty(), "{failure:?}");
        }
    }

    #[test]
    fn try_rename_agent_keeps_the_name_when_the_store_does_not_save() {
        for failure in FAILURES {
            let (manager, store) = scripted_manager();
            let agent = create(&manager, "C:/store/rename", Some("before"));
            store.set(failure);

            let err = manager
                .try_rename_agent(agent.id, Some("after".into()))
                .expect_err(
                    "a save failure is an error — answering `Renamed` would make the caller see a rename that did not happen as done",
                );

            assert!(matches!(err, PtyError::Store(_)), "{failure:?}: {err}");
            assert_eq!(name_of(&manager, agent.id), "before", "{failure:?}");
            assert!(!manager.profiles.is_dirty(), "{failure:?}");
        }
    }

    #[test]
    fn try_reparent_agent_keeps_the_hierarchy_when_the_store_does_not_save() {
        for failure in FAILURES {
            let (manager, store) = scripted_manager();
            let lead = create(&manager, "C:/store/lead", Some("lead"));
            let helper = create(&manager, "C:/store/helper", Some("helper"));
            store.set(failure);

            let err = manager
                .try_reparent_agent(helper.id, Some(lead.id))
                .expect_err("a save failure is an error");

            assert!(matches!(err, PtyError::Store(_)), "{failure:?}: {err}");
            assert_eq!(
                manager.agent_snapshot(helper.id).and_then(|p| p.parent_id),
                None,
                "{failure:?}"
            );
            assert!(!manager.profiles.is_dirty(), "{failure:?}");
        }
    }

    #[test]
    fn try_set_agent_auto_restore_keeps_the_flag_when_the_store_does_not_save() {
        for failure in FAILURES {
            let (manager, store) = scripted_manager();
            let agent = create(&manager, "C:/store/auto", Some("auto"));
            assert!(!agent.auto_restore, "premise");
            store.set(failure);

            let err = manager
                .try_set_agent_auto_restore(agent.id, true)
                .expect_err("a save failure is an error");

            assert!(matches!(err, PtyError::Store(_)), "{failure:?}: {err}");
            assert_eq!(
                manager.agent_snapshot(agent.id).map(|p| p.auto_restore),
                Some(false),
                "{failure:?}"
            );
            assert!(!manager.profiles.is_dirty(), "{failure:?}");
        }
    }

    #[test]
    fn the_caller_verbs_commit_and_persist_when_the_store_saves() {
        let (manager, store) = scripted_manager();
        let lead = manager
            .try_create_agent(agent_profile("C:/store/ok-lead", Some("lead")))
            .expect("registers when saved");
        let helper = manager
            .try_create_agent(agent_profile("C:/store/ok-helper", Some("helper")))
            .expect("registers when saved");

        assert!(matches!(
            manager.try_rename_agent(helper.id, Some("renamed".into())),
            Ok(RenameOutcome::Renamed(name)) if name == "renamed"
        ));
        assert!(manager
            .try_reparent_agent(helper.id, Some(lead.id))
            .expect("saved"));
        assert!(manager
            .try_set_agent_auto_restore(helper.id, true)
            .expect("saved"));
        let saved = store.disk(helper.id).expect("on disk");
        assert_eq!(saved.display_name.as_deref(), Some("renamed"));
        assert_eq!(saved.parent_id, Some(lead.id));
        assert!(saved.auto_restore);

        manager.try_delete_agent(lead.id).expect("saved");
        assert!(store.disk(lead.id).is_none());
        assert!(manager.agent_snapshot(lead.id).is_none());
    }

    // D2 · E1 — internal changes without a caller (first-submission latch).

    /// ★The latch's record stands in memory and sets dirty on any save `Err`★ — dropping it would lose this incarnation's session id and the next activation the conversation (ADR-0226).
    #[test]
    fn the_latch_commit_port_keeps_the_id_in_memory_and_marks_dirty_on_any_save_error() {
        for failure in FAILURES {
            let store = ScriptedStore::new();
            let profiles = Arc::new(crate::profile::ProfileRegistry::new(store.clone()));
            let p = AgentProfile::new(
                "raw".into(),
                codex(crate::profile::AgentOutputFormat::StreamJson),
                std::path::PathBuf::from("."),
                vec![],
                true,
            );
            let id = p.id;
            profiles.upsert(p);
            let epoch = profiles.epoch_for_spawn(id).expect("a profile just put in");
            store.set(failure);
            let sid = Uuid::new_v4();

            session_id_sink(profiles.clone(), id, epoch, None)(&sid.to_string());

            assert_eq!(
                profiles.get(id).and_then(|p| p.backend_session_id),
                Some(sid),
                "{failure:?}"
            );
            assert!(profiles.is_dirty(), "{failure:?}");
            assert_eq!(
                store.disk(id).and_then(|p| p.backend_session_id),
                None,
                "{failure:?}: not on disk yet"
            );
        }
    }

    /// ★During a refusal the disk bytes stay the same★ — with a real file store: if `agents.json` becomes a newer edition while running, the latch's record stands only in memory (dirty) and the file does not change by a byte.
    #[test]
    fn a_refused_latch_commit_leaves_the_agents_file_bytes_alone() {
        let dir =
            std::env::temp_dir().join(format!("engram-manager-latch-refused-{}", Uuid::new_v4()));
        let profiles = Arc::new(crate::profile::ProfileRegistry::new(Arc::new(
            FileProfileStore::new(dir.clone()),
        )));
        let p = AgentProfile::new(
            "raw".into(),
            codex(crate::profile::AgentOutputFormat::StreamJson),
            std::path::PathBuf::from("."),
            vec![],
            true,
        );
        let id = p.id;
        profiles.upsert(p);
        let epoch = profiles.epoch_for_spawn(id).expect("a profile just put in");
        let newer = r#"{"schema_version":2,"profiles":[]}"#;
        std::fs::write(dir.join("agents.json"), newer).expect("write a newer-edition file");
        let sid = Uuid::new_v4();

        session_id_sink(profiles.clone(), id, epoch, None)(&sid.to_string());

        assert_eq!(
            profiles.get(id).and_then(|p| p.backend_session_id),
            Some(sid)
        );
        assert!(profiles.is_dirty());
        assert_eq!(
            std::fs::read_to_string(dir.join("agents.json")).expect("read"),
            newer,
            "during the refusal the dirty registry does not touch the newer-edition file"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    // E3 · D8 — the entry value of the launch path.

    /// ★Same verb, different entry — boot restore launches, manual wake-up refuses (E3)★: under the same save failure boot restore applies the pre-launch change to memory + dirty and launches (the post-launch `auto_restore`
    /// raise and the reaper's lowering are internal, in memory); manual activation's save `Err` does not launch and returns `PtyError::Store` with roster and dirty unchanged. ★Real process★: `Started` and the post-launch raise pass
    /// only a real spawn.
    #[cfg(windows)]
    #[test]
    fn boot_restore_starts_through_a_failing_store_while_a_manual_wake_refuses() {
        for failure in [
            Outcome::FailsIo,
            Outcome::Refuses(crate::profile::Refusal::Newer { found: 2 }),
        ] {
            let (manager, store) = scripted_manager();
            let profile = create(
                &manager,
                &std::env::temp_dir().to_string_lossy(),
                Some("restore-e3"),
            );
            store.set(failure);

            // The snapshot carries one field different from the roster — that field shows whether the registration (①) was applied.
            let mut restored = profile.clone();
            restored.name = "restored-snapshot".into();
            let outcome = manager.restore_one(&restored);

            assert!(
                matches!(outcome, RestoreOutcome::Started),
                "{failure:?}: {outcome:?}"
            );
            let mem = manager.agent_snapshot(profile.id).expect("in the roster");
            assert_eq!(
                mem.name, "restored-snapshot",
                "{failure:?}: the registration stood in memory"
            );
            assert!(
                mem.auto_restore,
                "{failure:?}: the post-launch raise stood in memory"
            );
            assert!(manager.profiles.is_dirty(), "{failure:?}");
            assert_eq!(
                store.disk(profile.id).map(|p| p.name),
                Some(profile.name.clone()),
                "{failure:?}: the disk is unchanged"
            );

            manager.kill_agent(profile.id).expect("a live session");
            let lowered = (0..250).any(|_| {
                let reaped = manager.list_agents().is_empty()
                    && manager
                        .agent_snapshot(profile.id)
                        .is_some_and(|p| !p.auto_restore);
                if !reaped {
                    std::thread::sleep(Duration::from_millis(20));
                }
                reaped
            });
            assert!(
                lowered,
                "{failure:?}: the reaper's lowering must stand in memory"
            );
            assert!(manager.profiles.is_dirty(), "{failure:?}");

            let mut woken = profile.clone();
            woken.name = "woken-snapshot".into();
            let err = manager
                .activate_profile(&woken, SpawnMode::Fresh)
                .expect_err("the caller entry stops on a save failure");

            assert!(matches!(err, PtyError::Store(_)), "{failure:?}: {err}");
            assert_eq!(
                manager.agent_snapshot(profile.id).map(|p| p.name),
                Some("restored-snapshot".to_string()),
                "{failure:?}: in-memory roster unchanged"
            );
            assert!(
                manager.list_agents().is_empty(),
                "{failure:?}: did not launch"
            );
            assert!(
                manager.profiles.is_dirty(),
                "{failure:?}: the refused request does not clear the pending change"
            );
        }
    }

    /// A profile that stays claude so session id clearing and resume branches run, but process opening fails: working folder and child PATH are the same empty folder (a regression past the save `Err` would not launch real claude
    /// and the first assertion catches it); the caller deletes `cwd`. ★Blocked by PATH, not program name★: the backend pins `cmd.exe /c claude`, and an empty-folder PATH makes CreateProcessW fail on `cmd.exe` (os error 2) — only
    /// the PTY link (JSON output makes std `Command` search System32). ★The folder must exist★ — portable-pty replaces a nonexistent one with the home folder (real claude launched).
    fn claude_profile_that_cannot_launch() -> AgentProfile {
        let dir = std::env::temp_dir().join(format!("engram-no-claude-{}", Uuid::new_v4()));
        std::fs::create_dir(&dir).expect("create an empty folder");
        let path = dir.to_string_lossy().to_string();
        AgentProfile::new(
            "raw".into(),
            AgentCommand::Claude {
                extra_args: vec![],
                output_format: crate::profile::AgentOutputFormat::Terminal,
            },
            dir,
            vec![("PATH".into(), path)],
            false,
        )
    }

    /// ★A resume activation also returns the save `Err` in its original form (caller entry)★ — folded into the restore report as `PtyError::SpawnFailed`, the bus could not tell a save refusal from other failures. Both a
    /// resume with a handle and one without (new-conversation branch); the launch stops at registration (①), no process.
    #[test]
    fn a_resume_activation_returns_the_store_error_unflattened() {
        for failure in FAILURES {
            for stored in [Some(Uuid::new_v4()), None] {
                let (manager, store) = scripted_manager();
                let mut profile = claude_profile_that_cannot_launch();
                profile.backend_session_id = stored;
                manager.profiles.upsert(profile.clone());
                store.set(failure);

                let err = manager
                    .activate_profile(&profile, SpawnMode::Resume)
                    .expect_err("the caller entry stops on a save failure");

                assert!(
                    matches!(err, PtyError::Store(_)),
                    "{failure:?} · handle {stored:?}: {err}"
                );
                assert!(
                    manager.list_agents().is_empty(),
                    "{failure:?} · handle {stored:?}: did not launch"
                );
                assert_eq!(
                    manager
                        .agent_snapshot(profile.id)
                        .and_then(|p| p.backend_session_id),
                    stored,
                    "{failure:?}: in-memory roster unchanged"
                );
                assert_eq!(
                    manager
                        .agent_snapshot(profile.id)
                        .and_then(|p| p.last_failure),
                    Some(AgentFailureKind::Other),
                    "{failure:?} · handle {stored:?}: records the failure before returning the original (E2) — as other, not as a launch \
                     failure"
                );
                assert!(!manager.profiles.is_dirty(), "{failure:?}");
                let _ = std::fs::remove_dir(&profile.cwd);
            }
        }
    }

    /// ★The save `Err` of a new-conversation activation is also recorded as 「other」★ — no process open was tried, so not `SpawnFailed` (executable · permission). The launch stops at registration (①).
    // ADR-0291 R17
    #[test]
    fn a_fresh_activation_the_store_refuses_records_other_not_spawn_failed() {
        for failure in FAILURES {
            let (manager, store) = scripted_manager();
            let profile = claude_profile_that_cannot_launch();
            manager.profiles.upsert(profile.clone());
            store.set(failure);

            let err = manager
                .activate_profile(&profile, SpawnMode::Fresh)
                .expect_err("the caller entry stops on a save failure");

            assert!(matches!(err, PtyError::Store(_)), "{failure:?}: {err}");
            assert_eq!(
                manager
                    .agent_snapshot(profile.id)
                    .and_then(|p| p.last_failure),
                Some(AgentFailureKind::Other),
                "{failure:?}"
            );
            assert!(
                manager.list_agents().is_empty(),
                "{failure:?}: did not launch"
            );
            let _ = std::fs::remove_dir(&profile.cwd);
        }
    }

    /// ★Partial success of the caller entry is not reverted (D8)★ — if ③ (session id clearing) save is `Err` after ① (registration) saved, the registration stays, the field keeps the old handle, and it does not launch
    /// (stops at ③, no process).
    #[test]
    fn a_caller_spawn_whose_release_save_fails_keeps_the_registration_and_the_old_handle() {
        for failure in FAILURES {
            let (manager, store) = scripted_manager();
            let old = Uuid::new_v4();
            let mut profile = claude_profile_that_cannot_launch();
            profile.display_name = Some("d8".into());
            profile.backend_session_id = Some(old);
            store.script(&[Outcome::Saves], failure);

            let err = manager
                .spawn_agent(&profile, SpawnMode::Fresh)
                .expect_err("stops on the save failure of the session id clearing");

            assert!(matches!(err, PtyError::Store(_)), "{failure:?}: {err}");
            let mem = manager
                .agent_snapshot(profile.id)
                .expect("the registration is not reverted");
            assert_eq!(
                mem.backend_session_id,
                Some(old),
                "{failure:?}: the clearing was not applied, so the old handle remains"
            );
            assert!(mem.old_session_ids.is_empty(), "{failure:?}");
            assert_ne!(
                mem.epoch, profile.epoch,
                "{failure:?}: the new incarnation marker remains in memory"
            );
            assert!(
                manager.list_agents().is_empty(),
                "{failure:?}: did not launch"
            );
            assert!(!manager.profiles.is_dirty(), "{failure:?}");
            assert_eq!(
                store.attempts(),
                2,
                "{failure:?}: only the two saves of registration · clearing"
            );
            assert_eq!(
                store.disk(profile.id).and_then(|p| p.backend_session_id),
                Some(old),
                "{failure:?}: the registration was saved"
            );
            let _ = std::fs::remove_dir(&profile.cwd);
        }
    }
}
