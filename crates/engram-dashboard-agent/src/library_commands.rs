//! `library.*`, `preset.*` and agent-preset handlers — the port asset-library implements ([`LibraryCommandHost`]) and
//! the one mapping of a port refusal onto a command error ([`refusal_error`]). The declarations are in the one
//! `declare_commands!` block of `commands.rs` (one block per module); `agent.new` judges its `presets` through the
//! same port ([`LibraryCommandHost::judge_new_presets`]) and inserts the agent only while
//! [`LibraryCommandHost::change_number`] still answers the number that judgement was made at, judging again
//! otherwise (ADR-0400 decision 18).
//!
//! ## Rules (invariants)
//!
//! - **The port speaks this crate's types.** Its methods take the spellings the caller typed and answer the declared
//!   rows or a [`PortRefusal`]; asset-library parses the spellings (a path that is not a library path refuses there,
//!   `item-path-invalid`) and converts its own refusal 1:1. Bus codes are chosen here, never there.
//! - **The port answers agents by id only.** `library.read`, `library.move` / `rename` and `library.delete` name them
//!   from one roster read and drop an id the roster no longer holds (a `CONFLICT` plan's agents carry the names the
//!   library judged them by and pass through); the agent-preset commands name the agent from the `target` resolution (an agent-preset
//!   edit never renames).
//! - **Answers are mapped as the bodies built them** — nothing here calls a second body or reads back (ADR-0384).
//! - **A refusal applied nothing** — except `library-config-write-failed`, which leaves the mount change live and
//!   unsaved; repeating the same call saves it (ADR-0381 · ADR-0400 decision 14). **It maps onto a code that does not keep the request id**, except the `INTERNAL` cases:
//!   `profile-edit-unavailable`, a profile-store write failure (ADR-0379 decisions 3–5) and `folder-listing-unanswered`
//!   (ADR-0386 decision 16 — on a `Read` command, so no id is kept).
// ADR-0379
// ADR-0384
// ADR-0386

use std::sync::Arc;

use engram_dashboard_base::path::unquote;
use engram_dashboard_command::{
    blocking_handler, CommandError, CommandTable, ErrorCode, RetryMode,
};
use serde_json::{Map, Value};

use crate::commands::{
    reject_blanks, resolve, store_command_error, store_error, store_error_reason,
    AgentAttachPresetArgs, AgentAttachPresetOk, AgentCommandHost, AgentDetachPresetArgs,
    AgentDetachPresetOk, AgentFolderRow, AgentOutputFormat, AgentPresetsArgs, AgentPresetsOk,
    AgentPresetsView, AgentRefRow, AgentReorderPresetsArgs, AgentReorderPresetsOk, AgentRosterRow,
    Blank, DeleteItemRow, GitTreeRow, LaunchNoticeRow, LibraryAddMountArgs, LibraryAddMountOk,
    LibraryCopyArgs, LibraryCopyOk, LibraryDeleteArgs, LibraryDeleteOk, LibraryEmptyTrashArgs,
    LibraryEmptyTrashOk, LibraryFoldersArgs, LibraryFoldersOk, LibraryKind, LibraryListArgs,
    LibraryListOk, LibraryListTrashArgs, LibraryListTrashOk, LibraryMoveArgs, LibraryMoveOk,
    LibraryNewFileArgs, LibraryNewFileKind, LibraryNewFileOk, LibraryNewFolderArgs,
    LibraryNewFolderOk, LibraryOnConflict, LibraryReadArgs, LibraryReadOk, LibraryRefreshArgs,
    LibraryRefreshOk, LibraryRelinkArgs, LibraryRelinkOk, LibraryRelocateMountArgs,
    LibraryRelocateMountOk, LibraryRemoveMountArgs, LibraryRemoveMountOk, LibraryRenameArgs,
    LibraryRenameMountArgs, LibraryRenameMountOk, LibraryRenameOk, LibraryRestoreArgs,
    LibraryRestoreOk, LibraryStatusArgs, LibraryStatusOk, ListRow, ListedBundleArg, MoveItemRow,
    MovePairRow, PathReasonRow, PointingRow, PresetAddArgs, PresetAddOk, PresetCheckArgs,
    PresetCheckOk, PresetEntryArg, PresetGetArgs, PresetGetOk, PresetPreviewFolderArgs,
    PresetPreviewFolderOk, PresetRemoveArgs, PresetRemoveOk, PresetReorderArgs, PresetReorderOk,
    PresetSaveArgs, PresetSaveOk, PresetSetBackendArgs, PresetSetBackendOk, PresetView,
    ProblemItemRow, ProblemRow, RelinkPairRow, RelinkStopRow, SkippedRow, TrashAddressRow,
};
use crate::launch::{escape_controls, put_reason, BlockReason, Reason};
use crate::persistence::PROFILES_FILE;
use crate::profile::{AgentProfile, StoreError};
use crate::profile_edit::Attachments;
use crate::types::AgentId;

/// The code a relink `stopped` carries when the agents' rewrite met a profile-store error (`args` = `{why}` — which
/// file did not accept and why, the words a registry-writing command answers with). A store error has no reason code
/// of its own, and `stopped` carries the refusal itself (ADR-0384 decision 8).
// ADR-0386
pub const PROFILE_STORE_REFUSED: &str = "profile-store-refused";

/// The refusal codes a caller can clear and then repeat with the same request — an IO or write failure on a library
/// file, a mount folder or `library.json` (the share comes back, the lock or permission is released), and no usable
/// library (a mount command or an IO fix clears it; the daemon's `NoLibrary` is test and harness only): they answer
/// [`RetryMode::AfterCondition`], as an unreadable profile store does (`store_command_error`). Every other refusal
/// keeps its code's default — a retry cannot fix it until the caller changes the request (a stale root generation, a
/// taken name) or the daemon restarts (a read-only `library.json`). asset-library's tests pin its consts to this list.
// ADR-0381
// ADR-0386
pub const RETRY_AFTER_CONDITION: &[&str] = &[
    "library-unavailable",
    "library-write-failed",
    "library-read-failed",
    "library-root-unusable",
    "library-config-write-failed",
];

/// The tail of a [`RETRY_AFTER_CONDITION`] refusal's message — the CLI prints the message and not the error's `retry`,
/// so the directive reaches the LLM as these words.
// ADR-0386
const RETRY_AFTER_CONDITION_TAIL: &str = "clear the cause, then repeat the same call";

/// Why a library body refused — the class picks the bus code ([`refusal_error`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PortClass {
    Conflict,
    NotFound,
    InvalidArgument,
    /// No library to answer from, or a library file or `library.json` that could not be used — `CONFLICT`.
    Unavailable,
    /// A wiring bug (`profile-edit-unavailable`) or a broken body invariant (`folder-listing-unanswered`) —
    /// `INTERNAL`, the one class that keeps the request id (on a `Write` command; a `Read` keeps none).
    Internal,
}

/// A refusal crossing the port: nothing was applied, except `library-config-write-failed` (the mount change is live but
/// not saved — ADR-0381).
#[derive(Debug)]
pub enum PortRefusal {
    Refused {
        class: PortClass,
        reason: Reason,
    },
    /// The profile store's own error, unchanged — answered as every registry-writing command answers it.
    Store(StoreError),
}

/// One item's detail as the port answers it — [`LibraryReadOk`] with the agents by id.
#[derive(Clone, Debug, PartialEq)]
pub struct LibraryReadView {
    pub row: ListRow,
    pub absolute: String,
    pub presets_direct: Vec<String>,
    pub presets_indirect: Vec<String>,
    pub agents_direct: Vec<AgentId>,
    pub agents_indirect: Vec<AgentId>,
    pub excerpt: Option<String>,
    pub settings_fields: Vec<String>,
}

/// What `preset.add` / `preset.remove` name — items or folders, never both in one call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PresetEntries {
    /// Reference spellings (a library path or `engram/…`), as typed.
    Items(Vec<String>),
    /// Folder spellings, as typed.
    Folders(Vec<String>),
}

/// A draft of a preset's lists (`preset.save` · `preset.check`), spellings as typed.
#[derive(Clone, Debug, PartialEq)]
pub struct PresetDraftInput {
    pub presets: Vec<PresetEntryArg>,
    pub instructions: Vec<PresetEntryArg>,
    pub skills: Vec<PresetEntryArg>,
    /// `None` = the file's hook list stays as it is.
    // ADR-0420
    pub hooks: Option<Vec<PresetEntryArg>>,
    pub settings: Vec<PresetEntryArg>,
    /// `None` = the file's folder list stays as it is.
    pub folders: Option<Vec<String>>,
}

/// The `agent.presets` view as the port answers it — [`AgentPresetsView`] without the agent's name, which the handler
/// adds from its `target` resolution.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(test, derive(serde::Deserialize))]
pub struct AgentPresetsPortView {
    pub agent_id: AgentId,
    pub default_preset: bool,
    pub presets: Vec<String>,
    pub cwd: String,
    pub workspace_lock: Option<String>,
    pub backend: String,
    pub mode: AgentOutputFormat,
    pub base_rev: String,
    pub takes_presets: bool,
    pub folders: Vec<AgentFolderRow>,
    pub conflicts: Vec<ProblemRow>,
    pub problems: Vec<ProblemRow>,
    pub notices: Vec<LaunchNoticeRow>,
}

/// `agent.attachPreset`'s answer as the port answers it — the view without the agent's name.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(test, derive(serde::Deserialize))]
pub struct AgentAttachPortAnswer {
    pub applied: Vec<String>,
    pub skipped: Vec<SkippedRow>,
    pub view: AgentPresetsPortView,
}

/// The `agent.new {presets}` judgement: the attachments to create the agent with, and the presets left out.
#[derive(Clone, Debug, PartialEq)]
pub struct NewAgentPresets {
    pub attachments: Attachments,
    pub skipped: Vec<SkippedRow>,
    /// The library's change number the judgement was made at, when it looked a library preset up — `agent.new`
    /// inserts only while [`LibraryCommandHost::change_number`] still answers it, and judges again otherwise
    /// (ADR-0400 decision 18); `None` = nothing to re-check.
    pub change: Option<u64>,
}

/// A move's or rename's answer as the port answers it — [`LibraryMoveOk`] with the agents by id.
// ADR-0419
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(test, derive(serde::Deserialize))]
pub struct MovePortAnswer {
    pub items: Vec<MoveItemRow>,
    pub mapping: Vec<MovePairRow>,
    pub detached: Vec<String>,
    pub written: Vec<String>,
    pub skipped: Vec<PathReasonRow>,
    pub trees: Vec<GitTreeRow>,
    /// The agents re-pointed, in id order.
    pub agents: Vec<AgentId>,
    pub stopped: Option<RelinkStopRow>,
}

/// A delete's answer as the port answers it — [`LibraryDeleteOk`] with the agents by id.
// ADR-0419
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(test, derive(serde::Deserialize))]
pub struct DeletePortAnswer {
    pub items: Vec<DeleteItemRow>,
    pub detached: Vec<String>,
    pub written: Vec<String>,
    pub skipped: Vec<PathReasonRow>,
    /// The agents whose attachments lost a deleted preset, in id order.
    pub agents: Vec<AgentId>,
    pub stopped: Option<RelinkStopRow>,
}

/// A section of a preset file — `preset.reorder`'s `section`, a lowercase word checked here (the preset file's own
/// keys and the refusal arg `list`): the declaration carries a plain string, so another word is refused by the handler
/// as INVALID_ARGUMENT.
// ADR-0400 decision 22
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PresetSection {
    Presets,
    Instructions,
    Skills,
    // ADR-0420
    Hooks,
    Settings,
    Folders,
}

impl PresetSection {
    /// Every section, in the order the words are listed.
    pub const ALL: [PresetSection; 6] = [
        PresetSection::Presets,
        PresetSection::Instructions,
        PresetSection::Skills,
        PresetSection::Hooks,
        PresetSection::Settings,
        PresetSection::Folders,
    ];

    /// Its word.
    pub fn word(self) -> &'static str {
        match self {
            PresetSection::Presets => "presets",
            PresetSection::Instructions => "instructions",
            PresetSection::Skills => "skills",
            PresetSection::Hooks => "hooks",
            PresetSection::Settings => "settings",
            PresetSection::Folders => "folders",
        }
    }

    /// The section `word` names, exactly; `None` = another word.
    pub fn parse(word: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|section| section.word() == word)
    }
}

/// `library.removeMount`'s `outcome` when the mount was removed now.
// ADR-0400 decision 14
pub const REMOVE_OUTCOME_REMOVED: &str = "removed";
/// `library.removeMount`'s `outcome` when the mount was already gone — a repeat of a removal whose save failed: only
/// library.json was written.
// ADR-0400 decision 14
pub const REMOVE_OUTCOME_SAVED_ONLY: &str = "saved_only";

/// **Everything** the `library.*`, `preset.*` and agent-preset handlers have the library do (a port) — the real thing
/// = asset-library `LibraryService` (`host.rs`); assemblies without a library plug the daemon's `NoLibrary`.
///
/// ★Synchronous, on a blocking pool thread★: the bodies wait at the library's first-scan entrance and on bounded
/// folder IO, so every handler is a `blocking_handler`.
/// ★Spellings go in as typed★ — the implementation parses them with its own path rules.
// ADR-0012
// ADR-0311
// ADR-0386
pub trait LibraryCommandHost: Send + Sync {
    fn status(&self) -> Result<LibraryStatusOk, PortRefusal>;
    /// `path` = an absolute folder; answers the status after the change and what already points at `name`.
    fn add_mount(
        &self,
        name: &str,
        path: &str,
        create: bool,
    ) -> Result<LibraryAddMountOk, PortRefusal>;
    /// Answers the status after the change, what now points at `mount`, and [`REMOVE_OUTCOME_REMOVED`] or
    /// [`REMOVE_OUTCOME_SAVED_ONLY`].
    fn remove_mount(&self, mount: &str) -> Result<LibraryRemoveMountOk, PortRefusal>;
    /// `path` = an absolute folder; answers the status after the change.
    fn relocate_mount(
        &self,
        mount: &str,
        path: &str,
        create: bool,
    ) -> Result<LibraryStatusOk, PortRefusal>;
    /// Renames the mount `mount` to `to` and re-points the references to it (`yes` confirms the plan,
    /// `root_generation` and `plan_digest` the ones it answered); answers the status after the rename and what it did
    /// (a stopped answer carries no plan digest — the repeat plans what is left and, when that plan needs
    /// confirmation, answers it to confirm, ADR-0407).
    // ADR-0400 decision 12
    fn rename_mount(
        &self,
        mount: &str,
        to: &str,
        yes: bool,
        root_generation: Option<u64>,
        plan_digest: Option<&str>,
    ) -> Result<LibraryRenameMountOk, PortRefusal>;
    /// The library's change number now (`Numbers.change` — raised by every library change, a mount rename before
    /// its agents pass). `agent.new {presets}` asks it inside the registry's insert, so an implementation takes no lock
    /// but a leaf (the library's turn mutex) and never the library lock (ADR-0400 decision 18). `None` = no library
    /// runs.
    fn change_number(&self) -> Option<u64>;
    fn folders(&self) -> Result<LibraryFoldersOk, PortRefusal>;
    /// Refuses only a `folder` that is not a library path — a failed scan is the report.
    fn refresh(&self, folder: Option<&str>) -> Result<LibraryRefreshOk, PortRefusal>;
    /// `kinds` empty = every kind.
    fn list(
        &self,
        folder: Option<&str>,
        query: Option<&str>,
        kinds: &[LibraryKind],
    ) -> Result<LibraryListOk, PortRefusal>;
    fn read(&self, path: &str) -> Result<LibraryReadView, PortRefusal>;
    /// Answers the new folder's path.
    fn new_folder(
        &self,
        parent: Option<&str>,
        name: &str,
        root_generation: Option<u64>,
    ) -> Result<String, PortRefusal>;
    /// Answers the new file's path.
    fn new_file(
        &self,
        folder: Option<&str>,
        name: &str,
        kind: LibraryNewFileKind,
        root_generation: Option<u64>,
    ) -> Result<String, PortRefusal>;
    fn relink(
        &self,
        from: &str,
        to: &str,
        root_generation: Option<u64>,
    ) -> Result<LibraryRelinkOk, PortRefusal>;
    /// `path` = a library path or a built-in (`engram/…`).
    fn preset_get(&self, path: &str) -> Result<PresetView, PortRefusal>;
    fn preset_check(
        &self,
        path: &str,
        draft: &PresetDraftInput,
    ) -> Result<PresetCheckOk, PortRefusal>;
    /// `backend` is for every item — the handler never passes one with folders.
    fn preset_add(
        &self,
        path: &str,
        entries: &PresetEntries,
        backend: Option<&str>,
        after: Option<&str>,
        root_generation: Option<u64>,
    ) -> Result<PresetAddOk, PortRefusal>;
    fn preset_remove(
        &self,
        path: &str,
        entries: &PresetEntries,
        root_generation: Option<u64>,
    ) -> Result<PresetView, PortRefusal>;
    fn preset_reorder(
        &self,
        path: &str,
        section: PresetSection,
        order: &[String],
        base_rev: &str,
        root_generation: Option<u64>,
    ) -> Result<PresetView, PortRefusal>;
    /// `backend` `None` = every backend.
    fn preset_set_backend(
        &self,
        path: &str,
        items: &[String],
        backend: Option<&str>,
        root_generation: Option<u64>,
    ) -> Result<PresetSetBackendOk, PortRefusal>;
    fn preset_save(
        &self,
        path: &str,
        draft: &PresetDraftInput,
        base_rev: Option<&str>,
        root_generation: Option<u64>,
    ) -> Result<PresetView, PortRefusal>;
    /// `backend` `None` = every registered backend; `mode` `None` = both modes.
    fn preview_folder(
        &self,
        folder: &str,
        backend: Option<&str>,
        mode: Option<AgentOutputFormat>,
    ) -> Result<PresetPreviewFolderOk, PortRefusal>;
    fn agent_presets(&self, agent: AgentId) -> Result<AgentPresetsPortView, PortRefusal>;
    /// `presets` spelled as typed; `after` = an attached preset to put them after.
    fn attach_preset(
        &self,
        agent: AgentId,
        presets: &[String],
        after: Option<&str>,
        root_generation: Option<u64>,
    ) -> Result<AgentAttachPortAnswer, PortRefusal>;
    /// `yes` confirms turning the Default slot off.
    fn detach_preset(
        &self,
        agent: AgentId,
        presets: &[String],
        yes: bool,
        root_generation: Option<u64>,
    ) -> Result<AgentPresetsPortView, PortRefusal>;
    fn reorder_presets(
        &self,
        agent: AgentId,
        order: &[String],
        base_rev: &str,
        root_generation: Option<u64>,
    ) -> Result<AgentPresetsPortView, PortRefusal>;
    /// The `agent.new {presets}` judgement: `presets` judged over `profile` (not in the registry yet) as
    /// [`LibraryCommandHost::attach_preset`] judges them, under one library read lock that also compares
    /// `root_generation` — nothing is written. Refuses as `attach_preset` does when a library preset is named and the
    /// library cannot be read, and on a stale root generation.
    // ADR-0386
    fn judge_new_presets(
        &self,
        profile: &AgentProfile,
        presets: &[String],
        root_generation: Option<u64>,
    ) -> Result<NewAgentPresets, PortRefusal>;
    /// Moves `paths` into `to_folder` (`on_conflict` `None` = a taken name is that item's conflict); `yes` /
    /// `detach_refs` / `plan_digest` answer a `skill-unconfirmed` or `hook-unconfirmed` plan. Answers per item and the
    /// reference fix.
    // ADR-0419
    #[allow(clippy::too_many_arguments)]
    fn move_items(
        &self,
        paths: &[String],
        to_folder: &str,
        on_conflict: Option<LibraryOnConflict>,
        yes: bool,
        detach_refs: bool,
        plan_digest: Option<&str>,
        root_generation: Option<u64>,
    ) -> Result<MovePortAnswer, PortRefusal>;
    /// Renames `path` to `name` in its folder — a move answered as [`LibraryCommandHost::move_items`] answers.
    // ADR-0419
    fn rename(
        &self,
        path: &str,
        name: &str,
        yes: bool,
        detach_refs: bool,
        plan_digest: Option<&str>,
        root_generation: Option<u64>,
    ) -> Result<MovePortAnswer, PortRefusal>;
    // ADR-0419
    fn copy_items(
        &self,
        paths: &[String],
        to_folder: &str,
        on_conflict: Option<LibraryOnConflict>,
        root_generation: Option<u64>,
    ) -> Result<LibraryCopyOk, PortRefusal>;
    /// `yes` / `detach_refs` / `plan_digest` answer a `delete-unconfirmed` plan.
    // ADR-0419
    fn delete(
        &self,
        paths: &[String],
        yes: bool,
        detach_refs: bool,
        plan_digest: Option<&str>,
        root_generation: Option<u64>,
    ) -> Result<DeletePortAnswer, PortRefusal>;
    // ADR-0419
    fn list_trash(&self) -> Result<LibraryListTrashOk, PortRefusal>;
    /// `keep_both` = a taken name restores under the next free keep-both name instead of answering the row's conflict.
    // ADR-0419
    fn restore(
        &self,
        rows: &[TrashAddressRow],
        keep_both: bool,
        root_generation: Option<u64>,
    ) -> Result<LibraryRestoreOk, PortRefusal>;
    /// Removes for good the rows each of `bundles` lists (empty-all) and the rows `rows` (permanent delete); neither
    /// given refuses `InvalidArgument` `nothing-to-empty`. Who may call is the daemon's gate, not the port's.
    // ADR-0408
    // ADR-0419
    fn empty_trash(
        &self,
        bundles: &[ListedBundleArg],
        rows: &[TrashAddressRow],
    ) -> Result<LibraryEmptyTrashOk, PortRefusal>;
}

/// The answers of the mount commands carry [`LibraryStatusOk`]'s fields. ★Moved by destructuring (without `..`)★ — if
/// either side gains a field, compilation stops here.
// ADR-0400 decision 7
impl From<LibraryStatusOk> for LibraryRelocateMountOk {
    fn from(status: LibraryStatusOk) -> Self {
        let LibraryStatusOk {
            daemon_id,
            mounts,
            config,
            config_why,
            suggested_new,
            root_generation,
        } = status;
        Self {
            daemon_id,
            mounts,
            config,
            config_why,
            suggested_new,
            root_generation,
        }
    }
}

impl LibraryAddMountOk {
    /// The status after the add and what already points at the name (see [`LibraryRelocateMountOk`]'s `From`).
    pub fn of(status: LibraryStatusOk, already_pointing: PointingRow) -> Self {
        let LibraryStatusOk {
            daemon_id,
            mounts,
            config,
            config_why,
            suggested_new,
            root_generation,
        } = status;
        Self {
            daemon_id,
            mounts,
            config,
            config_why,
            suggested_new,
            root_generation,
            already_pointing,
        }
    }
}

impl LibraryRenameMountOk {
    /// The status after the rename and what it did (see [`LibraryRelocateMountOk`]'s `From`).
    pub fn of(
        status: LibraryStatusOk,
        fixed: Vec<RelinkPairRow>,
        written: Vec<String>,
        skipped: Vec<PathReasonRow>,
        stopped: Option<RelinkStopRow>,
        already_pointing: PointingRow,
    ) -> Self {
        let LibraryStatusOk {
            daemon_id,
            mounts,
            config,
            config_why,
            suggested_new,
            root_generation,
        } = status;
        Self {
            daemon_id,
            mounts,
            config,
            config_why,
            suggested_new,
            root_generation,
            fixed,
            written,
            skipped,
            stopped,
            already_pointing,
        }
    }
}

impl LibraryRemoveMountOk {
    /// The status after the removal, what now points at the name and the outcome word (see
    /// [`LibraryRelocateMountOk`]'s `From`).
    pub fn of(status: LibraryStatusOk, now_absent: PointingRow, outcome: &str) -> Self {
        let LibraryStatusOk {
            daemon_id,
            mounts,
            config,
            config_why,
            suggested_new,
            root_generation,
        } = status;
        Self {
            daemon_id,
            mounts,
            config,
            config_why,
            suggested_new,
            root_generation,
            now_absent,
            outcome: outcome.to_owned(),
        }
    }
}

/// The answers of `preset.get` and of the preset edits that answer the view alone — their fields are the same as
/// [`PresetView`]. ★Moved by destructuring (without `..`)★ — if either side gains a field, compilation stops here.
macro_rules! from_preset_view {
    ($($ok:ident),* $(,)?) => {$(
        impl From<PresetView> for $ok {
            fn from(view: PresetView) -> Self {
                let PresetView {
                    path,
                    rev,
                    description,
                    presets,
                    instructions,
                    skills,
                    hooks,
                    settings,
                    folders,
                    conflicts,
                    problems,
                    notices,
                    agents,
                } = view;
                Self {
                    path,
                    rev,
                    description,
                    presets,
                    instructions,
                    skills,
                    hooks,
                    settings,
                    folders,
                    conflicts,
                    problems,
                    notices,
                    agents,
                }
            }
        }
    )*};
}

from_preset_view!(PresetGetOk, PresetRemoveOk, PresetReorderOk, PresetSaveOk);

/// The answers of `agent.presets` and of the agent-preset edits that answer the view alone — their fields are the same
/// as [`AgentPresetsView`]. ★Moved by destructuring (without `..`)★ — if either side gains a field, compilation stops
/// here.
macro_rules! from_agent_presets_view {
    ($($ok:ident),* $(,)?) => {$(
        impl From<AgentPresetsView> for $ok {
            fn from(view: AgentPresetsView) -> Self {
                let AgentPresetsView {
                    agent_id,
                    name,
                    default_preset,
                    presets,
                    cwd,
                    workspace_lock,
                    backend,
                    mode,
                    base_rev,
                    takes_presets,
                    folders,
                    conflicts,
                    problems,
                    notices,
                } = view;
                Self {
                    agent_id,
                    name,
                    default_preset,
                    presets,
                    cwd,
                    workspace_lock,
                    backend,
                    mode,
                    base_rev,
                    takes_presets,
                    folders,
                    conflicts,
                    problems,
                    notices,
                }
            }
        }
    )*};
}

from_agent_presets_view!(AgentPresetsOk, AgentDetachPresetOk, AgentReorderPresetsOk);

/// `library.rename` answers as `library.move` does. ★Moved by destructuring (without `..`)★ — if either side gains a
/// field, compilation stops here.
// ADR-0419
impl From<LibraryMoveOk> for LibraryRenameOk {
    fn from(moved: LibraryMoveOk) -> Self {
        let LibraryMoveOk {
            items,
            mapping,
            detached,
            written,
            skipped,
            trees,
            agents,
            stopped,
        } = moved;
        Self {
            items,
            mapping,
            detached,
            written,
            skipped,
            trees,
            agents,
            stopped,
        }
    }
}

/// The port's view named — `name` = the agent's name from the `target` resolution (an agent-preset edit never
/// renames; a rename by another caller in between shows the name as resolved). ★Destructured (without `..`)★ — a field added to the port view stops compilation here.
// ADR-0386
fn named(view: AgentPresetsPortView, name: String) -> AgentPresetsView {
    let AgentPresetsPortView {
        agent_id,
        default_preset,
        presets,
        cwd,
        workspace_lock,
        backend,
        mode,
        base_rev,
        takes_presets,
        folders,
        conflicts,
        problems,
        notices,
    } = view;
    AgentPresetsView {
        agent_id: agent_id.to_string(),
        name,
        default_preset,
        presets,
        cwd,
        workspace_lock,
        backend,
        mode,
        base_rev,
        takes_presets,
        folders,
        conflicts,
        problems,
        notices,
    }
}

/// A launch-block reason as the declared row — the row [`crate::launch::LaunchBlocked::to_args`] builds, with the keys
/// it leaves out sent as `null` (the command macro's `Option` has no skip).
// ADR-0386
pub fn problem_row(reason: BlockReason) -> ProblemRow {
    let (kind, item, via, items, reason) = match reason {
        BlockReason::Missing { item, via, reason } => {
            ("missing", Some(item), Some(via), None, reason)
        }
        BlockReason::Broken { item, via, reason } => {
            ("broken", Some(item), Some(via), None, reason)
        }
        BlockReason::Conflict { reason, items } => {
            let items = items
                .into_iter()
                .map(|(item, via)| ProblemItemRow { item, via })
                .collect();
            ("conflict", None, None, Some(items), reason)
        }
        BlockReason::LibraryUnavailable { reason } => {
            ("library_unavailable", None, None, None, reason)
        }
    };
    ProblemRow {
        kind: kind.to_owned(),
        item,
        via,
        items,
        code: reason.code,
        args: reason.args,
        line: reason.line,
    }
}

/// Plugs the `library.*`, `preset.*` and agent-preset handlers into `table` — the `make_table` line for the library
/// port.
pub(crate) fn insert(
    table: &mut CommandTable,
    host: &Arc<dyn AgentCommandHost>,
    library: Arc<dyn LibraryCommandHost>,
) {
    // The insert rejects are build-decided values (`make_table` doc), so they blow up at assembly.
    let status = Arc::clone(&library);
    table
        .insert(
            "library.status",
            blocking_handler(move |_: LibraryStatusArgs| verb_status(status.as_ref())),
        )
        .expect("could not plug library.status into the table");
    let add_mount = Arc::clone(&library);
    table
        .insert(
            "library.addMount",
            blocking_handler(move |args: LibraryAddMountArgs| {
                verb_add_mount(add_mount.as_ref(), args)
            }),
        )
        .expect("could not plug library.addMount into the table");
    let remove_mount = Arc::clone(&library);
    table
        .insert(
            "library.removeMount",
            blocking_handler(move |args: LibraryRemoveMountArgs| {
                verb_remove_mount(remove_mount.as_ref(), args)
            }),
        )
        .expect("could not plug library.removeMount into the table");
    let relocate_mount = Arc::clone(&library);
    table
        .insert(
            "library.relocateMount",
            blocking_handler(move |args: LibraryRelocateMountArgs| {
                verb_relocate_mount(relocate_mount.as_ref(), args)
            }),
        )
        .expect("could not plug library.relocateMount into the table");
    let rename_mount = Arc::clone(&library);
    table
        .insert(
            "library.renameMount",
            blocking_handler(move |args: LibraryRenameMountArgs| {
                verb_rename_mount(rename_mount.as_ref(), args)
            }),
        )
        .expect("could not plug library.renameMount into the table");
    let folders = Arc::clone(&library);
    table
        .insert(
            "library.folders",
            blocking_handler(move |_: LibraryFoldersArgs| verb_folders(folders.as_ref())),
        )
        .expect("could not plug library.folders into the table");
    let refresh = Arc::clone(&library);
    table
        .insert(
            "library.refresh",
            blocking_handler(move |args: LibraryRefreshArgs| verb_refresh(refresh.as_ref(), args)),
        )
        .expect("could not plug library.refresh into the table");
    let list = Arc::clone(&library);
    table
        .insert(
            "library.list",
            blocking_handler(move |args: LibraryListArgs| verb_list(list.as_ref(), args)),
        )
        .expect("could not plug library.list into the table");
    let read = (Arc::clone(host), Arc::clone(&library));
    table
        .insert(
            "library.read",
            blocking_handler(move |args: LibraryReadArgs| {
                verb_read(read.0.as_ref(), read.1.as_ref(), args)
            }),
        )
        .expect("could not plug library.read into the table");
    let new_folder = Arc::clone(&library);
    table
        .insert(
            "library.newFolder",
            blocking_handler(move |args: LibraryNewFolderArgs| {
                verb_new_folder(new_folder.as_ref(), args)
            }),
        )
        .expect("could not plug library.newFolder into the table");
    let new_file = Arc::clone(&library);
    table
        .insert(
            "library.newFile",
            blocking_handler(move |args: LibraryNewFileArgs| {
                verb_new_file(new_file.as_ref(), args)
            }),
        )
        .expect("could not plug library.newFile into the table");
    let relink = Arc::clone(&library);
    table
        .insert(
            "library.relink",
            blocking_handler(move |args: LibraryRelinkArgs| verb_relink(relink.as_ref(), args)),
        )
        .expect("could not plug library.relink into the table");
    let get = Arc::clone(&library);
    table
        .insert(
            "preset.get",
            blocking_handler(move |args: PresetGetArgs| verb_preset_get(get.as_ref(), args)),
        )
        .expect("could not plug preset.get into the table");
    let check = Arc::clone(&library);
    table
        .insert(
            "preset.check",
            blocking_handler(move |args: PresetCheckArgs| verb_preset_check(check.as_ref(), args)),
        )
        .expect("could not plug preset.check into the table");
    let add = Arc::clone(&library);
    table
        .insert(
            "preset.add",
            blocking_handler(move |args: PresetAddArgs| verb_preset_add(add.as_ref(), args)),
        )
        .expect("could not plug preset.add into the table");
    let remove = Arc::clone(&library);
    table
        .insert(
            "preset.remove",
            blocking_handler(move |args: PresetRemoveArgs| {
                verb_preset_remove(remove.as_ref(), args)
            }),
        )
        .expect("could not plug preset.remove into the table");
    let reorder = Arc::clone(&library);
    table
        .insert(
            "preset.reorder",
            blocking_handler(move |args: PresetReorderArgs| {
                verb_preset_reorder(reorder.as_ref(), args)
            }),
        )
        .expect("could not plug preset.reorder into the table");
    let set_backend = Arc::clone(&library);
    table
        .insert(
            "preset.setBackend",
            blocking_handler(move |args: PresetSetBackendArgs| {
                verb_preset_set_backend(set_backend.as_ref(), args)
            }),
        )
        .expect("could not plug preset.setBackend into the table");
    let save = Arc::clone(&library);
    table
        .insert(
            "preset.save",
            blocking_handler(move |args: PresetSaveArgs| verb_preset_save(save.as_ref(), args)),
        )
        .expect("could not plug preset.save into the table");
    let preview = Arc::clone(&library);
    table
        .insert(
            "preset.previewFolder",
            blocking_handler(move |args: PresetPreviewFolderArgs| {
                verb_preview_folder(preview.as_ref(), args)
            }),
        )
        .expect("could not plug preset.previewFolder into the table");
    let agent_presets = (Arc::clone(host), Arc::clone(&library));
    table
        .insert(
            "agent.presets",
            blocking_handler(move |args: AgentPresetsArgs| {
                verb_agent_presets(agent_presets.0.as_ref(), agent_presets.1.as_ref(), args)
            }),
        )
        .expect("could not plug agent.presets into the table");
    let attach = (Arc::clone(host), Arc::clone(&library));
    table
        .insert(
            "agent.attachPreset",
            blocking_handler(move |args: AgentAttachPresetArgs| {
                verb_attach_preset(attach.0.as_ref(), attach.1.as_ref(), args)
            }),
        )
        .expect("could not plug agent.attachPreset into the table");
    let detach = (Arc::clone(host), Arc::clone(&library));
    table
        .insert(
            "agent.detachPreset",
            blocking_handler(move |args: AgentDetachPresetArgs| {
                verb_detach_preset(detach.0.as_ref(), detach.1.as_ref(), args)
            }),
        )
        .expect("could not plug agent.detachPreset into the table");
    let reorder_presets = (Arc::clone(host), Arc::clone(&library));
    table
        .insert(
            "agent.reorderPresets",
            blocking_handler(move |args: AgentReorderPresetsArgs| {
                verb_reorder_presets(reorder_presets.0.as_ref(), reorder_presets.1.as_ref(), args)
            }),
        )
        .expect("could not plug agent.reorderPresets into the table");
    let move_items = (Arc::clone(host), Arc::clone(&library));
    table
        .insert(
            "library.move",
            blocking_handler(move |args: LibraryMoveArgs| {
                verb_move(move_items.0.as_ref(), move_items.1.as_ref(), args)
            }),
        )
        .expect("could not plug library.move into the table");
    let rename = (Arc::clone(host), Arc::clone(&library));
    table
        .insert(
            "library.rename",
            blocking_handler(move |args: LibraryRenameArgs| {
                verb_rename(rename.0.as_ref(), rename.1.as_ref(), args)
            }),
        )
        .expect("could not plug library.rename into the table");
    let copy = Arc::clone(&library);
    table
        .insert(
            "library.copy",
            blocking_handler(move |args: LibraryCopyArgs| verb_copy(copy.as_ref(), args)),
        )
        .expect("could not plug library.copy into the table");
    let delete = (Arc::clone(host), Arc::clone(&library));
    table
        .insert(
            "library.delete",
            blocking_handler(move |args: LibraryDeleteArgs| {
                verb_delete(delete.0.as_ref(), delete.1.as_ref(), args)
            }),
        )
        .expect("could not plug library.delete into the table");
    let list_trash = Arc::clone(&library);
    table
        .insert(
            "library.listTrash",
            blocking_handler(move |_: LibraryListTrashArgs| verb_list_trash(list_trash.as_ref())),
        )
        .expect("could not plug library.listTrash into the table");
    let restore = Arc::clone(&library);
    table
        .insert(
            "library.restore",
            blocking_handler(move |args: LibraryRestoreArgs| verb_restore(restore.as_ref(), args)),
        )
        .expect("could not plug library.restore into the table");
    let empty_trash = library;
    table
        .insert(
            "library.emptyTrash",
            blocking_handler(move |args: LibraryEmptyTrashArgs| {
                verb_empty_trash(empty_trash.as_ref(), args)
            }),
        )
        .expect("could not plug library.emptyTrash into the table");
}

fn verb_status(library: &dyn LibraryCommandHost) -> Result<LibraryStatusOk, CommandError> {
    library
        .status()
        .map_err(|refusal| refusal_error("library.status", refusal))
}

fn verb_add_mount(
    library: &dyn LibraryCommandHost,
    args: LibraryAddMountArgs,
) -> Result<LibraryAddMountOk, CommandError> {
    reject_blanks(&[
        ("name", Some(args.name.as_str()), Blank::NeedsValue),
        ("path", Some(args.path.as_str()), Blank::NeedsValue),
    ])?;
    // One layer of quotes off the path and nothing else: the host's folder reader takes `\`, and `\` → `/` would turn a
    //   verbatim folder (`\\?\D:\lib`, as `library.status` can answer it) into a UNC server named `?`.
    library
        .add_mount(
            &args.name,
            unquote(&args.path),
            args.create.unwrap_or(false),
        )
        .map_err(|refusal| refusal_error("library.addMount", refusal))
}

fn verb_remove_mount(
    library: &dyn LibraryCommandHost,
    args: LibraryRemoveMountArgs,
) -> Result<LibraryRemoveMountOk, CommandError> {
    reject_blanks(&[("mount", Some(args.mount.as_str()), Blank::NeedsValue)])?;
    library
        .remove_mount(&args.mount)
        .map_err(|refusal| refusal_error("library.removeMount", refusal))
}

fn verb_relocate_mount(
    library: &dyn LibraryCommandHost,
    args: LibraryRelocateMountArgs,
) -> Result<LibraryRelocateMountOk, CommandError> {
    reject_blanks(&[
        ("mount", Some(args.mount.as_str()), Blank::NeedsValue),
        ("path", Some(args.path.as_str()), Blank::NeedsValue),
    ])?;
    // The path's quotes: as `verb_add_mount`.
    library
        .relocate_mount(
            &args.mount,
            unquote(&args.path),
            args.create.unwrap_or(false),
        )
        .map(LibraryRelocateMountOk::from)
        .map_err(|refusal| refusal_error("library.relocateMount", refusal))
}

fn verb_rename_mount(
    library: &dyn LibraryCommandHost,
    args: LibraryRenameMountArgs,
) -> Result<LibraryRenameMountOk, CommandError> {
    reject_blanks(&[
        ("mount", Some(args.mount.as_str()), Blank::NeedsValue),
        ("to", Some(args.to.as_str()), Blank::NeedsValue),
    ])?;
    library
        .rename_mount(
            &args.mount,
            &args.to,
            args.yes.unwrap_or(false),
            args.root_generation,
            args.plan_digest.as_deref(),
        )
        .map_err(|refusal| refusal_error("library.renameMount", refusal))
}

fn verb_folders(library: &dyn LibraryCommandHost) -> Result<LibraryFoldersOk, CommandError> {
    library
        .folders()
        .map_err(|refusal| refusal_error("library.folders", refusal))
}

fn verb_refresh(
    library: &dyn LibraryCommandHost,
    args: LibraryRefreshArgs,
) -> Result<LibraryRefreshOk, CommandError> {
    let folder = args.folder.as_deref();
    reject_blanks(&[("folder", folder, Blank::OrOmit)])?;
    library
        .refresh(folder)
        .map_err(|refusal| refusal_error("library.refresh", refusal))
}

fn verb_list(
    library: &dyn LibraryCommandHost,
    args: LibraryListArgs,
) -> Result<LibraryListOk, CommandError> {
    let (folder, query) = (args.folder.as_deref(), args.query.as_deref());
    reject_blanks(&[
        ("folder", folder, Blank::OrOmit),
        ("query", query, Blank::OrOmit),
    ])?;
    library
        .list(folder, query, args.kinds.as_deref().unwrap_or_default())
        .map_err(|refusal| refusal_error("library.list", refusal))
}

fn verb_read(
    host: &dyn AgentCommandHost,
    library: &dyn LibraryCommandHost,
    args: LibraryReadArgs,
) -> Result<LibraryReadOk, CommandError> {
    reject_blanks(&[("path", Some(args.path.as_str()), Blank::NeedsValue)])?;
    let view = library
        .read(&args.path)
        .map_err(|refusal| refusal_error("library.read", refusal))?;
    // One roster read names both lists, so one agent never shows under two names in one answer.
    let roster = host.roster();
    Ok(LibraryReadOk {
        row: view.row,
        absolute: view.absolute,
        presets_direct: view.presets_direct,
        presets_indirect: view.presets_indirect,
        agents_direct: agent_refs(&roster, view.agents_direct),
        agents_indirect: agent_refs(&roster, view.agents_indirect),
        excerpt: view.excerpt,
        settings_fields: view.settings_fields,
    })
}

fn verb_new_folder(
    library: &dyn LibraryCommandHost,
    args: LibraryNewFolderArgs,
) -> Result<LibraryNewFolderOk, CommandError> {
    let parent = args.parent.as_deref();
    reject_blanks(&[
        ("parent", parent, Blank::OrOmit),
        ("name", Some(args.name.as_str()), Blank::NeedsValue),
    ])?;
    library
        .new_folder(parent, &args.name, args.root_generation)
        .map(|path| LibraryNewFolderOk { path })
        .map_err(|refusal| refusal_error("library.newFolder", refusal))
}

fn verb_new_file(
    library: &dyn LibraryCommandHost,
    args: LibraryNewFileArgs,
) -> Result<LibraryNewFileOk, CommandError> {
    let folder = args.folder.as_deref();
    reject_blanks(&[
        ("folder", folder, Blank::OrOmit),
        ("name", Some(args.name.as_str()), Blank::NeedsValue),
    ])?;
    library
        .new_file(folder, &args.name, args.kind, args.root_generation)
        .map(|path| LibraryNewFileOk { path })
        .map_err(|refusal| refusal_error("library.newFile", refusal))
}

fn verb_relink(
    library: &dyn LibraryCommandHost,
    args: LibraryRelinkArgs,
) -> Result<LibraryRelinkOk, CommandError> {
    reject_blanks(&[
        ("from", Some(args.from.as_str()), Blank::NeedsValue),
        ("to", Some(args.to.as_str()), Blank::NeedsValue),
    ])?;
    library
        .relink(&args.from, &args.to, args.root_generation)
        .map_err(|refusal| refusal_error("library.relink", refusal))
}

fn verb_preset_get(
    library: &dyn LibraryCommandHost,
    args: PresetGetArgs,
) -> Result<PresetGetOk, CommandError> {
    reject_blanks(&[("path", Some(args.path.as_str()), Blank::NeedsValue)])?;
    library
        .preset_get(&args.path)
        .map(PresetGetOk::from)
        .map_err(|refusal| refusal_error("preset.get", refusal))
}

fn verb_preset_check(
    library: &dyn LibraryCommandHost,
    args: PresetCheckArgs,
) -> Result<PresetCheckOk, CommandError> {
    reject_blanks(&[("path", Some(args.path.as_str()), Blank::NeedsValue)])?;
    let draft = PresetDraftInput {
        presets: args.presets,
        instructions: args.instructions,
        skills: args.skills,
        hooks: args.hooks,
        settings: args.settings,
        folders: args.folders.map(unquoted),
    };
    library
        .preset_check(&args.path, &draft)
        .map_err(|refusal| refusal_error("preset.check", refusal))
}

fn verb_preset_add(
    library: &dyn LibraryCommandHost,
    args: PresetAddArgs,
) -> Result<PresetAddOk, CommandError> {
    let (backend, after) = (args.backend.as_deref(), args.after.as_deref());
    reject_blanks(&[
        ("path", Some(args.path.as_str()), Blank::NeedsValue),
        ("backend", backend, Blank::OrOmit),
        ("after", after, Blank::OrOmit),
    ])?;
    let entries = items_or_folders("preset.add", args.items, args.folders)?;
    if matches!(entries, PresetEntries::Folders(_)) && backend.is_some() {
        return Err(CommandError::invalid_argument(
            "preset.add: `backend` goes with `items` only — a folder entry has no backend; omit it",
        ));
    }
    // ADR-0386: the anchor of a folder add is a folder spelling too — quoted, it would match no listed folder and the
    //   folders would land at the end with no word said.
    let after = match entries {
        PresetEntries::Folders(_) => after.map(unquote),
        PresetEntries::Items(_) => after,
    };
    library
        .preset_add(&args.path, &entries, backend, after, args.root_generation)
        .map_err(|refusal| refusal_error("preset.add", refusal))
}

fn verb_preset_remove(
    library: &dyn LibraryCommandHost,
    args: PresetRemoveArgs,
) -> Result<PresetRemoveOk, CommandError> {
    reject_blanks(&[("path", Some(args.path.as_str()), Blank::NeedsValue)])?;
    let entries = items_or_folders("preset.remove", args.items, args.folders)?;
    library
        .preset_remove(&args.path, &entries, args.root_generation)
        .map(PresetRemoveOk::from)
        .map_err(|refusal| refusal_error("preset.remove", refusal))
}

fn verb_preset_reorder(
    library: &dyn LibraryCommandHost,
    args: PresetReorderArgs,
) -> Result<PresetReorderOk, CommandError> {
    reject_blanks(&[
        ("path", Some(args.path.as_str()), Blank::NeedsValue),
        ("base_rev", Some(args.base_rev.as_str()), Blank::NeedsValue),
    ])?;
    // ADR-0400 decision 22: a plain string in the declaration, so the word is checked here.
    let Some(section) = PresetSection::parse(&args.section) else {
        let words: Vec<&str> = PresetSection::ALL.iter().map(|s| s.word()).collect();
        return Err(CommandError::invalid_argument(format!(
            "preset.reorder: section {:?} is not one of {}",
            escape_controls(&args.section),
            words.join(", ")
        )));
    };
    let order = match section {
        PresetSection::Folders => unquoted(args.order),
        _ => args.order,
    };
    library
        .preset_reorder(
            &args.path,
            section,
            &order,
            &args.base_rev,
            args.root_generation,
        )
        .map(PresetReorderOk::from)
        .map_err(|refusal| refusal_error("preset.reorder", refusal))
}

fn verb_preset_set_backend(
    library: &dyn LibraryCommandHost,
    args: PresetSetBackendArgs,
) -> Result<PresetSetBackendOk, CommandError> {
    let backend = args.backend.as_deref();
    reject_blanks(&[
        ("path", Some(args.path.as_str()), Blank::NeedsValue),
        ("backend", backend, Blank::OrOmit),
    ])?;
    library
        .preset_set_backend(&args.path, &args.items, backend, args.root_generation)
        .map_err(|refusal| refusal_error("preset.setBackend", refusal))
}

fn verb_preset_save(
    library: &dyn LibraryCommandHost,
    args: PresetSaveArgs,
) -> Result<PresetSaveOk, CommandError> {
    let base_rev = args.base_rev.as_deref();
    reject_blanks(&[
        ("path", Some(args.path.as_str()), Blank::NeedsValue),
        ("base_rev", base_rev, Blank::OrOmit),
    ])?;
    let draft = PresetDraftInput {
        presets: args.presets,
        instructions: args.instructions,
        skills: args.skills,
        hooks: args.hooks,
        settings: args.settings,
        folders: args.folders.map(unquoted),
    };
    library
        .preset_save(&args.path, &draft, base_rev, args.root_generation)
        .map(PresetSaveOk::from)
        .map_err(|refusal| refusal_error("preset.save", refusal))
}

fn verb_preview_folder(
    library: &dyn LibraryCommandHost,
    args: PresetPreviewFolderArgs,
) -> Result<PresetPreviewFolderOk, CommandError> {
    let backend = args.backend.as_deref();
    reject_blanks(&[
        ("folder", Some(args.folder.as_str()), Blank::NeedsValue),
        ("backend", backend, Blank::OrOmit),
    ])?;
    library
        .preview_folder(unquote(&args.folder), backend, args.mode)
        .map_err(|refusal| refusal_error("preset.previewFolder", refusal))
}

fn verb_agent_presets(
    host: &dyn AgentCommandHost,
    library: &dyn LibraryCommandHost,
    args: AgentPresetsArgs,
) -> Result<AgentPresetsOk, CommandError> {
    reject_blanks(&[("target", Some(args.target.as_str()), Blank::NeedsValue)])?;
    let agent = resolve(host, &args.target)?;
    library
        .agent_presets(agent.id)
        .map(|view| named(view, agent.name).into())
        .map_err(|refusal| refusal_error("agent.presets", refusal))
}

fn verb_attach_preset(
    host: &dyn AgentCommandHost,
    library: &dyn LibraryCommandHost,
    args: AgentAttachPresetArgs,
) -> Result<AgentAttachPresetOk, CommandError> {
    let after = args.after.as_deref();
    reject_blanks(&[
        ("target", Some(args.target.as_str()), Blank::NeedsValue),
        ("after", after, Blank::OrOmit),
    ])?;
    let agent = resolve(host, &args.target)?;
    library
        .attach_preset(agent.id, &args.presets, after, args.root_generation)
        .map(|answer| AgentAttachPresetOk {
            applied: answer.applied,
            skipped: answer.skipped,
            view: named(answer.view, agent.name),
        })
        .map_err(|refusal| refusal_error("agent.attachPreset", refusal))
}

fn verb_detach_preset(
    host: &dyn AgentCommandHost,
    library: &dyn LibraryCommandHost,
    args: AgentDetachPresetArgs,
) -> Result<AgentDetachPresetOk, CommandError> {
    reject_blanks(&[("target", Some(args.target.as_str()), Blank::NeedsValue)])?;
    let agent = resolve(host, &args.target)?;
    library
        .detach_preset(
            agent.id,
            &args.presets,
            args.yes.unwrap_or(false),
            args.root_generation,
        )
        .map(|view| named(view, agent.name).into())
        .map_err(|refusal| refusal_error("agent.detachPreset", refusal))
}

fn verb_reorder_presets(
    host: &dyn AgentCommandHost,
    library: &dyn LibraryCommandHost,
    args: AgentReorderPresetsArgs,
) -> Result<AgentReorderPresetsOk, CommandError> {
    reject_blanks(&[
        ("target", Some(args.target.as_str()), Blank::NeedsValue),
        ("base_rev", Some(args.base_rev.as_str()), Blank::NeedsValue),
    ])?;
    let agent = resolve(host, &args.target)?;
    library
        .reorder_presets(agent.id, &args.order, &args.base_rev, args.root_generation)
        .map(|view| named(view, agent.name).into())
        .map_err(|refusal| refusal_error("agent.reorderPresets", refusal))
}

fn verb_move(
    host: &dyn AgentCommandHost,
    library: &dyn LibraryCommandHost,
    args: LibraryMoveArgs,
) -> Result<LibraryMoveOk, CommandError> {
    let plan_digest = args.plan_digest.as_deref();
    reject_blanks(&[
        (
            "to_folder",
            Some(args.to_folder.as_str()),
            Blank::NeedsValue,
        ),
        ("plan_digest", plan_digest, Blank::OrOmit),
    ])?;
    let answer = library
        .move_items(
            &args.paths,
            &args.to_folder,
            args.on_conflict,
            args.yes.unwrap_or(false),
            args.detach_refs.unwrap_or(false),
            plan_digest,
            args.root_generation,
        )
        .map_err(|refusal| refusal_error("library.move", refusal))?;
    Ok(named_move(host, answer))
}

fn verb_rename(
    host: &dyn AgentCommandHost,
    library: &dyn LibraryCommandHost,
    args: LibraryRenameArgs,
) -> Result<LibraryRenameOk, CommandError> {
    let plan_digest = args.plan_digest.as_deref();
    reject_blanks(&[
        ("path", Some(args.path.as_str()), Blank::NeedsValue),
        ("name", Some(args.name.as_str()), Blank::NeedsValue),
        ("plan_digest", plan_digest, Blank::OrOmit),
    ])?;
    let answer = library
        .rename(
            &args.path,
            &args.name,
            args.yes.unwrap_or(false),
            args.detach_refs.unwrap_or(false),
            plan_digest,
            args.root_generation,
        )
        .map_err(|refusal| refusal_error("library.rename", refusal))?;
    Ok(named_move(host, answer).into())
}

fn verb_copy(
    library: &dyn LibraryCommandHost,
    args: LibraryCopyArgs,
) -> Result<LibraryCopyOk, CommandError> {
    reject_blanks(&[(
        "to_folder",
        Some(args.to_folder.as_str()),
        Blank::NeedsValue,
    )])?;
    library
        .copy_items(
            &args.paths,
            &args.to_folder,
            args.on_conflict,
            args.root_generation,
        )
        .map_err(|refusal| refusal_error("library.copy", refusal))
}

fn verb_delete(
    host: &dyn AgentCommandHost,
    library: &dyn LibraryCommandHost,
    args: LibraryDeleteArgs,
) -> Result<LibraryDeleteOk, CommandError> {
    let plan_digest = args.plan_digest.as_deref();
    reject_blanks(&[("plan_digest", plan_digest, Blank::OrOmit)])?;
    let DeletePortAnswer {
        items,
        detached,
        written,
        skipped,
        agents,
        stopped,
    } = library
        .delete(
            &args.paths,
            args.yes.unwrap_or(false),
            args.detach_refs.unwrap_or(false),
            plan_digest,
            args.root_generation,
        )
        .map_err(|refusal| refusal_error("library.delete", refusal))?;
    Ok(LibraryDeleteOk {
        items,
        detached,
        written,
        skipped,
        agents: agent_refs(&host.roster(), agents),
        stopped,
    })
}

fn verb_list_trash(library: &dyn LibraryCommandHost) -> Result<LibraryListTrashOk, CommandError> {
    library
        .list_trash()
        .map_err(|refusal| refusal_error("library.listTrash", refusal))
}

fn verb_restore(
    library: &dyn LibraryCommandHost,
    args: LibraryRestoreArgs,
) -> Result<LibraryRestoreOk, CommandError> {
    library
        .restore(
            &args.rows,
            args.keep_both.unwrap_or(false),
            args.root_generation,
        )
        .map_err(|refusal| refusal_error("library.restore", refusal))
}

fn verb_empty_trash(
    library: &dyn LibraryCommandHost,
    args: LibraryEmptyTrashArgs,
) -> Result<LibraryEmptyTrashOk, CommandError> {
    library
        .empty_trash(
            args.bundles.as_deref().unwrap_or_default(),
            args.rows.as_deref().unwrap_or_default(),
        )
        .map_err(|refusal| refusal_error("library.emptyTrash", refusal))
}

/// A move's port answer with its agents named. ★Destructured (without `..`)★ — a field added to the port answer stops
/// compilation here.
// ADR-0419
fn named_move(host: &dyn AgentCommandHost, answer: MovePortAnswer) -> LibraryMoveOk {
    let MovePortAnswer {
        items,
        mapping,
        detached,
        written,
        skipped,
        trees,
        agents,
        stopped,
    } = answer;
    LibraryMoveOk {
        items,
        mapping,
        detached,
        written,
        skipped,
        trees,
        agents: agent_refs(&host.roster(), agents),
        stopped,
    }
}

/// `ids` named from `roster` (one read), in their order — an id the roster no longer holds is dropped: there is no name
/// to give it.
// ADR-0386
fn agent_refs(roster: &[AgentRosterRow], ids: Vec<AgentId>) -> Vec<AgentRefRow> {
    ids.into_iter()
        .filter_map(|id| {
            roster
                .iter()
                .find(|row| row.id == id)
                .map(|row| AgentRefRow {
                    id: id.to_string(),
                    name: row.canonical_name.clone(),
                })
        })
        .collect()
}

/// Folder spellings with one layer of quotes off and nothing else, as `library.addMount` takes its path — an absolute
/// folder pasted from a shell often comes quoted, and a quoted spelling would match no listed folder (a remove would
/// answer success with the folder still listed). Library item spellings are never unquoted.
fn unquoted(folders: Vec<String>) -> Vec<String> {
    folders
        .into_iter()
        .map(|folder| unquote(&folder).to_owned())
        .collect()
}

/// Exactly one of `items` and `folders` — both or neither is an argument error the handler raises itself (no args,
/// like [`reject_blanks`]).
fn items_or_folders(
    command: &str,
    items: Option<Vec<String>>,
    folders: Option<Vec<String>>,
) -> Result<PresetEntries, CommandError> {
    match (items, folders) {
        (Some(items), None) => Ok(PresetEntries::Items(items)),
        (None, Some(folders)) => Ok(PresetEntries::Folders(unquoted(folders))),
        (Some(_), Some(_)) => Err(CommandError::invalid_argument(format!(
            "{command}: give `items` or `folders`, not both — make one call for each"
        ))),
        (None, None) => Err(CommandError::invalid_argument(format!(
            "{command}: give `items` (library paths) or `folders` (absolute folders)"
        ))),
    }
}

/// The one mapping of a port refusal onto a command error (ADR-0379 decision 3): `Unavailable` → `CONFLICT`, the
/// other classes onto their namesakes; `args` = `{code, args, line?}` of the reason — the keys of one `args.reasons`
/// row; the message names the command and the code. A [`RETRY_AFTER_CONDITION`] code answers `AfterCondition`, and its
/// message ends with [`RETRY_AFTER_CONDITION_TAIL`] — the CLI error body carries no `retry`. A store error answers as
/// every registry-writing command does, with no args.
// ADR-0379
// ADR-0381
// ADR-0386
pub(crate) fn refusal_error(command: &'static str, refusal: PortRefusal) -> CommandError {
    match refusal {
        PortRefusal::Refused { class, reason } => {
            let clearable = RETRY_AFTER_CONDITION.contains(&reason.code.as_str());
            coded_error(command, class, &reason, clearable)
        }
        PortRefusal::Store(error) => store_error(
            command,
            &error,
            store_command_error(
                &error,
                format!(
                    "{command}: {} — nothing was changed",
                    store_error_reason(PROFILES_FILE, &error)
                ),
            ),
        ),
    }
}

/// A coded refusal as a command error — `args` = the reason's `{code, args, line?}`, the message one English line.
/// `clearable` = a retry of the same call can succeed once the cause clears: [`RetryMode::AfterCondition`] and the
/// words saying so. [`refusal_error`] decides it by code ([`RETRY_AFTER_CONDITION`]); a caller whose code is clearable
/// for some arguments only decides it itself (`agent.setWorkspace`).
// ADR-0386
pub(crate) fn coded_error(
    command: &str,
    class: PortClass,
    reason: &Reason,
    clearable: bool,
) -> CommandError {
    let code = match class {
        PortClass::Conflict | PortClass::Unavailable => ErrorCode::Conflict,
        PortClass::NotFound => ErrorCode::NotFound,
        PortClass::InvalidArgument => ErrorCode::InvalidArgument,
        PortClass::Internal => ErrorCode::Internal,
    };
    let mut args = Map::new();
    put_reason(&mut args, reason);
    let message = refusal_message(command, reason);
    let error = match clearable {
        true => CommandError::with_retry(
            code,
            format!("{message}; {RETRY_AFTER_CONDITION_TAIL}"),
            RetryMode::AfterCondition,
        ),
        false => CommandError::of(code, message),
    };
    error.with_args(args)
}

/// A `stopped` row of a reference fix (a relink's, or a mount rename's, move's or delete's): the refusal it met, carried
/// as a refusal would carry it — a store error as [`PROFILE_STORE_REFUSED`] —, its message naming `command`.
// ADR-0384
// ADR-0386
// ADR-0419
pub fn relink_stop_row(command: &str, path: Option<String>, refusal: PortRefusal) -> RelinkStopRow {
    let reason = match refusal {
        PortRefusal::Refused { reason, .. } => reason,
        PortRefusal::Store(error) => Reason {
            code: PROFILE_STORE_REFUSED.to_owned(),
            args: serde_json::json!({ "why": store_error_reason(PROFILES_FILE, &error) }),
            line: None,
        },
    };
    RelinkStopRow {
        path,
        message: refusal_message(command, &reason),
        code: reason.code,
        args: reason.args,
    }
}

/// `<command>: <code>` plus the reason's plain arguments and line — one readable English line; `args` carries them
/// as data.
fn refusal_message(command: &str, reason: &Reason) -> String {
    let mut details: Vec<String> = reason
        .args
        .as_object()
        .into_iter()
        .flatten()
        .filter_map(|(key, value)| match value {
            Value::String(text) => Some(format!("{key}: {}", escape_controls(text))),
            Value::Number(number) => Some(format!("{key}: {number}")),
            Value::Bool(flag) => Some(format!("{key}: {flag}")),
            _ => None,
        })
        .collect();
    if let Some(line) = reason.line {
        details.push(format!("line: {line}"));
    }
    match details.is_empty() {
        true => format!("{command}: {}", reason.code),
        false => format!("{command}: {} — {}", reason.code, details.join(", ")),
    }
}

#[cfg(test)]
pub(crate) mod fake {
    //! A scripted [`LibraryCommandHost`] — records each call with the values it was given and answers what the test
    //! set (default: every call refuses `library-unavailable`, `refresh` answers an empty report).

    use std::sync::Mutex;

    use super::*;
    use crate::commands::{LibraryStatusOk, ListRow};

    /// One recorded call: the method and its arguments, as a value a test compares.
    pub(crate) type Call = (&'static str, Value);

    #[derive(Default)]
    pub(crate) struct FakeLibrary {
        pub(crate) calls: Mutex<Vec<Call>>,
        /// The next answer of any method — `Ok(value)` is deserialized into that method's answer type.
        pub(crate) next: Mutex<Option<Result<Value, PortRefusal>>>,
        /// The answers after `next`, in order.
        pub(crate) then: Mutex<std::collections::VecDeque<Result<Value, PortRefusal>>>,
        /// What `change_number` answers, one per call in order; empty = `None`.
        pub(crate) change_numbers: Mutex<std::collections::VecDeque<u64>>,
    }

    impl FakeLibrary {
        /// A port for tables whose tests do not reach the library.
        pub(crate) fn unplugged() -> Arc<dyn LibraryCommandHost> {
            Arc::new(FakeLibrary::default())
        }

        pub(crate) fn answering(answer: Result<Value, PortRefusal>) -> Arc<Self> {
            let fake = FakeLibrary::default();
            *fake.next.lock().unwrap() = Some(answer);
            Arc::new(fake)
        }

        pub(crate) fn calls(&self) -> Vec<Call> {
            self.calls.lock().unwrap().clone()
        }

        fn answer<T: serde::de::DeserializeOwned>(
            &self,
            method: &'static str,
            args: Value,
        ) -> Result<T, PortRefusal> {
            self.calls.lock().unwrap().push((method, args));
            let next = self.next.lock().unwrap().take();
            let next = next.or_else(|| self.then.lock().unwrap().pop_front());
            match next {
                Some(Ok(value)) => {
                    Ok(serde_json::from_value(value).expect("scripted answer shape"))
                }
                Some(Err(refusal)) => Err(refusal),
                None => Err(PortRefusal::Refused {
                    class: PortClass::Unavailable,
                    reason: Reason {
                        code: "library-unavailable".to_owned(),
                        args: serde_json::json!({ "message": "fake" }),
                        line: None,
                    },
                }),
            }
        }
    }

    /// A judgement as JSON — [`NewAgentPresets`] holds the agent's `Attachments`, so the fake builds it from fields.
    #[derive(serde::Deserialize)]
    struct NewWire {
        default_preset: bool,
        presets: Vec<String>,
        skipped: Vec<crate::commands::SkippedRow>,
        #[serde(default)]
        change: Option<u64>,
    }

    /// A read view as JSON — [`LibraryReadView`] is not a wire type, so the fake builds it from fields.
    #[derive(serde::Deserialize)]
    struct ReadWire {
        row: ListRow,
        absolute: String,
        presets_direct: Vec<String>,
        presets_indirect: Vec<String>,
        agents_direct: Vec<AgentId>,
        agents_indirect: Vec<AgentId>,
        excerpt: Option<String>,
        settings_fields: Vec<String>,
    }

    impl LibraryCommandHost for FakeLibrary {
        fn status(&self) -> Result<LibraryStatusOk, PortRefusal> {
            self.answer("status", Value::Null)
        }
        fn add_mount(
            &self,
            name: &str,
            path: &str,
            create: bool,
        ) -> Result<LibraryAddMountOk, PortRefusal> {
            self.answer(
                "add_mount",
                serde_json::json!({ "name": name, "path": path, "create": create }),
            )
        }
        fn remove_mount(&self, mount: &str) -> Result<LibraryRemoveMountOk, PortRefusal> {
            self.answer("remove_mount", serde_json::json!({ "mount": mount }))
        }
        fn relocate_mount(
            &self,
            mount: &str,
            path: &str,
            create: bool,
        ) -> Result<LibraryStatusOk, PortRefusal> {
            self.answer(
                "relocate_mount",
                serde_json::json!({ "mount": mount, "path": path, "create": create }),
            )
        }
        fn rename_mount(
            &self,
            mount: &str,
            to: &str,
            yes: bool,
            root_generation: Option<u64>,
            plan_digest: Option<&str>,
        ) -> Result<LibraryRenameMountOk, PortRefusal> {
            self.answer(
                "rename_mount",
                serde_json::json!({
                    "mount": mount, "to": to, "yes": yes, "root_generation": root_generation,
                    "plan_digest": plan_digest,
                }),
            )
        }
        fn change_number(&self) -> Option<u64> {
            let answer = self.change_numbers.lock().unwrap().pop_front();
            self.calls
                .lock()
                .unwrap()
                .push(("change_number", serde_json::json!(answer)));
            answer
        }
        fn folders(&self) -> Result<LibraryFoldersOk, PortRefusal> {
            self.answer("folders", Value::Null)
        }
        fn refresh(&self, folder: Option<&str>) -> Result<LibraryRefreshOk, PortRefusal> {
            self.answer("refresh", serde_json::json!({ "folder": folder }))
        }
        fn list(
            &self,
            folder: Option<&str>,
            query: Option<&str>,
            kinds: &[LibraryKind],
        ) -> Result<LibraryListOk, PortRefusal> {
            self.answer(
                "list",
                serde_json::json!({ "folder": folder, "query": query, "kinds": kinds }),
            )
        }
        fn read(&self, path: &str) -> Result<LibraryReadView, PortRefusal> {
            let wire: ReadWire = self.answer("read", serde_json::json!({ "path": path }))?;
            Ok(LibraryReadView {
                row: wire.row,
                absolute: wire.absolute,
                presets_direct: wire.presets_direct,
                presets_indirect: wire.presets_indirect,
                agents_direct: wire.agents_direct,
                agents_indirect: wire.agents_indirect,
                excerpt: wire.excerpt,
                settings_fields: wire.settings_fields,
            })
        }
        fn new_folder(
            &self,
            parent: Option<&str>,
            name: &str,
            root_generation: Option<u64>,
        ) -> Result<String, PortRefusal> {
            self.answer(
                "new_folder",
                serde_json::json!({
                    "parent": parent, "name": name, "root_generation": root_generation,
                }),
            )
        }
        fn new_file(
            &self,
            folder: Option<&str>,
            name: &str,
            kind: LibraryNewFileKind,
            root_generation: Option<u64>,
        ) -> Result<String, PortRefusal> {
            self.answer(
                "new_file",
                serde_json::json!({
                    "folder": folder, "name": name, "kind": kind, "root_generation": root_generation,
                }),
            )
        }
        fn relink(
            &self,
            from: &str,
            to: &str,
            root_generation: Option<u64>,
        ) -> Result<LibraryRelinkOk, PortRefusal> {
            self.answer(
                "relink",
                serde_json::json!({ "from": from, "to": to, "root_generation": root_generation }),
            )
        }
        fn preset_get(&self, path: &str) -> Result<PresetView, PortRefusal> {
            self.answer("preset_get", serde_json::json!({ "path": path }))
        }
        fn preset_check(
            &self,
            path: &str,
            draft: &PresetDraftInput,
        ) -> Result<PresetCheckOk, PortRefusal> {
            self.answer(
                "preset_check",
                serde_json::json!({ "path": path, "draft": draft_json(draft) }),
            )
        }
        fn preset_add(
            &self,
            path: &str,
            entries: &PresetEntries,
            backend: Option<&str>,
            after: Option<&str>,
            root_generation: Option<u64>,
        ) -> Result<PresetAddOk, PortRefusal> {
            self.answer(
                "preset_add",
                serde_json::json!({
                    "path": path, "entries": entries_json(entries), "backend": backend,
                    "after": after, "root_generation": root_generation,
                }),
            )
        }
        fn preset_remove(
            &self,
            path: &str,
            entries: &PresetEntries,
            root_generation: Option<u64>,
        ) -> Result<PresetView, PortRefusal> {
            self.answer(
                "preset_remove",
                serde_json::json!({
                    "path": path, "entries": entries_json(entries), "root_generation": root_generation,
                }),
            )
        }
        fn preset_reorder(
            &self,
            path: &str,
            section: PresetSection,
            order: &[String],
            base_rev: &str,
            root_generation: Option<u64>,
        ) -> Result<PresetView, PortRefusal> {
            self.answer(
                "preset_reorder",
                serde_json::json!({
                    "path": path, "section": section.word(), "order": order, "base_rev": base_rev,
                    "root_generation": root_generation,
                }),
            )
        }
        fn preset_set_backend(
            &self,
            path: &str,
            items: &[String],
            backend: Option<&str>,
            root_generation: Option<u64>,
        ) -> Result<PresetSetBackendOk, PortRefusal> {
            self.answer(
                "preset_set_backend",
                serde_json::json!({
                    "path": path, "items": items, "backend": backend,
                    "root_generation": root_generation,
                }),
            )
        }
        fn preset_save(
            &self,
            path: &str,
            draft: &PresetDraftInput,
            base_rev: Option<&str>,
            root_generation: Option<u64>,
        ) -> Result<PresetView, PortRefusal> {
            self.answer(
                "preset_save",
                serde_json::json!({
                    "path": path, "draft": draft_json(draft), "base_rev": base_rev,
                    "root_generation": root_generation,
                }),
            )
        }
        fn preview_folder(
            &self,
            folder: &str,
            backend: Option<&str>,
            mode: Option<AgentOutputFormat>,
        ) -> Result<PresetPreviewFolderOk, PortRefusal> {
            self.answer(
                "preview_folder",
                serde_json::json!({ "folder": folder, "backend": backend, "mode": mode }),
            )
        }
        fn agent_presets(&self, agent: AgentId) -> Result<AgentPresetsPortView, PortRefusal> {
            self.answer("agent_presets", serde_json::json!({ "agent": agent }))
        }
        fn attach_preset(
            &self,
            agent: AgentId,
            presets: &[String],
            after: Option<&str>,
            root_generation: Option<u64>,
        ) -> Result<AgentAttachPortAnswer, PortRefusal> {
            self.answer(
                "attach_preset",
                serde_json::json!({
                    "agent": agent, "presets": presets, "after": after,
                    "root_generation": root_generation,
                }),
            )
        }
        fn detach_preset(
            &self,
            agent: AgentId,
            presets: &[String],
            yes: bool,
            root_generation: Option<u64>,
        ) -> Result<AgentPresetsPortView, PortRefusal> {
            self.answer(
                "detach_preset",
                serde_json::json!({
                    "agent": agent, "presets": presets, "yes": yes, "root_generation": root_generation,
                }),
            )
        }
        fn reorder_presets(
            &self,
            agent: AgentId,
            order: &[String],
            base_rev: &str,
            root_generation: Option<u64>,
        ) -> Result<AgentPresetsPortView, PortRefusal> {
            self.answer(
                "reorder_presets",
                serde_json::json!({
                    "agent": agent, "order": order, "base_rev": base_rev,
                    "root_generation": root_generation,
                }),
            )
        }
        fn judge_new_presets(
            &self,
            profile: &AgentProfile,
            presets: &[String],
            root_generation: Option<u64>,
        ) -> Result<NewAgentPresets, PortRefusal> {
            let attached: Vec<&str> = profile
                .presets
                .iter()
                .map(|attached| attached.path.as_str())
                .collect();
            let wire: NewWire = self.answer(
                "judge_new_presets",
                serde_json::json!({
                    "profile": { "default_preset": profile.default_preset, "presets": attached },
                    "presets": presets, "root_generation": root_generation,
                }),
            )?;
            Ok(NewAgentPresets {
                attachments: Attachments {
                    default_preset: wire.default_preset,
                    presets: wire
                        .presets
                        .into_iter()
                        .map(|path| crate::profile::AttachedPreset { path })
                        .collect(),
                },
                skipped: wire.skipped,
                change: wire.change,
            })
        }
        fn move_items(
            &self,
            paths: &[String],
            to_folder: &str,
            on_conflict: Option<LibraryOnConflict>,
            yes: bool,
            detach_refs: bool,
            plan_digest: Option<&str>,
            root_generation: Option<u64>,
        ) -> Result<MovePortAnswer, PortRefusal> {
            self.answer(
                "move_items",
                serde_json::json!({
                    "paths": paths, "to_folder": to_folder, "on_conflict": on_conflict, "yes": yes,
                    "detach_refs": detach_refs, "plan_digest": plan_digest,
                    "root_generation": root_generation,
                }),
            )
        }
        fn rename(
            &self,
            path: &str,
            name: &str,
            yes: bool,
            detach_refs: bool,
            plan_digest: Option<&str>,
            root_generation: Option<u64>,
        ) -> Result<MovePortAnswer, PortRefusal> {
            self.answer(
                "rename",
                serde_json::json!({
                    "path": path, "name": name, "yes": yes, "detach_refs": detach_refs,
                    "plan_digest": plan_digest, "root_generation": root_generation,
                }),
            )
        }
        fn copy_items(
            &self,
            paths: &[String],
            to_folder: &str,
            on_conflict: Option<LibraryOnConflict>,
            root_generation: Option<u64>,
        ) -> Result<LibraryCopyOk, PortRefusal> {
            self.answer(
                "copy_items",
                serde_json::json!({
                    "paths": paths, "to_folder": to_folder, "on_conflict": on_conflict,
                    "root_generation": root_generation,
                }),
            )
        }
        fn delete(
            &self,
            paths: &[String],
            yes: bool,
            detach_refs: bool,
            plan_digest: Option<&str>,
            root_generation: Option<u64>,
        ) -> Result<DeletePortAnswer, PortRefusal> {
            self.answer(
                "delete",
                serde_json::json!({
                    "paths": paths, "yes": yes, "detach_refs": detach_refs, "plan_digest": plan_digest,
                    "root_generation": root_generation,
                }),
            )
        }
        fn list_trash(&self) -> Result<LibraryListTrashOk, PortRefusal> {
            self.answer("list_trash", Value::Null)
        }
        fn restore(
            &self,
            rows: &[TrashAddressRow],
            keep_both: bool,
            root_generation: Option<u64>,
        ) -> Result<LibraryRestoreOk, PortRefusal> {
            self.answer(
                "restore",
                serde_json::json!({
                    "rows": rows, "keep_both": keep_both, "root_generation": root_generation,
                }),
            )
        }
        fn empty_trash(
            &self,
            bundles: &[ListedBundleArg],
            rows: &[TrashAddressRow],
        ) -> Result<LibraryEmptyTrashOk, PortRefusal> {
            self.answer(
                "empty_trash",
                serde_json::json!({ "bundles": bundles, "rows": rows }),
            )
        }
    }

    /// A draft as the recorded call shows it.
    fn draft_json(draft: &PresetDraftInput) -> Value {
        serde_json::json!({
            "presets": draft.presets, "instructions": draft.instructions, "skills": draft.skills,
            "hooks": draft.hooks, "settings": draft.settings, "folders": draft.folders,
        })
    }

    /// Items or folders as the recorded call shows them.
    fn entries_json(entries: &PresetEntries) -> Value {
        match entries {
            PresetEntries::Items(items) => serde_json::json!({ "items": items }),
            PresetEntries::Folders(folders) => serde_json::json!({ "folders": folders }),
        }
    }
}

#[cfg(test)]
mod tests {
    use engram_dashboard_command::testing::block_on;
    use engram_dashboard_command::RetryMode;
    use serde_json::json;

    use super::fake::FakeLibrary;
    use super::*;
    use crate::commands::{AgentRosterRow, StartedAgent, COMMAND_SPECS};
    use crate::manager::{
        Activated, NotStarted, NoticeDelivery, RenameOutcome, WorkspaceRefusal, WorkspaceSet,
    };
    use crate::profile::{AgentProfile, Refusal, SpawnMode, StoreStatus};
    use crate::queued_input::QueuedListing;
    use crate::types::{CancelError, CancelOutcome, PtyError};

    /// A host that answers only the roster — the one thing a library handler reads from the manager.
    struct Roster(Vec<(AgentId, &'static str)>);

    impl AgentCommandHost for Roster {
        fn roster(&self) -> Vec<AgentRosterRow> {
            self.0
                .iter()
                .map(|(id, name)| AgentRosterRow {
                    id: *id,
                    canonical_name: (*name).to_owned(),
                    cwd: "C:/work".to_owned(),
                    parent: None,
                    live: None,
                })
                .collect()
        }
        fn store_status(&self) -> StoreStatus {
            unreachable!("not a library call")
        }
        fn agent_snapshot(&self, _: AgentId) -> Option<AgentProfile> {
            unreachable!("not a library call")
        }
        fn create_agent(&self, _: AgentProfile) -> Result<AgentProfile, PtyError> {
            unreachable!("not a library call")
        }
        fn create_agent_if(
            &self,
            _: AgentProfile,
            _: &dyn Fn() -> bool,
        ) -> Result<Option<AgentProfile>, PtyError> {
            unreachable!("not a library call")
        }
        fn activate_profile(
            &self,
            _: &AgentProfile,
            _: SpawnMode,
        ) -> Result<StartedAgent, PtyError> {
            unreachable!("not a library call")
        }
        fn create_and_activate(
            &self,
            _: &AgentProfile,
            _: NoticeDelivery,
        ) -> Result<Activated, NotStarted> {
            unreachable!("not a library call")
        }
        fn rename_agent(&self, _: AgentId, _: Option<String>) -> Result<RenameOutcome, PtyError> {
            unreachable!("not a library call")
        }
        fn reparent_agent(&self, _: AgentId, _: Option<AgentId>) -> Result<bool, PtyError> {
            unreachable!("not a library call")
        }
        fn set_workspace(&self, _: AgentId, _: &str) -> Result<WorkspaceSet, WorkspaceRefusal> {
            unreachable!("not a library call")
        }
        fn duplicate_agent(&self, _: AgentId) -> Result<AgentProfile, PtyError> {
            unreachable!("not a library call")
        }
        fn list_queued_inputs(&self, _: AgentId) -> Option<QueuedListing> {
            unreachable!("not a library call")
        }
        fn cancel_queued_input(&self, _: AgentId, _: &str) -> Result<CancelOutcome, CancelError> {
            unreachable!("not a library call")
        }
        fn interrupt_agent(&self, _: AgentId) -> Result<(), PtyError> {
            unreachable!("not a library call")
        }
    }

    fn table(roster: Vec<(AgentId, &'static str)>, library: Arc<FakeLibrary>) -> CommandTable {
        let mut table = CommandTable::new(COMMAND_SPECS);
        let host: Arc<dyn AgentCommandHost> = Arc::new(Roster(roster));
        insert(&mut table, &host, library);
        table
    }

    fn call(table: &CommandTable, name: &str, mut args: Value) -> Result<Value, CommandError> {
        block_on(table.call(name, &mut args).expect("a name in the table"))
    }

    fn refused(class: PortClass, code: &str, args: Value, line: Option<u32>) -> PortRefusal {
        PortRefusal::Refused {
            class,
            reason: Reason {
                code: code.to_owned(),
                args,
                line,
            },
        }
    }

    /// Every field distinct, so a field carried into another's place fails the comparison.
    fn status_row() -> Value {
        json!({
            "daemon_id": "d1",
            "mounts": [
                {
                    "name": "library", "path": "C:/lib", "default": true, "builtin": false,
                    "state": "missing", "state_why": "the stat timed out",
                },
                {
                    "name": "engram", "path": "C:/dist/builtin", "default": false, "builtin": true,
                    "state": "ok", "state_why": null,
                },
            ],
            "config": "unsaved", "config_why": "the write failed",
            "suggested_new": "C:/data/library", "root_generation": 41,
        })
    }

    /// `status_row()` with `extra`'s fields added — a mount command's answer.
    fn status_with(extra: Value) -> Value {
        let mut row = status_row();
        let fields = row.as_object_mut().expect("an object");
        for (key, value) in extra.as_object().expect("an object") {
            fields.insert(key.clone(), value.clone());
        }
        row
    }

    fn read_view(path: &str) -> Value {
        json!({
            "row": list_row(path), "absolute": "C:/lib/x", "presets_direct": [], "presets_indirect": [],
            "agents_direct": [], "agents_indirect": [], "excerpt": null, "settings_fields": [],
        })
    }

    fn list_row(path: &str) -> Value {
        json!({
            "path": path, "kind": "Preset", "name": "p.yaml", "state": null, "references": 2,
            "description": "d",
        })
    }

    /// Every verb hands the port exactly the values the caller typed — spellings unparsed (the host parses them),
    /// omitted options as their stated defaults, enum words in the declared spelling.
    #[test]
    fn each_verb_hands_the_port_the_values_the_caller_typed() {
        let empty_report = json!({
            "changed": 0, "problems": [], "no_kind": [], "skipped_links": [], "errors": [],
        });
        let cases: Vec<(&str, Value, Value, &str, Value)> = vec![
            (
                "library.status",
                json!({}),
                status_row(),
                "status",
                Value::Null,
            ),
            (
                "library.addMount",
                json!({ "name": "team", "path": "C:/lib" }),
                status_with(json!({ "already_pointing": { "presets": 3, "agents": 1 } })),
                "add_mount",
                json!({ "name": "team", "path": "C:/lib", "create": false }),
            ),
            (
                "library.addMount",
                json!({ "name": "team", "path": "\"C:\\data\\lib\"", "create": true }),
                status_with(json!({ "already_pointing": { "presets": 0, "agents": 0 } })),
                "add_mount",
                json!({ "name": "team", "path": "C:\\data\\lib", "create": true }),
            ),
            (
                "library.removeMount",
                json!({ "mount": "team" }),
                status_with(json!({
                    "now_absent": { "presets": 2, "agents": 5 }, "outcome": REMOVE_OUTCOME_SAVED_ONLY,
                })),
                "remove_mount",
                json!({ "mount": "team" }),
            ),
            (
                "library.relocateMount",
                json!({ "mount": "team", "path": "\\\\?\\D:\\lib." }),
                status_row(),
                "relocate_mount",
                json!({ "mount": "team", "path": "\\\\?\\D:\\lib.", "create": false }),
            ),
            (
                "library.relocateMount",
                json!({ "mount": "team", "path": "C:/lib", "create": true }),
                status_row(),
                "relocate_mount",
                json!({ "mount": "team", "path": "C:/lib", "create": true }),
            ),
            (
                "library.renameMount",
                json!({ "mount": "team", "to": "shared" }),
                status_with(json!({
                    "fixed": [], "written": [], "skipped": [], "stopped": null,
                    "already_pointing": { "presets": 1, "agents": 0 },
                })),
                "rename_mount",
                json!({
                    "mount": "team", "to": "shared", "yes": false, "root_generation": null,
                    "plan_digest": null,
                }),
            ),
            (
                "library.renameMount",
                json!({
                    "mount": "team", "to": "Team", "yes": true, "root_generation": 7, "plan_digest": "d1",
                }),
                status_with(json!({
                    "fixed": [{ "from": "team/p.yaml", "to": "Team/p.yaml" }],
                    "written": ["shared/q.yaml"], "skipped": [], "stopped": null,
                    "already_pointing": { "presets": 0, "agents": 0 },
                })),
                "rename_mount",
                json!({
                    "mount": "team", "to": "Team", "yes": true, "root_generation": 7, "plan_digest": "d1",
                }),
            ),
            (
                "library.folders",
                json!({}),
                json!({ "folders": [], "unavailable": [] }),
                "folders",
                Value::Null,
            ),
            (
                "library.read",
                json!({ "path": "rules/a.md" }),
                read_view("rules/a.md"),
                "read",
                json!({ "path": "rules/a.md" }),
            ),
            (
                "library.refresh",
                json!({}),
                empty_report.clone(),
                "refresh",
                json!({ "folder": null }),
            ),
            (
                "library.refresh",
                json!({ "folder": "rules" }),
                empty_report,
                "refresh",
                json!({ "folder": "rules" }),
            ),
            (
                "library.list",
                json!({}),
                json!({ "folder": null, "rows": [], "folders": [], "unavailable": [] }),
                "list",
                json!({ "folder": null, "query": null, "kinds": [] }),
            ),
            (
                "library.list",
                json!({ "folder": "a", "query": "x", "kinds": ["skillfile", "Preset"] }),
                json!({ "folder": "a", "rows": [], "folders": [], "unavailable": [] }),
                "list",
                json!({ "folder": "a", "query": "x", "kinds": ["SkillFile", "Preset"] }),
            ),
            (
                "library.newFolder",
                json!({ "name": "team", "root_generation": 41 }),
                json!("team"),
                "new_folder",
                json!({ "parent": null, "name": "team", "root_generation": 41 }),
            ),
            (
                "library.newFile",
                json!({ "folder": "team", "name": "p", "kind": "Preset" }),
                json!("team/p.yaml"),
                "new_file",
                json!({ "folder": "team", "name": "p", "kind": "Preset", "root_generation": null }),
            ),
            (
                "library.relink",
                json!({ "from": "a.md", "to": "b/a.md", "root_generation": 7 }),
                json!({ "fixed": [], "more": 0, "written": [], "skipped": [], "stopped": null }),
                "relink",
                json!({ "from": "a.md", "to": "b/a.md", "root_generation": 7 }),
            ),
        ];
        for (name, args, answer, method, given) in cases {
            let library = FakeLibrary::answering(Ok(answer));
            let table = table(Vec::new(), Arc::clone(&library));
            call(&table, name, args).unwrap_or_else(|e| panic!("{name}: {e:?}"));
            assert_eq!(library.calls(), vec![(method, given)], "{name}");
        }
    }

    /// The created path comes back as `path`.
    #[test]
    fn a_create_answers_the_path_the_port_made() {
        let library = FakeLibrary::answering(Ok(json!("team/p.yaml")));
        let out = call(
            &table(Vec::new(), library),
            "library.newFile",
            json!({ "name": "p", "kind": "Preset" }),
        )
        .expect("create");
        assert_eq!(out, json!({ "path": "team/p.yaml" }));
    }

    /// The port's answer reaches the caller as built — the handler rebuilds nothing (ADR-0384).
    #[test]
    fn answers_are_carried_unchanged() {
        let answer = json!({
            "folder": "a",
            "rows": [list_row("a/p.yaml")],
            "folders": [
                { "path": "a/b", "name": "b", "state": null, "state_why": null },
                { "path": "team", "name": "team", "state": "missing", "state_why": "missing" },
            ],
            "unavailable": [{ "mount": "team", "state": "missing", "why": "missing" }],
        });
        let library = FakeLibrary::answering(Ok(answer.clone()));
        let out = call(
            &table(Vec::new(), library),
            "library.list",
            json!({ "folder": "a" }),
        )
        .expect("list");
        assert_eq!(out, answer);

        let relinked = json!({
            "fixed": [{ "from": "a.md", "to": "b/a.md" }],
            "more": 1,
            "written": ["p.yaml"],
            "skipped": [{ "path": "q.yaml", "code": "yaml-syntax", "args": { "path": "q.yaml" }, "line": 3 }],
            "stopped": {
                "path": "r.yaml", "code": "library-write-failed",
                "args": { "path": "r.yaml", "why": "x" }, "message": "m",
            },
        });
        let library = FakeLibrary::answering(Ok(relinked.clone()));
        let out = call(
            &table(Vec::new(), library),
            "library.relink",
            json!({ "from": "a.md", "to": "b/a.md" }),
        )
        .expect("relink");
        assert_eq!(out, relinked);
    }

    /// `library.relocateMount` answers the very status row the port answered (the two declarations are one shape).
    #[test]
    fn relocate_mount_answers_the_status_row_the_port_answered() {
        let library = FakeLibrary::answering(Ok(status_row()));
        let out = call(
            &table(Vec::new(), library),
            "library.relocateMount",
            json!({ "mount": "team", "path": "C:/lib" }),
        )
        .expect("relocate");
        assert_eq!(out, status_row());
    }

    /// `preset.reorder` takes the six lowercase section words only — another spelling (a capitalized one included) is
    /// INVALID_ARGUMENT before the port is called (ADR-0400 decision 22).
    #[test]
    fn a_section_word_outside_the_six_is_refused_before_the_port_is_called() {
        for word in ["Presets", "skill", "", "folders "] {
            let library = Arc::new(FakeLibrary::default());
            let err = call(
                &table(Vec::new(), Arc::clone(&library)),
                "preset.reorder",
                json!({ "path": "p.yaml", "section": word, "order": [], "base_rev": "r" }),
            )
            .expect_err("an unknown section word");
            assert_eq!(err.code(), ErrorCode::InvalidArgument, "{word:?}");
            assert!(
                err.message()
                    .contains("presets, instructions, skills, hooks, settings, folders"),
                "{word:?}"
            );
            assert!(library.calls().is_empty(), "{word:?}: the port was called");
        }
        for section in PresetSection::ALL {
            assert_eq!(PresetSection::parse(section.word()), Some(section));
        }
    }

    /// Each class lands on its ADR-0379 code with the reason as `{code, args, line?}` and a message naming the
    /// command and the code; `Unavailable` is `CONFLICT`.
    #[test]
    fn each_refusal_class_maps_onto_its_bus_code_with_the_reason_as_args() {
        let cases = [
            (PortClass::Conflict, ErrorCode::Conflict),
            (PortClass::NotFound, ErrorCode::NotFound),
            (PortClass::InvalidArgument, ErrorCode::InvalidArgument),
            (PortClass::Unavailable, ErrorCode::Conflict),
            (PortClass::Internal, ErrorCode::Internal),
        ];
        for (class, code) in cases {
            let library = FakeLibrary::answering(Err(refused(
                class,
                "some-code",
                json!({ "path": "a.md", "nested": { "x": 1 } }),
                Some(4),
            )));
            let err = call(
                &table(Vec::new(), library),
                "library.read",
                json!({ "path": "a.md" }),
            )
            .expect_err("refused");
            assert_eq!(err.code(), code, "{class:?}");
            assert_eq!(
                Value::Object(err.args().expect("args").clone()),
                json!({ "code": "some-code", "args": { "path": "a.md", "nested": { "x": 1 } }, "line": 4 }),
                "{class:?}"
            );
            assert_eq!(
                err.message(),
                "library.read: some-code — path: a.md, line: 4",
                "{class:?}"
            );
        }
    }

    /// A refusal the caller can clear and then repeat answers `AfterCondition`; any other keeps its code's default.
    #[test]
    fn a_clearable_refusal_answers_retry_after_condition() {
        for code in RETRY_AFTER_CONDITION {
            let library = FakeLibrary::answering(Err(refused(
                PortClass::Unavailable,
                code,
                json!({ "why": "x" }),
                None,
            )));
            let err = call(
                &table(Vec::new(), library),
                "library.relink",
                json!({ "from": "a.md", "to": "b.md" }),
            )
            .expect_err("refused");
            assert_eq!(
                (err.code(), err.retry()),
                (ErrorCode::Conflict, RetryMode::AfterCondition),
                "{code}"
            );
        }
        for code in [
            "library-config-read-only",
            "root-generation-stale",
            "name-exists",
        ] {
            let library =
                FakeLibrary::answering(Err(refused(PortClass::Conflict, code, json!({}), None)));
            let err = call(
                &table(Vec::new(), library),
                "library.newFolder",
                json!({ "name": "x" }),
            )
            .expect_err("refused");
            assert_eq!(err.retry(), ErrorCode::Conflict.default_retry(), "{code}");
        }
    }

    /// A reason with no line carries no `line` key — the shape of an `args.reasons` row.
    #[test]
    fn a_reason_without_a_line_carries_no_line_key() {
        let library = FakeLibrary::answering(Err(refused(
            PortClass::Unavailable,
            "library-unavailable",
            json!({ "message": "no root" }),
            None,
        )));
        let err =
            call(&table(Vec::new(), library), "library.folders", json!({})).expect_err("refused");
        assert_eq!(
            Value::Object(err.args().expect("args").clone()),
            json!({ "code": "library-unavailable", "args": { "message": "no root" } })
        );
        assert_eq!(
            err.message(),
            "library.folders: library-unavailable — message: no root; clear the cause, then repeat the same call"
        );
    }

    /// The CLI prints a refusal's message and not its `retry`, so every clearable refusal says it in words — and no
    /// other refusal does (a retry of the same call cannot clear it).
    #[test]
    fn a_clearable_refusal_message_tells_the_caller_to_repeat_the_same_call() {
        for code in RETRY_AFTER_CONDITION {
            let library = FakeLibrary::answering(Err(refused(
                PortClass::Unavailable,
                code,
                json!({ "path": "p.yaml" }),
                None,
            )));
            let err = call(
                &table(Vec::new(), library),
                "preset.remove",
                json!({ "path": "p.yaml", "items": ["a.md"] }),
            )
            .expect_err("refused");
            assert_eq!(
                err.message(),
                format!(
                    "preset.remove: {code} — path: p.yaml; clear the cause, then repeat the same call"
                ),
                "{code}"
            );
        }
        for code in [
            "root-generation-stale",
            "rev-mismatch",
            "name-exists",
            "library-config-read-only",
            "profile-edit-unavailable",
        ] {
            let library =
                FakeLibrary::answering(Err(refused(PortClass::Conflict, code, json!({}), None)));
            let err = call(
                &table(Vec::new(), library),
                "preset.get",
                json!({ "path": "p.yaml" }),
            )
            .expect_err("refused");
            assert_eq!(err.message(), format!("preset.get: {code}"), "{code}");
        }
    }

    /// A store error answers as every registry-writing command (`store_command_error`): the agent picks the code and
    /// the retry, the text says which file did not accept, and there are no args.
    #[test]
    fn a_store_refusal_answers_as_a_registry_writing_command() {
        let cases = [
            (
                StoreError::Refused(Refusal::Unreadable),
                ErrorCode::Conflict,
                RetryMode::AfterCondition,
            ),
            (
                StoreError::ReadOnly(Refusal::Newer { found: 3 }),
                ErrorCode::Conflict,
                ErrorCode::Conflict.default_retry(),
            ),
            (
                StoreError::Io(std::io::Error::other("disk full")),
                ErrorCode::Internal,
                ErrorCode::Internal.default_retry(),
            ),
        ];
        for (error, code, retry) in cases {
            let reason = store_error_reason(PROFILES_FILE, &error);
            let library = FakeLibrary::answering(Err(PortRefusal::Store(error)));
            let err = call(
                &table(Vec::new(), library),
                "library.relink",
                json!({ "from": "a.md", "to": "b.md" }),
            )
            .expect_err("refused");
            assert_eq!((err.code(), err.retry()), (code, retry), "{reason}");
            assert!(err.args().is_none(), "{reason}");
            assert_eq!(
                err.message(),
                format!("library.relink: {reason} — nothing was changed")
            );
        }
    }

    /// `library.read` names the agents from the roster and drops an id the roster no longer holds.
    #[test]
    fn read_names_the_agents_from_the_roster_and_drops_unknown_ids() {
        let (bob, ann, gone) = (AgentId::new_v4(), AgentId::new_v4(), AgentId::new_v4());
        let view = json!({
            "row": list_row("p.yaml"),
            "absolute": "C:/lib/p.yaml",
            "presets_direct": ["q.yaml"],
            "presets_indirect": ["r.yaml"],
            "agents_direct": [bob, gone],
            "agents_indirect": [ann],
            "excerpt": "e",
            "settings_fields": ["f"],
        });
        let library = FakeLibrary::answering(Ok(view));
        let out = call(
            &table(vec![(bob, "bob"), (ann, "ann")], library),
            "library.read",
            json!({ "path": "p.yaml" }),
        )
        .expect("read");
        assert_eq!(
            out,
            json!({
                "row": list_row("p.yaml"),
                "absolute": "C:/lib/p.yaml",
                "presets_direct": ["q.yaml"],
                "presets_indirect": ["r.yaml"],
                "agents_direct": [{ "id": bob.to_string(), "name": "bob" }],
                "agents_indirect": [{ "id": ann.to_string(), "name": "ann" }],
                "excerpt": "e",
                "settings_fields": ["f"],
            })
        );
    }

    /// A blank value is refused before the port is called — an unset shell variable is not a path.
    #[test]
    fn blank_values_are_refused_before_the_port_is_called() {
        let cases = [
            ("library.addMount", json!({ "name": "team", "path": " " })),
            ("library.addMount", json!({ "name": " ", "path": "C:/lib" })),
            ("library.removeMount", json!({ "mount": "" })),
            (
                "library.relocateMount",
                json!({ "mount": "team", "path": "" }),
            ),
            (
                "library.relocateMount",
                json!({ "mount": " ", "path": "C:/lib" }),
            ),
            ("library.renameMount", json!({ "mount": "team", "to": "" })),
            (
                "library.renameMount",
                json!({ "mount": " ", "to": "shared" }),
            ),
            ("library.refresh", json!({ "folder": "" })),
            ("library.list", json!({ "query": "  " })),
            ("library.list", json!({ "folder": "" })),
            ("library.read", json!({ "path": "" })),
            ("library.newFolder", json!({ "name": " " })),
            ("library.newFolder", json!({ "parent": "", "name": "x" })),
            ("library.newFile", json!({ "name": "", "kind": "Markdown" })),
            (
                "library.newFile",
                json!({ "folder": " ", "name": "x", "kind": "Markdown" }),
            ),
            ("library.relink", json!({ "from": "", "to": "b.md" })),
            ("library.relink", json!({ "from": "a.md", "to": " " })),
        ];
        for (name, args) in cases {
            let library = Arc::new(FakeLibrary::default());
            let err = call(&table(Vec::new(), Arc::clone(&library)), name, args.clone())
                .expect_err("blank refused");
            assert_eq!(err.code(), ErrorCode::InvalidArgument, "{name} {args}");
            assert!(err.args().is_none(), "{name} {args}");
            assert!(
                library.calls().is_empty(),
                "{name} {args}: the port was called"
            );
        }
    }

    /// A relink stop carries the refusal it met as a refusal would: the reason's code and args and a message naming
    /// the command — and a store error as `profile-store-refused` with the store's words in `why`.
    #[test]
    fn a_relink_stop_carries_the_refusal_it_met() {
        let stop = relink_stop_row(
            "library.relink",
            Some("r.yaml".to_owned()),
            refused(
                PortClass::Unavailable,
                "library-write-failed",
                json!({ "path": "r.yaml", "why": "denied" }),
                None,
            ),
        );
        assert_eq!(
            stop,
            RelinkStopRow {
                path: Some("r.yaml".to_owned()),
                code: "library-write-failed".to_owned(),
                args: json!({ "path": "r.yaml", "why": "denied" }),
                message: "library.relink: library-write-failed — path: r.yaml, why: denied"
                    .to_owned(),
            }
        );

        let error = StoreError::Refused(Refusal::Unreadable);
        let why = store_error_reason(PROFILES_FILE, &error);
        let stop = relink_stop_row("library.relink", None, PortRefusal::Store(error));
        assert_eq!(stop.path, None);
        assert_eq!(stop.code, PROFILE_STORE_REFUSED);
        assert_eq!(stop.args, json!({ "why": why }));
        assert_eq!(
            stop.message,
            format!("library.relink: {PROFILE_STORE_REFUSED} — why: {why}")
        );
    }

    /// Control characters in a caller-typed spelling are escaped in the message line.
    #[test]
    fn the_message_escapes_control_characters() {
        let message = refusal_message(
            "library.read",
            &Reason {
                code: "not-found".to_owned(),
                args: json!({ "path": "a\nb" }),
                line: None,
            },
        );
        assert_eq!(message, "library.read: not-found — path: a\\nb");
    }

    // ── preset.* ──

    /// Every field distinct, so a field carried into another's place fails the comparison.
    fn preset_view() -> Value {
        let entry = |path: &str, line: u32| json!({ "path": path, "backend": null, "line": line, "state": null });
        json!({
            "path": "team/p.yaml",
            "rev": "r1",
            "description": "d",
            "presets": [entry("q.yaml", 3)],
            "instructions": [{
                "path": "a.md", "backend": "claude", "line": 5,
                "state": { "code": "not-found", "args": { "path": "a.md" }, "line": 5 },
            }],
            "skills": [entry("s1", 7)],
            "hooks": [entry("team/hooks/h", 8)],
            "settings": [entry("s.yaml", 9)],
            "folders": [{ "path": "C:\\f", "line": 11, "status": "unreadable", "status_why": "timed out" }],
            "conflicts": [{
                "kind": "conflict", "item": null, "via": null,
                "items": [{ "item": "a.md", "via": ["team/p.yaml"] }],
                "code": "c", "args": {}, "line": null,
            }],
            "problems": [{
                "kind": "missing", "item": "a.md", "via": ["team/p.yaml"], "items": null,
                "code": "not-found", "args": { "path": "a.md" }, "line": 5,
            }],
            "notices": [{
                "code": "builtin-unavailable",
                "args": { "path": "engram/default", "why": "w" },
                "line": null,
                "text": "t",
            }],
            "agents": 2,
        })
    }

    fn partial_answer() -> Value {
        json!({
            "applied": ["a.md"],
            "skipped": [{ "item": "b.md", "code": "not-found", "args": { "path": "b.md" }, "line": null }],
            "view": preset_view(),
        })
    }

    /// Every preset verb hands the port exactly the values the caller typed — spellings unparsed, `items` or `folders`
    /// as the one the caller gave, omitted options as `None`, enum words in the declared spelling.
    #[test]
    fn each_preset_verb_hands_the_port_the_values_the_caller_typed() {
        let entry = json!({ "path": "a.md", "backend": "codex" });
        let draft = json!({
            "presets": [], "instructions": [entry.clone()], "skills": [], "hooks": null, "settings": [],
            "folders": null,
        });
        let hook = json!({ "path": "team/hooks/h", "backend": "claude" });
        let check =
            json!({ "save_refuses": false, "folders": [], "conflicts": [], "problems": [] });
        let preview = json!({
            "path": null, "refused": null, "loads": [], "truncated": false, "skipped": [], "unlisted": null,
        });
        let cases: Vec<(&str, Value, Value, &str, Value)> = vec![
            (
                "preset.get",
                json!({ "path": "engram/default" }),
                preset_view(),
                "preset_get",
                json!({ "path": "engram/default" }),
            ),
            (
                "preset.check",
                json!({
                    "path": "p.yaml", "presets": [], "instructions": [entry.clone()], "skills": [],
                    "settings": [],
                }),
                check.clone(),
                "preset_check",
                json!({ "path": "p.yaml", "draft": draft }),
            ),
            (
                "preset.check",
                json!({
                    "path": "p.yaml", "presets": [], "instructions": [], "skills": [], "hooks": [hook.clone()],
                    "settings": [], "folders": ["C:/f"],
                }),
                check,
                "preset_check",
                json!({ "path": "p.yaml", "draft": {
                    "presets": [], "instructions": [], "skills": [], "hooks": [hook], "settings": [],
                    "folders": ["C:/f"],
                } }),
            ),
            (
                "preset.add",
                json!({ "path": "p.yaml", "items": ["a.md", "s1"], "backend": "claude", "after": "x.md" }),
                partial_answer(),
                "preset_add",
                json!({
                    "path": "p.yaml", "entries": { "items": ["a.md", "s1"] }, "backend": "claude",
                    "after": "x.md", "root_generation": null,
                }),
            ),
            (
                "preset.add",
                json!({ "path": "p.yaml", "folders": ["C:/f"], "root_generation": 4 }),
                partial_answer(),
                "preset_add",
                json!({
                    "path": "p.yaml", "entries": { "folders": ["C:/f"] }, "backend": null,
                    "after": null, "root_generation": 4,
                }),
            ),
            (
                "preset.remove",
                json!({ "path": "p.yaml", "folders": ["C:/f"] }),
                preset_view(),
                "preset_remove",
                json!({ "path": "p.yaml", "entries": { "folders": ["C:/f"] }, "root_generation": null }),
            ),
            (
                "preset.remove",
                json!({ "path": "p.yaml", "items": ["a.md"], "root_generation": 4 }),
                preset_view(),
                "preset_remove",
                json!({ "path": "p.yaml", "entries": { "items": ["a.md"] }, "root_generation": 4 }),
            ),
            (
                "preset.reorder",
                json!({ "path": "p.yaml", "section": "skills", "order": ["b", "a"], "base_rev": "r1" }),
                preset_view(),
                "preset_reorder",
                json!({
                    "path": "p.yaml", "section": "skills", "order": ["b", "a"], "base_rev": "r1",
                    "root_generation": null,
                }),
            ),
            (
                "preset.reorder",
                json!({ "path": "p.yaml", "section": "hooks", "order": ["h"], "base_rev": "r1" }),
                preset_view(),
                "preset_reorder",
                json!({
                    "path": "p.yaml", "section": "hooks", "order": ["h"], "base_rev": "r1",
                    "root_generation": null,
                }),
            ),
            (
                "preset.setBackend",
                json!({ "path": "p.yaml", "items": ["a.md"] }),
                partial_answer(),
                "preset_set_backend",
                json!({ "path": "p.yaml", "items": ["a.md"], "backend": null, "root_generation": null }),
            ),
            (
                "preset.setBackend",
                json!({ "path": "p.yaml", "items": ["a.md"], "backend": "codex", "root_generation": 4 }),
                partial_answer(),
                "preset_set_backend",
                json!({ "path": "p.yaml", "items": ["a.md"], "backend": "codex", "root_generation": 4 }),
            ),
            (
                "preset.save",
                json!({
                    "path": "p.yaml", "presets": [], "instructions": [entry], "skills": [], "settings": [],
                    "base_rev": "r1", "root_generation": 4,
                }),
                preset_view(),
                "preset_save",
                json!({
                    "path": "p.yaml",
                    "draft": {
                        "presets": [], "instructions": [{ "path": "a.md", "backend": "codex" }],
                        "skills": [], "hooks": null, "settings": [], "folders": null,
                    },
                    "base_rev": "r1", "root_generation": 4,
                }),
            ),
            (
                "preset.previewFolder",
                json!({ "folder": "C:/f" }),
                preview.clone(),
                "preview_folder",
                json!({ "folder": "C:/f", "backend": null, "mode": null }),
            ),
            (
                "preset.previewFolder",
                json!({ "folder": "C:/f", "backend": "codex", "mode": "terminal" }),
                preview,
                "preview_folder",
                json!({ "folder": "C:/f", "backend": "codex", "mode": "Terminal" }),
            ),
        ];
        for (name, args, answer, method, given) in cases {
            let library = FakeLibrary::answering(Ok(answer));
            let table = table(Vec::new(), Arc::clone(&library));
            call(&table, name, args).unwrap_or_else(|e| panic!("{name}: {e:?}"));
            assert_eq!(library.calls(), vec![(method, given)], "{name}");
        }
    }

    /// Every folder spelling reaches the port with one layer of quotes off and its backslashes kept — a quoted
    /// `preset.remove` folder would otherwise match no listed folder and answer success with it still listed. Library
    /// item spellings, and a `reorder` order of an item section, reach it untouched.
    #[test]
    fn folder_spellings_reach_the_port_unquoted_and_item_spellings_untouched() {
        let quoted = "\"C:\\docs\"";
        let lists = |folders: Value| {
            json!({
                "path": "p.yaml", "presets": [], "instructions": [{ "path": "\"a.md\"", "backend": null }],
                "skills": [], "hooks": [{ "path": "\"h\"", "backend": null }], "settings": [],
                "folders": folders,
            })
        };
        let draft = json!({
            "presets": [], "instructions": [{ "path": "\"a.md\"", "backend": null }], "skills": [],
            "hooks": [{ "path": "\"h\"", "backend": null }], "settings": [],
            "folders": ["C:\\docs", "'D:\\x y'"],
        });
        let check =
            json!({ "save_refuses": false, "folders": [], "conflicts": [], "problems": [] });
        let preview = json!({
            "path": null, "refused": null, "loads": [], "truncated": false, "skipped": [], "unlisted": null,
        });
        let cases: Vec<(&str, Value, Value, &str, Value)> = vec![
            (
                "preset.remove",
                json!({ "path": "p.yaml", "folders": [quoted] }),
                preset_view(),
                "preset_remove",
                json!({ "path": "p.yaml", "entries": { "folders": ["C:\\docs"] }, "root_generation": null }),
            ),
            (
                "preset.remove",
                json!({ "path": "p.yaml", "items": ["\"a.md\""] }),
                preset_view(),
                "preset_remove",
                json!({ "path": "p.yaml", "entries": { "items": ["\"a.md\""] }, "root_generation": null }),
            ),
            (
                "preset.add",
                json!({ "path": "p.yaml", "folders": [quoted, "\\\\?\\D:\\lib"] }),
                partial_answer(),
                "preset_add",
                json!({
                    "path": "p.yaml", "entries": { "folders": ["C:\\docs", "\\\\?\\D:\\lib"] },
                    "backend": null, "after": null, "root_generation": null,
                }),
            ),
            (
                "preset.check",
                lists(json!([quoted, "\"'D:\\x y'\""])),
                check,
                "preset_check",
                json!({ "path": "p.yaml", "draft": draft.clone() }),
            ),
            (
                "preset.save",
                lists(json!([quoted, "\"'D:\\x y'\""])),
                preset_view(),
                "preset_save",
                json!({ "path": "p.yaml", "draft": draft, "base_rev": null, "root_generation": null }),
            ),
            (
                "preset.reorder",
                json!({ "path": "p.yaml", "section": "folders", "order": [quoted, "D:\\y"], "base_rev": "r" }),
                preset_view(),
                "preset_reorder",
                json!({
                    "path": "p.yaml", "section": "folders", "order": ["C:\\docs", "D:\\y"],
                    "base_rev": "r", "root_generation": null,
                }),
            ),
            (
                "preset.reorder",
                json!({ "path": "p.yaml", "section": "instructions", "order": ["\"a.md\""], "base_rev": "r" }),
                preset_view(),
                "preset_reorder",
                json!({
                    "path": "p.yaml", "section": "instructions", "order": ["\"a.md\""],
                    "base_rev": "r", "root_generation": null,
                }),
            ),
            (
                "preset.previewFolder",
                json!({ "folder": quoted }),
                preview,
                "preview_folder",
                json!({ "folder": "C:\\docs", "backend": null, "mode": null }),
            ),
        ];
        for (name, args, answer, method, given) in cases {
            let library = FakeLibrary::answering(Ok(answer));
            let table = table(Vec::new(), Arc::clone(&library));
            call(&table, name, args.clone()).unwrap_or_else(|e| panic!("{name}: {e:?}"));
            assert_eq!(library.calls(), vec![(method, given)], "{name} {args}");
        }
    }

    /// `preset.get` and the edits that answer the view alone answer the very view the port built, flat; the partial
    /// edits answer `{applied, skipped, view}` as built (ADR-0384 decision 5).
    #[test]
    fn preset_answers_are_carried_as_the_port_built_them() {
        let view_cases = [
            ("preset.get", json!({ "path": "team/p.yaml" })),
            (
                "preset.remove",
                json!({ "path": "team/p.yaml", "items": ["a.md"] }),
            ),
            (
                "preset.reorder",
                json!({ "path": "team/p.yaml", "section": "presets", "order": [], "base_rev": "r1" }),
            ),
            (
                "preset.save",
                json!({
                    "path": "team/p.yaml", "presets": [], "instructions": [], "skills": [], "settings": [],
                }),
            ),
        ];
        for (name, args) in view_cases {
            let library = FakeLibrary::answering(Ok(preset_view()));
            let out = call(&table(Vec::new(), library), name, args).expect(name);
            assert_eq!(out, preset_view(), "{name}");
        }
        let partial_cases = [
            (
                "preset.add",
                json!({ "path": "team/p.yaml", "items": ["a.md", "b.md"] }),
            ),
            (
                "preset.setBackend",
                json!({ "path": "team/p.yaml", "items": ["a.md", "b.md"] }),
            ),
        ];
        for (name, args) in partial_cases {
            let library = FakeLibrary::answering(Ok(partial_answer()));
            let out = call(&table(Vec::new(), library), name, args).expect(name);
            assert_eq!(out, partial_answer(), "{name}");
        }

        let check = json!({
            "save_refuses": true,
            "folders": [{ "path": "C:\\f", "line": null, "status": "ok", "status_why": null }],
            "conflicts": [],
            "problems": [{
                "kind": "broken", "item": "x", "via": ["p.yaml"], "items": null,
                "code": "wrong-kind", "args": { "path": "x", "list": "skills" }, "line": null,
            }],
        });
        let library = FakeLibrary::answering(Ok(check.clone()));
        let out = call(
            &table(Vec::new(), library),
            "preset.check",
            json!({ "path": "p.yaml", "presets": [], "instructions": [], "skills": [], "settings": [] }),
        )
        .expect("check");
        assert_eq!(out, check);

        let preview = json!({
            "path": "C:\\f",
            "refused": { "path": "C:/f", "code": "folder-path-cli-unsafe", "why": null },
            "loads": [{
                "backend": "codex", "mode": "StreamJson", "predicted": true,
                "set": {
                    "loads": [{ "path": "AGENTS.md", "kind": "Instructions" }],
                    "omitted": [{ "path": ".codex/skills", "kind": "SkillRoot" }],
                    "carried": [],
                    "unsure": [{
                        "path": "x.md", "kind": "Rule", "why": "front_matter", "message": "bad", "line": 2,
                    }],
                    "malformed": [{ "path": "s/SKILL.md", "code": "m", "args": {}, "line": null }],
                    "skill_names": [{ "path": "s/SKILL.md", "name": "s" }],
                },
                "unread": "y.md",
            }],
            "truncated": true,
            "skipped": ["deep"],
            "unlisted": null,
        });
        let library = FakeLibrary::answering(Ok(preview.clone()));
        let out = call(
            &table(Vec::new(), library),
            "preset.previewFolder",
            json!({ "folder": "C:/f" }),
        )
        .expect("preview");
        assert_eq!(out, preview);
    }

    /// `preset.add` and `preset.remove` take exactly one of `items` and `folders`; `preset.add`'s `backend` goes with
    /// `items` only. Each mistake is the handler's own argument error — no args, the port never called.
    #[test]
    fn preset_items_and_folders_are_one_of_and_backend_goes_with_items() {
        let cases = [
            (
                "preset.add",
                json!({ "path": "p.yaml", "items": ["a.md"], "folders": ["C:/f"] }),
            ),
            ("preset.add", json!({ "path": "p.yaml" })),
            (
                "preset.add",
                json!({ "path": "p.yaml", "folders": ["C:/f"], "backend": "claude" }),
            ),
            (
                "preset.remove",
                json!({ "path": "p.yaml", "items": [], "folders": [] }),
            ),
            ("preset.remove", json!({ "path": "p.yaml" })),
        ];
        for (name, args) in cases {
            let library = Arc::new(FakeLibrary::default());
            let err = call(&table(Vec::new(), Arc::clone(&library)), name, args.clone())
                .expect_err("refused");
            assert_eq!(err.code(), ErrorCode::InvalidArgument, "{name} {args}");
            assert!(err.args().is_none(), "{name} {args}");
            assert!(
                library.calls().is_empty(),
                "{name} {args}: the port was called"
            );
        }
    }

    /// A blank value is refused before the port is called.
    #[test]
    fn blank_preset_values_are_refused_before_the_port_is_called() {
        let lists = json!({ "presets": [], "instructions": [], "skills": [], "settings": [] });
        let with_lists = |extra: Value| {
            let mut args = lists.clone();
            args.as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            args
        };
        let cases = [
            ("preset.get", json!({ "path": " " })),
            ("preset.check", with_lists(json!({ "path": "" }))),
            ("preset.add", json!({ "path": "", "items": ["a.md"] })),
            (
                "preset.add",
                json!({ "path": "p.yaml", "items": ["a.md"], "backend": " " }),
            ),
            (
                "preset.add",
                json!({ "path": "p.yaml", "items": ["a.md"], "after": "" }),
            ),
            ("preset.remove", json!({ "path": " ", "items": ["a.md"] })),
            (
                "preset.reorder",
                json!({ "path": "p.yaml", "section": "skills", "order": [], "base_rev": " " }),
            ),
            (
                "preset.setBackend",
                json!({ "path": "p.yaml", "items": ["a.md"], "backend": "" }),
            ),
            (
                "preset.save",
                with_lists(json!({ "path": "p.yaml", "base_rev": "" })),
            ),
            ("preset.previewFolder", json!({ "folder": " " })),
            (
                "preset.previewFolder",
                json!({ "folder": "C:/f", "backend": "" }),
            ),
        ];
        for (name, args) in cases {
            let library = Arc::new(FakeLibrary::default());
            let err = call(&table(Vec::new(), Arc::clone(&library)), name, args.clone())
                .expect_err("blank refused");
            assert_eq!(err.code(), ErrorCode::InvalidArgument, "{name} {args}");
            assert!(
                library.calls().is_empty(),
                "{name} {args}: the port was called"
            );
        }
    }

    /// A preset refusal maps as every library refusal: the class's code, the reason as `args`, the command in the
    /// message.
    #[test]
    fn a_preset_refusal_maps_onto_its_code_with_the_reason_as_args() {
        let cases = [
            (
                "preset.reorder",
                json!({ "path": "p.yaml", "section": "presets", "order": [], "base_rev": "old" }),
                refused(
                    PortClass::Conflict,
                    "rev-mismatch",
                    json!({ "rev": "new" }),
                    None,
                ),
                ErrorCode::Conflict,
            ),
            (
                "preset.save",
                json!({ "path": "p.yaml", "presets": [], "instructions": [], "skills": [], "settings": [] }),
                refused(
                    PortClass::InvalidArgument,
                    "preset-save-refused",
                    json!({ "items": [{ "list": "skills", "path": "x", "code": "wrong-kind" }] }),
                    None,
                ),
                ErrorCode::InvalidArgument,
            ),
            (
                "preset.get",
                json!({ "path": "nope.yaml" }),
                refused(
                    PortClass::NotFound,
                    "not-found",
                    json!({ "path": "nope.yaml" }),
                    None,
                ),
                ErrorCode::NotFound,
            ),
            (
                "preset.previewFolder",
                json!({ "folder": "C:/f", "backend": "zzz" }),
                refused(
                    PortClass::InvalidArgument,
                    "backend-tag-unknown",
                    json!({ "backend": "zzz" }),
                    None,
                ),
                ErrorCode::InvalidArgument,
            ),
        ];
        for (name, args, refusal, code) in cases {
            let PortRefusal::Refused { reason, .. } = &refusal else {
                unreachable!("built refused")
            };
            let mut expected = Map::new();
            put_reason(&mut expected, reason);
            let library = FakeLibrary::answering(Err(refusal));
            let err = call(&table(Vec::new(), library), name, args).expect_err("refused");
            assert_eq!(err.code(), code, "{name}");
            assert_eq!(err.args(), Some(&expected), "{name}");
            assert!(err.message().starts_with(&format!("{name}: ")), "{name}");
        }
    }

    /// A problem row equals the row `agent.spawn` puts in `args.reasons` (`LaunchBlocked::to_args`) for every kind,
    /// apart from the keys `to_args` leaves out, which the row sends as `null`.
    #[test]
    fn a_problem_row_is_the_launch_block_row_with_absent_keys_as_null() {
        let reason = |line: Option<u32>| Reason {
            code: "c".to_owned(),
            args: json!({ "path": "a.md" }),
            line,
        };
        let reasons = vec![
            BlockReason::Missing {
                item: "a.md".to_owned(),
                via: vec!["p.yaml".to_owned()],
                reason: reason(Some(3)),
            },
            BlockReason::Broken {
                item: "b.md".to_owned(),
                via: vec!["p.yaml".to_owned(), "q.yaml".to_owned()],
                reason: reason(None),
            },
            BlockReason::Conflict {
                reason: reason(None),
                items: vec![
                    ("a.md".to_owned(), vec!["p.yaml".to_owned()]),
                    ("b.md".to_owned(), vec![]),
                ],
            },
            BlockReason::LibraryUnavailable {
                reason: reason(None),
            },
        ];
        let blocked = crate::launch::LaunchBlocked {
            reasons: reasons.clone(),
        };
        let launch_rows = blocked.to_args()["reasons"].clone();
        let launch_rows = launch_rows.as_array().expect("rows");
        assert_eq!(launch_rows.len(), reasons.len());
        for (reason, launch_row) in reasons.into_iter().zip(launch_rows) {
            let row = serde_json::to_value(problem_row(reason)).expect("row");
            let mut row = row.as_object().expect("an object").clone();
            let absent: Vec<String> = row
                .iter()
                .filter(|(key, value)| {
                    value.is_null() && !launch_row.as_object().unwrap().contains_key(*key)
                })
                .map(|(key, _)| key.clone())
                .collect();
            for key in &absent {
                row.remove(key);
            }
            assert_eq!(Value::Object(row), *launch_row);
        }
    }

    /// A folder add's `after` is a folder spelling, so it reaches the port with one layer of quotes off — a quoted
    /// anchor would match no listed folder and the folders would land at the end with no word said. An item add's
    /// `after` is a library item spelling and reaches it untouched.
    #[test]
    fn a_folder_adds_anchor_reaches_the_port_unquoted_and_an_item_adds_untouched() {
        let cases = [
            (
                json!({ "path": "p.yaml", "folders": ["D:\\new"], "after": "\"C:\\docs\"" }),
                json!({
                    "path": "p.yaml", "entries": { "folders": ["D:\\new"] }, "backend": null,
                    "after": "C:\\docs", "root_generation": null,
                }),
            ),
            (
                json!({ "path": "p.yaml", "items": ["a.md"], "after": "\"b.md\"" }),
                json!({
                    "path": "p.yaml", "entries": { "items": ["a.md"] }, "backend": null,
                    "after": "\"b.md\"", "root_generation": null,
                }),
            ),
        ];
        for (args, given) in cases {
            let library = FakeLibrary::answering(Ok(partial_answer()));
            call(
                &table(Vec::new(), Arc::clone(&library)),
                "preset.add",
                args.clone(),
            )
            .unwrap_or_else(|e| panic!("{args}: {e:?}"));
            assert_eq!(library.calls(), vec![("preset_add", given)], "{args}");
        }
    }

    // ── agent presets ──

    /// The view as the port answers it — every field distinct, so a field carried into another's place fails.
    fn port_view(agent: AgentId) -> Value {
        json!({
            "agent_id": agent,
            "default_preset": true,
            "presets": ["P.yaml", "Q.yaml"],
            "cwd": "C:/work/bob",
            "workspace_lock": "conversed",
            "backend": "claude",
            "mode": "StreamJson",
            "base_rev": "b1",
            "takes_presets": true,
            "folders": [{
                "path": "D:\\docs", "via": ["P.yaml"], "line": 4, "status": "unreadable",
                "status_why": "timed out",
            }],
            "conflicts": [{
                "kind": "conflict", "item": null, "via": null,
                "items": [{ "item": "a.md", "via": ["P.yaml"] }], "code": "c", "args": {}, "line": null,
            }],
            "problems": [{
                "kind": "missing", "item": "m.md", "via": ["Q.yaml"], "items": null,
                "code": "not-found", "args": { "path": "m.md" }, "line": 6,
            }],
            "notices": [{ "code": "n", "args": { "names": ["x"] }, "line": null, "text": "t" }],
        })
    }

    /// The declared view: the port's, with `agent_id` as text and `name` beside it.
    fn named_view(agent: AgentId, name: &str) -> Value {
        let mut view = port_view(agent);
        view["agent_id"] = json!(agent.to_string());
        view["name"] = json!(name);
        view
    }

    /// Every agent-preset verb resolves `target` (a name or an id) and hands the port that agent's id with the values
    /// the caller typed — spellings unparsed, `yes` unspecified as `false`.
    #[test]
    fn each_agent_preset_verb_hands_the_port_the_resolved_id_and_the_values_typed() {
        let (bob, ann) = (AgentId::new_v4(), AgentId::new_v4());
        let attach = json!({ "applied": [], "skipped": [], "view": port_view(bob) });
        let cases: Vec<(&str, Value, Value, &str, Value)> = vec![
            (
                "agent.presets",
                json!({ "target": "bob" }),
                port_view(bob),
                "agent_presets",
                json!({ "agent": bob }),
            ),
            (
                "agent.presets",
                json!({ "target": ann.to_string() }),
                port_view(ann),
                "agent_presets",
                json!({ "agent": ann }),
            ),
            (
                "agent.attachPreset",
                json!({ "target": "bob", "presets": ["R.yaml", "\"S.yaml\""], "after": "P.yaml" }),
                attach.clone(),
                "attach_preset",
                json!({
                    "agent": bob, "presets": ["R.yaml", "\"S.yaml\""], "after": "P.yaml",
                    "root_generation": null,
                }),
            ),
            (
                "agent.attachPreset",
                json!({ "target": "bob", "presets": [], "root_generation": 7 }),
                attach,
                "attach_preset",
                json!({ "agent": bob, "presets": [], "after": null, "root_generation": 7 }),
            ),
            (
                "agent.detachPreset",
                json!({ "target": "bob", "presets": ["engram/default"] }),
                port_view(bob),
                "detach_preset",
                json!({
                    "agent": bob, "presets": ["engram/default"], "yes": false, "root_generation": null,
                }),
            ),
            (
                "agent.detachPreset",
                json!({ "target": "bob", "presets": ["P.yaml"], "yes": true, "root_generation": 7 }),
                port_view(bob),
                "detach_preset",
                json!({ "agent": bob, "presets": ["P.yaml"], "yes": true, "root_generation": 7 }),
            ),
            (
                "agent.reorderPresets",
                json!({ "target": "bob", "order": ["Q.yaml", "P.yaml"], "base_rev": "b1" }),
                port_view(bob),
                "reorder_presets",
                json!({
                    "agent": bob, "order": ["Q.yaml", "P.yaml"], "base_rev": "b1", "root_generation": null,
                }),
            ),
        ];
        for (name, args, answer, method, given) in cases {
            let library = FakeLibrary::answering(Ok(answer));
            let table = table(vec![(bob, "bob"), (ann, "ann")], Arc::clone(&library));
            call(&table, name, args.clone()).unwrap_or_else(|e| panic!("{name} {args}: {e:?}"));
            assert_eq!(library.calls(), vec![(method, given)], "{name} {args}");
        }
    }

    /// The port answers the agent by id only; every agent-preset answer carries the name the `target` resolution found
    /// — also when the caller pointed by id — beside the view as the port built it (ADR-0386 decision 3).
    #[test]
    fn agent_preset_answers_carry_the_name_the_target_resolution_found() {
        let bob = AgentId::new_v4();
        let flat = [
            ("agent.presets", json!({ "target": "bob" })),
            ("agent.presets", json!({ "target": bob.to_string() })),
            (
                "agent.detachPreset",
                json!({ "target": "bob", "presets": ["P.yaml"] }),
            ),
            (
                "agent.reorderPresets",
                json!({ "target": bob.to_string(), "order": [], "base_rev": "b1" }),
            ),
        ];
        for (name, args) in flat {
            let library = FakeLibrary::answering(Ok(port_view(bob)));
            let out = call(&table(vec![(bob, "bob")], library), name, args.clone())
                .unwrap_or_else(|e| panic!("{name} {args}: {e:?}"));
            assert_eq!(out, named_view(bob, "bob"), "{name} {args}");
        }

        let skipped = json!([{ "item": "M.yaml", "code": "not-found", "args": {}, "line": null }]);
        let library = FakeLibrary::answering(Ok(json!({
            "applied": ["R.yaml"], "skipped": skipped, "view": port_view(bob),
        })));
        let out = call(
            &table(vec![(bob, "bob")], library),
            "agent.attachPreset",
            json!({ "target": "bob", "presets": ["R.yaml", "M.yaml"] }),
        )
        .expect("attach");
        assert_eq!(
            out,
            json!({ "applied": ["R.yaml"], "skipped": skipped, "view": named_view(bob, "bob") })
        );
    }

    /// A `target` that points at no agent, or at several, is the resolver's refusal (NOT_FOUND · CONFLICT, the
    /// candidates in the text); a blank one or a blank `base_rev` / `after` is an argument error — the port is never
    /// called.
    #[test]
    fn an_agent_preset_target_that_resolves_to_no_one_agent_is_refused_before_the_port() {
        let (bob, twin_a, twin_b) = (AgentId::new_v4(), AgentId::new_v4(), AgentId::new_v4());
        let roster = vec![(bob, "bob"), (twin_a, "twin"), (twin_b, "twin")];
        let cases = [
            (
                "agent.presets",
                json!({ "target": "nobody" }),
                ErrorCode::NotFound,
            ),
            (
                "agent.presets",
                json!({ "target": "twin" }),
                ErrorCode::Conflict,
            ),
            (
                "agent.attachPreset",
                json!({ "target": "nobody", "presets": ["P.yaml"] }),
                ErrorCode::NotFound,
            ),
            (
                "agent.detachPreset",
                json!({ "target": "twin", "presets": ["P.yaml"] }),
                ErrorCode::Conflict,
            ),
            (
                "agent.reorderPresets",
                json!({ "target": "nobody", "order": [], "base_rev": "b1" }),
                ErrorCode::NotFound,
            ),
            (
                "agent.presets",
                json!({ "target": " " }),
                ErrorCode::InvalidArgument,
            ),
            (
                "agent.attachPreset",
                json!({ "target": "bob", "presets": ["P.yaml"], "after": "" }),
                ErrorCode::InvalidArgument,
            ),
            (
                "agent.detachPreset",
                json!({ "target": "", "presets": ["P.yaml"] }),
                ErrorCode::InvalidArgument,
            ),
            (
                "agent.reorderPresets",
                json!({ "target": "bob", "order": [], "base_rev": " " }),
                ErrorCode::InvalidArgument,
            ),
        ];
        for (name, args, code) in cases {
            let library = Arc::new(FakeLibrary::default());
            let err = call(
                &table(roster.clone(), Arc::clone(&library)),
                name,
                args.clone(),
            )
            .expect_err("refused");
            assert_eq!(err.code(), code, "{name} {args}");
            assert!(
                library.calls().is_empty(),
                "{name} {args}: the port was called"
            );
        }
    }

    /// An agent-preset refusal maps as every library refusal: the class's code (`profile-edit-unavailable` the one
    /// `INTERNAL`), the reason as `args`, the command in the message; a store error as a registry-writing command.
    #[test]
    fn an_agent_preset_refusal_maps_onto_its_code_with_the_reason_as_args() {
        let bob = AgentId::new_v4();
        let cases = [
            (
                "agent.detachPreset",
                json!({ "target": "bob", "presets": ["engram/default"] }),
                refused(
                    PortClass::Conflict,
                    "default-preset-detach",
                    json!({ "agent": bob.to_string() }),
                    None,
                ),
                ErrorCode::Conflict,
            ),
            (
                "agent.reorderPresets",
                json!({ "target": "bob", "order": [], "base_rev": "old" }),
                refused(
                    PortClass::Conflict,
                    "rev-mismatch",
                    json!({ "rev": "new" }),
                    None,
                ),
                ErrorCode::Conflict,
            ),
            (
                "agent.attachPreset",
                json!({ "target": "bob", "presets": ["P.yaml"] }),
                refused(
                    PortClass::Internal,
                    "profile-edit-unavailable",
                    json!({}),
                    None,
                ),
                ErrorCode::Internal,
            ),
            (
                "agent.presets",
                json!({ "target": "bob" }),
                refused(
                    PortClass::NotFound,
                    "not-found",
                    json!({ "agent": bob.to_string() }),
                    None,
                ),
                ErrorCode::NotFound,
            ),
        ];
        for (name, args, refusal, code) in cases {
            let PortRefusal::Refused { reason, .. } = &refusal else {
                unreachable!("built refused")
            };
            let mut expected = Map::new();
            put_reason(&mut expected, reason);
            let library = FakeLibrary::answering(Err(refusal));
            let err = call(&table(vec![(bob, "bob")], library), name, args).expect_err("refused");
            assert_eq!(err.code(), code, "{name}");
            assert_eq!(err.args(), Some(&expected), "{name}");
            assert!(err.message().starts_with(&format!("{name}: ")), "{name}");
        }

        let library = FakeLibrary::answering(Err(PortRefusal::Store(StoreError::Io(
            std::io::Error::other("disk full"),
        ))));
        let err = call(
            &table(vec![(bob, "bob")], library),
            "agent.attachPreset",
            json!({ "target": "bob", "presets": ["P.yaml"] }),
        )
        .expect_err("refused");
        assert_eq!(err.code(), ErrorCode::Internal);
        assert!(err.args().is_none());
    }

    fn move_answer(agents: Vec<AgentId>) -> Value {
        json!({
            "items": [
                {
                    "path": "a.md", "status": "done", "to": "rules/a.md",
                    "trashed": [{ "mount": "library", "bundle": "b1", "path": "rules/a.md" }],
                    "original": null,
                    "left": [{ "path": "rules/x", "code": "name-exists", "args": { "path": "rules/x" }, "line": null }],
                    "code": null, "args": null,
                },
                {
                    "path": "c.md", "status": "skipped", "to": null, "trashed": [], "original": null, "left": [],
                    "code": "in-place", "args": {},
                },
            ],
            "mapping": [{ "from": "library/a.md", "to": "library/rules/a.md", "exact": false }],
            "detached": ["library/s"],
            "written": ["library/p.yaml"],
            "skipped": [{ "path": "library/q.yaml", "code": "yaml-syntax", "args": {}, "line": 2 }],
            "trees": [{ "mount": "library", "git": true, "top": "C:/lib" }],
            "agents": agents,
            "stopped": null,
        })
    }

    fn delete_answer(agents: Vec<AgentId>) -> Value {
        json!({
            "items": [{
                "path": "a.md", "status": "done",
                "trashed": { "mount": "library", "bundle": "b1", "path": "a.md" },
                "code": null, "args": null,
            }],
            "detached": ["library/a.md"],
            "written": ["library/p.yaml"],
            "skipped": [],
            "agents": agents,
            "stopped": {
                "path": null, "code": "profile-store-refused", "args": { "why": "w" }, "message": "m",
            },
        })
    }

    /// ADR-0419: every file verb hands the port exactly the values the caller typed — library paths and trash rows
    /// untouched, omitted options as their stated defaults, an `on_conflict` word typed in another case as the declared
    /// spelling.
    #[test]
    fn each_file_verb_hands_the_port_the_values_the_caller_typed() {
        let row = json!({ "mount": "library", "bundle": "20261123T081500Z-abc123", "path": "rules/a.md" });
        let cases: Vec<(&str, Value, Value, &str, Value)> = vec![
            (
                "library.move",
                json!({ "paths": ["a.md", "\"b c.md\""], "to_folder": "library/rules" }),
                move_answer(Vec::new()),
                "move_items",
                json!({
                    "paths": ["a.md", "\"b c.md\""], "to_folder": "library/rules", "on_conflict": null,
                    "yes": false, "detach_refs": false, "plan_digest": null, "root_generation": null,
                }),
            ),
            (
                "library.move",
                json!({
                    "paths": ["a.md"], "to_folder": "library/rules", "on_conflict": "keepboth", "yes": true,
                    "detach_refs": true, "plan_digest": "d1", "root_generation": 7,
                }),
                move_answer(Vec::new()),
                "move_items",
                json!({
                    "paths": ["a.md"], "to_folder": "library/rules", "on_conflict": "KeepBoth",
                    "yes": true, "detach_refs": true, "plan_digest": "d1", "root_generation": 7,
                }),
            ),
            (
                "library.rename",
                json!({ "path": "library/a.md", "name": "b.md" }),
                move_answer(Vec::new()),
                "rename",
                json!({
                    "path": "library/a.md", "name": "b.md", "yes": false, "detach_refs": false,
                    "plan_digest": null, "root_generation": null,
                }),
            ),
            (
                "library.rename",
                json!({
                    "path": "library/s/SKILL.md", "name": "notes.md", "yes": true, "detach_refs": true,
                    "plan_digest": "d2", "root_generation": 3,
                }),
                move_answer(Vec::new()),
                "rename",
                json!({
                    "path": "library/s/SKILL.md", "name": "notes.md", "yes": true, "detach_refs": true,
                    "plan_digest": "d2", "root_generation": 3,
                }),
            ),
            (
                "library.copy",
                json!({ "paths": ["engram/default"], "to_folder": "library" }),
                json!({ "items": [] }),
                "copy_items",
                json!({
                    "paths": ["engram/default"], "to_folder": "library", "on_conflict": null,
                    "root_generation": null,
                }),
            ),
            (
                "library.copy",
                json!({
                    "paths": ["library/a.md"], "to_folder": "library/b", "on_conflict": "Overwrite",
                    "root_generation": 4,
                }),
                json!({ "items": [] }),
                "copy_items",
                json!({
                    "paths": ["library/a.md"], "to_folder": "library/b", "on_conflict": "Overwrite",
                    "root_generation": 4,
                }),
            ),
            (
                "library.delete",
                json!({ "paths": ["library/a.md"] }),
                delete_answer(Vec::new()),
                "delete",
                json!({
                    "paths": ["library/a.md"], "yes": false, "detach_refs": false, "plan_digest": null,
                    "root_generation": null,
                }),
            ),
            (
                "library.delete",
                json!({
                    "paths": ["library/a.md", "library/d"], "yes": true, "detach_refs": true,
                    "plan_digest": "d3", "root_generation": 9,
                }),
                delete_answer(Vec::new()),
                "delete",
                json!({
                    "paths": ["library/a.md", "library/d"], "yes": true, "detach_refs": true,
                    "plan_digest": "d3", "root_generation": 9,
                }),
            ),
            (
                "library.listTrash",
                json!({}),
                json!({ "mounts": [] }),
                "list_trash",
                Value::Null,
            ),
            (
                "library.restore",
                json!({ "rows": [row.clone()] }),
                json!({ "items": [] }),
                "restore",
                json!({ "rows": [row.clone()], "keep_both": false, "root_generation": null }),
            ),
            (
                "library.restore",
                json!({ "rows": [row.clone()], "keep_both": true, "root_generation": 2 }),
                json!({ "items": [] }),
                "restore",
                json!({ "rows": [row.clone()], "keep_both": true, "root_generation": 2 }),
            ),
            (
                "library.emptyTrash",
                json!({ "rows": [row.clone()] }),
                json!({ "rows": [], "bundles": [] }),
                "empty_trash",
                json!({ "bundles": [], "rows": [row.clone()] }),
            ),
            (
                "library.emptyTrash",
                json!({
                    "bundles": [{ "mount": "library", "bundle": "20261123T081500Z-abc123", "rows": ["rules/a.md", "d"] }],
                }),
                json!({ "rows": [], "bundles": [] }),
                "empty_trash",
                json!({
                    "bundles": [{ "mount": "library", "bundle": "20261123T081500Z-abc123", "rows": ["rules/a.md", "d"] }],
                    "rows": [],
                }),
            ),
            // Neither given reaches the port as two empty lists — `nothing-to-empty` is the service's to refuse.
            (
                "library.emptyTrash",
                json!({}),
                json!({ "rows": [], "bundles": [] }),
                "empty_trash",
                json!({ "bundles": [], "rows": [] }),
            ),
        ];
        for (name, args, answer, method, expected) in cases {
            let library = FakeLibrary::answering(Ok(answer));
            call(&table(Vec::new(), Arc::clone(&library)), name, args.clone())
                .unwrap_or_else(|e| panic!("{name} {args}: {e:?}"));
            assert_eq!(library.calls(), [(method, expected)], "{name} {args}");
        }
    }

    /// ADR-0419: a move, a rename and a delete answer the port's rows unchanged, with the agents named from one roster
    /// read and an id the roster no longer holds dropped; a copy, the trash view, a restore and an emptying are carried as
    /// built.
    #[test]
    fn file_answers_are_carried_with_the_agents_named_from_the_roster() {
        let (bob, ann, gone) = (AgentId::new_v4(), AgentId::new_v4(), AgentId::new_v4());
        let roster = || vec![(bob, "bob"), (ann, "ann")];
        let named = json!([
            { "id": ann.to_string(), "name": "ann" },
            { "id": bob.to_string(), "name": "bob" },
        ]);
        for (name, args) in [
            (
                "library.move",
                json!({ "paths": ["a.md"], "to_folder": "library/rules" }),
            ),
            (
                "library.rename",
                json!({ "path": "library/a.md", "name": "b.md" }),
            ),
        ] {
            let library = FakeLibrary::answering(Ok(move_answer(vec![ann, gone, bob])));
            let out = call(&table(roster(), library), name, args).expect("answer");
            let mut expected = move_answer(Vec::new());
            expected["agents"] = named.clone();
            assert_eq!(out, expected, "{name}");
        }

        let library = FakeLibrary::answering(Ok(delete_answer(vec![gone, ann, bob])));
        let out = call(
            &table(roster(), library),
            "library.delete",
            json!({ "paths": ["library/a.md"] }),
        )
        .expect("delete");
        let mut expected = delete_answer(Vec::new());
        expected["agents"] = named;
        assert_eq!(out, expected);

        let copied = json!({ "items": [{
            "path": "a.md", "status": "skipped", "to": null, "trashed": [],
            "code": "name-taken-during-copy",
            "args": { "path": "library/b/a.md", "staging_row": { "mount": "library", "bundle": "b", "path": "x" } },
        }] });
        let trash = json!({ "mounts": [
            { "mount": "team", "problem": { "code": "trash-unusable", "args": { "mount": "team", "why": "w" } }, "bundles": [] },
            { "mount": "library", "problem": null, "bundles": [{
                "bundle": "20261123T081500Z-abc123", "deleted_at": "2026-11-23T08:15:00Z", "source": "record",
                "why": null, "trouble": null,
                "rows": [{
                    "path": "a.md", "original": "library/a.md", "kind": "root", "state": "present",
                    "entry": "file", "why": null, "staging": false, "intended": null, "restorable": true,
                }],
            }] },
        ] });
        let restored = json!({ "items": [{
            "mount": "library", "bundle": "20261123T081500Z-abc123", "path": "a.md", "status": "conflict",
            "to": null, "code": "name-exists", "args": { "path": "library/a.md" },
        }] });
        let emptied = json!({
            "rows": [
                { "mount": "library", "bundle": "20261123T081500Z-abc123", "path": "a.md", "outcome": "gone", "code": null, "args": null },
                {
                    "mount": "library", "bundle": "20261123T081500Z-abc123", "path": "d", "outcome": "remaining",
                    "code": "library-write-failed", "args": { "path": "d/x.md", "os_error": 5 },
                },
            ],
            "bundles": [
                { "mount": "library", "bundle": "20261123T081500Z-abc123", "outcome": "remaining", "untouched": null },
                {
                    "mount": "team", "bundle": "20261123T081500Z-def456", "outcome": "remaining",
                    "untouched": { "code": "library-unavailable", "args": { "mount": "team", "message": "m" } },
                },
            ],
        });
        for (name, args, answer) in [
            (
                "library.copy",
                json!({ "paths": ["library/a.md"], "to_folder": "library/b" }),
                copied,
            ),
            ("library.listTrash", json!({}), trash),
            (
                "library.restore",
                json!({ "rows": [{ "mount": "library", "bundle": "20261123T081500Z-abc123", "path": "a.md" }] }),
                restored,
            ),
            (
                "library.emptyTrash",
                json!({ "rows": [{ "mount": "library", "bundle": "20261123T081500Z-abc123", "path": "a.md" }] }),
                emptied,
            ),
        ] {
            let library = FakeLibrary::answering(Ok(answer.clone()));
            let out = call(&table(roster(), library), name, args).expect("answer");
            assert_eq!(out, answer, "{name}");
        }
    }

    /// ADR-0419: a file verb's refusal maps as every library refusal does — the plan, its digest and the forms of a
    /// `delete-unconfirmed`, `skill-unconfirmed` or `hook-unconfirmed`, and the stagings a refusal met after copying names, reach the caller
    /// untouched in `args.args`, the message naming the command; an emptying of nothing is INVALID_ARGUMENT.
    #[test]
    fn a_file_refusal_carries_its_plan_forms_and_stagings_untouched() {
        let plan = json!({
            "plan": { "presets": ["library/p.yaml"], "agents": [{ "id": "x", "name": "bob" }] },
            "plan_digest": "d",
            "forms": {
                "keep": { "yes": true },
                "detach": { "yes": true, "detach_refs": true, "plan_digest": "d" },
            },
        });
        let stagings = json!({
            "current": 8,
            "stagings": [{ "staging_row": { "mount": "library", "bundle": "b", "path": "x" } }],
        });
        let cases = [
            (
                "library.delete",
                json!({ "paths": ["library/a.md"] }),
                refused(
                    PortClass::Conflict,
                    "delete-unconfirmed",
                    plan.clone(),
                    None,
                ),
                ErrorCode::Conflict,
            ),
            (
                "library.move",
                json!({ "paths": ["library/s/SKILL.md"], "to_folder": "library" }),
                refused(PortClass::Conflict, "skill-unconfirmed", plan.clone(), None),
                ErrorCode::Conflict,
            ),
            (
                "library.rename",
                json!({ "path": "library/s/SKILL.md", "name": "x.md" }),
                refused(PortClass::Conflict, "skill-unconfirmed", plan, None),
                ErrorCode::Conflict,
            ),
            (
                "library.copy",
                json!({ "paths": ["library/a.md"], "to_folder": "team" }),
                refused(PortClass::Conflict, "root-generation-stale", stagings, None),
                ErrorCode::Conflict,
            ),
            (
                "library.move",
                json!({ "paths": ["library/a.md"], "to_folder": "library/.x" }),
                refused(
                    PortClass::InvalidArgument,
                    "item-path-invalid",
                    json!({ "path": "library/.x", "why": "a dot or temp name" }),
                    None,
                ),
                ErrorCode::InvalidArgument,
            ),
            (
                "library.restore",
                json!({ "rows": [] }),
                refused(
                    PortClass::Unavailable,
                    "library-unavailable",
                    json!({ "message": "m" }),
                    None,
                ),
                ErrorCode::Conflict,
            ),
            (
                "library.listTrash",
                json!({}),
                refused(
                    PortClass::Unavailable,
                    "library-unavailable",
                    json!({ "message": "m" }),
                    None,
                ),
                ErrorCode::Conflict,
            ),
            (
                "library.emptyTrash",
                json!({}),
                refused(
                    PortClass::InvalidArgument,
                    "nothing-to-empty",
                    json!({}),
                    None,
                ),
                ErrorCode::InvalidArgument,
            ),
            (
                "library.emptyTrash",
                json!({ "rows": [] }),
                refused(
                    PortClass::Unavailable,
                    "library-unavailable",
                    json!({ "message": "m" }),
                    None,
                ),
                ErrorCode::Conflict,
            ),
        ];
        for (name, args, refusal, code) in cases {
            let PortRefusal::Refused { reason, .. } = &refusal else {
                unreachable!("built refused")
            };
            let mut expected = Map::new();
            put_reason(&mut expected, reason);
            let library = FakeLibrary::answering(Err(refusal));
            let err = call(&table(Vec::new(), library), name, args).expect_err("refused");
            assert_eq!(err.code(), code, "{name}");
            assert_eq!(err.args(), Some(&expected), "{name}");
            assert!(err.message().starts_with(&format!("{name}: ")), "{name}");
        }
    }

    /// ADR-0419: a blank `to_folder`, `path`, `name` or `plan_digest` is refused before the port is called.
    #[test]
    fn blank_file_values_are_refused_before_the_port_is_called() {
        let cases = [
            (
                "library.move",
                json!({ "paths": ["a.md"], "to_folder": " " }),
            ),
            (
                "library.move",
                json!({ "paths": ["a.md"], "to_folder": "library", "plan_digest": "" }),
            ),
            ("library.rename", json!({ "path": "", "name": "b.md" })),
            (
                "library.rename",
                json!({ "path": "library/a.md", "name": " " }),
            ),
            (
                "library.rename",
                json!({ "path": "library/a.md", "name": "b.md", "plan_digest": " " }),
            ),
            (
                "library.copy",
                json!({ "paths": ["a.md"], "to_folder": "" }),
            ),
            (
                "library.delete",
                json!({ "paths": ["library/a.md"], "plan_digest": "" }),
            ),
        ];
        for (name, args) in cases {
            let library = Arc::new(FakeLibrary::default());
            let err = call(&table(Vec::new(), Arc::clone(&library)), name, args.clone())
                .expect_err("blank refused");
            assert_eq!(err.code(), ErrorCode::InvalidArgument, "{name} {args}");
            assert!(
                library.calls().is_empty(),
                "{name} {args}: the port was called"
            );
        }
    }
}
