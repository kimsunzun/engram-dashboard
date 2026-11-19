//! The profile edit port — the only way the library changes agent attachments (it never edits `ProfileRegistry` directly).
//! agent defines it and `AgentManager` implements it — roster changes and spawn reservations share one lock order inside the manager.
// ADR-0311

use crate::profile::{AttachedPreset, StoreError};
use crate::types::AgentId;

/// The presets attached to an agent — carries the profile's 「Default」 field and the attach-order list as one value.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Attachments {
    pub default_preset: bool,
    pub presets: Vec<AttachedPreset>,
}

/// One library path rewrite — changes to `to` only when the whole attached path equals `from` by platform `same_path`
/// (case by OS rules · no prefix match — a folder move is expanded per item by the caller).
// ADR-0325
// ADR-0335
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresetPathRename {
    pub from: String,
    pub to: String,
}

/// Why the 「working folder」 is locked.
// ADR-0321
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceLock {
    /// Running.
    Running,
    /// Preparing a launch (a launch holds the agent's reservation — a working-folder change holding it does not count).
    // ADR-0386
    Preparing,
    /// Conversed (a persisted backend session id exists) — never unlocks for that agent.
    Conversed,
}

impl WorkspaceLock {
    /// The answer word (`agent.presets` `workspace_lock`, the `workspace-locked` refusal's `reason`).
    pub fn word(self) -> &'static str {
        match self {
            WorkspaceLock::Running => "running",
            WorkspaceLock::Preparing => "preparing",
            WorkspaceLock::Conversed => "conversed",
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ProfileEditError {
    #[error("agent {0} not in the roster")]
    NotFound(AgentId),
    /// The save failed and nothing changed.
    #[error("profile save failed — {0}")]
    Store(#[from] StoreError),
}

/// Library → roster change port.
///
/// The caller (asset-library) may call while holding its own lock — so the implementation never calls back into the launch-material
/// port (the library) (asset-library → manager, one direction).
// ADR-0311
// ADR-0324
pub trait ProfileEditPort: Send + Sync + 'static {
    /// Replaces this agent's attachments with `next` as a whole — one roster change.
    ///
    /// ★Last write wins with no comparison★ — correct only because every writer of an existing agent's attachments queues under the
    /// library write lock (clone · `agent.new` write a new id, and the spawn merge preserves attachments).
    fn set_attachments(&self, id: AgentId, next: Attachments) -> Result<(), ProfileEditError>;

    /// Rewrites attached paths across the **current** roster per `renames` — within one hold of the roster lock. Returns = the agents
    /// that changed. Not taking an agent list is intentional — a list taken beforehand misses copies cloned in between.
    ///
    /// `renames` is one mapping applied all at once — each line matches only **original** values, and one attached path changes at
    /// most once (no chaining: with `a → b` · `b → c`, `a` becomes `b`, not `c`). Several lines with `from` equal by `same_path` = the
    /// first wins (later ones are dropped). Only agents whose value changed literally are returned, in id order — an identity mapping
    /// (`a → a`) is not a change, a case-only fix (`a → A`) is. A changed agent's list keeps only the first of paths equal by
    /// `same_path` (`[a, b]` with `a → b` = `[b]`).
    // ADR-0335
    // ADR-0339
    fn rewrite_preset_paths(
        &self,
        renames: &[PresetPathRename],
    ) -> Result<Vec<AgentId>, ProfileEditError>;

    /// Removes, across the **current** roster, attached paths equal by `same_path` to any of `paths` — within one hold of the roster
    /// lock. Returns = agents whose list shrank (id order). Why no agent list is taken = as [`Self::rewrite_preset_paths`]. The order
    /// of the remaining items and the 「Default」 field stay. Matches whole paths only (no prefix match — a folder delete is expanded per
    /// attached item by the caller).
    // ADR-0325
    // ADR-0339
    fn remove_preset_paths(&self, paths: &[String]) -> Result<Vec<AgentId>, ProfileEditError>;

    /// Whether this agent's 「working folder」 is locked — `None` = changeable. The same decision as `agent.setWorkspace`.
    /// Several reasons = only the first by `Running` > `Preparing` > `Conversed`.
    ///
    /// ★During launch preprocessing that launch's reservation is held★ — asking about the very agent
    /// [`crate::launch::LaunchKitSource::check`] is launching gets `Preparing`.
    fn workspace_lock(&self, id: AgentId) -> Result<Option<WorkspaceLock>, ProfileEditError>;
}
