//! AgentManager — binds backend/transport/output_core/session together to manage the agent lifecycle.
//!
//! zero tauri imports — upper-layer status notifications are injected through the StatusSink trait (not AppHandle).
//!
//! ★Single owner of the roster (ADR-0119)★: "all agents + each one alive/asleep" is built only in one place, `roster()`.
//! The profile registry is **inside** this type and no handle goes out — the outside uses only narrow
//! verbs (create/delete/rename/reparent/set-auto-restore/snapshot).
//! Canonical names are unique across the whole roster (ADR-0120), and the enforcement points are only three: creation, new registration (spawn) and rename.
//!
//! Lock order (LLD §10 rule 1): the `sessions` RwLock is lookup-only. Clone the Arc<AgentSession> and
//! release the lock immediately, and only then take session-internal locks (core/transport). Taking session-internal locks
//! while holding the sessions lock is forbidden (deadlock prevention).

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

/// If, within this time after a resume spawn, it exits abnormally (code≠0/Failed/Killed) or **the diagnostic stream says it failed**,
/// it is decided as a resume failure (H-1.7 "early exit window"). A successful resume is a TUI, so it stays up.
///
/// ★Do not solve it by enlarging this (measured 2026-08-23)★: real claude emits the diagnostic at +2.2s and exits only at +6.4s
///   — a decision waiting only for death would need this window enlarged past 6s, but activation **blocks** for this window,
///   so every healthy resume would be that much slower. So the decision ground was changed from 「death」 to 「death or evidence」
///   (`EarlyVerdict`) — it actually settles failures **faster**.
// ADR-0172
const EARLY_EXIT_WINDOW: Duration = Duration::from_secs(3);

/// A liveness backstop used only when the link **never produces the connection outcome**.
///
/// ★★This is not a decision window — the decision is the connection itself★★ (user decision): resume success is not 「did not die within N seconds」
///   but **「the agent signaled it is ready to take input」**, and for codex that signal is
///   handshake completion (`Link::Up`). A refusal is also answered **immediately** by the other side (measured). So in both directions there is nothing
///   to wait for, and [`AgentManager::early_activation_verdict`] returns **the moment** the connection produces its outcome.
///   ★The 「connection decision window」 once here (a policy of waiting the full 15 seconds) was removed by that decision — do not bring it back★.
///
/// ★Still, there is **exactly one** reason this value remains★: a path where the link **cannot** produce the outcome exists.
///   The handshake cap of [`crate::backend::codex`] applies only to **waiting for responses**, not to **writes** —
///   if the other side does not read our stdin, `write_all` hangs forever on pipe backpressure, and then
///   the writer never reaches the failure branch and the link stays stuck in `Connecting` forever. In that state, without this backstop,
///   `activate_profile` blocks **forever** (one of the daemon's async dispatch workers is tied up entirely).
/// ★So what this value does is not 「deciding」 but 「giving up」★ — reaching here means the other side neither answers nor
///   accepts our writes, and that outcome is a failure and **the trigger of the teardown below** (that kill also releases the hung
///   write).
/// ★The value must be larger than the link's handshake cap★ — if smaller, this side intercepts **before** the link settles the failure itself,
///   and a reasonless give-up covers a refusal with a reason. That ordering is measured directly by the test rig.
pub(crate) const LINK_RESOLUTION_BACKSTOP: Duration = Duration::from_secs(15);
/// Spawn interval between agents during restore (stagger to prevent concurrent bursts).
const RESTORE_STAGGER: Duration = Duration::from_millis(200);

/// The outcome of one spawn request. ★The caller **must be able to tell** "launched" from "there was nothing to do"★ —
/// so that cleanup such as registration, epoch and recording is not done on a session it did not create.
///
/// ★A duplicate request is not an error (design decision — do not revert)★: if that entry is already up or already launching,
///   this request is **moot**. It used to be answered with `Err`, so every consumer had to know "this error is not a real
///   failure" — that list was fixed three times and one item was missed each time. Making it an outcome instead of an error
///   removes the very list to miss. The frontend is already the same shape (the in-flight guard of `AgentList` silently lets a second
///   activation pass).
/// ★moot changes nothing★: having done nothing, there is no failure to record and no ground to clear
///   (`AgentManager::note_spawn_result`).
// ADR-0172
#[derive(Debug, Clone)]
pub enum SpawnOutcome {
    /// This call launched an incarnation.
    Started(AgentInfo),
    /// **This request had nothing to do** — that entry was already up or already launching, so nothing was done.
    ///
    /// `Some` = the live session could be seen at that moment (that information is returned as is) · `None` = the winner had not yet put it on the roster,
    /// so there was nothing to see. Either way **this call created nothing**.
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

/// Hard cap on the output tail the failure classification looks into (`OutputCore::terminal_tail` keeps it exactly). The exit text we
/// look for is in the last few lines. This value does not go out to the screen or logs (classification input only).
// ADR-0172
const FAILURE_TAIL_BYTES: usize = 4096;

/// The early decision outcome of a resume attempt — split not by 「did it die」 but by 「what evidence stood」.
///
/// ★Why `Diagnosed` exists (measured 2026-08-23 — do not revert)★: real claude, for `--resume <empty sid>`,
///   **emits the diagnostic at +2.2s and dies only at +6.4s** (the SessionEnd hook runs in between). Waiting only for death
///   passes the early-exit window (3s) and **the failure is decided as a success and even the earlier record is cleared** — it was the place where this feature's headline
///   case died entirely. If the evidence already stood, it does not wait for the corpse.
/// ★Enlarging the window is not the answer to this problem★: activation **blocks** for this window, so enlarging it would make every healthy
///   resume that much slower. The answer is not waiting but changing the ground of judgment.
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
    /// It passed the window and no evidence stood = activation holds.
    ///
    /// ★This is **weak** success evidence★ — it is only 「did not die within N seconds」, and whether that agent can take input is
    ///   unknown. Links without a connection axis (claude · shell · stdio) have only this, so it is used as is.
    Alive,
    /// The agent **signaled it is ready to take input** = activation holds (user decision).
    ///
    /// ★There are two reasons it is split from `Alive`★:
    ///   ① **the kind of evidence differs** — this one is a positive signal from the other side (codex handshake completion), and that one is
    ///      an observation that nothing happened. So this one has nothing to wait for (it ends the moment the signal comes).
    ///   ② **the disposition of 「last failure」 differs** — this optimistic success **does not clear** the record (user condition).
    ///      The canonical clearing rule is at that place in `AgentManager::resume_no_fallback`.
    Ready,
    /// The link **could not set up the connection** — the process may be alive.
    ///
    /// ★Why it is split from `Terminal`★: that one is 「died」 and this one is 「alive but unusable」. The disposition is
    ///   the same failure, but the reason sentence and the classification input differ, and above all **this branch has no corpse** — the console
    ///   tail and the diagnostic tail are both empty and the only evidence is `reason`.
    LinkFailed { reason: String },
}

/// One roster entry = **one agent** (ADR-0119 decision 1). Removing the duplication of consumers each merging "the live list" and "the profile list"
/// is the reason this type exists.
///
/// ★A corpse left in the map in a terminal state is not an entry★ — it is neither alive nor asleep. If the profile remains, it is
///   asleep; if not (ad-hoc), it is not in the list at all.
#[derive(Debug, Clone)]
pub struct RosterEntry {
    pub id: AgentId,
    pub canonical_name: String,
    /// `Some` = alive (a Running|Exiting session attached) · `None` = asleep (profile only).
    pub live: Option<AgentInfo>,
    /// Working folder. For live entries the session cwd (canonicalized at spawn), for asleep entries the stored raw cwd — **the same material the name derivation
    /// looks at** (the "live/asleep sources differ" discipline of `roster()`). An empty string if there is neither a session nor a profile.
    pub cwd: String,
    /// Tree parent. The stored hierarchy is the only source, so live entries also read it from the profile — `None` = top level.
    pub parent: Option<AgentId>,
}

/// **Today the only path actually carrying an override is rename** — the only wire command carrying a display name override is
/// `RenameProfile`, and the creation and spawn side does not have that field at all. So the calls on the `try_create_agent`·
/// `register_for_spawn` side are a public API defense line.
///
/// ★The stored name · the name drawn on screen · the mail address must be **the same string**★: the uniqueness (ADR-0120) decision is
///   a string comparison, so `bob` and `" bob "` **both** pass as different names while being drawn the same in the tree —
///   the user cannot tell which of the two mail goes to. Moreover, the messaging entry trims the recipient token to
///   match, so an agent stored as `" bob "` is visible but cannot be addressed by name.
/// ★Do not delete that entry trim (the `messaging` service recipient matching)★: **the CLI entry** (`engram mail send --to a,b`)
///   splits the recipient list by commas because of shell constraints without removing spaces then (`"alice, bob"` → `["alice", " bob"]`),
///   and that trim rescues the second and later recipients. Normalizing the stored value only makes that trim a
///   no-op for well-stored names, and the comma-list rescue role remains as is.
/// ★Applied **before** the uniqueness decision★: the decision and the storage must see the same normalized value so that `" bob "` becomes a `bob` request
///   in every respect (applied after, `" bob "` would be decided as an empty name and same-name entries would leak in again).
/// ★Inner whitespace is part of the name★ — `"bob smith"` must survive as is, so only both ends are trimmed.
/// ★What remains is not "the residue of the same hole" but a different kind★: `str::trim` removes only Unicode White_Space (space · tab ·
///   NBSP), and zero-width characters (U+200B etc.) fall off **on neither side** — so for such names
///   stored == displayed == address holds as is and does not break the invariant above (what padded names broke was *address reachability*).
///   What remains is only **visual confusion** that the eye cannot tell apart, and its solutions (NFKC · confusable folding) would mash even legitimate
///   names, so it is not handled here with an improvised filter — it is a policy decision.
/// ★Already stored names are not fixed★ — it applies only to writes from now on (no migration device).
fn normalize_display_name(display_name: Option<String>) -> Option<String> {
    let trimmed = display_name?.trim().to_string();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

/// The form of `name` when it is in the `base` family. ★**Never mix** `Exact` and `Suffixed(0)` into one value★:
/// mixing would make one literal `bob(0)` look as if it occupied the unsuffixed `bob`, so even though `bob` is free
/// the next request would get `bob(1)`.
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
    // Leading zeros (`base(01)`) are not our notation either — the issuer never writes that. Leaving them out of the family
    // creates no duplicate: `base(01)` is a different string from any issuable string (`base(1)` etc.).
    if digits.len() > 1 && digits.starts_with('0') {
        return None;
    }
    digits.parse::<u32>().ok().map(NameKind::Suffixed)
}

/// ★The total roster cap = **a runaway backstop**, not a product limit★ (user decision 2026-08-11).
///
/// The control plane is open to all agents (ADR-0132 decision 5) and that decision stays — but if something
/// calls "create agent" repeatedly, the roster and `agents.json` grow **without bound**. Only that loop is cut.
///
/// ★Why the core (not the entry)★: putting the cap at the entry makes a copy per entry, and an entry without a copy just
///   passes — when it was actually put only on the control route, the desktop (WS `CreateProfile`) and ad-hoc spawn slipped through as is.
///   Putting it where the registration **is committed** makes it true regardless of the number of entries, and being inside the name allocation gate it is
///   **atomic** (no window where several requests each observe 511 and all register together).
/// ★Why a count check (not rate limiting)★: with no timers, per-caller state or tuning knobs, this defense cannot
///   break by itself. The only thing counted is the total roster size.
/// ★Why this number — **do not tune it**★: it is enough for it to sit where legitimate use cannot reach. Real team
///   trees are in the tens. **Do not use the argument "resources run out first, so the cap is a formality"** — that argument
///   works only for registrations that launch processes, not for **registering asleep agents**, which launch no process at all
///   (that side pushes nothing out, so there is no physical brake at all). That is exactly why a cap is needed.
/// ★The existing roster is not a hostage★: the cap looks only at **new registrations** — restore and respawn of a roster already over the cap
///   run as is (the existing-id branch of `register_for_spawn` does not pass this check).
// ADR-0132
pub const MAX_ROSTER_SIZE: usize = 512;

/// The total check before a new registration — called **only while holding the name allocation gate** (that makes it atomic).
///
/// ★It is not atomic with deletion (intended)★: deletion does not take this gate but only **decreases** the count, so the worst case is
/// "a slot just freed up but this call is refused", and the next call passes. Safety in the direction a backstop needs.
fn check_roster_capacity(roster: &[RosterEntry]) -> Result<(), PtyError> {
    if roster.len() >= MAX_ROSTER_SIZE {
        return Err(PtyError::RosterFull {
            current: roster.len(),
            limit: MAX_ROSTER_SIZE,
        });
    }
    Ok(())
}

/// The single error text for suffix space exhaustion — refuses instead of giving up uniqueness and issuing a duplicate name (name =
/// address, so a duplicate translates into misdelivered mail, ADR-0116).
///
/// ★Unreachable in reality★: `pick_suffix` emits `None` only when **all** of 1..=u32::MAX of the family are occupied
/// (4.2 billion entries in the roster), so this refusal does not actually fire. Saturation is absorbed before that by "the lowest free number".
/// Still, the only reason this path and `RenameOutcome::Exhausted` are kept is to close the decision table as a total function —
/// do not read it as a living policy.
/// ★No dedicated error variant is made★: the only fact the caller needs is "this verb could not be performed now", and
/// that is the same shape as the existing unsupported signal (the same judgment as `write_stdin_observed_if_epoch`).
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

/// Rename outcome — the caller must be able to **tell apart** the failure reasons. With a bool, "no such agent" and
/// "cannot issue a name" would be mashed into the same value and the wire response would state a false cause.
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

/// `used` = the set of numbers that family currently occupies.
///
/// The basic rule is ADR-0115 as is, **observed max + 1** (monotonically increasing within the same roster — numbers do not overlap
/// among live ones). There is no separate high-water state, so when the family becomes empty the numbers start again from 1 (ADR-0123).
///
/// ★Saturation escape (saturation-only)★: if the max is `u32::MAX`, there is no "max + 1". Only then does it go down to **the lowest free
/// number** — otherwise one `name(4294967295)` would permanently block the 4.2 billion free numbers of that family.
/// The normal path keeps monotonic increase, and this branch opens only when the max is MAX.
/// `None` = 1..=MAX are all occupied (unreachable in reality — a terminal branch to keep it a total function).
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

/// One connection outcome the link delivered — ★carries **together** which incarnation it belongs to★.
///
/// ★The marker being attached to the value is the point of this type★: if the outcome and the incarnation travel separately, the outcome of a collected incarnation
///   is applied to the successor that just launched (that cleanup **kills a live session**). Consumers look at this marker **only for match/mismatch**
///   — "newer" is not derived from ordering (ADR-0163/0164).
#[derive(Debug, Clone)]
pub(crate) struct LinkVerdict {
    pub(crate) epoch: u32,
    pub(crate) resolution: LinkResolution,
}

/// The set a caller waiting for the connection outcome holds — **the mailbox and the spawn reservation are one body**.
///
/// ★Binding the two fields is the point★: on a link that declares a connection, between 「the session went onto the roster」 and 「input can be sent to that session」
///   there is a gap of one handshake round trip. If a second activation coming into that interval
///   looked only at the roster and answered 「it is up」, the moment the first request decides a failure and collects that incarnation, **the two callers
///   split holding different facts** (the latter reports an agent about to die under it as alive).
///   Holding the reservation until the outcome makes the answer in that interval one: 「already launching」.
/// ★So **dropping this value releases the reservation immediately**★ — [`AgentManager::spawn_agent`] does that.
///   That one is a branch that does not wait for the outcome, so it has no ground to hold the reservation either.
/// ★On links without a connection axis this type does not come at all★ (`Option` is `None`) — on that path both the reservation lifetime
///   and the decision are as before.
struct LinkWatch {
    rx: Receiver<LinkVerdict>,
    /// Not released until the outcome happens — not reading it is normal (a lifetime is all this field is).
    _reservation: SpawnReservation,
}

/// Builds the delivery port to hand to the link — the incarnation marker is stamped **here** and passed into the channel.
///
/// ★The link does not know the incarnation★ (the same shape and reason as [`session_id_sink`]): the marker is an assembly-point concept, and putting it
///   into the link would leak that knowledge into the backend layer (ADR-0004).
/// ★Swallowing a send failure is intended★ — it means the receiver has already left (the decision finished, or it is a Fresh spawn so
///   nobody waits in the first place), and then there is nothing the link can do or needs to know.
fn link_verdict_sink(tx: Sender<LinkVerdict>, incarnation: u32) -> LinkSink {
    Arc::new(move |resolution: LinkResolution| {
        let _ = tx.send(LinkVerdict {
            epoch: incarnation,
            resolution,
        });
    })
}

/// The profile vanished during spawn — **the spawn is aborted**. `at` = at which stage it was noticed.
///
/// ★Why the stage name is carried★: `spawn_agent` has **three** places for this decision (settling the incarnation marker · Fresh's session
///   id clearing · Resume's reading of the resume handle). Which place was hit tells when that deletion slipped in,
///   but with the same text they cannot be told apart in logs and errors.
/// ★All three having the same disposition is the discipline★ — if even one swallowed `None`, a session would launch with a deleted profile or
///   a resume would silently become a new conversation.
fn profile_vanished_mid_spawn(id: AgentId, at: &str) -> PtyError {
    PtyError::SpawnFailed(format!(
        "profile {id} vanished mid-spawn at [{at}] (concurrent delete) — spawn aborted"
    ))
}

/// The kind to write into 「the last failure」 when the launch ended with `Err` before producing a process.
///
/// ★A save `Err` is not `SpawnFailed` but 「other」★ — the store did not accept the pre-launch roster change, so there was no attempt to open
///   a process. Recording it as `SpawnFailed` (missing executable · permission etc.) would make the screen and the LLM suspect the execution environment.
// ADR-0291 R17
fn spawn_error_kind(e: &PtyError) -> AgentFailureKind {
    match e {
        PtyError::Store(_) => AgentFailureKind::Other,
        _ => AgentFailureKind::SpawnFailed,
    }
}

/// Which change entry of the registry to change through — decided by whoever called the change. The verbs below only send to the registry
/// verb of that entry, and the `Internal` branch emits no `Err` (even a save failure commits + dirty).
///
/// ★The launch path carries this one value all the way to the shared function — do not keep per-entry copies of the launch★: boot restore and
///   manual wake-up using the same verb is the device closing 「the decision splits per entry and one falls behind」,
///   which ADR-0082 · ADR-0201 · ADR-0202 blocked.
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

/// Whether a Fresh spawn **must start by clearing the stored session id**.
///
/// ★Both true conditions are required★
///   1. **It is Fresh.** Clearing on Resume would lose the handle to resume (the very value ADR-0083 guards).
///   2. ★**This incarnation has a path to fill that field**★ — there must be at least one ground to write a new value into that field at the first submission.
///      Clearing without one would make a hand-planted value disappear with nobody filling it again.
///
/// ★The enumeration of 2 is **managed by hand**★: we draw it and write it at the first submission ([`backend::assigns_session_id`]) ·
///   link connection ([`backend::declares_link`] — that link receives it in the handshake response and passes it to the latch) · the control
///   plane ([`backend::supports_control_channel`]). If a fourth path appears it must be added here, and the compiler cannot catch that
///   omission — there is no field declaring the real axis (「do we come to know this incarnation's id」).
/// ★Where this decision actually splits today★: claude · gemini (issuing) · codex are true, shell is false (there is no path).
/// ★**But codex's terminal mode being true today is 「by chance」 — when a change turning off the control channel comes, look at this
///   line first**★: the reason that field is on is MCP mail (ADR-0214), and the path by which that mode actually fills this field
///   is not the control plane but **the writer lock the child holds** (ADR-0218). The answer is right but the reason is wrong,
///   so if that field were turned off for mail reasons, this decision would **silently** flip to false. The app-server
///   mode is not like that — there `declares_link` is separately true, so it stands regardless of the control channel.
///
/// ★What happens without it (do not bring it back)★: Fresh opens a new conversation per incarnation, but if the field stays filled with the previous incarnation's value,
///   that field is the very field `resume_session_id` below reads, so **the resume rewinds to the previous conversation.**
///   The same goes for issuing backends — the new value is written only at the first submission, so after an incarnation that ended before that, the old value remains.
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

/// The **resume handle** this spawn hands to the backend — ★`None` on Fresh even if there is a stored value★.
///
/// ★This one line is where 「`resume_session_id` exists ⟺ it is a resuming incarnation」 is made to hold★. There are two consumers relying on that
///   equivalence and they look at different arguments: the backend's argv assembly looks at both `(mode, handle)`,
///   and the decision of whether to run reclamation looks at **only** the handle. For the two decisions to diverge, a handle would have to go out
///   on Fresh, and the only place that can make that combination is here, so it is blocked here.
/// ★Not passing the mode along to the backend is a decision★ — it is what that side's contract (the `resume_session_id` doc of
///   `AgentBackend::open_spawn`) wrote: 「receiving both the mode and the value makes the combination "Fresh but with a value to resume" expressible」,
///   and this function is the real thing creating that inexpressibility.
// ADR-0185
// ADR-0218
fn resume_handle_for(mode: SpawnMode, stored: Option<uuid::Uuid>) -> Option<uuid::Uuid> {
    match mode {
        SpawnMode::Resume => stored,
        SpawnMode::Fresh => None,
    }
}

/// **Our side's session id** this spawn hands to the backend and puts into the first-submission latch — `Some` only for issuing-axis backends
/// (Fresh = a newly drawn value · Resume = the stored value spawn read).
///
/// ★Decided by the issuing axis (`assigns_session_id`) alone — do not change it to the resume axis (ADR-0185)★: giving a value to a backend
///   that issues its own id (codex) would put that value into the latch before `open_spawn`, the backend would
///   drop it from argv, and the real id the other side gave would be dropped as the latch's second offer. The first submission would persist that fake value and
///   **every later resume would silently fail** (ADR-0226).
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

/// Builds the **commit port** that writes this incarnation's session id into the profile roster — the caller is that incarnation's first-submission
/// latch ([`SessionIdLatch`]), and the latch calls this port at most once per incarnation.
///
/// ★Writing is compare-and-swap ([`ProfileRegistry::commit_session_id`])★ — it writes only when the field is `expected` or already that value.
///   `expected` = **the value of the field this incarnation saw when it started** (Fresh = `None`, after clearing · Resume = the stored value spawn
///   read). When the commit succeeds, the port moves `expected` to that value — a defensive clause: the latch calls it only once per
///   incarnation, so in production the moved value is not used again.
/// ★Do not revert it to an overwrite (`observe_session_id`)★ — if the claude file watcher changed the field before the first submission,
///   an overwrite would rewind that value to the value at spawn (that verb's doc is canonical).
/// ★Why the `String` → `Uuid` decoding is here★: what the other side gives is a string (codex `Thread.id`), and what the profile
///   holds is a `Uuid`. Narrowing that width is profile schema knowledge, so it cannot be placed in `backend/` (ADR-0004).
/// ★**Do not assume the value coming in here is v4 — the measured one is UUIDv7**★: what the real app-server gave was
///   `01a0a08f-…` (version nibble `7`). So in the one field `backend_session_id`, **a different UUID version per backend**
///   sits — for claude we draw a v4 and hand it over, for codex we record what we receive. This repository's test rigs all
///   build values with `Uuid::new_v4()`, so they never once experience that difference. ★Do not add version validation★ —
///   `Uuid::parse_str` does not look at the version, and that is why this path accepts both versions.
/// ★If it cannot be read as a uuid, it is logged and **dropped**★ — neither a panic nor an `unwrap`. This call runs outside the containment marker (codex
///   writer thread · the reader's marker-off section · the input submission path), so a panic here would let the hook end the daemon
///   just because the other side's format changed once (ADR-0292). Failing only to record is better (then the resume falls to Fresh and
///   is reported honestly).
/// ★The value itself is not logged★ — what is recorded is only the decoding failure reason.
/// ★Pins the incarnation marker (ADR-0007/0163)★ — this port is bound to **one spawn**, and that spawn's marker is already settled at call
///   time. So a record arriving after **a newer incarnation already stood** is refused.
///   ★Do not widen it to 「it blocks late records」★ — when the session **only ended**, the marker stays the same, so
///   this guard does not stand (the canonical source of that window = the link's `record_session_id` doc).
/// ★The three outcomes are split in the logs (「instrumentation duty」 of `docs/reference/logging-conventions.md`)★ — recorded (info) ·
///   refused (debug) · uuid decoding failure (warn). **The outcome of not receiving** does not come here; the link's handshake failure log
///   emits it. The outcome of receiving but being held back for lack of a submission is emitted by the latch.
/// ★Do not read `true` as 「it was persisted」★ — what that value means is only that the in-memory roster changed, and even if the disk
///   save fails or is refused, the memory change remains (the store logs it in its own place).
/// ★The roster is called while holding the `expected` field★ — the lock order is `latch → this field → profiles → store-state field` one-way, and
///   the only place taking this field is this port.
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
        // ★Do not put the record call in a match guard★ — a guard with side effects silently gets called 0 or 2 times if just one arm slips in front.
        //   The decision and the branching are kept apart.
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
            // ★The reason cannot be told apart — what comes back is one `bool`★: incarnation marker mismatch · another writer changed it
            //   first · profile absent · already the same value, all four come as the same `false`. Written as is without making things up.
            tracing::debug!(
                agent = %id,
                epoch = incarnation,
                "the session id the backend gave was not reflected into the roster — one of incarnation marker mismatch · another writer · profile absent · same value"
            );
        }
    })
}

/// The incarnation facts spawn carries into the session — a **required** argument of [`AgentManager::spawn_session`].
///
/// ★Do not change it to `Option` or give it a default★: if the latch is not carried into the session, no submission is ever counted, so no
///   incarnation can persist its session id, and **every resume silently disappears** (every next activation is a new conversation —
///   with no error). It is the shape that blocks with the compiler the way a default could reach production. Carrying the received latch into the session
///   cannot be forced by the compiler, so a wiring structure test measures it.
/// `continues_conversation` = this spawn carried a resume handle (not whether the resume succeeded).
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
    // ADR-0061: single owner of presets (cwd bookmarks). Held by the daemon just like profiles (single ownership of user data,
    // ADR-0029). The reaper does not look at presets (unrelated to agent lifetimes), so it is kept only as a manager field.
    presets: Arc<PresetRegistry>,
    tracker: Arc<SessionTracker>,

    // ── ADR-0019 reaper ──────────────────────────────────────
    /// Daemon/app shutdown global flag. shutdown_all sets it **before** each kill → the finish hook of a session that ended
    /// in between snapshots true, and the reaper skips the disposition (boot restore kept).
    shutting_down: Arc<AtomicBool>,
    /// The channel through which the session/pump finish hook sends ReapMsg (consumed by the single supervisor).
    reaper_tx: Sender<ReaperCmd>,
    /// Reaper thread handle. On Drop, join (wait after sending Stop) — prevents test leaks.
    reaper_handle: Option<JoinHandle<()>>,

    /// ADR-0086 control channel provisioning seam. On spawn, provision (issue token + mcp-config), on terminal
    /// the reaper revokes (discard + delete file). Only the daemon injects the real implementation (`DaemonControlChannel`), and the default is
    /// NoopControlChannel (no control channel — headless tests · shell-only paths). Arc, so it is shared with the reaper.
    control: Arc<dyn ControlChannel>,

    /// ADR-0086 provision race guard (FIX 6) — the set of AgentIds currently being spawned (reservations). If **another connection** spawns the same
    /// AgentId concurrently in the TOCTOU window between the contains_key guard (read lock) and the actual sessions.insert (write lock),
    /// both would call provision and write to the same (AgentId,epoch) config path, and one side's reaper could wrongly delete the other's live session.
    /// On entry it reserves atomically in this set (immediate Err if already present)
    /// to cleanly refuse the second concurrent spawn. The reservation is released on drop by SpawnReservation (RAII) on either success (registration done)
    /// or failure (any early return). ★A leaf lock separate from the sessions map★: while holding this Mutex,
    /// sessions/status locks are not taken (ADR-0006 — a short critical section, pure HashSet manipulation).
    spawning: Arc<Mutex<HashSet<AgentId>>>,

    /// The name allocation gate — a stateless serializing lock. The one invariant it guards: **between observing the roster and
    /// committing a name from that result, no other allocation can slip in** (without it, two concurrent creations would see the same name as free
    /// and both take it). The whole decision table — derivation · observation · commit — happens inside this lock.
    ///
    /// ★Lock order = name_allocation → sessions/profiles one-way★. The places taking this lock are three
    /// (`try_create_agent`·`try_rename_agent`·`register_for_spawn`), and all three touch the roster only **after** taking it.
    /// The reverse order (taking this lock while holding profiles) does not exist, so there is no cycle in the ADR-0006 order.
    ///
    /// ★The critical section is not cheap★: ① the `roster()` observation pays **one `dunce::canonicalize` syscall per asleep agent
    /// without an override** and ② the commit writes the whole `agents.json` to disk (ADR-0071).
    /// So creation, rename and new-registration spawn are globally serialized, and if there is even one agent whose cwd is on a dead network share,
    /// the three paths are blocked together while that syscall stalls.
    /// ★Still, mail delivery is not blocked — do not break this property★: `DeliveryPort` (injection · roster · names)
    /// never takes this lock (`roster()` can be called without a lock, and the delivery path uses it that way).
    // ADR-0006
    name_allocation: Arc<Mutex<()>>,

    /// The turn observation table (the ADR-0113 fact layer) — shared by every `OutputCore` this manager launched.
    /// Write = `OutputCore::emit` (★there are two calling threads★ — the output pump and the injection thread that emitted the input echo,
    /// turn.rs header), cleanup = `OutputCore::finish` (★not the reaper★ — because the race with late emits after exit must be closed
    /// at the same point as the finalize flag. ADR-0127 decision 5 · rejected alternative (d)),
    /// read = consumers (the mail idle gate etc.).
    /// ★A leaf unrelated to the sessions lock★: there is no path taking sessions/profiles while holding this table
    /// (no cycle is made in the ADR-0006 order).
    // ADR-0113
    // ADR-0127
    turns: Arc<TurnObservations>,

    /// The waiting-list table (「this incarnation's user waiting list is not empty」) — a leaf of the same shape as the turn observation table.
    /// Read = the mail busy adapter. ★Unrelated to the sessions lock★ (the same discipline as `turns` above).
    // ADR-0231
    inputs_pending: Arc<InputsPendingTable>,
}

/// The RAII guard that takes the AgentId reservation during spawn and releases it automatically on drop (ADR-0086 FIX 6). Keeps the reservation from leaking
/// on any early return of spawn_agent (provision failure · PTY failure · `?`). `reserve` returns None for an already-reserved
/// id (refusing the second concurrent spawn).
///
/// ★★Two other safeties sit on this guard's **lifetime** — shortening it opens both **together**★★. On a link declaring a connection,
/// [`LinkWatch`] holds this reservation **until the connection outcome happens**, and meanwhile no new incarnation can stand with the same id.
/// Two things rely on that fact:
///   1. **the mailbox's incarnation marker comparison does not fire** — so the `as_verdict` mismatch branch of
///      [`AgentManager::link_activation_verdict`] is dead code in production (half of what that doc calls
///      「channel freshness is the real reason」 is this lifetime).
///   2. **it is safe for the supervisor to pick the session by id alone** — `link_activation_verdict` reads the state and exit intent of the session
///      taken by `get_session(id)` without an incarnation marker comparison. It is a property holding because no successor can stand while waiting,
///      not a property that function guards by itself.
/// ★So a change narrowing this lifetime **must re-examine both**★ — shortening it while looking at only one would make the shape rely on a guard
/// that never fired (`as_verdict` was written wrongly that way — do not repeat the same mistake
/// here). The dependency is not visible from the consumer-side comments alone, so it is written **here, at the definition**.
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

/// If it fails after provision succeeds and **before** session registration (exe/PTY error · `?` early return), the issued token + config
/// file leak forever (there is no session, so the reaper never revokes) — the RAII guard preventing this (ADR-0086 FIX 3).
/// It is armed only when provision returned a real endpoint, and is disarmed with `disarm()` when session registration finishes.
/// On drop, if still armed, it calls revoke (discard + delete file) — covering every pre-registration failure path.
///
/// ★No lock held (ADR-0006)★: drop happens only at points not holding the sessions/status locks (spawn_agent early returns),
///   so revoke (registry leaf lock + file IO) does not break the lock order.
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

    /// The control-channel-injected form (ADR-0086) — the daemon plugs in `DaemonControlChannel`. The reaper also shares the same Arc
    /// and revokes at the terminal convergence point (spawn=provision / terminal=revoke causal symmetry).
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

    // ★No mail-eligibility query verb is put here (do not revert)★: consumers read `AgentInfo::reads_messages` of the list
    //   snapshot. Making a verb that asks back by id would revive TOCTOU, non-atomicity and per-session locks
    //   right there (that field's doc). The decision source is the value the session received from the backend at spawn
    //   (`AgentSession::reads_messages`), **not the profile** — `DeleteProfile` does not kill a live session, so the
    //   profile axis becomes "unknown" in production, and that fail-open was the hole that actually put a shell back on the list.

    // ── Roster — the single entry ────────────────────────────────────────────

    /// **The only place** building "all agents + each one alive/asleep" (ADR-0119 decision 2).
    ///
    /// ★One snapshot★: the roster and "the set of live session ids" (the basis of the asleep difference) come from **the same single
    ///   `list_agents()` sheet**. Taking it twice would let a spawn, exit or deletion slip in between, so two recipients of the same send would see different worlds
    ///   (the kind forbidden by ADR-0111 decision 2).
    /// ★Asleep = the difference on **the id axis**★: a profile's session launches with that profile id (`activate_profile`), so "a profile
    ///   without a live session" is split exactly by id. Subtracting on the name axis would let **one live same-name entry hide another asleep profile
    ///   entirely** (that regression is sealed by the `messaging_host` tests).
    /// ★Asleep names are not folded★: two asleep entries with the same name come up as two (it is the same-name decision axis, so dedup
    ///   would silently change the decision).
    /// ★The live/asleep name sources **differ** (intended — do not merge)★: live entries use the `AgentInfo.name` `list_agents()` built
    ///   (= `resolve_canonical_name`, based on session.cwd), asleep entries use
    ///   `AgentProfile::canonical_name_when_live()` (profile.cwd + the same normalization). Merging the two derivations would
    ///   change the parking key behavior — the live session looks at `session.cwd` (canonicalized at spawn), the asleep profile at raw
    ///   `profile.cwd` normalized, different materials.
    /// ★fs access only for asleep profiles without an override★: **do not defeat here** the shortcut of `canonical_name_when_live()` (0 syscalls if display_name
    ///   is non-blank) — this lookup is on the send critical path, and if the cwd is a dead
    ///   network share, one canonicalize blocks for tens of seconds. So that function is used as is, not reimplemented.
    /// ★Lock discipline (ADR-0006 · ADR-0071)★: `sessions` (RwLock) and the profile map (Mutex) are **independent domains**.
    ///   `list_agents()` takes the sessions lock, clones the Arcs and releases right away, and only then does `profiles.list()`
    ///   take the profile lock — **sequential, not nested**. Merging the two maps under one lock would make profile saves (disk write while holding the lock)
    ///   block session lookups (= the envelope injection path) (ADR-0119 rejected alternative).
    /// ★Not atomic, and not made atomic★: the race residue between the two lookups was already judged by ADR-0116, which accepted it as
    ///   bounded by a 24h TTL. "Now that it is one entry, make it atomic" is the direction 0116 rejected.
    // ADR-0119
    pub fn roster(&self) -> Vec<RosterEntry> {
        // ★The profile snapshot is **one sheet**, and the names and hierarchy of live entries also come from that one sheet (load-bearing)★.
        //   Previously the names of live entries were taken from profiles `list_agents()` fetched separately per session, and the hierarchy from the following
        //   list query — if a rename + hierarchy move were committed in between, **a combination that never existed**
        //   (old name + new parent) would be carried in one row. Drawing from one sheet makes that combination impossible to build.
        //   ★The non-atomicity between sessions↔profiles remains, and it is an already-accepted property★ (the doc above · ADR-0116).
        //   What was closed here is profile↔profile inconsistency.
        // ★Side effect = it got cheaper★: the old path did `profiles.get` (= a whole AgentProfile clone +
        //   a mutex acquisition) per session. Now it ends with one list, so profile clones as many as live sessions disappear —
        //   this lookup is on the critical path of mail sending (ADR-0119 · messaging_host `addressing_sources`).
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
            // A corpse (terminal) stays in the map until reaped — split by state, not existence (the ADR-0116 predicate).
            //   ★The profile is left in the map★: the corpse's profile must come up as an **asleep** entry in the loop below.
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
    /// Return = **the profile this call registered** (the value with the assigned name reflected). Needed because the caller's response must carry that name
    /// — returning the pre-registration snapshot when a suffix was attached would give the screen and the roster different names.
    /// ★It is not the stored value read back★: `ProfileRegistry::mutate` runs `normalize_hierarchy` right before saving,
    ///   so in principle `parent_id` could diverge. This path is always a new profile with `parent_id == None`,
    ///   so it does not actually diverge, but if a read-back becomes necessary, query again explicitly.
    ///
    /// ★The suffix is pinned as `display_name`★: without an override the canonical name is derived from the cwd basename,
    ///   so two pointing at the same folder become same-named automatically even without a rename (ADR-0120 §Impact). The only storage place
    ///   that can resolve that collision is the override.
    /// ★A change with a caller★ — Err = roster cap · suffix space exhausted · save refusal · failure ([`PtyError::Store`]), and
    ///   either way no registration happens (the in-memory roster unchanged).
    // ADR-0291 R17 (E4)
    pub fn try_create_agent(&self, mut profile: AgentProfile) -> Result<AgentProfile, PtyError> {
        profile.display_name = normalize_display_name(profile.display_name.take());
        // ★The derivation is also inside the gate★: if the read deciding the requested name were outside the gate, a window would open not between observation and commit but
        //   **between derivation and observation** (if someone commits the same name in between, both decide it is free).
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

    /// Agent deletion (tree "delete") — a change with a caller: if the save is `Err`, it does not delete and is [`PtyError::Store`]. A missing
    /// id also attempts a save, so in a refusal state it is `Err`.
    // ADR-0291 R17 (E4)
    pub fn try_delete_agent(&self, id: AgentId) -> Result<(), PtyError> {
        self.profiles.try_remove(id).map_err(PtyError::Store)
    }

    /// Display name override set/clear (tree "rename").
    ///
    /// ★Rename is also a uniqueness check point (ADR-0120 decision 2)★: 0115 looked only at new registration, but tree rename really exists and
    ///   uniqueness leaks through it. `None` (clearing the override) is also checked — clearing makes the canonical name the cwd
    ///   basename, **changing it**, so that too is a rename.
    ///
    /// ★★The decision table (`decide_name`) does all the deciding — no shortcut here in advance★★. Previously "if it already holds a name
    ///   of the requested family, no-op" was filtered **before** the gate, but then it became a no-op **even when the requested name was
    ///   free**: an agent holding `bob(1)` was not renamed even though `bob` was free, and only reported success
    ///   (the caller even sends Ack + list broadcast, so the user and LLM see something that did not happen as done). Clearing the override
    ///   was permanently impossible for the same reason (`shared(1)` of `C:/shared` always hit because the clear result was its own family).
    ///   The decision "give it if free" can be known only by looking at the roster, so it must be inside the gate.
    ///
    /// ★A change with a caller★ — if the save is `Err`, the name is left as is and it is [`PtyError::Store`]. Outcomes that do not attempt a save
    ///   (`Unchanged` · `Exhausted` · `NotFound` at the lookup stage) are `Ok` regardless of the store state.
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
        // ★Its own current name must be **the value the roster says**★: a live agent's roster name is based on session.cwd
        //   and the profile derivation on profile.cwd, so they can diverge (`roster()` doc). Deriving separately and comparing
        //   could mistake a name someone else holds for its own and commit it. For a profile-only (asleep) case the two are the same value.
        let roster = self.roster();
        let current = roster
            .iter()
            .find(|e| e.id == id)
            .map(|e| e.canonical_name.clone())
            .unwrap_or_else(|| profile.canonical_name_when_live());
        // ★★The rule is not duplicated; **the existing derivation matching liveness is chosen**★★. The two axes differ in material and function:
        //   - live axis (`resolve_canonical_name`) = `canonical_name_or_id_fallback(override, **raw** session.cwd)`
        //     — does not canonicalize.
        //   - asleep axis (`canonical_name_when_live`) = canonicalize first, then basename.
        //   So running a live agent's prediction through the asleep function relies on the **assumption**
        //   `basename(canonicalize(cwd)) == basename(cwd)`. That assumption is not universal: if spawn's canonicalize fails, the raw
        //   `profile.cwd` becomes session.cwd as is (the cwd handling of spawn_agent), and the harness seam can plug in an arbitrary cwd.
        //   The moment they diverge, it **misjudges a name someone else holds as Free**, commits it and reports the wrong name —
        //   the point where ADR-0120 global uniqueness breaks. So live entries are predicted with the live-axis function.
        // ★The opposite direction (adding canonicalize to the live axis) is forbidden★: that path is the mail delivery critical path, so a syscall cannot be put
        //   in front (ADR-0119). This fix changes only the prediction side and **removes one** syscall from the clear path.
        // ★For a non-blank override request the two functions are literally the same★ (both return the override as is without looking at the cwd)
        //   — that path's behavior is unchanged. A whitespace-only override is also treated as "no override" on both sides.
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
            // ★The commit result is not swallowed★: `get` · `roster()` · `rename` each take the profile lock **separately**, so if
            //   another connection's `DeleteProfile` slips in between, the commit cannot find the target and returns false. Reporting that as success
            //   would make the wire send Ack + list broadcast for a nonexistent agent (the gate serializes only name allocations
            //   with each other — deletion does not take this gate).
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

    /// Tree hierarchy move (set/clear parent) — a change with a caller: if the save is `Err`, the hierarchy is left as is and it is
    /// [`PtyError::Store`]. A rule violation (`Ok(false)`) also attempts a save, so in a refusal state it is `Err`.
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

    /// Toggles the boot auto-restore target — a change with a caller: if the save is `Err`, it is left as is and [`PtyError::Store`]. A missing
    /// id (`Ok(false)`) also attempts a save, so in a refusal state it is `Err`.
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

    /// ★Harness-only roster injection seam (ADR-0012 — the same gate shape as `insert_test_session`)★ — plants a profile as is,
    ///   **bypassing** name uniqueness (ADR-0120). Only for sealing tests that must reproduce states uniqueness makes impossible on the normal path
    ///   (e.g. two asleep entries with the same name). It is not compiled into production builds
    ///   (feature OFF = method absent) — if a production path called this, roster uniqueness would silently break.
    #[cfg(feature = "test-harness")]
    #[doc(hidden)]
    pub fn seed_agent_bypassing_uniqueness(&self, profile: AgentProfile) {
        self.profiles.upsert(profile);
    }

    /// Acquires the allocation gate. ★Ignores poison★: this Mutex wraps `()` and **protects no state** —
    /// there is no inconsistent data another thread's panic could leave in the first place. With `expect`, one unrelated panic would
    /// permanently block creation, rename and spawn until a daemon restart (pure downside). The places holding this gate are outside the containment marker,
    /// so in the production process the hook ends that panic first — what this tolerance receives is only test builds without the hook (ADR-0292).
    fn lock_name_allocation(&self) -> std::sync::MutexGuard<'_, ()> {
        sync::lock(&self.name_allocation)
    }

    /// ★The name decision table (ADR-0120 uniqueness · ADR-0123 numbering rule) — called only while holding the gate★.
    ///
    /// `self_id` is **excluded** from observation (so it does not collide with its own name). `current` = the canonical name the roster currently says
    /// for this agent (`None` for a new registration).
    ///
    /// The decision order is itself the contract:
    ///   3) if **nobody** has `desired` → `Free`. Whether it is in that family or not does not matter
    ///      (looking at the family first here would make a silent success of "the requested name is free but it is not given").
    ///   4a) if someone else has it and `current` is in the `desired` family (`desired` or `desired(n)`) → `KeepCurrent`.
    ///       Prevents a re-request from burning a number or dragging it down to a free lower number (name = mail address, ADR-0116).
    ///   4b) otherwise → `Suffixed` with a free number of the family.
    ///
    /// ★It **receives** the roster snapshot (does not take it itself)★: the caller also looks at the total cap with that roster in the same gate section
    ///   (and a rename reads its own current name there too) — taking it twice would make each decision see a different world, and the lookup cost
    ///   would double.
    // ADR-0120
    fn decide_name_with_roster(
        &self,
        roster: &[RosterEntry],
        self_id: AgentId,
        desired: &str,
        current: Option<&str>,
    ) -> NameDecision {
        // ★The two axes are separated★: `taken_exact` (whether someone else uses that string — decides whether to attach a suffix) and the family number
        //   set (decides which number to attach). Mixing them goes wrong in two directions: `bob` looks taken even when only `bob(1)` exists,
        //   or a literal `bob(0)` looks as if it occupied the unsuffixed `bob`.
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

    /// The point where spawn registers the profile in the roster (called only by `spawn_agent_watching_link`). The registry entry
    /// is chosen by `entry`.
    ///
    /// ★★Uniqueness is applied **only to new registrations** — epoch replacement is not a rename target (ADR-0115 §Impact · ADR-0007)★★.
    ///   restart / restore (`restore_all`·`restore_one`) / reactivation (`activate_profile`) /
    ///   `resume_no_fallback` are all **map replacements of the same AgentId**, not new agents. The two are told apart by whether the id
    ///   is already in the roster. An ad-hoc spawn (a profile the connection made on the spot) has an id not in the roster, so it is caught as a new
    ///   registration.
    /// ★`upsert_preserving_hierarchy` keeps the live entry's `display_name`★ — so planting the override only in the new-registration
    ///   branch is safe (in the existing-id branch the live value wins anyway). For a name to actually
    ///   leak, removing this branch and losing that preservation rule would have to happen **together** (that compound regression is caught by
    ///   `epoch_replacement_never_renames_an_existing_agent`).
    /// ★One more reason for the branch = restore path cost★: boot restore calls this per agent, and if the check ran it would pay
    ///   a full roster scan + canonicalize syscalls as many as asleep agents without an override each time, inside the allocation gate
    ///   (the gate field comment).
    /// ★Err = roster cap · suffix space exhausted · deleted in the meantime · (for `Entry::Caller`) save refusal · failure★ — the spawn
    ///   aborts with that Err (it does not launch with a duplicate name · on a save `Err` the roster is unchanged).
    // ADR-0115
    // ADR-0291 R17 (E3)
    fn register_for_spawn(&self, profile: &AgentProfile, entry: Entry) -> Result<(), PtyError> {
        // For an existing id there is no name allocation at all, so the gate is not taken.
        //
        // ★★「is it there」 and 「write」 are done in **one critical section** — do not revert to two calls★★:
        //   if the lookup and the write split, a `DeleteProfile` slipping in between is ignored, and **after the deletion is reported as successful,
        //   that profile comes back to life and even its child process launches with it.** So the update
        //   has [`ProfileRegistry::update_preserving_hierarchy`] decide existence too inside the lock,
        //   and here it only branches on that result.
        // ★`false` = deleted in the meantime → **cut the spawn**★: dropping it to the new-registration branch below would be exactly that
        //   resurrection, and just proceeding would put a session without a profile onto the roster.
        //   ★**The same disposition, a different point in time** as the three `profile_vanished_mid_spawn` checks standing later★ — those catch a deletion
        //   **after** settling the marker, and this one catches it before. Neither replaces the other.
        // ★**This line does not close every resurrection path**★ — if the deletion finished **before** registration started, this
        //   lookup does not exist in the first place, so it goes to the new-registration (ad-hoc spawn) branch below. Telling those two apart requires the caller to
        //   bring 「this is an entry that existed」, and that signal is not in the `spawn_agent` signature (unresolved).
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
        // ★The cap is checked here too★: this branch is **the new-registration point of ad-hoc spawn**, so leaving it empty would let the roster
        //   grow without bound without passing through `try_create_agent` (making the statement that the cap is a "total" false).
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

    /// ★The 「nothing to do」 of the two branches is `Ok(Moot)` — do not revert to `Err`★: if that entry is already up
    /// (the double-spawn guard below) or already launching (reservation lost), this request is moot, and moot is not a failure.
    /// The grounds and what that switch removed are kept by the `SpawnOutcome` comment.
    // ADR-0172
    /// ★A thin delegation that drops the delivery channel★ — callers waiting for the connection outcome use
    /// [`AgentManager::spawn_agent_watching_link`] below. If the channel closes in this branch, the link's delivery
    /// silently fails (that port was designed that way) and nobody waits.
    /// ★So **the connection failure of an incarnation launched with this verb has no owner**★ — the activation entries
    /// ([`AgentManager::activate_profile`]·[`AgentManager::restore_one`]) go, for both Fresh and Resume, into the branch holding a
    /// [`LinkWatch`], so they do not reach that hole.
    /// ★The remaining consumer is only the daemon's **instant by-cwd creation** — ★at one point this said 「WS `Spawn`」, but
    ///   that is false★: that command goes into `activate_profile` (the comment there pins it with 「there must be one activation entry」).
    ///   The only production place actually calling this verb directly is that branch, which creates on the spot
    ///   without registering a profile, and **the codex reaching there today has terminal output format, so it has no connection axis**
    ///   (the canonical grounds = the same-hole comment of `backend/codex/transport.rs`).
    /// ★The entry with a caller★ — if the save of the roster registration or the session id clearing is `Err`, it does not launch and is
    ///   [`PtyError::Store`] (partial success = [`AgentManager::spawn_agent_watching_link`]).
    pub fn spawn_agent(
        &self,
        profile: &AgentProfile,
        mode: SpawnMode,
    ) -> Result<SpawnOutcome, PtyError> {
        self.spawn_agent_watching_link(profile, mode, Entry::Caller)
            .map(|(o, _)| o)
    }

    /// The shared body of launching — the three entries (`spawn_agent` · `activate_profile` · `restore_one`) reach here through
    /// two verbs.
    ///
    /// `entry` = whoever called this launch. Of the three roster changes before opening the process, ① registration and ③ Fresh's session id
    ///   clearing change the registry through that entry. ② the incarnation marker is memory-only, so it is unrelated to the entry, and the post-launch
    ///   `auto_restore` raise (`spawn_session`) is always internal regardless of the entry.
    /// ★Partial success is not reverted (`Entry::Caller`)★ — each change is an independent change with its own save. If ③'s save is `Err`
    ///   after ① was saved, ① remains (an agent on the roster · updated but not launched — the same end shape as after a process open
    ///   failure), ②'s new marker also remains in memory (harmless since no incarnation holds it), and the field keeps the old
    ///   handle. The launch stopped, so there is no incarnation to diverge. If ① fails, ② · ③ are not reached.
    // ADR-0291 R17 (E3 · D8)
    fn spawn_agent_watching_link(
        &self,
        profile: &AgentProfile,
        mode: SpawnMode,
        entry: Entry,
    ) -> Result<(SpawnOutcome, Option<LinkWatch>), PtyError> {
        // ★★The reservation is taken **before the roster lookup** — do not revert the order★★: on a link declaring a connection,
        //   the reservation lives not until the spawn ends but **until the connection outcome happens** ([`LinkWatch`]).
        //   The session in that interval is on the roster, but it is an incarnation **not yet known to be able to take input**. Putting the lookup
        //   first would make a second request seeing that incarnation answer `Moot(Some(..))` — that is, 「it is up」 — and that
        //   answer itself is the defect: when the first request soon decides a failure and collects it, the second request's caller **ends up holding
        //   as alive an agent about to die under it.**
        //   Looking at the reservation first makes the answer in that interval `Moot(None)` (「already launching」), so the two callers do not split.
        //   ★On links without a connection axis this order changes nothing★ — there the reservation ends together with this
        //   function, so the interval where 「up」 and 「launching」 overlap is only a few commands wide in the first place.
        let Some(reservation) = SpawnReservation::reserve(self.spawning.clone(), profile.id) else {
            // The winner has not yet put it on the roster, or even if it has, that connection has not produced its outcome yet — either way
            // we have no ground to answer 「it is up」.
            tracing::info!(
                agent = %profile.id,
                "spawn_agent: already launching — this request has nothing to do (moot)"
            );
            return Ok((SpawnOutcome::Moot(None), None));
        };

        // ★Residual race (ADR-0082 unresolved · follow-up)★: this double-spawn guard takes a read lock here to look at contains_key and
        //   releases it, and the actual registration (sessions.insert) is done below with a separate write lock — there is a window in between.
        //   If the same id is SpawnProfile'd concurrently, both may pass this check and the pre-check of activate_profile and a
        //   double-spawn can happen. This window is a **pre-existing
        //   race** from before ADR-0082, and that change neither introduced nor closed it (flagged as a follow-up).
        //   ★「they overlap only across connections, since each connection is serial」, once written here, is now **false**★:
        //   the daemon takes activation commands off that connection's arrival-order lane and runs them as separate tasks (the daemon crate's
        //   `connection_core::dispatch_order`), so **two requests of the same connection** can also overlap here. The width of the window is
        //   unchanged — what blocks it is the reservation above and the pre-check, not caller-side serialization — and what changed is
        //   only the range of callers that can reach that window.
        //   ★The reservation above **narrows** that width **but does not close it**★ — there is the same window between taking the reservation and this lookup.
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
        // ★Why this place (not the caller)★: this is where the map replacement actually happens, and **every spawn
        //   passes this one line regardless of mode**. In the old days issuing was only in the Resume branch of `activate_profile`,
        //   so the Fresh respawn paths (the WS `Spawn` command · the Fresh branch of `activate_profile` · boot restore of a profile without a sid)
        //   **reused the dead incarnation's marker as is**. That reuse collapses every structure keyed by (AgentId, epoch)
        //   — the turn observation table (ADR-0113: a dead incarnation's late emit overwrites the live incarnation's
        //   entry and that emit's finalize recheck erases it) · the control channel token (ADR-0086) ·
        //   the reap epoch-guard (ADR-0084). Scattering it per call site makes a new call site miss it again.
        // ★The mode is not looked at (Resume follows the same rule)★: if a new process was launched, it is a new incarnation, and the mode does not change that
        //   fact. Splitting by mode would leave a Resume-side respawn (calling `restore_all` outside boot etc.) as the same reuse
        //   hole — that is only a convention, not enforced.
        // ★If the profile vanished, **the spawn is aborted**★: there is no reason to launch a session with a deleted profile. Cut with `?`.
        //
        // ★**It must come right after `register_for_spawn` — do not put a single line in between**★:
        //   [`ProfileRegistry::upsert_preserving_hierarchy`] **keeps** the live marker (ADR-0084 —
        //   because the snapshot must not author the marker), so until this line runs, the marker standing in the roster is still
        //   **the previous incarnation's**. A late record of the previous incarnation arriving in that interval
        //   (the `Some(epoch)` branch of [`ProfileRegistry::observe_session_id`]) has a matching marker, so it **is not refused
        //   but passes** — what that guard blocks is only 「a newer incarnation already stands」, and the new incarnation has not
        //   stood yet. So this line is **the verb that closes that interval**.
        //   ★What that interval actually breaks (not 「just a stale value」)★: if this line moved after the Fresh clearing
        //   below, the previous incarnation's late record would pass the marker guard and sit in the cleared field. Then this incarnation's
        //   first-submission commit would be refused because the field differs from the value seen at the start (`None`), this incarnation's id would not be persisted,
        //   and the next activation would open the dead incarnation's conversation (ADR-0226).
        //   ★**In exchange, the places that check 「the profile vanished」 grew — the two below also cut with `?`**★:
        //   as this line moved to the very front, a window opened where the profile is deleted **between** passing here and the clearing / roster read.
        //   Without blocking that window, the clearing and the read would silently return `None` and the spawn would go on,
        //   the resume would silently become a new conversation and a session without a profile would go onto the roster. In the old layout this line was
        //   at the very end, so that interleaving was caught here — now each of the three places catches it.
        // ADR-0007
        let epoch = self.profiles.epoch_for_spawn(profile.id).ok_or_else(|| {
            profile_vanished_mid_spawn(profile.id, "settling the incarnation marker")
        })?;

        // cwd normalization — fixes the notation of the claude session directory (avoids UNC). On failure the original is used (best-effort).
        let cwd = dunce::canonicalize(&profile.cwd).unwrap_or_else(|_| profile.cwd.clone());

        // ★Session id rule (ADR-0226 — 「a stored id exists ⟺ there is a conversation to resume」)★: here **nothing
        //   new is persisted**. This incarnation's id is written by the first-submission latch right before the first user turn goes out
        //   (`SessionIdLatch` below). Do not bring back the path issuing and persisting at spawn — every session closed without input
        //   would leave an id without a conversation, and the next activation would fail to resume with that id.
        //   - Fresh: pushes the old handle into the history and starts from an empty field (the canonical decision and reason =
        //     [`fresh_spawn_release_session_id`]). An issuing backend **must draw a new sid** — reusing the stored
        //     sid would launch with `--session-id <stored sid>` and collide with the disk session ("Session ID already in
        //     use" → claude dies immediately, ADR-0076).
        //   - Resume: **only reads** the stored sid — the handle read below is the only read of this spawn.
        //   The canonical source of to whom we hand our sid (the issuing axis) = [`session_id_to_hand_over`].
        // ★`None` is not 「nothing to push」 but 「the profile vanished」 — do not swallow it, cut★: the case deleted
        //   after passing the marker settlement above comes here.
        // ADR-0226
        if fresh_spawn_release_session_id(&profile.command, mode) {
            entry
                .release_session_id(&self.profiles, profile.id)
                .map_err(PtyError::Store)?
                .ok_or_else(|| profile_vanished_mid_spawn(profile.id, "clearing the session id"))?;
        }

        // ★**It reads the roster, not the snapshot the caller gave**★ — `profile` is a copy the caller took, and the value of this field
        //   is **written not by us but by the other side** (the record port writes the thread id codex gave in the handshake into the
        //   roster). So a handle that arrived after the snapshot time is not in the copy — reading it would resume an
        //   old thread.
        //   ★This read actually has a value only since [`ProfileRegistry::upsert_preserving_hierarchy`] came to
        //   keep `backend_session_id` (user decision)★ — before that, `register_for_spawn` above
        //   overwrote this field with the snapshot, so even reading the roster read the snapshot.
        // ★The read being **after** `epoch_for_spawn` above is the grounds of this read★ — from when the marker newly stands until
        //   this incarnation's link receives the latch's `offer_sink` below, every record **carrying a marker** is
        //   refused (the previous incarnation mismatches, and this incarnation's link does not exist yet).
        //   ★**That does not mean 「nobody can write」 — writing that would be false**★:
        //   the `incarnation: None` branch of [`ProfileRegistry::observe_session_id`] **writes unconditionally**
        //   without comparison. A production caller entering that branch exists — the callback the daemon assembly point set on [`SessionTracker`]
        //   (`engram-dashboard-daemon/src/lib.rs`) is it.
        //   So the grounds that 「the read value is frozen」 holds is not the marker alone but **the fact that that observer does not attach
        //   to codex profiles**: `tracker.watch` is called below only behind the `assigns_sid` gate, and codex has that axis
        //   off (ADR-0185). The day that gate is switched to the resume axis, this sentence breaks
        //   first, so look here together then.
        // ★`get` giving `None` = the profile was deleted in the meantime → **cut**★: swallowing it with `and_then` would make the resume
        //   **silently a new conversation**, and the following `spawn_session` would put a session without a profile onto the roster. Both of those
        //   directly violate the 「do not silently make a new conversation」 discipline, so it is cut with `?`.
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
        // ★This read is authoritative — if empty, refuse before launching the process★: a backend resuming with the stored handle
        //   **silently drops** the resume flag from argv when there is no handle. Launching as is would open a new conversation with an id we
        //   do not know, and the decision would report it as 「resumed」. The common case (not in the roster from the start)
        //   is turned into a new conversation by the head of `resume_no_fallback` — this is the back door of the race where the field empties **between** that decision and this read,
        //   and it uses the same predicate. The next activation has no handle, so it goes to a new conversation.
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

        // ADR-0086 ★Called right before spec assembly★ — because build_command_spec must receive the endpoint and inject it into the command line
        //   the backend's way (claude=`--mcp-config`). The incarnation marker is the current value settled above, so
        //   a new token is issued every time the incarnation changes (token lifetime = (AgentId,epoch)).
        //
        // ★backend-conditional (round-2 F3)★: provision is called only for backends that **consume** the control channel (claude)
        //   — shell has supports_control_channel=false, so provision is not touched at all (registry
        //   untouched). This way there is no regression where a config-write failure aborts a shell spawn that never needed MCP.
        //   The decision is backend dispatch (ADR-0004) — the manager does not matches! on command directly.
        // ★fail-closed (FIX 2)★: for a backend that **calls** provision, if the provision three-value (Ok(Some)/Ok(None)/
        //   Err) is Err (CSPRNG/file write failure), **the spawn is aborted** (no agent secretly running without a control channel
        //   — prevents health disguise). Ok(None) = legitimate absence of not using a control channel (Noop), so it proceeds.
        //   Ok(Some) = issued → armed with ProvisionGuard below, so the issued token/config is reclaimed on any failure before session registration
        //   (FIX 3 leak prevention). A backend with supports_control_channel=false skips provision,
        //   so it flows the same as None (absence) — the fail-closed contract does not apply to that backend.
        let control_endpoint = if backend::supports_control_channel(&profile.command) {
            // ADR-0099: hands the backend's MCP capability to provision so the physical channel wiring, whether priming is loaded
            //   (not a variant — the non-MCP branch receives no instructions at all, ADR-0209) and grants are
            //   split all at once (consistency invariant = taught channels ⊆ laid channels — revised to one-way by ADR-0126 decision 4).
            //   The decision is backend dispatch (ADR-0004) — the manager does not matches! on command directly.
            // ADR-0126
            // ★The two axes are asked **each separately** — one is not derived from the other★: that folding is the place that silently opened send authorization
            //   for a backend 「using control but outside the mail plane」 ([`ControlChannelNeeds`] doc).
            // ADR-0133
            // ★The third axis is also asked **separately**★ — deriving 「does it read the file we wrote」 from 「does it use mail through MCP」
            //   would make a backend turning MCP on by another mechanism get a plaintext token file it does not read on every spawn
            //   (ADR-0209 · `backend::writes_mcp_config_file` doc).
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

        // ★The second fail-closed door — cuts here failures the daemon cannot see★: what the `?` of provision above catches is
        //   only daemon-side failures (CSPRNG · mcp-config write), so 「could not build the attachment value」
        //   of **a backend that writes no file** passes that door as is (codex's `-c mcp_servers.…`). Launched in that state, that agent
        //   goes onto the delivery list but has 0 send entries, so every arriving request becomes a contract nobody can answer.
        // ★Placed **after** the guard is armed★ — if cut here, that guard reclaims the token and file already issued.
        // ★The decision is backend dispatch (ADR-0004)★ — the manager knows neither the `-c` syntax nor MCP attachment.
        // ADR-0209
        backend::precheck_control_endpoint(&profile.command, control_endpoint.as_ref()).map_err(
            |e| {
                PtyError::SpawnFailed(format!(
                    "backend cannot use the provisioned control channel (fail-closed): {e}"
                ))
            },
        )?;

        // ★`sid` is not passed as is — this field is **the stored backend sid** (the two axes of ADR-0185)★:
        //   `sid` is filled only for backends with the issuing axis on, so on the recording side (codex) it is always `None`
        //   even if a value to resume actually exists in the profile.
        // ★One value is passed to **two places** — only the backend that uses it differs★: `build_command_spec` below is
        //   read by backends that resume by command line (`resume <id>` of the codex terminal), and `open_spawn` further below
        //   by backends whose link requests the resume (`thread/resume` of the codex app-server). Computing and passing different values to the two places
        //   would split which one is 「what this spawn resumes」.
        // ★The assembly is not done here (ADR-0004)★ — this place only reads the handle and passes it on, and whether to use it as a subcommand,
        //   a flag or not at all is decided by the backend.
        // ★Passed empty on Fresh★ — sending the value and having the backend look at the mode again would put the decision in two
        //   places. A request to open a new conversation with that dead incarnation's thread id is Fresh, and without emptying it here that
        //   request would be interpreted differently per backend (that door = `resume_handle_for` above).
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

        // ADR-0191: the backend hands out the real link and the values to be carried on the session (backend caps · encoder · turn classifier · mail eligibility)
        //   **in one dispatch** — the same switch is not ridden again per aspect. This place does not know the actual type of
        //   the returned link.
        //   The reason for carrying mail eligibility on the session is unchanged — even if the profile is deleted, the live session keeps knowing that fact
        //   (`AgentManager::reads_messages` doc).
        // ★This call launches the child process — it must come after the transcript read above★: swapping them would make
        //   that conversation file read while the program is already running.
        // ADR-0185: hands here the place where a backend that **fetches** the session id (codex) announces that value — the receiver is
        //   this incarnation's first-submission latch (ADR-0226). The latch writes it into the roster only right before the first user turn goes out.
        // ★Handing it over unconditionally is intended — no new declaration axis like `receives_session_id()` was made★:
        //   the two siblings of this place (`supports_control_channel` · `accepts_mcp_config`) are split by declaration
        //   **because giving them has an effect** (issuing tokens · config files). This field is the opposite — it has no effect on backends that do not read it,
        //   so setting up an axis would write the same decision in two places (declaration table + impl), and the two could diverge.
        //   The decision point is kept in the impl alone.
        //   The latch's commit port binds the `epoch` settled above, so that a record arriving after this spawn died does not overwrite the next
        //   incarnation.
        // ★The delivery port is laid down only for backends declaring a connection★ — laying it where there is none would make the supervisor wait on
        //   a channel nobody calls, and the decision of that backend must stay on the old path (claude · shell · stdio ·
        //   codex terminal become `None` here and go the same way byte for byte).
        // ★The marker is stamped **here**★ — the `epoch` settled above is bound to the port, so this incarnation's outcome is never applied to the next
        //   incarnation (the doc of `link_verdict_sink` is the canonical source of that reason).
        let (link_sink, link_rx) = if backend::declares_link(&profile.command) {
            let (tx, rx) = std::sync::mpsc::channel();
            (Some(link_verdict_sink(tx, epoch)), Some(rx))
        } else {
            (None, None)
        };
        // ★An issuing backend puts its id into the latch **here, before `open_spawn`**★: the session takes input from the moment of the roster registration
        //   below, and the roster announcement that follows releases parked mail right away. Putting it in later
        //   would let the first submission go out without knowing the id and the commit would come after the turn. The decision is not the backend name but
        //   one thing, whether it holds `sid` (the issuing axis), so it is backend-neutral — Fresh is the value just drawn, Resume the stored value read.
        // ★The value commit compares against = the value of the field this incarnation saw when it started (`stored_handle` — Fresh is
        //   `None`, after clearing above)★: if another writer (the claude file watcher) changed the field in between, the commit does not rewind but refuses.
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
            // ★A backend whose link owns the turn counts submissions here★ — the session of that mode does not count. Whether to plug it in is
            //   decided by the backend (the rest drop it).
            // ADR-0226 · ADR-0231: user decision — the id is persisted when it becomes real (the other side's echo of the first user message).
            Some(latch.first_turn_sink()),
            resume_session_id,
            link_sink,
            // ★**The same endpoint** as passed to `build_command_spec` above★ — what was to be translated into the command line finished
            //   there, and this place hands the same value once more for backends that must carry it **in the link
            //   handshake** (the codex app-server puts the priming in `thread/start`). What goes where is
            //   decided by the backend — this place does not know which of the two means it is (ADR-0004).
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

        // sid drift observation attached (best-effort). Making the observer and answering "nothing to make" are both the backend's
        //   job (ADR-0004), so here it is unknown what that program reads.
        // ★Why the gate is **the issuing axis** (ADR-0185)★: the observer receives the sid we handed over as the **reference value** (`expected_sid`)
        //   and observes what differs from it — a session we did not issue has no such reference value at all.
        //   Switching to the resume axis would attach an observer to a session without a reference value.
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
        // ★The mailbox and the reservation are handed over **as one set**★ — keeping a caller waiting for the outcome from releasing the reservation in between
        //   is why [`LinkWatch`] exists. Without a connection axis it is `None`, and then the reservation ends together with
        //   this function as before (`reservation` is dropped here).
        let watch = match link_rx {
            Some(rx) => Some(LinkWatch {
                rx,
                _reservation: reservation,
            }),
            None => None,
        };
        Ok((SpawnOutcome::Started(info), watch))
    }

    /// ★Manual activation entry point — resume only, fresh-fallback abolished (ADR-0082)★.
    /// The SpawnProfile handler calls this instead of `spawn_agent`. It splits into three branches:
    ///
    /// 1. **Already running (reactivation guard)** — if a session with the same id is alive, it returns that session's AgentInfo as is **without killing or
    ///    respawning anything** (a harmless "already running" signal,
    ///    epoch unchanged). ★This is the core fix of the a4aac1a regression★: previously this path met the "already running" **Err**
    ///    the double-spawn guard of `spawn_agent` emitted, `resume_with_fresh_fallback` mistook it for "resume
    ///    failure" → `fallback_fresh` **killed a live agent running fine** → a new incarnation marker → replaced with an empty fresh one
    ///    (a regression measured by the user). Now it is blocked in two layers: the pre-emptive lookup here filters first and leaves the live agent
    ///    alone, and even if that is passed, that guard now emits `SpawnOutcome::Moot` instead of an Err (there is no error to
    ///    mistake at all).
    /// 2. **Fresh (truly new — no session)** — delegates to `spawn_fresh_settled`. This is not a failure fallback but
    ///    normal new creation (ADR-0076 "Fresh = new sid" valid).
    /// 3. **Resume** — tries only to resume with `resume_no_fallback`, and exposes its Failed outcome as Err.
    ///
    /// ★blocking★: how long the call blocks is decided **not by the mode but by the link's connection axis**.
    ///   - Links not declaring a connection (claude · shell · stdio · codex terminal): Resume polls at most
    ///     EARLY_EXIT_WINDOW (currently 3s), and Fresh returns immediately without polling (the old property as is).
    ///   - Links declaring a connection (codex app-server): **both Fresh and Resume** wait until the connection outcome happens
    ///     — but that is not a window but **a signal** (ADR-0201), so on a normal round trip it returns immediately
    ///     (measured about 130ms). The cap is [`LINK_RESOLUTION_BACKSTOP`], and that is not a decision window but giving up.
    ///   The reactivation guard and the in-flight guard below return immediately without polling on either axis.
    /// **So this verb must be called only on a blocking pool — that is the caller's duty**: the worst case
    ///   is the backstop (15 s) plus, after a failure verdict, the teardown's `session.kill` (pump join
    ///   bounded by 5 s), with **no yield point** in between. Called directly on an async worker it ties up
    ///   that whole worker, and N stuck activations take N workers and stop other connections' commands
    ///   too. All four production callers keep the contract today — the two command-bus ones behind
    ///   `blocking_handler` (`crate::commands::make_table`), the two WS ones behind
    ///   `tokio::task::spawn_blocking` (`connection_core`'s `Spawn` · `SpawnProfile`). **This spot once
    ///   said "only that connection's reply is delayed, other sessions are unaffected"; that is true only
    ///   when callers keep the contract above, and the two WS callers did not** — so it is written as a
    ///   contract, not a property.
    ///   **A failure can return sooner than this cap** — when the diagnostic stream speaks first, it cuts
    ///   off right there.
    /// ★The entry with a caller (`Entry::Caller`)★ — if the save of the pre-launch roster change is `Err`, it does not launch and is
    ///   [`PtyError::Store`] (partial success = [`AgentManager::spawn_agent_watching_link`]). Boot
    ///   restore calls the same verb with `Entry::Internal`.
    // ADR-0082
    // ADR-0076
    // ADR-0201
    // ADR-0291 R17 (E3)
    pub fn activate_profile(
        &self,
        profile: &AgentProfile,
        mode: SpawnMode,
    ) -> Result<AgentInfo, PtyError> {
        // ★★Before answering 「it is up」, look at 「an activation without an outcome yet」 — do not revert the order★★:
        //   on a link declaring a connection, even after the session goes onto the roster, whether that incarnation can take input
        //   is not settled yet ([`LinkWatch`]), and the request opening that interval holds the reservation. Putting the lookup below
        //   first would make a second activation in that interval receive `Ok` (running), and when the first request soon decides a failure and
        //   collects that incarnation, **the two callers split holding different facts** — the latter reports as alive an agent about to die
        //   under it.
        // ★Why `Err` is the honest answer★: this request did nothing and has no live session to return. The Fresh branch below already uses
        //   the same text (when the winner has not yet put it on the roster) — here **that answer is extended until the connection
        //   stands**.
        // ★This guard **narrows** the window **but does not close it**★ — between this check and the reservation acquisition of `spawn_agent_watching_link`
        //   there is still a window a few commands wide (the pre-existing race of ADR-0082 「Open items」 ③).
        //   What was closed is the **long** window that was open for one handshake round trip.
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
            // ★This branch writes nothing to 「the last failure」★: no activation was attempted, so there is no failure to
            //   record and no ground to clear — the same discipline as the moot branch of `spawn_agent` below (ADR-0172).
            return Ok(self.agent_info(&session));
        }

        if mode == SpawnMode::Fresh {
            // ★The record is made inside this — writing here again would make two clearing points★ (only the place that saw the connection outcome
            //   knows what happened. The same discipline as the Resume branch leaving it to `resume_no_fallback`).
            let outcome = self.spawn_fresh_settled(profile, Entry::Caller);
            // If moot, return the session someone else launched (is launching). If it is not on the roster yet, look once more with a lookup.
            // ★The text when even that is missing is not "a nonexistent agent"★: the profile really exists and **another request is
            //   launching it** right now, so we just have no session to return. Letting `NotFound` flow as is would send a text misidentifying
            //   the cause ("agent not found") to the caller and the LLM.
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
            // Both success and failure were already recorded inside `resume_no_fallback` (the only place that knows
            //   what happened). Writing again here would make two clearing points.
            //
            // ★A lookup failure does not flip a success★: the moment the early-exit window passed, we decided success and **cleared**
            //   the record. If the child dies right after and the reaper collects it, the lookup fails, and emitting Err then would
            //   be the contradiction of "answering failure while having cleared the record". So it falls back to the spawn-time information.
            // ★But that information is not returned **as alive**★: the status of the spawn-time snapshot is `Running`, so
            //   emitting it as is would make the caller report a corpse as a live agent (`state_of` translates it to "live", and WS
            //   emits `Spawned` with that status — then the next verb writes mail to a corpse). The lookup having missed it
            //   is itself an observation that "it was collected in between", so it is carried lowered to a terminal state. The code is unknown (`None`).
            // ADR-0172
            // ★`Started` is handled **the same as** `Resumed` — both are 「launched」 and what differs is the reporting vocabulary★:
            //   the former is the case where there was no handle to resume so a new conversation was opened (that decision of `resume_no_fallback`).
            //   Lowering it to `Err` here would report a normally launched agent as a failure.
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
            // It enters only with resumable profiles, so the other outcomes are unreachable (defensive Err).
            // ★The defensive branch does not touch state★: it is a place not knowing what happened, so writing a failure assertively
            //   could stamp an agent that just launched (and without an epoch guard at that).
            other => Err(PtyError::SpawnFailed(format!(
                "activate_profile: unexpected outcome {other:?}"
            ))),
        }
    }

    /// ★**The only point** writing 「the last failure」 (ADR-0172 decision 4 — amended)★. `None` = the activation held
    /// → clear. `Some(kind)` = record on the spot.
    ///
    /// ★Why the clearing condition changed from 「a conversation round trip held」 to 「activation success」 — do not revert★:
    ///   a round trip holding is observed only by **the turn-end signal**, and that signal comes only from structured (stream-json) output
    ///   (for terminal · shell the classifier is silent). Conversely, failure **classification** comes only from console bytes (structured sessions
    ///   send diagnostics to stderr). The two halves are split by output format and mutually exclusive, so by the round-trip criterion
    ///   a terminal agent's record **would never be cleared**. Moreover that signal has no epoch gate
    ///   (`StatusSink::turn_ended` — "surplus is a harmless no-op") and has a non-blocking contract, so taking the profile lock there
    ///   was itself a contract violation.
    /// ★Why using success as the ground for clearing is honest★: if `--resume` passed the early-exit window, the conversation to resume
    ///   **really existed**, and if fresh launched, one now exists. If it is closed afterwards without a single word, **the next
    ///   activation fails again and is recorded again** — that is 「record where it failed instead of detecting in advance」.
    /// ★Called on the same thread as the record★: the thread executing the activation (not the output pump).
    /// ★`incarnation` is the late-write guard★ — the contract is held by `ProfileRegistry::set_last_failure`.
    ///   Outcomes where an incarnation was made (success · early exit) send that epoch, and only failures that ended before the incarnation are `None`.
    // ADR-0172
    fn note_activation_result(
        &self,
        id: AgentId,
        incarnation: Option<u32>,
        failure: Option<AgentFailureKind>,
    ) {
        self.profiles.set_last_failure(id, incarnation, failure);
    }

    /// Whether this entry's activation is **still in progress without an outcome**.
    ///
    /// ★Reading the reservation set as is is all of it — no second state was made★: the lifetime of that set is already
    ///   exactly 「someone is launching this entry」, and on links declaring a connection [`LinkWatch`] extends that
    ///   lifetime **until the connection outcome**. Making a separate 「establishing」 table would add two new places to set and clear,
    ///   and go back to the old shape where the link put its own state in a field and the supervisor peeked at it
    ///   (that shape was the root of the four defects this round removed).
    /// ★The section holding the lock is one `contains`★ — while holding this Mutex, sessions/profiles are not taken
    ///   (ADR-0006 · the `spawning` field doc).
    fn activation_in_flight(&self, id: AgentId) -> bool {
        self.spawning
            .lock()
            .expect("spawning set poisoned")
            .contains(&id)
    }

    /// The record of branches **settled by one spawn** — if launched, clears with that incarnation's epoch; if failed,
    /// records; **if there was nothing to do, writes nothing**.
    ///
    /// ★Not 「the record of Fresh」 — its only caller now is [`AgentManager::spawn_fresh_settled`], and
    ///   even there it is used only for **links without a connection axis**★: on a link declaring a connection, at the point spawn returns `Started`
    ///   the outcome has not happened yet (the handshake has not even started), so clearing here would be exactly
    ///   the evidence destruction ADR-0202 wants to prevent. That branch writes in its own place after seeing the decision.
    ///
    /// ★moot writing nothing is not an exception but the definition★: this call did nothing, so there is no failure to record
    ///   and no ground to clear. The same reason the pre-emptive "already running" branch of `activate_profile` does not
    ///   write.
    /// Resume does not use this — even if spawn is Ok, it holds only after passing the early-exit window, so the place knowing the outcome is
    /// `resume_no_fallback`.
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
    /// ★Why this function was made (regression prevention — deleting it brings that hole right back)★: `spawn_agent` returns `Started` **the moment
    ///   the child process launches** and drops the mailbox. On a link declaring a connection, at that
    ///   point the handshake has not even started — so even when a codex launched Fresh had `thread/start`
    ///   refused, **nobody saw that outcome**, and two things happened at once:
    ///   ① if the other side stayed holding stdout, the reader never saw EOF, so [`OutputCore::finish`] never ran and
    ///      the reaper, the sole consumer of that message, did not move either — **the freeze** came back, Running on screen but every input refused
    ///      (exactly the shape ADR-0200 closed on the resume side).
    ///   ② `note_spawn_result` read that `Started` as success and **cleared 「the last failure」** — a refused
    ///      activation was reported as success while destroying the earlier failure evidence along with it (the hole ADR-0202 wrote down as
    ///      「not yet true」).
    ///   Fresh is **the everyday path** of new codex agents, not a fringe.
    ///
    /// ★So the meaning of 「success」 is kept **the same** as Resume (ADR-0201)★: success = the agent **signaled it is ready to take
    ///   input**. The process being alive is not the answer to that question. Whether a new conversation or
    ///   a resume, what is asked of the other side is the same (`thread/start` vs `thread/resume` — a split inside the link),
    ///   so there is no reason to split the decision vocabulary.
    /// ★Success **does not clear** 「the last failure」 (ADR-0202)★ — the observation of being ready says nothing about what happens
    ///   after, and clearing cannot be undone. Automatic clearing is done in no branch.
    /// ★Links without a connection axis **do not change by a single line**★ — they fall out through the let-else below and take the old
    ///   `note_spawn_result` path (clear if launched) as is. claude's Fresh is not broken now, and
    ///   ADR-0201 pinned that path out of this scope.
    /// ★The failure kind is not classified but left as [`AgentFailureKind::Other`]★ — `resume_failure_kind` emits
    ///   **resume** vocabulary (「no conversation to resume」), and in this branch opening a new conversation that text would be
    ///   false (it never tried to resume). Instead of inventing a missing vocabulary it is left as 「other」 — that one is
    ///   retryable, so the screen does not block the entry either (`failureKinds.ts`).
    /// `entry` is handed to the shared body as received — this verb does not split entries.
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

        // The three that have no outcome to wait for fall out here — links without a connection axis · a request that made nothing (moot) ·
        // a failure that could not even launch a process. All three follow the old record discipline as is (`note_spawn_result`).
        let (Some(watch), Ok(SpawnOutcome::Started(info))) = (watch.as_ref(), &result) else {
            self.note_spawn_result(profile.id, &result);
            return result;
        };
        // ★The marker is carried **out of** the outcome★ — the cleanup and record below happen as late as the decision, and in between this
        //   incarnation may have been collected and the next incarnation launched. Carrying it as a value makes both reach only this incarnation
        //   (ADR-0163/0164 — comparison is match/mismatch only).
        let incarnation = info.epoch;

        match self.link_activation_verdict(
            profile.id,
            incarnation,
            &watch.rx,
            LINK_RESOLUTION_BACKSTOP,
        ) {
            // ★The other side signaled it is ready = holds★. The record is not touched (doc above · ADR-0202).
            //
            // ★★The snapshot is **taken again** — do not return `result` as is★★: the `info` in it was assembled right **after** the child
            //   launched and its status is `Running`. Before, that value was 0ms old so returning it as is
            //   was harmless, but this place is now **after** waiting for the connection outcome, so it is stale by up to the backstop.
            //   If the child died in between, the caller would be told a corpse is a live agent (WS `Spawned` carries that
            //   status as is) and **the next verb would write mail to a corpse.**
            // ★If the lookup fails, it does not flip the success but only lowers the state★ — the lookup having missed it is itself
            //   an observation that 「it was collected in between」. The activation held, so it is not changed to `Err`.
            //   The code is unknown (`None`). It is **the same discipline** as the way `activate_profile` handles the sibling branch of
            //   `resume_no_fallback`, and that comment is the canonical source of this causality.
            EarlyVerdict::Ready => match self.agent_info_by_id(profile.id) {
                Ok(fresh) => Ok(SpawnOutcome::Started(fresh)),
                Err(_) => {
                    let mut gone = info.clone();
                    gone.status = AgentStatus::Exited { code: None };
                    Ok(SpawnOutcome::Started(gone))
                }
            },
            // ★Cleanup **first**, record later★ — the same order as the same branch of `resume_no_fallback`, for the same
            //   reason: the record takes the profile mutex, and if that mutex is held, the side with the order reversed
            //   **never kills the child**. The backstop exists exactly for that stall.
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
            // The process died before the decision finished — the child is already terminal, so there is nothing to clean up (ADR-0082
            // 「kill nothing」: an observed corpse is left as is).
            // ★What the user interrupted is not an activation failure★ — not recorded (the same discipline as `resume_no_fallback`).
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
                // ★The tail is **not carried in the error text**★ — a console tail of up to 4 KiB would go out as is as the command response.
                //   The log above carries it, and the text keeps the same shape as the sibling of `resume_no_fallback`
                //   (one state).
                Err(PtyError::SpawnFailed(format!(
                    "exited before the new conversation was set up ({status:?})"
                )))
            }
            // ★Unreachable — [`AgentManager::link_activation_verdict`] does not emit these two★.
            //   The defensive branch does not touch state: it is a place not knowing what happened, so writing a failure assertively
            //   could stamp an agent that just launched.
            other => Err(PtyError::SpawnFailed(format!(
                "spawn_fresh_settled: unexpected outcome {other:?}"
            ))),
        }
    }

    /// ★The link is not made here (ADR-0191)★ — it receives the already-made one as `parts`, and this function does not know its
    ///   actual type. Before the caller hands it over, **the child process is already up**.
    fn spawn_session(
        &self,
        id: AgentId,
        spec: CommandSpec,
        parts: backend::SpawnParts,
        epoch: u32,
        seed_events: Vec<OutputEvent>,
        incarnation: SpawnIncarnation,
    ) -> Result<(Arc<AgentSession>, Option<u32>), PtyError> {
        // ★Destructured by name — do not drop it with `{ latch: _, .. }`★: the required argument drags the latch only this far
        //   and cannot force carrying it into the session (that wiring is measured by a structure test — `SpawnIncarnation` doc).
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

        // ADR-0113: plugs in the shared turn observation table + this backend's signal classifier together — without them only this session
        //   silently falls out of observation.
        // ADR-0231: this incarnation's new registry + the shared waiting-list table go in the same place — omitting them makes the user list invisible to mail.
        //   The session takes the registry from the core, so it is plugged in only in this one place.
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

        // 2.1. ★ADR-0079 seed-before-publish (load-bearing order — cross-family review 2026-07-13)★:
        //      seeds the restored past events of resume into the core Ring **before the session becomes observable** (= before sessions map insert).
        //      Right now the core is only this function's local Arc, so no other thread can reach it (both the subscribe and emit
        //      paths go through a sessions map lookup). So finishing the seed here blocks the following two windows at the source:
        //        (a) empty-ring replay: if a reconnecting subscription slipped in after insert and before seed, it would replay an empty Ring, and
        //            seed does not fan out, so the past would be lost permanently → removed by seeding before insert.
        //        (b) seq interleave: concurrent emits/writes in that window would mix seqs with the seed and break the Ring order as
        //            [0,2,1], violating replay's partition_point premise → removed by seeding first.
        //      The seed is still before start_pump (live emits start only when the pump is turned on). If seed_events is empty
        //      (Fresh · non-json · no transcript), a no-op → existing fresh buffer behavior unchanged.
        if !seed_events.is_empty() {
            tracing::info!(
                agent = %id,
                epoch,
                count = seed_events.len(),
                "ADR-0079: resume transcript seed (before publish)"
            );
            core.seed(seed_events);
        }

        // ★ADR-0019 finish-snapshot hook★: called once on the finalize winner path of core.finish,
        //   it snapshots intent · shutting_down **at that moment** and sends ReapMsg (no live read at reap time
        //   — prevents the crash→user-kill misclassification race). A send failure (reaper ended) is ignored.
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

        // ★ADR-0113 claiming the turn observation slot — **before** the sessions map insert★: this incarnation takes the entry of that id
        //   (replacing the previous incarnation's entry if any). Before insert no thread can reach this core yet
        //   (the subscribe and emit paths all go through a sessions lookup), so it is **always before this incarnation's first
        //   signal**. If reversed, that first signal would not match the previous incarnation's marker and be dropped, and this incarnation would answer
        //   unobserved (= not in a turn) until the previous incarnation's entry is collected — mail injection mid-turn is that outcome.
        //   ★The old rule of filtering by marker ordering was moved here★ (with random markers ordering is right only half the time —
        //   `turn::TurnObservations::register`).
        // ADR-0113
        self.turns.register(id, epoch);
        // The waiting-list table goes in the same place for the same reason — this table drops writes without registration, so if late, that incarnation's list
        //   would never be visible to mail.
        // ADR-0231
        self.inputs_pending.register(id, epoch);

        // ★ADR-0019 — sessions registration comes **before** the pump start (start)★: the finish hook sends ReapMsg,
        //    and if the pump immediately does EOF→finish, the session must be in the map at that point for the reaper to reap it. Starting before insert
        //    would, on a fast exit, make the hook send point at an id not in the map, reap would be a no-op → the session becomes a zombie.
        //    attach_pump completes synchronously inside start, so there is no effect on join_pump (independent of insert order).
        self.sessions
            .write()
            .expect("sessions poisoned")
            .insert(id, session.clone());

        // 5.5. ★ADR-0019 activation — must be before start_pump★: if spawned (= must be up now), the profile is
        //      settled and persisted as auto_restore=true (so it becomes a boot restore target after a forced exit). This flip
        //      must be put **before** the pump start to close the race: an immediate crash (`cmd /c exit 1`) right after start_pump has
        //      the pump do EOF→finish→reaper lowering auto_restore=false, and if this flip came later than that, it would
        //      overwrite false with true and the crashed session would wrongly remain a boot restore target (crash loop).
        //      The order is fixed as "flip true → start_pump → (on crash) reaper false" so that the reaper's
        //      downgrade (false) is always **last**. Spawn is an activation action, so it is raised only here
        //      (the reaper is downgrade-only — it does not raise to true).
        // ★Always the internal entry regardless of the entry (ADR-0291 E1 · do not change it to `try_`)★: the process is already up,
        //   so it cannot be reverted on a save failure. Even if the save is `Err`, it is raised in memory and dirty — the next successful save carries it.
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
            // If spawn succeeded, the latest marker `epoch_for_spawn` issued is in the roster, so that is read.
            //   The profile is missing only when spawn failed, so the outcome is Failed, and the snapshot value then is
            //   only a notation for the report.
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

    /// ★Boot restore is an internal entry (`Entry::Internal`)★ — there is no person or LLM request to receive the result and revert, so whatever `Err`
    ///   the save of the pre-launch roster change comes back with, it applies to memory + dirty and continues the launch. Not writing the roster over a newer-edition or
    ///   unreadable file is guarded by the re-decision of every save.
    // ADR-0291 R17 (E1 · E3)
    fn restore_one(&self, profile: &AgentProfile) -> RestoreOutcome {
        // ADR-0185: the resume axis — **it does not ask who issued** the stored sid (that axis belongs to
        //   `assigns_session_id` at spawn time). The decision rule is one body with the other activation entries, so dispatch holds it.
        let resumable = backend::can_resume_profile(profile);

        if !resumable {
            // ADR-0172: boot restore follows the same discipline — clear if launched, record on the spot if failed.
            // ★It uses **the same verb** as manual activation (ADR-0201/0202)★: a codex profile without a resume handle
            //   also comes to this branch in boot restore, and if `thread/start` is refused there, the old hole where nobody sees that outcome
            //   would stand **for every profile** at once. Splitting the decision per entry would make one of them
            //   fall behind for sure.
            let outcome = self.spawn_fresh_settled(profile, Entry::Internal);
            return match outcome {
                // moot (already up) is also folded into the same report since the result is "that entry is up" — the restore report
                //   vocabulary has no 「nothing to do」 field (making that field is a wire change, a separate matter).
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

    /// ★Shared discipline for resume only (ADR-0082 — shared by boot restore and manual activation, fresh-fallback abolished)★.
    /// Premise: at call time this profile has already been decided resumable (claude + sid present) — an explicit resume request
    /// comes even without a handle, and for a backend resuming with the stored handle such a request is left to a new conversation at the head (ADR-0226).
    ///
    /// Tries to resume, and on a spawn failure, or if within EARLY_EXIT_WINDOW it **exits abnormally or the diagnostic stream
    /// says it failed** (for an empty/unconversed/damaged session claude emits "No conversation found ..." to stderr),
    /// it goes straight to a terminal point (corpse) **without automatically creating a new conversation (fresh)** — leaving the reason in the log
    /// for the LLM to read and escalate.
    /// ★That terminal point is not `AgentStatus::Failed`★: the state is split only by `TerminalReason`
    ///   (`OutputCore::finish`), and this path has no line writing state — the resume child dies by itself, so
    ///   it goes to `Exited{code}` (usually code≠0). `RestoreOutcome::Failed` below is **a different axis** (the activation result).
    /// ★It is not "dying immediately" (corrected by measurement 2026-08-23)★: claude lives a few seconds more while its exit hook runs even after emitting that diagnostic.
    ///   So the decision stands not only on death but also on evidence (`EarlyVerdict::Diagnosed`).
    /// ★Nothing is killed or respawned★: the resume child does EOF→finish through its own pump, and the reaper
    ///   collects that session from the map and lowers the profile to `auto_restore=false` (KeepDisableAutoRestore),
    ///   leaving it in the tree as a corpse **in the observed terminal state as is** (usually `Exited{code≠0}` — the profile is not deleted:
    ///   exit≠0/unknown is not a deletion target).
    ///   This helper only observes the exit and does no destructive action (the old fallback_fresh's remove_session ·
    ///   incarnation marker reissue · respawn were all removed — ADR-0082 user decision: "kill nothing, create nothing new").
    /// restore_one (boot restore) and activate_profile (manual activation) reuse this logic **identically**.
    ///
    /// The second field of the return = the incarnation this attempt **produced** (if any). Used by `activate_profile` so as not to flip a success when the lookup
    /// fails after the success decision (that comment is the canonical source of the causality).
    /// `entry` is handed to the shared body as received — this verb does not split entries.
    /// ★`Err` = the save of the pre-launch roster change was `Err`, so it did not launch (happens only at the entry with a caller)★ — do not fold it into
    ///   a `RestoreOutcome::Failed` text like other launch failures: folding it would keep `activate_profile` from returning `PtyError::Store`,
    ///   so the bus could not tell a save refusal from other failures. The failure record and log are left the same as when folding.
    // ADR-0082
    // ADR-0172
    // ADR-0291 R17
    fn resume_no_fallback(
        &self,
        profile: &AgentProfile,
        entry: Entry,
    ) -> Result<(RestoreOutcome, Option<AgentInfo>), StoreError> {
        // ★A resume request without a resume handle **opens a new conversation** — not a choice but the consequence of user decision D1
        //   (「a stored id exists ⟺ there is a conversation to resume · no id → Fresh」)★.
        //   ★What actually comes into that combination is the explicit `resume: true` request of WS `SpawnProfile`★ — that
        //   flag leaves it as Resume even without a stored session (that place's comment).
        //   ★Instead of launching as Resume and only changing the report, it is handed to the Fresh path entirely★: a backend resuming with the stored handle
        //   silently drops the resume flag and launches when there is no handle, so launching as Resume would report a new conversation
        //   with an unknown id as 「resumed」. The outcome is `Started`, not `Resumed`.
        //   ★It does not clash with 「do not create a new conversation」 (ADR-0082)★ — what that decision guards is requests **with a conversation
        //   to resume**, and here is a request with nothing to resume in the first place (normal creation).
        //   This decision is the front door cheaply filtering the common case, and the race where the field empties between this decision and the handle read inside spawn
        //   is refused by spawn before launching the process (the same predicate).
        //   Backends that do not resume with the stored handle, like shell, are not caught here and take the old way as is.
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
                // ADR-0172: failure is recorded where it was attempted — this record is the screen/API-side real thing of what ADR-0082 required, "leave
                //   the cause for the control LLM to read" (logs are read only by people).
                self.note_activation_result(profile.id, None, Some(spawn_error_kind(&e)));
                // ★A save `Err` does not emit the warning below★ — it stopped before opening the process, so there is no terminal point (corpse), and the surface turning that error
                //   into a caller error (bus · WS) leaves one line. Emitting here too would make two lines for one request.
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
            // ★There was nothing to do — no reason to watch the early-exit window either★: it is a session someone else launched (is launching), so we
            //   observed nothing, and so neither record nor clear (the same discipline as `note_spawn_result`).
            //   The report is folded into 「it is up」 — the restore vocabulary has no 「nothing to do」 field.
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

        // ★fable M-1★: a successful claude resume is a TUI, so it does not exit within the window.
        // So entering terminal within the window is a resume failure signal regardless of the code
        // (misjudging a code==0 early exit as Resumed would misreport an empty screen as "restore succeeded").
        //
        // ★`spawned.epoch` is carried **out of** the window (late-write guard)★: the two writes below happen up to
        //   EARLY_EXIT_WINDOW later, and by then the reservation is already released. If in between another connection successfully activates the same
        //   profile, the epoch goes up, so sending this value keeps the old observation from overwriting the new incarnation
        //   (the `set_last_failure` contract).
        // ADR-0172
        // ★The decision path is split **by the link's axis**★: with a delivery channel it waits for the outcome, without one it
        //   takes the old window decision as is. Why the two are not mixed in one loop = the `link_activation_verdict` doc.
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
                // ★What the user interrupted is not an activation failure — not recorded★: if a kill comes within the window
                //   (tree termination · WS `Kill` frame · daemon shutdown path), it is not a failure for us to observe but an explicit
                //   ★at one point this said `agent.kill`, but **there is no such verb**★ — kill is **intentionally** absent from the command bus's
                //   `agent.*` (`types::CLI_AGENT_VERBS` comment · ADR-0122
                //   unresolved). So today the only surface a kill reaches is the WS frame.
                //   intervention. Recording it would leave 「it exited right after resuming」 in the tree, making an entry the user just
                //   turned off themselves look broken. The activation result is still Failed (the agent is not up).
                //   Nor is it cleared — there is no ground to claim success for this attempt either.
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
            // ★Still alive but folded into failure — and **nothing is killed**★: claude lives a few seconds more while its exit hook runs
            //   even after emitting the diagnostic (measured +2.2s diagnostic / +6.4s exit). Waiting for that corpse
            //   would pass the window and decide the failure as success, so it is settled here. The disposition is still observation only
            //   — the child does EOF→finish through its own pump and the reaper collects it (ADR-0082 "kill nothing").
            // ★So for a few seconds the combination 「running + last failure」 stands on screen — not a bug★: the two axes
            //   are separate, so it is an expressible state, and the tree draws that interval with 「running wins」 (ADR-0173).
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
            // ★The process is alive but **unusable** — the place where being alive is not counted as success★.
            //
            // The outcome back when this branch did not exist was exactly what ADR-0082 wanted to prevent: even when the resume was refused,
            //   the child was fine and the refusal came over stdout, so the window saw only `Running`, fell to `Alive` below,
            //   **was reported as success and even cleared the last failure record.**
            // ★The evidence is `reason` alone★ — there is no corpse, so both the console tail and the diagnostic tail are empty. So
            //   that string is passed as is as the classification input (what it means is backend knowledge — ADR-0004).
            // ★★The disposition **differs** from `Diagnosed` — this branch collects★★: the link only goes as far as releasing its own stdin
            //   (the line of ADR-0199), and if the other side stays holding stdout that chain does not run. So
            //   below `tear_down_failed_activation` collects it with the existing 2 verbs (that function's doc is the canonical source of 「why the manager is the one」).
            //   ★At one point this said 「kills nothing · the manager only observes」, but fourteen lines
            //   below there is that kill★ — that sentence is a leftover of the round where the link waited boundedly and collected by itself.
            //   Why it does not conflict with 「kill nothing」 of ADR-0082 (an observed corpse vs an incarnation whose terminal point never
            //   stood) is also in that function's doc.
            EarlyVerdict::LinkFailed { reason } => {
                let kind = backend::resume_failure_kind(&profile.command, &reason)
                    .unwrap_or(AgentFailureKind::EarlyExitAfterResume);
                let reason = format!("resume connection failed: {reason}");
                // ★★Cleanup **first** — before the record★★: the record takes the profile mutex, and that mutex
                //   may be held by the session id record path **while holding** a disk write. Reversing the order
                //   would make that stall become 「never kills the child」 — the backstop exists exactly for that stall,
                //   and it would become unusable because of that very stall.
                //   ★At one point this said 「the record must come first — killing first sets `Killed` and it cannot be told from a user exit」,
                //   but **that reason was wrong**★: that discipline is
                //   inside the terminal branch of `early_activation_verdict`, and that function has already made its decision and
                //   returned. A state standing **after** the decision cannot change that decision.
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
            // ★The other side **signaled it is ready** = holds (user decision)★ — not watching a window but receiving positive evidence.
            //   So this branch arrives **sooner** than `Alive`, and **on stronger grounds**.
            //
            // ★★But 「the last failure」 is **not cleared**★★ (user condition): this success is optimistic —
            //   what is observed goes only as far as the agent being able to take input, and what happens after that (turn errors · connection
            //   drops) is **a different event carried by a different path**. Half of the original defect was 「a success report erasing the failure evidence with it」,
            //   and as the decision got faster, that destruction would happen earlier.
            // ★So the clearing rule is set like this — **nothing is cleared automatically**★:
            //   the question this record answers is 「how the last activation ended」, and the only evidence invalidating it is
            //   **the next failure overwriting it** (`set_last_failure(Some(..))`) and **the profile itself
            //   disappearing**. 「it is up now」 is not an answer to that question.
            //   ★The cost written honestly★: so a stale failure may remain next to a live agent. On screen
            //   that combination is drawn with 「running wins」 (ADR-0173), so it does not lead to misunderstanding, and the cost of the opposite direction
            //   (evidence loss) cannot be undone — so the side of keeping it is chosen.
            // ★claude does not come to this branch★ — its link has no connection axis, so it goes to `Alive` below.
            //   Do not widen the clearing rule to that side (a separate matter, and that path is outside this decision's scope).
            EarlyVerdict::Ready => {
                // A request without a handle already fell out to a new conversation at the head above, so what comes here is only resumes.
                (RestoreOutcome::Resumed, Some(spawned))
            }
            EarlyVerdict::Alive => {
                // ★Passed the early-exit window and the diagnostics stayed silent = the conversation to resume really existed★ — this is the ground of 「clearing」
                //   (the `note_activation_result` comment).
                // ★Only **links without a connection axis** come to this branch★ (claude · shell · stdio) — links declaring
                //   a connection are already split at `Ready`/`LinkFailed` above. So the clearing rule remaining here
                //   is the same as leaving the claude path as is.
                self.note_activation_result(profile.id, Some(spawned.epoch), None);
                (RestoreOutcome::Resumed, Some(spawned))
            }
        })
    }

    /// Ends a session whose activation is **settled as failed** — meaningful only while the process is still alive.
    ///
    /// ★Why the manager is the one★: when a resume is refused, the link releases only its own stdin and then waits for the other side to end
    ///   by itself. If the other side stays holding stdout, the reader never sees EOF, `OutputCore::finish` never runs,
    ///   and the reaper, the sole consumer of that message, does not move either — the child, Job and threads stay tied up for the whole daemon lifetime.
    ///   The owner of that disposition is **the kill handler** (the 2 verbs of ADR-0001), not the link.
    ///   ★There was once a place where the link's writer killed directly, but that crossed the line ADR-0199 drew, so
    ///   it was removed — do not bring it back★. This is its replacement, and not a newly invented verb but an existing teardown.
    /// ★Why `kill_agent` is not called as is = **the user-exit marker**★: that one sets
    ///   `TerminationIntent::UserKill`, but this is not the user turning it off. That marker goes out frozen in `ReapMsg`,
    ///   so setting it falsely would make the whole exit classification record false.
    /// ★It does not conflict with ADR-0082 「kill nothing」★ — what that decision guards is **an observed terminal point (corpse)**, and
    ///   what is ended here is an incarnation whose terminal point **never once stood**. Killing it does not erase evidence but
    ///   **creates evidence that did not exist (the exit transition)**. The failure reason is already recorded **before** this call.
    /// ★So it is not called in the `Terminal`·`Diagnosed` branches★ — there the child has already died (corpse kept),
    ///   or claude is running its exit hook, so ADR-0082 applies as is.
    fn tear_down_failed_activation(&self, id: AgentId, incarnation: u32) {
        let Ok(session) = self.get_session(id) else {
            return;
        };
        // ★★If the marker differs, **it is not touched**★★: this cleanup arrives as late as the decision, and in between this incarnation
        //   may have been collected and **the next incarnation launched.** Finding and killing by id alone would then kill not the failed session but
        //   **the healthy successor that just launched**, and `revoke` would also erase that successor's token.
        //   ★Comparison is match/mismatch only★ — "newer" is not derived from ordering (ADR-0163/0164).
        if session.epoch != incarnation {
            tracing::info!(
                agent = %id,
                expected = incarnation,
                live = session.epoch,
                "skipping cleanup of the failed activation — another incarnation stood in between (not killing a live session)"
            );
            return;
        }
        // The retreat notice is the first step (reason = the same line of `kill_agent`). ★It must come **after** the marker comparison★ — before it, the live successor's
        //   the link would receive an irreversible retreat marker.
        session.begin_retire();
        self.control.revoke(id, session.epoch);
        let _ = session.enter_exiting();
        session.kill(Duration::from_secs(5));
    }

    /// After spawn, decides the resume outcome within the window — **death or evidence** (`EarlyVerdict`).
    ///
    /// ★Evidence is not looked at before death (the order inside the loop — load-bearing)★: on each poll the state is looked at first,
    ///   then the diagnostics. Reversed, a **user exit** arriving within the window would be caught first as `Diagnosed`,
    ///   bypassing the "what the user interrupted is not a failure" discipline (`resume_no_fallback` below).
    /// ★Only the stderr stream settles by diagnostics alone — do not put the console ring in here★: the console ring receives
    ///   **the conversation body claude redrew** on resume as a whole. Looking for failure text in the ring of a live session would,
    ///   the moment it resumes a conversation that once talked about that text (sessions of this repository actually do),
    ///   **stamp a successful activation as a failure.** stderr is claude's own diagnostic channel, so the conversation body
    ///   does not mix in and that false hit is impossible at the source. The console tail is used as evidence only **after death** (below).
    /// ★The session Arc is taken **once outside the loop** (load-bearing — do not revert)★: calling `get_session` every time inside the loop
    ///   misses the tail almost always. `finish` wakes the reaper **right after** setting the terminal state,
    ///   and the reaper's first action is to remove the session from the roster, while this loop's interval is 100ms — that is, by the time we
    ///   wake up the lookup has already failed and falls to an empty tail, and the classification always becomes the contextual default
    ///   (then the 「no conversation to resume」 decision effectively becomes dead code). Holding the Arc keeps seeing the same `OutputCore`
    ///   even after it leaves the roster, so the terminal state, the output ring and the diagnostic buffer are all readable as is.
    /// ★The first lookup failure is not "there was no session"★: between `spawn_agent` returning Ok and reaching this lookup, the child
    ///   may have died and the reaper already collected it (a fast immediate-death batch actually produces that order). In that case the tail
    ///   is missed, so the classification falls to the contextual default — either way it is on the retryable side, fail-open, so
    ///   the two causes are not told apart here.
    /// ★The two kinds of evidence are exclusive, so they are passed merged★: for a PTY (terminal) session stderr is merged into the console and
    ///   is only in the ring, and for a pipe (structured) session it is only in the diagnostic buffer. Either way an empty value is a normal result, and
    ///   if both are empty the caller falls back to the contextual default (fail-open).
    /// ★Why it receives `command`★: "what this text means" is backend knowledge, so the decision is delegated to `backend`
    ///   (ADR-0004) — the manager does not read strings directly.
    // ADR-0172
    fn early_activation_verdict(
        &self,
        id: AgentId,
        command: &AgentCommand,
        window: Duration,
        // ★This branch is only for links without a connection axis, so it does not use the backstop★ — keeping the argument is to keep the call shape of the two
        //   decisions the same, and the fact that it is unused is itself the contract.
        _link_backstop: Duration,
    ) -> EarlyVerdict {
        let session = match self.get_session(id) {
            Ok(s) => s,
            // Not in the roster = not on it yet or already collected → either way treated as exited (the doc above).
            // ★This `AgentStatus::Failed` is **not published**★: it is a local value that does not reach `StatusSink`, so
            //   it does not go out to the screen or frontend, and is used only in the reason string `format!("resume exited early ({status:?})")` below and
            //   the `matches!(status, AgentStatus::Killed)` decision. So "a published `Failed` is
            //   only for pump panics" holds as is — but writing *only two places in the code* would be false here
            //   (the docs said so and were corrected on 2026-08-25. Canonical = docs/reference/architecture.md 「Session restore · activation」).
            Err(_) => {
                return EarlyVerdict::Terminal {
                    status: AgentStatus::Failed {
                        message: "session gone".into(),
                    },
                    evidence: String::new(),
                }
            }
        };
        // ★Only links **without** a connection axis come here★ — the side with the axis has
        //   [`AgentManager::link_activation_verdict`] below wait for delivery. This branch is the way of claude · shell · stdio ·
        //   codex terminal, and is **byte for byte the same** as before round 2 (one window, outcome `Alive`).
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

    /// Moves the user's cancellation into the decision — ★`Killed` is **set here**★.
    ///
    /// ★This `AgentStatus::Killed` is not published★: it is a local value that does not reach `StatusSink`, so it does not go out to the screen or frontend,
    ///   and is used only in the caller's reason string and the `matches!(status, AgentStatus::Killed)` decision
    ///   (the same kind and the same grounds as the `"session gone"` value of `early_activation_verdict`).
    /// ★It means not 「already dead」 but 「the user decided to interrupt」★ — at that moment the child may still be alive
    ///   (`session.kill()` is running). What we assert is only that **the user decided the outcome of this
    ///   activation**, and the only thing the caller does with that value is 「not record it as a failure」.
    /// ★★The reason of the intercepted delivery is **not carried here — because there is nowhere to carry it**★★:
    ///   the two consumers receiving this value (`spawn_fresh_settled` · `resume_no_fallback`) **record nothing and do not read `evidence`**
    ///   in the `Killed` branch — because what the user turned off is not a failure to classify.
    ///   At one point this function appended that reason to `evidence` and wrote 「if not carried here, it remains nowhere」,
    ///   but **both were false**: after carrying it nobody read it, and that text already remains —
    ///   the link emits it with `tracing::warn!` as soon as it enters the failure branch (`writer_loop` of `backend/codex/transport.rs`,
    ///   **before** `deliver_link`, and **not suppressed** by the exit latch). It was a place promising preservation that did not exist,
    ///   so it was removed — the same kind as the `take()` claim removed last round.
    /// ★So the tail is not dumped into the log either★ — printing `evidence` in the `Killed` branch would send, every time the user turns it off,
    ///   a console tail of up to 4 KiB (which may contain the conversation body) to the daemon log. There is nothing to gain.
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
    /// ★Why this function is separate★: read-based decisions and delivery-based decisions cannot go into the same loop. Mixing them would revive
    ///   「the gap between reading and acting」, and that gap is exactly the four defects this round closed. And
    ///   not touching **a single line** of the way of backends without the axis is the second worth of this separation.
    ///
    /// It looks at only four things — ★the first comes **before the other three**★:
    ///   1. **The user cancellation latch** — `kill_agent` sets it **before** bringing the link down. It is visible before delivery, channel loss and state,
    ///      so without looking at it first, the user's cancellation masquerades as a failure or a success
    ///      (the comments of the two branches below are the canonical source of those two masquerades).
    ///   2. **The delivered outcome** — once sent, it is not withdrawn. Accepted only when the incarnation marker is this spawn's.
    ///   3. **The terminal state** — a latch (`OutputCore` sets it only once). So reading it does not flip it.
    ///   4. **The backstop** — a liveness device for paths where the link **cannot** produce an outcome, not a decision window
    ///      ([`LINK_RESOLUTION_BACKSTOP`]).
    ///
    /// ★★The terminal **does not** beat delivery — only cancellation beats it★★: if `Ready` was actually delivered, the other side
    ///   **signaled** it was ready, and dying by itself after that is what ADR-0201 pinned as 「a separate event」
    ///   (pump → terminal → the reaper sees it). So its own death does not flip the success decision —
    ///   instead the caller **takes the state at that moment again** and returns it (the `Ready` branch of `spawn_fresh_settled` ·
    ///   the Resume branch of `activate_profile`). User cancellation is different: it is **a person deciding** the outcome of this activation,
    ///   so the decision itself changes.
    /// ★It receives `epoch` and accepts **only when they match**★ — why that mismatch does not actually happen today and why it is still
    ///   kept are held by the doc of `as_verdict` below (not repeated here).
    /// ★Right before the backstop it **checks delivery once more**★ — if the time limit and delivery overlap, delivery wins. Otherwise
    ///   a connection that just held would be recorded as a failure and die (defect ②: the original defect with only the sign flipped).
    fn link_activation_verdict(
        &self,
        id: AgentId,
        epoch: u32,
        link_rx: &Receiver<LinkVerdict>,
        backstop: Duration,
    ) -> EarlyVerdict {
        /// Moves one delivery into a decision — if the marker differs, it is **dropped** (it is another incarnation's outcome).
        ///
        /// ★★This guard **does not fire in production today — kept knowingly**★★: the mailbox is made anew per spawn
        ///   ([`AgentManager::spawn_agent_watching_link`]) and the marker is baked into the port closure then,
        ///   so every message coming into this `rx` carries by definition the very marker we wait for. That is, what actually blocks
        ///   mismatches today is not this `if` but **the freshness of the channel**.
        /// ★Why it is still not deleted★: if a change comes that makes the channel shared (one supervisor watching several spawns, or
        ///   reusing a port), at that moment this becomes the only wall. It is cheap and right.
        /// ★At one point this place said 「this guard prevents a collected incarnation's outcome from being applied to the successor that just launched」,
        ///   but **read that way it is false**★ — the real reason that does not happen is channel freshness,
        ///   so whoever believes that sentence ends up relying on a guard that never fired. The firing path exists only in the test rig
        ///   (`a_verdict_from_another_incarnation_is_discarded` builds the channel by hand).
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
        /// ★This cannot be seen through the terminal state★: `kill_agent` goes in the order `set_intent(UserKill)` → `enter_exiting()`
        ///   → `session.kill()`, and the terminal transition (`Killed`) stands only after that when the pump wakes up. The
        ///   state in between is `Exiting`, **not terminal**, and if the link disappears in that interval, the `Disconnected`
        ///   branch below records it as 「connection failure」 — the user's cancellation masquerades as a resume failure.
        /// ★This latch is **always visible before** all those observations★ — the order above guarantees it
        ///   (the doc of `AgentSession::termination_intent` is the canonical source of that causality).
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
            // ★The reason the wait is cut into slices is **to look at the terminal latch together**★ — delivery wakes this wait
            //   immediately, so the slice length does not create success latency.
            let slice = deadline
                .saturating_duration_since(Instant::now())
                .min(Duration::from_millis(100));
            match link_rx.recv_timeout(slice) {
                Ok(v) => {
                    // ★★User cancellation **beats delivery**★★: if the user interrupts right after the handshake succeeded and `Ready` stood in the queue,
                    //   this place took that `Ready` as is and **reported a dead agent
                    //   as a success** (`restore_one` as `Resumed`, `activate_profile` as `Ok`).
                    //   Looking at the latch first makes that combination fall to 「what the user turned off」, making both the record and the report honest.
                    // ★The reason of the dropped delivery is **not carried here**★ — the receiver does not read `evidence` in the `Killed` branch,
                    //   and the link already logged that text (`cancelled_verdict` doc).
                    if user_cancelled(&session) {
                        return Self::cancelled_verdict(&session);
                    }
                    if let Some(verdict) = as_verdict(v, epoch) {
                        return verdict;
                    }
                    continue;
                }
                Err(RecvTimeoutError::Disconnected) => {
                    // ★★The latch is looked at **before the state** — do not revert the order★★: one common cause of the link disappearing
                    //   is **the user kill itself**. The kill suppresses delivery (the link's
                    //   `deliver_link`) and ends the writer, and when the writer ends the sending end drops and this branch
                    //   wakes up — at that time `session.kill()` is still inside `child.kill()`/`wait()` and the pump
                    //   has not run `finish`, so the state is `Exiting` (**not terminal**).
                    //   The old shape that looked only at the state settled with that one read 「the link disappeared without delivering the outcome」,
                    //   and so **the user's cancellation was recorded as a resume failure** —
                    //   exactly the outcome the suppression device meant to prevent.
                    // ★Having no grace period is intended★ — on the kill path this latch stands **always before** the link disappears,
                    //   so there is no reason to wait, and for a disappearance that is not a kill, even a grace period would give the same failure
                    //   outcome (only the text would differ).
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

    /// `requested_epoch` = the incarnation marker the subscriber last saw. The session compares it with its own marker.
    ///
    /// ★The session lookup is one — do not add a second lookup★: the response's incarnation marker, resume marker and the replay
    ///   must come from the same session. Looking up separately, the incarnation could change in between and the marker would speak of
    ///   a different incarnation from the one replayed.
    ///
    /// ## ★Contract: `Err` ⟹ `on_ready` is never called (load-bearing — breaking it kills output)★
    /// The only failure is **the one session lookup**, and that is structurally *before* handing over `on_ready`. The daemon's
    /// subscription refusal notice rests on this order: the daemon turns `Err` into `AgentEvent::SubscribeFailed` and sends it, advertising as a contract
    /// "neither `SubscribeAck` (= `on_ready`) nor `ReplayComplete` follows this subscription",
    /// and the client (src-tauri) relies on that advertisement to release its single-flight slot **immediately**.
    ///
    /// So if a branch that can fail *after* calling `on_ready` is added to this function, the refusal and the Ack go out together for the same subscription,
    /// and a late Ack/Complete arrives over the slot the client already released, attaching **a success marker to a generation where the replay
    /// did not run** (gen fence collapse). If a new failure branch is needed, put it before `on_ready`, or fix the contract of the refusal
    /// notice together. Regression net = `subscribe_from_err_never_invokes_on_ready`.
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

    /// Keystroke and turn writes carrying the origin — human input (WS `WriteStdin`) is `User`, non-human callers `Mail`.
    /// ★The `*_observed` verbs take no origin — they are all `Mail`★ (the mail delivery path).
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

    /// The **submit-including** version of `write_stdin_observed` — if the backend requires submit bytes (terminal), they go out once more
    /// as a separate write after the body (grounds · measurement = `AgentSession::submit_input_observed`).
    ///
    /// ★Which to call = the nature of the caller★: this for "one completed message = one turn" (mail delivery), and
    ///   `write_stdin`/`write_stdin_observed` for keystroke streaming where a person types Enter directly.
    pub fn submit_stdin_observed(
        &self,
        agent_id: AgentId,
        data: &[u8],
    ) -> Result<crate::types::WriteOutcome, PtyError> {
        self.get_session(agent_id)?.submit_input_observed(data)
    }

    /// ★Incarnation-conditional write★ — writes only when `expected_epoch` equals the epoch of the session that AgentId points at **now**.
    ///   If different, it does not touch the transport at all (zero side effects) and emits `Err`.
    ///
    /// ★Why it is needed (check-then-write TOCTOU — load-bearing)★: if a caller decides the recipient by `(id, epoch)` and then
    ///   calls `write_stdin_observed(id, ..)`, when the agent restarted in between (= session map replacement + a new incarnation marker),
    ///   the write lands on **the new incarnation**. As long as resolution and write are separate operations, however early the caller checks,
    ///   that window does not close — it closes only by pulling the decision into **the same unit as the write**. That is why this function exists.
    /// ★★No consumers (since ADR-0111 decision 6)★★: the **only** caller of this verb was the group broadcast binding of daemon messaging
    ///   (= "only to the incarnation alive at the moment of sending"), and **that invariant was abolished** — parked mail is also delivered to a new
    ///   incarnation of the same name. So no code calls this function now.
    ///   ★Why it was kept · the revival condition★: if "this mail goes only to the incarnation at the moment of sending" is needed again, it was decided
    ///   to add it non-destructively as a v2 **personal mail option** (spec §8), and what is needed then is exactly this conditional write. Reviving it as a group-only
    ///   rule without a formal reopening violates ADR-0111.
    ///
    /// ★Why this actually closes the window (read together with the ADR-0006 lock discipline)★: `get_session` takes the sessions read lock,
    ///   clones the `Arc<AgentSession>` and **releases immediately**. The epoch comparison and the write after that happen on **the same Arc**,
    ///   and `AgentSession.epoch` is an immutable field fixed at creation (a restart only *replaces* the session
    ///   and does not change the epoch of the existing session). So even if the map is replaced after the comparison, the target we write to cannot
    ///   change — when this function returns `Ok`, "it wrote to exactly the session with epoch == expected" is true.
    ///
    /// ★Mismatch signal = `PtyError::Unsupported` (no dedicated variant is made)★: the only fact the caller needs is
    ///   "this verb could not be performed **on this target now** and nothing was written", and that is the same shape as the existing
    ///   unsupported signal. The message identifies the cause (it carries the requested epoch / current epoch) —
    ///   growing the error vocabulary would widen every caller's match because of this one branch.
    // ADR-0088
    // ADR-0111
    pub fn write_stdin_observed_if_epoch(
        &self,
        agent_id: AgentId,
        expected_epoch: u32,
        data: &[u8],
    ) -> Result<crate::types::WriteOutcome, PtyError> {
        let session = self.get_session(agent_id)?;
        // ★Zero side-effect guarantee★: on a mismatch it leaves **before** touching the transport — the caller must be able to settle this Err as "not sent"
        //   for the reparking and skip decisions to hold.
        if session.epoch != expected_epoch {
            return Err(PtyError::Unsupported(format!(
                "epoch mismatch: agent {agent_id} is now at epoch {}, caller required {expected_epoch} — nothing was written",
                session.epoch
            )));
        }
        session.write_input_observed(data)
    }

    /// The waiting list of the live incarnation — the bus `agent.listQueuedInputs` and the WS list query carry this same value.
    /// `Err(NotFound)` = no live session (an asleep agent holds no list).
    /// ★Locks are not held overlapping★: registry (snapshot) → release → link (acceptance unknown) → turn observation table, one at a time in order.
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

    /// Cancels one queued input of the live incarnation (outcome translation in `AgentSession::cancel_queued_input`).
    /// `NotFound` if there is no live session — an asleep agent has no entries to cancel.
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

    /// ★Harness-only session injection seam (ADR-0088 / ADR-0012)★ — registers a pre-assembled `AgentSession` (including a test transport)
    ///   directly in the sessions map. The purpose is to drive the delivery-boundary observation tests (reachable = structured carrier but write success/failure)
    ///   **without binary dependencies**, without going through the spawn pipe (real PTY · claude binary) — the daemon integration tests must see it
    ///   cross-crate, so it is gated by the `test-harness` feature (`#[doc(hidden)]` does not block access — injecting an arbitrary `AgentSession` bypasses
    ///   the spawn reservation, profile+epoch coordination, control-token issuing, pump/reaper wiring and tracker lifetime entirely,
    ///   so it must not be compiled into production builds at all). Feature OFF = this method absent in production builds. The feature
    ///   is turned on only in the daemon's `[dev-dependencies]` (not a production dep), so it is not unified into the production daemon
    ///   binary. Runtime/production paths never call it — only spawn_session is the regular registration point.
    ///
    /// ★Safety/invariants★: (a) reaper not wired — the injected session does not start the pump, so there is no finish hook,
    ///   and no ReapMsg goes out, so it stays in the sessions map until the manager Drop (no exposed remove, and `kill_agent` does not
    ///   remove the injected session from the map either — each test uses a fresh manager and is cleaned up by its Drop).
    ///   (b) lock discipline (ADR-0006) — takes the sessions write lock, inserts and releases immediately, takes no internal lock.
    ///   (c) profiles untouched — no auto_restore flip or persist (pure map registration). Reinjecting the same id replaces it.
    #[cfg(feature = "test-harness")]
    #[doc(hidden)]
    pub fn insert_test_session(&self, session: Arc<AgentSession>) {
        self.sessions
            .write()
            .expect("sessions poisoned")
            .insert(session.id, session);
    }

    /// ★Harness only★ — the pair of `insert_test_session`. Builds an `OutputCore` **connected to this manager's notification path and turn observation table**
    ///   (the same shape as the wiring spawn_session does).
    ///
    /// ★Why it is needed★: if an injected session were assembled with `OutputCore::new` alone, that session's emits would not reach the manager's table,
    ///   and integration tests wanting to see the gate and doorbell wiring would become false tests that "pass because there is no observation".
    ///   Conversely, tests that need no observation simply do not use this (unlike production sessions, it is optional).
    /// ★The waiting-list table is registered here (the turn observation table is not)★ — the turn observation table accepts writes without registration, but
    ///   the waiting-list table drops them. Without registration, this core's list would never be visible to the manager's mail busy.
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

    /// Agent termination (delegated to the ADR-0019 reaper). **No map removal, disposition or notification is done** — the reaper singly consumes and handles
    /// the ReapMsg the pump sent. So the session may still be in the map right after return —
    /// a caller wanting to assert "gone" must poll (headless tests do that).
    pub fn kill_agent(&self, agent_id: AgentId) -> Result<(), PtyError> {
        let session = self.get_session(agent_id)?;
        // ★The retreat notice is the first step — before the permission revoke · intent · `Exiting` · `session.kill`★: it keeps the window where the work the link runs
        //   behind overlaps with the termination below as narrow as possible. It is a notice that collects no resources, so the causality below (ADR-0001) is unchanged, and
        //   the roster lock was already released by `get_session` (ADR-0006).
        session.begin_retire();
        let epoch = session.epoch;

        // 0. ★Discard the control channel token immediately — **before** the blocking kill (FIX 4)★. If this revoke were **after** session.kill (up to
        //    5s join), the dying agent's token would be valid during that 5s window, and in between the agent could
        //    issue commands through the control channel (TOCTOU). Here no lock is held, so it is safe (§10 — the registry is a leaf
        //    lock, ADR-0006), and revoke is idempotent, so overlapping with the reaper's terminal revoke is harmless (that is the backstop).
        // ADR-0086
        self.control.revoke(agent_id, epoch);

        session.set_intent(TerminationIntent::UserKill);

        let _ = session.enter_exiting();

        // 1~6. ★The revoke placement does not touch this causality (ADR-0001)★ — revoke touches only the registry/files and
        //       does not intervene in the shutdown chain, so it does not block or reorder the kill. Even if the join times out, it just
        //       proceeds (the Arc is cut by session removal, so it ends naturally).
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

    /// A lightweight sibling that looks only at the epoch, avoiding the full list_agents traversal and AgentInfo assembly (profiles lock).
    /// ★There is no production caller★ — the marker of the subscription response is carried by the reply of `subscribe_from` (ADR-0226). The remaining
    ///   consumer is the daemon integration test (`tests/ws_e2e.rs`).
    pub fn agent_epoch(&self, agent_id: AgentId) -> Option<u32> {
        self.sessions
            .read()
            .expect("sessions poisoned")
            .get(&agent_id)
            .map(|s| s.epoch)
    }

    pub fn shutdown_all(&self) {
        // ★ADR-0019★: shutting_down is set **before** each kill. If it came later than the kill, a session that exited in that gap
        //   would have its finish hook snapshot shutting_down=false and get KeepDisableAutoRestore
        //   (auto_restore=false → dropped from boot restore targets), a race. With the set first,
        //   every finish after this point snapshots shutting_down=true → the reaper does KeepAsIs (left untouched).
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

    /// Used by activate_profile to get the live session's info after a resume succeeds — resume_no_fallback
    /// only registers the session in the map and does not return info (it returns RestoreOutcome), so it looks up again by id.
    fn agent_info_by_id(&self, id: AgentId) -> Result<AgentInfo, PtyError> {
        let session = self.get_session(id)?;
        Ok(self.agent_info(&session))
    }

    /// The single source letting callers that do not need the whole AgentInfo, such as the envelope sender, get a name **byte-identical to agent_info**
    /// — gathering the session.cwd-based resolve here in one place prevents duplicated logic.
    pub fn canonical_name(&self, id: AgentId) -> Option<String> {
        let session = self.get_session(id).ok()?;
        Some(self.resolve_canonical_name(&session))
    }

    /// The shared core of agent_info · canonical_name — gathers name derivation in one place so it does not diverge from
    /// reaper/ingress/cli.
    ///
    /// ADR-0101 (WYSIWYA — unified canonical names): AgentInfo.name is aligned to "the name a person sees in the tree".
    ///   Previously profile.name (= the full cwd string passed to createClaudeProfile, often a path) was used as is,
    ///   so the address routing/roster expected and the tree display name (display_name ?? basename(cwd)) diverged.
    ///   Routing (resolve_recipient) · roster · envelope sender · frontend tree must use **the same string** for "pointing at the visible
    ///   name reaches that agent" to hold.
    ///
    /// ★cwd source = session.cwd (not profile.cwd)★: the frontend tree draws `display_name ?? basename(AgentInfo.cwd)`
    ///   and AgentInfo.cwd = session.cwd (canonicalized at spawn). profile.cwd is raw ("." · ".." · symlinks),
    ///   so deriving from it would split the basename and make tree display ≠ routing address. So it is derived from **the same value**
    ///   as AgentInfo.cwd (session.cwd).
    // ADR-0101
    fn resolve_canonical_name(&self, session: &Arc<AgentSession>) -> String {
        let cwd = session.cwd.to_string_lossy();
        // get() takes the profiles lock, clones and releases immediately — this function itself is called only without holding the sessions lock,
        //   so the two locks are not held at the same time (§10 lock order).
        let display_name = self.profiles.get(session.id).and_then(|p| p.display_name);
        crate::name::canonical_name_or_id_fallback(display_name.as_deref(), &cwd, session.id)
    }

    /// Called only without holding the sessions lock.
    fn agent_info(&self, session: &Arc<AgentSession>) -> AgentInfo {
        // get() takes the profiles lock, clones and releases immediately — this function is called only without holding the sessions lock,
        //   so the two locks are not held at the same time (§10 lock order).
        let display_name = self.profiles.get(session.id).and_then(|p| p.display_name);
        self.agent_info_with(session, display_name.as_deref())
    }

    /// The **display-name-injected form** of `agent_info` — keeps a caller already holding the profile snapshot when building entries for several sessions
    /// (`roster`) from taking the profile again per session. The name derivation rule itself exists only here
    /// (`canonical_name_or_id_fallback` — exactly the discipline of `resolve_canonical_name` of not duplicating rules).
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
    /// ★Why an explicit Stop is needed★: dropping reaper_tx alone closes the channel so recv ends with Err, but
    /// the finish hook closures the sessions hold carry reaper_tx clones, so it may not close immediately.
    /// A send failure (the reaper already ended) is ignored.
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
    // What is measured here is the one verb `session_id_sink` builds — whether that verb is actually carried on spawn is
    //   measured by the app-server end-to-end entry of `backend/codex/mod.rs`.

    /// ★The store being **memory** is intended★ — what these three entries assert is only the registry's decision, so
    /// persistence is irrelevant, and using file storage like `bare_manager` would leave one temp folder per call
    /// (9 piled up in one crate regression run — measured).
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

    /// ★Decision table — split by **reading** the backend declarations (not a literal copy)★.
    ///
    /// ★What this test does not measure★: whether this predicate is actually carried on `spawn_agent`. That is a place requiring a real process,
    ///   so this file does not measure it (the same scope as the sibling `session_id_sink` tests).
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

    /// ★The commit port is compare-and-swap — it does not rewind a field another writer changed before the first submission (ADR-0226)★.
    ///
    /// When the field holds a value (Y) the claude file watcher wrote without comparison, a commit by this incarnation's port with the value seen at start (S)
    /// must be refused. If it changed to an overwrite, Y would be rewound to S and Y pushed into the history.
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

    /// (f) ★The first echo of a resumed incarnation has nothing to write★ — if the id the handshake gave is the stored value as is, even when the link calls the first-turn
    ///   port and the commit runs, neither the field nor the history moves (the 「already the same value」 of compare-and-swap · ADR-0226 amendment).
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

    /// ★Our sid goes out only to issuing-axis backends (ADR-0185 · ADR-0226)★ — measured by **reading** the declaration table.
    ///
    /// If a value went out to a backend that issues its own id (codex), that value would enter the latch before `open_spawn`,
    /// the backend would drop it from argv, and the real id the other side gave would be dropped as the latch's second offer — the first submission would
    /// persist a fake value and every later resume would silently fail. It must not go out **even with** a stored value (if the decision splits by the resume
    /// axis, it is caught here).
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

    /// ★The defensive clause where the port moves `expected`★ — after a successful commit, a second call of the same port takes the first value as 「the value this incarnation saw」
    /// and replaces it. The latch calls only once per incarnation, so it is a branch not reached in production.
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

    /// ★Fresh **does not send the stored handle out to the backend**★ — this one field makes 「`resume_session_id` exists ⟺ a resuming incarnation」
    /// hold, and the session id reclamation of the codex terminal mode stands on that equivalence
    /// (that side decides fresh by looking only at the handle — ADR-0218 decision 5). If it leaks here, a resumed incarnation
    /// would try to reclaim its own id again, or a new conversation would skip the reclamation.
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

    /// ★A dead incarnation's late record cannot overwrite the live incarnation's value★ — this verb holds the marker of its own spawn,
    /// and that marker goes stale as the next spawn draws a new one.
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

    /// ★Measures the **cost** of dropping — not 「not written」 but 「diverged」★.
    ///
    /// The entry above starts from an empty profile, so 「dropped」 and 「was never there」 cannot be told apart. The shape that actually hurts is
    /// this one: when a stored handle **already exists** and the other side gives a non-uuid thread id, the link holds that string
    /// and sends every later turn with it, while the disk stays at the old value. The two axes silently diverge, and
    /// the next resume opens a place that is not the thread this incarnation actually talked in — with no indication on screen.
    ///
    /// ★The pair making that condition is
    ///   `tests::a_resume_target_makes_the_handshake_issue_thread_resume` of `backend/codex/mod.rs`★ — that test rig's fake
    ///   returns `resumed-<uuid>`, and that string is not a uuid (that side asserts the fact).
    /// ★This entry is not about flipping 「must drop」★ — panicking here or erasing the old value would be worse
    ///   (the reason is in the doc of [`session_id_sink`]). What it pins is the fact that **this outcome has a cost**,
    ///   and so it is kept so that when this branch goes quiet it is **a decision**, not a regression.
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
    // ★Measured not by the link **type** but by the caps the link reports★: the code that builds it moved inside `backend/<name>/`,
    //   so the assembly point has no type to call by name (ADR-0191). The fact measured is unchanged.
    // ★`probe_spec` is not a real CLI★: it launches a probe that ends immediately through that backend's link choice and looks only at which
    //   implementation was chosen (ADR-0012 isolation — no real claude binary needed).
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
    // ★Why the session is plugged into the map directly without a real spawn★: the only thing verified is "whether it compares the epoch of the session the map points at
    //   with the requested epoch and executes/refuses the write", so a real child, PTY and claude binary are all irrelevant (ADR-0012 isolation).
    //   It is an in-crate test, so it accesses the private `sessions` directly — `insert_test_session` (feature gate) is not needed.

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

    /// A stand-in for a link that **must set up** a connection — `link_state` can be swapped from outside.
    ///
    /// ★Why it is measured without a real codex★: what is measured is 「does the decision look at that axis」, and a real CLI is irrelevant to that answer
    ///   (ADR-0012 isolation). Measuring with the real thing would need a codex binary + a thread id to be refused, which cannot run in CI.
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

    /// Plugs one **live** (non-terminal) session into the roster and returns that incarnation's `OutputCore`.
    ///
    /// ★Why no real process is used★: what is measured here is only "how the decision stands when there is diagnostic text and the session is
    ///   alive", so a child, PTY and claude binary are all irrelevant (ADR-0012 isolation). `RecordingTransport`
    ///   launches nothing, so this session **never dies by itself** — that is the property needed to measure "it does not wait for death"
    ///   (with a real claude that state cannot be held deterministically).
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

    /// Plugs into the roster a live session carrying a link with a connection axis — returns the core and the kill counter.
    ///
    /// ★The connection state is not passed in★ — the outcome is now **delivered**, and the test rig holds that channel directly.
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

    /// ★A delivered `Ready` finishes the decision **on the spot**★ — it waits for no window (user decision).
    ///
    /// The point is giving a large backstop and measuring the elapsed time: if the decision starts waiting for time again, it is caught here.
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

    /// ★★Another incarnation's outcome is **dropped**★★ — this is the root of 「the cleanup of a collected incarnation kills the live successor」.
    ///
    /// Back when the marker was not attached to the value, the supervisor found it by id alone, and if the incarnation changed in between, the old outcome
    /// became the new session's decision. Here a mismatch is silently dropped, and the decision goes to the backstop **as if nothing
    /// happened**.
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

    /// ★A delivered value **does not lose to the time limit**★ — defect ② (the original defect with only the sign flipped) closes here.
    ///
    /// Even when called with the backstop already expired, if the channel has a value, that value is the decision. The old shape read the state and
    /// then looked at the time limit, so **a connection that just held was recorded as a failure and died**.
    /// ★What is measured here is the property 「with a value it never loses to the time limit」★ — the test rig cannot make the moment where the time limit and delivery exactly
    ///   overlap, so that property is checked with an expired time limit instead.
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

    /// ★A user kill comes to the terminal **without delivery** — so it is not recorded as a resume failure★.
    ///
    /// The premise is that the link does not deliver the kill branch (its `deliver_link` is the canonical source of that discipline), and here
    /// on that premise it looks at what the supervisor emits: the terminal branch, where the 「what the user turned off is not a failure」
    /// discipline already exists.
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

    /// ★★When a user kill **suppresses** delivery the channel closes, and that is not read as a failure★★.
    ///
    /// A defect this round nearly revived: `deliver_link` suppresses **the message** but cannot prevent **the channel disappearing**
    /// (when the writer thread holding the port ends, the sending end drops). At that time `session.kill()` is still
    /// inside `child.kill()`/`wait()`, so the pump has not run `finish`, and the state is `Exiting` — **not terminal.**
    /// The old shape that looked only at the state settled with that one read 「the link disappeared without delivering the outcome」,
    /// and so **the user's cancellation was recorded as a resume failure** — exactly the outcome the suppression device meant to prevent.
    /// ★The test rig makes that interleaving **deterministically**★: sets the intent latch, transitions to `Exiting`, and then
    ///   drops the sending end — exactly the states the real kill path passes through (the terminal has not stood yet).
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

    /// ★★A queued `Ready` **cannot beat** the user's cancellation★★ — the opposite-sign version of the sibling above.
    ///
    /// If the user interrupts right after the handshake succeeded and `Ready` was delivered, a shape receiving the delivery first **reports a dead
    /// agent as success** (`restore_one` as `Resumed`, `activate_profile` as `Ok(info)`).
    /// ★Still, it was not widened to 「the terminal beats delivery」★ — dying by itself was pinned by ADR-0201 as 「a separate event」,
    ///   and there the caller takes the state again and returns it instead of flipping the decision. The only thing that wins is
    ///   **an outcome a person decided**.
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

    /// ★★Even if the cancellation intercepts the delivery, **its reason already remains in the link's log**★★ — that is the ground for this place
    /// not carrying the reason, and so that log line is guarded here.
    ///
    /// ★Why this entry was made★: at one point the supervisor moved the intercepted reason into `evidence` and wrote 「if not carried here, it remains nowhere」,
    ///   but **both were false** — the two receivers did not read `evidence` in the `Killed` branch,
    ///   so it was just dropped (that branch does not record at all), and the link was already emitting the text. With the machinery removed,
    ///   **that log line, the ground of the removal,** is pinned instead.
    /// ★Two properties to keep★: ① on entering the failure branch, the reason is **logged** ② that line is **before**
    ///   `deliver_link` — if moved after, the reason vanishes entirely in the interleaving where the exit latch suppresses delivery
    ///   (what is suppressed is the delivery, not the log).
    /// ★Why it is measured in the source★: measuring log emission by running would need a `tracing` collector attached, and this crate has
    ///   no such harness, and above all what must be measured is **the order of two lines**.
    #[test]
    fn the_transport_logs_the_rejection_reason_before_it_may_be_suppressed() {
        // ★★Comments are stripped before scanning — otherwise **a comment line poses as a real call**★★: that branch is
        //   covered with comments quoting the very names they guard (this repository's comment convention), and the moment one
        //   of them mentions `deliver_link(` before the log line, this entry becomes **green while measuring
        //   nothing**. Through the same defect, two order guards on the link side were actually empty.
        //   ★That side's helper (`writer_loop_code`) cannot be used★ — it is inside the test module of another file, so it is not
        //   reachable here. So one copy of the same stripping is kept here (moving it first needs a place both know).
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

    /// ★★A Fresh success returns **the state after waiting** — not the spawn-time snapshot★★.
    ///
    /// `info` is assembled right after the child launches and its status is `Running`. Before, it was a 0ms-old value so returning it as is
    /// was harmless, but now that it waits for the connection it is stale by up to the backstop — if the caller were told an agent that died in between
    /// is alive (WS `Spawned` carries that status as is), **the next verb would write mail to a
    /// corpse.** The Resume sibling does the same in `activate_profile`.
    /// ★Why it is measured in the source★: driving this branch by running would need a real codex to make the handshake succeed.
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

    /// ★★The cleanup of a failed activation reaches **only that incarnation**★★ — defect ①.
    ///
    /// The cleanup arrives as late as the decision, and in between this incarnation may have been collected and the next incarnation launched. Without looking at the marker,
    /// what dies then is not the failed session but **the healthy successor that just launched**.
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
    // In one record, the control channel (permission revoke) · the core's status notification (`Exiting`) · the link (notice · termination) are written in the order called. The termination
    //   intent has no verb to write it, so the value at the moment of the notice is stamped separately.

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

    /// ★★An activation without an outcome yet is not answered as 「running」★★ — the place where two callers split.
    ///
    /// On a link declaring a connection, even after the session goes onto the roster, whether that incarnation can take input is not yet
    /// settled. If a second activation coming into that interval received `Ok` (running), the moment the first request soon decides a failure
    /// and collects that incarnation, **the two callers split holding different facts**.
    /// ★It makes that interval by holding the reservation directly★ — it does not wait for a thread race (what `LinkWatch` does in the real thing
    ///   is exactly this: it does not release the reservation until the outcome happens).
    /// ★The control group is the point★ — without a reservation, the same combination must still answer 「running」. Without that assertion
    ///   「always refuse」 would also pass, and that would break ADR-0082's reactivation guard.
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

    /// ★Measures here **the half the two entries above cannot**★ — whether that interval actually lasts **until the connection outcome**.
    ///
    /// The two above simulate that interval with the test rig holding the reservation directly, so even if production code released the reservation where spawn
    /// ends, **they stay green**. Then the guard remains but the interval it must guard disappears.
    /// ★Why it is measured in the source★: observing that interval by running would need a real codex app-server holding on to the handshake
    ///   — the test rig has no such counterpart.
    /// ★What this entry **cannot catch** is also written down★: if the side waiting for the outcome took only the mailbox out of the received value
    ///   and dropped the rest (e.g. `watch.map(|w| w.rx)`), the reservation would be silently released and this assertion would pass as is.
    ///   What prevents that shape is neither the compiler nor this entry, but only the fact that both callers hold `watch`
    ///   whole.
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

    /// ★★A Fresh spawn also claims success only after seeing the connection outcome★★ — and **both entries** do so.
    ///
    /// ★Why it is measured in the source★: driving this branch by running would need a real codex app-server to refuse `thread/start`,
    ///   and the test rig has no such counterpart (that is why the `link_activation_verdict` unit entries hold the channel directly).
    ///   What a regression produces is not a red assertion but **a silent ownerless state** — Running on screen but
    ///   every input refused and nobody collecting it.
    /// ★Measuring both entries is the point★ — fixing only one makes the other fall behind, and if the boot restore side fell behind,
    ///   that hole would stand at once for every codex profile without a resume handle.
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

        // ★The two entries pass **different entry values** to **the same verb** (ADR-0291 E3)★ — if per-entry launch copies appear,
        //   「the decision splits per entry and one falls behind」, which ADR-0082 · ADR-0201 · ADR-0202 blocked, comes back.
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

    /// ★Fresh's optimistic success does not clear 「the last failure」 either★ (ADR-0202) — the same discipline as the Resume-side sibling.
    ///
    /// The hole ADR-0202 wrote down as 「not yet true」 was exactly this place: Fresh read the moment the process launched
    /// as success and cleared the earlier failure record.
    #[test]
    fn a_fresh_success_does_not_clear_the_failure_record() {
        let ready = fresh_arm("EarlyVerdict::Ready =>", "EarlyVerdict::LinkFailed");
        assert!(
            !ready.contains("note_activation_result"),
            "the Fresh success branch touches the last failure — this decision observes only up to being ready to take input, so erasing old failure evidence with it is an irreversible loss: {ready}"
        );
    }

    /// ★The Fresh failure branch also does **cleanup first, record later**★ — the same order and the same reason as the Resume-side sibling.
    ///
    /// The record takes the profile mutex, and if that mutex is held, the side with the order reversed **never kills the
    /// child**. And the cleanup must be called holding the incarnation marker — otherwise it kills the live successor.
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

    /// ★An optimistic success **does not clear** 「the last failure」★ (user condition) — half of the original defect was that destruction.
    ///
    /// ★Why it is measured in the source★: the clearing is **one** call `note_activation_result(.., None)`, and measuring its absence
    ///   by running would need a real codex to make a resume succeed and then look at whether the old failure record remains —
    ///   the test rig does not have that combination. What a regression produces is not a red assertion but **a silent evidence loss**.
    /// ★The `Alive` branch must **keep** that call★ — that one is the way of links without a connection axis (claude) and
    ///   is outside this decision's scope. Measuring the two together catches even 「it did not overstep the scope」.
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

    /// ★The failure branch does **cleanup first, record later**★ — reversing the order disables the backstop.
    ///
    /// The record takes the profile mutex, and that mutex may be held by the session id record path while holding a disk write.
    /// If the record came first, that stall would become **「never kills the child」** — the backstop exists exactly for such a stall,
    /// and it would become unusable because of that very stall.
    /// ★And the cleanup must be called **holding the incarnation marker**★ — otherwise it kills the live successor (defect ①).
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

    /// ★Even if the terminal outruns delivery, **the reason survives**★ — this link's failure text is in neither tail.
    ///
    /// On the real refusal path the other side exits in 41–51ms, so the terminal observation may come before the delivery. Dropping
    /// the reason then would stamp the contextual default (early exit) as the last failure instead of the cause.
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

        // ★Whichever branch it falls to, **the reason must be alive**★ — which of delivery and terminal observation comes first is
        //   timing, and pinning it would make the entry itself flicker. What is measured is 「does the cause reach the classification」.
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

    /// ★Regression net of this feature's headline case (measured 2026-08-23)★ — if the diagnostics already said it failed, it is settled on the spot even while the process
    /// is still alive.
    ///
    /// The regression caught is exactly this: real claude, for `--resume <empty sid>`, **emits the diagnostic at +2.2s and
    /// dies only at +6.4s** (the SessionEnd hook runs in between). The old decision waiting only for death passed the 3s window and
    /// **decided the failure as success and even cleared the earlier record** — the headline case was entirely dead.
    ///
    /// ★The clock is not asserted (preventing the old flaky recurrence of this repository)★: "came back quickly" is not measured by elapsed time.
    ///   Instead **a very long window** is given and only the outcome is looked at — an implementation waiting for death would, since this session never dies by itself,
    ///   go to the end of the window and emit `Alive`, turning that assertion red. The pass condition does not depend on
    ///   timing.
    #[test]
    fn a_diagnosed_failure_settles_the_verdict_without_waiting_for_the_process_to_die() {
        let manager = bare_manager();
        let id = AgentId::new_v4();
        let core = put_live_session(&manager, id, 1);
        // The seeding is done outside the assertion (avoiding action-in-assertion).
        core.push_diagnostic("No conversation found with session ID: 8b1c");

        // ★The window is made much longer than the production value (3s)★: this session never dies by itself, so an implementation waiting only for death
        //   goes to the end of the window, emits `Alive`, and the assertion below turns red. A large value makes "passing by accidentally hitting
        //   the deadline" impossible (the cost is only that the red is slow on a regression).
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

    /// ★False-hit defense at the same place★ — if the diagnostics are silent, a live session passes as holding as is.
    ///
    /// Without this assertion, the test above would also pass with an implementation that "emits Diagnosed for any text". That implementation
    /// **stamps a successful resume as a failure** (a fail-open violation — ADR-0172 §Impact).
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

    /// ★The evidence of a dead session includes **the diagnostic stream** too★ — for a structured session the console ring is empty and only stderr fills.
    ///
    /// This branch being missing was defect (2): the classifier looked only at the console tail, and stream-json is the default output
    /// format, so **for most agents the classifier ran blind**.
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
    // ★What it guards★: the daemon turns this `Err` into `AgentEvent::SubscribeFailed` and sends it, advertising "neither Ack nor
    //   ReplayComplete follows this subscription", and the client relies on that advertisement to release its single-flight slot
    //   immediately. If a branch failing after `on_ready` appeared, the refusal and the Ack would go out together for the same subscription,
    //   and a late Ack/Complete would arrive over the already-released slot, attaching a success marker to a generation where the replay did not run.
    //   That regression is signal-less at runtime (output silently dies), so this assertion is the only detector.
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

    /// ★The incarnation facts of the subscription response come from the session that replayed★ — what `on_ready` (= the daemon's Ack) received and
    ///   what was returned (= the daemon's `ReplayComplete`) speak of the same incarnation, and that session also does the marker comparison.
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

    /// ★The name and hierarchy of a live entry come from **the same profile snapshot**★.
    ///
    /// Previously the name came from the per-session profile lookups inside `list_agents()` and the hierarchy from the list lookup after that — if
    /// a rename + hierarchy move were committed in between, **a combination that never existed** (old name + new parent) would be carried in one row.
    /// ★What this test proves and what it cannot★: that the two facts are read together from the profile registry is seen here.
    /// That there is **no** window between two reads is guaranteed by structure (there is only one lookup), and a deterministic reproduction test
    /// cannot be made — reproducing it would require putting a seam between two reads that no longer exist.
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

    /// A manager only for cap tests — profile storage is **memory**. The file storage of `bare_manager` writes the whole roster
    /// to disk per registration (save while holding the lock), so filling 512 would become O(n²) disk I/O. What is looked at here is
    /// the count decision, not persistence.
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

    /// Fills up to just below the cap. The only reason for using the seam that bypasses uniqueness allocation is speed (filling through the normal path
    /// would scan the roster per registration, O(n²)).
    fn fill_roster_to(manager: &AgentManager, count: usize) {
        for i in 0..count {
            let mut p = agent_profile(&format!("C:/filler/{i}"), Some(&format!("filler-{i}")));
            p.id = AgentId::new_v4();
            manager.profiles.upsert(p);
        }
        assert_eq!(manager.roster().len(), count, "fill premise");
    }

    /// ★The cap is not at the entry but at **the place where registration commits**★ — so whichever entry it comes through, the answer is the same.
    ///   The two paths that actually leaked when it was at the entry (desktop CreateProfile · ad-hoc spawn registration) are looked at here together.
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
        // ★Planted directly, bypassing uniqueness★: through the normal path (try_create_agent), ADR-0120 would rename it to "twin(1)",
        //   so this state could not be made. What is looked at here is not that upper-layer rule but **the axis of the difference**.
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
        // ★Limit of the observation method (stated honestly)★: the syscall itself cannot be counted — a nonexistent cwd is used to make
        //   a situation where "the result would have differed if canonicalize had run", and the result being unchanged is asserted.
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
        // ★Safety ground★: profile deletion runs the messaging deletion cleanup hook (connection_core DeleteProfile), so a reissued
        //   name does not inherit the parked mail or open contracts of the old owner.
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

        // ★★Stale snapshot re-registration — a real regression scenario★★: rename in the tree while the session is live → a later restart
        //   re-registers holding **the pre-rename snapshot**, and that old name was **taken by another agent** in the meantime.
        //   If the new-registration check ran then, the old name would be decided a collision and the restart would rename the agent to a wrong name.
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
        // A whitespace-only override is already ignored by the derivation functions (preventing empty labels), but that rescues only **the display**,
        //   and a useless string remains in the roster.
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

    /// ★Why the hook point is `capabilities()`★: `roster()` → `list_agents()` → `agent_info()` calls it per session.
    ///   That is, it is the only injection point where arbitrary code can be inserted **during the observation** of `try_rename_agent`,
    ///   so the window where "the profile disappears right before commit" can be reproduced deterministically without threads or timing.
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
        // ★The overlap is made **by design** with a Barrier★: without a barrier the threads flow sequentially, and contention is reproduced only when they happen to
        //   overlap (that chance rests on the harness's disk write latency, not a guarantee). Right after everyone passes the barrier
        //   they enter allocation at the same time, so without the gate several threads see the same observation.
        // ★Still a probabilistic detection (stated honestly)★: the barrier aligns only **the entry time**, and the interleaving after that is up to the scheduler.
        //   Removing the gate fails almost always but may pass by luck. A deterministic seal needs a yield-seam at the allocation
        //   point, and that is a separate decision.
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

    /// ★A request that came to an entry already launching — the situation is made **deterministically** by holding the reservation★.
    ///
    /// It does not wait for a thread race: holding the reservation directly makes the request coming after it lose for sure.
    /// Three things checked — that it is an outcome, not an error · that the caller can tell 「I did not launch it」 ·
    /// and that **neither record nor clear happens** (the value recorded earlier stays as is).
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

    /// ★A request that came to an entry already up — looked at with a real process★. Where it splits from the sibling above is that **there is a session
    /// to return**, so the caller gets that agent's information even when receiving moot.
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

    /// When activation holds, it clears on the spot (ADR-0172 decision 4 amended) — and even after that the axes are still
    /// separate: a record must be attachable again to a running entry for the screen's 「running wins」 rule to have a place to stand.
    ///
    /// ★Why it is an in-crate test★: the seeding needs `set_last_failure`, and that verb is `pub(crate)`
    ///   (making the compiler guard the single-write-point boundary outside the crate).
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

        // Axis independence — had they been merged into the state enum, this combination could not be expressed at all.
        // The seeding is done outside the assertion (avoiding action-in-assertion).
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

    /// ★Whether the output tail of a dead session is actually readable — only a real process can say (ADR-0172)★.
    ///
    /// The regression this test catches is one: if the session is looked up again **inside** the polling loop, the reaper removes it from the roster right after
    /// `finish` sets the state (our polling interval is 100ms), so the lookup almost always fails and the tail is empty.
    /// Then the classification always falls to the contextual default and the 「no conversation to resume」 decision becomes **dead code** —
    /// a pure dispatch test cannot see that fact.
    #[cfg(windows)]
    #[test]
    fn early_verdict_captures_the_dead_sessions_output_tail() {
        use crate::failure::AgentFailureKind;

        // ★Why a batch file★: portable-pty CommandBuilder quotes `&`·`>` individually and breaks them while passing through ConPTY
        //   (measured in the activation.rs fixture). A batch is parsed directly by cmd, so it is deterministic.
        // ★It lives briefly before dying (ping)★: the subject of this test is "does it read the ring **even after** the reaper removed it from the roster".
        //   Dying immediately could make the first `get_session` race the reaper and lose by failing to catch the session,
        //   and then it turns red for a reason unrelated to the subject (false red). Keeping it alive briefly to make sure the Arc is acquired and then killing it —
        //   the verification subject, reading after collection, is unchanged.
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

        // ★It is a shell profile, so the diagnostic buffer is empty (the PTY merges stderr into the console)★ — that is, the evidence
        //   standing here is solely **the console tail**, and that is what this test measures.
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

    /// ★Pins **the order of three verbs** inside `spawn_agent` — this order yields the race-freedom of the resume value★.
    ///
    /// Two things pinned:
    ///   1. **Right after** `register_for_spawn` is `epoch_for_spawn`. The former **keeps** the live marker
    ///      (ADR-0084), so in the interval between them **the previous incarnation's marker** still stands, and a late record of the previous incarnation
    ///      arriving in that interval has a matching marker and **is not refused**. If cwd normalization (a syscall) or the Fresh clearing
    ///      (writing the whole `agents.json`) slipped in between, that interval would actually open.
    ///   2. The place reading the resume handle is **after** `epoch_for_spawn`. From when the marker changes until this spawn
    ///      hands over the record port, no incarnation's record passes, so the roster value is frozen, and so
    ///      「the value read = the value this incarnation resumes」 holds.
    ///
    /// ★And that read must be **from the roster**★ — reading the caller snapshot (`profile.backend_session_id`) would entirely miss the
    ///   thread id the other side gave in between (that field is written by the other side, not us).
    ///
    /// ★Why it is measured in the source★: measuring this order by running would need a real codex binary and a window to slip a late record into
    ///   at the same time. The test rig has neither, and what goes wrong when they diverge is neither a compile error nor a red assertion but
    ///   **a low-probability silent miswrite**. Precedent · same reason =
    ///   `backend::codex::transport::tests::the_session_id_is_recorded_before_the_gate_opens`.
    #[test]
    fn the_resume_handle_is_read_from_the_roster_after_the_incarnation_tag_is_stamped() {
        let src = include_str!("manager.rs");
        let production = src.split("mod tests {").next().expect("production section");

        // ★Before cutting out the body, first check **that what is cut out is right**★ (review finding): the two boundary names are free
        //   text, so if another function moved above `spawn_agent`, the cut lump would grow wrongly and the order assertions below
        //   would turn green over unrelated code. These two lines block that silent pass.
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

        // ★Looked at not as byte offsets but as **the list of executed lines**★ — rustfmt splits chains into lines,
        //   so a single-lump string search breaks with one formatting (it actually broke).
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
        // ★The anchor is **the line reading from the roster**★ — after it there is one more line where `resume_handle_for` clears Fresh
        //   (that function's doc is the canonical reason), and what the order assertion below measures is the place of 「the read」.
        let resume_read = only("let stored_handle = match mode {");
        let handle_gated = only("let resume_session_id = resume_handle_for(");
        let spec_built = only("let spec = backend::build_command_spec(");

        // ★The read must be **before** the spec assembly (ADR-0208)★ — a backend resuming by command line (the codex terminal's
        //   `resume <id>`) receives that value at argv assembly time. Pushed after, there would be no value to pass there, so
        //   the resume would **silently become a new conversation**, and only the sibling resuming through the link (app-server) would be fine, so the symptom
        //   would appear in only one mode.
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

        // ★The door clearing Fresh must be **between** the read and the spec assembly★ — if that door disappeared, a Fresh spawn would
        //   carry the stored handle out as is, and the consumer looking only at that one value (the session id reclamation of the codex terminal —
        //   ADR-0218 decision 5) would misjudge a new conversation as a 「resuming incarnation」 and skip the reclamation entirely.
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
        // ★Absence is not swallowed★ — if the profile was deleted and `None` flowed on, the resume would silently become a new conversation
        //   (the comment on that line above is the canonical reason).
        assert!(
            binding.iter().any(|l| l.contains("profile_vanished_mid_spawn")),
            "it does not cut on profile absence — swallowed with `and_then`, the resume silently becomes a new conversation: {binding:?}"
        );
    }

    /// Gives the function body between `start` and `end` in the production section as a string **with comment lines removed and all whitespace erased**
    /// — however rustfmt splits the lines, the same call is caught as the same string. Trailing commas (`,}`·`,)`) are folded.
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

    /// ★The queued input registry is carried on the spawn path (ADR-0231)★ — the wiring is pinned in the source.
    ///
    /// What is pinned: the core is plugged with this incarnation's new registry + the manager's waiting-list table, and this incarnation is registered in that table
    ///   **before** the sessions map insert. Omitting either still compiles with no error — the only symptom is that incarnation's user list being invisible to mail,
    ///   so mail reaches stdin before user text even while the list is filled.
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

    /// ★The first-submission latch is actually carried on the spawn path (ADR-0226)★ — the wiring is pinned in the source.
    ///
    /// What is pinned:
    ///   ① the record port slot of `open_spawn` is the latch's `offer_sink`, and there is no old port writing straight to the roster.
    ///   ② the `offer` of the issued value is **before** `open_spawn`, and that value went through the issuing-axis decision
    ///      (`session_id_to_hand_over`) — the session takes input from the moment of the roster registration, so if late, the first
    ///      submission goes out without knowing the id and the commit comes after the turn.
    ///   ③ the same latch goes to `spawn_session`, and that function destructures `SpawnIncarnation` **by name** at its head
    ///      and calls **both** `.with_incarnation(` and `.with_session_id_latch(`. The production section has no old verb issuing and persisting
    ///      at spawn.
    /// ★Do not remove the `.with_session_id_latch(` assertion of ③★ — the required argument drags the latch only up to the function and cannot force
    ///   carrying it into the session (dropping it with `{ latch: _, .. }` compiles, and destructuring by name and not using it is
    ///   only a warning). If that builder were missing, no submission would ever be counted and **every resume would silently disappear.**
    ///   The real-process pair = in `tests/activation.rs`
    ///   `d1_a_fresh_claude_persists_its_minted_id_only_after_the_first_submission`.
    #[test]
    fn the_first_submission_latch_is_wired_through_the_spawn_path() {
        let spawning =
            squashed_production_body("fn spawn_agent_watching_link(", "pub fn activate_profile(");
        assert!(
            spawning.contains("Some(latch.offer_sink())"),
            "the record port of `open_spawn` is not the latch's receiving port — the fetched id bypasses the latch"
        );
        // ★Submissions of the mode where the link owns the turn (codex JSON) come only through this port★ — if missing, no incarnation of that mode can persist
        //   its id (the session does not count in that mode — ADR-0226 amendment).
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

    /// Confirms with a real process that the real spawn path really takes `register_for_spawn` (= that the unit test above is not
    /// looking at dead code). Two cmd.exe are launched briefly and killed.
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

    /// ★The latch's record stands in memory and sets dirty on any save `Err`★ — dropping it would lose this incarnation's session id
    ///   and the next activation would lose the conversation (ADR-0226).
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

    /// ★During a refusal the disk bytes stay the same★ — seen with a real file store: if `agents.json` becomes a newer edition while running,
    ///   the latch's record stands only in memory (dirty) and that file does not change by a single byte.
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

    /// ★Same verb, different entry — boot restore launches and manual wake-up refuses (E3)★.
    ///
    /// Under the same save failure, boot restore applies the pre-launch roster change to memory + dirty and launches (the post-launch
    ///   `auto_restore` raise · the reaper's lowering are also internal changes and stand in memory). Manual activation has that same change's
    ///   save as `Err`, so it does not launch and comes back with `PtyError::Store`, and both the in-memory roster and the pending dirty are unchanged.
    /// ★Why it is looked at with a real process★: `Started` and the post-launch raise are passed only by a real spawn.
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

    /// A profile that stays the claude backend so the session id clearing and resume branches run — but opening the process fails: the working
    /// folder and the child PATH are the same empty folder. Even if it regressed past the save `Err`, the real claude would not launch, and that outcome is not a save
    /// error, so the first assertion catches it. The caller deletes that folder (`cwd`) at the end.
    /// ★Blocked by PATH, not by the program name★ — the backend pins it to `cmd.exe /c claude`. If the child PATH is an empty
    ///   folder, CreateProcessW cannot even find `cmd.exe` and fails (measured — os error 2, 0 processes launched). Only the terminal (PTY)
    ///   link is like that — switching the output format to JSON makes std `Command` search System32 and `cmd.exe` launches (claude is
    ///   still not found).
    /// ★The folder must exist★ — a nonexistent working folder is replaced by portable-pty with the home folder and launched (measured: with only a nonexistent folder the real
    ///   claude launched).
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

    /// ★A resume activation also returns the save `Err` in its original form (the caller entry)★ — if it were folded into the restore report text and became
    ///   `PtyError::SpawnFailed`, the bus could not tell a save refusal from other failures. Both a resume with a handle and a resume request without a handle
    ///   (the new-conversation branch) are looked at. The launch stops at registration (①), so no process is launched.
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

    /// ★The save `Err` of a new-conversation activation is also recorded as 「other」★ — it never tried to open a process, so it is not `SpawnFailed`
    ///   (executable · permission). The launch stops at registration (①), so no process is launched.
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

    /// ★Partial success of the caller entry is not reverted (D8)★ — if the save of the session id clearing (③) is `Err` after the registration (①) was saved,
    ///   the registration remains, the field keeps the old handle, and it does not launch. The launch stops at ③, so no process is
    ///   launched.
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
