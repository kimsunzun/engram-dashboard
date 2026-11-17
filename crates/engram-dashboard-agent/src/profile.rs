//! Agent profiles — the single source of truth for restart and session restore.
//!
//! This module is intentionally transport- and claude-neutral. claude-only argument assembly
//! (`--session-id` / `--resume`) is handled by `backend/claude/`, and here only neutral data about "what to run and
//! which session to resume" is kept.
//!
//! zero tauri imports — the isolation rule is kept.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use engram_dashboard_base::time::now_epoch_ms;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::failure::AgentFailureKind;
use crate::types::AgentId;

// ── Neutral run command ─────────────────────────────────────────────────────────────

/// Agent output mode — the profile axis that goes into per-backend transport and codec choice (ADR-0044).
/// `Terminal` = PTY interactive. `StreamJson` = a JSON-based non-terminal mode, and the concrete exchange
/// shape is decided by the backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum AgentOutputFormat {
    #[default]
    Terminal,
    StreamJson,
}

/// Here only the branch tag and the user's extra arguments are kept.
///
/// ★Name collision★ — `protocol::AgentCommand` means something different (the wire command sent to the daemon).
/// The wire mirror of this type is `protocol::AgentSpawnCommand`, and the frontend mirror is the type of the same name in
/// `src/api/types.ts`. Saying "AgentCommand" without the crate leaves the meaning undecided.
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
    /// codex CLI. `extra_args` are the user's extra arguments excluding the arguments the backend assembles
    /// (`--cd`·`-s`·`-a`). Stored JSON without `output_format` deserializes as `Terminal`.
    ///
    /// ★Do not grow the fields carelessly★: this enum is `#[serde(tag = "kind")]`, so the shape written here sits
    /// in `agents.json` as is — it is a contract baked onto disk, so **the less frozen, the cheaper**. Values such as the sandbox or approval mode
    /// are still at the grade of `--help` text (TRD §2), and fixing them as types now would freeze them at that grade.
    /// Having only the one field `extra_args` was also a decision for that reason (user decision 2026-09-07), and adding the mode axis,
    /// ADR-0194 updated only that place — the reason still lives.
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

/// Auto-restart policy. **Reserved — not a dead field.** The behavior is unimplemented (gated), but it is the seam for a future feature
/// that ADR-0016 states as "boot restore, guard counter and Failed persistence remain valid (to be revisited later)".
/// Keeping the field in advance saves later schema/wire migration costs (H-3).
/// ※Do not remove: it spans agent→protocol wire (domain.rs)→ts-rs bindings→daemon conversion→frontend,
/// would cause a PROTOCOL_VERSION bump, and conflicts with the "to be revisited later" intent of ADR-0016.
/// Only the "runtime auto-restart" interpretation is discarded (ADR-0019) — boot restore/guard/Failed remain valid.
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
    /// `--resume` succeeded — the existing conversation resumed as is.
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

    /// User-set display name override (ADR-0061 enrichment — tree rename). **A separate axis from the existing `name`**: `name` is
    /// the name passed at CreateProfile (claude profiles) or the cwd string of an ad-hoc spawn, so it is not "a clean display
    /// name". `Some` → shown as is, `None` → derived from the cwd basename.
    /// `#[serde(default)]`, so an old agents.json without this field is absorbed as `None` (no migration needed).
    #[serde(default)]
    pub display_name: Option<String>,

    /// The parent profile id in the tree hierarchy. `Some(pid)` → this profile is
    /// a child of pid (indented under pid in the tree), `None` → top level (root). **Only one level of nesting is allowed**: a child cannot become
    /// a parent again (a simplification to prevent cycles), and the parent must be a root — validation is in `ProfileRegistry::try_reparent`.
    /// `#[serde(default)]`, so an old agents.json without this field is absorbed as `None` (root) (no migration needed).
    // ADR-0072
    #[serde(default)]
    pub parent_id: Option<AgentId>,

    pub command: AgentCommand,

    /// ★The stored value is **raw** — it is not normalized (review fix N3 · load-bearing)★: no code canonicalizes this field.
    /// So `"."`·`".."`, symlinks and Windows paths with different case can sit as is,
    /// and normalization is done by each side that uses this value (spawn's `session.cwd`, `canonical_name_when_live`).
    pub cwd: PathBuf,

    /// ※No credentials. On persist, `*_KEY`/`*_TOKEN` patterns are warned about (persistence).
    pub env: Vec<(String, String)>,

    /// The current backend session id. **Mutable** — written right before the incarnation's first user turn goes out (ADR-0226), and when it changes via `/clear`
    /// etc. the session_tracker watcher updates it. None means there is no conversation to resume.
    ///
    /// ★The name being neutral is intended★ — **it does not say who draws the id.** claude receives the id we chose
    /// through `--session-id`, and codex tells us the id it drew itself. Either way,
    /// the meaning 「an id in that backend's session namespace」 is the same.
    /// ※Until 2026-09-07 this field was named `claude_session_id` (including disk and wire).
    ///   The grounds for having no backward-compatibility shim = `docs/tracking.md` T-33 · S21 TRD §6-1.
    pub backend_session_id: Option<Uuid>,

    /// History of past session ids discarded by fallback or clear (for audit and debugging).
    pub old_session_ids: Vec<Uuid>,

    /// ★**An opaque marker** pointing at one incarnation★ — its order has no meaning. The only question to ask is
    /// "is the output stream being read now the same as that earlier stream", and the answer is only same/different. So
    /// comparison uses **only match/mismatch** — do not derive "newer" from the ordering of two values (the value is a random number drawn anew
    /// per incarnation, so that derivation does not hold — `ProfileRegistry::epoch_for_spawn`).
    ///
    /// ★Skipping the read is part of the meaning★: an incarnation cannot outlive this daemon process, so its marker
    ///   is not revived from disk. Reviving it would have a restarted daemon put **the old marker** back on restored agents,
    ///   and an attached client would mistake it for "the same stream" and keep its old progress cursor — the new session's frames
    ///   are numbered again from 0, so they are dropped wholesale by dedup and that slot stays empty until the app is relaunched.
    ///   An old `agents.json` without the value and an `agents.json` carrying the old counter are both read as is
    ///   (the read is skipped, so that value is ignored — no migration needed).
    /// ★But the write is not skipped — the key is sent as `0`★: the previous release's struct declared this field
    ///   as **required**, so reading a file without the key breaks parsing with `missing field`. Then that
    ///   release's persistence moves the file aside to `.corrupt-<ts>` and starts with an empty list, and the next save overwrites with that empty
    ///   list, making the profiles, session ids and tree parents disappear wholesale (this actually holds on the path of running this build once and then
    ///   reverting or reinstalling). Why `0` = it is the old counter's "never respawned" state, so an old
    ///   binary trusting it as is is harmless — sending a live random number would make it sit in the counter's place.
    /// This value is not put into the frontend subscription deps — the reattach trigger is not the incarnation marker but observation of the authoritative
    /// roster (ADR-0164 decision 8).
    // ADR-0007, ADR-0163, ADR-0164
    #[serde(skip_deserializing, serialize_with = "serialize_zero_placeholder")]
    pub epoch: u32,

    /// ★The kind of the last activation failure of this entry★ — `None` = no failure record. Not a pre-decision result but
    /// **a fact observed at the place it was attempted** (ADR-0172 decision 1), and an axis separate from the current state (`AgentStatus`).
    ///
    /// ★`#[serde(skip)]` is part of the meaning (in the same vein as the read skip of `epoch` above, on different grounds)★: when the daemon is brought down,
    ///   those sessions die with it, so there is no reason to hold old failures on disk — this skip pins the lifetime
    ///   to **the daemon's lifetime** (ADR-0171 layer 3). It also exempts file format changes and backward-compatibility handling.
    /// ★The type has no serde derive at all★ — so a change deleting this skip fails to compile
    ///   (the `failure::AgentFailureKind` comment).
    /// ★There is only one writing point★: `manager::AgentManager::note_activation_result` (activation success = clear /
    ///   failure = record). That function's comment is the canonical source of 「why activation success」. Adding callers splits the causality —
    ///   the same reason as ADR-0127 pinning the turn observation cleanup points to two.
    /// ★A snapshot cannot revert it★: `upsert_preserving_hierarchy` keeps the live value (the same causality as the incarnation marker
    ///   `epoch` — the old copy the spawn caller passes must not overwrite a failure or clear recorded in between).
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

    /// ★The **canonical name this profile will have when it comes up again** (load-bearing)★ — it must be **literally the same** as the live canonical
    /// name the restored session has (if they diverge, asleep parking keyed by this value cannot find its owner).
    ///
    /// ★The same rule as a live session = `canonical_name_or_id_fallback` (+ the canonicalized cwd)★:
    ///   - It is the function `manager::resolve_canonical_name` uses for live sessions. `name::resolve_display_name` is
    ///     **not** it — that one uses an empty/whitespace-only override as the name as is and does not degrade to the id when the basename is
    ///     the placeholder (no path), so it diverges from the live name on two edges.
    ///   - The cwd must also be in the same notation as the live session: spawn uses the value of `dunce::canonicalize(profile.cwd)` as
    ///     session.cwd, while the profile-side `cwd` can hold a raw value (`.`·`..`, symlinks, Windows paths with different case).
    ///     So **the same normalization** is done here too, and if it fails (the path is already gone), it
    ///     degrades to the original just like spawn.
    /// ★There is fs access **only when there is no override** (not pure)★: canonicalize looks at the real file system —
    ///   so this verb is on the profile side, not in the pure name-derivation core (`name.rs`).
    /// ★The override shortcut is not an optimization — do not delete it★: when `display_name` is non-blank, the canonical name
    ///   **does not depend on the file system at all** (the live session rule also uses the override as is). So in that case
    ///   no syscall is made at all. Deleting it would ① make the name of a profile whose cwd directory vanished degrade to raw and
    ///   diverge from the name derived at asleep time, and ② if the cwd is a dead network share (SMB), one canonicalize blocks for tens of seconds
    ///   and that delay attaches to the caller's path.
    /// ★The remaining gap (known and accepted)★: a profile **without an override whose cwd vanished** still degrades to raw, so it can diverge from the asleep-time
    ///   name. Preventing it would require redesigning name-keyed parking itself (a new decision's concern), so it is not changed
    ///   secretly here.
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
/// ★A refusal has two variants by where it came from★ — what the caller should do differs: a load refusal clears only when the daemon is restarted, and a re-decision
/// refusal is decided again by the next save (unreadable is usually a briefly held lock, so retrying a little later works).
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
    /// The write (including the copy-aside before overwriting · the write cap) failed — the file is left as is.
    #[error("save failed — {0}")]
    Io(std::io::Error),
}

/// The current state of the store.
// ADR-0291 R17 (D6)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StoreStatus {
    #[default]
    Writable,
    /// The load decision refused — it does not save for this whole run. Restarting the daemon clears it.
    ReadOnly(Refusal),
    /// Loaded, but the re-decision of the last save refused — the next save decides again and clears it on success.
    Refusing(Refusal),
}

/// Implemented by the persistence module. Trait injection lets headless tests plug in an in-memory store.
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

/// Forcibly restores the whole map to the "valid one-level forest" invariant — called while holding the lock **right before saving and committing a change**
/// (`mutate` · `mutate_if` · `try_mutate` · `try_mutate_if`).
///
/// ★Why at the boundary and not per path★: reparent has its own validation, but upsert/update_with accept arbitrary
/// parent_ids (cycle · dangling), and spawn reinserts a stale snapshot, reviving a parent deleted in between.
/// Keeping validation only in reparent would let these paths bypass it and persist cycles and dangling references.
///
/// **idempotent**: an already-valid one-level forest (the result of a normal reparent) trips none of the rules and survives
/// as is — running it twice gives the same result.
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

/// The in-memory **single owner** of profiles. All CRUD and session id updates go through here and
/// are persisted to the store immediately on change. The responsibility for creating and updating session ids is also here (not spawn_agent — H-1.4).
///
/// **There are two change entries — split by who receives the save's `Err`:**
/// - Verbs starting with `try_` = **changes with a caller** (bus · WS · launches those two called). Applied to a copy → hierarchy
///   normalization → save → committed only on success. On `Err` (either refusal or write failure), memory is left as is and the error returned
///   — so a change believed to be 「changed」 does not vanish on restart. Do not change it into the shape of asking about the save first and then applying — a write failure
///   between asking and saving (disk full · rename lock budget exceeded) would again create 「only memory changed」.
/// - Other change verbs = **changes without a caller** (first-submission latch · file watcher · reaper · raising `auto_restore` after launching
///   · boot restore). They commit on any save `Err` and set dirty — there is no request to receive the result and revert,
///   and dropping them would lose the reaper's `auto_restore=false`, reviving dead agents at the next boot restore, and lose the latch's session
///   id. Not writing a dirty registry over a newer-edition or unreadable file is guarded by the re-decision of every save (the store).
///
/// Invariant = in-memory registry = the last successful save + (if dirty) the changes committed after it.
///
/// Lock discipline: disk IO (`store.save`) happens **while holding** the profiles lock.
/// ★Do not move save outside the lock (§5 concurrency consistency > lock-hold time)★: taking only a snapshot inside the lock and saving
/// after releasing it would, when two mutations overlap in the order "A snapshot → unlock → B snapshot → unlock → B save → A save",
/// leave the in-memory state and broadcast at the latest (B) while the disk stays stale (A), loading the old value on restart
/// (persisted ≠ observed). Under §5, when an LLM/orchestrator calls rename/create/delete
/// **programmatically, concurrently and in succession**, it actually hits this window that a human cannot open.
/// **No deadlock (unrelated to ADR-0006):** `store.save` takes only the store-state field (a leaf lock — it does no IO while held) and
/// does not re-enter the registry → the lock order is `profiles (including dirty) → store-state field` one-way, no cycle. The profiles lock is
/// also separate from the session (sessions/core/status) lock domain, so it does not tangle with that order. IO while holding the lock is usually one small file
/// read (the re-decision right before saving) + a write, and only while that file is locked does retrying make it longer, about 0.4 seconds at worst
/// (ADR-0291 · D4).
/// ★**Reopening 「let us reduce lock-hold anyway」 was closed by ADR-0207**★ — the place that *accepted* that cost is
/// ADR-0071:27 (not this comment — here only the IO cost is discussed), and 0207 reconfirmed the status quo on the grounds that ① what would be reduced has never been
/// measured and ② this save runs only on lifetime events. The conditions for reopening are there too.
// ADR-0291 R17 (D2 · E1)
pub struct ProfileRegistry {
    profiles: Mutex<Profiles>,
    store: Arc<dyn ProfileStore>,
}

type ProfileMap = HashMap<AgentId, AgentProfile>;

/// The registry and its save state — kept under one lock (every place that sets and clears them is inside a change path holding that lock).
struct Profiles {
    map: ProfileMap,
    /// The save of a committed change failed — memory may be ahead of the disk. The next successful save (from any entry — a save
    /// always writes the whole map) carries it and clears this. The `Err` of a change with a caller does not clear it (that memory is left as is).
    /// ★What remains at exit is lost — no last save is added to the exit path (current behavior as is)★.
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

    /// The common path of changes without a caller — the closure commit and save are one critical section, and it commits regardless of the save result.
    fn mutate<R>(&self, f: impl FnOnce(&mut ProfileMap) -> R) -> R {
        let mut guard = self.lock();
        let result = f(&mut guard.map);
        normalize_hierarchy(&mut guard.map);
        // ADR-0071 · ADR-0207
        self.persist(&mut guard);
        result
    }

    /// The conditional variant of `mutate` — saves only when the closure returns `true` (there was an actual change) or when dirty.
    /// ★While dirty, even calls without a change do disk IO (re-decision read + write) while holding the profiles lock★ — the price of carrying
    /// the pending change (the in-lock time of ADR-0291 D4 becomes that much more frequent).
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

    /// Saves the committed map — on failure the memory change is kept and dirty is set. The store logs the failure.
    fn persist(&self, profiles: &mut Profiles) {
        match self.store.save(&snapshot(&profiles.map)) {
            Ok(()) => profiles.dirty = false,
            Err(error) => {
                profiles.dirty = true;
                tracing::debug!(%error, "profile save failed — the memory change is kept and the next save carries it");
            }
        }
    }

    /// The common path of changes with a caller — applies to a copy and commits only if the save succeeds.
    // ADR-0291 R17
    fn try_mutate<R>(&self, f: impl FnOnce(&mut ProfileMap) -> R) -> Result<R, StoreError> {
        let mut guard = self.lock();
        let mut draft = guard.map.clone();
        let result = f(&mut draft);
        self.commit_if_saved(&mut guard, draft)?;
        Ok(result)
    }

    /// The conditional variant of `try_mutate` — if the closure returns `false`, this request changed nothing and it is `Ok(false)`.
    /// ★While dirty, even that request without a change saves the pending change while holding the profiles lock (disk IO)★ — the same as `mutate_if`.
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

    /// The copy was taken from the committed map (including pending dirty changes), so a successful save carries those too.
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

    /// Inserts into the registry or replaces wholesale — the internal entry (commits + dirty on any save `Err`). ★There is no caller on the
    ///   production path★ — only tests and the harness seam (`AgentManager::seed_agent_bypassing_uniqueness`) use it to plant the registry. Production
    ///   registration is [`ProfileRegistry::try_upsert`].
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

    /// Spawn-only upsert — inserts the snapshot but **keeps the values of the live entry already in the map for fields
    /// whose author is the runtime**. For an id that did not exist (ad-hoc spawn), the snapshot as is.
    ///
    /// ★The canonical preservation list is the body below — do not count them in this paragraph★ (the counted 「those two fields」 actually
    /// fell behind twice). There is one splitting rule: **who writes that field.** For every field whose author is not the copy the caller took,
    /// live wins, and only fields the spawn actually settles (cwd · command · env etc.) reflect the snapshot.
    ///
    /// ★Why the spawn snapshot is not used as is (sealing lost updates)★: the incoming profile is a **snapshot** taken at call
    /// time, so if another connection committed a reparent/rename in between, the old `parent_id`/`display_name`
    /// would overwrite the latest value and the concurrent edit would be lost (lost update). Those two are pure tree metadata unrelated to process startup,
    /// so there is no reason for spawn to author them, and the live entry's values are kept if it exists.
    ///
    /// ★The incarnation marker (epoch) is also kept live (ADR-0084)★: that value is pure runtime metadata unrelated to process startup,
    ///   and the **snapshot** spawn passes must not author it. Spawn newly settles the registry marker with `epoch_for_spawn` **right after** this upsert,
    ///   and inserting the old snapshot value as is here would revert the earlier settlement
    ///   (lost update), making the live session use the dead session's marker → the frontend misses the resubscription (empty slot) + turn observation and the control
    ///   token cannot tell the two generations apart. So, the same as parent_id/display_name, the live value is kept.
    // ADR-0070 ADR-0072 ADR-0084
    pub fn upsert_preserving_hierarchy(&self, profile: AgentProfile) {
        self.mutate(|m| {
            merge_preserving_live(m, profile);
        });
    }

    /// The entry with a caller — the same as [`ProfileRegistry::upsert_preserving_hierarchy`] and commits only if the save succeeds.
    pub fn try_upsert_preserving_hierarchy(&self, profile: AgentProfile) -> Result<(), StoreError> {
        self.try_mutate(|m| {
            merge_preserving_live(m, profile);
        })
    }

    /// Does the same merge but **does not create a missing id** — if present, updates it and `true`; if absent, a no-op and `false`.
    ///
    /// ★Why it exists = to prevent the **resurrection** of a deleted profile★: spawn registration branches on 「does it already exist」,
    ///   and if that lookup and the write are separate, a deletion slipping in between is ignored — after the deletion is reported as successful, that profile
    ///   **comes back to life and even its child process launches with it.** Putting the decision and the write into one critical section is the
    ///   only way to close that window, and it is the same shape as this store's other TOCTOU seals (`reparent` · `epoch_for_spawn`).
    /// ★Do not swallow `false`★ — the caller must read it as 「deleted in the meantime」 and cut the spawn.
    ///   Letting it pass would put a session without a profile onto the roster.
    /// ★**This does not close every resurrection path**★ — when the deletion finished **before registration started**, the lookup is
    ///   `None` in the first place, so it goes to the new-registration (ad-hoc spawn) branch. Telling those two apart would require the caller to bring
    ///   「this is an entry that existed」, and that signal is not in the signature of `spawn_agent` now.
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

/// The merge body of the `upsert_preserving_hierarchy` family — if there is a live entry, it guards **the fields whose author is the runtime**.
///
/// ★This function is the canonical preservation list★ — the splitting rule is 「who writes that field」, and for every field whose author is not
/// the copy the caller took, live wins. The grounds are next to each line.
fn merge_preserving_live(m: &mut HashMap<AgentId, AgentProfile>, mut profile: AgentProfile) {
    if let Some(live) = m.get(&profile.id) {
        profile.parent_id = live.parent_id;
        profile.display_name = live.display_name.clone();
        profile.epoch = live.epoch;
        // The last failure also has live win for the same reason — the only place writing that fact is activation observation,
        //   and if the old copy spawn passes reverted it, an already-cleared failure would come back on screen.
        // ADR-0172
        profile.last_failure = live.last_failure;
        // ★The backend session handle and its history also have live win (user decision)★ — **the same reason** as the four above:
        //   what writes this field is the runtime, not the spawn snapshot. For any backend, that incarnation's first-submission
        //   latch writes it **after this upsert** (ADR-0226), and the claude file watcher also writes it separately. Either way,
        //   the copy the caller took is not the author of that value.
        // ★What it prevents★: a handle that arrived after the copy was taken would be overwritten here and **persisted to disk as is**.
        //   Then the way back to that conversation would vanish — it is a loss with no way to undo, so of overwriting and
        //   keeping, the cheaper one when wrong is chosen (keeping). What remains is carrying a stale value one more time,
        //   and the next observation fixes that.
        // ★The history is kept too — the two are one lump★: the record verbs
        //   push the old value into `old_session_ids` and update them as a pair, so keeping only the new value while reverting the history
        //   to the snapshot would make handles pushed out in between vanish from the list.
        // ★A Fresh spawn does not reuse the old conversation because of this★ — spawn pushes the old
        //   value into the history and empties the field **after** this upsert, and hands a new uuid to an issuing backend (ADR-0076).
        // ★**Behavior changes for claude too — do not read this as 「something only for recording backends」**★: the Resume of an issuing-axis
        //   backend also reads this field **from the registry**, and previously the registration right before
        //   overwrote it with the snapshot so a stale value was read. Now the drift sid observed and put in by `SessionTracker`
        //   is read (that observer writes unconditionally without the incarnation axis — `observe_session_id(id, None, ..)` —
        //   and attaches only to issuing-axis backends). The direction is an improvement, but **it is not no change**.
        profile.backend_session_id = live.backend_session_id;
        profile.old_session_ids = live.old_session_ids.clone();
    }
    m.insert(profile.id, profile);
}

impl ProfileRegistry {
    /// **Promoting children to root on parent deletion (orphan-to-root):**
    /// The `parent_id` of children that pointed at the deletion target as their parent is cleared to `None` **in the same critical section** —
    /// no orphan reference pointing at a nonexistent parent (dangling parent) is left (tree render and restore invariant).
    /// It is promotion, not cascade deletion, so the children's data is preserved (user decision — prevents losing a whole group by mistake).
    ///
    /// The entry with a caller — applied to a copy and committed only if the save succeeds. A save is attempted even for a missing id.
    // ADR-0072
    // ADR-0291 R17
    pub fn try_remove(&self, id: AgentId) -> Result<(), StoreError> {
        self.try_mutate(|m| remove_promoting_children(m, id))
    }

    /// Sets/clears the display name override (ADR-0061 enrichment — tree rename). `Some(name)` → save the override, `None` →
    /// clear (back to deriving from the cwd basename). If it exists, persists after the change and true; for a missing id, no-op and false.
    /// ★Normalization is the responsibility of the save gate (`AgentManager::try_rename_agent`)★: trimming both ends and the decision "if only whitespace remains, no override"
    /// finish there **before** the name uniqueness decision (`normalize_display_name`) — trimming again here would make
    /// the value the decision saw and the value saved diverge. Only an already-normalized value or an explicit None comes here.
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

    /// Sets/clears the tree parent (ADR-0072 — hierarchy reparent). `Some(pid)` → makes child_id a child of pid,
    /// `None` → promotes to root. Does all validation inside **one critical section (try_mutate)** and only on success persists and returns true;
    /// on a violation, no-op and false (a bool return of the same shape as try_rename/try_update_with).
    ///
    /// **One-level nesting rule (cycle prevention):**
    /// - child must exist (false if not).
    /// - `Some(pid)`: ① the pid profile must exist ② `pid != child_id` (no self-parent) ③ child must not currently
    ///   be anyone's parent (a node with children cannot become a child — one-level cap) ④ the target parent pid
    ///   must itself be a root (`parent_id == None` — no two levels where a parent has a parent). false if any is violated.
    /// - `None`: always allowed if child exists (promotion to root).
    ///
    /// ★Validation inside the lock★: the existence/parenthood decision and the write are one critical section, closing the window where cycles and orphans leak
    /// through TOCTOU (state changes between check and change) with concurrent reparent/delete (via the ADR-0071 lock discipline).
    ///
    /// The entry with a caller — applied to a copy and committed only if the save succeeds. A save is attempted even on a rule violation (`Ok(false)`).
    // ADR-0072
    // ADR-0291 R17
    pub fn try_reparent(
        &self,
        child_id: AgentId,
        parent_id: Option<AgentId>,
    ) -> Result<bool, StoreError> {
        self.try_mutate(|m| reparent_one_level(m, child_id, parent_id))
    }

    /// If it exists, applies the closure and persists; otherwise false.
    pub fn update_with(&self, id: AgentId, f: impl FnOnce(&mut AgentProfile)) -> bool {
        self.mutate(|m| update_present(m, id, f))
    }

    /// The entry with a caller — the same as [`ProfileRegistry::update_with`] and commits only if the save succeeds.
    pub fn try_update_with(
        &self,
        id: AgentId,
        f: impl FnOnce(&mut AgentProfile),
    ) -> Result<bool, StoreError> {
        self.try_mutate(|m| update_present(m, id, f))
    }

    /// Records/clears 「the last failure」. `true` if the value actually changed, `false` for a missing id · epoch mismatch · same value.
    ///
    /// `incarnation`:
    /// - `Some(epoch)` — this observation is about **that incarnation**. Written only when it equals the profile's current epoch.
    /// - `None` — it failed before the incarnation was made (reservation refusal · PTY open failure). There is nothing to compare against.
    ///
    /// ★What the epoch comparison prevents (load-bearing)★: the resume observation writes **late**, by the early-exit window (currently 3s). If in between
    ///   the reaper collects that session and another connection successfully activates the same profile (epoch +1 · clear done),
    ///   the old observation waking up late **stamps the dead incarnation's failure on the live new incarnation**. It is the same shape as this store's other late-write
    ///   defenses (the session removal guard of `reaper` · `TurnObservations::forget`).
    /// ★The decision and the write must be one critical section★ — so the comparison is not left to the caller but done inside this lock
    ///   (the same reason as `epoch_for_spawn`).
    /// ★The residual of the `None` branch (known and accepted)★: there is no incarnation and so no axis to compare, so it writes unconditionally. A write of this branch
    ///   happens **right after** the observation (no wait), so the window is a few commands wide, and the direction is not the safe side either — that is, it is a hole that remains
    ///   in theory. Closing it would require failed spawns to carry their own generation too, which is a change to `spawn_agent`'s error type
    ///   and a separate matter.
    /// ★There is only one caller★: `AgentManager::note_activation_result`. Narrowing it to `pub(crate)` lets **the compiler** prevent a second write path
    ///   from appearing in the daemon or shell (within the crate, convention and review guard it —
    ///   no more than that is claimed).
    /// ★It does not write to disk — a write that does not take `mutate` (same shape = `epoch_for_spawn`)★: this field is `#[serde(skip)]`, so
    ///   saving would not change a single byte of the file content. Taking `mutate` would only add the pure cost of rewriting the whole `agents.json`
    ///   on every activation.
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

    /// Called where the session id is observed — moves the old sid into the history and replaces it with the new value,
    /// persisting immediately on change (1-b: prevents stale restore on a crash before clear→observe→persist).
    /// A call with the same value is a no-op (avoids unnecessary disk writes).
    ///
    /// `incarnation`:
    /// - `Some(epoch)` — this observation belongs to **that incarnation**. Written only when it equals the profile's current epoch.
    /// - `None` — the caller has no incarnation axis to compare. Written unconditionally.
    ///
    /// ★The `None` branch is a residual (known and accepted — precedent [`ProfileRegistry::set_last_failure`])★: what comes into that branch today
    ///   is only the file watcher ([`crate::session_tracker::SessionTracker`]), and that observer
    ///   does not carry the incarnation marker. Closing it would require widening `SessionTracker::on_change`·`WatchEntry` and fixing all their creation
    ///   points — there is **only one** production call site (the daemon assembly point) and the rest are all test rigs and
    ///   experimental bins — and **not one thing about today's claude behavior changes** with it.
    /// ★Comparison is match/mismatch only★ — "newer" is not derived from the marker (ADR-0007/0163).
    /// ★What this guard prevents is only 「a newer incarnation already stands」 — do not widen it to 「it blocks late writes」★:
    ///   none of the exit paths touches `epoch` (the reaper writes only `auto_restore`, and the link's
    ///   teardown does not touch the profile), so the marker of a session that **only ended and did not come up again** is the same as that of the live session.
    ///   An observation arriving through that window passes as is and is written to the dead session's profile.
    /// ★What `bool` means is 「the in-memory map changed」 — not 「it stayed on disk」★:
    ///   even if [`ProfileStore::save`] fails or refuses, the memory change remains (dirty — the next successful save carries it).
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
    /// Looked at in order inside the profile mutex:
    /// - the incarnation marker differs from `incarnation` → `false`
    /// - the field is already `Some(new)` → `false` — nothing to write (no disk write)
    /// - the field is `expected` → push the old value into the history, write `new` and persist, `true`
    /// - otherwise → `false` — another writer changed the field first
    ///
    /// `expected` = **the value of the field this incarnation saw when it started** (Fresh = `None`, since the field was emptied · Resume = the stored value).
    ///
    /// ★Do not change it into an overwrite ([`ProfileRegistry::observe_session_id`])★: if another writer changed the field **before** the first submission,
    ///   an overwrite would rewind that value to the value at spawn. That writer exists — the claude file
    ///   watcher writes without comparison.
    /// ★It is not 「write only into an empty field」 (the rule ADR-0216 rejected)★: `expected` is not the empty field but the value seen at the start,
    ///   so if the resume of a resuming incarnation yields an id different from the stored value, it is replaced as is.
    /// ★`bool` is in terms of the in-memory map★ — even if [`ProfileStore::save`] fails or refuses, the memory change remains (dirty).
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
    /// Kept here to leave the single issuing point in the registry. It is a new uuid every time, so the Fresh seal 「always a new sid」
    /// (ADR-0076 — 「Session ID already in use」) stays as is. Persistence is done at the first submission by
    /// [`ProfileRegistry::commit_session_id`].
    // ADR-0226
    // ADR-0076
    pub fn mint_session_id() -> Uuid {
        Uuid::new_v4()
    }

    /// The verb with which a Fresh spawn starts by emptying the field — pushes the old value into the history and persists the empty field.
    ///
    /// `None` = no profile (deleted in the meantime) · `Some(true)` = pushed · `Some(false)` = the field was already empty.
    /// ★Why `None` is told apart from 「there was nothing to push」★ — spawn must cut off when it sees the profile vanished.
    /// ★Fresh only — calling it on Resume erases the handle to resume★. The canonical call condition is `manager`'s
    ///   `fresh_spawn_release_session_id` (a crate-internal item, so written by name rather than as a link). It pushes into the history, so
    ///   it is not the destruction ADR-0202 forbids.
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

    /// The entry with a caller — the same as [`ProfileRegistry::release_session_id`] and commits only if the save succeeds.
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
    /// `None` = the profile vanished in the meantime (concurrent deletion) → **the caller must abort the spawn**.
    ///
    /// ★Why not an ordered counter★: the only question this value must answer is "is it the same stream", and order is
    ///   not needed for that. Implementing it as a counter instead raises the problem of **where that counter lives** — leaving it on disk
    ///   puts the old marker back on a new incarnation (that field's comment), and not leaving it has a restarted daemon count again from 0, so
    ///   **restored agents deterministically inherit the value of a dead incarnation**. A random number removes that determinism.
    /// ★But "different if it is a new process" is **not** a guarantee (stated honestly)★: this field is not read from disk
    ///   (`skip_deserializing`), so after a restart the comparison target of a restored profile is 0, and the value the dead daemon used remains
    ///   nowhere, so **the comparison itself does not happen**. What is actually guaranteed here is only **the adjacency guarantee** —
    ///   it differs from the immediately previous incarnation within the same process (the while loop below). A collision across processes remains at 2^-32, and
    ///   if hit, that one agent's incarnation guards are disabled for one incarnation (observation pollution · token collision · missed resubscription).
    /// ★It always differs from the immediately previous value★: every consumer guard tells two incarnations apart by match/mismatch alone (reap
    ///   epoch-guard ADR-0084 · control channel token ADR-0086 · turn observation table ADR-0113), so if the value overlapped with the
    ///   previous incarnation being drained, those guards would be disabled entirely. The probability is 2^-32, but it closes for free, so it is closed.
    /// ★Why the decision and the commit must be in one lock (load-bearing)★: split in two, a `remove` slipping in between would
    ///   plant a marker on a vanished profile, or the comparison with the previous value would be made against an old map.
    /// ★The current production call site is **only** `AgentManager::spawn_agent`★ — regardless of mode (Fresh/Resume),
    ///   only that point passes here. Scattering it per call site makes a new call site miss it again (measured: WS `Spawn` and boot restore
    ///   missed it that way), so keep the single entry point here. Not dead code (do not delete it by mistake — deleting it makes incarnation
    ///   replacement reuse the marker, causing missed frontend resubscription, observation pollution and token collisions all at once).
    /// ★It does not write to disk — it only takes the lock and replaces the marker (the same shape as `set_last_failure`)★: this field skips reads
    ///   and writes a `0` placeholder (`AgentProfile::epoch`), so saving does not change a single byte of the file.
    ///   So this verb has no save refusal or failure, and does not distinguish entries (with a caller · internal).
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

/// Rules = the [`ProfileRegistry::try_reparent`] doc.
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

/// The **common body** of the two session id record verbs that apply the incarnation guard ([`ProfileRegistry::observe_session_id`] ·
/// [`ProfileRegistry::commit_session_id`]) — the incarnation guard and history push of those two exist only here, once.
/// The unguarded clear ([`ProfileRegistry::release_session_id`]) also uses the same `retire_session_id` for the history push.
///
/// ★A new record verb must not write the guard separately without going through this helper★ — with two copies, only one gets fixed and the guards
///   diverge. That is 「making a new bypass」, which ADR-0217 decision 6 (inherited by ADR-0218 decision 6) forbids.
///
/// `admits` is called **only after passing** the guard and the 「already the same value」 check (argument = the current value of the field).
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

/// Pushes the field's value into the history (`old_session_ids`) and empties the field. `true` if there was something pushed.
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

    /// Uses the queued results once each from the front and is `then` once they run out (with `cycle`, used results are queued again at the end and it loops forever).
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

        /// The whole of the last successful save.
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
        // ★The relative path is the core of this test★: `PathBuf::from(".")` is raw, so its basename becomes `"."`, but the live
        //   session uses the basename of the canonicalized absolute path. This pins down that the two values must be the same
        //   (drawing it with `resolve_display_name(None, ".")` gives `"."` and silently diverges).
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
        // ★Review fix (D3)★: with an override, the name must not depend on fs at all.
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

    /// ★A dead incarnation's late observation cannot overwrite the live incarnation's value★ — if overwritten, that profile believes another conversation is
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

    /// Puts in one profile and settles its incarnation marker — the common starting point of the entries below.
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

    /// ★The pair measuring that it is not 「write only into an empty field」, which ADR-0216 rejected★ — if a resuming incarnation's first offer is an id
    ///   different from the stored value (the resume returned a new id), field == `expected` == stored value, so **it is replaced**. Under a rule
    ///   writing only into empty fields, it would be refused here and the handle would go stale.
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

    /// If a resuming incarnation's offer is the stored value as is, there is nothing to write — the disk is not touched.
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

    /// ★commit does not rewind the value the tracker wrote **before** the first submission★ — the tracker (claude file watcher)
    ///   writes without comparison. With an overwrite, it would be rewound here to the value at spawn and the tracker value pushed into the history.
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

    /// The paired direction — with a different marker it does not write even if the field is `expected`. Without this, the entries above are green even without the guard.
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

    /// ★The two record verbs go through **one** guard helper★ — a new verb writing the guard separately would be the bypass ADR-0217 decision 6
    ///   forbids. Running shows only 「the same result」, not 「the same code」, so it is measured in the source.
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

    /// Drawing does not touch the registry — persistence is the first submission's job.
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
        //   incarnations apart by match/mismatch alone, so if the value overlapped with the previous incarnation being drained, those guards would be disabled entirely.
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

    // ★What this test targets = the issuing is **not deterministic**★. The adjacency guarantee (test above) alone does not catch a counter
    //   regression — a counter, whether incrementing or decrementing, also always differs from the immediately previous value. The point where they differ is **a new process's
    //   first issue**: a counter hands out the same value every time (one step from 0), so a restarted daemon puts a dead
    //   incarnation's marker back as is on restored agents (the outcome = an attached client mistakes it for "the same stream"
    //   → keeps the old cursor → new frames dropped wholesale by dedup = empty slot). Only a random number lacks that determinism.
    // ★But this is a probabilistic assertion★: drawing 64 from a 32-bit space with fewer than half distinct is an event of ~1e-7 or less,
    //   effectively 0, but honestly it is a probability, not "guaranteed by construction" (the `epoch_for_spawn` paragraph is canonical for it).
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

    /// ★Late-write guard★: the resume observation writes late, by the early-exit window (3s). If in between another connection
    /// successfully activates the same profile (a new incarnation marker), the late old observation stamps the dead incarnation's failure on **the live new incarnation**.
    ///
    /// ★Markers are not written as constants★: incarnation markers are random numbers drawn anew per incarnation (ADR-0163), so there is no order
    ///   like `0 → 1`. So the old and new values are **issued and held** for the comparison, and the guard also stands on
    ///   match/mismatch only, not ordering.
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

        // A failure that ended before the incarnation has no generation to compare, so it writes unconditionally (the `None` branch of the contract).
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

    /// ★Reproduction path★: bring in by load as is a state where the store already has a cyclic pair (a damaged/hand-edited agents.json, or a legacy write
    /// bypassing reparent validation) (load does not normalize), then trigger any one write, and
    /// boundary normalization heals the cycle. **It fails without normalize_hierarchy** — both parent_ids stay
    /// and the cycle persists in memory and on disk.
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

        // Repeats to shake the interleaving widely (raises the race reproduction probability).
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
    /// This merge races with deletion: if 「is it there」 and 「write」 are separate, a deletion slipping in between is ignored, and after the deletion
    /// is reported as successful the entry comes back to the list and even its child process launches with it. Putting the decision inside the lock is
    /// the only way to close that window, and `false` is the signal for the caller to cut the spawn.
    /// ★When present, it merges as usual★ — splitting only when absent is all this verb does, so that half is measured too.
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

    /// ★A backend handle arriving **after** the copy was taken is not overwritten by spawn registration (user decision)★.
    ///
    /// What writes this field is the runtime — for any backend, that incarnation's first-submission latch writes it **after** this upsert.
    /// The copy the caller holds is not the author.
    /// ★If overwritten, it is not just a stale value but **an irreversible loss**★ — this upsert is `mutate`, so it is persisted to disk
    ///   as is, and no handle to return to that conversation remains anywhere.
    /// ★Measuring the history together is the point★ — keeping only the new value while reverting `old_session_ids` to the snapshot would make
    ///   handles pushed out in between vanish from the list, preventing only half the loss.
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

    /// ★A **live** incarnation marker must not be carried on disk★: if it were, a restarted daemon would put the old marker back
    /// on restored agents, and a surviving client would mistake it for "the same stream" and keep its old progress cursor — the new incarnation's
    /// frames are numbered again from 0, so they are dropped wholesale by dedup and that slot stays empty.
    /// (The key itself is carried — a `0` placeholder for previous-release readers. The grounds are in the forward-compat test below.)
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

    /// ★Even if an old counter is carried, the in-memory value does not come from the file★ — an `agents.json` a previous release left
    /// (the path of upgrading to this build, reverting and upgrading again) holds a nonzero counter. If that value
    /// were restored, the same outcome as what the test above prevents would happen (old cursor kept → new frames dropped wholesale by dedup).
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

    /// ★The key stays and only the value is pinned to 0★ — the struct of a binary reverted to the previous release declares `epoch`
    /// as a **required** field, so an `agents.json` with no key at all breaks parsing with `missing field`.
    /// Then that release's persistence moves the file aside to `.corrupt-<ts>` and starts with an empty list, and the next save
    /// overwrites with that empty list, making the profiles, session ids and tree parents disappear wholesale.
    /// Why the value sent is **0, not the live marker**: 0 is the old counter's "never respawned" state, so
    /// an old binary trusting it as is is harmless — sending a random number would make it sit in the counter's place.
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
        // Old wire/agents.json has no output_format field → #[serde(default)] = Terminal.
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

    /// A registry with one profile saved.
    fn scripted() -> (ProfileRegistry, Arc<ScriptedStore>, AgentId) {
        let store = ScriptedStore::new();
        let reg = ProfileRegistry::new(store.clone());
        let p = sample();
        let id = p.id;
        reg.upsert(p);
        (reg, store, id)
    }

    /// Looks at both the `mutate` path (the reaper's lowering) and the `mutate_if` path (first-submission latch · boot restore's session id clearing).
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

        // Someone put a usable file back and the refusal cleared.
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

    /// ★A change with a caller saves and commits while holding the lock it took the copy under — looked at deterministically★: inside the closure running on the copy
    ///   it starts an internal change on another thread. Within one lock, the internal change is pushed behind that commit and remains, and if it regressed to taking
    ///   the copy outside the lock, the internal change would commit first and then vanish, overwritten by the old copy.
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

    /// ★Even when the two entries overlap, no internal change is lost, and if not dirty, disk = memory★ — this holds only if the caller entry's
    ///   copy → save → commit is within one lock. Taking the copy outside the lock would let the following caller commit overwrite with the old copy
    ///   an internal change committed in between. Save results alternate between success and failure to mix the branches of the two entries.
    /// ★A probabilistic regression net★ — it cannot force the overlap, so even if it regresses into that shape it does not turn red every time (it caught the shape of taking the copy
    ///   outside the lock 2 out of 5 measured runs). The deterministic pair = `a_caller_change_copies_saves_and_commits_under_one_lock`.
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
