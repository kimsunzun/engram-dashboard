//! The library command port — `impl LibraryCommandHost for LibraryService`: the `library.*`, `preset.*` and
//! agent-preset commands agent declares and the `agent.new {presets}` judgement, answered by the bodies of this crate.
//!
//! ## Rules (invariants)
//!
//! - **One body per call, mapped as built.** Each method parses its spellings, calls its one body and converts the
//!   crate-local answer into agent's declared rows; it never calls a second body to rebuild an answer and never reads a
//!   file back (ADR-0384).
//! - **Spellings are parsed with the bodies' own rules** (`edit::lib_path` · `edit::parse_ref`) — here for `refresh` /
//!   `list` / `read` and the preset `path` (`preset.get`: a library path or a built-in), by the body for `newFolder` /
//!   `newFile` / `relink` and for the items and folders a preset command names: a `path` / `folder` / `parent` that is
//!   not a library path refuses `InvalidArgument` · `item-path-invalid`, nothing done.
//! - **Refusals cross 1:1.** [`LibraryRefusal`] becomes agent's `PortRefusal` with the same class and reason (or the
//!   same store error); agent picks the bus code (ADR-0379). `list`'s bare reason (`library-unavailable`) gets class
//!   `Unavailable`.
//! - **Agents cross by id.** The agent-preset views carry the agent's id and no name — agent names it from its own
//!   resolution of `target` (ADR-0386 decision 3); so do a move's, rename's and delete's answers, named from agent's
//!   roster read.
//! - **A file operation answers one flat row per item** (`library.move` … `library.restore`): the service's outcome
//!   becomes `status` plus the done fields, or the reason's `code` and `args` as built — only a skip's reason is made
//!   here, its code per [`Skip`] variant. The trash view's `restorable` is restore's own admission of a row (ADR-0419).
//!   Emptying answers every addressed row and bundle with its outcome word, a row's reason as built (ADR-0408).
// ADR-0311
// ADR-0379
// ADR-0384
// ADR-0386

use std::path::Path;

use serde_json::{json, Value};

use engram_dashboard_agent::backend::settings_check::{
    FolderLoad, FolderLoadKind, FolderLoadSet, FolderUnsure, UnsureWhy,
};
use engram_dashboard_agent::commands::{
    self as declared, launch_notice_row, AgentFolderRow, AgentOutputFormat, CopyItemRow,
    DeleteItemRow, EmptiedBundleRow, EmptiedRow, FolderLoadKindWord, FolderRow, GitTreeRow,
    LibraryAddMountOk, LibraryCopyOk, LibraryEmptyTrashOk, LibraryFolderRow, LibraryFoldersOk,
    LibraryKind, LibraryListOk, LibraryListTrashOk, LibraryNewFileKind, LibraryOnConflict,
    LibraryRefreshOk, LibraryRelinkOk, LibraryRemoveMountOk, LibraryRenameMountOk,
    LibraryRestoreOk, LibraryStatusOk, ListFolderRow, ListedBundleArg, MountStatusRow,
    MountTrashRow, MoveItemRow, MovePairRow, PathReasonRow, PointingRow, PresetAddOk,
    PresetCheckOk, PresetEntryArg, PresetEntryRow, PresetPreviewFolderOk, PresetSection,
    PresetSetBackendOk, PreviewItemRow, PreviewLoadRow, PreviewRefusalRow, PreviewSetRow,
    PreviewSkillNameRow, PreviewUnsureRow, ReasonRow, RefreshErrorRow, RelinkPairRow,
    RestoreItemRow, SkippedRow, TrashAddressRow, TrashBundleRow, TrashProblemRow, TrashViewRow,
    UnavailableMountRow,
};
use engram_dashboard_agent::launch::Reason;
use engram_dashboard_agent::library_commands::{
    problem_row, relink_stop_row, AgentAttachPortAnswer, AgentPresetsPortView, DeletePortAnswer,
    LibraryCommandHost, LibraryReadView, MovePortAnswer, NewAgentPresets, PortClass, PortRefusal,
    PresetDraftInput, PresetEntries, REMOVE_OUTCOME_REMOVED, REMOVE_OUTCOME_SAVED_ONLY,
};
use engram_dashboard_agent::profile::{AgentOutputFormat as CoreOutputFormat, AgentProfile};
use engram_dashboard_agent::types::AgentId;

use crate::agent_edit::{judge_new_presets_without_library, AgentPresetsView, NewPresets};
use crate::catalog::ItemKind;
use crate::config::ConfigHealth;
use crate::create::{NewFileKind, NAME_EXISTS};
use crate::edit::{
    lib_path, parse_ref, read_path, AddEntries, CheckAnswer, FolderPreview, NewItem, PartialEdit,
    PresetDraft, PresetFolder, PresetRow, RemoveEntries, Skipped,
};
use crate::folder::{slash_path, FolderStatus};
use crate::format::ListName;
use crate::mounts::{admits_inside, virtual_path};
use crate::ops::{
    reason, Copied, CopyAnswer, CopyItem, DeleteAnswer, DeleteItem, MoveAnswer, MoveItem, Moved,
    OnConflict, Outcome, RestoreAnswer, RestoreItem, Skip, TrashAddress,
};
use crate::relink::RelinkAnswer;
use crate::rename::{GitTree, RenameAnswer};
use crate::resolve::ResolvedFolder;
use crate::store::EntryKind;
use crate::trash::{
    BundleOutcome, BundleView, Emptied, EmptiedBundle, EmptyAnswer, ListedBundle, MountTrash,
    RootState, RowKind, RowSource, TrashListing, TrashRow,
};
use crate::{
    FoldersAnswer, LibraryRefusal, LibraryService, LibraryStatus, ListAnswer, ListFilter, ListRow,
    MountState, Pointing, ReadAnswer, RefreshReport, RefusalClass, Removal, UnavailableMount,
};

impl LibraryCommandHost for LibraryService {
    fn status(&self) -> Result<LibraryStatusOk, PortRefusal> {
        Ok(status_ok(LibraryService::status(self)))
    }

    fn add_mount(
        &self,
        name: &str,
        path: &str,
        create: bool,
    ) -> Result<LibraryAddMountOk, PortRefusal> {
        LibraryService::add_mount(self, name, Path::new(path), create)
            .map(|added| {
                LibraryAddMountOk::of(status_ok(added.status), pointing_row(added.pointing))
            })
            .map_err(port)
    }

    fn remove_mount(&self, mount: &str) -> Result<LibraryRemoveMountOk, PortRefusal> {
        LibraryService::remove_mount(self, mount)
            .map(|removed| {
                let outcome = match removed.outcome {
                    Removal::Removed => REMOVE_OUTCOME_REMOVED,
                    Removal::SavedOnly => REMOVE_OUTCOME_SAVED_ONLY,
                };
                LibraryRemoveMountOk::of(
                    status_ok(removed.status),
                    pointing_row(removed.now_absent),
                    outcome,
                )
            })
            .map_err(port)
    }

    fn relocate_mount(
        &self,
        mount: &str,
        path: &str,
        create: bool,
    ) -> Result<LibraryStatusOk, PortRefusal> {
        LibraryService::relocate_mount(self, mount, Path::new(path), create)
            .map(status_ok)
            .map_err(port)
    }

    fn rename_mount(
        &self,
        mount: &str,
        to: &str,
        yes: bool,
        root_generation: Option<u64>,
        plan_digest: Option<&str>,
    ) -> Result<LibraryRenameMountOk, PortRefusal> {
        LibraryService::rename_mount(self, mount, to, yes, root_generation, plan_digest)
            .map(rename_ok)
            .map_err(port)
    }

    // ADR-0400 decision 18
    fn change_number(&self) -> Option<u64> {
        Some(LibraryService::change_number(self))
    }

    fn folders(&self) -> Result<LibraryFoldersOk, PortRefusal> {
        LibraryService::folders(self).map(folders_ok).map_err(port)
    }

    fn refresh(&self, folder: Option<&str>) -> Result<LibraryRefreshOk, PortRefusal> {
        let folder = folder.map(read_path).transpose().map_err(port)?;
        Ok(refresh_ok(LibraryService::refresh(self, folder.as_ref())))
    }

    fn list(
        &self,
        folder: Option<&str>,
        query: Option<&str>,
        kinds: &[LibraryKind],
    ) -> Result<LibraryListOk, PortRefusal> {
        let folder = folder.map(read_path).transpose().map_err(port)?;
        let filter = ListFilter {
            query: query.map(str::to_owned),
            kinds: kinds.iter().map(item_kind).collect(),
        };
        self.list_with(folder.as_ref(), &filter)
            .map(list_ok)
            .map_err(|reason| PortRefusal::Refused {
                class: PortClass::Unavailable,
                reason,
            })
    }

    fn read(&self, path: &str) -> Result<LibraryReadView, PortRefusal> {
        let path = read_path(path).map_err(port)?;
        LibraryService::read(self, &path)
            .map(read_view)
            .map_err(port)
    }

    fn new_folder(
        &self,
        parent: Option<&str>,
        name: &str,
        root_generation: Option<u64>,
    ) -> Result<String, PortRefusal> {
        LibraryService::new_folder(self, parent, name, root_generation)
            .map(|path| path.as_str().to_owned())
            .map_err(port)
    }

    fn new_file(
        &self,
        folder: Option<&str>,
        name: &str,
        kind: LibraryNewFileKind,
        root_generation: Option<u64>,
    ) -> Result<String, PortRefusal> {
        let kind = match kind {
            LibraryNewFileKind::Markdown => NewFileKind::Markdown,
            LibraryNewFileKind::Settings => NewFileKind::Settings,
            LibraryNewFileKind::Preset => NewFileKind::Preset,
        };
        LibraryService::new_file(self, folder, name, kind, root_generation)
            .map(|path| path.as_str().to_owned())
            .map_err(port)
    }

    fn relink(
        &self,
        from: &str,
        to: &str,
        root_generation: Option<u64>,
    ) -> Result<LibraryRelinkOk, PortRefusal> {
        LibraryService::relink(self, from, to, root_generation)
            .map(relink_ok)
            .map_err(port)
    }

    fn preset_get(&self, path: &str) -> Result<declared::PresetView, PortRefusal> {
        let preset = parse_ref(path).map_err(|reason| PortRefusal::Refused {
            class: PortClass::InvalidArgument,
            reason,
        })?;
        LibraryService::preset_get(self, &preset)
            .map(preset_view)
            .map_err(port)
    }

    fn preset_check(
        &self,
        path: &str,
        draft: &PresetDraftInput,
    ) -> Result<PresetCheckOk, PortRefusal> {
        let preset = lib_path(path).map_err(port)?;
        LibraryService::preset_check(self, &preset, &preset_draft(draft))
            .map(check_ok)
            .map_err(port)
    }

    fn preset_add(
        &self,
        path: &str,
        entries: &PresetEntries,
        backend: Option<&str>,
        after: Option<&str>,
        root_generation: Option<u64>,
    ) -> Result<PresetAddOk, PortRefusal> {
        let preset = lib_path(path).map_err(port)?;
        let entries = match entries {
            PresetEntries::Items(items) => AddEntries::Items(
                items
                    .iter()
                    .map(|item| NewItem {
                        path: item.clone(),
                        backend: backend.map(str::to_owned),
                    })
                    .collect(),
            ),
            PresetEntries::Folders(folders) => AddEntries::Folders(folders.clone()),
        };
        LibraryService::preset_add(self, &preset, &entries, after, root_generation)
            .map(|edit| {
                let (applied, skipped, view) = partial(edit);
                PresetAddOk {
                    applied,
                    skipped,
                    view,
                }
            })
            .map_err(port)
    }

    fn preset_remove(
        &self,
        path: &str,
        entries: &PresetEntries,
        root_generation: Option<u64>,
    ) -> Result<declared::PresetView, PortRefusal> {
        let preset = lib_path(path).map_err(port)?;
        let entries = match entries {
            PresetEntries::Items(items) => RemoveEntries::Items(items.clone()),
            PresetEntries::Folders(folders) => RemoveEntries::Folders(folders.clone()),
        };
        LibraryService::preset_remove(self, &preset, &entries, root_generation)
            .map(preset_view)
            .map_err(port)
    }

    fn preset_reorder(
        &self,
        path: &str,
        section: PresetSection,
        order: &[String],
        base_rev: &str,
        root_generation: Option<u64>,
    ) -> Result<declared::PresetView, PortRefusal> {
        let preset = lib_path(path).map_err(port)?;
        let section = match section {
            PresetSection::Presets => ListName::Presets,
            PresetSection::Instructions => ListName::Instructions,
            PresetSection::Skills => ListName::Skills,
            PresetSection::Settings => ListName::Settings,
            PresetSection::Folders => ListName::Folders,
        };
        LibraryService::preset_reorder(self, &preset, section, order, base_rev, root_generation)
            .map(preset_view)
            .map_err(port)
    }

    fn preset_set_backend(
        &self,
        path: &str,
        items: &[String],
        backend: Option<&str>,
        root_generation: Option<u64>,
    ) -> Result<PresetSetBackendOk, PortRefusal> {
        let preset = lib_path(path).map_err(port)?;
        LibraryService::preset_set_backend(self, &preset, items, backend, root_generation)
            .map(|edit| {
                let (applied, skipped, view) = partial(edit);
                PresetSetBackendOk {
                    applied,
                    skipped,
                    view,
                }
            })
            .map_err(port)
    }

    fn preset_save(
        &self,
        path: &str,
        draft: &PresetDraftInput,
        base_rev: Option<&str>,
        root_generation: Option<u64>,
    ) -> Result<declared::PresetView, PortRefusal> {
        let preset = lib_path(path).map_err(port)?;
        LibraryService::preset_save(
            self,
            &preset,
            &preset_draft(draft),
            base_rev,
            root_generation,
        )
        .map(preset_view)
        .map_err(port)
    }

    fn preview_folder(
        &self,
        folder: &str,
        backend: Option<&str>,
        mode: Option<AgentOutputFormat>,
    ) -> Result<PresetPreviewFolderOk, PortRefusal> {
        let mode = mode.map(|mode| match mode {
            AgentOutputFormat::Terminal => CoreOutputFormat::Terminal,
            AgentOutputFormat::StreamJson => CoreOutputFormat::StreamJson,
        });
        LibraryService::preview_folder(self, folder, backend, mode)
            .map(preview_ok)
            .map_err(port)
    }

    fn agent_presets(&self, agent: AgentId) -> Result<AgentPresetsPortView, PortRefusal> {
        LibraryService::agent_presets(self, agent)
            .map(agent_view)
            .map_err(port)
    }

    fn attach_preset(
        &self,
        agent: AgentId,
        presets: &[String],
        after: Option<&str>,
        root_generation: Option<u64>,
    ) -> Result<AgentAttachPortAnswer, PortRefusal> {
        LibraryService::attach_preset(self, agent, presets, after, root_generation)
            .map(|attach| AgentAttachPortAnswer {
                applied: attach.applied,
                skipped: attach.skipped.into_iter().map(skipped_row).collect(),
                view: agent_view(attach.view),
            })
            .map_err(port)
    }

    fn detach_preset(
        &self,
        agent: AgentId,
        presets: &[String],
        yes: bool,
        root_generation: Option<u64>,
    ) -> Result<AgentPresetsPortView, PortRefusal> {
        LibraryService::detach_preset(self, agent, presets, yes, root_generation)
            .map(agent_view)
            .map_err(port)
    }

    fn reorder_presets(
        &self,
        agent: AgentId,
        order: &[String],
        base_rev: &str,
        root_generation: Option<u64>,
    ) -> Result<AgentPresetsPortView, PortRefusal> {
        LibraryService::reorder_presets(self, agent, order, base_rev, root_generation)
            .map(agent_view)
            .map_err(port)
    }

    fn judge_new_presets(
        &self,
        profile: &AgentProfile,
        presets: &[String],
        root_generation: Option<u64>,
    ) -> Result<NewAgentPresets, PortRefusal> {
        LibraryService::judge_new_presets(self, profile, presets, root_generation)
            .map(new_presets)
            .map_err(port)
    }

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
    ) -> Result<MovePortAnswer, PortRefusal> {
        let paths: Vec<&str> = paths.iter().map(String::as_str).collect();
        LibraryService::move_items(
            self,
            &paths,
            to_folder,
            on_conflict.map(on_conflict_of),
            yes,
            detach_refs,
            plan_digest,
            root_generation,
        )
        .map(|answer| move_answer("library.move", answer))
        .map_err(port)
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
        LibraryService::rename(
            self,
            path,
            name,
            yes,
            detach_refs,
            plan_digest,
            root_generation,
        )
        .map(|answer| move_answer("library.rename", answer))
        .map_err(port)
    }

    fn copy_items(
        &self,
        paths: &[String],
        to_folder: &str,
        on_conflict: Option<LibraryOnConflict>,
        root_generation: Option<u64>,
    ) -> Result<LibraryCopyOk, PortRefusal> {
        let paths: Vec<&str> = paths.iter().map(String::as_str).collect();
        LibraryService::copy_items(
            self,
            &paths,
            to_folder,
            on_conflict.map(on_conflict_of),
            root_generation,
        )
        .map(copy_ok)
        .map_err(port)
    }

    fn delete(
        &self,
        paths: &[String],
        yes: bool,
        detach_refs: bool,
        plan_digest: Option<&str>,
        root_generation: Option<u64>,
    ) -> Result<DeletePortAnswer, PortRefusal> {
        let paths: Vec<&str> = paths.iter().map(String::as_str).collect();
        LibraryService::delete(self, &paths, yes, detach_refs, plan_digest, root_generation)
            .map(delete_answer)
            .map_err(port)
    }

    fn list_trash(&self) -> Result<LibraryListTrashOk, PortRefusal> {
        Ok(trash_ok(LibraryService::trash_view(self)))
    }

    fn restore(
        &self,
        rows: &[TrashAddressRow],
        keep_both: bool,
        root_generation: Option<u64>,
    ) -> Result<LibraryRestoreOk, PortRefusal> {
        let rows: Vec<TrashAddress> = rows.iter().map(trash_address).collect();
        LibraryService::restore(self, &rows, keep_both, root_generation)
            .map(restore_ok)
            .map_err(port)
    }

    fn empty_trash(
        &self,
        bundles: &[ListedBundleArg],
        rows: &[TrashAddressRow],
    ) -> Result<LibraryEmptyTrashOk, PortRefusal> {
        let bundles: Vec<ListedBundle> = bundles
            .iter()
            .map(|listed| ListedBundle {
                mount: listed.mount.clone(),
                bundle: listed.bundle.clone(),
                rows: listed.rows.clone(),
            })
            .collect();
        let rows: Vec<TrashAddress> = rows.iter().map(trash_address).collect();
        LibraryService::empty_trash(self, &bundles, &rows)
            .map(emptied_ok)
            .map_err(port)
    }
}

/// The skip code of a path given with a folder above it in the same call — only the folder counts (`args` =
/// `{folder}`).
// ADR-0419
const INSIDE_GIVEN_FOLDER: &str = "inside-given-folder";
/// The skip code of a move into the folder the item is in, or a rename to its own spelling (`args` = `{}`).
// ADR-0419
const IN_PLACE: &str = "in-place";
/// The skip code of a name taken at the destination during a staging copy under `on_conflict` `Skip` (`args` = `{path}`
/// plus the staging's keys — `staging_row`, or `staging` · `staging_disk` · `staging_kept`).
// ADR-0416
// ADR-0419
const NAME_TAKEN_DURING_COPY: &str = "name-taken-during-copy";

/// The `agent.new {presets}` judgement of an assembly that runs no library service (the daemon's `NoLibrary`): the
/// attach rule with no catalog — a library preset refuses the whole call `Unavailable` · `library-unavailable`
/// (`message` = `why`); `engram/default` and the other spellings are judged as the service judges them.
// ADR-0386
pub fn judge_new_presets_with_no_library(
    profile: &AgentProfile,
    presets: &[String],
    why: &str,
) -> Result<NewAgentPresets, PortRefusal> {
    judge_new_presets_without_library(profile, presets, why)
        .map(new_presets)
        .map_err(port)
}

/// [`LibraryRefusal`] → the port refusal, 1:1.
fn port(refusal: LibraryRefusal) -> PortRefusal {
    match refusal {
        LibraryRefusal::Refused { class, reason } => PortRefusal::Refused {
            class: match class {
                RefusalClass::Conflict => PortClass::Conflict,
                RefusalClass::NotFound => PortClass::NotFound,
                RefusalClass::InvalidArgument => PortClass::InvalidArgument,
                RefusalClass::Unavailable => PortClass::Unavailable,
                RefusalClass::Internal => PortClass::Internal,
            },
            reason,
        },
        LibraryRefusal::Store(error) => PortRefusal::Store(error),
    }
}

/// A count as the declared `u32` — a library never holds 2^32 of anything, so the cap is never reached in practice.
fn count(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

fn status_ok(status: LibraryStatus) -> LibraryStatusOk {
    let (config, config_why) = match status.config {
        ConfigHealth::Ok => ("ok", None),
        ConfigHealth::ReadOnly { why } => ("read_only", Some(why)),
        ConfigHealth::Corrupt => ("corrupt", None),
        ConfigHealth::Unsaved { why } => ("unsaved", Some(why)),
    };
    LibraryStatusOk {
        daemon_id: status.daemon_id,
        mounts: status
            .mounts
            .into_iter()
            .map(|mount| {
                let (state, state_why) = mount_state_words(Some(mount.state));
                MountStatusRow {
                    name: mount.name,
                    path: mount.folder.display().to_string(),
                    default: mount.default,
                    builtin: mount.builtin,
                    state: state.unwrap_or_default(),
                    state_why,
                }
            })
            .collect(),
        config: config.to_owned(),
        config_why,
        suggested_new: status.suggested_new.display().to_string(),
        root_generation: status.root_generation,
    }
}

fn pointing_row(pointing: Pointing) -> PointingRow {
    PointingRow {
        presets: count(pointing.presets),
        agents: count(pointing.agents),
    }
}

fn folders_ok(answer: FoldersAnswer) -> LibraryFoldersOk {
    LibraryFoldersOk {
        folders: answer
            .folders
            .into_iter()
            .map(|folder| {
                let (state, state_why) = mount_state_words(folder.mount_state);
                LibraryFolderRow {
                    path: folder.path,
                    name: folder.name,
                    skill: folder.skill,
                    state,
                    state_why,
                }
            })
            .collect(),
        unavailable: answer
            .unavailable
            .into_iter()
            .map(unavailable_row)
            .collect(),
    }
}

/// A mount's state as the wire words of `library.status` (`ok` · `missing` · `unreadable` + why); `None` = not a
/// mount's row.
// ADR-0400 decision 4
fn mount_state_words(state: Option<MountState>) -> (Option<String>, Option<String>) {
    let Some(state) = state else {
        return (None, None);
    };
    let word = Some(state.word().to_owned());
    match state {
        MountState::Ok => (word, None),
        MountState::Missing { why } | MountState::Unreadable { why } => (word, Some(why)),
    }
}

fn unavailable_row(left: UnavailableMount) -> UnavailableMountRow {
    let (state, why) = mount_state_words(Some(left.state));
    UnavailableMountRow {
        mount: left.mount,
        state: state.unwrap_or_default(),
        why: why.unwrap_or_default(),
    }
}

fn refresh_ok(report: RefreshReport) -> LibraryRefreshOk {
    LibraryRefreshOk {
        changed: count(report.changed),
        problems: report
            .problems
            .into_iter()
            .map(|(path, reason)| path_reason(path.as_str().to_owned(), reason))
            .collect(),
        no_kind: report
            .no_kind
            .iter()
            .map(|path| path.as_str().to_owned())
            .collect(),
        skipped_links: report
            .skipped_links
            .iter()
            .map(|path| path.as_str().to_owned())
            .collect(),
        errors: report
            .errors
            .into_iter()
            .map(|(path, why)| RefreshErrorRow {
                path: path.map(|path| path.as_str().to_owned()),
                why,
            })
            .collect(),
    }
}

fn list_ok(answer: ListAnswer) -> LibraryListOk {
    LibraryListOk {
        folder: answer.folder.map(|folder| folder.as_str().to_owned()),
        rows: answer.rows.into_iter().map(list_row).collect(),
        folders: answer
            .folders
            .into_iter()
            .map(|folder| {
                let (state, state_why) = mount_state_words(folder.mount_state);
                ListFolderRow {
                    path: folder.path,
                    name: folder.name,
                    state,
                    state_why,
                }
            })
            .collect(),
        unavailable: answer
            .unavailable
            .into_iter()
            .map(unavailable_row)
            .collect(),
    }
}

fn read_view(answer: ReadAnswer) -> LibraryReadView {
    LibraryReadView {
        row: list_row(answer.row),
        absolute: answer.absolute.display().to_string(),
        presets_direct: answer.presets_direct,
        presets_indirect: answer.presets_indirect,
        agents_direct: answer.agents_direct,
        agents_indirect: answer.agents_indirect,
        excerpt: answer.excerpt,
        settings_fields: answer.settings_fields,
    }
}

fn relink_ok(answer: RelinkAnswer) -> LibraryRelinkOk {
    LibraryRelinkOk {
        fixed: answer
            .fixed
            .into_iter()
            .map(|(from, to)| RelinkPairRow { from, to })
            .collect(),
        more: count(answer.more),
        written: answer.written,
        skipped: answer
            .skipped
            .into_iter()
            .map(|skip| path_reason(skip.path, skip.reason))
            .collect(),
        stopped: answer
            .stopped
            .map(|stop| relink_stop_row("library.relink", stop.path, port(stop.refusal))),
    }
}

fn rename_ok(answer: RenameAnswer) -> LibraryRenameMountOk {
    LibraryRenameMountOk::of(
        status_ok(answer.status),
        answer
            .fixed
            .into_iter()
            .map(|(from, to)| RelinkPairRow { from, to })
            .collect(),
        answer.written,
        answer
            .skipped
            .into_iter()
            .map(|skip| path_reason(skip.path, skip.reason))
            .collect(),
        answer
            .stopped
            .map(|stop| relink_stop_row("library.renameMount", stop.path, port(stop.refusal))),
        pointing_row(answer.already_pointing),
    )
}

fn on_conflict_of(word: LibraryOnConflict) -> OnConflict {
    match word {
        LibraryOnConflict::Overwrite => OnConflict::Overwrite,
        LibraryOnConflict::Skip => OnConflict::Skip,
        LibraryOnConflict::KeepBoth => OnConflict::KeepBoth,
    }
}

fn address_row(row: TrashAddress) -> TrashAddressRow {
    TrashAddressRow {
        mount: row.mount,
        bundle: row.bundle,
        path: row.path,
    }
}

fn trash_address(row: &TrashAddressRow) -> TrashAddress {
    TrashAddress {
        mount: row.mount.clone(),
        bundle: row.bundle.clone(),
        path: row.path.clone(),
    }
}

/// One item's outcome as a flat row's `status`, `code` and `args`, and what a `done` item came to. A non-`done` row
/// carries its reason's code and args as built — a skip's reason is made here (its code per [`Skip`] variant); a
/// per-item reason never has a `line`, so none is carried.
// ADR-0419
fn outcome_parts<T>(outcome: Outcome<T>) -> (String, Option<String>, Value, Option<T>) {
    let (status, why, done) = match outcome {
        Outcome::Done(done) => ("done", None, Some(done)),
        Outcome::Skipped(skip) => ("skipped", Some(skip_reason(skip)), None),
        Outcome::Conflict(why) => ("conflict", Some(why), None),
        Outcome::Failed(why) => ("failed", Some(why), None),
    };
    let (code, args) = match why {
        Some(Reason { code, args, .. }) => (Some(code), args),
        None => (None, Value::Null),
    };
    (status.to_owned(), code, args, done)
}

/// Why an item was skipped, as a reason: [`INSIDE_GIVEN_FOLDER`] `{folder}` · [`IN_PLACE`] `{}` · `name-exists`
/// `{path}` · [`NAME_TAKEN_DURING_COPY`] `{path, …staging}`.
// ADR-0419
fn skip_reason(skip: Skip) -> Reason {
    match skip {
        Skip::Inside(folder) => reason(INSIDE_GIVEN_FOLDER, json!({ "folder": folder })),
        Skip::InPlace => reason(IN_PLACE, json!({})),
        Skip::Taken(path) => reason(NAME_EXISTS, json!({ "path": path })),
        Skip::TakenDuringCopy { path, mut staging } => {
            staging["path"] = json!(path);
            reason(NAME_TAKEN_DURING_COPY, staging)
        }
    }
}

/// A move's or rename's answer in declared rows, the agents by id; `command` names a `stopped` row's message.
// ADR-0419
fn move_answer(command: &str, answer: MoveAnswer) -> MovePortAnswer {
    MovePortAnswer {
        items: answer
            .items
            .into_iter()
            .map(|MoveItem { path, outcome }| {
                let (status, code, args, done) = outcome_parts(outcome);
                let (to, trashed, original, left) = match done {
                    Some(Moved {
                        to,
                        trashed,
                        original,
                        left,
                    }) => (
                        Some(to),
                        trashed.into_iter().map(address_row).collect(),
                        original.map(address_row),
                        left.into_iter()
                            .map(|(path, why)| path_reason(path, why))
                            .collect(),
                    ),
                    None => (None, Vec::new(), None, Vec::new()),
                };
                MoveItemRow {
                    path,
                    status,
                    to,
                    trashed,
                    original,
                    left,
                    code,
                    args,
                }
            })
            .collect(),
        mapping: answer
            .mapping
            .into_iter()
            .map(|(from, to, exact)| MovePairRow { from, to, exact })
            .collect(),
        detached: answer.detached,
        written: answer.written,
        skipped: answer
            .skipped
            .into_iter()
            .map(|skip| path_reason(skip.path, skip.reason))
            .collect(),
        trees: answer.trees.into_iter().map(git_tree_row).collect(),
        agents: answer.agents,
        stopped: answer
            .stopped
            .map(|stop| relink_stop_row(command, stop.path, port(stop.refusal))),
    }
}

fn git_tree_row((mount, tree): (String, GitTree)) -> GitTreeRow {
    let (git, top) = match tree {
        GitTree::At(top) => (Some(true), Some(top.display().to_string())),
        GitTree::Outside => (Some(false), None),
        GitTree::Unknown => (None, None),
    };
    GitTreeRow { mount, git, top }
}

fn copy_ok(answer: CopyAnswer) -> LibraryCopyOk {
    LibraryCopyOk {
        items: answer
            .items
            .into_iter()
            .map(|CopyItem { path, outcome }| {
                let (status, code, args, done) = outcome_parts(outcome);
                let (to, trashed) = match done {
                    Some(Copied { to, trashed }) => {
                        (Some(to), trashed.into_iter().map(address_row).collect())
                    }
                    None => (None, Vec::new()),
                };
                CopyItemRow {
                    path,
                    status,
                    to,
                    trashed,
                    code,
                    args,
                }
            })
            .collect(),
    }
}

fn delete_answer(answer: DeleteAnswer) -> DeletePortAnswer {
    DeletePortAnswer {
        items: answer
            .items
            .into_iter()
            .map(|DeleteItem { path, outcome }| {
                let (status, code, args, done) = outcome_parts(outcome);
                DeleteItemRow {
                    path,
                    status,
                    trashed: done.map(address_row),
                    code,
                    args,
                }
            })
            .collect(),
        detached: answer.detached,
        written: answer.written,
        skipped: answer
            .skipped
            .into_iter()
            .map(|skip| path_reason(skip.path, skip.reason))
            .collect(),
        agents: answer.agents,
        stopped: answer
            .stopped
            .map(|stop| relink_stop_row("library.delete", stop.path, port(stop.refusal))),
    }
}

fn restore_ok(answer: RestoreAnswer) -> LibraryRestoreOk {
    LibraryRestoreOk {
        items: answer
            .items
            .into_iter()
            .map(|RestoreItem { row, outcome }| {
                let (status, code, args, to) = outcome_parts(outcome);
                RestoreItemRow {
                    mount: row.mount,
                    bundle: row.bundle,
                    path: row.path,
                    status,
                    to,
                    code,
                    args,
                }
            })
            .collect(),
    }
}

/// An emptying's answer as declared: each row's and bundle's outcome as its word; a row that is not gone carries its
/// reason's code and args as built (no `line` — a per-row reason has none), a bundle left untouched its reason.
// ADR-0408 decision 2
// ADR-0419
fn emptied_ok(answer: EmptyAnswer) -> LibraryEmptyTrashOk {
    let bundle_word = |outcome: BundleOutcome| match outcome {
        BundleOutcome::Gone => "gone",
        BundleOutcome::Remaining => "remaining",
        BundleOutcome::NotFound => "not_found",
    };
    LibraryEmptyTrashOk {
        rows: answer
            .rows
            .into_iter()
            .map(|crate::trash::EmptiedRow { row, outcome }| {
                let (outcome, why) = match outcome {
                    Emptied::Gone => ("gone", None),
                    Emptied::Remaining(why) => ("remaining", Some(why)),
                    Emptied::NotFound(why) => ("not_found", Some(why)),
                };
                let (code, args) =
                    why.map_or((None, Value::Null), |why| (Some(why.code), why.args));
                EmptiedRow {
                    mount: row.mount,
                    bundle: row.bundle,
                    path: row.path,
                    outcome: outcome.to_owned(),
                    code,
                    args,
                }
            })
            .collect(),
        bundles: answer
            .bundles
            .into_iter()
            .map(
                |EmptiedBundle {
                     mount,
                     bundle,
                     outcome,
                     untouched,
                 }| EmptiedBundleRow {
                    mount,
                    bundle,
                    outcome: bundle_word(outcome).to_owned(),
                    untouched: untouched.map(|why| TrashProblemRow {
                        code: why.code,
                        args: why.args,
                    }),
                },
            )
            .collect(),
    }
}

fn trash_ok(listing: TrashListing) -> LibraryListTrashOk {
    LibraryListTrashOk {
        mounts: listing.mounts.iter().map(mount_trash_row).collect(),
    }
}

fn mount_trash_row(trash: &MountTrash) -> MountTrashRow {
    let (problem, bundles) = match &trash.view {
        Ok(bundles) => (
            None,
            bundles
                .iter()
                .map(|bundle| bundle_row(trash, bundle))
                .collect(),
        ),
        Err(why) => (
            Some(TrashProblemRow {
                code: why.code.clone(),
                args: why.args.clone(),
            }),
            Vec::new(),
        ),
    };
    MountTrashRow {
        mount: trash.mount.as_str().to_owned(),
        problem,
        bundles,
    }
}

fn bundle_row(trash: &MountTrash, bundle: &BundleView) -> TrashBundleRow {
    let (source, why) = match &bundle.source {
        RowSource::Record => ("record", None),
        RowSource::NoRecord => ("no_record", None),
        RowSource::Unreadable { why } => ("record_unreadable", Some(why.clone())),
        RowSource::Unchecked => ("unchecked", None),
    };
    TrashBundleRow {
        bundle: bundle.id.as_str().to_owned(),
        deleted_at: bundle.id.deleted_at_rfc3339(),
        source: source.to_owned(),
        why,
        trouble: bundle.trouble.clone(),
        rows: bundle.rows.iter().map(|row| view_row(trash, row)).collect(),
    }
}

/// A trash row as declared — `restorable` as `library.restore` admits a row before anything else: a root or found row,
/// no staging, present, and an original that holds no dot or temp name (ops `restorable`).
// ADR-0419
fn view_row(trash: &MountTrash, row: &TrashRow) -> TrashViewRow {
    let original = trash.original(row);
    let (state, entry, why) = match &row.state {
        RootState::Present(kind) => {
            let entry = match kind {
                EntryKind::File => "file",
                EntryKind::Folder => "folder",
                EntryKind::Link => "link",
            };
            ("present", Some(entry.to_owned()), None)
        }
        RootState::ThroughLink => ("through_link", None, None),
        RootState::Failed(why) => ("failed", None, Some(why.clone())),
    };
    let kind = match row.kind {
        RowKind::Root => "root",
        RowKind::Found => "found",
        RowKind::Other => "other",
    };
    let restorable = row.kind != RowKind::Other
        && !row.staging
        && matches!(row.state, RootState::Present(_))
        && admits_inside(&original);
    TrashViewRow {
        path: row.path.as_str().to_owned(),
        original: original.as_str().to_owned(),
        kind: kind.to_owned(),
        state: state.to_owned(),
        entry,
        why,
        staging: row.staging,
        intended: row
            .intended
            .as_ref()
            .map(|path| virtual_path(&trash.mount, Some(path)).as_str().to_owned()),
        restorable,
    }
}

fn preset_view(view: crate::edit::PresetView) -> declared::PresetView {
    let entries = |rows: Vec<PresetRow>| rows.into_iter().map(entry_row).collect();
    declared::PresetView {
        path: view.path,
        rev: view.rev,
        description: view.description,
        presets: entries(view.presets),
        instructions: entries(view.instructions),
        skills: entries(view.skills),
        settings: entries(view.settings),
        folders: view.folders.into_iter().map(folder_row).collect(),
        conflicts: view.conflicts.into_iter().map(problem_row).collect(),
        problems: view.problems.into_iter().map(problem_row).collect(),
        notices: view.notices.into_iter().map(launch_notice_row).collect(),
        agents: count(view.agents),
    }
}

fn entry_row(row: PresetRow) -> PresetEntryRow {
    PresetEntryRow {
        path: row.path,
        backend: row.backend,
        line: row.line,
        state: row.state.map(reason_row),
    }
}

fn folder_row(folder: PresetFolder) -> FolderRow {
    let (status, status_why) = status_words(folder.status);
    FolderRow {
        path: folder.path,
        line: folder.line,
        status,
        status_why,
    }
}

/// A folder stat as a word, and the reason only `unreadable` carries; `None` = not stat'ed.
fn status_words(status: Option<FolderStatus>) -> (Option<String>, Option<String>) {
    let (word, why) = match status {
        None => return (None, None),
        Some(FolderStatus::Ok) => ("ok", None),
        Some(FolderStatus::Missing) => ("missing", None),
        Some(FolderStatus::NotADirectory) => ("not_a_directory", None),
        Some(FolderStatus::Unreadable(why)) => ("unreadable", Some(why)),
    };
    (Some(word.to_owned()), why)
}

/// A partial edit's answer as the declared `{applied, skipped, view}` parts.
fn partial(edit: PartialEdit) -> (Vec<String>, Vec<SkippedRow>, declared::PresetView) {
    let skipped = edit.skipped.into_iter().map(skipped_row).collect();
    (edit.applied, skipped, preset_view(edit.view))
}

/// A skip as the declared row — the reason's `code` · `args` · `line` beside the item.
fn skipped_row(Skipped { item, reason }: Skipped) -> SkippedRow {
    SkippedRow {
        item,
        code: reason.code,
        args: reason.args,
        line: reason.line,
    }
}

/// The `agent.presets` view in declared rows, by id. ★Destructured (without `..`)★ — a field added to the body's view
/// stops compilation here instead of being dropped.
fn agent_view(view: AgentPresetsView) -> AgentPresetsPortView {
    let AgentPresetsView {
        agent,
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
    AgentPresetsPortView {
        agent_id: agent,
        default_preset,
        presets,
        cwd: cwd.to_string_lossy().into_owned(),
        workspace_lock: workspace_lock.map(|lock| lock.word().to_owned()),
        backend,
        mode: declared_mode(mode),
        base_rev,
        takes_presets,
        folders: folders.into_iter().map(agent_folder_row).collect(),
        conflicts: conflicts.into_iter().map(problem_row).collect(),
        problems: problems.into_iter().map(problem_row).collect(),
        notices: notices.into_iter().map(launch_notice_row).collect(),
    }
}

fn agent_folder_row(folder: ResolvedFolder) -> AgentFolderRow {
    let (status, status_why) = status_words(folder.status);
    AgentFolderRow {
        path: folder.path.to_string_lossy().into_owned(),
        via: folder.via,
        line: folder.line,
        status,
        status_why,
    }
}

fn new_presets(judged: NewPresets) -> NewAgentPresets {
    NewAgentPresets {
        attachments: judged.attachments,
        skipped: judged.skipped.into_iter().map(skipped_row).collect(),
        change: judged.change,
    }
}

fn declared_mode(mode: CoreOutputFormat) -> AgentOutputFormat {
    match mode {
        CoreOutputFormat::Terminal => AgentOutputFormat::Terminal,
        CoreOutputFormat::StreamJson => AgentOutputFormat::StreamJson,
    }
}

fn check_ok(answer: CheckAnswer) -> PresetCheckOk {
    PresetCheckOk {
        save_refuses: answer.save_refuses,
        folders: answer.folders.into_iter().map(folder_row).collect(),
        conflicts: answer.conflicts.into_iter().map(problem_row).collect(),
        problems: answer.problems.into_iter().map(problem_row).collect(),
    }
}

fn preset_draft(draft: &PresetDraftInput) -> PresetDraft {
    let items = |entries: &[PresetEntryArg]| {
        entries
            .iter()
            .map(|entry| NewItem {
                path: entry.path.clone(),
                backend: entry.backend.clone(),
            })
            .collect()
    };
    PresetDraft {
        presets: items(&draft.presets),
        instructions: items(&draft.instructions),
        skills: items(&draft.skills),
        settings: items(&draft.settings),
        folders: draft.folders.clone(),
    }
}

fn preview_ok(preview: FolderPreview) -> PresetPreviewFolderOk {
    PresetPreviewFolderOk {
        path: preview.path.map(|path| path.to_string_lossy().into_owned()),
        refused: preview.refused.map(|refused| PreviewRefusalRow {
            path: refused.path,
            code: refused.code.to_owned(),
            why: refused.why,
        }),
        loads: preview
            .loads
            .into_iter()
            .map(|load| PreviewLoadRow {
                backend: load.backend.to_owned(),
                mode: declared_mode(load.mode),
                predicted: load.predicted,
                set: preview_set(load.set),
                unread: load.unread,
            })
            .collect(),
        truncated: preview.truncated,
        skipped: preview.skipped,
        unlisted: preview.unlisted,
    }
}

/// The agent port's load set as rows — folder-relative paths `/`-separated, as the omission notice spells them.
fn preview_set(set: FolderLoadSet) -> PreviewSetRow {
    let items = |loads: Vec<FolderLoad>| -> Vec<PreviewItemRow> {
        loads
            .into_iter()
            .map(|load| PreviewItemRow {
                path: slash_path(&load.path),
                kind: load_kind(load.kind),
            })
            .collect()
    };
    PreviewSetRow {
        loads: items(set.loads),
        omitted: items(set.omitted),
        carried: items(set.carried),
        unsure: set.unsure.into_iter().map(unsure_row).collect(),
        malformed: set
            .malformed
            .into_iter()
            .map(|malformed| path_reason(slash_path(&malformed.path), malformed.reason))
            .collect(),
        skill_names: set
            .skill_names
            .into_iter()
            .map(|skill| PreviewSkillNameRow {
                path: slash_path(&skill.path),
                name: skill.name,
            })
            .collect(),
    }
}

fn load_kind(kind: FolderLoadKind) -> FolderLoadKindWord {
    match kind {
        FolderLoadKind::Instructions => FolderLoadKindWord::Instructions,
        FolderLoadKind::Rule => FolderLoadKindWord::Rule,
        FolderLoadKind::Skill => FolderLoadKindWord::Skill,
        FolderLoadKind::Agent => FolderLoadKindWord::Agent,
        FolderLoadKind::Command => FolderLoadKindWord::Command,
        FolderLoadKind::SkillRoot => FolderLoadKindWord::SkillRoot,
    }
}

/// The variant as a word; its payload text in `message` and the front matter's line in `line`.
fn unsure_row(unsure: FolderUnsure) -> PreviewUnsureRow {
    let (why, message, line) = match unsure.why {
        UnsureWhy::Unreadable(error) => ("unreadable", Some(error), None),
        UnsureWhy::FrontMatter(error) => ("front_matter", Some(error.message), error.line),
        UnsureWhy::Link => ("link", None, None),
        UnsureWhy::DashesInFrontMatter => ("dashes_in_front_matter", None, None),
        UnsureWhy::UnclearPaths => ("unclear_paths", None, None),
        UnsureWhy::EmptyBody => ("empty_body", None, None),
        UnsureWhy::HookKeys => ("hook_keys", None, None),
        UnsureWhy::SkillFileInCommandFolder => ("skill_file_in_command_folder", None, None),
    };
    PreviewUnsureRow {
        path: slash_path(&unsure.path),
        kind: unsure.kind.map(load_kind),
        why: why.to_owned(),
        message,
        line,
    }
}

fn list_row(row: ListRow) -> declared::ListRow {
    declared::ListRow {
        path: row.path,
        kind: row.kind.listed().as_ref().and_then(library_kind),
        name: row.name,
        state: row.state.map(reason_row),
        references: count(row.references),
        description: row.description,
    }
}

fn reason_row(reason: Reason) -> ReasonRow {
    ReasonRow {
        code: reason.code,
        args: reason.args,
        line: reason.line,
    }
}

fn path_reason(path: String, reason: Reason) -> PathReasonRow {
    PathReasonRow {
        path,
        code: reason.code,
        args: reason.args,
        line: reason.line,
    }
}

/// A listed kind ([`ItemKind::listed`]) as the declared word; `Broken` has none.
fn library_kind(kind: &ItemKind) -> Option<LibraryKind> {
    match kind {
        ItemKind::Instruction => Some(LibraryKind::Instruction),
        ItemKind::Skill => Some(LibraryKind::Skill),
        ItemKind::SkillFile => Some(LibraryKind::SkillFile),
        ItemKind::Settings => Some(LibraryKind::Settings),
        ItemKind::Preset => Some(LibraryKind::Preset),
        ItemKind::Broken { .. } => None,
    }
}

fn item_kind(kind: &LibraryKind) -> ItemKind {
    match kind {
        LibraryKind::Instruction => ItemKind::Instruction,
        LibraryKind::Skill => ItemKind::Skill,
        LibraryKind::SkillFile => ItemKind::SkillFile,
        LibraryKind::Settings => ItemKind::Settings,
        LibraryKind::Preset => ItemKind::Preset,
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::sync::Arc;

    use engram_dashboard_agent::launch::BlockReason;
    use engram_dashboard_agent::profile::{
        AgentCommand, AgentProfile, AttachedPreset, ProfileRegistry, ProfileStore, Refusal,
        StoreError,
    };
    use engram_dashboard_agent::profile_edit::{
        Attachments, PresetPathRename, ProfileEditError, ProfileEditPort, WorkspaceLock,
    };
    use serde_json::json;
    use tempfile::TempDir;

    use super::*;
    use crate::backends::AgentBackends;
    use crate::config::{LibraryConfig, MountPlace};
    use crate::folder::{BoundedIo, LaunchWaits, StdFolderOps, ThreadSpawn, IO_LIMIT, LIST_CAP};
    use crate::format::read::ITEM_PATH_INVALID;
    use crate::relink::{RelinkSkip, RelinkStop};
    use crate::store::{FsStore, LibraryStore};
    use crate::{LibraryValues, LIBRARY_UNAVAILABLE, PATH_THROUGH_LINK};
    use engram_dashboard_agent::commands::{MountStatusRow, PointingRow};
    use engram_dashboard_agent::library_commands::REMOVE_OUTCOME_REMOVED;
    use engram_dashboard_platform::fs::{link_dir, unlink_dir};

    struct Fixed(Vec<AgentProfile>);

    impl ProfileStore for Fixed {
        fn save(&self, _: &[AgentProfile]) -> Result<(), StoreError> {
            Ok(())
        }
        fn load(&self) -> Vec<AgentProfile> {
            self.0.clone()
        }
    }

    fn agent(presets: &[&str], default_preset: bool) -> AgentProfile {
        let command = AgentCommand::Shell {
            program: "sh".to_owned(),
            args: Vec::new(),
        };
        let mut profile = AgentProfile::new(
            "a".to_owned(),
            command,
            PathBuf::from("/work"),
            Vec::new(),
            false,
        );
        profile.default_preset = default_preset;
        profile.presets = presets
            .iter()
            .map(|path| AttachedPreset {
                path: (*path).to_owned(),
            })
            .collect();
        profile
    }

    /// A temp folder holding the library root `lib\` (published when `root`) over the real store, a registry over the
    /// profiles, no built-in folder and no profile-edit port.
    struct World {
        _temp: TempDir,
        base: PathBuf,
        registry: Arc<ProfileRegistry>,
        svc: Arc<LibraryService>,
    }

    impl Drop for World {
        fn drop(&mut self) {
            let link = self.base.join("lib").join("team");
            if fs::symlink_metadata(&link).is_ok_and(|meta| meta.file_type().is_symlink()) {
                if let Err(e) = unlink_dir(&link) {
                    eprintln!("could not remove the test link {}: {e}", link.display());
                }
            }
        }
    }

    impl World {
        fn new(profiles: Vec<AgentProfile>) -> World {
            World::make(profiles, true)
        }

        fn make(profiles: Vec<AgentProfile>, root: bool) -> World {
            World::build(profiles, root, None)
        }

        /// A world whose built-in folder holds `engram/default` as `default_preset` (the other built-ins stand in).
        fn with_default_preset(profiles: Vec<AgentProfile>, default_preset: &str) -> World {
            World::build(profiles, true, Some(default_preset))
        }

        fn build(profiles: Vec<AgentProfile>, root: bool, default_preset: Option<&str>) -> World {
            let temp = tempfile::tempdir().unwrap();
            let base = dunce::canonicalize(temp.path()).unwrap();
            fs::create_dir_all(base.join("lib")).unwrap();
            if let Some(text) = default_preset {
                let file = base.join("builtin").join(crate::builtin::file_of(
                    crate::path::BuiltinId::DefaultPreset,
                ));
                fs::create_dir_all(file.parent().unwrap()).unwrap();
                fs::write(file, text).unwrap();
            }
            let state = base.join("state");
            fs::create_dir_all(&state).unwrap();
            let config = match root {
                true => LibraryConfig::usable(&state, MountPlace::Explicit(base.join("lib"))),
                false => LibraryConfig::missing(&state),
            };
            let registry = Arc::new(ProfileRegistry::new(Arc::new(Fixed(profiles))));
            let values = LibraryValues {
                registry: Arc::clone(&registry),
                backends: Arc::new(AgentBackends),
                config,
                default_root: base.join("default-library"),
                daemon_id: "test-daemon".to_owned(),
                root_generation_base: 7,
                builtin_dir: default_preset.map(|_| base.join("builtin")),
                launch_root: base.join("launch"),
                home: None,
                env: Arc::new(|_| None),
            };
            let io = BoundedIo::new(
                IO_LIMIT,
                LaunchWaits::LAUNCH,
                LIST_CAP,
                Arc::new(StdFolderOps),
                Arc::new(ThreadSpawn),
            );
            let svc = Arc::new(LibraryService::new(
                values,
                Arc::new(FsStore) as Arc<dyn LibraryStore>,
                io,
            ));
            World {
                _temp: temp,
                base,
                registry,
                svc,
            }
        }

        /// Plugs a profile-edit port over the world's registry — keep the answer alive while the port is needed.
        fn plug(&self) -> Arc<RegistryPort> {
            let port = Arc::new(RegistryPort(Arc::clone(&self.registry)));
            let weak: std::sync::Weak<RegistryPort> = Arc::downgrade(&port);
            self.svc.plug_profile_edit(weak);
            port
        }

        fn put(&self, rel: &str, text: &str) {
            let path = self.base.join("lib").join(rel);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        }

        /// The service through the port, as the daemon table holds it.
        fn port(&self) -> &dyn LibraryCommandHost {
            &*self.svc
        }

        fn list(
            &self,
            folder: Option<&str>,
            query: Option<&str>,
            kinds: &[LibraryKind],
        ) -> LibraryListOk {
            self.port()
                .list(folder, query, kinds)
                .unwrap_or_else(|e| panic!("list: {e:?}"))
        }
    }

    /// A profile-edit port that writes the attachments into the registry, as the manager does, and answers no
    /// working-folder lock.
    struct RegistryPort(Arc<ProfileRegistry>);

    impl ProfileEditPort for RegistryPort {
        fn set_attachments(&self, id: AgentId, next: Attachments) -> Result<(), ProfileEditError> {
            let present = self.0.try_update_with(id, |profile| {
                profile.default_preset = next.default_preset;
                profile.presets = next.presets;
            })?;
            match present {
                true => Ok(()),
                false => Err(ProfileEditError::NotFound(id)),
            }
        }

        fn rewrite_preset_paths(
            &self,
            _: &[PresetPathRename],
        ) -> Result<Vec<AgentId>, ProfileEditError> {
            unreachable!("not an agent-preset command's call")
        }

        fn remove_preset_paths(&self, _: &[String]) -> Result<Vec<AgentId>, ProfileEditError> {
            unreachable!("not an agent-preset command's call")
        }

        fn workspace_lock(&self, id: AgentId) -> Result<Option<WorkspaceLock>, ProfileEditError> {
            match self.0.get(id) {
                Some(_) => Ok(None),
                None => Err(ProfileEditError::NotFound(id)),
            }
        }
    }

    fn refused<T: std::fmt::Debug>(answer: Result<T, PortRefusal>) -> (PortClass, Reason) {
        match answer {
            Err(PortRefusal::Refused { class, reason }) => (class, reason),
            other => panic!("expected a refusal: {other:?}"),
        }
    }

    fn paths(rows: &[declared::ListRow]) -> Vec<&str> {
        rows.iter().map(|row| row.path.as_str()).collect()
    }

    const SKILL: &str = "---\nname: s1\ndescription: Says hello\n---\n";

    /// The row of the built-in mount in a world with no built-in folder.
    fn builtin_status_row() -> MountStatusRow {
        MountStatusRow {
            name: "engram".to_owned(),
            path: String::new(),
            default: false,
            builtin: true,
            state: "unreadable".to_owned(),
            state_why: Some(crate::builtin::NO_FOLDER.to_owned()),
        }
    }

    /// `library.status` and the mount commands carry the bodies' answers field by field, in the declared words
    /// (ADR-0400 decision 7).
    #[test]
    fn status_and_the_mount_commands_carry_the_body_answer_field_by_field() {
        let w = World::new(vec![agent(&["team/P.yaml"], false)]);
        let body = w.svc.status();
        let out = w.port().status().expect("status");
        let library = MountStatusRow {
            name: "library".to_owned(),
            path: w.base.join("lib").display().to_string(),
            default: false,
            builtin: false,
            state: "ok".to_owned(),
            state_why: None,
        };
        assert_eq!(
            out,
            LibraryStatusOk {
                daemon_id: "test-daemon".to_owned(),
                mounts: vec![library.clone(), builtin_status_row()],
                config: "ok".to_owned(),
                config_why: None,
                suggested_new: w.base.join("default-library").display().to_string(),
                root_generation: body.root_generation,
            }
        );

        let other = w.base.join("other");
        let added = w
            .port()
            .add_mount("team", &other.display().to_string(), true)
            .expect("add");
        assert_eq!(
            added.already_pointing,
            PointingRow {
                presets: 0,
                agents: 1
            }
        );
        let team = MountStatusRow {
            name: "team".to_owned(),
            path: other.display().to_string(),
            ..library.clone()
        };
        assert_eq!(added.mounts, [library.clone(), team, builtin_status_row()]);
        assert_eq!(added.root_generation, w.svc.status().root_generation);
        assert_ne!(added.root_generation, body.root_generation);

        let removed = w.port().remove_mount("team").expect("remove");
        assert_eq!(
            (removed.outcome.as_str(), removed.now_absent),
            (
                REMOVE_OUTCOME_REMOVED,
                PointingRow {
                    presets: 0,
                    agents: 1
                }
            )
        );
        assert_eq!(removed.mounts, [library.clone(), builtin_status_row()]);

        let moved_to = w.base.join("lib2");
        let moved = w
            .port()
            .relocate_mount("library", &moved_to.display().to_string(), true)
            .expect("relocate");
        assert_eq!(moved.mounts[0].path, moved_to.display().to_string());
        assert_eq!(moved.root_generation, w.svc.status().root_generation);
    }

    /// With no mount registered the status lists only the built-in.
    #[test]
    fn with_no_mount_the_status_lists_only_the_builtin() {
        let w = World::make(Vec::new(), false);
        let out = w.port().status().expect("status");
        assert_eq!(out.mounts, [builtin_status_row()]);
    }

    /// Every path argument that is not a library path refuses `InvalidArgument` · `item-path-invalid` with nothing
    /// done — the old built-in spelling included, its `why` naming the new one (ADR-0400 decision 20).
    #[test]
    fn every_path_argument_that_is_not_a_library_path_is_refused_item_path_invalid() {
        let w = World::new(Vec::new());
        let port = w.port();
        let (_, old) = refused(port.read("engram:/default"));
        assert!(
            old.args["why"]
                .as_str()
                .unwrap()
                .contains("\"engram/default\""),
            "{old:?}"
        );
        let cases: Vec<(&str, (PortClass, Reason))> = vec![
            ("read", refused(port.read("../x.md"))),
            ("read old built-in", (PortClass::InvalidArgument, old)),
            (
                "list old built-in",
                refused(port.list(Some("engram:/default"), None, &[])),
            ),
            (
                "list",
                refused(port.list(Some("library/a/../b"), None, &[])),
            ),
            ("refresh", refused(port.refresh(Some("/abs")))),
            (
                "newFolder",
                refused(port.new_folder(Some("a\\b"), "x", None)),
            ),
            (
                "newFile",
                refused(port.new_file(Some(".."), "x", LibraryNewFileKind::Markdown, None)),
            ),
            ("relink from", refused(port.relink("..", "a.md", None))),
            (
                "relink to",
                refused(port.relink("library/a.md", "b/", None)),
            ),
        ];
        for (what, (class, reason)) in cases {
            assert_eq!(
                (class, reason.code.as_str()),
                (PortClass::InvalidArgument, ITEM_PATH_INVALID),
                "{what}"
            );
        }
        for (class, reason) in [
            refused(port.add_mount("team", "relative/lib", false)),
            refused(port.relocate_mount("library", "relative/lib", false)),
        ] {
            assert_eq!(
                (class, reason.code.as_str()),
                (PortClass::InvalidArgument, ITEM_PATH_INVALID)
            );
        }
    }

    /// `list`'s bare reason gets the class `Unavailable` and names the mount; the whole-tree reads answer partially,
    /// naming the mount in `unavailable` and carrying its state on its row (ADR-0400 decisions 4 and 6).
    #[test]
    fn reads_in_a_mount_that_cannot_be_read_refuse_unavailable_and_whole_tree_reads_answer_partially(
    ) {
        let w = World::make(Vec::new(), true);
        // The mount's folder is gone before the first scan: it is not walked.
        fs::remove_dir_all(w.base.join("lib")).unwrap();
        let (class, reason) = refused(w.port().list(Some("library"), None, &[]));
        assert_eq!(
            (class, reason.code.as_str(), &reason.args["mount"]),
            (
                PortClass::Unavailable,
                LIBRARY_UNAVAILABLE,
                &json!("library")
            )
        );
        let missing = || {
            vec![
                // No built-in folder in this world: the built-in mount is named too.
                UnavailableMountRow {
                    mount: "engram".to_owned(),
                    state: "unreadable".to_owned(),
                    why: crate::builtin::NO_FOLDER.to_owned(),
                },
                UnavailableMountRow {
                    mount: "library".to_owned(),
                    state: "missing".to_owned(),
                    why: "missing".to_owned(),
                },
            ]
        };
        let folders = w.port().folders().expect("a partial answer");
        assert_eq!(
            folders.folders.last(),
            Some(&LibraryFolderRow {
                path: "library".to_owned(),
                name: "library".to_owned(),
                skill: false,
                state: Some("missing".to_owned()),
                state_why: Some("missing".to_owned()),
            })
        );
        let paths: Vec<&str> = folders
            .folders
            .iter()
            .map(|row| row.path.as_str())
            .collect();
        assert_eq!(paths, ["engram", "engram/settings", "library"]);
        assert_eq!(folders.unavailable, missing());
        let found = w
            .port()
            .list(None, Some("a.md"), &[])
            .expect("a partial answer");
        assert!(found.rows.is_empty());
        assert_eq!(found.unavailable, missing());
        let top = w.port().list(None, None, &[]).expect("the mounts");
        assert_eq!(top.folders[1].path, "library");
        assert_eq!(
            (top.folders[1].state.as_deref(), top.unavailable.len()),
            (Some("missing"), 0),
            "the virtual root carries the state on the mount's row"
        );
    }

    /// A refresh that cannot scan is a report, never a refusal: one `errors` row — naming the mount a whole refresh
    /// could not walk, or with no path when the folder asked for could not be scanned.
    #[test]
    fn a_refresh_that_cannot_scan_answers_one_error_row() {
        let w = World::make(Vec::new(), true);
        fs::remove_dir_all(w.base.join("lib")).unwrap();
        let out = w.port().refresh(None).expect("a report");
        assert_eq!(out.errors.len(), 1, "{out:?}");
        assert_eq!(out.errors[0].path.as_deref(), Some("library"));
        assert_eq!((out.changed, out.problems.len()), (0, 0));
        let out = w.port().refresh(Some("library/a")).expect("a report");
        assert_eq!(out.errors.len(), 1, "{out:?}");
        assert_eq!(out.errors[0].path, None);
        assert_eq!((out.changed, out.problems.len()), (0, 0));
    }

    /// The report is carried as built: each problem keeps its path, code, args and line; links, no-kind files and
    /// errors keep their paths.
    #[test]
    fn a_refresh_report_is_carried_as_built() {
        let lp = |text: &str| crate::path::LibPath::parse(text).unwrap();
        let report = RefreshReport {
            changed: 3,
            problems: vec![(
                lp("library/p.yaml"),
                Reason {
                    code: "yaml-syntax".to_owned(),
                    args: json!({ "message": "m" }),
                    line: Some(2),
                },
            )],
            no_kind: vec![lp("library/n.yaml")],
            skipped_links: vec![lp("library/team")],
            errors: vec![
                (Some(lp("library/d")), "denied".to_owned()),
                (None, "gone".to_owned()),
            ],
        };
        assert_eq!(
            refresh_ok(report),
            LibraryRefreshOk {
                changed: 3,
                problems: vec![PathReasonRow {
                    path: "library/p.yaml".to_owned(),
                    code: "yaml-syntax".to_owned(),
                    args: json!({ "message": "m" }),
                    line: Some(2),
                }],
                no_kind: vec!["library/n.yaml".to_owned()],
                skipped_links: vec!["library/team".to_owned()],
                errors: vec![
                    RefreshErrorRow {
                        path: Some("library/d".to_owned()),
                        why: "denied".to_owned(),
                    },
                    RefreshErrorRow {
                        path: None,
                        why: "gone".to_owned(),
                    },
                ],
            }
        );
    }

    /// A skill folder lists as `Skill` in its parent; its `SKILL.md` lists inside it as `SkillFile`, and `kinds`
    /// matches that row only by `SkillFile` — `Skill` does not.
    #[test]
    fn a_skill_file_row_is_matched_only_by_skill_file() {
        let w = World::new(Vec::new());
        w.put("skills/s1/SKILL.md", SKILL);
        let parent = w.list(Some("library/skills"), None, &[LibraryKind::Skill]);
        assert_eq!(paths(&parent.rows), ["library/skills/s1"]);
        assert_eq!(parent.rows[0].kind, Some(LibraryKind::Skill));
        assert_eq!(parent.rows[0].name, "s1");

        let inside = w.list(Some("library/skills/s1"), None, &[]);
        assert_eq!(paths(&inside.rows), ["library/skills/s1/SKILL.md"]);
        assert_eq!(inside.rows[0].kind, Some(LibraryKind::SkillFile));
        let by_file = w.list(Some("library/skills/s1"), None, &[LibraryKind::SkillFile]);
        assert_eq!(paths(&by_file.rows), ["library/skills/s1/SKILL.md"]);
        let by_skill = w.list(Some("library/skills/s1"), None, &[LibraryKind::Skill]);
        assert!(by_skill.rows.is_empty(), "{by_skill:?}");
    }

    /// A query searches name and description, case-insensitively, in every item below the folder (not the folder
    /// itself), and the answer carries matching rows only — no folder lines.
    #[test]
    fn a_query_searches_every_item_below_the_folder_and_answers_no_folder_lines() {
        let w = World::new(Vec::new());
        w.put("rules/a.md", "Hello there");
        w.put("rules/deep/hello-b.md", "x");
        w.put("rules/deep/c.md", "nothing");
        w.put("other.md", "HELLO");
        w.put("skills/s1/SKILL.md", SKILL);

        let all = w.list(Some("library"), Some("hello"), &[]);
        assert_eq!(
            paths(&all.rows),
            [
                "library/other.md",
                "library/rules/a.md",
                "library/rules/deep/hello-b.md",
                "library/skills/s1"
            ]
        );
        assert!(all.folders.is_empty(), "{all:?}");

        let below = w.list(Some("library/rules/deep"), Some("HELLO"), &[]);
        assert_eq!(paths(&below.rows), ["library/rules/deep/hello-b.md"]);

        let parent = w.list(Some("library/skills"), Some("s1"), &[]);
        assert_eq!(paths(&parent.rows), ["library/skills/s1"]);
        let itself = w.list(Some("library/skills/s1"), Some("s1"), &[]);
        assert!(
            itself.rows.is_empty(),
            "the folder asked for is not its own result: {itself:?}"
        );

        let plain = w.list(Some("library/rules"), None, &[]);
        assert_eq!(paths(&plain.rows), ["library/rules/a.md"]);
        assert_eq!(
            plain.folders.len(),
            1,
            "without a query the folder lines stay"
        );
    }

    /// `kinds` keeps a broken file by the kind it claims; a broken file that claims none matches no kind.
    #[test]
    fn kinds_keep_a_broken_file_by_the_kind_it_claims() {
        let w = World::new(Vec::new());
        w.put("bad.yaml", "kind: preset\nversion: 1\ninstructions: [\n");
        w.put("odd.yaml", "kind: gadget\nversion: 1\n");
        w.put("good.yaml", "kind: preset\nversion: 1\ndescription: fine\n");
        w.put("a.md", "x");

        let every = w.list(Some("library"), None, &[]);
        let bad = every
            .rows
            .iter()
            .find(|row| row.path == "library/bad.yaml")
            .unwrap();
        assert_eq!(bad.kind, Some(LibraryKind::Preset));
        assert!(bad.state.is_some(), "a broken file carries its reason");
        let odd = every
            .rows
            .iter()
            .find(|row| row.path == "library/odd.yaml")
            .unwrap();
        assert_eq!(odd.kind, None);

        let presets = w.list(Some("library"), None, &[LibraryKind::Preset]);
        assert_eq!(
            paths(&presets.rows),
            ["library/bad.yaml", "library/good.yaml"]
        );
        let both = w.list(
            Some("library"),
            None,
            &[LibraryKind::Preset, LibraryKind::Instruction],
        );
        assert_eq!(
            paths(&both.rows),
            ["library/a.md", "library/bad.yaml", "library/good.yaml"]
        );
    }

    /// ADR-0400 decision 20: the built-in is the read-only mount `engram`, listed like any mount — its row at the
    /// virtual root, its items (spelling, last name, kind, a stand-in's reason, the agents whose Default slot is on)
    /// and its virtual folder `settings`, under the same query and kinds; a search of the whole tree finds them too.
    #[test]
    fn the_builtin_mount_lists_its_items_and_folder_like_any_mount_under_the_same_filters() {
        let w = World::new(vec![
            agent(&[], true),
            agent(&[], true),
            agent(&["library/p.yaml"], false),
        ]);
        w.put("default-notes.md", "x");
        let top = w.list(None, None, &[]);
        let tops: Vec<(&str, Option<&str>)> = top
            .folders
            .iter()
            .map(|row| (row.path.as_str(), row.state.as_deref()))
            .collect();
        // ADR-0400 decision 20: with no built-in folder the mount row says so; its items stand in.
        assert_eq!(
            tops,
            [("engram", Some("unreadable")), ("library", Some("ok"))]
        );

        let engram = w.list(Some("engram"), None, &[]);
        assert_eq!(paths(&engram.rows), ["engram/default", "engram/priming"]);
        let row = &engram.rows[0];
        assert_eq!(
            (row.name.as_str(), row.kind.clone(), row.references),
            ("default", Some(LibraryKind::Preset), 2)
        );
        let state = row.state.as_ref().expect("no built-in folder: a stand-in");
        assert_eq!(
            state.code,
            engram_dashboard_agent::launch::BUILTIN_UNAVAILABLE
        );
        assert_eq!(engram.rows[1].kind, Some(LibraryKind::Instruction));
        let folders: Vec<(&str, Option<&str>)> = engram
            .folders
            .iter()
            .map(|row| (row.path.as_str(), row.state.as_deref()))
            .collect();
        assert_eq!(folders, [("engram/settings", None)]);
        let settings = w.list(Some("engram/settings"), None, &[]);
        assert_eq!(paths(&settings.rows), ["engram/settings/default"]);
        assert_eq!(settings.rows[0].kind, Some(LibraryKind::Settings));
        assert!(settings.folders.is_empty());

        let kept = w.list(Some("engram"), Some("DEF"), &[LibraryKind::Preset]);
        assert_eq!(paths(&kept.rows), ["engram/default"]);
        assert!(kept.folders.is_empty(), "no folder lines with a query");
        let by_kind = w.list(Some("engram"), None, &[LibraryKind::Instruction]);
        assert_eq!(paths(&by_kind.rows), ["engram/priming"]);
        assert!(w.list(Some("engram"), Some("zzz"), &[]).rows.is_empty());
        let whole = w.list(None, Some("default"), &[]);
        assert_eq!(
            paths(&whole.rows),
            [
                "engram/default",
                "engram/settings/default",
                "library/default-notes.md"
            ]
        );
        assert!(w.list(Some("engram/nope"), None, &[]).rows.is_empty());
    }

    /// `library.read` carries the body's answer field by field, agents by id.
    #[test]
    fn read_carries_the_body_answer_field_by_field() {
        let attached = agent(&["library/p.yaml"], false);
        let id: AgentId = attached.id;
        let w = World::new(vec![attached]);
        w.put("rules/a.md", "Body text");
        w.put(
            "p.yaml",
            "kind: preset\nversion: 1\ninstructions:\n  - library/rules/a.md\n",
        );
        let view = w.port().read("library/rules/a.md").expect("read");
        let body = w
            .svc
            .read(&crate::path::LibPath::parse("library/rules/a.md").unwrap())
            .expect("body read");
        assert_eq!(view.absolute, body.absolute.display().to_string());
        assert_eq!(view.presets_direct, ["library/p.yaml"]);
        assert_eq!(view.presets_direct, body.presets_direct);
        assert_eq!(view.presets_indirect, body.presets_indirect);
        assert_eq!(view.agents_direct, body.agents_direct);
        assert_eq!(view.agents_indirect, [id]);
        assert_eq!(view.excerpt, body.excerpt);
        assert_eq!(view.settings_fields, body.settings_fields);
        assert_eq!(view.row.path, "library/rules/a.md");
        assert_eq!(view.row.kind, Some(LibraryKind::Instruction));
        assert_eq!(view.row.references, 1);
        assert_eq!(view.row.description, body.row.description);

        let (class, reason) = refused(w.port().read("library/rules/none.md"));
        assert_eq!(
            (class, reason.code.as_str()),
            (PortClass::NotFound, crate::NOT_FOUND)
        );
    }

    /// The creates answer the path the body made, and a taken name is the body's refusal.
    #[test]
    fn creates_answer_the_path_the_body_made() {
        let w = World::new(Vec::new());
        let folder = w
            .port()
            .new_folder(Some("library"), "team", None)
            .expect("folder");
        assert_eq!(folder, "library/team");
        let file = w
            .port()
            .new_file(Some("library/team"), "p", LibraryNewFileKind::Preset, None)
            .expect("file");
        assert_eq!(file, "library/team/p.yaml");
        assert!(w.base.join("lib").join("team").join("p.yaml").is_file());
        let (class, reason) = refused(w.port().new_folder(Some("library"), "team", None));
        assert_eq!(
            (class, reason.code.as_str()),
            (PortClass::Conflict, crate::create::NAME_EXISTS)
        );
        let (class, reason) = refused(w.port().new_folder(Some("library"), "x", Some(1)));
        assert_eq!(
            (class, reason.code.as_str()),
            (PortClass::Conflict, crate::ROOT_GENERATION_STALE)
        );
    }

    /// A relink whose agents need the profile-edit port while none is plugged refuses `Internal` ·
    /// `profile-edit-unavailable` — the one class that keeps the request id.
    #[test]
    fn a_relink_with_no_profile_edit_port_refuses_internal() {
        let w = World::new(vec![agent(&["library/p.yaml"], false)]);
        w.put("p.yaml", "kind: preset\nversion: 1\n");
        w.put("q/p.yaml", "kind: preset\nversion: 1\n");
        let (class, reason) = refused(w.port().relink("library/p.yaml", "library/q/p.yaml", None));
        assert_eq!(
            (class, reason.code.as_str()),
            (
                PortClass::Internal,
                crate::agent_edit::PROFILE_EDIT_UNAVAILABLE
            )
        );
    }

    /// The relink answer is carried as built: pairs, `more`, written files, skips with their full reason, and a stop
    /// as the refusal it met — a store error as `profile-store-refused`.
    #[test]
    fn a_relink_answer_is_carried_as_built() {
        let answer = RelinkAnswer {
            fixed: vec![("a.md".to_owned(), "b/a.md".to_owned())],
            more: 1,
            written: vec!["library/p.yaml".to_owned()],
            skipped: vec![RelinkSkip {
                path: "q.yaml".to_owned(),
                reason: Reason {
                    code: "yaml-syntax".to_owned(),
                    args: json!({ "message": "m" }),
                    line: Some(3),
                },
            }],
            stopped: Some(RelinkStop {
                path: None,
                refusal: LibraryRefusal::Store(StoreError::Refused(Refusal::Unreadable)),
            }),
        };
        let out = relink_ok(answer);
        assert_eq!(
            out.fixed,
            [RelinkPairRow {
                from: "a.md".to_owned(),
                to: "b/a.md".to_owned(),
            }]
        );
        assert_eq!(
            (out.more, out.written.as_slice()),
            (1, ["library/p.yaml".to_owned()].as_slice())
        );
        assert_eq!(
            out.skipped,
            [PathReasonRow {
                path: "q.yaml".to_owned(),
                code: "yaml-syntax".to_owned(),
                args: json!({ "message": "m" }),
                line: Some(3),
            }]
        );
        let stop = out.stopped.expect("stopped");
        assert_eq!(stop.path, None);
        assert_eq!(
            stop.code,
            engram_dashboard_agent::library_commands::PROFILE_STORE_REFUSED
        );
    }

    /// A mount rename's stop names the command that stopped, `library.renameMount`, not the relink whose reference fix it
    /// shares (ADR-0419 — `relink_stop_row` takes the command).
    #[test]
    fn a_mount_rename_stop_names_library_rename_mount() {
        let answer = RenameAnswer {
            status: LibraryStatus {
                daemon_id: "d".to_owned(),
                mounts: Vec::new(),
                config: ConfigHealth::Ok,
                suggested_new: PathBuf::from("C:/data/library"),
                root_generation: 1,
            },
            fixed: Vec::new(),
            written: Vec::new(),
            skipped: Vec::new(),
            stopped: Some(RelinkStop {
                path: Some("a/p.yaml".to_owned()),
                refusal: LibraryRefusal::Store(StoreError::Refused(Refusal::Unreadable)),
            }),
            already_pointing: Pointing::default(),
        };
        let stop = rename_ok(answer).stopped.expect("stopped");
        assert!(
            stop.message.starts_with("library.renameMount: "),
            "{}",
            stop.message
        );
        assert_eq!(stop.path.as_deref(), Some("a/p.yaml"));
    }

    /// Each refusal the caller can clear and then repeat is in agent's `AfterCondition` list, under the code this
    /// crate answers — a renamed const would fall out of the directive silently.
    #[test]
    fn the_clearable_refusal_codes_are_in_the_after_condition_list() {
        use engram_dashboard_agent::library_commands::RETRY_AFTER_CONDITION;
        for code in [
            crate::LIBRARY_UNAVAILABLE,
            crate::edit::LIBRARY_WRITE_FAILED,
            crate::launch::LIBRARY_READ_FAILED,
            crate::LIBRARY_ROOT_UNUSABLE,
            crate::LIBRARY_CONFIG_WRITE_FAILED,
        ] {
            assert!(RETRY_AFTER_CONDITION.contains(&code), "{code}");
        }
        assert_eq!(RETRY_AFTER_CONDITION.len(), 5);
    }

    /// Every status word and reason lands in its own field: each mount's state word with its `state_why` (`ok` has
    /// none), its flags and folder, and each `config` word with its `config_why` (`corrupt` has none).
    #[test]
    fn every_status_word_lands_in_its_own_field() {
        let row = |name: &str, state: MountState| crate::MountRow {
            name: name.to_owned(),
            folder: PathBuf::from(format!("C:/{name}")),
            default: name == "a",
            builtin: name == "engram",
            state,
        };
        let status = |config: ConfigHealth| LibraryStatus {
            daemon_id: "d".to_owned(),
            mounts: vec![
                row(
                    "a",
                    MountState::Unreadable {
                        why: "timed out".to_owned(),
                    },
                ),
                row(
                    "b",
                    MountState::Missing {
                        why: "not a folder".to_owned(),
                    },
                ),
                row("engram", MountState::Ok),
            ],
            config,
            suggested_new: PathBuf::from("C:/data/library"),
            root_generation: 9,
        };
        let out = status_ok(status(ConfigHealth::ReadOnly {
            why: "newer".to_owned(),
        }));
        let words: Vec<(&str, &str, bool, bool, &str, Option<&str>)> = out
            .mounts
            .iter()
            .map(|row| {
                (
                    row.name.as_str(),
                    row.path.as_str(),
                    row.default,
                    row.builtin,
                    row.state.as_str(),
                    row.state_why.as_deref(),
                )
            })
            .collect();
        assert_eq!(
            words,
            [
                ("a", "C:/a", true, false, "unreadable", Some("timed out")),
                ("b", "C:/b", false, false, "missing", Some("not a folder")),
                ("engram", "C:/engram", false, true, "ok", None),
            ]
        );
        assert_eq!(
            (out.config.as_str(), out.config_why.as_deref()),
            ("read_only", Some("newer"))
        );
        let out = status_ok(status(ConfigHealth::Unsaved {
            why: "write failed".to_owned(),
        }));
        assert_eq!(
            (out.config.as_str(), out.config_why.as_deref()),
            ("unsaved", Some("write failed"))
        );
        let out = status_ok(status(ConfigHealth::Corrupt));
        assert_eq!((out.config.as_str(), out.config_why), ("corrupt", None));
    }

    /// `Settings` crosses both ways: a settings file lists as `Settings` and `kinds: [Settings]` keeps it, and
    /// `newFile {kind: Settings}` writes the settings template.
    #[test]
    fn the_settings_kind_crosses_both_ways() {
        let w = World::new(Vec::new());
        w.put("s.yaml", "kind: settings\nversion: 1\ndescription: d\n");
        w.put("p.yaml", "kind: preset\nversion: 1\n");
        let kept = w.list(Some("library"), None, &[LibraryKind::Settings]);
        assert_eq!(paths(&kept.rows), ["library/s.yaml"]);
        assert_eq!(kept.rows[0].kind, Some(LibraryKind::Settings));

        let made = w
            .port()
            .new_file(Some("library"), "fresh", LibraryNewFileKind::Settings, None)
            .expect("file");
        assert_eq!(made, "library/fresh.yaml");
        let text = fs::read_to_string(w.base.join("lib").join("fresh.yaml")).unwrap();
        assert!(text.starts_with("kind: settings\n"), "{text}");
    }

    /// `library.folders` answers every folder with the skill flag as the body set it.
    #[test]
    fn folders_carry_the_skill_flag() {
        let w = World::new(Vec::new());
        w.put("rules/a.md", "x");
        w.put("skills/s1/SKILL.md", SKILL);
        let out = w.port().folders().expect("folders");
        let rows: Vec<(&str, &str, bool)> = out
            .folders
            .iter()
            .map(|row| (row.path.as_str(), row.name.as_str(), row.skill))
            .collect();
        assert_eq!(
            rows,
            [
                ("engram", "engram", false),
                ("engram/settings", "settings", false),
                ("library", "library", false),
                ("library/rules", "rules", false),
                ("library/skills", "skills", false),
                ("library/skills/s1", "s1", true),
            ]
        );
    }

    /// A folder behind a link answers its one `path-through-link` row whatever the filters say, and still the
    /// built-in row the filters keep.
    #[test]
    fn a_folder_behind_a_link_answers_its_link_row_under_any_filter() {
        let w = World::new(Vec::new());
        let outside = w.base.join("outside");
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("x.md"), "x").unwrap();
        link_dir(&outside, &w.base.join("lib").join("team")).unwrap();

        let out = w
            .port()
            .list(
                Some("library/team/x.md"),
                Some("zzz"),
                &[LibraryKind::Skill],
            )
            .expect("list");
        assert_eq!(paths(&out.rows), ["library/team/x.md"]);
        assert_eq!(
            out.rows[0].state.as_ref().map(|state| state.code.as_str()),
            Some(PATH_THROUGH_LINK)
        );

        let out = w
            .port()
            .list(Some("library/team/x.md"), None, &[])
            .expect("list");
        assert_eq!(paths(&out.rows), ["library/team/x.md"]);
    }

    // ── preset.* ──

    fn entry_paths(rows: &[PresetEntryRow]) -> Vec<&str> {
        rows.iter().map(|row| row.path.as_str()).collect()
    }

    fn strings(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| (*item).to_owned()).collect()
    }

    fn empty_draft() -> PresetDraftInput {
        PresetDraftInput {
            presets: Vec::new(),
            instructions: Vec::new(),
            skills: Vec::new(),
            settings: Vec::new(),
            folders: None,
        }
    }

    fn arg(path: &str) -> PresetEntryArg {
        PresetEntryArg {
            path: path.to_owned(),
            backend: None,
        }
    }

    /// A preset with a present and a missing instruction, a folder entry, attached by one agent.
    fn preset_world() -> (World, String) {
        let w = World::new(vec![agent(&["library/p.yaml"], false)]);
        let folder = w.base.join("f");
        fs::create_dir_all(&folder).unwrap();
        let folder = folder.to_string_lossy().into_owned();
        w.put("rules/a.md", "A");
        w.put(
            "p.yaml",
            &format!(
                "kind: preset\nversion: 1\ndescription: team\ninstructions:\n  - library/rules/a.md\n  - library/rules/gone.md\nfolders:\n  - {folder}\n"
            ),
        );
        (w, folder)
    }

    /// `preset.get` answers the body's view in the declared rows: entries with their lines and states, the folder with
    /// its stat word, the missing item as a `missing` problem through the preset, the `rev` and the agent count.
    #[test]
    fn preset_get_carries_the_body_view_in_declared_rows() {
        let (w, folder) = preset_world();
        let body = w
            .svc
            .preset_get(&crate::path::AssetRef::parse("library/p.yaml").unwrap())
            .expect("body view");
        let view = w.port().preset_get("library/p.yaml").expect("view");
        assert_eq!(view.path, "library/p.yaml");
        assert_eq!(view.rev, body.rev);
        assert_eq!(view.description.as_deref(), Some("team"));
        assert_eq!(
            entry_paths(&view.instructions),
            ["library/rules/a.md", "library/rules/gone.md"]
        );
        assert_eq!(
            view.instructions[0],
            PresetEntryRow {
                path: "library/rules/a.md".to_owned(),
                backend: None,
                line: 5,
                state: None,
            }
        );
        let gone = &view.instructions[1];
        assert_eq!(gone.line, 6);
        assert_eq!(
            gone.state.as_ref().map(|state| state.code.as_str()),
            Some(crate::NOT_FOUND)
        );
        assert_eq!(
            view.folders,
            [FolderRow {
                path: folder,
                line: Some(8),
                status: Some("ok".to_owned()),
                status_why: None,
            }]
        );
        let missing = view
            .problems
            .iter()
            .find(|problem| problem.kind == "missing")
            .expect("the missing item is a problem");
        assert_eq!(missing.item.as_deref(), Some("library/rules/gone.md"));
        assert_eq!(
            missing.via.as_deref(),
            Some(&["library/p.yaml".to_owned()][..])
        );
        assert_eq!(missing.items, None);
        assert!(view.conflicts.is_empty());
        assert_eq!(view.agents, 1);
        assert_eq!(
            (view.problems.len(), view.conflicts.len()),
            (body.problems.len(), body.conflicts.len())
        );
    }

    /// Every preset `path` that is not a library path refuses `InvalidArgument` · `item-path-invalid` before any body
    /// runs; `preset.get` takes a built-in spelling.
    #[test]
    fn every_preset_path_that_is_not_a_library_path_is_refused_item_path_invalid() {
        let (w, _) = preset_world();
        let port = w.port();
        let items = PresetEntries::Items(strings(&["library/rules/a.md"]));
        let cases: Vec<(&str, (PortClass, Reason))> = vec![
            ("get", refused(port.preset_get("../p.yaml"))),
            (
                "check",
                refused(port.preset_check("/p.yaml", &empty_draft())),
            ),
            (
                "check built-in",
                refused(port.preset_check("engram:/default", &empty_draft())),
            ),
            (
                "add",
                refused(port.preset_add("a\\p.yaml", &items, None, None, None)),
            ),
            (
                "remove",
                refused(port.preset_remove("p.yaml/", &items, None)),
            ),
            (
                "reorder",
                refused(port.preset_reorder("..", PresetSection::Presets, &[], "r", None)),
            ),
            (
                "setBackend",
                refused(port.preset_set_backend("a:b", &[], None, None)),
            ),
            (
                "save",
                refused(port.preset_save("engram:/default", &empty_draft(), None, None)),
            ),
        ];
        for (what, (class, reason)) in cases {
            assert_eq!(
                (class, reason.code.as_str()),
                (PortClass::InvalidArgument, ITEM_PATH_INVALID),
                "{what}"
            );
        }
        let builtin = port.preset_get("engram/default");
        assert!(
            !matches!(&builtin, Err(PortRefusal::Refused { reason, .. }) if reason.code == ITEM_PATH_INVALID),
            "{builtin:?}"
        );
    }

    /// ADR-0400 decision 20: nothing is written into the built-in mount — a preset edit or check of a built-in or any
    /// other path under `engram`, a create in it, a relink to or from it refuse `Conflict` · `mount-read-only`
    /// `{mount, path}`, nothing done; a write whose target is `engram` alone is still the top of a mount (decision 5).
    #[test]
    fn every_write_into_the_builtin_mount_refuses_mount_read_only() {
        let (w, _) = preset_world();
        let port = w.port();
        let items = PresetEntries::Items(strings(&["library/rules/a.md"]));
        let cases: Vec<(&str, (PortClass, Reason))> = vec![
            (
                "check",
                refused(port.preset_check("engram/default", &empty_draft())),
            ),
            (
                "save",
                refused(port.preset_save("Engram/default", &empty_draft(), None, None)),
            ),
            (
                "add",
                refused(port.preset_add("engram/settings/x.yaml", &items, None, None, None)),
            ),
            (
                "reorder",
                refused(port.preset_reorder(
                    "engram/default",
                    PresetSection::Presets,
                    &[],
                    "r",
                    None,
                )),
            ),
            (
                "newFolder",
                refused(port.new_folder(Some("engram"), "x", None)),
            ),
            (
                "newFile",
                refused(port.new_file(
                    Some("engram/settings"),
                    "x",
                    LibraryNewFileKind::Markdown,
                    None,
                )),
            ),
            (
                "relink to",
                refused(port.relink("library/p.yaml", "engram/priming", None)),
            ),
        ];
        for (what, (class, reason)) in cases {
            assert_eq!(
                (class, reason.code.as_str(), &reason.args["mount"]),
                (
                    PortClass::Conflict,
                    crate::MOUNT_READ_ONLY,
                    &json!("engram")
                ),
                "{what}"
            );
        }
        let (class, reason) = refused(port.preset_save("engram", &empty_draft(), None, None));
        assert_eq!(
            (class, reason.code.as_str()),
            (PortClass::InvalidArgument, crate::VIRTUAL_ROOT_READ_ONLY)
        );
        assert!(!w.base.join("engram").exists(), "nothing created");
    }

    /// `preset.add` items applies what it can and answers the skips with their full reason beside `item`, and the
    /// view of the preset as committed.
    #[test]
    fn preset_add_answers_applied_skipped_and_the_view() {
        let (w, _) = preset_world();
        w.put("rules/b.md", "B");
        let out = w
            .port()
            .preset_add(
                "library/p.yaml",
                &PresetEntries::Items(strings(&["library/rules/b.md", "library/rules/none.md"])),
                Some("codex"),
                None,
                None,
            )
            .expect("add");
        assert_eq!(out.applied, ["library/rules/b.md"]);
        assert_eq!(
            out.skipped,
            [SkippedRow {
                item: "library/rules/none.md".to_owned(),
                code: crate::NOT_FOUND.to_owned(),
                args: json!({ "path": "library/rules/none.md" }),
                line: None,
            }]
        );
        assert_eq!(
            entry_paths(&out.view.instructions),
            [
                "library/rules/a.md",
                "library/rules/gone.md",
                "library/rules/b.md"
            ]
        );
        assert_eq!(out.view.instructions[2].backend.as_deref(), Some("codex"));
        let after = w.port().preset_get("library/p.yaml").expect("view");
        assert_eq!(out.view.rev, after.rev, "the answer is the committed file");
    }

    /// A folder `add` adds the folder normalized; a relative folder refuses the whole call.
    #[test]
    fn preset_add_folders_applies_or_refuses_whole() {
        let (w, _) = preset_world();
        let other = w.base.join("g");
        fs::create_dir_all(&other).unwrap();
        let out = w
            .port()
            .preset_add(
                "library/p.yaml",
                &PresetEntries::Folders(vec![other.to_string_lossy().into_owned()]),
                None,
                None,
                None,
            )
            .expect("add folder");
        assert_eq!(out.applied, [other.to_string_lossy().into_owned()]);
        assert_eq!(out.view.folders.len(), 2);
        let (class, reason) = refused(w.port().preset_add(
            "library/p.yaml",
            &PresetEntries::Folders(strings(&["library/rel/x"])),
            None,
            None,
            None,
        ));
        assert_eq!(
            (class, reason.code.as_str()),
            (
                PortClass::InvalidArgument,
                crate::edit::PRESET_FOLDERS_REFUSED
            )
        );
    }

    /// `preset.remove` answers the view after the change — items or folders.
    #[test]
    fn preset_remove_answers_the_view_after_the_change() {
        let (w, folder) = preset_world();
        let view = w
            .port()
            .preset_remove(
                "library/p.yaml",
                &PresetEntries::Items(strings(&["library/rules/gone.md"])),
                None,
            )
            .expect("remove");
        assert_eq!(entry_paths(&view.instructions), ["library/rules/a.md"]);
        let view = w
            .port()
            .preset_remove(
                "library/p.yaml",
                &PresetEntries::Folders(vec![folder]),
                None,
            )
            .expect("remove folder");
        assert!(view.folders.is_empty());
    }

    /// `preset.reorder` puts the section in order under the right `base_rev` and refuses a stale one.
    #[test]
    fn preset_reorder_checks_the_base_rev_and_answers_the_new_order() {
        let (w, _) = preset_world();
        let rev = w.port().preset_get("library/p.yaml").expect("view").rev;
        let order = strings(&["library/rules/gone.md", "library/rules/a.md"]);
        let (class, reason) = refused(w.port().preset_reorder(
            "library/p.yaml",
            PresetSection::Instructions,
            &order,
            "stale",
            None,
        ));
        assert_eq!(
            (class, reason.code.as_str()),
            (PortClass::Conflict, crate::edit::REV_MISMATCH)
        );
        let view = w
            .port()
            .preset_reorder(
                "library/p.yaml",
                PresetSection::Instructions,
                &order,
                &rev,
                None,
            )
            .expect("reorder");
        assert_eq!(
            entry_paths(&view.instructions),
            ["library/rules/gone.md", "library/rules/a.md"]
        );
    }

    /// `preset.setBackend` tags the items it can and skips the rest; an unknown word refuses the whole call.
    #[test]
    fn preset_set_backend_tags_skips_and_refuses_an_unknown_word() {
        let (w, _) = preset_world();
        let out = w
            .port()
            .preset_set_backend(
                "library/p.yaml",
                &strings(&["library/rules/a.md", "library/rules/other.md"]),
                Some("claude"),
                None,
            )
            .expect("set backend");
        assert_eq!(out.applied, ["library/rules/a.md"]);
        assert_eq!(out.skipped.len(), 1);
        assert_eq!(out.skipped[0].item, "library/rules/other.md");
        assert_eq!(out.view.instructions[0].backend.as_deref(), Some("claude"));
        let (class, reason) = refused(w.port().preset_set_backend(
            "library/p.yaml",
            &strings(&["library/rules/a.md"]),
            Some("nobody"),
            None,
        ));
        assert_eq!(
            (class, reason.code.as_str()),
            (PortClass::InvalidArgument, "backend-tag-unknown")
        );
    }

    /// `preset.save` writes the draft and answers the view; a mismatch refuses the whole save and writes nothing.
    #[test]
    fn preset_save_writes_the_draft_or_refuses_whole() {
        let (w, _) = preset_world();
        let before = fs::read(w.base.join("lib").join("p.yaml")).unwrap();
        let mut bad = empty_draft();
        bad.skills = vec![arg("library/rules/a.md")];
        let (class, reason) = refused(w.port().preset_save("library/p.yaml", &bad, None, None));
        assert_eq!(
            (class, reason.code.as_str()),
            (PortClass::InvalidArgument, crate::edit::PRESET_SAVE_REFUSED)
        );
        assert_eq!(fs::read(w.base.join("lib").join("p.yaml")).unwrap(), before);

        let mut good = empty_draft();
        good.instructions = vec![arg("library/rules/a.md")];
        good.folders = Some(Vec::new());
        let view = w
            .port()
            .preset_save("library/p.yaml", &good, None, None)
            .expect("save");
        assert_eq!(entry_paths(&view.instructions), ["library/rules/a.md"]);
        assert!(view.folders.is_empty());
    }

    /// `preset.check` answers `save_refuses` and each draft mismatch as a `broken` problem row with `args`
    /// `{path, list}` and no line; the file is not written.
    #[test]
    fn preset_check_answers_the_mismatch_as_a_broken_row_and_writes_nothing() {
        let (w, folder) = preset_world();
        let before = fs::read(w.base.join("lib").join("p.yaml")).unwrap();
        let mut draft = empty_draft();
        draft.skills = vec![arg("library/rules/a.md")];
        let out = w
            .port()
            .preset_check("library/p.yaml", &draft)
            .expect("check");
        assert!(out.save_refuses);
        let mismatch = out
            .problems
            .iter()
            .find(|problem| {
                problem.kind == "broken" && problem.item.as_deref() == Some("library/rules/a.md")
            })
            .expect("the mismatch row");
        assert_eq!(
            mismatch.via.as_deref(),
            Some(&["library/p.yaml".to_owned()][..])
        );
        assert_eq!(
            mismatch.args,
            json!({ "path": "library/rules/a.md", "list": "skills" })
        );
        assert_eq!(mismatch.line, None);
        assert_eq!(
            out.folders,
            [FolderRow {
                path: folder,
                line: Some(8),
                status: Some("ok".to_owned()),
                status_why: None,
            }],
            "an omitted folder list keeps the file's"
        );
        assert_eq!(fs::read(w.base.join("lib").join("p.yaml")).unwrap(), before);
    }

    /// `preset.previewFolder` answers the add verdict and, per backend and mode asked, the load set as typed rows.
    #[test]
    fn preview_folder_answers_the_verdict_and_typed_load_rows() {
        let w = World::new(Vec::new());
        let good = w.base.join("good");
        fs::create_dir_all(&good).unwrap();
        fs::write(good.join("AGENTS.md"), "rules").unwrap();
        let out = w
            .port()
            .preview_folder(&good.to_string_lossy(), Some("codex"), None)
            .expect("preview");
        assert_eq!(out.path, Some(good.to_string_lossy().into_owned()));
        assert_eq!(out.refused, None);
        let modes: Vec<(&str, &AgentOutputFormat)> = out
            .loads
            .iter()
            .map(|load| (load.backend.as_str(), &load.mode))
            .collect();
        assert_eq!(
            modes,
            [
                ("codex", &AgentOutputFormat::Terminal),
                ("codex", &AgentOutputFormat::StreamJson)
            ]
        );
        let json = &out.loads[1];
        assert!(!json.predicted);
        assert!(json.set.loads.contains(&PreviewItemRow {
            path: "AGENTS.md".to_owned(),
            kind: FolderLoadKindWord::Instructions,
        }));

        let one = w
            .port()
            .preview_folder(
                &good.to_string_lossy(),
                Some("codex"),
                Some(AgentOutputFormat::StreamJson),
            )
            .expect("preview");
        assert_eq!(one.loads.len(), 1);
        assert_eq!(one.loads[0].mode, AgentOutputFormat::StreamJson);

        let relative = w
            .port()
            .preview_folder("rel/x", None, None)
            .expect("preview");
        assert_eq!(relative.path, None);
        assert_eq!(
            relative.refused.map(|refused| refused.code),
            Some("folder-path-relative".to_owned())
        );
        assert!(relative.loads.is_empty());
        let (class, reason) = refused(w.port().preview_folder(
            &good.to_string_lossy(),
            Some("nobody"),
            None,
        ));
        assert_eq!(
            (class, reason.code.as_str()),
            (PortClass::InvalidArgument, "backend-tag-unknown")
        );
    }

    /// `preset.reorder` reorders the section it names and no other — each `PresetSection` reaches its own list (a
    /// section mapped onto another list would refuse the order as a mismatch or reorder the wrong list).
    #[test]
    fn preset_reorder_reaches_the_list_of_each_section() {
        let w = World::new(Vec::new());
        let (f1, f2) = (w.base.join("f1"), w.base.join("f2"));
        fs::create_dir_all(&f1).unwrap();
        fs::create_dir_all(&f2).unwrap();
        let (f1, f2) = (
            f1.to_string_lossy().into_owned(),
            f2.to_string_lossy().into_owned(),
        );
        w.put("rules/a.md", "A");
        w.put("rules/b.md", "B");
        w.put(
            "skills/s1/SKILL.md",
            "---\nname: s1\ndescription: one\n---\n",
        );
        w.put(
            "skills/s2/SKILL.md",
            "---\nname: s2\ndescription: two\n---\n",
        );
        w.put("s1.yaml", "kind: settings\nversion: 1\n");
        w.put("s2.yaml", "kind: settings\nversion: 1\n");
        w.put("q.yaml", "kind: preset\nversion: 1\n");
        w.put("r.yaml", "kind: preset\nversion: 1\n");
        w.put(
            "p.yaml",
            &format!(
                "kind: preset\nversion: 1\npresets:\n  - library/q.yaml\n  - library/r.yaml\ninstructions:\n  - library/rules/a.md\n  - library/rules/b.md\nskills:\n  - library/skills/s1\n  - library/skills/s2\nsettings:\n  - library/s1.yaml\n  - library/s2.yaml\nfolders:\n  - {f1}\n  - {f2}\n"
            ),
        );
        let lists = |view: &declared::PresetView| -> Vec<Vec<String>> {
            let entries =
                |rows: &[PresetEntryRow]| rows.iter().map(|row| row.path.clone()).collect();
            vec![
                entries(&view.presets),
                entries(&view.instructions),
                entries(&view.skills),
                entries(&view.settings),
                view.folders.iter().map(|row| row.path.clone()).collect(),
            ]
        };
        let sections = [
            PresetSection::Presets,
            PresetSection::Instructions,
            PresetSection::Skills,
            PresetSection::Settings,
            PresetSection::Folders,
        ];
        for (index, section) in sections.into_iter().enumerate() {
            let before = w.port().preset_get("library/p.yaml").expect("view");
            let mut expected = lists(&before);
            expected[index].reverse();
            let view = w
                .port()
                .preset_reorder(
                    "library/p.yaml",
                    section.clone(),
                    &expected[index],
                    &before.rev,
                    None,
                )
                .unwrap_or_else(|e| panic!("{section:?}: {e:?}"));
            assert_eq!(lists(&view), expected, "{section:?}");
        }
    }

    /// Each `FolderLoadKind` lands as its own word.
    #[test]
    fn every_folder_load_kind_lands_as_its_word() {
        let cases = [
            (
                FolderLoadKind::Instructions,
                FolderLoadKindWord::Instructions,
            ),
            (FolderLoadKind::Rule, FolderLoadKindWord::Rule),
            (FolderLoadKind::Skill, FolderLoadKindWord::Skill),
            (FolderLoadKind::Agent, FolderLoadKindWord::Agent),
            (FolderLoadKind::Command, FolderLoadKindWord::Command),
            (FolderLoadKind::SkillRoot, FolderLoadKindWord::SkillRoot),
        ];
        for (kind, word) in cases {
            assert_eq!(load_kind(kind), word, "{kind:?}");
        }
    }

    /// `preset.get engram/default` answers the built-in preset as asked: its spelling, the `rev` of the file it was
    /// read from, and how many agents have the Default slot on.
    #[test]
    fn preset_get_answers_the_builtin_default_preset() {
        let text = "kind: preset\nversion: 1\ndescription: built in\n";
        let w = World::with_default_preset(
            vec![
                agent(&[], true),
                agent(&[], true),
                agent(&["library/p.yaml"], false),
            ],
            text,
        );
        let view = w.port().preset_get("engram/default").expect("view");
        assert_eq!(view.path, "engram/default");
        assert_eq!(view.rev, crate::launch::sha256_hex(text.as_bytes()));
        assert_eq!(view.description.as_deref(), Some("built in"));
        assert_eq!(view.agents, 2);
        assert!(
            view.problems.is_empty() && view.conflicts.is_empty() && view.notices.is_empty(),
            "{view:?}"
        );
    }

    /// ADR-0400 decision 20: `library.read` of a built-in answers its real file in the built-in folder as `absolute`,
    /// its row and the agents whose Default slot is on; the mount's top and its virtual folder are no items; with no
    /// built-in folder at all the mount cannot be read.
    #[test]
    fn read_answers_a_builtin_at_its_file_in_the_builtin_folder() {
        let (on, off) = (agent(&[], true), agent(&[], false));
        let w = World::with_default_preset(
            vec![on.clone(), off],
            "kind: preset\nversion: 1\ndescription: built in\n",
        );
        for spelled in ["engram/default", "Engram/default"] {
            let view = w.port().read(spelled).expect(spelled);
            let file = w.base.join("builtin").join("presets").join("default.yaml");
            assert_eq!(view.absolute, file.to_string_lossy(), "{spelled}");
            assert_eq!(view.row.path, "engram/default");
            assert_eq!(
                (
                    view.row.kind,
                    view.row.description.as_str(),
                    view.row.references
                ),
                (Some(LibraryKind::Preset), "built in", 1)
            );
            assert_eq!(view.agents_direct, [on.id]);
            assert!(view.presets_direct.is_empty() && view.agents_indirect.is_empty());
        }
        let priming = w.port().read("engram/priming").expect("priming");
        assert_eq!(
            priming.absolute,
            w.base
                .join("builtin")
                .join("instructions")
                .join("priming.md")
                .to_string_lossy()
        );
        assert_eq!(priming.row.kind, Some(LibraryKind::Instruction));
        for folder in ["engram", "engram/settings", "engram/nope"] {
            let (class, reason) = refused(w.port().read(folder));
            assert_eq!(
                (class, reason.code.as_str()),
                (PortClass::NotFound, crate::NOT_FOUND),
                "{folder}"
            );
        }

        let none = World::new(Vec::new());
        let (class, reason) = refused(none.port().read("engram/default"));
        assert_eq!(
            (class, reason.code.as_str(), &reason.args["mount"]),
            (
                PortClass::Unavailable,
                LIBRARY_UNAVAILABLE,
                &json!("engram")
            )
        );
    }

    /// 10a hand-over: `preset.get engram/default` tells a built-in that stands in — here no built-in folder — as a
    /// `builtin-unavailable` notice (ADR-0361 · ADR-0400 decision 20).
    #[test]
    fn preset_get_of_a_builtin_that_stands_in_carries_its_notice() {
        let w = World::new(Vec::new());
        let view = w.port().preset_get("engram/default").expect("view");
        let notices: Vec<(&str, &serde_json::Value)> = view
            .notices
            .iter()
            .map(|notice| (notice.code.as_str(), &notice.args))
            .collect();
        assert_eq!(
            notices,
            [(
                engram_dashboard_agent::launch::BUILTIN_UNAVAILABLE,
                &json!({ "path": "engram/default", "why": "no built-in folder" })
            )]
        );
        assert!(!view.notices[0].text.is_empty());
    }

    /// Each folder stat lands as its word, and only `unreadable` carries its reason.
    #[test]
    fn every_folder_status_lands_as_its_word() {
        let row = |status: Option<FolderStatus>| {
            folder_row(PresetFolder {
                path: "C:/f".to_owned(),
                line: None,
                status,
            })
        };
        let cases = [
            (None, None, None),
            (Some(FolderStatus::Ok), Some("ok"), None),
            (Some(FolderStatus::Missing), Some("missing"), None),
            (
                Some(FolderStatus::NotADirectory),
                Some("not_a_directory"),
                None,
            ),
            (
                Some(FolderStatus::Unreadable("timed out".to_owned())),
                Some("unreadable"),
                Some("timed out"),
            ),
        ];
        for (status, word, why) in cases {
            let out = row(status.clone());
            assert_eq!(
                (out.status.as_deref(), out.status_why.as_deref()),
                (word, why),
                "{status:?}"
            );
        }
    }

    /// Each `UnsureWhy` variant lands as its word, its payload text in `message` and the front matter's line in
    /// `line`; paths are `/`-separated.
    #[test]
    fn every_unsure_reason_lands_as_its_word_with_its_payload() {
        use engram_dashboard_agent::backend::settings_check::FrontMatterError;
        let cases = [
            (
                UnsureWhy::Unreadable("denied".to_owned()),
                ("unreadable", Some("denied"), None),
            ),
            (
                UnsureWhy::FrontMatter(FrontMatterError {
                    message: "bad".to_owned(),
                    line: Some(4),
                }),
                ("front_matter", Some("bad"), Some(4)),
            ),
            (UnsureWhy::Link, ("link", None, None)),
            (
                UnsureWhy::DashesInFrontMatter,
                ("dashes_in_front_matter", None, None),
            ),
            (UnsureWhy::UnclearPaths, ("unclear_paths", None, None)),
            (UnsureWhy::EmptyBody, ("empty_body", None, None)),
            (UnsureWhy::HookKeys, ("hook_keys", None, None)),
            (
                UnsureWhy::SkillFileInCommandFolder,
                ("skill_file_in_command_folder", None, None),
            ),
        ];
        for (why, (word, message, line)) in cases {
            let out = unsure_row(FolderUnsure {
                path: PathBuf::from("rules").join("x.md"),
                kind: Some(FolderLoadKind::Rule),
                why,
            });
            assert_eq!(out.path, "rules/x.md");
            assert_eq!(out.kind, Some(FolderLoadKindWord::Rule));
            assert_eq!(
                (out.why.as_str(), out.message.as_deref(), out.line),
                (word, message, line)
            );
        }
    }

    // ── agent presets ──

    fn claude_agent(presets: &[&str]) -> AgentProfile {
        let mut profile = agent(presets, true);
        profile.command = AgentCommand::Claude {
            extra_args: Vec::new(),
            output_format: CoreOutputFormat::Terminal,
        };
        profile
    }

    fn reason(code: &str, path: &str, line: Option<u32>) -> Reason {
        Reason {
            code: code.to_owned(),
            args: json!({ "path": path }),
            line,
        }
    }

    /// The body's `agent.presets` view crosses the port field by field in declared rows, by id: the working folder
    /// lock as its word, each folder with its stat words and chain, the problem rows, the notices with their text.
    #[test]
    fn an_agent_view_crosses_the_port_field_by_field_by_id() {
        let agent = AgentId::new_v4();
        let folder = |path: &str, status: Option<FolderStatus>| ResolvedFolder {
            path: PathBuf::from(path),
            key: engram_dashboard_platform::fs::path_key(Path::new(path)),
            via: vec!["library/P.yaml".to_owned(), "library/Q.yaml".to_owned()],
            line: 3,
            instructions_at: 0,
            status,
        };
        let missing = BlockReason::Missing {
            item: "library/m.md".to_owned(),
            via: vec!["library/P.yaml".to_owned()],
            reason: reason("not-found", "m.md", Some(5)),
        };
        let conflict = BlockReason::Conflict {
            reason: reason("c", "x", None),
            items: vec![("a.md".to_owned(), vec!["library/P.yaml".to_owned()])],
        };
        let notice = reason("omitted", "n.md", Some(2));
        let view = AgentPresetsView {
            agent,
            default_preset: false,
            presets: vec!["P.yaml".to_owned(), "Q.yaml".to_owned()],
            cwd: PathBuf::from("C:/work/bob"),
            workspace_lock: Some(WorkspaceLock::Preparing),
            backend: "codex".to_owned(),
            mode: CoreOutputFormat::Terminal,
            base_rev: "b1".to_owned(),
            takes_presets: true,
            folders: vec![
                folder(
                    "D:/docs",
                    Some(FolderStatus::Unreadable("timed out".to_owned())),
                ),
                folder("D:/gone", Some(FolderStatus::Missing)),
                folder("D:/later", None),
            ],
            conflicts: vec![conflict.clone()],
            problems: vec![missing.clone()],
            notices: vec![notice.clone()],
        };

        let out = agent_view(view);

        let row = |path: &str, status: Option<&str>, why: Option<&str>| AgentFolderRow {
            path: path.to_owned(),
            via: vec!["library/P.yaml".to_owned(), "library/Q.yaml".to_owned()],
            line: 3,
            status: status.map(str::to_owned),
            status_why: why.map(str::to_owned),
        };
        assert_eq!(
            out,
            AgentPresetsPortView {
                agent_id: agent,
                default_preset: false,
                presets: vec!["P.yaml".to_owned(), "Q.yaml".to_owned()],
                cwd: "C:/work/bob".to_owned(),
                workspace_lock: Some("preparing".to_owned()),
                backend: "codex".to_owned(),
                mode: AgentOutputFormat::Terminal,
                base_rev: "b1".to_owned(),
                takes_presets: true,
                folders: vec![
                    row("D:/docs", Some("unreadable"), Some("timed out")),
                    row("D:/gone", Some("missing"), None),
                    row("D:/later", None, None),
                ],
                conflicts: vec![problem_row(conflict)],
                problems: vec![problem_row(missing)],
                notices: vec![launch_notice_row(notice)],
            }
        );
    }

    /// Every working-folder lock lands as its own word; no lock is `null`.
    #[test]
    fn every_workspace_lock_lands_as_its_word() {
        let cases = [
            (None, None),
            (Some(WorkspaceLock::Running), Some("running")),
            (Some(WorkspaceLock::Preparing), Some("preparing")),
            (Some(WorkspaceLock::Conversed), Some("conversed")),
        ];
        for (lock, word) in cases {
            let view = AgentPresetsView {
                agent: AgentId::new_v4(),
                default_preset: true,
                presets: Vec::new(),
                cwd: PathBuf::from("/work"),
                workspace_lock: lock,
                backend: "claude".to_owned(),
                mode: CoreOutputFormat::StreamJson,
                base_rev: "b".to_owned(),
                takes_presets: false,
                folders: Vec::new(),
                conflicts: Vec::new(),
                problems: Vec::new(),
                notices: Vec::new(),
            };
            assert_eq!(agent_view(view).workspace_lock.as_deref(), word, "{lock:?}");
        }
    }

    /// Through the port, `agent.presets` answers the body's view and the edits answer what they committed: `attach`
    /// its applied presets and skips as rows, `reorder` (with the shown `base_rev`) and `detach` the view after the
    /// change — each the attachments now in the registry.
    #[test]
    fn the_agent_preset_commands_answer_the_bodies_answers_in_declared_rows() {
        let profile = claude_agent(&["library/p.yaml"]);
        let id = profile.id;
        let w = World::new(vec![profile]);
        let _port = w.plug();
        w.put("p.yaml", "kind: preset\nversion: 1\n");
        w.put("q.yaml", "kind: preset\nversion: 1\n");

        let shown = w.port().agent_presets(id).unwrap();
        assert_eq!(shown, agent_view(w.svc.agent_presets(id).unwrap()));
        assert_eq!(shown.agent_id, id);

        let attach = w
            .port()
            .attach_preset(
                id,
                &strings(&["library/q.yaml", "library/gone.yaml"]),
                None,
                None,
            )
            .unwrap();
        assert_eq!(attach.applied, strings(&["library/q.yaml"]));
        assert_eq!(
            attach.skipped,
            vec![SkippedRow {
                item: "library/gone.yaml".to_owned(),
                code: crate::NOT_FOUND.to_owned(),
                args: json!({ "path": "library/gone.yaml" }),
                line: None,
            }]
        );
        assert_eq!(
            attach.view.presets,
            strings(&["library/p.yaml", "library/q.yaml"])
        );

        let reorder = w
            .port()
            .reorder_presets(
                id,
                &strings(&["library/q.yaml", "library/p.yaml"]),
                &attach.view.base_rev,
                None,
            )
            .unwrap();
        assert_eq!(
            reorder.presets,
            strings(&["library/q.yaml", "library/p.yaml"])
        );
        let detach = w
            .port()
            .detach_preset(
                id,
                &strings(&["engram/default", "library/q.yaml"]),
                true,
                None,
            )
            .unwrap();
        assert_eq!(
            (detach.default_preset, detach.presets.clone()),
            (false, strings(&["library/p.yaml"]))
        );
        assert_eq!(
            w.registry.attachments_of(id).expect("the agent"),
            Attachments {
                default_preset: false,
                presets: vec![AttachedPreset {
                    path: "library/p.yaml".to_owned()
                }],
            }
        );
        assert_eq!(detach, agent_view(w.svc.agent_presets(id).unwrap()));
    }

    /// The agent-preset refusals cross 1:1: a stale `base_rev` `Conflict` `rev-mismatch`, the Default slot without
    /// `yes` `Conflict` `default-preset-detach`, no port `Internal` `profile-edit-unavailable`.
    #[test]
    fn the_agent_preset_refusals_cross_with_their_class() {
        let profile = claude_agent(&["library/p.yaml"]);
        let id = profile.id;
        let w = World::new(vec![profile]);
        w.put("p.yaml", "kind: preset\nversion: 1\n");
        let (class, reason) = refused(w.port().agent_presets(id));
        assert_eq!(
            (class, reason.code.as_str()),
            (
                PortClass::Internal,
                crate::agent_edit::PROFILE_EDIT_UNAVAILABLE
            )
        );

        let _port = w.plug();
        let (class, reason) =
            refused(
                w.port()
                    .reorder_presets(id, &strings(&["library/p.yaml"]), "stale", None),
            );
        assert_eq!(
            (class, reason.code.as_str()),
            (PortClass::Conflict, crate::edit::REV_MISMATCH)
        );
        let (class, reason) =
            refused(
                w.port()
                    .detach_preset(id, &strings(&["engram/default"]), false, None),
            );
        assert_eq!(
            (class, reason.code.as_str()),
            (
                PortClass::Conflict,
                crate::agent_edit::DEFAULT_PRESET_DETACH
            )
        );
    }

    /// The `agent.new {presets}` judgement crosses the port with its attachments as judged and its skips as rows; a
    /// stale root generation crosses as `Conflict` `root-generation-stale`, and no profile is added either way.
    #[test]
    fn the_new_agent_judgement_crosses_the_port_with_its_skips_as_rows() {
        let w = World::new(Vec::new());
        w.put("p.yaml", "kind: preset\nversion: 1\n");
        w.put("rules/a.md", "a");
        let new = claude_agent(&[]);
        let current = w.svc.status().root_generation;

        let judged = w
            .port()
            .judge_new_presets(
                &new,
                &strings(&["library/p.yaml", "library/rules/a.md", "engram/default"]),
                Some(current),
            )
            .unwrap();
        assert_eq!(
            judged,
            NewAgentPresets {
                attachments: Attachments {
                    default_preset: true,
                    presets: vec![AttachedPreset {
                        path: "library/p.yaml".to_owned()
                    }],
                },
                skipped: vec![
                    SkippedRow {
                        item: "library/rules/a.md".to_owned(),
                        code: crate::format::read::WRONG_KIND.to_owned(),
                        args: json!({ "path": "library/rules/a.md" }),
                        line: None,
                    },
                    SkippedRow {
                        item: "engram/default".to_owned(),
                        code: crate::edit::ALREADY_PRESENT.to_owned(),
                        args: json!({ "path": "engram/default" }),
                        line: None,
                    },
                ],
                change: Some(w.svc.change_number()),
            }
        );
        let (class, reason) = refused(w.port().judge_new_presets(
            &new,
            &strings(&["library/p.yaml"]),
            Some(current + 1),
        ));
        assert_eq!(
            (class, reason.code.as_str()),
            (PortClass::Conflict, crate::ROOT_GENERATION_STALE)
        );
        assert!(w.registry.get(new.id).is_none());
    }

    /// With no library service the judgement answers in port types: a library preset `Unavailable`
    /// `library-unavailable` with the given reason; `engram/default` alone judged without it.
    #[test]
    fn the_judgement_with_no_library_answers_the_attach_rule_in_port_types() {
        let new = claude_agent(&[]);
        let (class, reason) = refused(judge_new_presets_with_no_library(
            &new,
            &strings(&["library/p.yaml"]),
            "none here",
        ));
        assert_eq!(
            (class, reason.code.as_str(), reason.args),
            (
                PortClass::Unavailable,
                LIBRARY_UNAVAILABLE,
                json!({ "message": "none here" })
            )
        );
        let judged =
            judge_new_presets_with_no_library(&new, &strings(&["engram/default"]), "none here")
                .unwrap();
        assert_eq!(judged.attachments.presets, Vec::new());
        assert_eq!(judged.skipped.len(), 1);
        assert_eq!(judged.skipped[0].code, crate::edit::ALREADY_PRESENT);
    }

    // ── files and trash (ADR-0419) ───────────────────────────────────────────────────────────────────────────────

    fn address(bundle: &str, path: &str) -> TrashAddress {
        TrashAddress {
            mount: "library".to_owned(),
            bundle: bundle.to_owned(),
            path: path.to_owned(),
        }
    }

    fn address_json(bundle: &str, path: &str) -> Value {
        json!({ "mount": "library", "bundle": bundle, "path": path })
    }

    fn why(code: &str, args: Value) -> Reason {
        Reason {
            code: code.to_owned(),
            args,
            line: None,
        }
    }

    /// Every outcome lands on its flat row and every skip on its code: `done` fills the result fields and leaves
    /// `code` / `args` null; a skip's reason is made here (`inside-given-folder` `{folder}` · `in-place` `{}` ·
    /// `name-exists` `{path}` · `name-taken-during-copy` `{path, …staging}`); a conflict and a failure carry their
    /// reason as built, a staging's fields inside `args`. The mapping, the git trees, the skipped files and a stop
    /// naming the command are carried field by field, the agents by id.
    #[test]
    fn a_move_answer_lands_every_outcome_and_skip_on_its_row() {
        let staged = json!({
            "path": "library/b.md", "staging": "team/~tmp", "staging_disk": "D:/team/~tmp",
            "staging_kept": "trash-not-reserved", "not_copied": "x",
        });
        let agent_id = AgentId::new_v4();
        let answer = MoveAnswer {
            items: vec![
                MoveItem {
                    path: "library/a".to_owned(),
                    outcome: Outcome::Done(Moved {
                        to: "team/a".to_owned(),
                        trashed: vec![address("b1", "a/x.md")],
                        original: Some(address("b2", "a")),
                        left: vec![(
                            "library/a/y.md".to_owned(),
                            why(NAME_EXISTS, json!({ "path": "team/a/y.md" })),
                        )],
                    }),
                },
                MoveItem {
                    path: "library/d/e.md".to_owned(),
                    outcome: Outcome::Skipped(Skip::Inside("library/d".to_owned())),
                },
                MoveItem {
                    path: "library/f.md".to_owned(),
                    outcome: Outcome::Skipped(Skip::InPlace),
                },
                MoveItem {
                    path: "library/g.md".to_owned(),
                    outcome: Outcome::Skipped(Skip::Taken("team/g.md".to_owned())),
                },
                MoveItem {
                    path: "library/h.md".to_owned(),
                    outcome: Outcome::Skipped(Skip::TakenDuringCopy {
                        path: "team/h.md".to_owned(),
                        staging: json!({ "staging_row": address_json("b3", "~tmp") }),
                    }),
                },
                MoveItem {
                    path: "library/i.md".to_owned(),
                    outcome: Outcome::Conflict(why(NAME_EXISTS, json!({ "path": "team/i.md" }))),
                },
                MoveItem {
                    path: "library/b.md".to_owned(),
                    outcome: Outcome::Failed(why(crate::ops::SOURCE_CHANGED, staged.clone())),
                },
            ],
            mapping: vec![
                ("library/a".to_owned(), "team/a".to_owned(), false),
                ("library/s".to_owned(), "team/s".to_owned(), true),
            ],
            detached: vec!["library/s".to_owned()],
            written: vec!["library/p.yaml".to_owned()],
            skipped: vec![RelinkSkip {
                path: "other".to_owned(),
                reason: why(LIBRARY_UNAVAILABLE, json!({ "mount": "other" })),
            }],
            trees: vec![
                ("library".to_owned(), GitTree::At(PathBuf::from("C:/lib"))),
                ("team".to_owned(), GitTree::Outside),
                ("other".to_owned(), GitTree::Unknown),
            ],
            agents: vec![agent_id],
            stopped: Some(RelinkStop {
                path: Some("library/q.yaml".to_owned()),
                refusal: LibraryRefusal::Refused {
                    class: RefusalClass::Unavailable,
                    reason: why(
                        crate::edit::LIBRARY_WRITE_FAILED,
                        json!({ "path": "library/q.yaml" }),
                    ),
                },
            }),
        };
        let out = move_answer("library.move", answer);
        let row = |path: &str, status: &str, code: &str, args: Value| MoveItemRow {
            path: path.to_owned(),
            status: status.to_owned(),
            to: None,
            trashed: Vec::new(),
            original: None,
            left: Vec::new(),
            code: Some(code.to_owned()),
            args,
        };
        let mut taken_during_copy = json!({ "staging_row": address_json("b3", "~tmp") });
        taken_during_copy["path"] = json!("team/h.md");
        assert_eq!(
            out.items,
            [
                MoveItemRow {
                    path: "library/a".to_owned(),
                    status: "done".to_owned(),
                    to: Some("team/a".to_owned()),
                    trashed: vec![address_row(address("b1", "a/x.md"))],
                    original: Some(address_row(address("b2", "a"))),
                    left: vec![PathReasonRow {
                        path: "library/a/y.md".to_owned(),
                        code: NAME_EXISTS.to_owned(),
                        args: json!({ "path": "team/a/y.md" }),
                        line: None,
                    }],
                    code: None,
                    args: Value::Null,
                },
                row(
                    "library/d/e.md",
                    "skipped",
                    INSIDE_GIVEN_FOLDER,
                    json!({ "folder": "library/d" })
                ),
                row("library/f.md", "skipped", IN_PLACE, json!({})),
                row(
                    "library/g.md",
                    "skipped",
                    NAME_EXISTS,
                    json!({ "path": "team/g.md" })
                ),
                row(
                    "library/h.md",
                    "skipped",
                    NAME_TAKEN_DURING_COPY,
                    taken_during_copy
                ),
                row(
                    "library/i.md",
                    "conflict",
                    NAME_EXISTS,
                    json!({ "path": "team/i.md" })
                ),
                row("library/b.md", "failed", crate::ops::SOURCE_CHANGED, staged),
            ]
        );
        assert_eq!(
            out.mapping,
            [
                MovePairRow {
                    from: "library/a".to_owned(),
                    to: "team/a".to_owned(),
                    exact: false,
                },
                MovePairRow {
                    from: "library/s".to_owned(),
                    to: "team/s".to_owned(),
                    exact: true,
                },
            ]
        );
        assert_eq!(
            (out.detached.as_slice(), out.written.as_slice()),
            (
                ["library/s".to_owned()].as_slice(),
                ["library/p.yaml".to_owned()].as_slice()
            )
        );
        assert_eq!(
            out.skipped,
            [PathReasonRow {
                path: "other".to_owned(),
                code: LIBRARY_UNAVAILABLE.to_owned(),
                args: json!({ "mount": "other" }),
                line: None,
            }]
        );
        assert_eq!(
            out.trees,
            [
                GitTreeRow {
                    mount: "library".to_owned(),
                    git: Some(true),
                    top: Some(PathBuf::from("C:/lib").display().to_string()),
                },
                GitTreeRow {
                    mount: "team".to_owned(),
                    git: Some(false),
                    top: None,
                },
                GitTreeRow {
                    mount: "other".to_owned(),
                    git: None,
                    top: None,
                },
            ]
        );
        assert_eq!(out.agents, [agent_id]);
        let stop = out.stopped.expect("stopped");
        assert_eq!(
            (stop.path.as_deref(), stop.code.as_str()),
            (Some("library/q.yaml"), crate::edit::LIBRARY_WRITE_FAILED)
        );
        assert!(
            stop.message.starts_with("library.move: "),
            "{}",
            stop.message
        );
    }

    /// A copy, a delete and a restore map their outcomes as a move's: a copy's displaced entry as a list of none or one,
    /// a delete's trash row, a restore row keyed by its address with the path it is back at; a delete's reference fix
    /// and stop are carried, the stop naming `library.delete`.
    #[test]
    fn copy_delete_and_restore_answers_land_on_their_rows() {
        let copied = copy_ok(CopyAnswer {
            items: vec![
                CopyItem {
                    path: "library/a.md".to_owned(),
                    outcome: Outcome::Done(Copied {
                        to: "library/a - Copy.md".to_owned(),
                        trashed: None,
                    }),
                },
                CopyItem {
                    path: "library/b.md".to_owned(),
                    outcome: Outcome::Done(Copied {
                        to: "team/b.md".to_owned(),
                        trashed: Some(address("b1", "b.md")),
                    }),
                },
                CopyItem {
                    path: "library/c".to_owned(),
                    outcome: Outcome::Conflict(why(
                        crate::ops::COPY_MERGE_REFUSED,
                        json!({ "path": "library/c", "to": "team/c" }),
                    )),
                },
            ],
        });
        assert_eq!(
            copied.items,
            [
                CopyItemRow {
                    path: "library/a.md".to_owned(),
                    status: "done".to_owned(),
                    to: Some("library/a - Copy.md".to_owned()),
                    trashed: Vec::new(),
                    code: None,
                    args: Value::Null,
                },
                CopyItemRow {
                    path: "library/b.md".to_owned(),
                    status: "done".to_owned(),
                    to: Some("team/b.md".to_owned()),
                    trashed: vec![address_row(address("b1", "b.md"))],
                    code: None,
                    args: Value::Null,
                },
                CopyItemRow {
                    path: "library/c".to_owned(),
                    status: "conflict".to_owned(),
                    to: None,
                    trashed: Vec::new(),
                    code: Some(crate::ops::COPY_MERGE_REFUSED.to_owned()),
                    args: json!({ "path": "library/c", "to": "team/c" }),
                },
            ]
        );

        let agent_id = AgentId::new_v4();
        let deleted = delete_answer(DeleteAnswer {
            items: vec![
                DeleteItem {
                    path: "library/a.md".to_owned(),
                    outcome: Outcome::Done(address("b1", "a.md")),
                },
                DeleteItem {
                    path: "library/x".to_owned(),
                    outcome: Outcome::Failed(why(
                        crate::gate::NOT_FOUND,
                        json!({ "path": "library/x" }),
                    )),
                },
            ],
            detached: vec!["library/a.md".to_owned()],
            written: vec!["library/p.yaml".to_owned()],
            skipped: Vec::new(),
            agents: vec![agent_id],
            stopped: Some(RelinkStop {
                path: None,
                refusal: LibraryRefusal::Store(StoreError::Refused(Refusal::Unreadable)),
            }),
        });
        assert_eq!(
            deleted.items,
            [
                DeleteItemRow {
                    path: "library/a.md".to_owned(),
                    status: "done".to_owned(),
                    trashed: Some(address_row(address("b1", "a.md"))),
                    code: None,
                    args: Value::Null,
                },
                DeleteItemRow {
                    path: "library/x".to_owned(),
                    status: "failed".to_owned(),
                    trashed: None,
                    code: Some(crate::gate::NOT_FOUND.to_owned()),
                    args: json!({ "path": "library/x" }),
                },
            ]
        );
        assert_eq!(
            (
                deleted.detached.as_slice(),
                deleted.written.as_slice(),
                deleted.agents.as_slice()
            ),
            (
                ["library/a.md".to_owned()].as_slice(),
                ["library/p.yaml".to_owned()].as_slice(),
                [agent_id].as_slice()
            )
        );
        let stop = deleted.stopped.expect("stopped");
        assert_eq!(
            stop.code,
            engram_dashboard_agent::library_commands::PROFILE_STORE_REFUSED
        );
        assert!(
            stop.message.starts_with("library.delete: "),
            "{}",
            stop.message
        );

        let restored = restore_ok(RestoreAnswer {
            items: vec![
                crate::ops::RestoreItem {
                    row: address("b1", "a.md"),
                    outcome: Outcome::Done("library/a (2).md".to_owned()),
                },
                crate::ops::RestoreItem {
                    row: address("b1", "b.md"),
                    outcome: Outcome::Conflict(why(NAME_EXISTS, json!({ "path": "library/b.md" }))),
                },
            ],
        });
        assert_eq!(
            restored.items,
            [
                RestoreItemRow {
                    mount: "library".to_owned(),
                    bundle: "b1".to_owned(),
                    path: "a.md".to_owned(),
                    status: "done".to_owned(),
                    to: Some("library/a (2).md".to_owned()),
                    code: None,
                    args: Value::Null,
                },
                RestoreItemRow {
                    mount: "library".to_owned(),
                    bundle: "b1".to_owned(),
                    path: "b.md".to_owned(),
                    status: "conflict".to_owned(),
                    to: None,
                    code: Some(NAME_EXISTS.to_owned()),
                    args: json!({ "path": "library/b.md" }),
                },
            ]
        );
    }

    /// Through the port on a real library: a delete answers the trash row it made, the trash view lists that bundle
    /// (its id, `deleted_at` from the id, the row with its `original`, restorable), and a restore of that row puts the
    /// file back and answers where — each one body call mapped as built.
    #[test]
    fn a_delete_lists_in_the_trash_view_and_restores_through_the_port() {
        let w = World::new(Vec::new());
        w.put("rules/a.md", "A");
        let deleted = w
            .port()
            .delete(&strings(&["library/rules/a.md"]), false, false, None, None)
            .expect("delete");
        assert_eq!(deleted.items.len(), 1);
        let item = &deleted.items[0];
        assert_eq!(
            (item.path.as_str(), item.status.as_str(), &item.code),
            ("library/rules/a.md", "done", &None)
        );
        let trashed = item.trashed.clone().expect("a trash row");
        assert_eq!(
            (trashed.mount.as_str(), trashed.path.as_str()),
            ("library", "rules/a.md")
        );
        assert!(!w.base.join("lib/rules/a.md").exists());

        let view = w.port().list_trash().expect("view");
        let library = view
            .mounts
            .iter()
            .find(|mount| mount.mount == "library")
            .expect("the library mount");
        assert_eq!(library.problem, None);
        let id = crate::trash::BundleId::parse(&trashed.bundle).expect("a bundle id");
        assert_eq!(
            library.bundles,
            [TrashBundleRow {
                bundle: trashed.bundle.clone(),
                deleted_at: id.deleted_at_rfc3339(),
                source: "record".to_owned(),
                why: None,
                trouble: None,
                rows: vec![TrashViewRow {
                    path: "rules/a.md".to_owned(),
                    original: "library/rules/a.md".to_owned(),
                    kind: "root".to_owned(),
                    state: "present".to_owned(),
                    entry: Some("file".to_owned()),
                    why: None,
                    staging: false,
                    intended: None,
                    restorable: true,
                }],
            }]
        );

        let restored = w
            .port()
            .restore(&[trashed.clone()], false, None)
            .expect("restore");
        assert_eq!(
            restored.items,
            [RestoreItemRow {
                mount: trashed.mount,
                bundle: trashed.bundle,
                path: trashed.path,
                status: "done".to_owned(),
                to: Some("library/rules/a.md".to_owned()),
                code: None,
                args: Value::Null,
            }]
        );
        assert_eq!(
            fs::read_to_string(w.base.join("lib/rules/a.md")).unwrap(),
            "A"
        );
    }

    /// `deleted_at` is the bundle id's time as RFC 3339 UTC.
    #[test]
    fn deleted_at_is_the_bundle_time_in_rfc_3339() {
        let id = crate::trash::BundleId::parse("20261123T081500Z-abc123").expect("an id");
        assert_eq!(id.deleted_at_rfc3339(), "2026-11-23T08:15:00Z");
    }

    /// A bundle with no record lists its files and empty folders as `found` rows; a row whose original holds a dot name
    /// — a `.DS_Store`, a file under `.hidden/` — is not restorable, as restore itself would refuse it; a plain file is.
    #[test]
    fn a_recordless_row_under_a_dot_name_is_not_restorable() {
        let w = World::new(Vec::new());
        let bundle = "20261123T081500Z-abc123";
        w.put(&format!(".trash/{bundle}/.DS_Store"), "x");
        w.put(&format!(".trash/{bundle}/.hidden/file.md"), "x");
        w.put(&format!(".trash/{bundle}/plain.md"), "x");
        let view = w.port().list_trash().expect("view");
        let library = view
            .mounts
            .iter()
            .find(|mount| mount.mount == "library")
            .expect("the library mount");
        assert_eq!(library.bundles.len(), 1, "{:?}", library.bundles);
        let found = &library.bundles[0];
        assert_eq!(
            (
                found.bundle.as_str(),
                found.deleted_at.as_str(),
                found.source.as_str()
            ),
            (bundle, "2026-11-23T08:15:00Z", "no_record")
        );
        let rows: Vec<(&str, &str, &str, bool)> = found
            .rows
            .iter()
            .map(|row| {
                (
                    row.path.as_str(),
                    row.original.as_str(),
                    row.kind.as_str(),
                    row.restorable,
                )
            })
            .collect();
        assert_eq!(
            rows,
            [
                (".DS_Store", "library/.DS_Store", "found", false),
                (".hidden/file.md", "library/.hidden/file.md", "found", false),
                ("plain.md", "library/plain.md", "found", true),
            ]
        );
    }

    /// A mount whose `.trash` is not a folder is named with its problem and no bundles, not failed.
    #[test]
    fn an_unusable_trash_is_named_with_its_problem() {
        let w = World::new(Vec::new());
        w.put(".trash", "not a folder");
        let view = w.port().list_trash().expect("view");
        let library = view
            .mounts
            .iter()
            .find(|mount| mount.mount == "library")
            .expect("the library mount");
        let problem = library.problem.as_ref().expect("a problem");
        assert_eq!(
            (problem.code.as_str(), &problem.args["mount"]),
            (crate::trash::TRASH_UNUSABLE, &json!("library"))
        );
        assert!(library.bundles.is_empty());
    }

    /// Through the port on a real library: emptying a bundle of only the rows given removes those for good, a path that
    /// is no row answers `not_found` with its reason, and the bundle stays (`remaining`) while a row of it is left; the
    /// left row then goes on its own and the bundle with it (`gone`). Emptying nothing refuses `nothing-to-empty`.
    // ADR-0408
    // ADR-0419
    #[test]
    fn an_emptying_through_the_port_answers_each_row_and_bundle() {
        let w = World::new(Vec::new());
        w.put("rules/a.md", "A");
        w.put("rules/b.md", "B");
        let deleted = w
            .port()
            .delete(
                &strings(&["library/rules/a.md", "library/rules/b.md"]),
                false,
                false,
                None,
                None,
            )
            .expect("delete");
        let bundle = deleted.items[0]
            .trashed
            .clone()
            .expect("a trash row")
            .bundle;
        let row = |path: &str| TrashAddressRow {
            mount: "library".to_owned(),
            bundle: bundle.clone(),
            path: path.to_owned(),
        };

        let emptied = w
            .port()
            .empty_trash(
                &[ListedBundleArg {
                    mount: "library".to_owned(),
                    bundle: bundle.clone(),
                    rows: strings(&["rules/a.md"]),
                }],
                &[row("rules/nope.md")],
            )
            .expect("emptied");
        assert_eq!(
            emptied.rows[0],
            EmptiedRow {
                mount: "library".to_owned(),
                bundle: bundle.clone(),
                path: "rules/a.md".to_owned(),
                outcome: "gone".to_owned(),
                code: None,
                args: Value::Null,
            }
        );
        let missing = &emptied.rows[1];
        assert_eq!(
            (
                missing.path.as_str(),
                missing.outcome.as_str(),
                missing.code.as_deref()
            ),
            ("rules/nope.md", "not_found", Some(crate::NOT_FOUND))
        );
        assert_eq!(emptied.rows.len(), 2);
        assert_eq!(
            emptied.bundles,
            [EmptiedBundleRow {
                mount: "library".to_owned(),
                bundle: bundle.clone(),
                outcome: "remaining".to_owned(),
                untouched: None,
            }]
        );
        let view = w.port().list_trash().expect("view");
        let library = view
            .mounts
            .iter()
            .find(|mount| mount.mount == "library")
            .expect("the library mount");
        let left: Vec<&str> = library.bundles[0]
            .rows
            .iter()
            .map(|row| row.path.as_str())
            .collect();
        assert_eq!(left, ["rules/b.md"]);

        let emptied = w
            .port()
            .empty_trash(&[], &[row("rules/b.md")])
            .expect("emptied");
        assert_eq!(emptied.rows[0].outcome, "gone");
        assert_eq!(emptied.bundles[0].outcome, "gone");
        assert!(!w.base.join("lib/.trash").join(&bundle).exists());

        let (class, why) = refused(w.port().empty_trash(&[], &[]));
        assert_eq!(
            (class, why.code.as_str()),
            (PortClass::InvalidArgument, crate::trash::NOTHING_TO_EMPTY)
        );
    }

    /// A row left standing carries its reason as built — the OS error code inside `args` —, a row that is no row its
    /// `not-found`, and a bundle left untouched its reason as `untouched`; every outcome lands as its word.
    // ADR-0408 decision 2
    #[test]
    fn every_emptying_outcome_lands_as_its_word_with_its_reason() {
        let failed = json!({ "path": "library/d/x.md", "os_error": 5 });
        let unread = json!({ "mount": "team", "message": "m" });
        let answer = emptied_ok(EmptyAnswer {
            rows: vec![
                crate::trash::EmptiedRow {
                    row: address("b1", "a.md"),
                    outcome: Emptied::Gone,
                },
                crate::trash::EmptiedRow {
                    row: address("b1", "d"),
                    outcome: Emptied::Remaining(why("library-write-failed", failed.clone())),
                },
                crate::trash::EmptiedRow {
                    row: address("b2", "z.md"),
                    outcome: Emptied::NotFound(why("not-found", json!({ "path": "z.md" }))),
                },
            ],
            bundles: vec![
                EmptiedBundle {
                    mount: "library".to_owned(),
                    bundle: "b1".to_owned(),
                    outcome: BundleOutcome::Remaining,
                    untouched: None,
                },
                EmptiedBundle {
                    mount: "library".to_owned(),
                    bundle: "b2".to_owned(),
                    outcome: BundleOutcome::NotFound,
                    untouched: None,
                },
                EmptiedBundle {
                    mount: "team".to_owned(),
                    bundle: "b3".to_owned(),
                    outcome: BundleOutcome::Remaining,
                    untouched: Some(why("library-unavailable", unread.clone())),
                },
                EmptiedBundle {
                    mount: "library".to_owned(),
                    bundle: "b4".to_owned(),
                    outcome: BundleOutcome::Gone,
                    untouched: None,
                },
            ],
        });
        let rows: Vec<(&str, &str, Option<&str>, &Value)> = answer
            .rows
            .iter()
            .map(|row| {
                (
                    row.path.as_str(),
                    row.outcome.as_str(),
                    row.code.as_deref(),
                    &row.args,
                )
            })
            .collect();
        assert_eq!(
            rows,
            [
                ("a.md", "gone", None, &Value::Null),
                ("d", "remaining", Some("library-write-failed"), &failed),
                (
                    "z.md",
                    "not_found",
                    Some("not-found"),
                    &json!({ "path": "z.md" })
                ),
            ]
        );
        assert_eq!(answer.rows[1].bundle, "b1");
        assert_eq!(answer.rows[1].mount, "library");
        let bundles: Vec<(&str, &str, Option<&TrashProblemRow>)> = answer
            .bundles
            .iter()
            .map(|bundle| {
                (
                    bundle.bundle.as_str(),
                    bundle.outcome.as_str(),
                    bundle.untouched.as_ref(),
                )
            })
            .collect();
        let untouched = TrashProblemRow {
            code: "library-unavailable".to_owned(),
            args: unread,
        };
        assert_eq!(
            bundles,
            [
                ("b1", "remaining", None),
                ("b2", "not_found", None),
                ("b3", "remaining", Some(&untouched)),
                ("b4", "gone", None),
            ]
        );
        assert_eq!(answer.bundles[2].mount, "team");
    }

    /// Through the port on a real library: a move answers its rows and the mapping its references followed — the
    /// preset that named the item is rewritten (`written`); a move into the item's own folder is skipped `in-place`;
    /// a taken name under `Skip` is skipped `name-exists`; a rename and a copy into the item's own folder answer as
    /// built.
    #[test]
    fn move_rename_and_copy_answer_through_the_port() {
        let w = World::new(Vec::new());
        w.put("rules/a.md", "A");
        w.put("rules/b.md", "B");
        w.put("other/b.md", "taken");
        w.put(
            "p.yaml",
            "kind: preset\nversion: 1\ninstructions:\n  - library/rules/a.md\n",
        );
        let moved = w
            .port()
            .move_items(
                &strings(&[
                    "library/rules/a.md",
                    "library/rules/b.md",
                    "library/other/b.md",
                ]),
                "library/other",
                Some(LibraryOnConflict::Skip),
                false,
                false,
                None,
                None,
            )
            .expect("move");
        let rows: Vec<(&str, &str, Option<&str>, Option<&str>)> = moved
            .items
            .iter()
            .map(|row| {
                (
                    row.path.as_str(),
                    row.status.as_str(),
                    row.to.as_deref(),
                    row.code.as_deref(),
                )
            })
            .collect();
        assert_eq!(
            rows,
            [
                (
                    "library/rules/a.md",
                    "done",
                    Some("library/other/a.md"),
                    None
                ),
                ("library/rules/b.md", "skipped", None, Some(NAME_EXISTS)),
                ("library/other/b.md", "skipped", None, Some(IN_PLACE)),
            ]
        );
        assert_eq!(moved.items[1].args, json!({ "path": "library/other/b.md" }));
        assert_eq!(
            moved.mapping,
            [MovePairRow {
                from: "library/rules/a.md".to_owned(),
                to: "library/other/a.md".to_owned(),
                exact: false,
            }]
        );
        assert_eq!(moved.written, ["library/p.yaml"]);
        assert!(fs::read_to_string(w.base.join("lib/p.yaml"))
            .unwrap()
            .contains("library/other/a.md"));

        let renamed = w
            .port()
            .rename("library/other/a.md", "c.md", false, false, None, None)
            .expect("rename");
        assert_eq!(
            (
                renamed.items[0].status.as_str(),
                renamed.items[0].to.as_deref()
            ),
            ("done", Some("library/other/c.md"))
        );

        let copied = w
            .port()
            .copy_items(
                &strings(&["library/rules/b.md"]),
                "library/rules",
                None,
                None,
            )
            .expect("copy");
        assert_eq!(
            (
                copied.items[0].status.as_str(),
                copied.items[0].to.as_deref()
            ),
            ("done", Some("library/rules/b - Copy.md"))
        );
    }

    /// The `delete-unconfirmed` forms spell the wire's argument names: `forms.detach` added to the same call passes
    /// the declared entrance check and deserializes as the command's arguments, and sent back through the port it
    /// deletes and drops the reference.
    #[test]
    fn the_detach_form_fed_back_as_arguments_passes_the_entrance_check() {
        use engram_dashboard_agent::commands::{LibraryDeleteArgs, LibraryDeleteOk, COMMAND_SPECS};
        use engram_dashboard_command::{blocking_handler, CommandError, CommandTable};

        let w = World::new(Vec::new());
        w.put("rules/a.md", "A");
        w.put(
            "p.yaml",
            "kind: preset\nversion: 1\ninstructions:\n  - library/rules/a.md\n",
        );
        let paths = strings(&["library/rules/a.md"]);
        let (class, reason) = refused(w.port().delete(&paths, false, false, None, None));
        assert_eq!(
            (class, reason.code.as_str()),
            (PortClass::Conflict, crate::ops::DELETE_UNCONFIRMED)
        );
        let mut args = reason.args["forms"]["detach"].clone();
        args["paths"] = json!(paths);

        let mut table = CommandTable::new(COMMAND_SPECS);
        table
            .insert(
                "library.delete",
                blocking_handler(
                    |_: LibraryDeleteArgs| -> Result<LibraryDeleteOk, CommandError> {
                        unreachable!("only the entrance check runs")
                    },
                ),
            )
            .unwrap();
        table
            .check_args("library.delete", &args)
            .expect("the detach form passes the entrance check");
        let parsed: LibraryDeleteArgs =
            serde_json::from_value(args).expect("the detach form deserializes");

        let deleted = w
            .port()
            .delete(
                &parsed.paths,
                parsed.yes.unwrap_or(false),
                parsed.detach_refs.unwrap_or(false),
                parsed.plan_digest.as_deref(),
                None,
            )
            .expect("confirmed");
        assert_eq!(deleted.items[0].status, "done");
        assert_eq!(deleted.detached, ["library/rules/a.md"]);
        assert_eq!(deleted.written, ["library/p.yaml"]);
    }
}
