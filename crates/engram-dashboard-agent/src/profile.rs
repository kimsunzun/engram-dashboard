//! Agent profiles — the single source of truth for restart and session restore.
//!
//! Transport- and claude-neutral: claude-only argument assembly (`--session-id` / `--resume`) lives in `backend/claude/`; here only
//!   neutral data about
//! "what to run and which session to resume". Zero tauri imports.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use engram_dashboard_base::time::now_epoch_ms;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::failure::AgentFailureKind;
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
    /// ★Do not grow the fields carelessly★: this enum is `#[serde(tag = "kind")]`, so its shape sits in `agents.json` as is — a contract
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
    Blocked { reason: String },
    /// Even fresh failed → stop. A terminal point without recursive retry (H-1.7).
    Failed { reason: String },
}

// ── Persistent profiles ────────────────────────────────────────────────────────────────

/// The persistent profile of one agent — the unit stored in `agents.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentProfile {
    /// Immutable key. This id stays for life even as processes and sessions change (the frontend subscription key).
    pub id: AgentId,
    pub name: String,

    /// User-set display name override (ADR-0061 enrichment — tree rename), **separate from `name`** (the CreateProfile name or an ad-hoc
    ///   spawn's cwd string, not a clean
    /// display name). `Some` → shown as is, `None` → derived from the cwd basename. `#[serde(default)]`: an old agents.json without it
    ///   reads as `None`.
    #[serde(default)]
    pub display_name: Option<String>,

    /// Parent profile id in the tree. `Some(pid)` → child of pid, `None` → root. **One level only**: a child cannot become a parent, and
    ///   the parent must be a root
    /// (validation: `ProfileRegistry::try_reparent`). `#[serde(default)]`: an old agents.json without it reads as `None`.
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
    ///   (empty slot). Old files without the value, or carrying an old counter, are read as is (value ignored).
    /// ★The write is not skipped — the key is sent as `0`★: the previous release declared this field **required**, so a file without the
    ///   key fails with `missing field`, that release moves it to `.corrupt-<ts>`
    ///   and the next save overwrites it with an empty list (profiles, session ids, tree parents vanish; reachable by running this build
    ///   once, then reverting). `0` = the old counter's "never respawned" state, harmless to an
    ///   old binary.
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
    /// override as the name and does not degrade to the id on a placeholder basename). The cwd must have the live notation: spawn uses
    ///   `dunce::canonicalize(profile.cwd)`, so the same normalization is done
    /// here, degrading to the original on failure like spawn.
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
        if let Some(n) = self.display_name.as_deref() {
            if !n.trim().is_empty() {
                return n.to_string();
            }
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

/// The reason the store refused a save by decision. [`crate::preset::PresetStore`] uses the same type.
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

/// Failure of [`ProfileStore::save`] · [`crate::preset::PresetStore::save`].
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
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Profiles> {
        self.profiles.lock().expect("profiles poisoned")
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

    /// Spawn-only upsert — inserts the snapshot but **keeps the live entry's values for fields whose author is the runtime**. An id that
    ///   did not exist (ad-hoc spawn) takes the snapshot as is.
    ///
    /// ★The canonical preservation list is the body below.★ One rule: **who writes that field** — fields whose author is not the caller's
    ///   copy: live wins; only fields spawn settles (cwd · command · env etc.) take the snapshot.
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

/// Placeholder for previous-release readers — always writes `0` regardless of the live marker (grounds = the `AgentProfile::epoch` comment).
fn serialize_zero_placeholder<S: serde::Serializer>(_tag: &u32, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_u32(0)
}

/// Issues one incarnation marker. ★Uses the bytes of a uuid v4 to avoid bringing in a random-number crate★ — v4 is filled by the OS CSPRNG
/// and the version/variant bits sit in bytes 6 and 8, so the first 4 bytes are fully random.
fn random_incarnation_tag() -> u32 {
    let b = Uuid::new_v4().into_bytes();
    u32::from_be_bytes([b[0], b[1], b[2], b[3]])
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
            "if this field went out to agents.json, the daemon-lifetime constraint would break — got {json}"
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

    /// ★Reproduction★: load a store that already holds a cyclic pair (a damaged/hand-edited agents.json or a legacy write bypassing
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
    /// cursor, and the new frames (numbered from 0) would be dropped by dedup and leave the slot empty. (The key itself is carried as a
    ///   `0` placeholder; see the forward-compat test below.)
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

    /// ★Even if an old counter is carried, the in-memory value does not come from the file★ — an `agents.json` left by a previous release
    ///   (upgrade → revert → upgrade) holds a
    /// nonzero counter; restoring it would cause what the test above prevents.
    #[test]
    fn a_persisted_non_zero_tag_is_never_restored_into_memory() {
        let mut v: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&sample()).expect("serialize profile"))
                .expect("JSON parse");
        v["epoch"] = serde_json::json!(7);
        let back: AgentProfile = serde_json::from_value(v).expect("deserialize");
        assert_eq!(
            back.epoch, 0,
            "the file's old counter must not be restored as the in-memory marker"
        );
    }

    /// ★The key stays and only the value is pinned to 0★ — a binary reverted to the previous release declares `epoch` **required**, so a
    ///   file with no key fails with
    /// `missing field`; that release then moves it to `.corrupt-<ts>` and the next save overwrites with an empty list (profiles, session
    ///   ids and tree parents vanish).
    /// **0, not the live marker**: 0 is the old counter's "never respawned" state, harmless to an old binary; a random number would sit in
    ///   the counter's place.
    #[test]
    fn the_persisted_incarnation_tag_is_a_fixed_zero_so_older_readers_still_parse() {
        let mut p = sample();
        p.epoch = 0xDEAD_BEEF;
        let v: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&p).expect("serialize profile"))
                .expect("JSON parse");
        assert_eq!(
            v.get("epoch"),
            Some(&serde_json::json!(0)),
            "the key that old readers read as required must be carried as 0: {v}"
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

    #[test]
    fn a_refused_internal_change_leaves_the_file_alone_and_rides_the_first_save_after() {
        use crate::persistence::FileProfileStore;

        let dir = std::env::temp_dir().join(format!(
            "engram-profile-test-dirty-refused-{}",
            Uuid::new_v4()
        ));
        let path = dir.join("agents.json");
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
            moved.cwd = PathBuf::from("C:/elsewhere");
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
            assert_eq!(after.cwd, before.cwd);
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
        moved.cwd = PathBuf::from("C:/elsewhere");
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
        assert_eq!(disk.cwd, PathBuf::from("C:/elsewhere"));
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
}
