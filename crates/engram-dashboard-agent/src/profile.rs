//! Agent profiles — the single source of truth for restart and session restore.
//!
//! Transport- and claude-neutral: claude-only argument assembly (`--session-id` / `--resume`) lives in `backend/claude/`; here only
//!   neutral data about
//! "what to run and which session to resume". Zero tauri imports.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use engram_dashboard_base::time::now_epoch_ms;
use engram_dashboard_platform::fs::{path_key, PathKey};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::failure::AgentFailureKind;
use crate::profile_edit::{Attachments, PresetPathRename};
use crate::types::AgentId;

// ── Neutral run command ─────────────────────────────────────────────────────────────

/// Agent output mode — the profile axis for per-backend transport and codec choice (ADR-0044). `Terminal` = PTY interactive; `StreamJson`
///   = JSON-based
/// non-terminal mode, concrete exchange shape decided by the backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum AgentOutputFormat {
    #[default]
    Terminal,
    StreamJson,
}

/// Here only the branch tag and the user's extra arguments are kept.
///
/// ★Name collision★ — `protocol::AgentCommand` is a different thing (the wire command sent to the daemon). Wire mirror =
///   `protocol::AgentSpawnCommand`;
/// frontend mirror = the same-named type in `src/api/types.ts`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum AgentCommand {
    /// claude CLI. `extra_args` are the user's extra arguments excluding session arguments (`--session-id` etc.).
    /// `output_format` is `#[serde(default)]`, so old profiles and existing callers are absorbed as Terminal with behavior unchanged.
    Claude {
        extra_args: Vec<String>,
        #[serde(default)]
        output_format: AgentOutputFormat,
    },
    Shell {
        program: String,
        args: Vec<String>,
    },
    /// codex CLI. `extra_args` = the user's extra arguments excluding those the backend assembles (`--cd`·`-s`·`-a`). Stored JSON without
    ///   `output_format`
    /// deserializes as `Terminal`.
    ///
    /// ★Do not grow the fields carelessly★: this enum is `#[serde(tag = "kind")]`, so its shape sits in the profile file as is — a contract
    ///   baked onto disk, so the less
    /// frozen the cheaper. Sandbox / approval values are still at `--help` grade (TRD §2); typing them now would freeze them at that grade
    ///   (ADR-0194).
    Codex {
        extra_args: Vec<String>,
        #[serde(default)]
        output_format: AgentOutputFormat,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpawnMode {
    /// Start a new session (for claude, `--session-id <new uuid>`).
    Fresh,
    /// Resume an existing session (for claude, `--resume <backend_session_id>`).
    Resume,
}

/// Auto-restart policy. **Reserved — not a dead field.** Behavior is unimplemented (gated), but it is the seam for what ADR-0016 calls
///   "boot restore, guard counter
/// and Failed persistence remain valid (to be revisited later)".
/// ※Do not remove: it spans agent→protocol wire (domain.rs)→ts-rs bindings→daemon conversion→frontend, so removal bumps PROTOCOL_VERSION.
/// Only the "runtime auto-restart" interpretation is discarded (ADR-0019).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum RestartPolicy {
    Never,
    OnCrash,
    #[default]
    Always,
}

// ── Restore result ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct RestoreReport {
    pub agent_id: AgentId,
    pub epoch: u32,
    pub outcome: RestoreOutcome,
}

/// Shared with the frontend, so serialized internally-tagged (discriminated union).
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type")]
pub enum RestoreOutcome {
    Resumed,
    /// Started a new session rather than a resume target (shell, or claude without a sid). Not a resume (fable Mn-2).
    Started,
    /// resume failed → fallback to a new session. States which sid was discarded and which was newly created.
    /// (no silent stale — always make visible what changed)
    FreshFallback {
        old_sid: Option<Uuid>,
        new_sid: Uuid,
        reason: String,
    },
    /// Skipped because it is not a restore target, e.g. `auto_restore=false`.
    Blocked {
        reason: String,
    },
    /// Even fresh failed → stop. A terminal point without recursive retry (H-1.7).
    Failed {
        reason: String,
    },
}

// ── Persistent profiles ────────────────────────────────────────────────────────────────

/// One preset attached to an agent — a library reference (a `/` path from the root, or a built-in `engram/…`).
/// The 「Default」 preset is not in here — the [`AgentProfile::default_preset`] field carries it separately.
// ADR-0315
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttachedPreset {
    pub path: String,
}

/// The persistent profile of one agent — the unit stored in the profile file (`persistence::PROFILES_FILE`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentProfile {
    /// Immutable key. This id stays for life even as processes and sessions change (the frontend subscription key).
    pub id: AgentId,
    pub name: String,

    /// User-set display name override (ADR-0061 enrichment — tree rename), **separate from `name`** (the CreateProfile name or an ad-hoc
    ///   spawn's cwd string, not a clean display name). `Some` → shown as is, `None` → derived from the cwd basename.
    /// ★Filled with the name of the moment when empty at creation★ (the manager's two new-registration spots) — a name left derived from
    ///   the working folder would let a working-folder change silently change the mail address · tree name. In production `None` comes
    ///   only from clearing the name (`try_rename(id, None)`).
    // ADR-0321
    // TRD S25 §1-5 「display_name pinned」
    #[serde(default)]
    pub display_name: Option<String>,

    /// Parent profile id in the tree. `Some(pid)` → child of pid, `None` → root. **One level only**: a child cannot become a parent, and
    ///   the parent must be a root (validation: `ProfileRegistry::try_reparent`).
    // ADR-0072
    #[serde(default)]
    pub parent_id: Option<AgentId>,

    pub command: AgentCommand,

    /// ★The stored value is **raw**, not normalized (load-bearing)★: `"."`·`".."`, symlinks and differently-cased Windows paths sit as is;
    ///   each user of the value
    /// normalizes (spawn's `session.cwd`, `canonical_name_when_live`).
    pub cwd: PathBuf,

    /// ※No credentials. On persist, `*_KEY`/`*_TOKEN` patterns are warned about (persistence).
    pub env: Vec<(String, String)>,

    /// Whether the 「Default」 preset is attached — `true` at creation. An entry without the key reads as `true` too (the same value as just created).
    #[serde(default = "default_preset_attached")]
    pub default_preset: bool,

    /// The attached presets — in attach order (the launch expands them in this order). Empty at creation.
    #[serde(default)]
    pub presets: Vec<AttachedPreset>,

    /// The hash of the instructions the last launch carried to the peer — codex JSON resume checks it to tell whether the instructions
    /// changed (carried on the first turn only when different). `None` = never carried.
    /// ★Author = a launch★ — written as a roster change with the incarnation-marker guard when the first turn carrying those instructions
    ///   reached the peer (not on the session-id compare-and-swap — a resume's compare-and-swap writes the same value, so it does not
    ///   write). Written at spawn, a resume after an incarnation whose first turn never went out would skip the changed instructions. Clone does not copy it.
    // ADR-0337
    // TRD S25 §1-5 profile table · T6 · M16
    #[serde(default)]
    pub last_applied_instructions_hash: Option<String>,

    /// The omission identity ([`crate::launch::notice_fingerprint`]) of the launch notices last delivered to a caller.
    /// `None` = nothing recorded · `Some("")` = an empty set was delivered (X → none → X shows X again).
    /// ★Author = the launch alone★ — written by `ProfileRegistry::record_notice_fingerprint` once a caller that delivers
    ///   notices got a success reply from a launch that started a process; never on boot restore, a moot activation or
    ///   any failure. `merge_preserving_live` keeps the live value. A duplicated agent starts without it (`None`) —
    ///   `agent.duplicate` builds its copy from `AgentProfile::new` and does not carry it. The creation paths store the
    ///   caller copy's value as is, so this holds only while every builder starts from `AgentProfile::new`.
    // ADR-0341
    // TRD S25 §1-5 「Notices once per agent」
    #[serde(default)]
    pub notice_fingerprint: Option<String>,

    /// The current backend session id. **Mutable** — written right before the incarnation's first user turn goes out (ADR-0226), and
    ///   updated by the session_tracker
    /// watcher on `/clear` etc. `None` = no conversation to resume.
    ///
    /// ★The neutral name is intended★ — it does not say who draws the id: claude receives the id we chose via `--session-id`, codex tells
    ///   us the id it drew.
    /// Either way it is 「an id in that backend's session namespace」. No backward-compat shim for the old `claude_session_id` name
    ///   (`docs/tracking.md` T-33 · S21 TRD §6-1).
    pub backend_session_id: Option<Uuid>,

    /// History of past session ids discarded by fallback or clear (for audit and debugging).
    pub old_session_ids: Vec<Uuid>,

    /// ★**An opaque marker** of one incarnation★ — order has no meaning; the only question is "is the stream being read the same as that
    ///   earlier one". Compare by **match/mismatch only**, never derive "newer"
    /// (random per incarnation: `ProfileRegistry::epoch_for_spawn`).
    ///
    /// ★Skipping the read is part of the meaning★: an incarnation cannot outlive the daemon, and a revived marker would make an attached
    ///   client keep its old cursor so the new frames (numbered from 0) are dropped by dedup
    ///   (empty slot). Files without the value, or carrying a non-zero one, are read as is (value ignored).
    /// ★The write always carries the key as `0` — a harmless leftover placeholder★: the only file an old build that read this key as
    ///   required reads is the old version's `agents.json`, which this build never writes (ADR-0314 decision 5) — no reader reads this key
    ///   from the profile file. The value is a fixed `0`, so the live marker never reaches disk. Removing the key is a separate matter.
    /// Not in the frontend subscription deps — the reattach trigger is the authoritative roster (ADR-0164 decision 8).
    // ADR-0007, ADR-0163, ADR-0164
    #[serde(skip_deserializing, serialize_with = "serialize_zero_placeholder")]
    pub epoch: u32,

    /// ★Kind of the last activation failure of this entry★ — `None` = no record. **A fact observed where it was attempted** (ADR-0172
    ///   decision 1), separate from the
    /// current state (`AgentStatus`).
    ///
    /// ★`#[serde(skip)]` is part of the meaning★: sessions die with the daemon, so old failures are not held on disk — this pins the
    ///   lifetime to **the daemon's lifetime**
    ///   (ADR-0171 layer 3) and exempts file-format compatibility. The type has no serde derive, so deleting this skip fails to compile
    ///   (`failure::AgentFailureKind`).
    /// ★One writing point★: `manager::AgentManager::note_activation_result` (success = clear / failure = record) — its comment is the
    ///   canonical 「why」. More callers
    ///   would split the causality (cf. ADR-0127).
    /// ★A snapshot cannot revert it★: `upsert_preserving_hierarchy` keeps the live value.
    // ADR-0172
    #[serde(skip)]
    pub last_failure: Option<AgentFailureKind>,

    pub auto_restore: bool,

    /// Auto-restart policy. **Reserved** — behavior unimplemented (gated), do not remove (see the RestartPolicy comment).
    #[serde(default)]
    pub restart_policy: RestartPolicy,

    /// Crash guard counter. **Reserved** — behavior unimplemented, ADR-0016 "to be revisited later" valid.
    #[serde(default)]
    pub restart_count: u32,

    /// Failed (auto-restore suspend) reason — persisted across cold boots, excluded from auto-restore until a manual wake-up (ADR-0016).
    /// **Reserved** — behavior unimplemented but valid in ADR-0016, do not remove (comes with wire/bindings + version bump).
    #[serde(default)]
    pub failed_reason: Option<String>,

    pub created_at: i64,
    pub last_active: i64,

    /// Last process start time (for records and debugging, not used for reset decisions). epoch millis. None if absent.
    #[serde(default)]
    pub last_start_at: Option<i64>,
}

impl AgentProfile {
    /// The display-name override that names the agent — `None` when unset or blank (a blank one names nothing, as in
    /// [`Self::canonical_name_when_live`]).
    pub(crate) fn name_override(&self) -> Option<&str> {
        self.display_name
            .as_deref()
            .filter(|name| !name.trim().is_empty())
    }

    /// The session id is created by ProfileRegistry at the first spawn (not made here).
    pub fn new(
        name: String,
        command: AgentCommand,
        cwd: PathBuf,
        env: Vec<(String, String)>,
        auto_restore: bool,
    ) -> Self {
        let now = now_epoch_ms();
        Self {
            id: Uuid::new_v4(),
            name,
            display_name: None,
            parent_id: None,
            command,
            cwd,
            env,
            default_preset: true,
            presets: Vec::new(),
            last_applied_instructions_hash: None,
            notice_fingerprint: None,
            backend_session_id: None,
            old_session_ids: Vec::new(),
            epoch: 0,
            last_failure: None,
            auto_restore,
            restart_policy: RestartPolicy::Always,
            restart_count: 0,
            failed_reason: None,
            created_at: now,
            last_active: now,
            last_start_at: None,
        }
    }

    /// ★The **canonical name this profile will have when it comes up again** (load-bearing)★ — must be **literally the same** as the live
    ///   canonical name of the restored session, or asleep parking keyed by it
    /// cannot find its owner.
    ///
    /// ★Same rule as a live session = `canonical_name_or_id_fallback` (+ the canonicalized cwd)★ — what `manager::resolve_canonical_name`
    ///   uses. `name::resolve_display_name` is **not** it (it takes an empty/whitespace
    /// override as the name and does not degrade to the id on a placeholder basename).
    /// The cwd must have the live session's spelling too: the spawn's session.cwd is the launch's working folder
    ///   (`LaunchKitSource::working_folder` — the `dunce` canonical path, or the cwd as given when that has no answer), while the
    ///   profile's `cwd` may be raw (`.` · `..` · a link · a Windows path in another case). So this applies the same canonicalize and
    ///   degrades to the raw cwd when it fails. Known gap: with the library plugged, the working folder comes from a bounded canonicalize
    ///   (the advisory allowance — ADR-0360); when the folder answers too slowly the spawn runs in the raw cwd, so the live name (from that
    ///   raw spelling) and this name (from the canonical path) differ whenever their basenames differ (case, a link name, `.`). Not closed
    ///   here — closing it needs the spawn's folder stored.
    /// ★fs access only when there is no override (not pure)★ — hence on the profile side, not in `name.rs`.
    /// ★The override shortcut is not an optimization — do not delete it★: with a non-blank `display_name` the name is independent of the
    ///   file system. Deleting it would ① degrade the name of a profile whose cwd
    ///   vanished and diverge from the asleep-time name, ② block one canonicalize for tens of seconds on a dead SMB share.
    /// ★Known and accepted gap★: a profile **without an override whose cwd vanished** still degrades to raw and can diverge; fixing it
    ///   means redesigning name-keyed parking.
    // ADR-0101 (WYSIWYA) / ADR-0116 (decision 1 — the asleep parking key)
    pub fn canonical_name_when_live(&self) -> String {
        // **Literally the same** as the first branch of the live session rule (`canonical_name_or_id_fallback`) (if non-blank, no trim
        //   — the original as is is the name).
        if let Some(n) = self.name_override() {
            return n.to_string();
        }
        // ★Syscalls that cannot change the answer are not made★: if the cwd is empty/whitespace-only, the basename is the placeholder, so
        //   the result is the first 8 characters of the id regardless of canonicalize success/failure (`canonical_name_or_id_fallback` degrade).
        let raw = self.cwd.to_string_lossy();
        if raw.trim().is_empty() {
            return crate::name::canonical_name_or_id_fallback(None, &raw, self.id);
        }
        let cwd = dunce::canonicalize(&self.cwd).unwrap_or_else(|_| self.cwd.clone());
        crate::name::canonical_name_or_id_fallback(None, &cwd.to_string_lossy(), self.id)
    }
}

// ── Persistence abstraction ──────────────────────────────────────────────────────────────

/// The reason the store refused a save by decision.
// ADR-0291 R2 · R8
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// A file written by a newer edition (`found`) than this build — it is neither read, overwritten nor copied aside.
    Newer { found: u64 },
    /// The file could not be read (after the lock budget · permissions · other IO) — not knowing what it holds, it is not overwritten.
    Unreadable,
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Refusal::Newer { found } => write!(
                f,
                "a file written by a newer edition ({found}) than this build"
            ),
            Refusal::Unreadable => f.write_str("the file could not be read"),
        }
    }
}

/// Failure of [`ProfileStore::save`].
///
/// ★A refusal has two variants by origin★ — the caller's handling differs: a load refusal clears only on daemon restart; a re-decision
///   refusal is decided again by the
/// next save (unreadable is usually a briefly held lock, so a later retry works).
// ADR-0291 R17
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// A store the load decision refused — it does not save for this whole run (the [`StoreStatus::ReadOnly`] of the store's `status()`).
    #[error("save refused (since load, for this whole run) — {0}")]
    ReadOnly(Refusal),
    /// The re-decision right before this save refused — the next save decides again (of the store's `status()`
    /// [`StoreStatus::Refusing`]).
    #[error("save refused (re-decision right before writing) — {0}")]
    Refused(Refusal),
    #[error("save failed — {0}")]
    Io(std::io::Error),
}

/// The current state of the store.
// ADR-0291 R17 (D6)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StoreStatus {
    #[default]
    Writable,
    ReadOnly(Refusal),
    /// Loaded, but the re-decision of the last save refused — the next save decides again and clears it on success.
    Refusing(Refusal),
}

pub trait ProfileStore: Send + Sync + 'static {
    /// Saves the whole snapshot atomically. The implementation logs the failure — the caller only branches the flow on `Err`.
    fn save(&self, profiles: &[AgentProfile]) -> Result<(), StoreError>;
    /// Loaded once at boot. Empty list if missing, damaged or refused.
    fn load(&self) -> Vec<AgentProfile>;
    /// Default = always usable (a store that never refuses).
    fn status(&self) -> StoreStatus {
        StoreStatus::Writable
    }
}

// ── Hierarchy normalization (ADR-0072) ────────────────────────────────────────────────────────

/// Forcibly restores the whole map to the "valid one-level forest" invariant — called under the lock **right before saving and committing
///   a change**
/// (`mutate` · `mutate_if` · `try_mutate` · `try_mutate_if`).
///
/// ★Why at the boundary, not per path★: reparent validates, but upsert/update_with accept arbitrary parent_ids (cycle · dangling) and
///   spawn reinserts a stale snapshot,
/// reviving a parent deleted in between; validating only in reparent would let these paths persist cycles and dangling references.
///
/// **Idempotent**: an already-valid forest trips no rule; running it twice gives the same result.
fn normalize_hierarchy(map: &mut HashMap<AgentId, AgentProfile>) {
    // The decision is based on the pre-change snapshot (so a clear within the same pass does not pollute the decision of other nodes).
    // Mixing has_own_parent (= a child) and is_a_parent (= a node that has children) would misdetect normal children.
    let existing: std::collections::HashSet<AgentId> = map.keys().copied().collect();
    let has_own_parent: std::collections::HashSet<AgentId> = map
        .values()
        .filter(|p| p.parent_id.is_some())
        .map(|p| p.id)
        .collect();
    let is_a_parent: std::collections::HashSet<AgentId> =
        map.values().filter_map(|p| p.parent_id).collect();

    let mut clear: Vec<AgentId> = Vec::new();
    for p in map.values() {
        if let Some(pid) = p.parent_id {
            let dangling = !existing.contains(&pid);
            let self_parent = pid == p.id;
            let two_level = has_own_parent.contains(&pid);
            let node_is_parent = is_a_parent.contains(&p.id);
            if dangling || self_parent || two_level || node_is_parent {
                clear.push(p.id);
            }
        }
    }
    for id in clear {
        if let Some(p) = map.get_mut(&id) {
            p.parent_id = None;
        }
    }
}

// ── ProfileRegistry ────────────────────────────────────────────────────────────

/// The in-memory **single owner** of profiles. All CRUD and session id updates go through here and persist to the store immediately;
///   creating and updating session ids lives here too (H-1.4).
///
/// **Two change entries — split by who receives the save's `Err`:**
/// - `try_` verbs = **changes with a caller** (bus · WS · launches): applied to a copy → hierarchy normalization → save → committed only
///   on success. On `Err` memory is left as is and the error returned,
///   so a change believed 「changed」 does not vanish on restart. Do not ask the save first and then apply — a write failure in between
///   would recreate 「only memory changed」.
/// - Other verbs = **changes without a caller** (first-submission latch · file watcher · reaper · `auto_restore` after launch · boot
///   restore): they commit on any save `Err` and set dirty — nobody can
///   revert, and dropping them would lose the reaper's `auto_restore=false` (reviving dead agents at boot) and the latch's session id. A
///   dirty registry is not written over a newer-edition or unreadable
///   file because every save re-decides (the store).
///
/// Invariant = in-memory registry = the last successful save + (if dirty) the changes committed after it.
///
/// Lock discipline: `store.save` runs **while holding** the profiles lock. ★Do not move save outside the lock (§5 concurrency consistency
///   > lock-hold time)★: snapshot → unlock → save lets two
/// overlapping mutations leave memory/broadcast at B while the disk keeps A (persisted ≠ observed). No deadlock: `store.save` takes only
///   the store-state leaf lock and never re-enters the registry
/// (order `profiles → store-state`, separate from the session lock domain); in-lock IO is one small read + write, ~0.4 s worst case while
///   the file is locked (ADR-0291 · D4).
/// ★Reopening 「reduce lock-hold anyway」 was closed by ADR-0207★ (accepting place ADR-0071:27; reopen conditions there).
// ADR-0291 R17 (D2 · E1)
pub struct ProfileRegistry {
    profiles: Mutex<Profiles>,
    store: Arc<dyn ProfileStore>,
    /// How many takes of the registry lock found it held — tests observe a waiter.
    #[cfg(test)]
    lock_blocked: std::sync::atomic::AtomicUsize,
}

type ProfileMap = HashMap<AgentId, AgentProfile>;

/// The registry and its save state — kept under one lock (every place that sets and clears them is inside a change path holding that lock).
struct Profiles {
    map: ProfileMap,
    /// The save of a committed change failed — memory may be ahead of the disk. The next successful save (any entry; a save writes the
    ///   whole map) carries it and clears this.
    /// The `Err` of a change with a caller does not clear it (that memory is left as is). ★What remains at exit is lost — no last save on
    ///   the exit path★.
    // ADR-0291 (D2 · E1)
    dirty: bool,
}

impl ProfileRegistry {
    pub fn new(store: Arc<dyn ProfileStore>) -> Self {
        let loaded = store.load();
        let map = loaded.into_iter().map(|p| (p.id, p)).collect();
        Self {
            profiles: Mutex::new(Profiles { map, dirty: false }),
            store,
            #[cfg(test)]
            lock_blocked: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Profiles> {
        #[cfg(test)]
        if matches!(
            self.profiles.try_lock(),
            Err(std::sync::TryLockError::WouldBlock)
        ) {
            self.lock_blocked
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
        self.profiles.lock().expect("profiles poisoned")
    }

    /// How many takes of the registry lock found it held so far.
    #[cfg(test)]
    pub(crate) fn lock_blocked(&self) -> usize {
        self.lock_blocked.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// The common path of changes without a caller — the closure commit and save are one critical section, and it commits regardless of
    ///   the save result.
    fn mutate<R>(&self, f: impl FnOnce(&mut ProfileMap) -> R) -> R {
        let mut guard = self.lock();
        let result = f(&mut guard.map);
        normalize_hierarchy(&mut guard.map);
        // ADR-0071 · ADR-0207
        self.persist(&mut guard);
        result
    }

    /// The conditional variant of `mutate` — saves only when the closure returns `true` (an actual change) or when dirty.
    /// ★While dirty, even calls without a change do disk IO (re-decision read + write) under the profiles lock★ — the price of carrying
    ///   the pending change (ADR-0291 D4).
    fn mutate_if(&self, f: impl FnOnce(&mut ProfileMap) -> bool) -> bool {
        let mut guard = self.lock();
        let changed = f(&mut guard.map);
        if changed {
            normalize_hierarchy(&mut guard.map);
        }
        // The pending change is not deferred until the next save 「with a change」.
        // ADR-0291 (D2)
        if changed || guard.dirty {
            self.persist(&mut guard);
        }
        changed
    }

    fn persist(&self, profiles: &mut Profiles) {
        match self.store.save(&snapshot(&profiles.map)) {
            Ok(()) => profiles.dirty = false,
            Err(error) => {
                profiles.dirty = true;
                tracing::debug!(%error, "profile save failed — the memory change is kept and the next save carries it");
            }
        }
    }

    // ADR-0291 R17
    fn try_mutate<R>(&self, f: impl FnOnce(&mut ProfileMap) -> R) -> Result<R, StoreError> {
        let mut guard = self.lock();
        let mut draft = guard.map.clone();
        let result = f(&mut draft);
        self.commit_if_saved(&mut guard, draft)?;
        Ok(result)
    }

    /// The conditional variant of `try_mutate` — if the closure returns `false`, this request changed nothing and it is `Ok(false)`.
    /// ★While dirty, even that request without a change saves the pending change while holding the profiles lock (disk IO)★ — the same as
    ///   `mutate_if`.
    fn try_mutate_if(&self, f: impl FnOnce(&mut ProfileMap) -> bool) -> Result<bool, StoreError> {
        let mut guard = self.lock();
        let mut draft = guard.map.clone();
        if !f(&mut draft) {
            // A failure of this save carrying the pending change is not this request's failure — this request changed nothing.
            if guard.dirty {
                self.persist(&mut guard);
            }
            return Ok(false);
        }
        self.commit_if_saved(&mut guard, draft)?;
        Ok(true)
    }

    fn commit_if_saved(
        &self,
        profiles: &mut Profiles,
        mut draft: ProfileMap,
    ) -> Result<(), StoreError> {
        normalize_hierarchy(&mut draft);
        self.store.save(&snapshot(&draft))?;
        profiles.map = draft;
        profiles.dirty = false;
        Ok(())
    }

    /// Some committed changes may not have made it to disk — the next successful save clears this. Failures of saves without changes also
    /// set it, so `true` does not guarantee the existence of a pending change.
    pub fn is_dirty(&self) -> bool {
        self.lock().dirty
    }

    /// The store's current state — does not take the registry lock (the store-state field is the end of the lock order).
    // ADR-0291 R17 (D6)
    pub fn store_status(&self) -> StoreStatus {
        self.store.status()
    }

    pub fn list(&self) -> Vec<AgentProfile> {
        self.lock().map.values().cloned().collect()
    }

    pub fn get(&self, id: AgentId) -> Option<AgentProfile> {
        self.lock().map.get(&id).cloned()
    }

    pub fn restorable(&self) -> Vec<AgentProfile> {
        self.lock()
            .map
            .values()
            .filter(|p| p.auto_restore)
            .cloned()
            .collect()
    }

    /// Inserts into the registry or replaces wholesale — the internal entry (commits + dirty on any save `Err`). ★No caller on the
    ///   production path★ — only tests and the
    /// harness seam (`AgentManager::seed_agent_bypassing_uniqueness`); production registration is [`ProfileRegistry::try_upsert`].
    // ADR-0291 R17
    #[cfg(any(test, feature = "test-harness"))]
    pub(crate) fn upsert(&self, profile: AgentProfile) {
        self.mutate(|m| {
            m.insert(profile.id, profile);
        });
    }

    /// Inserts into the registry or replaces wholesale — the entry with a caller: applied to a copy and committed only if the save succeeds.
    // ADR-0291 R17
    pub fn try_upsert(&self, profile: AgentProfile) -> Result<(), StoreError> {
        self.try_mutate(|m| {
            m.insert(profile.id, profile);
        })
    }

    /// [`ProfileRegistry::try_upsert`] only when `still` answers `true`, asked inside the registry lock just before the
    /// insert — `Ok(false)` = nothing inserted (nothing saved for this request). `still` must take no lock but a leaf
    /// (the library's turn mutex — ADR-0400 decision 18) and must not call back into the registry.
    // ADR-0400 decision 18
    pub fn try_upsert_if(
        &self,
        profile: AgentProfile,
        still: impl FnOnce() -> bool,
    ) -> Result<bool, StoreError> {
        self.try_mutate_if(|m| {
            if !still() {
                return false;
            }
            m.insert(profile.id, profile);
            true
        })
    }

    /// Spawn-only upsert — inserts the snapshot but **keeps the live entry's values for fields whose author is the runtime**. An id that
    ///   did not exist (ad-hoc spawn) takes the snapshot as is.
    ///
    /// ★The canonical preservation list is the body below.★ One rule: **who writes that field** — fields whose author is not the caller's
    ///   copy: live wins; only the other fields (command · env etc.) take the snapshot.
    ///
    /// ★Why not the snapshot as is (lost updates)★: a reparent/rename committed by another connection in between would be overwritten by
    ///   the snapshot's old `parent_id`/`display_name` (tree metadata unrelated to startup).
    /// ★The incarnation marker (epoch) is also kept live (ADR-0084)★: spawn settles it with `epoch_for_spawn` **right after** this upsert;
    ///   inserting the old snapshot value would revert that, so the live session would use the
    /// dead session's marker (missed resubscription, polluted turn observation, indistinguishable control tokens).
    // ADR-0070 ADR-0072 ADR-0084
    pub fn upsert_preserving_hierarchy(&self, profile: AgentProfile) {
        self.mutate(|m| {
            merge_preserving_live(m, profile);
        });
    }

    /// The entry with a caller (commits only if the save succeeds).
    pub fn try_upsert_preserving_hierarchy(&self, profile: AgentProfile) -> Result<(), StoreError> {
        self.try_mutate(|m| {
            merge_preserving_live(m, profile);
        })
    }

    /// Same merge but **does not create a missing id** — if present, updates and `true`; if absent, no-op and `false`.
    ///
    /// ★Prevents **resurrection** of a deleted profile★: with lookup and write separate, a deletion slipping in between is ignored and the
    ///   profile returns and even launches its child process. One critical section
    /// closes it (same shape as `reparent` · `epoch_for_spawn`). ★Do not swallow `false`★ — the caller must read it as 「deleted in the
    ///   meantime」 and cut the spawn.
    /// ★Not every resurrection path is closed★ — a deletion finished **before registration started** gives a `None` lookup and goes to the
    ///   new-registration branch; telling them apart needs a 「this existed」 signal
    /// `spawn_agent`'s signature lacks.
    pub fn update_preserving_hierarchy(&self, profile: AgentProfile) -> bool {
        self.mutate_if(|m| merge_if_present(m, profile))
    }

    /// The entry with a caller — the same as [`ProfileRegistry::update_preserving_hierarchy`] and commits only if the save succeeds.
    /// `Ok(false)` means exactly that it was deleted in the meantime.
    pub fn try_update_preserving_hierarchy(
        &self,
        profile: AgentProfile,
    ) -> Result<bool, StoreError> {
        self.try_mutate_if(|m| merge_if_present(m, profile))
    }
}

fn merge_if_present(m: &mut ProfileMap, profile: AgentProfile) -> bool {
    if !m.contains_key(&profile.id) {
        return false;
    }
    merge_preserving_live(m, profile);
    true
}

/// The merge body of the `upsert_preserving_hierarchy` family — with a live entry, guards **the fields whose author is the runtime**.
///
/// ★This function is the canonical preservation list★ (rule: who writes that field; the grounds sit next to each line).
fn merge_preserving_live(m: &mut HashMap<AgentId, AgentProfile>, mut profile: AgentProfile) {
    if let Some(live) = m.get(&profile.id) {
        profile.parent_id = live.parent_id;
        profile.display_name = live.display_name.clone();
        profile.epoch = live.epoch;
        // The last failure: the only writer is activation observation; reverting to the old copy would bring back an already-cleared failure.
        // ADR-0172
        profile.last_failure = live.last_failure;
        // ★The backend session handle and its history also keep the live value★ — the writer is the runtime (the first-submission latch
        //   writes it **after** this upsert, ADR-0226; the claude file watcher too), not the
        //   snapshot. ★Prevents★ a handle that arrived after the copy was taken being overwritten and **persisted** (the way back to that
        //   conversation lost with no undo, so keep; worst case is carrying a stale value until the
        //   next observation). ★The history is one lump with it★: record verbs push the old value into `old_session_ids` as a pair.
        // ★A Fresh spawn does not reuse the old conversation because of this★: spawn pushes the old value into the history and empties the
        //   field **after** this upsert, then hands a new uuid to an issuing backend (ADR-0076).
        // ★Behavior changes for claude too★: Resume of an issuing-axis backend reads this field from the registry and now sees the drift
        //   sid `SessionTracker` put in (`observe_session_id(id, None, ..)`), where the registration
        //   right before used to overwrite it with the snapshot.
        profile.backend_session_id = live.backend_session_id;
        profile.old_session_ids = live.old_session_ids.clone();
        // ★Attached presets · the 「Default」 field · the last applied instructions hash · the working folder also go to live★ — after
        //   creation these fields are written by preset commands · launches · the working-folder change (`agent.setWorkspace`), not by
        //   an old copy spawn took. If the copy won, presets attached and a working folder changed in between would silently revert and persist.
        // ADR-0321
        // TRD S25 §1-5 「merge_preserving_live preserved fields」
        profile.default_preset = live.default_preset;
        profile.presets = live.presets.clone();
        profile.last_applied_instructions_hash = live.last_applied_instructions_hash.clone();
        // ADR-0341: the launch writes it (`record_notice_fingerprint`) — a stale spawn copy would re-show notices.
        profile.notice_fingerprint = live.notice_fingerprint.clone();
        profile.cwd = live.cwd.clone();
    }
    m.insert(profile.id, profile);
}

impl ProfileRegistry {
    /// **Promote children to root on parent deletion (orphan-to-root):** children whose `parent_id` pointed at the target are set to
    ///   `None` **in the same critical
    /// section** — no dangling parent reference (tree render and restore invariant). Promotion, not cascade, so children's data is
    ///   preserved (prevents losing a whole group by mistake).
    ///
    /// The entry with a caller — applied to a copy and committed only if the save succeeds. A save is attempted even for a missing id.
    // ADR-0072
    // ADR-0291 R17
    pub fn try_remove(&self, id: AgentId) -> Result<(), StoreError> {
        self.try_mutate(|m| remove_promoting_children(m, id))
    }

    /// Sets/clears the display name override (ADR-0061 enrichment — tree rename). `Some(name)` saves the override, `None` clears it (back
    ///   to the cwd basename). Persists
    /// after a change and `true`; a missing id is a no-op and `false`.
    /// ★Normalization belongs to the save gate (`AgentManager::try_rename_agent`)★: trimming and 「whitespace-only → no override」 finish
    ///   there **before** the uniqueness decision
    /// (`normalize_display_name`); trimming again here would diverge the decided value from the saved one. Only normalized values or an
    ///   explicit `None` come here.
    ///
    /// The entry with a caller — applied to a copy and committed only if the save succeeds.
    // ADR-0291 R17
    pub fn try_rename(
        &self,
        id: AgentId,
        display_name: Option<String>,
    ) -> Result<bool, StoreError> {
        self.try_update_with(id, |p| p.display_name = display_name)
    }

    /// Sets/clears the tree parent (ADR-0072 — reparent). `Some(pid)` makes child_id a child of pid, `None` promotes to root. All
    ///   validation inside **one critical
    /// section (try_mutate)**; persists and returns `true` only on success, otherwise no-op and `false`.
    ///
    /// **One-level nesting rule (cycle prevention):** child must exist. `Some(pid)`: ① pid exists ② `pid != child_id` ③ child is not
    ///   currently anyone's parent ④ pid is itself
    /// a root (`parent_id == None`). `None`: always allowed if child exists.
    ///
    /// ★Validation inside the lock★: the existence/parenthood decision and the write are one critical section, closing the TOCTOU window
    ///   where concurrent reparent/delete would
    /// leak cycles and orphans (ADR-0071 lock discipline).
    ///
    /// The entry with a caller — a save is attempted even on a rule violation (`Ok(false)`).
    // ADR-0072
    // ADR-0291 R17
    pub fn try_reparent(
        &self,
        child_id: AgentId,
        parent_id: Option<AgentId>,
    ) -> Result<bool, StoreError> {
        self.try_mutate(|m| reparent_one_level(m, child_id, parent_id))
    }

    pub fn update_with(&self, id: AgentId, f: impl FnOnce(&mut AgentProfile)) -> bool {
        self.mutate(|m| update_present(m, id, f))
    }

    /// The entry with a caller (commits only if the save succeeds).
    pub fn try_update_with(
        &self,
        id: AgentId,
        f: impl FnOnce(&mut AgentProfile),
    ) -> Result<bool, StoreError> {
        self.try_mutate(|m| update_present(m, id, f))
    }

    /// Records/clears 「the last failure」. `true` if the value actually changed; `false` for a missing id · epoch mismatch · same value.
    ///
    /// `incarnation`: `Some(epoch)` — about **that incarnation**, written only when it equals the profile's current epoch. `None` — failed
    ///   before the incarnation was made (reservation refusal · PTY open
    /// failure), nothing to compare.
    ///
    /// ★What the epoch comparison prevents (load-bearing)★: the resume observation writes **late** (early-exit window, currently 3 s); if
    ///   the reaper collects the session and another connection activates the
    ///   same profile meanwhile (epoch +1 · clear done), the late observation would **stamp the dead incarnation's failure on the live
    ///   one**. Compare inside this lock, not in the caller (as `epoch_for_spawn`).
    /// ★Known and accepted residual of `None`★: no axis to compare, so it writes unconditionally, right after the observation (a few
    ///   commands wide). Closing it needs failed spawns to carry a generation
    ///   (a `spawn_agent` error-type change, separate).
    /// ★One caller★: `AgentManager::note_activation_result`; `pub(crate)` lets **the compiler** keep a second write path out of the
    ///   daemon/shell.
    /// ★No disk write — does not take `mutate` (as `epoch_for_spawn`)★: the field is `#[serde(skip)]`, so saving changes no byte.
    // ADR-0172
    pub(crate) fn set_last_failure(
        &self,
        id: AgentId,
        incarnation: Option<u32>,
        kind: Option<AgentFailureKind>,
    ) -> bool {
        let mut guard = self.lock();
        match guard.map.get_mut(&id) {
            Some(p) if incarnation.is_some_and(|e| e != p.epoch) => false,
            Some(p) if p.last_failure != kind => {
                p.last_failure = kind;
                true
            }
            _ => false,
        }
    }

    /// Records the notice fingerprint a caller was just shown — a compare-and-set. `true` = the in-memory map changed.
    ///
    /// Written only when, under the roster lock, both hold:
    /// - `incarnation` (the incarnation whose launch produced the notices) is still the profile's current one;
    /// - the stored value still equals `baseline` (the value that launch compared against in `prelaunch`).
    ///
    /// Otherwise it is skipped (logged at debug): a late record of an earlier launch must not overwrite what a later
    /// launch compared against. Skipping never marks a set that no caller was shown; in the common orders it only
    /// makes a set show once more. One improbable interleaving (a dying earlier incarnation recording between a later
    /// launch's compare and its epoch bump) can still leave the earlier set stored — accepted (S25 step 16 review).
    /// The fingerprint already stored → no write. An internal roster write, like the post-launch `auto_restore` raise
    /// (ADR-0291): a save failure keeps the change in memory, dirty (no `Err`) — the activation has already succeeded,
    /// and its reply is about to leave.
    // ADR-0341
    // ADR-0291
    // TRD S25 §1-5 「Notices once per agent」
    pub(crate) fn record_notice_fingerprint(
        &self,
        id: AgentId,
        incarnation: u32,
        baseline: Option<&str>,
        fingerprint: String,
    ) -> bool {
        let mut skipped = None;
        let written = self.mutate_if(|m| {
            let Some(p) = m.get_mut(&id) else {
                skipped = Some("the profile is gone");
                return false;
            };
            if p.epoch != incarnation {
                skipped = Some("a later incarnation is current");
                return false;
            }
            if p.notice_fingerprint.as_deref() == Some(fingerprint.as_str()) {
                return false;
            }
            if p.notice_fingerprint.as_deref() != baseline {
                skipped = Some("the stored fingerprint moved since the launch compared it");
                return false;
            }
            p.notice_fingerprint = Some(fingerprint);
            true
        });
        if let Some(why) = skipped {
            tracing::debug!(
                agent = %id,
                epoch = incarnation,
                why,
                "notice fingerprint not recorded — a later delivered launch may show this set again"
            );
        }
        written
    }

    /// Records the instructions hash an incarnation's first turn delivered ([`AgentProfile::last_applied_instructions_hash`]).
    /// `true` = the in-memory map changed.
    ///
    /// Written only while `incarnation` is still the profile's current one — a late report of an earlier incarnation
    /// is skipped (logged at debug). The hash already stored → no write. An internal roster write, like
    /// [`Self::record_notice_fingerprint`]: a save failure keeps the change in memory, dirty (no `Err`). Backend-neutral
    /// — a backend that never reads the stored value is unaffected.
    // ADR-0337
    pub(crate) fn record_instructions_hash(
        &self,
        id: AgentId,
        incarnation: u32,
        hash: &str,
    ) -> bool {
        let mut skipped = None;
        let written = self.mutate_if(|m| {
            let Some(p) = m.get_mut(&id) else {
                skipped = Some("the profile is gone");
                return false;
            };
            if p.epoch != incarnation {
                skipped = Some("a later incarnation is current");
                return false;
            }
            if p.last_applied_instructions_hash.as_deref() == Some(hash) {
                return false;
            }
            p.last_applied_instructions_hash = Some(hash.to_owned());
            true
        });
        if let Some(why) = skipped {
            tracing::debug!(
                agent = %id,
                epoch = incarnation,
                why,
                "instructions hash not recorded"
            );
        }
        written
    }

    /// Called where the session id is observed — moves the old sid into the history, replaces it, persists on change. A same-value call is
    ///   a no-op (no disk write).
    ///
    /// `incarnation`: `Some(epoch)` — written only when it equals the profile's current epoch. `None` — no incarnation axis; written
    ///   unconditionally.
    ///
    /// ★The `None` branch is a known and accepted residual (precedent [`ProfileRegistry::set_last_failure`])★: today only the file watcher
    ///   ([`crate::session_tracker::SessionTracker`]) comes in, and it
    ///   carries no marker; closing it would widen `SessionTracker::on_change`·`WatchEntry` and all creation points for no change in
    ///   today's claude behavior.
    /// ★Match/mismatch only★ — never "newer" (ADR-0007/0163).
    /// ★The guard prevents only 「a newer incarnation already stands」, not 「late writes」★: no exit path touches `epoch`, so a session that
    ///   **only ended** has the live marker and its late observation is written
    ///   to the dead session's profile.
    /// ★`bool` = 「the in-memory map changed」, not 「it stayed on disk」★ — a failing [`ProfileStore::save`] leaves the memory change (dirty).
    // ADR-0007
    // ADR-0163
    // ADR-0185
    pub fn observe_session_id(&self, id: AgentId, incarnation: Option<u32>, new_sid: Uuid) -> bool {
        self.mutate_if(|m| {
            m.get_mut(&id)
                .is_some_and(|p| replace_session_id(p, incarnation, new_sid, |_| true))
        })
    }

    /// The compare-and-swap verb by which this incarnation's **first submission** persists the session id.
    ///
    /// Checked in order inside the profile mutex:
    /// - incarnation marker differs from `incarnation` → `false`
    /// - field already `Some(new)` → `false` (nothing to write, no disk write)
    /// - field is `expected` → push the old value into the history, write `new`, persist, `true`
    /// - otherwise → `false` (another writer changed the field first)
    ///
    /// `expected` = **the field value this incarnation saw at start** (Fresh = `None` since the field was emptied · Resume = the stored value).
    ///
    /// ★Do not change it into an overwrite ([`ProfileRegistry::observe_session_id`])★: a writer that changed the field **before** the
    ///   first submission (the claude file watcher
    ///   writes without comparison) would be rewound to the spawn-time value.
    /// ★Not 「write only into an empty field」 (the rule ADR-0216 rejected)★: `expected` is the value seen at start, so a resuming
    ///   incarnation whose resume yields an id different
    ///   from the stored one replaces it.
    /// ★`bool` is in terms of the in-memory map★ — a failing or refusing [`ProfileStore::save`] leaves the memory change (dirty).
    // ADR-0226
    // ADR-0216
    // ADR-0217
    pub fn commit_session_id(
        &self,
        id: AgentId,
        incarnation: u32,
        expected: Option<Uuid>,
        new: Uuid,
    ) -> bool {
        let mut changed_by_another_writer = false;
        let written = self.mutate_if(|m| {
            m.get_mut(&id).is_some_and(|p| {
                replace_session_id(p, Some(incarnation), new, |current| {
                    changed_by_another_writer = current != expected;
                    !changed_by_another_writer
                })
            })
        });
        if changed_by_another_writer {
            tracing::debug!(
                agent = %id,
                epoch = incarnation,
                "another writer changed the session id field first — refusing the persistence of the first submission"
            );
        }
        written
    }

    /// Draws a new session id — ★does not touch the registry (does not persist)★.
    ///
    /// Kept here to leave the single issuing point in the registry. A new uuid every time keeps the Fresh seal 「always a new sid」
    ///   (ADR-0076 — 「Session ID already in use」).
    /// Persisted at the first submission by [`ProfileRegistry::commit_session_id`].
    // ADR-0226
    // ADR-0076
    pub fn mint_session_id() -> Uuid {
        Uuid::new_v4()
    }

    /// The verb by which a Fresh spawn starts by emptying the field — pushes the old value into the history and persists the empty field.
    ///
    /// `None` = no profile (deleted in the meantime) · `Some(true)` = pushed · `Some(false)` = the field was already empty. `None` is told
    ///   apart from 「nothing to push」
    /// because spawn must cut off on a vanished profile.
    /// ★Fresh only — on Resume it erases the handle to resume★. The canonical call condition is `manager`'s
    ///   `fresh_spawn_release_session_id` (crate-internal, so named, not linked).
    /// It pushes into the history, so it is not the destruction ADR-0202 forbids.
    // ADR-0226
    // ADR-0185
    pub fn release_session_id(&self, id: AgentId) -> Option<bool> {
        let mut outcome = None;
        self.mutate_if(|m| {
            outcome = m.get_mut(&id).map(retire_session_id);
            outcome == Some(true)
        });
        outcome
    }

    /// The entry with a caller (commits only if the save succeeds).
    pub fn try_release_session_id(&self, id: AgentId) -> Result<Option<bool>, StoreError> {
        let mut outcome = None;
        self.try_mutate_if(|m| {
            outcome = m.get_mut(&id).map(retire_session_id);
            outcome == Some(true)
        })?;
        Ok(outcome)
    }

    /// ★Settles in one critical section the **incarnation marker** spawn will use (ADR-0007)★ — a random number drawn anew per incarnation.
    ///
    /// `None` = the profile vanished (concurrent deletion) → **the caller must abort the spawn**.
    ///
    /// ★Why not an ordered counter★: only "same stream" is asked; a counter on disk puts the old marker back on a new incarnation, off
    ///   disk a restarted daemon counts from 0 and **restored agents
    ///   deterministically inherit a dead incarnation's value**. A random number removes the determinism.
    /// ★"Different for a new process" is **not** guaranteed★: the field is not read from disk (`skip_deserializing`), so a restored
    ///   profile compares against 0 and the comparison does not happen. Guaranteed:
    ///   **adjacency** — different from the immediately previous incarnation in the same process (the while loop below; every consumer
    ///   guard — ADR-0084 · ADR-0086 · ADR-0113 — compares by match/mismatch
    ///   alone). A cross-process collision remains at 2^-32 and disables that agent's incarnation guards for one incarnation.
    /// ★Decision and commit in one lock (load-bearing)★: split, a `remove` slipping in would plant a marker on a vanished profile.
    /// ★The only production call site is `AgentManager::spawn_agent`★ (Fresh/Resume). Scattering it made WS `Spawn` and boot restore miss
    ///   it (measured). Not dead code — deleting it makes replacement reuse
    ///   the marker (missed resubscription, observation pollution, token collisions).
    /// ★No disk write (same as `set_last_failure`)★: the field skips reads and writes a `0` placeholder (`AgentProfile::epoch`), so saving
    ///   changes no byte; no save refusal or failure, no entry distinction.
    // ADR-0084
    // ADR-0007
    // ADR-0291 (D8)
    pub fn epoch_for_spawn(&self, id: AgentId) -> Option<u32> {
        let mut guard = self.lock();
        let p = guard.map.get_mut(&id)?;
        let mut next = random_incarnation_tag();
        while next == p.epoch {
            next = random_incarnation_tag();
        }
        p.epoch = next;
        Some(next)
    }
}

/// Attachment (the 「Default」 field + attached presets) read · write.
///
/// ★The write and the comparison being `pub(crate)` is intentional★ — outside the crate (asset-library) changes go through the profile
/// edit port ([`crate::profile_edit::ProfileEditPort`] — implemented by `AgentManager`). ★The compiler does not enforce it★ — the generic
/// writes (`try_update_with` · `update_with` · `try_upsert`) are `pub`, so this path is a promise review keeps. Reads read the roster
/// directly (M5).
// ADR-0311
// TRD S25 §1-5 「profile edit port」
impl ProfileRegistry {
    pub fn attachments_of(&self, id: AgentId) -> Option<Attachments> {
        self.lock().map.get(&id).map(attachments_of_profile)
    }

    /// The whole roster — unordered.
    pub fn attachments(&self) -> Vec<(AgentId, Attachments)> {
        self.lock()
            .map
            .values()
            .map(|p| (p.id, attachments_of_profile(p)))
            .collect()
    }

    /// Whether the current launch inputs equal `seen` — compares the three fields together under one roster lock and does not save.
    /// `None` = the profile is gone (deleted meanwhile).
    // TRD S25 §1-5 (ga) 2 「compare-and-swap」
    pub(crate) fn launch_inputs_unchanged(&self, id: AgentId, seen: &LaunchInputs) -> Option<bool> {
        self.lock().map.get(&id).map(|p| seen.matches(p))
    }

    /// Replaces the attachments with `next` as a whole — an entry with a caller (commits only if the save succeeds).
    ///
    /// `Ok(false)` = not in the roster · `Ok(true)` = the attachments are now `next`. The same value is not saved (only pending dirty
    /// changes ride — that save's failure is not this request's failure).
    pub(crate) fn try_set_attachments(
        &self,
        id: AgentId,
        next: Attachments,
    ) -> Result<bool, StoreError> {
        let mut present = false;
        self.try_mutate_if(|m| {
            let Some(p) = m.get_mut(&id) else {
                return false;
            };
            present = true;
            if same_attachments(p, &next) {
                return false;
            }
            p.default_preset = next.default_preset;
            p.presets = next.presets;
            true
        })?;
        Ok(present)
    }

    /// Rewrites attached paths across the roster per `renames` — one lock · one save (an entry with a caller). Returns = agents whose
    /// value actually changed (id order). Nothing changed = no save.
    ///
    /// The mapping applies all at once — each path is looked up once by its **original** value (no chaining). Matching compares the
    /// whole path with platform `same_path` (no prefix match). Several `from` with the same key = the first line wins. 「Changed」 is
    /// literal — a case-only fix `a.md → A.md` is a change too. A changed agent's list keeps only the first of paths equal by
    /// `same_path` (an unchanged agent's list is left alone).
    // ADR-0335
    pub(crate) fn try_rewrite_preset_paths(
        &self,
        renames: &[PresetPathRename],
    ) -> Result<Vec<AgentId>, StoreError> {
        // ADR-0325: looked up by a key that answers like `same_path`.
        let mut mapping: HashMap<PathKey, &str> = HashMap::with_capacity(renames.len());
        for r in renames {
            mapping
                .entry(path_key(Path::new(&r.from)))
                .or_insert(r.to.as_str());
        }
        let mut changed = Vec::new();
        self.try_mutate_if(|m| {
            for p in m.values_mut() {
                let mut touched = false;
                for preset in &mut p.presets {
                    match mapping.get(&path_key(Path::new(&preset.path))) {
                        Some(&to) if to != preset.path => {
                            preset.path = to.to_owned();
                            touched = true;
                        }
                        _ => {}
                    }
                }
                if touched {
                    // ADR-0339
                    // An overwrite-move = delete + rename, so `[a, b]` with `a → b` gives `[b, b]` — the second is leftover.
                    let mut seen = HashSet::with_capacity(p.presets.len());
                    p.presets
                        .retain(|preset| seen.insert(path_key(Path::new(&preset.path))));
                    changed.push(p.id);
                }
            }
            !changed.is_empty()
        })?;
        changed.sort_unstable();
        Ok(changed)
    }

    /// Removes attached paths across the roster per `paths` — one lock · one save (an entry with a caller · rules =
    /// [`crate::profile_edit::ProfileEditPort::remove_preset_paths`]). Nothing removed = no save (only pending dirty
    /// changes ride — that save's failure is not this request's failure).
    // ADR-0339
    pub(crate) fn try_remove_preset_paths(
        &self,
        paths: &[String],
    ) -> Result<Vec<AgentId>, StoreError> {
        // ADR-0325: looked up by a key that answers like `same_path`.
        let gone: HashSet<PathKey> = paths.iter().map(|p| path_key(Path::new(p))).collect();
        let mut changed = Vec::new();
        self.try_mutate_if(|m| {
            for p in m.values_mut() {
                let before = p.presets.len();
                p.presets
                    .retain(|preset| !gone.contains(&path_key(Path::new(&preset.path))));
                if p.presets.len() < before {
                    changed.push(p.id);
                }
            }
            !changed.is_empty()
        })?;
        changed.sort_unstable();
        Ok(changed)
    }

    /// `agent.setWorkspace`'s write, in one locked section: the agent still in the roster and never conversed (no stored
    /// backend session id), then `cwd` written and `display_name` set to `name` only while no override names the agent
    /// (unset or blank — [`AgentProfile::name_override`]). A refusal (gone ·
    /// conversed) or a request that changes nothing saves nothing of its own (only a pending dirty change rides — that
    /// save's failure is not this request's), so a store error never replaces the refusal; a store error of the write
    /// itself is `Err` with nothing changed.
    /// ★The conversed check is made here too, not only by the caller★ — the first-submission latch and
    ///   `observe_session_id` write the session id without the agent's reservation, so a check before this lock can be stale.
    // ADR-0321
    // ADR-0386
    pub(crate) fn try_set_workspace(
        &self,
        id: AgentId,
        cwd: &Path,
        name: &str,
    ) -> Result<WorkspaceWrite, StoreError> {
        let mut outcome = WorkspaceWrite::NotFound;
        self.try_mutate_if(|m| {
            let Some(p) = m.get_mut(&id) else {
                return false;
            };
            if p.backend_session_id.is_some() {
                outcome = WorkspaceWrite::Conversed;
                return false;
            }
            outcome = WorkspaceWrite::Written;
            let named = p.name_override().is_some();
            if p.cwd == cwd && named {
                return false;
            }
            p.cwd = cwd.to_path_buf();
            if !named {
                p.display_name = Some(name.to_owned());
            }
            true
        })?;
        Ok(outcome)
    }

    /// Inserts the copy `build` makes of `original`, reading the original and inserting the copy in one locked section — so
    /// a copy queues with the attachment rewrites on this lock ([`Self::try_rewrite_preset_paths`] ·
    /// [`Self::try_remove_preset_paths`]) and cannot slip past one. `Ok(None)` = the original is gone: nothing is saved for
    /// this request (only a pending dirty change rides), so a store error never masks it. A store error of the insert is
    /// `Err` with nothing inserted.
    // ADR-0386
    pub(crate) fn try_insert_copy(
        &self,
        original: AgentId,
        build: impl FnOnce(&AgentProfile) -> AgentProfile,
    ) -> Result<Option<AgentProfile>, StoreError> {
        let mut inserted = None;
        self.try_mutate_if(|m| {
            let Some(source) = m.get(&original) else {
                return false;
            };
            let copy = build(source);
            inserted = Some(copy.clone());
            m.insert(copy.id, copy);
            true
        })?;
        Ok(inserted)
    }
}

/// What [`ProfileRegistry::try_set_workspace`] did.
// ADR-0386
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WorkspaceWrite {
    /// The folder is set (or already was, with the name pinned).
    Written,
    NotFound,
    /// The agent has conversed — nothing was written.
    Conversed,
}

fn attachments_of_profile(p: &AgentProfile) -> Attachments {
    Attachments {
        default_preset: p.default_preset,
        presets: p.presets.clone(),
    }
}

fn same_attachments(p: &AgentProfile, a: &Attachments) -> bool {
    p.default_preset == a.default_preset && p.presets == a.presets
}

/// The fields the launch preprocessing checked that the launch uses — attachments · command · env. The launch uses that check's answer
/// only if these three are unchanged after it.
///
/// ★The working folder is not included★ — a change (`agent.setWorkspace`) takes that agent's reservation as a working-folder holder so it does not cross a
///   launch, and the merge keeps the roster value even when the caller copy's working folder is stale (so including it in the copy comparison would cut off a legitimate launch).
// TRD S25 §1-5 (ga) 2
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct LaunchInputs {
    pub(crate) attachments: Attachments,
    pub(crate) command: AgentCommand,
    pub(crate) env: Vec<(String, String)>,
}

impl LaunchInputs {
    pub(crate) fn of(p: &AgentProfile) -> Self {
        Self {
            attachments: attachments_of_profile(p),
            command: p.command.clone(),
            env: p.env.clone(),
        }
    }

    fn matches(&self, p: &AgentProfile) -> bool {
        same_attachments(p, &self.attachments) && self.same_command_and_env(p)
    }

    /// Whether `p`'s command · env equal these — the two fields the spawn merge moves from the caller copy to the roster.
    pub(crate) fn same_command_and_env(&self, p: &AgentProfile) -> bool {
        p.command == self.command && p.env == self.env
    }
}

fn remove_promoting_children(m: &mut ProfileMap, id: AgentId) {
    m.remove(&id);
    // With one-level nesting, children are not parents, so recursive promotion is unnecessary — one sweep is enough.
    for p in m.values_mut() {
        if p.parent_id == Some(id) {
            p.parent_id = None;
        }
    }
}

fn reparent_one_level(m: &mut ProfileMap, child_id: AgentId, parent_id: Option<AgentId>) -> bool {
    if !m.contains_key(&child_id) {
        return false;
    }
    if let Some(pid) = parent_id {
        if pid == child_id {
            return false;
        }
        match m.get(&pid) {
            Some(parent) if parent.parent_id.is_none() => {}
            _ => return false,
        }
        if m.values().any(|p| p.parent_id == Some(child_id)) {
            return false;
        }
    }
    match m.get_mut(&child_id) {
        Some(c) => {
            c.parent_id = parent_id;
            true
        }
        None => false,
    }
}

fn update_present(m: &mut ProfileMap, id: AgentId, f: impl FnOnce(&mut AgentProfile)) -> bool {
    match m.get_mut(&id) {
        Some(p) => {
            f(p);
            true
        }
        None => false,
    }
}

fn snapshot(map: &ProfileMap) -> Vec<AgentProfile> {
    map.values().cloned().collect()
}

/// The **common body** of the two incarnation-guarded session id record verbs ([`ProfileRegistry::observe_session_id`] ·
///   [`ProfileRegistry::commit_session_id`]) — their
/// guard and history push exist only here. The unguarded clear ([`ProfileRegistry::release_session_id`]) shares `retire_session_id` for
///   the history push.
///
/// ★A new record verb must go through this helper★ — a separately written guard diverges from the other copy; that is the 「new bypass」
///   ADR-0217 decision 6 (inherited by
///   ADR-0218 decision 6) forbids.
///
/// `admits` is called **only after passing** the guard and the 「already the same value」 check (argument = the field's current value).
// ADR-0217
// ADR-0226
fn replace_session_id(
    p: &mut AgentProfile,
    incarnation: Option<u32>,
    new_sid: Uuid,
    admits: impl FnOnce(Option<Uuid>) -> bool,
) -> bool {
    if incarnation.is_some_and(|e| e != p.epoch)
        || p.backend_session_id == Some(new_sid)
        || !admits(p.backend_session_id)
    {
        return false;
    }
    retire_session_id(p);
    p.backend_session_id = Some(new_sid);
    p.last_active = now_epoch_ms();
    true
}

fn retire_session_id(p: &mut AgentProfile) -> bool {
    match p.backend_session_id.take() {
        Some(old) => {
            p.old_session_ids.push(old);
            true
        }
        None => false,
    }
}

fn default_preset_attached() -> bool {
    true
}

/// A leftover placeholder — always writes `0` regardless of the live marker (grounds = the `AgentProfile::epoch` comment).
fn serialize_zero_placeholder<S: serde::Serializer>(_tag: &u32, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_u32(0)
}

/// Issues one incarnation marker. ★Uses the bytes of a uuid v4 to avoid bringing in a random-number crate★ — v4 is filled by the OS CSPRNG
/// and the version/variant bits sit in bytes 6 and 8, so the first 4 bytes are fully random.
fn random_incarnation_tag() -> u32 {
    let b = Uuid::new_v4().into_bytes();
    u32::from_be_bytes([b[0], b[1], b[2], b[3]])
}

#[cfg(test)]
impl ProfileRegistry {
    /// Whether nobody holds the roster lock now — `false` even when the holder is this thread (does not wait).
    pub(crate) fn lock_is_free(&self) -> bool {
        self.profiles.try_lock().is_ok()
    }
}

/// A profile store whose save results the test decides — shared by the registry and manager tests.
#[cfg(test)]
pub(crate) mod scripted_store {
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use super::{AgentId, AgentProfile, ProfileStore, Refusal, StoreError};

    #[derive(Clone, Copy, Debug)]
    pub(crate) enum Outcome {
        Saves,
        FailsIo,
        Refuses(Refusal),
    }

    /// Both write IO failure and re-decision refusal — entry handling does not distinguish the variants of `StoreError`.
    pub(crate) const FAILURES: [Outcome; 3] = [
        Outcome::FailsIo,
        Outcome::Refuses(Refusal::Unreadable),
        Outcome::Refuses(Refusal::Newer { found: 2 }),
    ];

    /// Uses the queued results once each from the front and is `then` once they run out (with `cycle`, used results are queued again at
    ///   the end and it loops forever).
    /// Only successful saves remain in `disk`.
    pub(crate) struct ScriptedStore {
        queue: Mutex<VecDeque<Outcome>>,
        repeat: AtomicBool,
        then: Mutex<Outcome>,
        disk: Mutex<Vec<AgentProfile>>,
        attempts: AtomicUsize,
    }

    impl ScriptedStore {
        pub(crate) fn new() -> Arc<Self> {
            Arc::new(Self {
                queue: Mutex::default(),
                repeat: AtomicBool::new(false),
                then: Mutex::new(Outcome::Saves),
                disk: Mutex::default(),
                attempts: AtomicUsize::default(),
            })
        }

        pub(crate) fn script(&self, queued: &[Outcome], then: Outcome) {
            *self.queue.lock().unwrap() = queued.iter().copied().collect();
            self.repeat.store(false, Ordering::SeqCst);
            *self.then.lock().unwrap() = then;
        }

        pub(crate) fn cycle(&self, outcomes: &[Outcome]) {
            self.script(outcomes, Outcome::Saves);
            self.repeat.store(true, Ordering::SeqCst);
        }

        pub(crate) fn set(&self, outcome: Outcome) {
            self.script(&[], outcome);
        }

        pub(crate) fn disk(&self, id: AgentId) -> Option<AgentProfile> {
            self.disk
                .lock()
                .unwrap()
                .iter()
                .find(|p| p.id == id)
                .cloned()
        }

        pub(crate) fn disk_all(&self) -> Vec<AgentProfile> {
            self.disk.lock().unwrap().clone()
        }

        pub(crate) fn attempts(&self) -> usize {
            self.attempts.load(Ordering::SeqCst)
        }
    }

    impl ProfileStore for ScriptedStore {
        fn save(&self, profiles: &[AgentProfile]) -> Result<(), StoreError> {
            self.attempts.fetch_add(1, Ordering::SeqCst);
            let queued = {
                let mut queue = self.queue.lock().unwrap();
                let next = queue.pop_front();
                if let Some(outcome) = next.filter(|_| self.repeat.load(Ordering::SeqCst)) {
                    queue.push_back(outcome);
                }
                next
            };
            match queued.unwrap_or_else(|| *self.then.lock().unwrap()) {
                Outcome::Saves => {
                    *self.disk.lock().unwrap() = profiles.to_vec();
                    Ok(())
                }
                Outcome::FailsIo => Err(StoreError::Io(std::io::Error::other("disk full"))),
                Outcome::Refuses(refusal) => Err(StoreError::Refused(refusal)),
            }
        }

        fn load(&self) -> Vec<AgentProfile> {
            self.disk.lock().unwrap().clone()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct MemStore {
        saved: Mutex<Vec<AgentProfile>>,
        /// Looked at by the entry that measures 「no disk write」.
        saves: std::sync::atomic::AtomicUsize,
    }
    impl ProfileStore for MemStore {
        fn save(&self, profiles: &[AgentProfile]) -> Result<(), StoreError> {
            self.saves.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            *self.saved.lock().unwrap() = profiles.to_vec();
            Ok(())
        }
        fn load(&self) -> Vec<AgentProfile> {
            self.saved.lock().unwrap().clone()
        }
    }

    fn sample() -> AgentProfile {
        AgentProfile::new(
            "t".into(),
            AgentCommand::Claude {
                extra_args: vec![],
                output_format: AgentOutputFormat::Terminal,
            },
            PathBuf::from("."),
            vec![],
            true,
        )
    }

    #[test]
    fn canonical_name_when_live_mirrors_the_live_session_rule() {
        // ★The relative path is the core★: `PathBuf::from(".")` is raw (basename `"."`) while a live session uses the basename of the
        //   canonicalized absolute path — the two must match
        //   (`resolve_display_name(None, ".")` gives `"."` and silently diverges).
        let p = sample(); // cwd = "."
        let expected = {
            let abs = dunce::canonicalize(".").expect("the cwd exists");
            crate::name::canonical_name_or_id_fallback(None, &abs.to_string_lossy(), p.id)
        };
        assert_eq!(p.canonical_name_when_live(), expected);
        assert_ne!(
            p.canonical_name_when_live(),
            ".",
            "using the raw cwd basename diverges from the live name"
        );

        let mut named = sample();
        named.display_name = Some("Alice".into());
        assert_eq!(named.canonical_name_when_live(), "Alice");
        let mut blank = sample();
        blank.display_name = Some("   ".into());
        assert_eq!(
            blank.canonical_name_when_live(),
            expected,
            "a whitespace-only override is ignored (same as the live session rule — resolve_display_name lacks this guard)"
        );
    }

    #[test]
    fn canonical_name_when_live_survives_a_vanished_cwd_when_an_override_exists() {
        // With an override the name must not depend on fs at all.
        let vanished = PathBuf::from("C:/engram-does-not-exist-9f1c/never/created");
        assert!(
            dunce::canonicalize(&vanished).is_err(),
            "the premise of this test — this path must not exist so it takes the canonicalize failure path"
        );

        let mut named = sample();
        named.cwd = vanished.clone();
        named.display_name = Some("Renamed".into());
        assert_eq!(
            named.canonical_name_when_live(),
            "Renamed",
            "with an override fs is not looked at (a canonicalize failure must not sway the name)"
        );

        let mut anon = sample();
        anon.cwd = vanished;
        anon.display_name = None;
        assert_eq!(
            anon.canonical_name_when_live(),
            "created",
            "no override + cwd gone → raw basename (a known and accepted gap)"
        );

        let mut blank = sample();
        blank.cwd = PathBuf::from("   ");
        blank.display_name = None;
        let id8 = blank.id.to_string()[..8].to_string();
        assert_eq!(
            blank.canonical_name_when_live(),
            id8,
            "an empty cwd degrades to the id regardless of the canonicalize result"
        );
    }

    #[test]
    fn upsert_and_get() {
        let reg = ProfileRegistry::new(Arc::new(MemStore::default()));
        let p = sample();
        let id = p.id;
        reg.upsert(p);
        assert!(reg.get(id).is_some());
        assert_eq!(reg.list().len(), 1);
    }

    #[test]
    fn observe_session_id_pushes_old_and_persists() {
        let store = Arc::new(MemStore::default());
        let reg = ProfileRegistry::new(store.clone());
        let mut p = sample();
        let sid1 = Uuid::new_v4();
        p.backend_session_id = Some(sid1);
        let id = p.id;
        reg.upsert(p);
        let sid2 = Uuid::new_v4();

        assert!(reg.observe_session_id(id, None, sid2));
        let got = reg.get(id).unwrap();
        assert_eq!(got.backend_session_id, Some(sid2));
        assert!(
            got.old_session_ids.contains(&sid1),
            "the old sid must remain in the history"
        );

        assert!(!reg.observe_session_id(id, None, sid2));

        let persisted = store.load();
        assert_eq!(persisted[0].backend_session_id, Some(sid2));
    }

    // ── Incarnation guard (ADR-0007/0163 · ADR-0185) ──

    /// ★A dead incarnation's late observation cannot overwrite the live incarnation's value★ — if overwritten, that profile believes
    ///   another conversation is
    /// its own, and resume opens someone else's thread.
    #[test]
    fn observe_session_id_rejects_a_stale_incarnation() {
        let reg = ProfileRegistry::new(Arc::new(MemStore::default()));
        let p = sample();
        let id = p.id;
        reg.upsert(p);
        let live = reg.epoch_for_spawn(id).unwrap();
        let held = Uuid::new_v4();
        assert!(reg.observe_session_id(id, Some(live), held));

        let stale = live.wrapping_add(1);
        let intruder = Uuid::new_v4();
        assert!(
            !reg.observe_session_id(id, Some(stale), intruder),
            "an observation with a different marker passed"
        );
        assert_eq!(
            reg.get(id).unwrap().backend_session_id,
            Some(held),
            "refused but the value changed"
        );
    }

    /// The paired direction — with the same marker it writes. Without this, the entry above is green even as 「always refuse」.
    #[test]
    fn observe_session_id_accepts_a_matching_incarnation() {
        let reg = ProfileRegistry::new(Arc::new(MemStore::default()));
        let p = sample();
        let id = p.id;
        reg.upsert(p);
        let live = reg.epoch_for_spawn(id).unwrap();
        let sid = Uuid::new_v4();

        assert!(reg.observe_session_id(id, Some(live), sid));
        assert_eq!(reg.get(id).unwrap().backend_session_id, Some(sid));
    }

    /// `None` = no axis to compare → writes regardless of the marker (the claude-side file watcher branch).
    #[test]
    fn observe_session_id_without_an_incarnation_writes_unconditionally() {
        let reg = ProfileRegistry::new(Arc::new(MemStore::default()));
        let p = sample();
        let id = p.id;
        reg.upsert(p);
        let live = reg.epoch_for_spawn(id).unwrap();
        let sid = Uuid::new_v4();

        // ★It also measures that attaching a marker at the same place gets refused★ — without this pair, this entry is green even if the guard
        //   disappears entirely.
        assert!(!reg.observe_session_id(id, Some(live.wrapping_add(1)), sid));
        assert!(reg.observe_session_id(id, None, sid));
        assert_eq!(reg.get(id).unwrap().backend_session_id, Some(sid));
    }

    // ── First-submission persistence — compare-and-swap (ADR-0226) ──

    fn registry_with_live_incarnation() -> (ProfileRegistry, Arc<MemStore>, AgentId, u32) {
        let store = Arc::new(MemStore::default());
        let reg = ProfileRegistry::new(store.clone());
        let p = sample();
        let id = p.id;
        reg.upsert(p);
        let live = reg.epoch_for_spawn(id).unwrap();
        (reg, store, id, live)
    }

    fn saves(store: &MemStore) -> usize {
        store.saves.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Fresh incarnation — it started by emptying the field, so `expected = None`, and the first submission writes the value drawn beforehand.
    #[test]
    fn commit_session_id_writes_when_the_slot_is_what_the_incarnation_saw() {
        let (reg, store, id, live) = registry_with_live_incarnation();
        let minted = ProfileRegistry::mint_session_id();

        assert!(reg.commit_session_id(id, live, None, minted));
        assert_eq!(reg.get(id).unwrap().backend_session_id, Some(minted));
        assert_eq!(
            store.load()[0].backend_session_id,
            Some(minted),
            "when the compare-and-swap succeeds, it persists on the spot"
        );
        assert!(
            reg.get(id).unwrap().old_session_ids.is_empty(),
            "it started from an empty field, so there is no old value to push"
        );
    }

    /// ★Pins that it is not 「write only into an empty field」 (rejected by ADR-0216)★: a resuming incarnation whose first offer differs
    ///   from the stored value (resume returned a
    ///   new id) has field == `expected` == stored value, so **it is replaced**; an empty-field-only rule would refuse and leave the
    ///   handle stale.
    #[test]
    fn a_resumed_incarnations_first_offer_with_a_different_id_replaces_the_stored_one() {
        let (reg, store, id, live) = registry_with_live_incarnation();
        let stored = Uuid::new_v4();
        assert!(reg.observe_session_id(id, Some(live), stored));
        let resumed_as = Uuid::new_v4();

        assert!(reg.commit_session_id(id, live, Some(stored), resumed_as));
        let got = reg.get(id).unwrap();
        assert_eq!(got.backend_session_id, Some(resumed_as));
        assert!(
            got.old_session_ids.contains(&stored),
            "the old value is pushed into the history"
        );
        assert_eq!(store.load()[0].backend_session_id, Some(resumed_as));
    }

    #[test]
    fn commit_session_id_with_the_value_already_in_the_slot_does_not_write() {
        let (reg, store, id, live) = registry_with_live_incarnation();
        let stored = Uuid::new_v4();
        assert!(reg.observe_session_id(id, Some(live), stored));
        let before = saves(&store);

        assert!(!reg.commit_session_id(id, live, Some(stored), stored));
        assert_eq!(
            saves(&store),
            before,
            "with the same value there must be no disk write"
        );
        let got = reg.get(id).unwrap();
        assert_eq!(got.backend_session_id, Some(stored));
        assert!(
            !got.old_session_ids.contains(&stored),
            "the same value is not pushed into the history"
        );
    }

    /// ★commit does not rewind the value the tracker wrote **before** the first submission★ — the tracker (claude file watcher) writes
    ///   without comparison; an overwrite would
    ///   rewind it to the spawn-time value and push the tracker's into the history.
    #[test]
    fn commit_session_id_does_not_rewind_a_value_the_tracker_wrote_before_the_first_submission() {
        // Resuming incarnation: saw the stored value S at the start, but before the first submission the tracker wrote Y.
        let (reg, store, id, live) = registry_with_live_incarnation();
        let stored = Uuid::new_v4();
        assert!(reg.observe_session_id(id, Some(live), stored));
        let drifted = Uuid::new_v4();
        assert!(reg.observe_session_id(id, None, drifted));
        let before = saves(&store);

        assert!(!reg.commit_session_id(id, live, Some(stored), stored));
        assert_eq!(reg.get(id).unwrap().backend_session_id, Some(drifted));
        assert_eq!(saves(&store), before, "a refusal means no disk write");

        // Fresh incarnation: saw an empty field at the start, but before the first submission the tracker wrote Y.
        let (reg, _store, id, live) = registry_with_live_incarnation();
        let drifted = Uuid::new_v4();
        assert!(reg.observe_session_id(id, None, drifted));

        assert!(!reg.commit_session_id(id, live, None, ProfileRegistry::mint_session_id()));
        assert_eq!(
            reg.get(id).unwrap().backend_session_id,
            Some(drifted),
            "the tracker value stands as is"
        );
    }

    /// The paired direction — with a different marker it does not write even if the field is `expected`. Without this, the entries above
    ///   are green even without the guard.
    #[test]
    fn commit_session_id_rejects_a_stale_incarnation() {
        let (reg, store, id, live) = registry_with_live_incarnation();
        let before = saves(&store);

        assert!(!reg.commit_session_id(id, live.wrapping_add(1), None, Uuid::new_v4()));
        assert_eq!(reg.get(id).unwrap().backend_session_id, None);
        assert_eq!(saves(&store), before);
    }

    #[test]
    fn commit_session_id_on_a_missing_profile_is_refused() {
        let reg = ProfileRegistry::new(Arc::new(MemStore::default()));
        assert!(!reg.commit_session_id(AgentId::new_v4(), 0, None, Uuid::new_v4()));
    }

    /// ★The two record verbs go through **one** guard helper★ (ADR-0217 decision 6) — running shows only 「the same result」, not 「the same
    ///   code」, so it is measured in the source.
    #[test]
    fn the_two_session_id_writers_share_one_guard_helper() {
        let src = include_str!("profile.rs");
        let production = src.split("mod tests {").next().expect("production section");
        let body_of = |sig: &str, next: &str| -> String {
            assert_eq!(
                production.matches(sig).count(),
                1,
                "`{sig}` must be exactly one in the production section for this entry to pin down the body"
            );
            let rest = production.split(sig).nth(1).expect("body");
            let end = rest.find(next).unwrap_or_else(|| {
                panic!("`{next}` is not after `{sig}` — this entry's premise is stale")
            });
            rest[..end].to_string()
        };
        let observe = body_of(
            "pub fn observe_session_id(",
            "/// The compare-and-swap verb by which this incarnation's **first submission**",
        );
        let commit = body_of("pub fn commit_session_id(", "/// Draws a new session id");
        for (name, body) in [
            ("observe_session_id", &observe),
            ("commit_session_id", &commit),
        ] {
            assert!(
                body.contains("replace_session_id("),
                "`{name}` does not go through the common helper: {body}"
            );
            assert!(
                !body.contains("p.epoch"),
                "`{name}` wrote the incarnation guard separately — a second guard outside the helper: {body}"
            );
        }
    }

    // ── The verb by which Fresh empties the field (ADR-0226) ──

    #[test]
    fn release_session_id_pushes_the_old_value_and_persists_an_empty_slot() {
        let (reg, store, id, live) = registry_with_live_incarnation();
        let old = Uuid::new_v4();
        assert!(reg.observe_session_id(id, Some(live), old));

        assert_eq!(reg.release_session_id(id), Some(true));
        let got = reg.get(id).unwrap();
        assert_eq!(got.backend_session_id, None);
        assert!(
            got.old_session_ids.contains(&old),
            "the old value goes into the history"
        );
        assert_eq!(
            store.load()[0].backend_session_id,
            None,
            "the empty field is persisted"
        );
    }

    #[test]
    fn release_session_id_on_an_empty_slot_reports_nothing_to_push() {
        let (reg, store, id, _live) = registry_with_live_incarnation();
        let before = saves(&store);
        assert_eq!(reg.release_session_id(id), Some(false));
        assert_eq!(
            saves(&store),
            before,
            "nothing changed, so nothing is written"
        );
    }

    /// ★`None` is not 「there was nothing to push」 but 「there is no profile」★ — spawn cuts off by telling these apart.
    #[test]
    fn release_session_id_tells_a_vanished_profile_apart() {
        let reg = ProfileRegistry::new(Arc::new(MemStore::default()));
        assert_eq!(reg.release_session_id(AgentId::new_v4()), None);
    }

    #[test]
    fn mint_session_id_touches_nothing_and_never_repeats() {
        let (reg, store, id, _live) = registry_with_live_incarnation();
        let before = saves(&store);
        let a = ProfileRegistry::mint_session_id();
        let b = ProfileRegistry::mint_session_id();
        assert_ne!(
            a, b,
            "a new uuid every time (the Fresh seal of always a new sid)"
        );
        assert_eq!(saves(&store), before, "no disk write");
        assert_eq!(
            reg.get(id).unwrap().backend_session_id,
            None,
            "the registry field is unchanged too"
        );
    }

    #[test]
    fn epoch_for_spawn_never_repeats_the_previous_incarnations_tag() {
        let reg = ProfileRegistry::new(Arc::new(MemStore::default()));
        let p = sample();
        let id = p.id;
        reg.upsert(p);

        // Adjacency guarantee = the only thing the while loop actually guards. Every consumer guard tells two
        //   incarnations apart by match/mismatch alone, so if the value overlapped with the previous incarnation being drained, those
        //   guards would be disabled entirely.
        let mut seen = Vec::new();
        for _ in 0..64 {
            let tag = reg.epoch_for_spawn(id).expect("profile exists");
            assert!(
                seen.last() != Some(&tag),
                "the same marker as the immediately previous incarnation must not come out: {tag}"
            );
            seen.push(tag);
        }
        assert_ne!(
            seen[0], 0,
            "a new profile's default is 0, so only the first issue avoids 0 (later issues have no such guarantee)"
        );

        reg.try_remove(id).expect("save succeeds");
        assert_eq!(
            reg.epoch_for_spawn(id),
            None,
            "without a profile, None — planting a marker on a missing profile would have spawn make a ghost incarnation"
        );
    }

    // ★Targets that issuing is **not deterministic**★. The adjacency test above cannot catch a counter regression (a counter also differs
    //   from the previous value). They differ at
    //   **a new process's first issue**: a counter repeats the same value, so a restarted daemon puts a dead incarnation's marker back on
    //   restored agents (stale cursor → new frames
    //   dropped by dedup → empty slot). Only a random number lacks that determinism.
    // ★Probabilistic★: 64 draws from a 32-bit space with under half distinct is ~1e-7 — effectively 0, not "guaranteed by construction"
    //   (`epoch_for_spawn` is canonical).
    #[test]
    fn a_fresh_registry_does_not_hand_out_the_same_first_tag_every_time() {
        let first_tags: Vec<u32> = (0..64)
            .map(|_| {
                let reg = ProfileRegistry::new(Arc::new(MemStore::default()));
                let p = sample();
                let id = p.id;
                reg.upsert(p);
                reg.epoch_for_spawn(id).expect("profile exists")
            })
            .collect();
        let distinct: std::collections::HashSet<u32> = first_tags.iter().copied().collect();
        assert!(
            distinct.len() >= 32,
            "if every process gave the same first marker, restart restore would inherit the dead incarnation's value (counter regression): {distinct:?}"
        );
    }

    #[test]
    fn a_stale_snapshot_upsert_does_not_roll_back_the_incarnation_tag() {
        let reg = ProfileRegistry::new(Arc::new(MemStore::default()));
        let p = sample();
        let id = p.id;
        reg.upsert(p.clone());
        let live_tag = reg.epoch_for_spawn(id).expect("profile exists");
        // upsert with the old snapshot the spawn caller held (marker = default 0).
        reg.upsert_preserving_hierarchy(p);
        assert_eq!(
            reg.get(id).expect("exists").epoch,
            live_tag,
            "the live value must win (if the snapshot reverted the marker, the live session would use the dead incarnation's marker)"
        );
    }

    #[test]
    fn epoch_for_spawn_never_writes_to_disk() {
        let (reg, store, id, before_tag) = registry_with_live_incarnation();
        let before = saves(&store);

        let tag = reg.epoch_for_spawn(id).expect("profile exists");

        assert_ne!(tag, before_tag);
        assert_eq!(reg.get(id).unwrap().epoch, tag);
        assert_eq!(
            saves(&store),
            before,
            "if marker issuing called save, a save refusal or failure caused by a value not even carried in the file would cut the launch (D8)"
        );
    }

    // ── Last failure (ADR-0172) ────────────────────────────────────────────────────

    #[test]
    fn last_failure_never_reaches_the_serialized_profile() {
        let mut p = sample();
        p.last_failure = Some(AgentFailureKind::NoConversationToResume);
        let json = serde_json::to_string(&p).expect("serialize");
        assert!(
            !json.contains("last_failure"),
            "if this field went out to the profile file, the daemon-lifetime constraint would break — got {json}"
        );
        let back: AgentProfile = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(
            back.last_failure, None,
            "the value coming back from disk is always empty"
        );
    }

    #[test]
    fn set_last_failure_reports_change_and_never_writes_to_disk() {
        let store = Arc::new(MemStore::default());
        let reg = ProfileRegistry::new(store.clone());
        let p = sample();
        let id = p.id;
        reg.upsert(p);

        assert!(reg.set_last_failure(id, None, Some(AgentFailureKind::SpawnFailed)));
        assert!(
            !reg.set_last_failure(id, None, Some(AgentFailureKind::SpawnFailed)),
            "rewriting the same value is not a change"
        );
        assert_eq!(
            reg.get(id).unwrap().last_failure,
            Some(AgentFailureKind::SpawnFailed)
        );
        assert!(
            reg.set_last_failure(id, None, None),
            "clearing is also a change"
        );
        assert_eq!(reg.get(id).unwrap().last_failure, None);
        assert!(
            !reg.set_last_failure(Uuid::new_v4(), None, Some(AgentFailureKind::Other)),
            "a missing id is a no-op"
        );

        assert!(
            store.load().iter().all(|p| p.last_failure.is_none()),
            "the value is not carried in the snapshot that went out to the store either"
        );
    }

    /// ★Late-write guard★: the resume observation writes late (early-exit window, 3 s). If another connection activates the same profile
    ///   in between (new incarnation marker),
    /// the late old observation would stamp the dead incarnation's failure on **the live new incarnation**.
    ///
    /// ★Markers are not constants★: they are random per incarnation (ADR-0163), with no `0 → 1` order, so the old and new values are
    ///   **issued and held**; the guard also stands on
    ///   match/mismatch only.
    #[test]
    fn a_late_write_from_an_older_incarnation_is_rejected() {
        let reg = ProfileRegistry::new(Arc::new(MemStore::default()));
        let p = sample();
        let id = p.id;
        reg.upsert(p);
        // Two incarnation replacements — holding the previous incarnation's marker and the live incarnation's marker each in hand.
        let old = reg.epoch_for_spawn(id).expect("first incarnation marker");
        let live = reg.epoch_for_spawn(id).expect("second incarnation marker");
        assert_ne!(
            old, live,
            "premise: adjacent incarnations get different markers"
        );

        assert!(
            !reg.set_last_failure(id, Some(old), Some(AgentFailureKind::EarlyExitAfterResume)),
            "the old incarnation's late observation must be rejected"
        );
        assert_eq!(reg.get(id).unwrap().last_failure, None);

        assert!(
            reg.set_last_failure(id, Some(live), Some(AgentFailureKind::EarlyExitAfterResume)),
            "the current incarnation's observation is written as is"
        );
        // Clearing takes the same guard — the old incarnation's success observation must not clear the new incarnation's record.
        assert!(!reg.set_last_failure(id, Some(old), None));
        assert_eq!(
            reg.get(id).unwrap().last_failure,
            Some(AgentFailureKind::EarlyExitAfterResume)
        );
        assert!(
            reg.set_last_failure(id, Some(live), None),
            "the current incarnation clears it"
        );

        // A failure that ended before the incarnation has no generation to compare, so it writes unconditionally (the `None` branch of the
        //   contract).
        assert!(reg.set_last_failure(id, None, Some(AgentFailureKind::SpawnFailed)));
        assert_eq!(
            reg.get(id).unwrap().last_failure,
            Some(AgentFailureKind::SpawnFailed)
        );
    }

    #[test]
    fn last_failure_survives_a_stale_snapshot_upsert() {
        let reg = ProfileRegistry::new(Arc::new(MemStore::default()));
        let p = sample();
        let id = p.id;
        reg.upsert(p.clone());
        reg.set_last_failure(id, None, Some(AgentFailureKind::NoConversationToResume));
        // upsert with the old snapshot the spawn caller held (last_failure=None).
        reg.upsert_preserving_hierarchy(p);
        assert_eq!(
            reg.get(id).expect("exists").last_failure,
            Some(AgentFailureKind::NoConversationToResume),
            "the live value must win (if the snapshot cleared the failure record, the display would silently disappear)"
        );
    }

    // ── Display name override (ADR-0061 enrichment — tree rename) ──────────────────────────────

    #[test]
    fn new_profile_has_no_display_name_override() {
        assert_eq!(sample().display_name, None);
    }

    #[test]
    fn rename_sets_and_persists_display_name() {
        let store = Arc::new(MemStore::default());
        let reg = ProfileRegistry::new(store.clone());
        let p = sample();
        let id = p.id;
        reg.upsert(p);
        assert!(reg
            .try_rename(id, Some("my agent".to_string()))
            .expect("save succeeds"));
        assert_eq!(
            reg.get(id).unwrap().display_name,
            Some("my agent".to_string())
        );
        assert_eq!(store.load()[0].display_name, Some("my agent".to_string()));
    }

    #[test]
    fn rename_none_clears_display_name() {
        let reg = ProfileRegistry::new(Arc::new(MemStore::default()));
        let p = sample();
        let id = p.id;
        reg.upsert(p);
        reg.try_rename(id, Some("x".to_string()))
            .expect("save succeeds");
        assert!(reg.try_rename(id, None).expect("save succeeds"));
        assert_eq!(reg.get(id).unwrap().display_name, None);
    }

    #[test]
    fn rename_missing_is_noop_false() {
        let reg = ProfileRegistry::new(Arc::new(MemStore::default()));
        assert!(!reg
            .try_rename(Uuid::new_v4(), Some("y".to_string()))
            .expect("save succeeds"));
    }

    // ── Tree hierarchy reparent (ADR-0072) ────────────────────────────────────────────

    #[test]
    fn reparent_sets_and_persists() {
        let store = Arc::new(MemStore::default());
        let reg = ProfileRegistry::new(store.clone());
        let parent = sample();
        let child = sample();
        let (pid, cid) = (parent.id, child.id);
        reg.upsert(parent);
        reg.upsert(child);

        assert!(reg.try_reparent(cid, Some(pid)).expect("save succeeds"));
        assert_eq!(reg.get(cid).unwrap().parent_id, Some(pid));
        let disk = store.load();
        let persisted_child = disk.iter().find(|p| p.id == cid).unwrap();
        assert_eq!(persisted_child.parent_id, Some(pid));
    }

    #[test]
    fn reparent_none_promotes_to_root() {
        let reg = ProfileRegistry::new(Arc::new(MemStore::default()));
        let parent = sample();
        let child = sample();
        let (pid, cid) = (parent.id, child.id);
        reg.upsert(parent);
        reg.upsert(child);
        reg.try_reparent(cid, Some(pid)).expect("save succeeds");
        assert!(reg.try_reparent(cid, None).expect("save succeeds"));
        assert_eq!(reg.get(cid).unwrap().parent_id, None);
    }

    #[test]
    fn reparent_rejects_self_parent() {
        let reg = ProfileRegistry::new(Arc::new(MemStore::default()));
        let p = sample();
        let id = p.id;
        reg.upsert(p);
        assert!(
            !reg.try_reparent(id, Some(id)).expect("save succeeds"),
            "self-parent is refused"
        );
        assert_eq!(reg.get(id).unwrap().parent_id, None);
    }

    #[test]
    fn reparent_rejects_nonexistent_parent() {
        let reg = ProfileRegistry::new(Arc::new(MemStore::default()));
        let child = sample();
        let cid = child.id;
        reg.upsert(child);
        assert!(
            !reg.try_reparent(cid, Some(Uuid::new_v4()))
                .expect("save succeeds"),
            "specifying a missing parent is refused"
        );
        assert_eq!(reg.get(cid).unwrap().parent_id, None);
    }

    #[test]
    fn reparent_rejects_missing_child() {
        let reg = ProfileRegistry::new(Arc::new(MemStore::default()));
        let parent = sample();
        let pid = parent.id;
        reg.upsert(parent);
        assert!(
            !reg.try_reparent(Uuid::new_v4(), Some(pid))
                .expect("save succeeds"),
            "a missing child is refused"
        );
    }

    #[test]
    fn reparent_rejects_making_a_node_with_children_a_child() {
        let reg = ProfileRegistry::new(Arc::new(MemStore::default()));
        let a = sample(); // future parent
        let b = sample(); // child of a
        let c = sample(); // a new root trying to make a its child
        let (aid, bid, cid) = (a.id, b.id, c.id);
        reg.upsert(a);
        reg.upsert(b);
        reg.upsert(c);
        assert!(reg.try_reparent(bid, Some(aid)).expect("save succeeds"));
        assert!(
            !reg.try_reparent(aid, Some(cid)).expect("save succeeds"),
            "a node with children cannot become a child (one level)"
        );
        assert_eq!(reg.get(aid).unwrap().parent_id, None);
    }

    #[test]
    fn reparent_rejects_parent_that_has_a_parent() {
        let reg = ProfileRegistry::new(Arc::new(MemStore::default()));
        let a = sample(); // root
        let b = sample(); // child of a
        let c = sample(); // a node trying to take b as its parent
        let (aid, bid, cid) = (a.id, b.id, c.id);
        reg.upsert(a);
        reg.upsert(b);
        reg.upsert(c);
        assert!(reg.try_reparent(bid, Some(aid)).expect("save succeeds"));
        assert!(
            !reg.try_reparent(cid, Some(bid)).expect("save succeeds"),
            "specifying a child is refused when the parent has a parent (one level)"
        );
        assert_eq!(reg.get(cid).unwrap().parent_id, None);
    }

    #[test]
    fn delete_parent_orphans_children_to_root() {
        let store = Arc::new(MemStore::default());
        let reg = ProfileRegistry::new(store.clone());
        let a = sample();
        let b = sample();
        let c = sample();
        let (aid, bid, cid) = (a.id, b.id, c.id);
        reg.upsert(a);
        reg.upsert(b);
        reg.upsert(c);
        reg.try_reparent(bid, Some(aid)).expect("save succeeds");
        reg.try_reparent(cid, Some(aid)).expect("save succeeds");

        reg.try_remove(aid).expect("save succeeds");
        assert!(reg.get(aid).is_none());
        assert_eq!(
            reg.get(bid).unwrap().parent_id,
            None,
            "b is promoted to root"
        );
        assert_eq!(
            reg.get(cid).unwrap().parent_id,
            None,
            "c is promoted to root"
        );

        let disk = store.load();
        assert!(!disk.iter().any(|p| p.id == aid));
        assert!(disk
            .iter()
            .filter(|p| p.id == bid || p.id == cid)
            .all(|p| p.parent_id.is_none()));
    }

    // ── Write-boundary normalization (ADR-0072) ────────────────────────────────────────────────

    /// ★Reproduction★: load a store that already holds a cyclic pair (a damaged/hand-edited profile file or a legacy write bypassing
    ///   reparent validation; load does not normalize),
    /// then trigger any write — boundary normalization heals the cycle. **Fails without normalize_hierarchy** (both parent_ids stay, in
    ///   memory and on disk).
    #[test]
    fn cycle_in_map_is_normalized_at_write_boundary() {
        let mut a = sample();
        let mut b = sample();
        let (aid, bid) = (a.id, b.id);
        a.parent_id = Some(bid);
        b.parent_id = Some(aid);
        let store = Arc::new(MemStore::default());
        store.save(&[a, b]).expect("memory save");
        let reg = ProfileRegistry::new(store.clone());
        assert_eq!(
            reg.get(aid).unwrap().parent_id,
            Some(bid),
            "right after load the cycle remains"
        );

        reg.try_rename(aid, Some("touch".into()))
            .expect("save succeeds");

        assert_eq!(
            reg.get(aid).unwrap().parent_id,
            None,
            "cycle participant A → root"
        );
        assert_eq!(
            reg.get(bid).unwrap().parent_id,
            None,
            "cycle participant B → root"
        );
        let disk = store.load();
        assert!(
            disk.iter().all(|p| p.parent_id.is_none()),
            "no cycle on disk either"
        );
    }

    #[test]
    fn update_with_dangling_parent_is_normalized() {
        let store = Arc::new(MemStore::default());
        let reg = ProfileRegistry::new(store.clone());
        let child = sample();
        let cid = child.id;
        reg.upsert(child);

        let missing = Uuid::new_v4();
        assert!(reg.update_with(cid, |p| p.parent_id = Some(missing)));
        assert_eq!(
            reg.get(cid).unwrap().parent_id,
            None,
            "a dangling reference is normalized to root"
        );
        assert!(
            store
                .load()
                .iter()
                .find(|p| p.id == cid)
                .unwrap()
                .parent_id
                .is_none(),
            "no dangling on disk either"
        );
    }

    #[test]
    fn normalization_preserves_valid_hierarchy() {
        let reg = ProfileRegistry::new(Arc::new(MemStore::default()));
        let parent = sample();
        let child = sample();
        let (pid, cid) = (parent.id, child.id);
        reg.upsert(parent);
        reg.upsert(child);
        assert!(reg.try_reparent(cid, Some(pid)).expect("save succeeds"));
        reg.try_rename(cid, Some("x".into()))
            .expect("save succeeds");
        reg.try_rename(pid, Some("y".into()))
            .expect("save succeeds");
        assert_eq!(
            reg.get(cid).unwrap().parent_id,
            Some(pid),
            "a normal parent relation is not damaged by normalization (idempotent)"
        );
    }

    #[test]
    fn concurrent_reparent_vs_remove_no_dangling() {
        use std::thread;

        for _ in 0..200 {
            let store = Arc::new(MemStore::default());
            let reg = Arc::new(ProfileRegistry::new(store.clone()));
            let parent = sample();
            let child = sample();
            let (pid, cid) = (parent.id, child.id);
            reg.upsert(parent);
            reg.upsert(child);

            let r1 = reg.clone();
            let r2 = reg.clone();
            let h1 = thread::spawn(move || {
                r1.try_reparent(cid, Some(pid)).expect("save succeeds");
            });
            let h2 = thread::spawn(move || {
                r2.try_remove(pid).expect("save succeeds");
            });
            h1.join().unwrap();
            h2.join().unwrap();

            let disk = store.load();
            assert!(!disk.iter().any(|p| p.id == pid), "the parent is deleted");
            if let Some(c) = disk.iter().find(|p| p.id == cid) {
                assert_ne!(
                    c.parent_id,
                    Some(pid),
                    "a dangling reference pointing at the deleted parent must not persist"
                );
            }
            let ids: std::collections::HashSet<_> = disk.iter().map(|p| p.id).collect();
            assert!(
                disk.iter()
                    .filter_map(|p| p.parent_id)
                    .all(|pp| ids.contains(&pp)),
                "no dangling parent reference in the persisted hierarchy"
            );
        }
    }

    #[test]
    fn spawn_preserving_upsert_does_not_revert_concurrent_reparent_or_rename() {
        let reg = ProfileRegistry::new(Arc::new(MemStore::default()));
        let root = sample();
        let child = sample();
        let (rid, cid) = (root.id, child.id);
        reg.upsert(root);
        reg.upsert(child);

        let stale_snapshot = reg.get(cid).unwrap();
        assert_eq!(stale_snapshot.parent_id, None);
        assert_eq!(stale_snapshot.display_name, None);

        // In between, another connection commits a reparent + rename.
        assert!(reg.try_reparent(cid, Some(rid)).expect("save succeeds"));
        assert!(reg
            .try_rename(cid, Some("live".into()))
            .expect("save succeeds"));

        reg.upsert_preserving_hierarchy(stale_snapshot);
        let after = reg.get(cid).unwrap();
        assert_eq!(
            after.parent_id,
            Some(rid),
            "a stale spawn snapshot must not revert the latest parent_id (sealing lost updates)"
        );
        assert_eq!(
            after.display_name,
            Some("live".into()),
            "a stale spawn snapshot must not revert the latest display_name (ADR-0070 latent)"
        );
    }

    /// ★A deleted profile is **not revived** by spawn registration★.
    ///
    /// With 「is it there」 and 「write」 separate, a deletion slipping in between is ignored: the entry returns to the list and its child
    ///   process launches. The decision inside the
    /// lock is the only fix, and `false` is the caller's signal to cut the spawn. ★When present, it merges as usual★ — that half is
    ///   measured too.
    #[test]
    fn update_preserving_hierarchy_never_resurrects_a_removed_profile() {
        let reg = ProfileRegistry::new(Arc::new(MemStore::default()));
        let p = sample();
        let id = p.id;
        let snapshot = p.clone();
        reg.upsert(p);

        assert!(
            reg.update_preserving_hierarchy(snapshot.clone()),
            "could not update an existing id — the normal branch of this verb died"
        );

        reg.try_remove(id).expect("save succeeds");
        assert!(
            !reg.update_preserving_hierarchy(snapshot),
            "spawn registration put a value back into a deleted profile"
        );
        assert!(
            reg.get(id).is_none(),
            "a deleted profile came back to life — after the deletion is reported as successful, the entry and its child come back together"
        );
    }

    /// ★A backend handle arriving **after** the copy was taken is not overwritten by spawn registration★.
    ///
    /// The runtime writes this field (the first-submission latch writes it **after** this upsert); the caller's copy is not the author.
    /// ★Overwriting is an **irreversible loss**, not a stale value★ — this upsert is `mutate`, so it persists and no handle back to that
    ///   conversation remains.
    /// ★The history is measured too★ — reverting `old_session_ids` to the snapshot would drop handles pushed out in between (half the loss).
    #[test]
    fn spawn_preserving_upsert_does_not_revert_a_handle_recorded_after_the_snapshot() {
        let reg = ProfileRegistry::new(Arc::new(MemStore::default()));
        let mut p = sample();
        let first = Uuid::new_v4();
        p.backend_session_id = Some(first);
        let id = p.id;
        reg.upsert(p);

        let stale_snapshot = reg.get(id).unwrap();
        assert_eq!(stale_snapshot.backend_session_id, Some(first));
        assert!(stale_snapshot.old_session_ids.is_empty());

        // In between, the other side gave a new handle and the record port wrote it to the registry.
        let arrived = Uuid::new_v4();
        assert!(reg.observe_session_id(id, None, arrived));

        reg.upsert_preserving_hierarchy(stale_snapshot);

        let after = reg.get(id).unwrap();
        assert_eq!(
            after.backend_session_id,
            Some(arrived),
            "a handle arriving after the snapshot was overwritten by spawn registration — the way back to that conversation is gone"
        );
        assert_eq!(
            after.old_session_ids,
            vec![first],
            "the handle was kept but the history reverted to the snapshot — pushed-out handles vanish from the list"
        );
    }

    /// ★An old copy spawn took does not revert presets attached · the working folder changed · the instructions hash written in between★ —
    ///   a revert would persist as is. Fields whose author is the copy (`env` etc.) still follow the copy.
    // TRD S25 §1-5 「merge_preserving_live preserved fields」
    #[test]
    fn spawn_preserving_upsert_keeps_presets_workspace_and_instructions_hash_from_live() {
        let reg = ProfileRegistry::new(Arc::new(MemStore::default()));
        let p = sample();
        let id = p.id;
        reg.upsert(p);
        let mut stale_snapshot = reg.get(id).unwrap();

        assert!(reg
            .try_update_with(id, |p| {
                p.default_preset = false;
                p.presets = vec![AttachedPreset {
                    path: "presets/review.yaml".into(),
                }];
                p.last_applied_instructions_hash = Some("ab12".into());
                p.cwd = PathBuf::from("C:/moved");
            })
            .expect("save succeeds"));
        stale_snapshot.env = vec![("FROM_SPAWN".into(), "1".into())];

        for via_update in [false, true] {
            if via_update {
                assert!(reg.update_preserving_hierarchy(stale_snapshot.clone()));
            } else {
                reg.upsert_preserving_hierarchy(stale_snapshot.clone());
            }
            let after = reg.get(id).unwrap();
            assert!(!after.default_preset);
            assert_eq!(
                after.presets,
                vec![AttachedPreset {
                    path: "presets/review.yaml".into()
                }]
            );
            assert_eq!(
                after.last_applied_instructions_hash.as_deref(),
                Some("ab12")
            );
            assert_eq!(after.cwd, PathBuf::from("C:/moved"));
            assert_eq!(
                after.env, stale_snapshot.env,
                "fields whose author is the copy follow the copy"
            );
        }
    }

    #[test]
    fn a_new_profile_has_only_the_default_preset_and_no_instructions_hash() {
        let p = sample();
        assert!(p.default_preset);
        assert!(p.presets.is_empty());
        assert_eq!(p.last_applied_instructions_hash, None);
    }

    // ── Notice fingerprint (ADR-0341) ──

    /// A new (or duplicated) agent starts with nothing recorded — its first delivered launch shows every notice.
    #[test]
    fn a_new_profile_has_no_notice_fingerprint() {
        assert_eq!(sample().notice_fingerprint, None);
    }

    /// The launch writes the fingerprint, not the spawn copy — a stale copy must not undo a record.
    #[test]
    fn spawn_preserving_upsert_keeps_the_notice_fingerprint_from_live() {
        let reg = ProfileRegistry::new(Arc::new(MemStore::default()));
        let p = sample();
        let id = p.id;
        reg.upsert(p);
        let stale_snapshot = reg.get(id).unwrap();
        let live = reg.epoch_for_spawn(id).unwrap();
        assert!(reg.record_notice_fingerprint(id, live, None, "x".into()));

        reg.upsert_preserving_hierarchy(stale_snapshot.clone());
        assert_eq!(
            reg.get(id).unwrap().notice_fingerprint.as_deref(),
            Some("x")
        );
        assert!(reg.update_preserving_hierarchy(stale_snapshot));
        assert_eq!(
            reg.get(id).unwrap().notice_fingerprint.as_deref(),
            Some("x")
        );
    }

    /// Written for the current incarnation only, saved when it changes, and the empty set is a value of its own.
    #[test]
    fn record_notice_fingerprint_is_guarded_by_the_incarnation_and_saves_a_change() {
        let (reg, store, id, live) = registry_with_live_incarnation();
        let before = saves(&store);

        assert!(!reg.record_notice_fingerprint(id, live.wrapping_add(1), None, "x".into()));
        assert_eq!(
            reg.get(id).unwrap().notice_fingerprint,
            None,
            "another incarnation"
        );
        assert!(!reg.record_notice_fingerprint(Uuid::new_v4(), live, None, "x".into()));

        assert!(reg.record_notice_fingerprint(id, live, None, "x".into()));
        assert_eq!(saves(&store), before + 1, "a change is saved");
        assert_eq!(
            store
                .load()
                .into_iter()
                .find(|p| p.id == id)
                .unwrap()
                .notice_fingerprint
                .as_deref(),
            Some("x")
        );
        assert!(
            !reg.record_notice_fingerprint(id, live, Some("x"), "x".into()),
            "same value"
        );
        assert_eq!(saves(&store), before + 1, "the same value writes nothing");

        assert!(reg.record_notice_fingerprint(id, live, Some("x"), String::new()));
        assert_eq!(
            reg.get(id).unwrap().notice_fingerprint.as_deref(),
            Some(""),
            "an empty set is recorded, not cleared to `None`"
        );
    }

    /// ★Compare-and-set on the baseline★ — X → Y → X: launch A (plan Y, compared against X) records late, after launch B
    /// (plan X) compared against the still-stored X and showed nothing. B's record over A's Y is skipped (B's baseline X
    /// moved), so the next X launch shows X again instead of X staying hidden for good.
    #[test]
    fn record_notice_fingerprint_writes_only_over_the_baseline_it_compared_against() {
        let (reg, store, id, live) = registry_with_live_incarnation();
        assert!(reg.record_notice_fingerprint(id, live, None, "x".into()));
        let before = saves(&store);

        assert!(
            !reg.record_notice_fingerprint(id, live, None, "y".into()),
            "the stored value moved from the baseline (None → x)"
        );
        assert!(
            !reg.record_notice_fingerprint(id, live, Some("z"), "y".into()),
            "another baseline"
        );
        assert_eq!(
            reg.get(id).unwrap().notice_fingerprint.as_deref(),
            Some("x"),
            "a skipped record leaves the stored value"
        );
        assert_eq!(saves(&store), before, "a skipped record writes nothing");

        assert!(reg.record_notice_fingerprint(id, live, Some("x"), "y".into()));
        assert_eq!(
            reg.get(id).unwrap().notice_fingerprint.as_deref(),
            Some("y")
        );
    }

    /// Not a launch input — a record between the check and the compare must not race the launch.
    #[test]
    fn the_notice_fingerprint_is_not_a_launch_input() {
        let (reg, _store, id, live) = registry_with_live_incarnation();
        let seen = LaunchInputs::of(&reg.get(id).unwrap());
        assert!(reg.record_notice_fingerprint(id, live, None, "x".into()));
        assert_eq!(reg.launch_inputs_unchanged(id, &seen), Some(true));
    }

    /// Written for the current incarnation only (a later one is current → skipped, a gone profile → skipped), saved
    /// when it changes, and the same value writes nothing.
    #[test]
    fn record_instructions_hash_is_guarded_by_the_incarnation_and_saves_a_change() {
        let (reg, store, id, live) = registry_with_live_incarnation();
        let before = saves(&store);

        assert!(!reg.record_instructions_hash(id, live.wrapping_sub(1), "h"));
        assert_eq!(
            reg.get(id).unwrap().last_applied_instructions_hash,
            None,
            "an earlier incarnation's late report"
        );
        assert_eq!(saves(&store), before, "a skipped record writes nothing");
        assert!(!reg.record_instructions_hash(Uuid::new_v4(), live, "h"));

        assert!(reg.record_instructions_hash(id, live, "h"));
        assert_eq!(saves(&store), before + 1, "a change is saved");
        assert_eq!(
            store
                .load()
                .into_iter()
                .find(|p| p.id == id)
                .unwrap()
                .last_applied_instructions_hash
                .as_deref(),
            Some("h")
        );
        assert!(!reg.record_instructions_hash(id, live, "h"), "same value");
        assert_eq!(saves(&store), before + 1, "the same value writes nothing");

        let next = reg.epoch_for_spawn(id).unwrap();
        assert!(
            !reg.record_instructions_hash(id, live, "old"),
            "a later incarnation is current"
        );
        assert!(reg.record_instructions_hash(id, next, "k"));
        assert_eq!(
            reg.get(id)
                .unwrap()
                .last_applied_instructions_hash
                .as_deref(),
            Some("k")
        );
    }

    /// Not a launch input — the record lands after the spawn and must not race the next launch's check.
    #[test]
    fn the_instructions_hash_is_not_a_launch_input() {
        let (reg, _store, id, live) = registry_with_live_incarnation();
        let seen = LaunchInputs::of(&reg.get(id).unwrap());
        assert!(reg.record_instructions_hash(id, live, "h"));
        assert_eq!(reg.launch_inputs_unchanged(id, &seen), Some(true));
    }

    #[test]
    fn preserving_upsert_inserts_snapshot_when_no_live_entry() {
        let reg = ProfileRegistry::new(Arc::new(MemStore::default()));
        let mut p = sample();
        let id = p.id;
        p.display_name = Some("adhoc".into());
        reg.upsert_preserving_hierarchy(p);
        assert_eq!(reg.get(id).unwrap().display_name, Some("adhoc".into()));
    }

    #[test]
    fn deserializes_profile_without_parent_id_as_none() {
        let json = r#"{
            "id": "00000000-0000-0000-0000-000000000002",
            "name": "no-parent",
            "command": { "kind": "Claude", "extra_args": [] },
            "cwd": ".",
            "env": [],
            "backend_session_id": null,
            "old_session_ids": [],
            "epoch": 0,
            "auto_restore": true,
            "created_at": 1,
            "last_active": 1
        }"#;
        let p: AgentProfile =
            serde_json::from_str(json).expect("deserialize a profile without parent_id");
        assert_eq!(p.parent_id, None, "parent_id absent → None (root)");
    }

    #[test]
    fn concurrent_reparent_persisted_equals_final_map() {
        use std::thread;

        let store = Arc::new(MemStore::default());
        let reg = Arc::new(ProfileRegistry::new(store.clone()));
        let root = sample();
        let root_id = root.id;
        reg.upsert(root);

        let mut handles = Vec::new();
        for _ in 0..4 {
            let r = reg.clone();
            handles.push(thread::spawn(move || {
                for _ in 0..50 {
                    let child = sample();
                    let cid = child.id;
                    r.upsert(child);
                    r.try_reparent(cid, Some(root_id)).expect("save succeeds");
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }

        let mem = reg.list();
        let disk = store.load();
        assert_eq!(mem.len(), 201, "1 root + 200 children");
        assert_eq!(
            disk.len(),
            mem.len(),
            "disk count == in-memory (nothing missing)"
        );
        let children_ok = mem
            .iter()
            .filter(|p| p.id != root_id)
            .all(|p| p.parent_id == Some(root_id));
        assert!(
            children_ok,
            "after concurrent reparents every child has the root as parent"
        );
    }

    // ── Concurrency: persisted == latest (sealing the stale-overwrite race) ────────────────────

    #[test]
    fn save_writes_current_map_not_stale_snapshot() {
        let store = Arc::new(MemStore::default());
        let reg = ProfileRegistry::new(store.clone());
        let p = sample();
        let id = p.id;
        reg.upsert(p);
        reg.try_rename(id, Some("final".to_string()))
            .expect("save succeeds");
        let disk = store.load();
        let mem = reg.list();
        assert_eq!(disk.len(), mem.len());
        assert_eq!(disk[0].display_name, Some("final".to_string()));
        assert_eq!(
            disk[0].display_name, mem[0].display_name,
            "persisted == observed"
        );
    }

    #[test]
    fn concurrent_mutations_persisted_equals_final_map() {
        use std::thread;

        let store = Arc::new(MemStore::default());
        let reg = Arc::new(ProfileRegistry::new(store.clone()));

        let mut handles = Vec::new();
        for t in 0..4 {
            let r = reg.clone();
            handles.push(thread::spawn(move || {
                for i in 0..50 {
                    let p = sample();
                    let id = p.id;
                    r.upsert(p);
                    r.try_rename(id, Some(format!("t{t}-{i}")))
                        .expect("save succeeds");
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }

        let mem = reg.list();
        let disk = store.load();
        assert_eq!(mem.len(), 200, "200 in-memory upserts");
        assert_eq!(
            disk.len(),
            mem.len(),
            "disk count == in-memory count (no entry missing through a stale snapshot)"
        );

        let mut mem_sorted: Vec<_> = mem.iter().map(|p| (p.id, p.display_name.clone())).collect();
        let mut disk_sorted: Vec<_> = disk
            .iter()
            .map(|p| (p.id, p.display_name.clone()))
            .collect();
        mem_sorted.sort();
        disk_sorted.sort();
        assert_eq!(
            disk_sorted, mem_sorted,
            "after concurrent mutations disk == latest in-memory (persisted == observed)"
        );
    }

    /// The old `last_restore` key is ignored as an unknown field (serde's default deny_unknown not applied).
    #[test]
    fn deserializes_legacy_profile_without_new_fields() {
        let legacy = r#"{
            "id": "00000000-0000-0000-0000-000000000001",
            "name": "legacy",
            "command": { "kind": "Claude", "extra_args": [] },
            "cwd": ".",
            "env": [],
            "backend_session_id": null,
            "old_session_ids": [],
            "epoch": 3,
            "auto_restore": true,
            "created_at": 100,
            "last_active": 200,
            "last_restore": 150
        }"#;
        let p: AgentProfile =
            serde_json::from_str(legacy).expect("legacy profile must deserialize");
        // The `"epoch": 3` an old file carried is treated the same (an unknown field) and just ignored — the incarnation marker
        //   cannot live across processes, so the disk value must not be revived (that field's comment). No migration needed.
        assert_eq!(p.epoch, 0);
        assert_eq!(p.restart_policy, RestartPolicy::Always);
        assert_eq!(p.restart_count, 0);
        assert_eq!(p.failed_reason, None);
        assert_eq!(p.last_start_at, None);
        assert_eq!(p.display_name, None);
        assert_eq!(p.parent_id, None);
    }

    /// ★A **live** incarnation marker must not be carried on disk★: a restarted daemon would put the old marker back on restored agents, a
    ///   surviving client would keep its old
    /// cursor, and the new frames (numbered from 0) would be dropped by dedup and leave the slot empty. (The key itself is carried — a
    ///   leftover `0` placeholder; grounds = the `AgentProfile::epoch` comment.)
    #[test]
    fn the_live_incarnation_tag_never_reaches_disk() {
        let mut p = sample();
        p.epoch = 0xDEAD_BEEF;
        let json = serde_json::to_string(&p).expect("serialize profile");
        assert!(
            !json.contains("3735928559") && !json.to_lowercase().contains("deadbeef"),
            "a live incarnation marker was carried on disk: {json}"
        );
        let back: AgentProfile = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(
            back.epoch, 0,
            "a profile read in starts without a marker (the next spawn issues a new one)"
        );
    }

    /// ★Even if the file carries a non-zero value, the in-memory value does not come from the file★ — restoring it would cause what the
    ///   test above prevents (old cursor kept → new frames all dropped by dedup).
    #[test]
    fn a_persisted_non_zero_tag_is_never_restored_into_memory() {
        let mut v: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&sample()).expect("serialize profile"))
                .expect("JSON parse");
        v["epoch"] = serde_json::json!(7);
        let back: AgentProfile = serde_json::from_value(v).expect("deserialize");
        assert_eq!(
            back.epoch, 0,
            "the file's value must not be restored as the in-memory marker"
        );
    }

    /// The key is a leftover placeholder no reader reads — fix this test too when removing it (grounds = the `AgentProfile::epoch` comment).
    #[test]
    fn the_persisted_incarnation_tag_is_a_fixed_zero_placeholder() {
        let mut p = sample();
        p.epoch = 0xDEAD_BEEF;
        let v: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&p).expect("serialize profile"))
                .expect("JSON parse");
        assert_eq!(
            v.get("epoch"),
            Some(&serde_json::json!(0)),
            "the placeholder key is carried as 0, not as the live marker: {v}"
        );
    }

    // ── ADR-0044: output_format serde backward compatibility ─────────────────────────────────
    #[test]
    fn claude_command_without_output_format_defaults_terminal() {
        let legacy = r#"{ "kind": "Claude", "extra_args": ["--foo"] }"#;
        let cmd: AgentCommand =
            serde_json::from_str(legacy).expect("legacy claude cmd deserialize");
        assert!(
            matches!(
                &cmd,
                AgentCommand::Claude { output_format: AgentOutputFormat::Terminal, extra_args }
                    if extra_args == &vec!["--foo".to_string()]
            ),
            "output_format absent → Terminal + extra_args preserved"
        );
    }

    #[test]
    fn codex_command_without_output_format_defaults_terminal() {
        let stored = r#"{ "kind": "Codex", "extra_args": ["--foo"] }"#;
        let cmd: AgentCommand = serde_json::from_str(stored).expect("stored codex cmd deserialize");
        assert!(
            matches!(
                &cmd,
                AgentCommand::Codex { output_format: AgentOutputFormat::Terminal, extra_args }
                    if extra_args == &vec!["--foo".to_string()]
            ),
            "output_format absent → Terminal + extra_args preserved"
        );
    }

    #[test]
    fn stream_json_command_roundtrips() {
        let cmd = AgentCommand::Claude {
            extra_args: vec![],
            output_format: AgentOutputFormat::StreamJson,
        };
        let json = serde_json::to_string(&cmd).unwrap();
        let back: AgentCommand = serde_json::from_str(&json).unwrap();
        assert_eq!(cmd, back);
    }

    #[test]
    fn load_restores_existing() {
        let store = Arc::new(MemStore::default());
        {
            let reg = ProfileRegistry::new(store.clone());
            reg.upsert(sample());
        }
        let reg2 = ProfileRegistry::new(store.clone());
        assert_eq!(reg2.list().len(), 1);
    }

    // ── The two change entries · dirty (ADR-0291 R17 · D2 · E1) ─────────────────────────────

    use super::scripted_store::{Outcome, ScriptedStore, FAILURES};

    fn scripted() -> (ProfileRegistry, Arc<ScriptedStore>, AgentId) {
        let store = ScriptedStore::new();
        let reg = ProfileRegistry::new(store.clone());
        let p = sample();
        let id = p.id;
        reg.upsert(p);
        (reg, store, id)
    }

    /// Looks at both the `mutate` path (the reaper's lowering) and the `mutate_if` path (first-submission latch · boot restore's session
    ///   id clearing).
    #[test]
    fn an_internal_change_commits_and_marks_dirty_on_any_save_error() {
        for failure in FAILURES {
            let (reg, store, id) = scripted();
            let live = reg.epoch_for_spawn(id).unwrap();
            store.set(failure);

            assert!(reg.update_with(id, |p| p.auto_restore = false));
            let sid = ProfileRegistry::mint_session_id();
            assert!(reg.commit_session_id(id, live, None, sid));

            let mem = reg.get(id).unwrap();
            assert!(
                !mem.auto_restore,
                "dropping it revives the dead agent at the next boot restore — {failure:?}"
            );
            assert_eq!(
                mem.backend_session_id,
                Some(sid),
                "dropping it loses the latch's session id — {failure:?}"
            );
            assert!(reg.is_dirty(), "{failure:?}");

            assert_eq!(reg.release_session_id(id), Some(true), "{failure:?}");
            let mem = reg.get(id).unwrap();
            assert_eq!(
                mem.backend_session_id, None,
                "dropping it makes boot restore's Fresh launch holding the previous conversation's handle — {failure:?}"
            );
            assert_eq!(mem.old_session_ids.last(), Some(&sid), "{failure:?}");
            assert!(reg.is_dirty(), "{failure:?}");
        }
    }

    /// The first-turn hash record is an internal write too — a failed save keeps it in memory, dirty.
    #[test]
    fn a_failed_save_keeps_the_instructions_hash_dirty() {
        for failure in FAILURES {
            let (reg, store, id) = scripted();
            let live = reg.epoch_for_spawn(id).unwrap();
            store.set(failure);

            assert!(reg.record_instructions_hash(id, live, "h"), "{failure:?}");
            assert_eq!(
                reg.get(id)
                    .unwrap()
                    .last_applied_instructions_hash
                    .as_deref(),
                Some("h"),
                "{failure:?}"
            );
            assert!(reg.is_dirty(), "{failure:?}");
        }
    }

    #[test]
    fn a_refused_internal_change_leaves_the_file_alone_and_rides_the_first_save_after() {
        use crate::persistence::FileProfileStore;

        let dir = std::env::temp_dir().join(format!(
            "engram-profile-test-dirty-refused-{}",
            Uuid::new_v4()
        ));
        let path = dir.join(crate::persistence::PROFILES_FILE);
        let reg = ProfileRegistry::new(Arc::new(FileProfileStore::new(dir.clone())));
        let p = sample();
        let id = p.id;
        reg.upsert(p);

        let newer = r#"{"schema_version":2,"profiles":[]}"#;
        std::fs::write(&path, newer).unwrap();
        assert!(reg.update_with(id, |p| p.auto_restore = false));
        assert!(!reg.get(id).unwrap().auto_restore);
        assert!(reg.is_dirty());
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            newer,
            "during the refusal the dirty registry does not touch the newer-edition file"
        );

        std::fs::write(&path, r#"{"schema_version":1,"profiles":[]}"#).unwrap();
        assert!(reg
            .try_rename(id, Some("next change".into()))
            .expect("save succeeds"));
        assert!(!reg.is_dirty());
        let on_disk = FileProfileStore::new(dir.clone()).load();
        let saved = on_disk
            .iter()
            .find(|p| p.id == id)
            .expect("the registry was carried");
        assert!(
            !saved.auto_restore,
            "internal changes committed during the refusal were carried by the next save"
        );
        assert_eq!(saved.display_name.as_deref(), Some("next change"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_no_change_internal_write_saves_only_while_dirty() {
        let (reg, store, id) = scripted();
        let stale = reg.epoch_for_spawn(id).unwrap().wrapping_add(1);
        let sid = ProfileRegistry::mint_session_id();

        let before = store.attempts();
        assert!(!reg.commit_session_id(id, stale, None, sid));
        assert_eq!(
            store.attempts(),
            before,
            "without pending changes, a write without a change does not save"
        );

        store.set(Outcome::FailsIo);
        reg.update_with(id, |p| p.auto_restore = false);
        store.set(Outcome::Saves);
        assert!(
            !reg.commit_session_id(id, stale, None, sid),
            "this write itself is still a refusal"
        );
        assert!(!reg.is_dirty());
        assert!(
            !store.disk(id).unwrap().auto_restore,
            "the pending change did not wait for the next save with a change"
        );
    }

    #[test]
    fn a_caller_change_that_fails_to_save_leaves_memory_as_it_was() {
        for failure in FAILURES {
            let (reg, store, id) = scripted();
            let parent = sample();
            let parent_id = parent.id;
            reg.upsert(parent);
            let live = reg.epoch_for_spawn(id).unwrap();
            let sid = ProfileRegistry::mint_session_id();
            reg.commit_session_id(id, live, None, sid);
            let before = reg.get(id).unwrap();
            let mut moved = before.clone();
            moved.env = vec![("MOVED".into(), "1".into())];
            store.set(failure);

            let err = reg.try_rename(id, Some("new name".into())).unwrap_err();
            assert!(
                matches!(
                    (failure, &err),
                    (Outcome::FailsIo, StoreError::Io(_))
                        | (Outcome::Refuses(_), StoreError::Refused(_))
                ),
                "the store's `Err` is returned as is — {failure:?} → {err}"
            );
            assert!(reg.try_reparent(id, Some(parent_id)).is_err());
            assert!(reg.try_update_with(id, |p| p.auto_restore = false).is_err());
            assert!(reg.try_update_preserving_hierarchy(moved.clone()).is_err());
            assert!(reg.try_upsert_preserving_hierarchy(moved).is_err());
            assert!(reg.try_release_session_id(id).is_err());
            assert!(reg.try_upsert(sample()).is_err());
            assert!(reg.try_remove(parent_id).is_err());

            let after = reg.get(id).unwrap();
            assert_eq!(after.display_name, None, "{failure:?}");
            assert_eq!(after.parent_id, None);
            assert!(after.auto_restore);
            assert_eq!(after.env, before.env);
            assert_eq!(after.backend_session_id, Some(sid));
            assert_eq!(
                reg.list().len(),
                2,
                "create and delete were not committed either"
            );
            assert!(
                !reg.is_dirty(),
                "the failure of a change with a caller does not set dirty — memory is unchanged"
            );
        }
    }

    #[test]
    fn a_caller_error_keeps_pending_dirty_changes_and_a_caller_success_carries_them() {
        let (reg, store, id) = scripted();
        store.set(Outcome::FailsIo);
        reg.update_with(id, |p| p.auto_restore = false);

        store.set(Outcome::Refuses(Refusal::Unreadable));
        assert!(reg.try_rename(id, Some("refused".into())).is_err());
        assert!(
            reg.is_dirty(),
            "one refused request does not clear the pending internal change"
        );
        assert!(!reg.get(id).unwrap().auto_restore);

        store.set(Outcome::Saves);
        assert!(reg.try_rename(id, Some("name".into())).unwrap());
        assert!(!reg.is_dirty());
        let disk = store.disk(id).unwrap();
        assert!(
            !disk.auto_restore,
            "the copy was taken from the committed map, so the pending change is carried too"
        );
        assert_eq!(disk.display_name.as_deref(), Some("name"));
    }

    #[test]
    fn a_no_change_caller_request_carries_pending_changes_without_failing_on_them() {
        let (reg, store, id) = scripted();
        store.set(Outcome::FailsIo);
        reg.update_with(id, |p| p.auto_restore = false);
        let vanished = sample();

        assert!(
            !reg.try_update_preserving_hierarchy(vanished.clone())
                .unwrap(),
            "a request that changed nothing — the failure of the save carrying the pending change is not this request's failure"
        );
        assert!(reg.is_dirty());

        store.set(Outcome::Saves);
        assert!(!reg.try_update_preserving_hierarchy(vanished).unwrap());
        assert!(!reg.is_dirty());
        assert!(!store.disk(id).unwrap().auto_restore);
    }

    #[test]
    fn caller_changes_commit_and_persist_like_their_internal_siblings() {
        let (reg, store, id) = scripted();
        let parent = sample();
        let parent_id = parent.id;
        reg.try_upsert(parent).unwrap();
        assert!(reg.try_reparent(id, Some(parent_id)).unwrap());
        assert!(reg.try_rename(id, Some("name".into())).unwrap());
        assert!(!reg.try_rename(Uuid::new_v4(), None).unwrap());
        assert!(reg.try_update_with(id, |p| p.auto_restore = false).unwrap());
        let mut moved = reg.get(id).unwrap();
        moved.env = vec![("MOVED".into(), "1".into())];
        assert!(reg.try_update_preserving_hierarchy(moved).unwrap());
        assert!(!reg.try_update_preserving_hierarchy(sample()).unwrap());

        let live = reg.epoch_for_spawn(id).unwrap();
        let sid = ProfileRegistry::mint_session_id();
        reg.commit_session_id(id, live, None, sid);
        assert_eq!(reg.try_release_session_id(id).unwrap(), Some(true));
        assert_eq!(reg.try_release_session_id(id).unwrap(), Some(false));
        assert_eq!(reg.try_release_session_id(Uuid::new_v4()).unwrap(), None);

        let disk = store.disk(id).unwrap();
        assert_eq!(disk.parent_id, Some(parent_id));
        assert_eq!(disk.display_name.as_deref(), Some("name"));
        assert!(!disk.auto_restore);
        assert_eq!(disk.env, vec![("MOVED".to_string(), "1".to_string())]);
        assert_eq!(disk.backend_session_id, None);
        assert_eq!(disk.old_session_ids, vec![sid]);

        reg.try_remove(parent_id).unwrap();
        assert!(store.disk(parent_id).is_none());
        assert_eq!(
            store.disk(id).unwrap().parent_id,
            None,
            "children are promoted to root"
        );
        assert!(reg
            .try_update_with(id, |p| p.parent_id = Some(Uuid::new_v4()))
            .unwrap());
        assert_eq!(
            store.disk(id).unwrap().parent_id,
            None,
            "hierarchy normalization also stands on the copy"
        );

        let fresh = sample();
        let fresh_id = fresh.id;
        reg.try_upsert_preserving_hierarchy(fresh).unwrap();
        assert!(store.disk(fresh_id).is_some());
    }

    /// ★A change with a caller saves and commits under the lock it took the copy under — checked deterministically★: from the closure
    ///   running on the copy, start an internal change
    ///   on another thread. Under one lock the internal change is pushed behind that commit and survives; if the copy were taken outside
    ///   the lock, the internal change would commit first
    ///   and be overwritten by the old copy.
    #[test]
    fn a_caller_change_copies_saves_and_commits_under_one_lock() {
        let (reg, _store, id) = scripted();
        let other = sample();
        let other_id = other.id;
        reg.upsert(other);
        let reg = &reg;

        std::thread::scope(|s| {
            let (go, wait) = std::sync::mpsc::channel::<()>();
            let internal = s.spawn(move || {
                wait.recv().unwrap();
                reg.update_with(other_id, |p| p.auto_restore = false)
            });
            assert!(reg
                .try_update_with(id, |p| {
                    go.send(()).unwrap();
                    // Within one lock, the internal change waits for the lock meanwhile — outside the lock, it commits in between.
                    std::thread::sleep(std::time::Duration::from_millis(50));
                    p.display_name = Some("caller".into());
                })
                .unwrap());
            assert!(internal.join().unwrap());
        });

        assert!(
            !reg.get(other_id).unwrap().auto_restore,
            "the internal change was overwritten by the caller's old copy"
        );
        assert_eq!(reg.get(id).unwrap().display_name.as_deref(), Some("caller"));
    }

    /// ★With the two entries overlapping, no internal change is lost, and if not dirty, disk = memory★ — holds only if the caller entry's
    ///   copy → save → commit is one lock. Save results
    ///   alternate success/failure to mix both entries' branches.
    /// ★A probabilistic regression net★ — it cannot force the overlap (it caught the copy-outside-the-lock shape in 2 of 5 runs). The
    ///   deterministic pair =
    ///   `a_caller_change_copies_saves_and_commits_under_one_lock`.
    #[test]
    fn racing_caller_and_internal_changes_never_lose_an_internal_commit() {
        const ROUNDS: usize = 200;
        let store = ScriptedStore::new();
        let reg = ProfileRegistry::new(store.clone());
        let lead = sample();
        let helper = sample();
        let (lead_id, helper_id) = (lead.id, helper.id);
        reg.upsert(lead);
        reg.upsert(helper);
        let targets: Vec<AgentId> = (0..ROUNDS)
            .map(|_| {
                let p = sample();
                let id = p.id;
                reg.upsert(p);
                id
            })
            .collect();
        store.cycle(&[
            Outcome::Saves,
            Outcome::FailsIo,
            Outcome::Saves,
            Outcome::Refuses(Refusal::Unreadable),
        ]);

        // The caller thread loops without rest until the internal side finishes — so the overlapping interval covers all the internal changes.
        let internal_done = std::sync::atomic::AtomicBool::new(false);
        let start = std::sync::Barrier::new(2);
        std::thread::scope(|s| {
            s.spawn(|| {
                start.wait();
                for &id in &targets {
                    assert!(reg.update_with(id, |p| p.auto_restore = false));
                    std::thread::yield_now();
                }
                internal_done.store(true, std::sync::atomic::Ordering::SeqCst);
            });
            s.spawn(|| {
                start.wait();
                let mut i = 0usize;
                while !internal_done.load(std::sync::atomic::Ordering::SeqCst) {
                    let _ = reg.try_rename(helper_id, Some(format!("n{i}")));
                    let parent = (i % 2 == 0).then_some(lead_id);
                    let _ = reg.try_reparent(helper_id, parent);
                    i += 1;
                }
            });
        });

        let lost: Vec<AgentId> = targets
            .iter()
            .copied()
            .filter(|&id| reg.get(id).unwrap().auto_restore)
            .collect();
        assert!(
            lost.is_empty(),
            "{} internal changes vanished from memory — the caller commit overwrote them with an old copy",
            lost.len()
        );

        let as_json = |mut profiles: Vec<AgentProfile>| {
            profiles.sort_by_key(|p| p.id);
            serde_json::to_value(profiles).unwrap()
        };
        if !reg.is_dirty() {
            assert_eq!(as_json(store.disk_all()), as_json(reg.list()));
        }
        // The pending change is carried by the next successful save — one more save is triggered by an internal write without a change.
        store.set(Outcome::Saves);
        assert_eq!(reg.release_session_id(lead_id), Some(false));
        assert!(!reg.is_dirty());
        assert_eq!(
            as_json(store.disk_all()),
            as_json(reg.list()),
            "if not dirty, the last save = memory"
        );
    }

    // ── attachment read · write (TRD S25 §1-5 profile edit port · ADR-0311 · ADR-0335) ──────────────

    fn attached(paths: &[&str]) -> Vec<AttachedPreset> {
        paths
            .iter()
            .map(|p| AttachedPreset {
                path: (*p).to_string(),
            })
            .collect()
    }

    fn rename(from: &str, to: &str) -> PresetPathRename {
        PresetPathRename {
            from: from.into(),
            to: to.into(),
        }
    }

    /// Seeds one more profile with the attached paths.
    fn seed_with(reg: &ProfileRegistry, paths: &[&str]) -> AgentId {
        let mut p = sample();
        p.presets = attached(paths);
        let id = p.id;
        reg.upsert(p);
        id
    }

    fn paths_of(reg: &ProfileRegistry, id: AgentId) -> Vec<String> {
        reg.attachments_of(id)
            .unwrap()
            .presets
            .into_iter()
            .map(|p| p.path)
            .collect()
    }

    #[test]
    fn attachment_reads_mirror_the_profile_fields() {
        let (reg, _store, id) = scripted();
        let other = seed_with(&reg, &["/a.md", "engram/default"]);

        assert_eq!(
            reg.attachments_of(id),
            Some(Attachments {
                default_preset: true,
                presets: vec![],
            }),
            "just created = 「Default」 only"
        );
        assert_eq!(paths_of(&reg, other), ["/a.md", "engram/default"]);
        assert_eq!(reg.attachments_of(Uuid::new_v4()), None);

        let mut all = reg.attachments();
        all.sort_by_key(|(id, _)| *id);
        let mut expected = vec![
            (id, reg.attachments_of(id).unwrap()),
            (other, reg.attachments_of(other).unwrap()),
        ];
        expected.sort_by_key(|(id, _)| *id);
        assert_eq!(all, expected);
    }

    #[test]
    fn launch_inputs_unchanged_tells_same_changed_and_gone() {
        let (reg, store, id) = scripted();
        let seen = LaunchInputs::of(&reg.get(id).unwrap());
        let before = store.attempts();

        assert_eq!(reg.launch_inputs_unchanged(id, &seen), Some(true));
        assert_eq!(store.attempts(), before, "the comparison does not save");

        let next = Attachments {
            default_preset: false,
            presets: attached(&["/a.md"]),
        };
        assert!(reg.try_set_attachments(id, next).unwrap());
        assert_eq!(reg.launch_inputs_unchanged(id, &seen), Some(false));
        let seen = LaunchInputs::of(&reg.get(id).unwrap());
        assert_eq!(reg.launch_inputs_unchanged(id, &seen), Some(true));

        // Command · env are in the same comparison — the spawn merge moves those two from the caller copy.
        assert!(reg.update_with(id, |p| p.env.push(("ADDED".into(), "1".into()))));
        assert_eq!(reg.launch_inputs_unchanged(id, &seen), Some(false));
        let seen = LaunchInputs::of(&reg.get(id).unwrap());
        assert!(reg.update_with(id, |p| p.command = AgentCommand::Shell {
            program: "other".into(),
            args: vec![],
        }));
        assert_eq!(reg.launch_inputs_unchanged(id, &seen), Some(false));
        let seen = LaunchInputs::of(&reg.get(id).unwrap());

        // The working folder is outside the comparison (the launch reservation guards it).
        assert!(reg.update_with(id, |p| p.cwd = PathBuf::from("/elsewhere")));
        assert_eq!(reg.launch_inputs_unchanged(id, &seen), Some(true));

        reg.try_remove(id).unwrap();
        assert_eq!(
            reg.launch_inputs_unchanged(id, &seen),
            None,
            "a vanished profile is told apart from 「changed」"
        );
    }

    #[test]
    fn try_set_attachments_saves_a_change_and_reports_whether_the_agent_exists() {
        let (reg, store, id) = scripted();
        let next = Attachments {
            default_preset: false,
            presets: attached(&["/b.md", "/a.md"]),
        };

        assert!(reg.try_set_attachments(id, next.clone()).unwrap());
        let disk = store.disk(id).unwrap();
        assert!(!disk.default_preset);
        assert_eq!(disk.presets, next.presets, "attach order kept");

        let before = store.attempts();
        assert!(
            reg.try_set_attachments(id, next.clone()).unwrap(),
            "the same value is still 「present」"
        );
        assert_eq!(store.attempts(), before, "the same value is not saved");

        assert!(
            !reg.try_set_attachments(Uuid::new_v4(), next).unwrap(),
            "`false` = not in the roster"
        );
    }

    #[test]
    fn attachment_writes_that_fail_to_save_leave_memory_as_it_was() {
        for failure in FAILURES {
            let (reg, store, id) = scripted();
            let other = seed_with(&reg, &["/a.md"]);
            let before = (reg.attachments_of(id), reg.attachments_of(other));
            store.set(failure);

            let err = reg
                .try_set_attachments(
                    id,
                    Attachments {
                        default_preset: false,
                        presets: attached(&["/x.md"]),
                    },
                )
                .unwrap_err();
            assert!(
                matches!(
                    (failure, &err),
                    (Outcome::FailsIo, StoreError::Io(_))
                        | (Outcome::Refuses(_), StoreError::Refused(_))
                ),
                "returns the store's `Err` as is — {failure:?} → {err}"
            );
            assert!(reg
                .try_rewrite_preset_paths(&[rename("/a.md", "/b.md")])
                .is_err());

            assert_eq!(
                (reg.attachments_of(id), reg.attachments_of(other)),
                before,
                "{failure:?}"
            );
            assert!(!reg.is_dirty(), "{failure:?}");
        }
    }

    /// The mapping applies all at once — each line matches only original values (ADR-0335).
    #[test]
    fn rewrite_maps_every_matching_agent_once_against_the_original_values() {
        let (reg, store, untouched) = scripted();
        let a = seed_with(&reg, &["/a.md", "/x.md", "/a.md"]);
        let b = seed_with(&reg, &["/b.md"]);
        let c = seed_with(&reg, &["/c.md"]);
        let x = seed_with(&reg, &["/x.md"]);
        let before = store.attempts();

        let changed = reg
            .try_rewrite_preset_paths(&[
                rename("/a.md", "/b.md"),
                rename("/b.md", "/c.md"),
                rename("/x.md", "/x.md"),
            ])
            .unwrap();

        let mut expected = vec![a, b];
        expected.sort_unstable();
        assert_eq!(
            changed, expected,
            "changed agents only — an identity mapping (`/x.md` → `/x.md`) is not a change"
        );
        assert_eq!(
            paths_of(&reg, a),
            ["/b.md", "/x.md"],
            "`/a.md` becomes `/b.md` — it does not chain on to `/c.md` · the later duplicate is removed"
        );
        assert_eq!(paths_of(&reg, b), ["/c.md"]);
        assert_eq!(paths_of(&reg, c), ["/c.md"]);
        assert_eq!(paths_of(&reg, x), ["/x.md"]);
        assert!(paths_of(&reg, untouched).is_empty());
        assert_eq!(store.attempts(), before + 1, "the whole roster in one save");
        assert_eq!(
            store
                .disk(a)
                .unwrap()
                .presets
                .into_iter()
                .map(|p| p.path)
                .collect::<Vec<_>>(),
            ["/b.md", "/x.md"]
        );
    }

    #[test]
    fn rewrite_with_a_repeated_source_uses_its_first_line() {
        let (reg, _store, _id) = scripted();
        let a = seed_with(&reg, &["/a.md"]);

        let changed = reg
            .try_rewrite_preset_paths(&[
                rename("/a.md", "/first.md"),
                rename("/a.md", "/second.md"),
            ])
            .unwrap();

        assert_eq!(changed, vec![a]);
        assert_eq!(paths_of(&reg, a), ["/first.md"]);
    }

    #[test]
    fn rewrite_with_nothing_to_change_saves_nothing() {
        let (reg, store, _id) = scripted();
        let a = seed_with(&reg, &["/a.md"]);
        let before = store.attempts();

        assert!(reg
            .try_rewrite_preset_paths(&[rename("/nowhere.md", "/b.md")])
            .unwrap()
            .is_empty());
        assert!(reg.try_rewrite_preset_paths(&[]).unwrap().is_empty());
        assert_eq!(store.attempts(), before);
        assert_eq!(paths_of(&reg, a), ["/a.md"]);
    }

    /// Matches the whole path with platform `same_path` — no prefix match (a folder move is expanded per item by the caller).
    /// Case follows the OS rules (Windows = folds · elsewhere = case-sensitive — ADR-0325 · ADR-0335).
    #[test]
    fn rewrite_matches_the_whole_path_by_same_path() {
        let (reg, _store, _id) = scripted();
        let a = seed_with(&reg, &["/dir/a.md", "/A.md"]);
        let folds =
            engram_dashboard_platform::fs::same_path(Path::new("/A.md"), Path::new("/a.md"));

        let changed = reg
            .try_rewrite_preset_paths(&[rename("/dir", "/moved"), rename("/a.md", "/b.md")])
            .unwrap();

        if folds {
            assert_eq!(changed, vec![a]);
            assert_eq!(paths_of(&reg, a), ["/dir/a.md", "/b.md"]);
        } else {
            assert!(changed.is_empty());
            assert_eq!(paths_of(&reg, a), ["/dir/a.md", "/A.md"]);
        }
    }

    /// 「Changed」 is literal — a case-only fix of a name is a change too. Several `from` with the same key = the first line wins.
    #[test]
    fn rewrite_counts_a_case_only_rename_as_a_change() {
        let (reg, _store, _id) = scripted();
        let a = seed_with(&reg, &["/a.md"]);

        let changed = reg
            .try_rewrite_preset_paths(&[rename("/a.md", "/A.md"), rename("/A.md", "/second.md")])
            .unwrap();

        assert_eq!(changed, vec![a]);
        assert_eq!(paths_of(&reg, a), ["/A.md"]);
    }

    /// Return order = id order — not the iteration order of the roster (`HashMap`). With eight, the chance of matching by luck is 1/8!.
    #[test]
    fn rewrite_returns_the_changed_agents_in_id_order() {
        let (reg, _store, _id) = scripted();
        let mut ids: Vec<AgentId> = (0..8).map(|_| seed_with(&reg, &["/a.md"])).collect();

        let changed = reg
            .try_rewrite_preset_paths(&[rename("/a.md", "/b.md")])
            .unwrap();

        ids.sort_unstable();
        assert_eq!(changed, ids);
    }

    /// Paths that coincide after the rewrite keep only the first — the later spot of an overwrite-move is leftover.
    #[test]
    fn rewrite_keeps_only_the_first_of_paths_that_now_coincide() {
        let (reg, store, _id) = scripted();
        let ab = seed_with(&reg, &["/a.md", "/b.md"]);
        let ba = seed_with(&reg, &["/b.md", "/a.md"]);
        let axb = seed_with(&reg, &["/a.md", "/x.md", "/b.md"]);

        let changed = reg
            .try_rewrite_preset_paths(&[rename("/a.md", "/b.md")])
            .unwrap();

        let mut expected = vec![ab, ba, axb];
        expected.sort_unstable();
        assert_eq!(changed, expected);
        assert_eq!(paths_of(&reg, ab), ["/b.md"]);
        assert_eq!(paths_of(&reg, ba), ["/b.md"], "keeps the first spot");
        assert_eq!(paths_of(&reg, axb), ["/b.md", "/x.md"]);
        assert_eq!(
            store
                .disk(ab)
                .unwrap()
                .presets
                .into_iter()
                .map(|p| p.path)
                .collect::<Vec<_>>(),
            ["/b.md"]
        );
    }

    /// Coincidence is judged by `same_path` too — on Windows `/B.md`, differing only in case, equals `/b.md`, so only the first stays.
    #[test]
    fn rewrite_dedupes_by_same_path() {
        let (reg, _store, _id) = scripted();
        let a = seed_with(&reg, &["/a.md", "/B.md"]);
        let folds =
            engram_dashboard_platform::fs::same_path(Path::new("/B.md"), Path::new("/b.md"));

        let changed = reg
            .try_rewrite_preset_paths(&[rename("/a.md", "/b.md")])
            .unwrap();

        assert_eq!(changed, vec![a]);
        if folds {
            assert_eq!(paths_of(&reg, a), ["/b.md"]);
        } else {
            assert_eq!(paths_of(&reg, a), ["/b.md", "/B.md"]);
        }
    }

    /// The list of an agent not changed this time stays as is even if it already has duplicates.
    #[test]
    fn rewrite_leaves_an_untouched_agents_duplicates_alone() {
        let (reg, _store, _id) = scripted();
        let dup = seed_with(&reg, &["/d.md", "/d.md"]);
        let a = seed_with(&reg, &["/a.md"]);

        let changed = reg
            .try_rewrite_preset_paths(&[rename("/a.md", "/b.md"), rename("/d.md", "/d.md")])
            .unwrap();

        assert_eq!(changed, vec![a]);
        assert_eq!(paths_of(&reg, dup), ["/d.md", "/d.md"]);
    }

    fn strings(paths: &[&str]) -> Vec<String> {
        paths.iter().map(|p| (*p).to_string()).collect()
    }

    /// Removal spans the whole roster, and the order of the remaining items · the 「Default」 field stay (ADR-0339).
    #[test]
    fn remove_drops_matching_entries_across_agents_and_keeps_the_rest_in_order() {
        let (reg, store, untouched) = scripted();
        let a = seed_with(&reg, &["/x.md", "/a.md", "/y.md", "/a.md", "/z.md"]);
        let b = seed_with(&reg, &["/b.md", "/x.md"]);
        let c = seed_with(&reg, &["/c.md"]);
        let emptied = seed_with(&reg, &["/a.md"]);
        // With the default (true) a touched field would go unnoticed — set the two that change to false and watch them.
        assert!(reg.update_with(a, |p| p.default_preset = false));
        assert!(reg.update_with(emptied, |p| p.default_preset = false));
        let before = store.attempts();

        let changed = reg
            .try_remove_preset_paths(&strings(&["/a.md", "/b.md"]))
            .unwrap();

        let mut expected = vec![a, b, emptied];
        expected.sort_unstable();
        assert_eq!(changed, expected);
        assert_eq!(paths_of(&reg, a), ["/x.md", "/y.md", "/z.md"]);
        assert_eq!(paths_of(&reg, b), ["/x.md"]);
        assert_eq!(paths_of(&reg, c), ["/c.md"]);
        assert!(paths_of(&reg, untouched).is_empty());
        assert!(paths_of(&reg, emptied).is_empty());
        assert!(
            !reg.attachments_of(a).unwrap().default_preset,
            "the 「Default」 field stays"
        );
        assert!(
            !reg.attachments_of(emptied).unwrap().default_preset,
            "the 「Default」 field stays even with an empty list"
        );
        assert_eq!(store.attempts(), before + 1, "the whole roster in one save");
        assert_eq!(
            store
                .disk(a)
                .unwrap()
                .presets
                .into_iter()
                .map(|p| p.path)
                .collect::<Vec<_>>(),
            ["/x.md", "/y.md", "/z.md"]
        );
    }

    /// Removal does not deduplicate the remaining items — unlike a rewrite, removal creates no duplicates (ADR-0339).
    #[test]
    fn remove_keeps_surviving_duplicates() {
        let (reg, _store, _id) = scripted();
        let a = seed_with(&reg, &["/x.md", "/a.md", "/x.md"]);

        let changed = reg.try_remove_preset_paths(&strings(&["/a.md"])).unwrap();

        assert_eq!(changed, vec![a]);
        assert_eq!(paths_of(&reg, a), ["/x.md", "/x.md"]);
    }

    /// Matching uses the whole path with platform `same_path` — no prefix match, and case follows the OS rules (ADR-0325).
    #[test]
    fn remove_matches_the_whole_path_by_same_path() {
        let (reg, _store, _id) = scripted();
        let a = seed_with(&reg, &["/dir/a.md", "/A.md"]);
        let folds =
            engram_dashboard_platform::fs::same_path(Path::new("/A.md"), Path::new("/a.md"));

        let changed = reg
            .try_remove_preset_paths(&strings(&["/dir", "/a.md"]))
            .unwrap();

        if folds {
            assert_eq!(changed, vec![a]);
            assert_eq!(paths_of(&reg, a), ["/dir/a.md"]);
        } else {
            assert!(changed.is_empty());
            assert_eq!(paths_of(&reg, a), ["/dir/a.md", "/A.md"]);
        }
    }

    /// Return order = id order — not the iteration order of the roster (`HashMap`). With eight, the chance of matching by luck is 1/8!.
    #[test]
    fn remove_returns_the_changed_agents_in_id_order() {
        let (reg, _store, _id) = scripted();
        let mut ids: Vec<AgentId> = (0..8).map(|_| seed_with(&reg, &["/a.md"])).collect();

        let changed = reg.try_remove_preset_paths(&strings(&["/a.md"])).unwrap();

        ids.sort_unstable();
        assert_eq!(changed, ids);
    }

    #[test]
    fn remove_with_nothing_to_remove_saves_nothing() {
        let (reg, store, _id) = scripted();
        let a = seed_with(&reg, &["/a.md"]);
        let before = store.attempts();

        assert!(reg
            .try_remove_preset_paths(&strings(&["/nowhere.md"]))
            .unwrap()
            .is_empty());
        assert!(reg.try_remove_preset_paths(&[]).unwrap().is_empty());
        assert_eq!(store.attempts(), before);
        assert_eq!(paths_of(&reg, a), ["/a.md"]);
    }

    #[test]
    fn remove_that_fails_to_save_leaves_memory_as_it_was() {
        for failure in FAILURES {
            let (reg, store, _id) = scripted();
            let a = seed_with(&reg, &["/a.md", "/b.md"]);
            let b = seed_with(&reg, &["/a.md"]);
            let before = (reg.attachments_of(a), reg.attachments_of(b));
            store.set(failure);

            let err = reg
                .try_remove_preset_paths(&strings(&["/a.md"]))
                .unwrap_err();
            assert!(
                matches!(
                    (failure, &err),
                    (Outcome::FailsIo, StoreError::Io(_))
                        | (Outcome::Refuses(_), StoreError::Refused(_))
                ),
                "returns the store's `Err` as is — {failure:?} → {err}"
            );
            assert_eq!(
                (reg.attachments_of(a), reg.attachments_of(b)),
                before,
                "{failure:?}"
            );
            assert!(!reg.is_dirty(), "{failure:?}");
        }
    }
}
