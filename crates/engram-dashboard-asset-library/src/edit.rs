//! Preset views and edits: the `preset.get` view, the preset edits (`add` · `remove` · `reorder` · `setBackend` ·
//! `save`), the draft check (`check`) and the folder preview (`previewFolder`) — inherent [`LibraryService`] methods
//! answering crate-local types.
//!
//! ## Rules (invariants)
//!
//! - **One edit, one change.** An edit passes the first-scan entrance (its failure `library-unavailable`) and is
//!   refused when its preset's mount cannot be read (walked again first — ADR-0400 decision 6), so its disk is never
//!   touched under the write lock; the folders an `add` names are stat'ed before the change, outside every lock; then one
//!   [`LibraryService::change_at`] runs, under its write lock and in this order: link admission
//!   (`LibraryService::admit`) before any read, the store read of the file's current bytes, the parse, the `baseRev`
//!   check against the `rev` of those bytes, the operation over [`PresetLists`] keeping every kept item's origin, no
//!   write when the lists come out unchanged, `format::rewrite_lists`, `LibraryStore::write_atomic`, and the item made
//!   from the written bytes applied per item.
//! - **Never re-created.** A file or parent missing at the read answers `not-found` before anything is written.
//! - **Bytes read are applied also on a refusal.** When the bytes read differ from the catalog item, the item made from
//!   them is applied before the edit can refuse, so the next edit is judged against them.
//! - **Views read no preset file.** `preset.get` stats the mounts it names, then answers from the catalog (`rev`
//!   included) under one read lock; an edit's answer view is built inside its own change, from the preset as committed.
//!   Their folder stats and link probes run after the lock. `previewFolder` reads no catalog and takes no library lock.
//!
//! ## Refusals every edit can answer
//!
//! A path of one name (a mount's top, or a name at the virtual root) `InvalidArgument` · `virtual-root-read-only`,
//! before anything else about the target (ADR-0400 decision 5); no first scan applied `Unavailable` ·
//! `library-unavailable`; the preset's mount cannot be read `Unavailable` · `library-unavailable` `{mount, message}`
//! (ADR-0400 decision 6); a stale root generation `Conflict` · `root-generation-stale`; a dot or temp name below the
//! mount's top `InvalidArgument` · `item-path-invalid`; a name that is not `.yaml` / `.yml` `InvalidArgument` ·
//! `wrong-kind`; a path in no registered mount `NotFound` · `not-found`; a link at or above the file `InvalidArgument`
//! · `path-through-link`; the file or a parent missing `NotFound` · `not-found` (nothing is re-created); another read
//! error or a link probe error `Unavailable` · `library-read-failed`; a file that does not read as a preset, or one the
//! writer cannot rewrite exactly, `InvalidArgument` with the reader's or writer's code (`WriteError::to_reason`), the
//! file untouched; a failed write `Unavailable` · [`LIBRARY_WRITE_FAILED`], the file as it was.
// ADR-0324
// ADR-0338
// ADR-0341
// ADR-0358
// ADR-0365
// ADR-0366
// ADR-0378
// ADR-0400

use std::collections::{HashMap, HashSet};
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use engram_dashboard_agent::backend::settings_check::FolderLoadSet;
use engram_dashboard_agent::backend::SettingsCheck;
use engram_dashboard_agent::launch::{escape_controls, BlockReason, Reason};
use engram_dashboard_agent::profile::AgentOutputFormat;
use engram_dashboard_platform::fs::{normalize_folder, path_key, PathKey};
use engram_dashboard_platform::shell::path_is_cli_safe;
use serde_json::{json, Value};

use crate::catalog::{
    key, preset_refs, yaml_file, yaml_item, Catalog, Item, ItemContent, ItemKind,
};
use crate::folder::{FolderStatus, LaunchIo, Need};
use crate::format::read::{
    BACKEND_TAG_NOT_ALLOWED, BACKEND_TAG_UNKNOWN, FOLDER_PATH_RELATIVE, ITEM_PATH_INVALID,
    WRONG_KIND,
};
use crate::format::{
    parse_preset, rewrite_lists, FolderItem, ListName, ParseError, PresetDoc, PresetLists, Sniff,
    YamlFrontMatter, YAML_SYNTAX,
};
use crate::gate::{State, NOT_FOUND};
use crate::launch::{sha256_hex, FolderReads, LIBRARY_READ_FAILED};
use crate::mounts::{admits_inside, Mounts};
use crate::path::{AssetRef, BuiltinId, FolderRef, LibPath, PresetItem};
use crate::refs::{default_slot_count, referrers_of};
use crate::resolve::{
    ref_key, resolve, CatalogLookup, ItemView, Looked, Lookup, Problem, RefKey, Resolution,
    RootState, Target, BUILTIN_IN_LIBRARY_PRESET, FOLDER_NOT_FOUND, FOLDER_PATH_CLI_UNSAFE,
    FOLDER_UNREADABLE, PRESET_CYCLE,
};
use crate::store::ContentTag;
use crate::{
    link_verdict, refuse_unusable_now, virtual_root_read_only, LibraryRefusal, LibraryService,
    RefusalClass, Want, LIBRARY_READ_CAP, PATH_THROUGH_LINK, STAT_UNANSWERED,
};

/// The code of a `baseRev` that is not the `rev` of the bytes read inside the change (`args` = `{rev}`, the current
/// one).
// ADR-0378
pub const REV_MISMATCH: &str = "rev-mismatch";

/// The code of an edit whose file write failed; the file is as it was (`args` = `{path, why}`).
pub const LIBRARY_WRITE_FAILED: &str = "library-write-failed";

/// The code of a save refused whole for its mismatches (`args` = `{items: [{list, path, code}]}`).
// ADR-0338
pub const PRESET_SAVE_REFUSED: &str = "preset-save-refused";

/// The mismatch of one target listed twice in one list, whatever its tags, or of two folders equal after
/// normalization.
// ADR-0339
pub const ITEM_DUPLICATE: &str = "item-duplicate";

/// The code of a folder `add` refused whole (`args` = `{folders: [{path, code, why?}]}` — each bad path as given).
// ADR-0341
pub const PRESET_FOLDERS_REFUSED: &str = "preset-folders-refused";

/// The skip reason of an item or folder already in the list (`args` = `{path}`).
pub const ALREADY_PRESENT: &str = "already-present";

/// The code of a folder preview whose bounded listing gave no answer for the folder it was asked — a broken invariant
/// of `BoundedIo::list_all` (it answers every path it is asked), answered as an error rather than a panic (`args` =
/// `{path}`).
// ADR-0293
pub const FOLDER_LISTING_UNANSWERED: &str = "folder-listing-unanswered";

/// The code of a `reorder` whose order is not a permutation of the section as `preset.get` shows it (`args` =
/// `{expected}` — that section).
pub const REORDER_ORDER_MISMATCH: &str = "reorder-order-mismatch";

/// The four item lists, in file order.
pub(crate) const ITEM_LISTS: [ListName; 4] = [
    ListName::Presets,
    ListName::Instructions,
    ListName::Skills,
    ListName::Settings,
];

/// One item an edit names: a reference spelling (a library path or `engram:/…`) and its backend tag (`None` =
/// common).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewItem {
    pub path: String,
    pub backend: Option<String>,
}

/// What an `add` adds — items or folders, never both in one call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AddEntries {
    /// Each goes into the list of its kind in the catalog.
    Items(Vec<NewItem>),
    /// Absolute folder paths, as given.
    Folders(Vec<String>),
}

/// What a `remove` removes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RemoveEntries {
    /// Reference spellings, matched by catalog key (built-ins by id) in every list.
    Items(Vec<String>),
    /// Folder paths, matched by the key of the normalized path (a relative entry by its spelling).
    Folders(Vec<String>),
}

/// The lists a `save` writes or a `check` judges.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PresetDraft {
    pub presets: Vec<NewItem>,
    pub instructions: Vec<NewItem>,
    pub skills: Vec<NewItem>,
    pub settings: Vec<NewItem>,
    /// `None` = the folder list is left as it is.
    pub folders: Option<Vec<String>>,
}

/// The `preset.get` view.
#[derive(Clone, Debug, PartialEq)]
pub struct PresetView {
    /// The preset as asked.
    pub path: String,
    /// The `rev` of the catalog item (ADR-0378).
    pub rev: String,
    pub description: Option<String>,
    /// Every modeled entry of each list, in file order — a duplicate included.
    pub presets: Vec<PresetRow>,
    pub instructions: Vec<PresetRow>,
    pub skills: Vec<PresetRow>,
    pub settings: Vec<PresetRow>,
    /// The preset's own folder entries, normalized and deduplicated (first place kept).
    pub folders: Vec<PresetFolder>,
    /// The conflict rows of resolving the preset on its own.
    pub conflicts: Vec<Problem>,
    /// Every other problem row, inner presets' included (each with its `via`).
    pub problems: Vec<Problem>,
    /// The agents that attach it (`engram:/default`: the agents whose default slot is on).
    pub agents: usize,
}

/// One list entry of a [`PresetView`].
#[derive(Clone, Debug, PartialEq)]
pub struct PresetRow {
    pub path: String,
    pub backend: Option<String>,
    /// Its line in the file.
    pub line: u32,
    /// The entry's own state: `not-found`, [`PATH_THROUGH_LINK`], the item's broken reason or a settings asset's
    /// first refusal, `wrong-kind`, `builtin-in-library-preset`, `preset-cycle` (it lists itself); `None` = it reads.
    /// What an inner preset reaches is in the view's problem rows.
    pub state: Option<Reason>,
}

/// One folder entry of a [`PresetView`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PresetFolder {
    /// The normalized path; a relative entry (a line problem) as the file spells it.
    pub path: String,
    /// Its line in the file; `None` = a folder of a `check` draft.
    pub line: Option<u32>,
    /// The bounded stat's answer; `None` = not stat'ed (a relative entry, or the resolution was block ④).
    pub status: Option<FolderStatus>,
}

/// A partial edit's answer (`add` items · `setBackend`): what was applied, what was skipped and why, and the view
/// after the edit. A folder `add` lists the normalized paths.
#[derive(Clone, Debug, PartialEq)]
pub struct PartialEdit {
    pub applied: Vec<String>,
    pub skipped: Vec<Skipped>,
    pub view: PresetView,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Skipped {
    /// As given.
    pub item: String,
    pub reason: Reason,
}

/// One mismatch of a draft.
#[derive(Clone, Debug, PartialEq, Eq)]
struct SaveMismatch {
    list: ListName,
    /// As given.
    path: String,
    code: &'static str,
}

/// The answer of [`LibraryService::preset_check`] — the `preset.get` view's folder, conflict and problem shape for
/// what a `save` of the draft would write; the draft's mismatches are problem rows.
#[derive(Clone, Debug, PartialEq)]
pub struct CheckAnswer {
    /// A `save` of the draft would refuse for its mismatches — at least one problem row is a draft mismatch (a save can
    /// still refuse for reasons outside the draft: `baseRev`, the writer, a file gone).
    pub save_refuses: bool,
    pub folders: Vec<PresetFolder>,
    pub conflicts: Vec<Problem>,
    pub problems: Vec<Problem>,
}

/// Why a folder `add` would refuse one path. The codes match the launch's folder checks, but not its rows or `args`:
/// at launch `folder-path-relative` is a line problem and `folder-path-cli-unsafe` is row ②.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FolderRefusal {
    /// As given.
    pub path: String,
    /// `folder-path-relative` · [`FOLDER_PATH_CLI_UNSAFE`] · [`FOLDER_NOT_FOUND`] · [`FOLDER_UNREADABLE`].
    pub code: &'static str,
    /// For `folder-not-found`: missing · not a folder. For `folder-unreadable`: the OS error · a timeout · a share
    /// not reachable.
    pub why: Option<String>,
}

/// The answer of [`LibraryService::preview_folder`].
#[derive(Clone, Debug, PartialEq)]
pub struct FolderPreview {
    /// The normalized path; `None` = not absolute.
    pub path: Option<PathBuf>,
    /// The `add` verdict; `None` = an `add` accepts the path.
    pub refused: Option<FolderRefusal>,
    /// Per backend and mode asked, what it loads from the folder — empty unless the folder stat'ed `Ok` and listed.
    pub loads: Vec<PreviewLoad>,
    /// The listing stopped at its cap.
    pub truncated: bool,
    /// Entries below the folder the listing could not read.
    pub skipped: Vec<String>,
    /// Why a folder that stat'ed `Ok` could not be listed.
    pub unlisted: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PreviewLoad {
    pub backend: &'static str,
    pub mode: AgentOutputFormat,
    /// The backend reads the folder itself at launch: the set is the port's prediction.
    pub predicted: bool,
    pub set: FolderLoadSet,
    /// A file read that got no answer (a timeout or the budget spent) — the set may lack what that file decides.
    pub unread: Option<String>,
}

/// A preset standing in for the catalog item at its path ([`LibraryService::resolve_over`] — `preset.check`).
pub(crate) struct Draft {
    at: PathKey,
    kind: ItemKind,
    content: ItemContent,
}

impl Draft {
    pub(crate) fn over<'a>(&'a self, inner: &'a CatalogLookup<'a>) -> DraftLookup<'a> {
        DraftLookup { draft: self, inner }
    }
}

pub(crate) struct DraftLookup<'a> {
    draft: &'a Draft,
    inner: &'a CatalogLookup<'a>,
}

impl Lookup for DraftLookup<'_> {
    fn item(&self, r: &AssetRef) -> Looked<'_> {
        match r {
            AssetRef::Library(path) if key(path) == self.draft.at => Looked::Present(ItemView {
                kind: &self.draft.kind,
                content: &self.draft.content,
                file: self.inner.mounts.disk_path(path),
                stand_in: None,
            }),
            _ => self.inner.item(r),
        }
    }
}

/// What an edit's operation sees inside the change: the file as just read and the catalog with it applied.
pub(crate) struct Edit<'a> {
    pub(crate) doc: &'a PresetDoc,
    pub(crate) catalog: &'a Catalog,
    pub(crate) mounts: &'a Mounts,
}

/// What [`LibraryService::rewrite_in`] committed: `op`'s answer, the preset as committed (the file as read when its
/// lists came out unchanged) with its `rev`, and whether the file was written.
pub(crate) struct Rewritten<T> {
    pub(crate) answer: T,
    pub(crate) doc: PresetDoc,
    pub(crate) rev: String,
    pub(crate) written: bool,
}

/// What a view takes from the catalog under a library lock — the read lock of `preset.get`, or the write lock of the
/// edit whose answer it is.
struct ViewBase {
    view: PresetView,
    /// The keys of the own folder rows, by row (`None` = a relative entry).
    folder_keys: Vec<Option<PathKey>>,
    /// The mounts the rows' link probes run in.
    mounts: Mounts,
}

/// An edit's answer view as built inside its change: the catalog part and the pure resolution of the preset as
/// committed. The folder stats and link probes are still to run, outside every lock ([`LibraryService::settle`]).
struct PendingView {
    base: ViewBase,
    resolution: Resolution,
}

impl LibraryService {
    /// The `preset.get` view of `preset` from the catalog (no disk read of the preset): its rows with their states, its
    /// folder entries with their stats, the conflict and problem rows of resolving it on its own, its `rev` and how
    /// many agents attach it. A library preset passes the first-scan entrance; so does `engram:/default` when its file
    /// lists a library item, though it needs no mount.
    ///
    /// Refusals: a library preset with no first scan applied `Unavailable` · `library-unavailable`; one whose mount
    /// cannot be read `Unavailable` · `library-unavailable` `{mount, message}` (ADR-0400 decision 6); a path the catalog
    /// does not hold `NotFound` · `not-found`; a file that does not read `InvalidArgument` with its reason; an item
    /// that is not a preset `InvalidArgument` · `wrong-kind`.
    pub fn preset_get(&self, preset: &AssetRef) -> Result<PresetView, LibraryRefusal> {
        let library = match preset {
            AssetRef::Library(path) => {
                self.first_scan_join()
                    .map_err(LibraryRefusal::unavailable)?;
                // ADR-0400 decision 6: an item of a mount that cannot be read is not answered from a stale catalog.
                self.refuse_unusable_mount(path)?;
                true
            }
            AssetRef::BuiltIn(id) => match self.builtins.view(*id).content {
                ItemContent::Preset { doc, .. } => preset_refs(doc).next().is_some(),
                _ => false,
            },
        };
        let want = Want {
            backend: None,
            mode: None,
            default_preset: false,
            presets: vec![preset.clone()],
            unparsed: Vec::new(),
        };
        let (mut resolved, base) = self.resolve_over(&[&want], library, None, |state, lookup| {
            self.view_base(state, lookup, preset)
        });
        Ok(self.complete_view(base?, resolved.remove(0)))
    }

    /// The view's catalog part for the item `lookup` finds at `preset`.
    fn view_base(
        &self,
        state: &State,
        lookup: &dyn Lookup,
        preset: &AssetRef,
    ) -> Result<ViewBase, LibraryRefusal> {
        let spelling = preset.spelling();
        let Looked::Present(item) = lookup.item(preset) else {
            return Err(not_found(&spelling));
        };
        match item.content {
            ItemContent::Preset { doc, rev } => {
                Ok(self.view_parts(&state.catalog, &state.mounts, lookup, preset, doc, rev))
            }
            ItemContent::Broken { reason } => Err(invalid(reason.clone())),
            _ => Err(LibraryRefusal::new(
                RefusalClass::InvalidArgument,
                WRONG_KIND,
                json!({ "path": spelling }),
            )),
        }
    }

    /// The view's catalog part for preset `preset` read as `doc` with `rev`.
    fn view_parts(
        &self,
        catalog: &Catalog,
        mounts: &Mounts,
        lookup: &dyn Lookup,
        preset: &AssetRef,
        doc: &PresetDoc,
        rev: &str,
    ) -> ViewBase {
        let rows = |list: ListName| -> Vec<PresetRow> {
            doc_list(doc, list)
                .iter()
                .map(|item| PresetRow {
                    path: item.target.spelling(),
                    backend: item.backend.clone(),
                    line: item.line,
                    state: row_state(lookup, list, item, preset),
                })
                .collect()
        };
        let (folders, folder_keys) = own_folders(doc);
        let registry = &self.values.registry;
        let agents = match preset {
            AssetRef::Library(path) => referrers_of(catalog, registry, path).1.len(),
            AssetRef::BuiltIn(BuiltinId::DefaultPreset) => default_slot_count(registry),
            AssetRef::BuiltIn(_) => 0,
        };
        ViewBase {
            view: PresetView {
                path: preset.spelling(),
                rev: rev.to_owned(),
                description: doc.description.clone(),
                presets: rows(ListName::Presets),
                instructions: rows(ListName::Instructions),
                skills: rows(ListName::Skills),
                settings: rows(ListName::Settings),
                folders,
                conflicts: Vec::new(),
                problems: Vec::new(),
                agents,
            },
            folder_keys,
            mounts: mounts.clone(),
        }
    }

    /// An edit's answer view, built inside its change from `catalog` as the edit left it (the preset as committed:
    /// `doc` with `rev`) — the resolution is pure, with the library usable (the edit just read the file in it).
    fn pending_view(
        &self,
        catalog: &Catalog,
        mounts: &Mounts,
        preset: &LibPath,
        doc: &PresetDoc,
        rev: &str,
    ) -> PendingView {
        let lookup = CatalogLookup {
            catalog,
            mounts,
            builtins: &self.builtins,
        };
        let at = AssetRef::Library(preset.clone());
        let target = Target {
            backend: None,
            mode: None,
            default_preset: false,
            presets: vec![at.clone()],
            unparsed: Vec::new(),
            root: RootState::Ok,
        };
        PendingView {
            base: self.view_parts(catalog, mounts, &lookup, &at, doc, rev),
            resolution: resolve(&target, &lookup, &*self.values.backends),
        }
    }

    /// Finishes an edit's answer view outside every lock: the folder stats of its resolution on one read budget and
    /// the link probes of its `not-found` rows.
    fn settle(&self, pending: PendingView) -> PresetView {
        let PendingView {
            base,
            mut resolution,
        } = pending;
        self.settle_resolution(&mut resolution, &base.mounts);
        self.complete_view(base, resolution)
    }

    /// The view from its catalog part and its resolution (folders stat'ed, links marked): each own folder takes its
    /// status, each `not-found` row its link probe, and the problems split into conflicts and the rest.
    fn complete_view(&self, base: ViewBase, resolution: Resolution) -> PresetView {
        let ViewBase {
            mut view,
            folder_keys,
            mounts,
        } = base;
        attach_statuses(&mut view.folders, &folder_keys, &resolution);
        for row in rows_mut(&mut view) {
            self.mark_row_link(&mounts, row);
        }
        (view.conflicts, view.problems) = resolution
            .problems
            .into_iter()
            .partition(|problem| matches!(problem, BlockReason::Conflict { .. }));
        view
    }

    /// A `not-found` library row's link probe, judged by [`link_verdict`] — the rule of `LibraryService::mark_links`, so
    /// a line's state and the answer's problems agree. A store call — run it outside every lock.
    // ADR-0387
    fn mark_row_link(&self, mounts: &Mounts, row: &mut PresetRow) {
        if row
            .state
            .as_ref()
            .is_none_or(|state| state.code != NOT_FOUND)
        {
            return;
        }
        let Ok(AssetRef::Library(path)) = AssetRef::parse(&row.path) else {
            return;
        };
        let Some(probe) = self.first_link(mounts, &path) else {
            return;
        };
        if let Some(verdict) = link_verdict(probe, &row.path, Some(row.line)) {
            row.state = Some(verdict);
        }
    }

    /// Adds items or folders to `preset` (partial apply): an item goes into the list of its kind in the catalog,
    /// right after the `after` anchor of that list in call order (no anchor in that list = at its end); a folder is
    /// stored normalized, after the `after` folder (any spelling). Items are skipped with a reason — not a reference
    /// (`item-path-invalid`), not in the catalog (`not-found`), in no list of its kind (`wrong-kind`), an unknown or
    /// unallowed tag, a built-in in `presets`,
    /// a preset that would close a cycle, or already listed ([`ALREADY_PRESENT`], by key regardless of tag). Folders
    /// are stat'ed before the change; inside it a folder already listed (by normalized key) is skipped, and then any
    /// other relative, cmd-unsafe, missing, not-a-folder, unreadable or timed-out path refuses the whole call
    /// `InvalidArgument` · [`PRESET_FOLDERS_REFUSED`] naming each bad one. Plus the refusals every edit can answer
    /// (module header).
    pub fn preset_add(
        &self,
        preset: &LibPath,
        entries: &AddEntries,
        after: Option<&str>,
        root_generation: Option<u64>,
    ) -> Result<PartialEdit, LibraryRefusal> {
        let ((applied, skipped), pending) = match entries {
            AddEntries::Items(items) => self.edit(preset, root_generation, None, |edit| {
                Ok(self.add_items(edit, preset, items, after))
            })?,
            AddEntries::Folders(folders) => {
                self.refuse_unadmitted_target(preset)?;
                self.first_scan_join()
                    .map_err(LibraryRefusal::unavailable)?;
                // ADR-0341: the stats run here, outside every lock; the verdicts are judged inside the change.
                let verdicts = self.folder_verdicts(&self.io.begin_read(), folders);
                self.edit(preset, root_generation, None, |edit| {
                    add_folders(edit.doc, folders, &verdicts, after)
                })?
            }
        };
        Ok(PartialEdit {
            applied,
            skipped,
            view: self.settle(pending),
        })
    }

    /// Removes items (every modeled entry with the same key, in every list — an entry the reader could not model stays)
    /// or folders (every spelling of the folder) from
    /// `preset`. Idempotent and not edition-checked: removing what is not there writes nothing. A spelling that is not
    /// a reference refuses `InvalidArgument` · `item-path-invalid` before the change. Plus the refusals every edit can
    /// answer (module header).
    pub fn preset_remove(
        &self,
        preset: &LibPath,
        entries: &RemoveEntries,
        root_generation: Option<u64>,
    ) -> Result<PresetView, LibraryRefusal> {
        let ((), pending) = match entries {
            RemoveEntries::Items(items) => {
                let keys: HashSet<RefKey> = items
                    .iter()
                    .map(|item| parse_ref(item).map(|target| ref_key(&target)))
                    .collect::<Result<_, _>>()
                    .map_err(invalid)?;
                self.edit(preset, root_generation, None, |edit| {
                    let mut lists = as_read(edit.doc);
                    for list in ITEM_LISTS {
                        list_mut(&mut lists, list)
                            .retain(|item| !keys.contains(&ref_key(&item.target)));
                    }
                    Ok((lists, ()))
                })?
            }
            RemoveEntries::Folders(folders) => {
                let ids: HashSet<FolderId> = folders.iter().map(|raw| folder_id(raw).1).collect();
                self.edit(preset, root_generation, None, |edit| {
                    let mut lists = as_read(edit.doc);
                    lists
                        .folders
                        .retain(|entry| !ids.contains(&folder_id(&entry.path).1));
                    Ok((lists, ()))
                })?
            }
        };
        Ok(self.settle(pending))
    }

    /// Reorders one section of `preset` to `order`, which must be a permutation of the section as `preset.get` shows it
    /// (items by key, a duplicate counted as often as listed; folders normalized and deduplicated) — else
    /// `InvalidArgument` · [`REORDER_ORDER_MISMATCH`]. A `base_rev` other than the `rev` of the bytes read inside the
    /// change refuses `Conflict` · [`REV_MISMATCH`]. The folders section is written normalized and deduplicated. Plus
    /// the refusals every edit can answer (module header).
    pub fn preset_reorder(
        &self,
        preset: &LibPath,
        section: ListName,
        order: &[String],
        base_rev: &str,
        root_generation: Option<u64>,
    ) -> Result<PresetView, LibraryRefusal> {
        let ((), pending) = self.edit(preset, root_generation, Some(base_rev), |edit| {
            let mut lists = as_read(edit.doc);
            match section {
                ListName::Folders => lists.folders = reorder_folders(edit.doc, order)?,
                list => {
                    let current = list_mut(&mut lists, list);
                    *current = reorder_items(current, order)?;
                }
            }
            Ok((lists, ()))
        })?;
        Ok(self.settle(pending))
    }

    /// Sets the backend tag of each item in `preset` (`None` = common), every entry with the item's key in
    /// `instructions` and `skills` (partial apply): an item the preset does not list is skipped `not-found`, one only
    /// in `presets` / `settings` `backend-tag-not-allowed`, a spelling that is not a reference `item-path-invalid`. A
    /// `backend` word the registry does not know refuses the whole call `InvalidArgument` · `backend-tag-unknown` before
    /// the change. A tag that is already set writes nothing. Plus the refusals every edit can answer (module
    /// header).
    pub fn preset_set_backend(
        &self,
        preset: &LibPath,
        items: &[String],
        backend: Option<&str>,
        root_generation: Option<u64>,
    ) -> Result<PartialEdit, LibraryRefusal> {
        let word = match backend {
            None => None,
            Some(word) => Some(
                self.values
                    .backends
                    .for_word(word)
                    .map(SettingsCheck::backend)
                    .ok_or_else(|| {
                        LibraryRefusal::new(
                            RefusalClass::InvalidArgument,
                            BACKEND_TAG_UNKNOWN,
                            json!({ "backend": escape_controls(word) }),
                        )
                    })?,
            ),
        };
        let ((applied, skipped), pending) = self.edit(preset, root_generation, None, |edit| {
            Ok(set_backend(edit.doc, items, word))
        })?;
        Ok(PartialEdit {
            applied,
            skipped,
            view: self.settle(pending),
        })
    }

    /// Replaces the four lists of `preset` (and its folders, when the draft carries them) with `draft`, all or nothing:
    /// any mismatch refuses the whole save `InvalidArgument` · [`PRESET_SAVE_REFUSED`] listing every one — not a
    /// reference, an unknown or unallowed tag, a built-in in `presets`, one target twice in a list ([`ITEM_DUPLICATE`]),
    /// an item of another kind, a preset that would close a cycle, a relative folder, two folders equal after
    /// normalization. A missing item or folder and a cmd-unsafe folder are legal. Each list the save supplies is
    /// replaced whole — the entries the reader could not model go too; an omitted `folders` stays as it is. A kept
    /// item keeps its entry (its other keys survive a respelling); folders are stored normalized; keys outside the
    /// lists are never touched. A `base_rev` other than the `rev` of the bytes read inside the change refuses
    /// `Conflict` · [`REV_MISMATCH`]. Plus the refusals every edit can answer (module header).
    pub fn preset_save(
        &self,
        preset: &LibPath,
        draft: &PresetDraft,
        base_rev: Option<&str>,
        root_generation: Option<u64>,
    ) -> Result<PresetView, LibraryRefusal> {
        let ((), pending) = self.edit(preset, root_generation, base_rev, |edit| {
            let lookup = CatalogLookup {
                catalog: edit.catalog,
                mounts: edit.mounts,
                builtins: &self.builtins,
            };
            let (lists, refused) = self.draft_lists(draft, Some(edit.doc), preset, &lookup);
            if !refused.is_empty() {
                let items: Vec<Value> = refused.iter().map(mismatch_json).collect();
                return Err(LibraryRefusal::new(
                    RefusalClass::InvalidArgument,
                    PRESET_SAVE_REFUSED,
                    json!({ "items": items }),
                ));
            }
            Ok((lists, ()))
        })?;
        Ok(self.settle(pending))
    }

    /// Judges `draft` for `preset` as `save` would and resolves what `save` would write in the preset's place (the
    /// catalog overlaid with it, refused items left out, the line problems of the file entries it keeps) — no write; file,
    /// catalog and `rev` unchanged. Answers the folder list with its stats, the conflict rows, and the problem rows: the
    /// resolution's, then one `Broken` row per mismatch (`args` = `{path, list}`). A draft item has no file line, so its
    /// rows carry none. A draft listing a library item passes the first-scan entrance (its failure `Unavailable` ·
    /// `library-unavailable`); a built-in-only draft answers whatever the library is, after waiting out a pending first
    /// scan when it keeps the file's folders. A preset whose mount cannot be read refuses `Unavailable` ·
    /// `library-unavailable` `{mount, message}` (walked again first), as `preset.get` does — its current lists would come
    /// from a stale catalog (ADR-0400 decision 6).
    pub fn preset_check(
        &self,
        preset: &LibPath,
        draft: &PresetDraft,
    ) -> Result<CheckAnswer, LibraryRefusal> {
        let library = [
            &draft.presets,
            &draft.instructions,
            &draft.skills,
            &draft.settings,
        ]
        .into_iter()
        .flatten()
        .any(|item| matches!(AssetRef::parse(&item.path), Ok(AssetRef::Library(_))));
        if library {
            self.first_scan_join()
                .map_err(LibraryRefusal::unavailable)?;
        } else if draft.folders.is_none() {
            // The file's folder list is read from the catalog: wait out a pending first scan. Its failure is not this
            //   draft's answer — a built-in-only draft answers without the library (the list is then empty).
            if let Err(why) = self.first_scan_join() {
                tracing::debug!("preset check: first scan not applied; folders unknown: {why}");
            }
        }
        // ADR-0400 decision 6
        self.refuse_unusable_mount(preset)?;
        let (overlay, refused, mut folders, folder_keys) = self.gate.with_read(|state| {
            let lookup = CatalogLookup {
                catalog: &state.catalog,
                mounts: &state.mounts,
                builtins: &self.builtins,
            };
            let current = match state.catalog.get(&key(preset)).map(|item| &item.content) {
                Some(ItemContent::Preset { doc, .. }) => Some(doc),
                _ => None,
            };
            let (lists, refused) = self.draft_lists(draft, current, preset, &lookup);
            // ADR-0358: the save writes a kept entry from its original node, so what the reader found wrong in that node
            //   stays (a tag in `presets` / `settings` is kept as written) — the check shows the same line problems: those
            //   of each kept entry, and the whole folder list's when the draft leaves it as it is.
            let kept: HashSet<(&str, u64)> = ITEM_LISTS
                .into_iter()
                .flat_map(|list| {
                    lists_ref(&lists, list).iter().filter_map(move |item| {
                        item.origin.map(|origin| (list.key(), origin.index))
                    })
                })
                .chain(lists.folders.iter().filter_map(|folder| {
                    folder
                        .origin
                        .map(|origin| (ListName::Folders.key(), origin.index))
                }))
                .map(|(list, index)| (list, u64::from(index)))
                .collect();
            let line_problems: Vec<Reason> = current
                .map(|doc| {
                    doc.line_problems
                        .iter()
                        .filter(|problem| {
                            let list = problem.args["list"].as_str().unwrap_or_default();
                            let whole = list == ListName::Folders.key() && draft.folders.is_none();
                            whole
                                || problem.args["index"]
                                    .as_u64()
                                    .is_some_and(|index| kept.contains(&(list, index)))
                        })
                        .cloned()
                        .collect()
                })
                .unwrap_or_default();
            let folders = match (&draft.folders, current) {
                (None, Some(doc)) => doc.folders.clone(),
                (None, None) => Vec::new(),
                (Some(_), _) => lists
                    .folders
                    .iter()
                    .map(|folder| FolderRef::new(folder.path.clone(), 0, folder.origin))
                    .collect(),
            };
            let doc = PresetDoc {
                version: current.map_or(1, |doc| doc.version),
                description: current.and_then(|doc| doc.description.clone()),
                presets: lists.presets,
                instructions: lists.instructions,
                skills: lists.skills,
                settings: lists.settings,
                folders,
                unmodeled: Vec::new(),
                line_problems,
            };
            let (folders, folder_keys) = own_folders(&doc);
            // No bytes stand behind a draft: its `rev` is never answered.
            let content = ItemContent::Preset {
                doc,
                rev: String::new(),
            };
            let overlay = Draft {
                at: key(preset),
                kind: ItemKind::Preset,
                content,
            };
            (overlay, refused, folders, folder_keys)
        });
        let want = Want {
            backend: None,
            mode: None,
            default_preset: false,
            presets: vec![AssetRef::Library(preset.clone())],
            unparsed: Vec::new(),
        };
        let (mut resolved, ()) = self.resolve_over(&[&want], library, Some(&overlay), |_, _| ());
        let resolution = resolved.remove(0);
        attach_statuses(&mut folders, &folder_keys, &resolution);
        let mut problems = resolution.problems;
        let via = vec![preset.as_str().to_owned()];
        problems.extend(refused.iter().map(|mismatch| BlockReason::Broken {
            item: mismatch.path.clone(),
            via: via.clone(),
            reason: Reason {
                code: mismatch.code.to_owned(),
                args: json!({ "path": mismatch.path, "list": mismatch.list.key() }),
                line: None,
            },
        }));
        // A draft item's line is 0 (it has none); a file line is 1-based.
        for problem in &mut problems {
            let reason = match problem {
                BlockReason::Missing { reason, .. }
                | BlockReason::Broken { reason, .. }
                | BlockReason::Conflict { reason, .. }
                | BlockReason::LibraryUnavailable { reason } => reason,
            };
            if reason.line == Some(0) {
                reason.line = None;
            }
        }
        let (conflicts, problems) = problems
            .into_iter()
            .partition(|problem| matches!(problem, BlockReason::Conflict { .. }));
        Ok(CheckAnswer {
            save_refuses: !refused.is_empty(),
            folders,
            conflicts,
            problems,
        })
    }

    /// The folder preview: `folder` normalized, the verdict a folder `add` would give it (the same checks and codes),
    /// and — when it stat'ed `Ok` — per backend (`backend`, or every registered one) and mode (`mode`, or both), what
    /// the backend loads from it ([`SettingsCheck::folder_load_set`] over one bounded listing and bounded reads). Read
    /// only, on one read budget, no library lock and no catalog. A `backend` word the registry does not know refuses
    /// `InvalidArgument` · `backend-tag-unknown`; a listing with no answer for the folder `Internal` ·
    /// [`FOLDER_LISTING_UNANSWERED`].
    // ADR-0341
    pub fn preview_folder(
        &self,
        folder: &str,
        backend: Option<&str>,
        mode: Option<AgentOutputFormat>,
    ) -> Result<FolderPreview, LibraryRefusal> {
        let backends = &*self.values.backends;
        let checks: Vec<&dyn SettingsCheck> = match backend {
            None => backends.all(),
            Some(word) => vec![backends.for_word(word).ok_or_else(|| {
                LibraryRefusal::new(
                    RefusalClass::InvalidArgument,
                    BACKEND_TAG_UNKNOWN,
                    json!({ "backend": escape_controls(word) }),
                )
            })?],
        };
        let modes = match mode {
            Some(mode) => vec![mode],
            None => vec![AgentOutputFormat::Terminal, AgentOutputFormat::StreamJson],
        };
        let io = self.io.begin_read();
        let verdict = self
            .folder_verdicts(&io, std::slice::from_ref(&folder.to_owned()))
            .remove(0);
        let mut preview = FolderPreview {
            path: verdict.normalized.clone(),
            refused: verdict.judged.err(),
            loads: Vec::new(),
            truncated: false,
            skipped: Vec::new(),
            unlisted: None,
        };
        let (Some(path), Some(FolderStatus::Ok)) = (verdict.normalized, verdict.status) else {
            return Ok(preview);
        };
        let Some(listing) = self
            .io
            .list_all(&io, &[(path.clone(), Need::Required)])
            .remove(&path_key(&path))
        else {
            return Err(LibraryRefusal::new(
                RefusalClass::Internal,
                FOLDER_LISTING_UNANSWERED,
                json!({ "path": path.to_string_lossy() }),
            ));
        };
        let listing = match listing {
            Ok(listing) => listing,
            Err(status) => {
                preview.unlisted = Some(status.why());
                return Ok(preview);
            }
        };
        for check in checks {
            for &mode in &modes {
                let reads = FolderReads::new(self, &io, &path, Need::Required);
                let set = check.folder_load_set(
                    &listing.entries,
                    &|rel| reads.read(rel),
                    &YamlFrontMatter,
                    mode,
                );
                preview.loads.push(PreviewLoad {
                    backend: check.backend(),
                    mode,
                    predicted: !check.folder_listing_at_launch(mode),
                    set,
                    unread: reads.unanswered.into_inner(),
                });
            }
        }
        preview.truncated = listing.truncated;
        preview.skipped = listing.skipped;
        Ok(preview)
    }

    /// One edit of library preset `preset` (module header): the first-scan entrance, then `op` inside one
    /// [`LibraryService::change_at`] over the file as just read; `op` answers the new lists and its own answer. The
    /// answer view of the preset as committed is built inside the same change. Refusals: those every edit can answer
    /// (module header), `base_rev` other than the `rev` read `Conflict` · [`REV_MISMATCH`], and `op`'s own.
    fn edit<T>(
        &self,
        preset: &LibPath,
        root_generation: Option<u64>,
        base_rev: Option<&str>,
        op: impl FnOnce(&Edit<'_>) -> Result<(PresetLists, T), LibraryRefusal>,
    ) -> Result<(T, PendingView), LibraryRefusal> {
        self.refuse_unadmitted_target(preset)?;
        self.first_scan_join()
            .map_err(LibraryRefusal::unavailable)?;
        // ADR-0400 decision 6: no write into a mount that cannot be read — its disk is not touched under the write lock.
        self.refuse_unusable_mount(preset)?;
        self.change_at(root_generation, |catalog, mounts, seq| {
            // ADR-0400 decision 6: judged again from the recorded state, before any disk IO.
            if let Err(refusal) = refuse_unusable_now(mounts, preset) {
                return (Err(refusal), Vec::new());
            }
            let mut touched = Vec::new();
            let answer = self.edit_in(preset, base_rev, catalog, mounts, seq, op, &mut touched);
            (answer, touched)
        })?
    }

    /// The checks of an edit target by name alone, before any mount lookup: a path of one name
    /// `virtual-root-read-only`, a dot or temp name below the mount's top `item-path-invalid` (as
    /// [`LibraryService::admit`] refuses them), then a name that is not `.yaml` / `.yml` — the only files that hold
    /// presets — `wrong-kind`.
    // ADR-0400 decision 5
    fn refuse_unadmitted_target(&self, preset: &LibPath) -> Result<(), LibraryRefusal> {
        if preset.parent().is_none() {
            return Err(virtual_root_read_only(Some(preset)));
        }
        if !admits_inside(preset) {
            return Err(dot_or_temp(preset));
        }
        match yaml_file(preset) {
            true => Ok(()),
            false => Err(LibraryRefusal::new(
                RefusalClass::InvalidArgument,
                WRONG_KIND,
                json!({ "path": preset.as_str() }),
            )),
        }
    }

    /// The steps of [`LibraryService::edit`] inside the change; `touched` gathers every key applied.
    #[allow(clippy::too_many_arguments)]
    fn edit_in<T>(
        &self,
        preset: &LibPath,
        base_rev: Option<&str>,
        catalog: &mut Catalog,
        mounts: &Mounts,
        seq: &dyn Fn() -> u64,
        op: impl FnOnce(&Edit<'_>) -> Result<(PresetLists, T), LibraryRefusal>,
        touched: &mut Vec<PathKey>,
    ) -> Result<(T, PendingView), LibraryRefusal> {
        let done = self.rewrite_in(preset, base_rev, catalog, mounts, seq, op, touched)?;
        let pending = self.pending_view(catalog, mounts, preset, &done.doc, &done.rev);
        Ok((done.answer, pending))
    }

    /// One preset file's rewrite inside a change (module header, from the link admission to the per-item apply), with
    /// no answer view — what an edit and each file of a relink run. `touched` gathers every key applied, also when it
    /// refuses. Refusals: those every edit can answer (module header) but the first two, `base_rev` other than the
    /// `rev` read `Conflict` · [`REV_MISMATCH`], and `op`'s own.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn rewrite_in<T>(
        &self,
        preset: &LibPath,
        base_rev: Option<&str>,
        catalog: &mut Catalog,
        mounts: &Mounts,
        seq: &dyn Fn() -> u64,
        op: impl FnOnce(&Edit<'_>) -> Result<(PresetLists, T), LibraryRefusal>,
        touched: &mut Vec<PathKey>,
    ) -> Result<Rewritten<T>, LibraryRefusal> {
        let file = self.admit(mounts, preset)?;
        let read_seq = seq();
        let (tag, bytes) = self
            .store
            .tag(&file)
            .and_then(|tag| Ok((tag, self.store.read(&file, LIBRARY_READ_CAP)?)))
            .map_err(|e| read_refused(preset, &e))?;
        let rev = sha256_hex(&bytes);
        let holds = matches!(
            catalog.get(&key(preset)).map(|item| &item.content),
            Some(ItemContent::Preset { rev: held, .. }) if *held == rev
        );
        if !holds {
            touched.extend(self.apply_bytes(catalog, preset, &bytes, tag, read_seq));
        }
        let backends = &*self.values.backends;
        let text = std::str::from_utf8(&bytes).map_err(|_| {
            invalid(ParseError::new(YAML_SYNTAX, "the file is not valid UTF-8", None).to_reason())
        })?;
        let doc = parse_preset(text.strip_prefix('\u{feff}').unwrap_or(text), backends)
            .map_err(|e| invalid(e.to_reason()))?;
        // A file that parses but declares its `kind` where the scan does not see it (a flow mapping) is no catalog item:
        //   the apply above removed it, and an edit answering for it would describe a preset `preset.get` cannot find.
        if !matches!(
            catalog.get(&key(preset)).map(|item| &item.content),
            Some(ItemContent::Preset { .. })
        ) {
            return Err(LibraryRefusal::new(
                RefusalClass::InvalidArgument,
                WRONG_KIND,
                json!({ "path": preset.as_str() }),
            ));
        }
        if base_rev.is_some_and(|base| base != rev) {
            return Err(LibraryRefusal::new(
                RefusalClass::Conflict,
                REV_MISMATCH,
                json!({ "rev": rev }),
            ));
        }
        let (lists, answer) = op(&Edit {
            doc: &doc,
            catalog,
            mounts,
        })?;
        if same_lists(&as_read(&doc), &lists) {
            return Ok(Rewritten {
                answer,
                doc,
                rev,
                written: false,
            });
        }
        let written = rewrite_lists(text, &lists, backends).map_err(|e| invalid(e.to_reason()))?;
        // The answer describes the written file, so it must read as a preset — checked before the write, a writer
        //   refusal like the others (the file untouched).
        let written_doc = parse_preset(
            written.strip_prefix('\u{feff}').unwrap_or(&written),
            backends,
        )
        .map_err(|e| invalid(e.to_reason()))?;
        let written_seq = seq();
        self.store
            .write_atomic(&file, written.as_bytes())
            .map_err(|e| {
                LibraryRefusal::new(
                    RefusalClass::Unavailable,
                    LIBRARY_WRITE_FAILED,
                    json!({ "path": preset.as_str(), "why": e.to_string() }),
                )
            })?;
        let tag = self.tag_after_write(preset, &file, written.len());
        touched.extend(self.apply_bytes(catalog, preset, written.as_bytes(), tag, written_seq));
        tracing::info!(preset = preset.as_str(), "preset written");
        Ok(Rewritten {
            answer,
            doc: written_doc,
            rev: sha256_hex(written.as_bytes()),
            written: true,
        })
    }

    /// The tag of `file` (library path `path`) just written with `len` bytes, or a stand-in when it cannot be read: the
    /// item is applied racy whatever its tag (`Catalog::apply_items`), so a stand-in only costs a re-read.
    pub(crate) fn tag_after_write(&self, path: &LibPath, file: &Path, len: usize) -> ContentTag {
        self.store.tag(file).unwrap_or_else(|e| {
            tracing::debug!(
                path = path.as_str(),
                "tag after write failed; applying a stand-in tag: {e}"
            );
            ContentTag {
                modified: SystemTime::now(),
                size: len as u64,
                identity: None,
            }
        })
    }

    /// Link admission for a new way into the catalog, answering `path`'s disk path in its mount's folder: `path` must
    /// lie below a mount's top (a path of one name `InvalidArgument` · `virtual-root-read-only`, judged first), pass
    /// [`admits_inside`] (else `InvalidArgument` · `item-path-invalid`), lie in a registered mount (else `NotFound` ·
    /// `not-found`), and lie below no link inside the mount (else `InvalidArgument` · [`PATH_THROUGH_LINK`]; a probe
    /// error `Unavailable` · `library-read-failed`). A name that is missing or under a file passes: nothing below it
    /// can be a link, and the read that follows answers it. `LibraryStore` read and write follow links, so it runs
    /// before any read or write of `path`.
    // ADR-0365
    // ADR-0400
    pub(crate) fn admit(&self, mounts: &Mounts, path: &LibPath) -> Result<PathBuf, LibraryRefusal> {
        if path.parent().is_none() {
            return Err(virtual_root_read_only(Some(path)));
        }
        if !admits_inside(path) {
            return Err(dot_or_temp(path));
        }
        let (mount, Some(rest)) = mounts
            .locate(path)
            .ok_or_else(|| not_found(path.as_str()))?
        else {
            unreachable!("a path of two names or more has a rest in its mount");
        };
        let file = rest.to_path(&mount.folder);
        match self.store.first_link(&mount.folder, &rest) {
            Ok(None) => Ok(file),
            Ok(Some(_)) => Err(LibraryRefusal::new(
                RefusalClass::InvalidArgument,
                PATH_THROUGH_LINK,
                json!({ "path": path.as_str() }),
            )),
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
                ) =>
            {
                Ok(file)
            }
            Err(e) => Err(LibraryRefusal::new(
                RefusalClass::Unavailable,
                LIBRARY_READ_FAILED,
                json!({ "path": path.as_str(), "why": e.to_string() }),
            )),
        }
    }

    /// Applies the item `bytes` make at `path` (read after `read_seq`); bytes that declare no `kind` remove the item, as
    /// a scan would.
    fn apply_bytes(
        &self,
        catalog: &mut Catalog,
        path: &LibPath,
        bytes: &[u8],
        tag: ContentTag,
        read_seq: u64,
    ) -> Vec<PathKey> {
        match yaml_item(bytes, &*self.values.backends) {
            Some((kind, content)) => catalog.apply_items(
                vec![Item {
                    path: path.clone(),
                    kind,
                    tag,
                    read_seq,
                    content,
                }],
                true,
            ),
            None => {
                let at = key(path);
                match catalog.remove(&at, read_seq) {
                    true => vec![at],
                    false => Vec::new(),
                }
            }
        }
    }

    /// The `add` of items over the file as read: each item judged and placed (see [`LibraryService::preset_add`]).
    fn add_items(
        &self,
        edit: &Edit<'_>,
        preset: &LibPath,
        items: &[NewItem],
        after: Option<&str>,
    ) -> (PresetLists, (Vec<String>, Vec<Skipped>)) {
        let lookup = CatalogLookup {
            catalog: edit.catalog,
            mounts: edit.mounts,
            builtins: &self.builtins,
        };
        let anchor = after.and_then(|after| AssetRef::parse(after).ok());
        let mut lists = as_read(edit.doc);
        let mut next: HashMap<ListName, usize> = HashMap::new();
        let (mut applied, mut skipped) = (Vec::new(), Vec::new());
        for item in items {
            let target = match parse_ref(&item.path) {
                Ok(target) => target,
                Err(reason) => {
                    skipped.push(Skipped {
                        item: item.path.clone(),
                        reason,
                    });
                    continue;
                }
            };
            let list = match lookup.item(&target) {
                Looked::Present(view) => match list_of(view.kind) {
                    Some(list) => list,
                    None => {
                        skipped.push(skip(&item.path, WRONG_KIND, None));
                        continue;
                    }
                },
                _ => {
                    skipped.push(skip(&item.path, NOT_FOUND, None));
                    continue;
                }
            };
            let backend = item.backend.as_deref();
            if let Some(code) = self.mismatch(&lookup, edit.catalog, preset, list, &target, backend)
            {
                skipped.push(skip(&item.path, code, backend));
                continue;
            }
            let entries = list_mut(&mut lists, list);
            let target_key = ref_key(&target);
            if entries
                .iter()
                .any(|entry| ref_key(&entry.target) == target_key)
            {
                skipped.push(skip(&item.path, ALREADY_PRESENT, None));
                continue;
            }
            let at = next.get(&list).copied().or_else(|| {
                let anchor = anchor.as_ref().map(ref_key)?;
                let found = entries
                    .iter()
                    .position(|entry| ref_key(&entry.target) == anchor)?;
                Some(found + 1)
            });
            let new = PresetItem {
                target,
                backend: backend.map(|word| self.canonical_word(word)),
                line: 0,
                origin: None,
            };
            match at {
                Some(at) => {
                    entries.insert(at, new);
                    next.insert(list, at + 1);
                }
                None => entries.push(new),
            }
            applied.push(item.path.clone());
        }
        (lists, (applied, skipped))
    }

    /// The registry's own spelling of a known backend word.
    fn canonical_word(&self, word: &str) -> String {
        match self.values.backends.for_word(word) {
            Some(check) => check.backend().to_owned(),
            None => word.to_owned(),
        }
    }

    /// Why `target` (tagged `backend`) cannot be listed in `list` of library preset `own` — an unallowed or unknown tag,
    /// a built-in in `presets`, an item of another kind, a preset that reaches `own` (or is `own`); `None` = it fits. A
    /// target the lookup does not hold fits (a missing line is legal content).
    fn mismatch(
        &self,
        lookup: &dyn Lookup,
        catalog: &Catalog,
        own: &LibPath,
        list: ListName,
        target: &AssetRef,
        backend: Option<&str>,
    ) -> Option<&'static str> {
        if let Some(word) = backend {
            if !list.takes_tags() {
                return Some(BACKEND_TAG_NOT_ALLOWED);
            }
            if self.values.backends.for_word(word).is_none() {
                return Some(BACKEND_TAG_UNKNOWN);
            }
        }
        if list == ListName::Presets && matches!(target, AssetRef::BuiltIn(_)) {
            return Some(BUILTIN_IN_LIBRARY_PRESET);
        }
        if let Looked::Present(view) = lookup.item(target) {
            if list_of(view.kind) != Some(list) {
                return Some(WRONG_KIND);
            }
        }
        if list == ListName::Presets && closes_cycle(catalog, own, target) {
            return Some(PRESET_CYCLE);
        }
        None
    }

    /// The lists `draft` makes over `doc` (the file as read; `None` = none known) and every mismatch, the mismatched
    /// entries left out. A kept item takes the origin of the first unused entry of its list with its key; a folder the
    /// origin of the first unused entry with its normalized key, and is written normalized. Draft folders omitted =
    /// `doc`'s folders as they are.
    fn draft_lists(
        &self,
        draft: &PresetDraft,
        doc: Option<&PresetDoc>,
        own: &LibPath,
        lookup: &CatalogLookup<'_>,
    ) -> (PresetLists, Vec<SaveMismatch>) {
        let mut refused = Vec::new();
        let mut lists = PresetLists::default();
        for list in ITEM_LISTS {
            let given = match list {
                ListName::Presets => &draft.presets,
                ListName::Instructions => &draft.instructions,
                ListName::Skills => &draft.skills,
                _ => &draft.settings,
            };
            let original: &[PresetItem] = doc.map_or(&[], |doc| doc_list(doc, list));
            let mut used = HashSet::new();
            let mut seen = HashSet::new();
            let mut kept = Vec::new();
            for item in given {
                let mut refuse = |code| {
                    refused.push(SaveMismatch {
                        list,
                        path: escape_controls(&item.path),
                        code,
                    })
                };
                let Ok(target) = AssetRef::parse(&item.path) else {
                    refuse(ITEM_PATH_INVALID);
                    continue;
                };
                let backend = item.backend.as_deref();
                if let Some(code) =
                    self.mismatch(lookup, lookup.catalog, own, list, &target, backend)
                {
                    refuse(code);
                    continue;
                }
                let target_key = ref_key(&target);
                if !seen.insert(target_key.clone()) {
                    refuse(ITEM_DUPLICATE);
                    continue;
                }
                let origin = original
                    .iter()
                    .filter(|entry| ref_key(&entry.target) == target_key)
                    .find_map(|entry| entry.origin.filter(|origin| used.insert(*origin)));
                kept.push(PresetItem {
                    target,
                    backend: backend.map(|word| self.canonical_word(word)),
                    line: 0,
                    origin,
                });
            }
            *list_mut(&mut lists, list) = kept;
        }
        // ADR-0338: a supplied list replaces the file's whole list — the entries the reader could not model too.
        lists.drop_unmodeled = doc
            .map(|doc| {
                doc.unmodeled
                    .iter()
                    .filter(|entry| entry.list != ListName::Folders || draft.folders.is_some())
                    .map(|entry| (entry.list, entry.index))
                    .collect()
            })
            .unwrap_or_default();
        lists.folders = match &draft.folders {
            None => doc.map(|doc| as_read(doc).folders).unwrap_or_default(),
            Some(given) => {
                let original: &[FolderRef] = doc.map_or(&[], |doc| &doc.folders);
                let mut used = HashSet::new();
                let mut seen = HashSet::new();
                let mut kept = Vec::new();
                for raw in given {
                    let mut refuse = |code| {
                        refused.push(SaveMismatch {
                            list: ListName::Folders,
                            path: escape_controls(raw),
                            code,
                        })
                    };
                    let Ok(normalized) = normalize_folder(Path::new(raw)) else {
                        refuse(FOLDER_PATH_RELATIVE);
                        continue;
                    };
                    let id = FolderId::Key(path_key(&normalized));
                    if !seen.insert(id.clone()) {
                        refuse(ITEM_DUPLICATE);
                        continue;
                    }
                    let origin = original
                        .iter()
                        .filter(|entry| folder_id_of(entry).1 == id)
                        .find_map(|entry| entry.origin.filter(|origin| used.insert(*origin)));
                    kept.push(FolderItem {
                        path: normalized.to_string_lossy().into_owned(),
                        origin,
                    });
                }
                kept
            }
        };
        (lists, refused)
    }

    /// Per raw folder path: its normalized form, its bounded stat and the verdict a folder `add` gives it (relative,
    /// then cmd-unsafe, then the stat). Every normalized path is stat'ed, in one batch on `io`.
    fn folder_verdicts(&self, io: &LaunchIo, folders: &[String]) -> Vec<FolderVerdict> {
        let normalized: Vec<Option<PathBuf>> = folders
            .iter()
            .map(|raw| normalize_folder(Path::new(raw)).ok())
            .collect();
        let asks: Vec<(PathBuf, Need)> = normalized
            .iter()
            .flatten()
            .map(|path| (path.clone(), Need::Required))
            .collect();
        let statuses = self.io.stat_all(io, &asks);
        folders
            .iter()
            .zip(normalized)
            .map(|(raw, normalized)| {
                let refusal = |code, why| {
                    Err(FolderRefusal {
                        path: escape_controls(raw),
                        code,
                        why,
                    })
                };
                let Some(path) = normalized else {
                    return FolderVerdict {
                        normalized: None,
                        status: None,
                        judged: refusal(FOLDER_PATH_RELATIVE, None),
                    };
                };
                // ADR-0293: `stat_all` answers every path it is asked, but a broken answer is an unreadable folder
                //   here (the folder is refused), never an abort.
                let status = statuses.get(&path_key(&path)).cloned().unwrap_or_else(|| {
                    tracing::error!(
                        path = %escape_controls(&path.to_string_lossy()),
                        "the bounded stat gave no answer for the path it was asked"
                    );
                    FolderStatus::Unreadable(STAT_UNANSWERED.to_owned())
                });
                let judged = if !path_is_cli_safe(&path) {
                    refusal(FOLDER_PATH_CLI_UNSAFE, None)
                } else {
                    // ADR-0387: absent and unreadable folders refuse with their own codes, as at launch.
                    match &status {
                        FolderStatus::Ok => Ok(path.clone()),
                        FolderStatus::Missing | FolderStatus::NotADirectory => {
                            refusal(FOLDER_NOT_FOUND, Some(status.why()))
                        }
                        FolderStatus::Unreadable(why) => {
                            refusal(FOLDER_UNREADABLE, Some(why.clone()))
                        }
                    }
                };
                FolderVerdict {
                    normalized: Some(path),
                    status: Some(status),
                    judged,
                }
            })
            .collect()
    }
}

struct FolderVerdict {
    normalized: Option<PathBuf>,
    status: Option<FolderStatus>,
    /// The `add` verdict: the path to store, or why not.
    judged: Result<PathBuf, FolderRefusal>,
}

/// One entry of [`PRESET_FOLDERS_REFUSED`]'s `args.folders`.
fn refusal_json(refusal: &FolderRefusal) -> Value {
    let mut entry = json!({ "path": refusal.path, "code": refusal.code });
    if let Some(why) = &refusal.why {
        entry["why"] = json!(why);
    }
    entry
}

/// The identity a folder entry is matched and deduplicated by: the key of its normalized path, or — a relative entry,
/// which does not normalize — its spelling.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum FolderId {
    Key(PathKey),
    Raw(String),
}

/// `raw` as a folder row shows it and its identity.
fn folder_id(raw: &str) -> (String, FolderId) {
    match normalize_folder(Path::new(raw)) {
        Ok(path) => (
            path.to_string_lossy().into_owned(),
            FolderId::Key(path_key(&path)),
        ),
        Err(_) => (raw.to_owned(), FolderId::Raw(raw.to_owned())),
    }
}

/// The own folder rows of `doc` — normalized and deduplicated, first place kept — with each row's key (`None` = a
/// relative entry); a line of 0 (a `check` draft folder) is none.
fn own_folders(doc: &PresetDoc) -> (Vec<PresetFolder>, Vec<Option<PathKey>>) {
    let mut seen = HashSet::new();
    let (mut folders, mut keys) = (Vec::new(), Vec::new());
    for entry in &doc.folders {
        let (shown, id) = folder_id_of(entry);
        if seen.insert(id) {
            keys.push(entry.normalized.as_ref().ok().map(|path| path_key(path)));
            folders.push(PresetFolder {
                path: shown,
                line: (entry.line > 0).then_some(entry.line),
                status: None,
            });
        }
    }
    (folders, keys)
}

/// Each folder row (with its key in `keys`) takes the status `resolution` stat'ed for that key.
fn attach_statuses(
    folders: &mut [PresetFolder],
    keys: &[Option<PathKey>],
    resolution: &Resolution,
) {
    for (folder, folder_key) in folders.iter_mut().zip(keys) {
        folder.status = folder_key.as_ref().and_then(|folder_key| {
            resolution
                .folders
                .iter()
                .find(|resolved| resolved.key == *folder_key)
                .and_then(|resolved| resolved.status.clone())
        });
    }
}

fn folder_id_of(entry: &FolderRef) -> (String, FolderId) {
    match &entry.normalized {
        Ok(path) => (
            path.to_string_lossy().into_owned(),
            FolderId::Key(path_key(path)),
        ),
        Err(_) => (entry.raw.clone(), FolderId::Raw(entry.raw.clone())),
    }
}

/// The folder `add` over the file as read, with each path's verdict judged before the change (`verdicts`, in the
/// order of `raw`): a folder already listed (by its identity) is skipped first; then any other refused verdict refuses
/// the whole call [`PRESET_FOLDERS_REFUSED`] naming each; else each path is added, normalized, after the `after` folder
/// in call order (no anchor = at the end).
fn add_folders(
    doc: &PresetDoc,
    raw: &[String],
    verdicts: &[FolderVerdict],
    after: Option<&str>,
) -> Result<(PresetLists, (Vec<String>, Vec<Skipped>)), LibraryRefusal> {
    let mut lists = as_read(doc);
    let anchor = after.map(|after| folder_id(after).1);
    let mut next: Option<usize> = None;
    let (mut applied, mut skipped, mut refused) = (Vec::new(), Vec::new(), Vec::new());
    for (raw, verdict) in raw.iter().zip(verdicts) {
        let id = match &verdict.normalized {
            Some(path) => FolderId::Key(path_key(path)),
            None => FolderId::Raw(raw.clone()),
        };
        if lists
            .folders
            .iter()
            .any(|entry| folder_id(&entry.path).1 == id)
        {
            skipped.push(skip(raw, ALREADY_PRESENT, None));
            continue;
        }
        let path = match &verdict.judged {
            Ok(path) => path,
            Err(refusal) => {
                refused.push(refusal_json(refusal));
                continue;
            }
        };
        let at = next.or_else(|| {
            let anchor = anchor.as_ref()?;
            let found = lists
                .folders
                .iter()
                .position(|entry| folder_id(&entry.path).1 == *anchor)?;
            Some(found + 1)
        });
        let shown = path.to_string_lossy().into_owned();
        let new = FolderItem {
            path: shown.clone(),
            origin: None,
        };
        match at {
            Some(at) => {
                lists.folders.insert(at, new);
                next = Some(at + 1);
            }
            None => lists.folders.push(new),
        }
        applied.push(shown);
    }
    if !refused.is_empty() {
        return Err(LibraryRefusal::new(
            RefusalClass::InvalidArgument,
            PRESET_FOLDERS_REFUSED,
            json!({ "folders": refused }),
        ));
    }
    Ok((lists, (applied, skipped)))
}

/// `current` in `order` (spellings, matched by key; a duplicate takes the next unused entry of its key), or
/// [`REORDER_ORDER_MISMATCH`] when `order` is not a permutation of it.
fn reorder_items(
    current: &[PresetItem],
    order: &[String],
) -> Result<Vec<PresetItem>, LibraryRefusal> {
    let mismatch = || {
        let expected: Vec<String> = current.iter().map(|item| item.target.spelling()).collect();
        LibraryRefusal::new(
            RefusalClass::InvalidArgument,
            REORDER_ORDER_MISMATCH,
            json!({ "expected": expected }),
        )
    };
    if order.len() != current.len() {
        return Err(mismatch());
    }
    let mut used = vec![false; current.len()];
    let mut reordered = Vec::with_capacity(current.len());
    for spelling in order {
        let wanted = AssetRef::parse(spelling)
            .map(|target| ref_key(&target))
            .map_err(|_| mismatch())?;
        let at = (0..current.len())
            .find(|&at| !used[at] && ref_key(&current[at].target) == wanted)
            .ok_or_else(mismatch)?;
        used[at] = true;
        reordered.push(current[at].clone());
    }
    Ok(reordered)
}

/// The folder section in `order` (matched by identity against the section as `preset.get` shows it), written
/// normalized and deduplicated — each folder from its first-place entry — or [`REORDER_ORDER_MISMATCH`].
fn reorder_folders(doc: &PresetDoc, order: &[String]) -> Result<Vec<FolderItem>, LibraryRefusal> {
    let mut shown: Vec<(String, FolderId, Option<crate::path::ItemOrigin>)> = Vec::new();
    for entry in &doc.folders {
        let (path, id) = folder_id_of(entry);
        if !shown.iter().any(|(_, seen, _)| *seen == id) {
            shown.push((path, id, entry.origin));
        }
    }
    let mismatch = || {
        let expected: Vec<&str> = shown.iter().map(|(path, _, _)| path.as_str()).collect();
        LibraryRefusal::new(
            RefusalClass::InvalidArgument,
            REORDER_ORDER_MISMATCH,
            json!({ "expected": expected }),
        )
    };
    if order.len() != shown.len() {
        return Err(mismatch());
    }
    let mut used = vec![false; shown.len()];
    let mut reordered = Vec::with_capacity(shown.len());
    for raw in order {
        let wanted = folder_id(raw).1;
        let at = (0..shown.len())
            .find(|&at| !used[at] && shown[at].1 == wanted)
            .ok_or_else(mismatch)?;
        used[at] = true;
        let (path, _, origin) = &shown[at];
        reordered.push(FolderItem {
            path: path.clone(),
            origin: *origin,
        });
    }
    Ok(reordered)
}

/// The `setBackend` over the file as read (see [`LibraryService::preset_set_backend`]); `word` is the registry's
/// spelling. A tag equal to the one asked, ASCII case aside, is left as written.
fn set_backend(
    doc: &PresetDoc,
    items: &[String],
    word: Option<&'static str>,
) -> (PresetLists, (Vec<String>, Vec<Skipped>)) {
    let mut lists = as_read(doc);
    let (mut applied, mut skipped) = (Vec::new(), Vec::new());
    for item in items {
        let wanted = match parse_ref(item) {
            Ok(target) => ref_key(&target),
            Err(reason) => {
                skipped.push(Skipped {
                    item: item.clone(),
                    reason,
                });
                continue;
            }
        };
        let mut found = false;
        for list in [ListName::Instructions, ListName::Skills] {
            for entry in list_mut(&mut lists, list) {
                if ref_key(&entry.target) != wanted {
                    continue;
                }
                found = true;
                let same = match (&entry.backend, word) {
                    (Some(have), Some(want)) => have.eq_ignore_ascii_case(want),
                    (None, None) => true,
                    _ => false,
                };
                if !same {
                    entry.backend = word.map(str::to_owned);
                }
            }
        }
        if found {
            applied.push(item.clone());
            continue;
        }
        let elsewhere = [ListName::Presets, ListName::Settings]
            .into_iter()
            .any(|list| {
                doc_list(doc, list)
                    .iter()
                    .any(|entry| ref_key(&entry.target) == wanted)
            });
        let code = match elsewhere {
            true => BACKEND_TAG_NOT_ALLOWED,
            false => NOT_FOUND,
        };
        skipped.push(skip(item, code, word));
    }
    (lists, (applied, skipped))
}

/// The lists of `doc` as read — what every edit starts from: every modeled item with its origin, every folder entry
/// in its original spelling, nothing dropped.
pub(crate) fn as_read(doc: &PresetDoc) -> PresetLists {
    PresetLists {
        presets: doc.presets.clone(),
        instructions: doc.instructions.clone(),
        skills: doc.skills.clone(),
        settings: doc.settings.clone(),
        folders: doc
            .folders
            .iter()
            .map(|folder| FolderItem {
                path: folder.raw.clone(),
                origin: folder.origin,
            })
            .collect(),
        drop_unmodeled: Vec::new(),
    }
}

/// Whether `a` and `b` write the same lists: per item its spelling, tag and origin (not its line), per folder its
/// path and origin, and the dropped unmodeled entries.
fn same_lists(a: &PresetLists, b: &PresetLists) -> bool {
    let items =
        |list: &[PresetItem]| -> Vec<(String, Option<String>, Option<crate::path::ItemOrigin>)> {
            list.iter()
                .map(|item| (item.target.spelling(), item.backend.clone(), item.origin))
                .collect()
        };
    ITEM_LISTS
        .into_iter()
        .all(|list| items(lists_ref(a, list)) == items(lists_ref(b, list)))
        && a.folders == b.folders
        && a.drop_unmodeled == b.drop_unmodeled
}

pub(crate) fn doc_list(doc: &PresetDoc, list: ListName) -> &[PresetItem] {
    match list {
        ListName::Presets => &doc.presets,
        ListName::Instructions => &doc.instructions,
        ListName::Skills => &doc.skills,
        ListName::Settings => &doc.settings,
        ListName::Folders => &[],
    }
}

fn lists_ref(lists: &PresetLists, list: ListName) -> &[PresetItem] {
    match list {
        ListName::Presets => &lists.presets,
        ListName::Instructions => &lists.instructions,
        ListName::Skills => &lists.skills,
        ListName::Settings => &lists.settings,
        ListName::Folders => &[],
    }
}

/// The item list `list` of `lists` — one of [`ITEM_LISTS`].
pub(crate) fn list_mut(lists: &mut PresetLists, list: ListName) -> &mut Vec<PresetItem> {
    match list {
        ListName::Presets => &mut lists.presets,
        ListName::Instructions => &mut lists.instructions,
        ListName::Skills => &mut lists.skills,
        ListName::Settings => &mut lists.settings,
        ListName::Folders => unreachable!("the folders list holds no items"),
    }
}

fn rows_mut(view: &mut PresetView) -> impl Iterator<Item = &mut PresetRow> {
    view.presets
        .iter_mut()
        .chain(view.instructions.iter_mut())
        .chain(view.skills.iter_mut())
        .chain(view.settings.iter_mut())
}

/// The list an item of `kind` goes into; a broken file goes where its claimed kind goes. `None` = it can be listed
/// nowhere (a `SKILL.md`, or a broken file with no known kind).
pub(crate) fn list_of(kind: &ItemKind) -> Option<ListName> {
    match kind {
        ItemKind::Preset
        | ItemKind::Broken {
            claimed: Some(Sniff::Preset),
        } => Some(ListName::Presets),
        ItemKind::Settings
        | ItemKind::Broken {
            claimed: Some(Sniff::Settings),
        } => Some(ListName::Settings),
        ItemKind::Instruction => Some(ListName::Instructions),
        ItemKind::Skill => Some(ListName::Skills),
        ItemKind::SkillFile | ItemKind::Broken { .. } => None,
    }
}

/// Whether listing `target` in `own`'s `presets` closes a cycle: it is `own`, or it reaches `own` through nested
/// `presets` lists in the catalog — the only lists resolution expands (a preset listed under another list is a
/// `wrong-kind` row there, not an expansion). Own's own lists are never walked: the walk ends where it reaches `own`.
pub(crate) fn closes_cycle(catalog: &Catalog, own: &LibPath, target: &AssetRef) -> bool {
    let AssetRef::Library(target) = target else {
        return false;
    };
    let own = key(own);
    let mut visited = HashSet::new();
    let mut stack = vec![key(target)];
    while let Some(at) = stack.pop() {
        if at == own {
            return true;
        }
        if !visited.insert(at.clone()) {
            continue;
        }
        if let Some(ItemContent::Preset { doc, .. }) = catalog.get(&at).map(|item| &item.content) {
            stack.extend(doc.presets.iter().filter_map(|item| match &item.target {
                AssetRef::Library(path) => Some(key(path)),
                AssetRef::BuiltIn(_) => None,
            }));
        }
    }
    false
}

/// The state of one entry of `own`'s `list` (see [`PresetRow::state`]).
fn row_state(
    lookup: &dyn Lookup,
    list: ListName,
    item: &PresetItem,
    own: &AssetRef,
) -> Option<Reason> {
    let path = item.target.spelling();
    let reason = |code: &str, args: Value| {
        Some(Reason {
            code: code.to_owned(),
            args,
            line: Some(item.line),
        })
    };
    if list == ListName::Presets {
        if ref_key(&item.target) == ref_key(own) {
            return reason(PRESET_CYCLE, json!({ "path": path }));
        }
        if matches!(own, AssetRef::Library(_)) && matches!(item.target, AssetRef::BuiltIn(_)) {
            return reason(BUILTIN_IN_LIBRARY_PRESET, json!({ "path": path }));
        }
    }
    match lookup.item(&item.target) {
        Looked::Missing => reason(NOT_FOUND, json!({ "path": path })),
        Looked::Unreadable(why) => Some(why),
        Looked::Present(view) => match view.content {
            ItemContent::Broken { reason: why } => Some(why.clone()),
            _ if list_of(view.kind) != Some(list) => {
                reason(WRONG_KIND, json!({ "path": path, "list": list.key() }))
            }
            ItemContent::Settings { verdict, .. } => {
                verdict.problems.first().map(|(_, refusal)| refusal.clone())
            }
            _ => None,
        },
    }
}

/// A reference spelling, or why not: `item-path-invalid` (`args` = `{path, why}`).
pub(crate) fn parse_ref(text: &str) -> Result<AssetRef, Reason> {
    AssetRef::parse(text).map_err(|why| Reason {
        code: ITEM_PATH_INVALID.to_owned(),
        args: json!({ "path": escape_controls(text), "why": why.to_string() }),
        line: None,
    })
}

/// A library path spelling, or `InvalidArgument` · `item-path-invalid` (`args` = `{path, why}`) — a built-in included.
pub(crate) fn lib_path(text: &str) -> Result<LibPath, LibraryRefusal> {
    match parse_ref(text) {
        Ok(AssetRef::Library(path)) => Ok(path),
        Ok(AssetRef::BuiltIn(_)) => Err(LibraryRefusal::new(
            RefusalClass::InvalidArgument,
            ITEM_PATH_INVALID,
            json!({ "path": escape_controls(text), "why": "a built-in is not a library path" }),
        )),
        Err(reason) => Err(invalid(reason)),
    }
}

/// A skip with `code`: `args` = `{backend}` for a tag code (given `backend`), else `{path}`.
pub(crate) fn skip(item: &str, code: &str, backend: Option<&str>) -> Skipped {
    let args = match (code, backend) {
        (BACKEND_TAG_NOT_ALLOWED | BACKEND_TAG_UNKNOWN, Some(word)) => {
            json!({ "path": escape_controls(item), "backend": escape_controls(word) })
        }
        _ => json!({ "path": escape_controls(item) }),
    };
    Skipped {
        item: item.to_owned(),
        reason: Reason {
            code: code.to_owned(),
            args,
            line: None,
        },
    }
}

fn mismatch_json(mismatch: &SaveMismatch) -> Value {
    json!({ "list": mismatch.list.key(), "path": mismatch.path, "code": mismatch.code })
}

fn invalid(reason: Reason) -> LibraryRefusal {
    LibraryRefusal::Refused {
        class: RefusalClass::InvalidArgument,
        reason,
    }
}

pub(crate) fn dot_or_temp(path: &LibPath) -> LibraryRefusal {
    LibraryRefusal::new(
        RefusalClass::InvalidArgument,
        ITEM_PATH_INVALID,
        json!({ "path": path.as_str(), "why": "a dot or temp name" }),
    )
}

pub(crate) fn not_found(path: &str) -> LibraryRefusal {
    LibraryRefusal::new(RefusalClass::NotFound, NOT_FOUND, json!({ "path": path }))
}

/// A read inside an edit that failed: the file or a parent missing is `not-found`, anything else
/// `library-read-failed`.
fn read_refused(path: &LibPath, e: &io::Error) -> LibraryRefusal {
    match e.kind() {
        io::ErrorKind::NotFound | io::ErrorKind::NotADirectory => not_found(path.as_str()),
        _ => LibraryRefusal::new(
            RefusalClass::Unavailable,
            LIBRARY_READ_FAILED,
            json!({ "path": path.as_str(), "why": e.to_string() }),
        ),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{mpsc, Arc, Mutex, OnceLock, Weak};
    use std::thread;
    use std::time::Duration;

    use engram_dashboard_agent::profile::{
        AgentCommand, AgentProfile, AttachedPreset, ProfileRegistry, ProfileStore, StoreError,
    };
    use engram_dashboard_base::sync::lock;
    use engram_dashboard_platform::fs::{link_dir, same_path, unlink_dir};
    use tempfile::TempDir;

    use super::*;
    use crate::backends::AgentBackends;
    use crate::config::{LibraryConfig, MountPlace};
    use crate::folder::{
        BoundedIo, FolderListing, FolderOps, LaunchWaits, StdFolderOps, ThreadSpawn, IO_LIMIT,
        LIST_CAP,
    };
    use crate::mounts::MountPath;
    use crate::store::{FsStore, LibraryStore, ScanListing, TrashBundle};
    use crate::{LibraryValues, LIBRARY_UNAVAILABLE};

    // ── the world: a real library folder, a logging store, folder ops that watch the library lock ──────────────

    /// [`FsStore`] with a log of reads and writes, one injectable read or write failure, an injectable link-probe
    /// error, and one walk a test can park (it says it arrived, then waits for the go).
    #[derive(Default)]
    struct Disk {
        reads: Mutex<Vec<PathBuf>>,
        writes: Mutex<Vec<PathBuf>>,
        fail_read: Mutex<Option<PathBuf>>,
        fail_write: Mutex<Option<PathBuf>>,
        fail_link: Mutex<Option<io::ErrorKind>>,
        hold: Mutex<Option<Hold>>,
    }

    /// The root whose next walk parks, where it says it arrived, and where it waits for the go.
    type Hold = (PathBuf, mpsc::Sender<()>, mpsc::Receiver<()>);

    impl LibraryStore for Disk {
        fn scan(
            &self,
            root: &Path,
            under: Option<&MountPath>,
            seq: &dyn Fn() -> u64,
        ) -> io::Result<ScanListing> {
            let parked = {
                let mut hold = lock(&self.hold);
                match hold.as_ref() {
                    Some((at, _, _)) if at == root => hold.take(),
                    _ => None,
                }
            };
            if let Some((_, arrived, go)) = parked {
                arrived.send(()).unwrap();
                go.recv_timeout(Duration::from_secs(10)).unwrap();
            }
            FsStore.scan(root, under, seq)
        }
        fn first_link(&self, root: &Path, rel: &MountPath) -> io::Result<Option<MountPath>> {
            if let Some(kind) = *lock(&self.fail_link) {
                return Err(io::Error::from(kind));
            }
            FsStore.first_link(root, rel)
        }
        fn read(&self, path: &Path, cap: u64) -> io::Result<Vec<u8>> {
            lock(&self.reads).push(path.to_path_buf());
            if lock(&self.fail_read).as_deref() == Some(path) {
                return Err(io::Error::from(io::ErrorKind::PermissionDenied));
            }
            FsStore.read(path, cap)
        }
        fn tag(&self, path: &Path) -> io::Result<ContentTag> {
            FsStore.tag(path)
        }
        fn write_atomic(&self, path: &Path, bytes: &[u8]) -> io::Result<()> {
            lock(&self.writes).push(path.to_path_buf());
            if lock(&self.fail_write).as_deref() == Some(path) {
                return Err(io::Error::other("injected write failure"));
            }
            FsStore.write_atomic(path, bytes)
        }
        fn create_new(&self, path: &Path, bytes: &[u8]) -> io::Result<()> {
            FsStore.create_new(path, bytes)
        }
        fn create_dir(&self, path: &Path) -> io::Result<()> {
            FsStore.create_dir(path)
        }
        fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
            FsStore.rename(from, to)
        }
        fn copy(&self, from: &Path, to: &Path) -> io::Result<()> {
            FsStore.copy(from, to)
        }
        fn trash(&self, root: &Path, path: &Path, bundle: &TrashBundle) -> io::Result<PathBuf> {
            FsStore.trash(root, path, bundle)
        }
        fn exists(&self, path: &Path) -> bool {
            FsStore.exists(path)
        }
    }

    /// [`StdFolderOps`] whose stat logs each path with whether the library write lock was held, answers a fixed status
    /// for the paths a test sets, and runs a test's hook once at the stat of one path.
    #[derive(Default)]
    struct Ops {
        svc: OnceLock<Weak<LibraryService>>,
        stats: Mutex<Vec<(PathBuf, bool)>>,
        under_lock: AtomicBool,
        fixed: Mutex<HashMap<PathBuf, FolderStatus>>,
        on_stat: Mutex<Option<StatHook>>,
    }

    /// The path whose stat runs the hook (`None` = the next stat of any path) and the hook.
    type StatHook = (Option<PathBuf>, Box<dyn FnOnce() + Send>);

    impl Ops {
        fn stats_of(&self, path: &Path) -> usize {
            lock(&self.stats)
                .iter()
                .filter(|(at, _)| at == path)
                .count()
        }
    }

    impl FolderOps for Ops {
        fn stat(&self, p: &Path) -> FolderStatus {
            let mut held = false;
            if let Some(svc) = self.svc.get().and_then(Weak::upgrade) {
                if svc.gate.lib().try_write().is_err() {
                    held = true;
                    self.under_lock.store(true, Ordering::SeqCst);
                }
            }
            lock(&self.stats).push((p.to_path_buf(), held));
            let hook = {
                let mut hook = lock(&self.on_stat);
                match hook.as_ref() {
                    Some((at, _)) if at.as_deref().is_none_or(|at| at == p) => hook.take(),
                    _ => None,
                }
            };
            if let Some((_, run)) = hook {
                run();
            }
            match lock(&self.fixed).get(p) {
                Some(status) => status.clone(),
                None => StdFolderOps.stat(p),
            }
        }
        fn list(&self, p: &Path, cap: usize) -> Result<FolderListing, FolderStatus> {
            StdFolderOps.list(p, cap)
        }
        fn canonical(&self, p: &Path) -> io::Result<PathBuf> {
            StdFolderOps.canonical(p)
        }
        fn identity(&self, p: &Path) -> io::Result<engram_dashboard_platform::fs::FileIdentity> {
            StdFolderOps.identity(p)
        }
        fn read(&self, p: &Path, cap: u64) -> io::Result<String> {
            StdFolderOps.read(p, cap)
        }
    }

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

    /// A temp folder (canonical spelling) holding the library root `lib\` and, beside it, the folders a test makes.
    struct World {
        _temp: TempDir,
        base: PathBuf,
        disk: Arc<Disk>,
        ops: Arc<Ops>,
        profiles: Vec<AgentProfile>,
        root: bool,
        builtin: bool,
        svc: Arc<LibraryService>,
    }

    impl World {
        fn new() -> World {
            World::with(Vec::new(), true)
        }

        fn with(profiles: Vec<AgentProfile>, root: bool) -> World {
            World::make(profiles, root, None)
        }

        /// A world whose built-in folder holds `engram:/default` as `default_preset` (the other built-ins stand in).
        fn with_default(default_preset: &str) -> World {
            World::make(Vec::new(), true, Some(default_preset))
        }

        fn make(profiles: Vec<AgentProfile>, root: bool, default_preset: Option<&str>) -> World {
            let temp = tempfile::tempdir().unwrap();
            let base = dunce::canonicalize(temp.path()).unwrap();
            fs::create_dir_all(base.join("lib")).unwrap();
            fs::create_dir_all(base.join("state")).unwrap();
            if let Some(text) = default_preset {
                let file = base.join("builtin").join("presets").join("default.yaml");
                fs::create_dir_all(file.parent().unwrap()).unwrap();
                fs::write(file, text).unwrap();
            }
            let disk = Arc::new(Disk::default());
            let ops = Arc::new(Ops::default());
            let builtin = default_preset.is_some();
            let svc = build(&base, &disk, &ops, &profiles, root, builtin);
            World {
                _temp: temp,
                base,
                disk,
                ops,
                profiles,
                root,
                builtin,
                svc,
            }
        }

        /// A second service over the same folder — a daemon restart.
        fn rebuild(&self) -> Arc<LibraryService> {
            build(
                &self.base,
                &self.disk,
                &self.ops,
                &self.profiles,
                self.root,
                self.builtin,
            )
        }

        fn lib(&self, rel: &str) -> PathBuf {
            MountPath::parse(rel)
                .unwrap()
                .to_path(&self.base.join("lib"))
        }

        fn put(&self, rel: &str, text: &str) -> PathBuf {
            let path = self.lib(rel);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, text).unwrap();
            path
        }

        fn bytes(&self, rel: &str) -> Vec<u8> {
            fs::read(self.lib(rel)).unwrap()
        }

        fn text(&self, rel: &str) -> String {
            String::from_utf8(self.bytes(rel)).unwrap()
        }

        /// A folder outside the library, created; its normalized spelling.
        fn folder(&self, name: &str) -> String {
            let path = self.base.join(name);
            fs::create_dir_all(&path).unwrap();
            normalize_folder(&path)
                .unwrap()
                .to_string_lossy()
                .into_owned()
        }

        fn writes(&self) -> usize {
            lock(&self.disk.writes).len()
        }

        fn view(&self, rel: &str) -> PresetView {
            self.svc.preset_get(&AssetRef::Library(lp(rel))).unwrap()
        }

        fn catalog_rev(&self, rel: &str) -> Option<String> {
            self.svc.with_read(|state| {
                match state.catalog.get(&key(&lp(rel))).map(|item| &item.content) {
                    Some(ItemContent::Preset { rev, .. }) => Some(rev.clone()),
                    _ => None,
                }
            })
        }

        fn read_seq(&self, rel: &str) -> u64 {
            self.svc
                .with_read(|state| state.catalog.get(&key(&lp(rel))).unwrap().read_seq)
        }
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

    fn build(
        base: &Path,
        disk: &Arc<Disk>,
        ops: &Arc<Ops>,
        profiles: &[AgentProfile],
        root: bool,
        builtin: bool,
    ) -> Arc<LibraryService> {
        let state = base.join("state");
        let config = match root {
            true => LibraryConfig::usable(&state, MountPlace::Explicit(base.join("lib"))),
            false => LibraryConfig::missing(&state),
        };
        let values = LibraryValues {
            registry: Arc::new(ProfileRegistry::new(Arc::new(Fixed(profiles.to_vec())))),
            backends: Arc::new(AgentBackends),
            config,
            default_root: base.join("default-library"),
            data_root: base.join("data"),
            daemon_id: "test-daemon".to_owned(),
            root_generation_base: 7,
            builtin_dir: builtin.then(|| base.join("builtin")),
            launch_root: base.join("launch"),
            home: None,
            env: Arc::new(|_| None),
        };
        let io = BoundedIo::new(
            IO_LIMIT,
            LaunchWaits::LAUNCH,
            LIST_CAP,
            Arc::clone(ops) as Arc<dyn FolderOps>,
            Arc::new(ThreadSpawn),
        );
        let svc = Arc::new(LibraryService::new(
            values,
            Arc::clone(disk) as Arc<dyn LibraryStore>,
            io,
        ));
        // The first service of a world is the one the stat probe watches.
        let _ = ops.svc.set(Arc::downgrade(&svc));
        svc
    }

    fn lp(text: &str) -> LibPath {
        LibPath::parse(text).unwrap()
    }

    fn preset(body: &str) -> String {
        format!("kind: preset\nversion: 1\n{body}")
    }

    fn item(path: &str) -> NewItem {
        NewItem {
            path: path.to_owned(),
            backend: None,
        }
    }

    fn tagged(path: &str, backend: &str) -> NewItem {
        NewItem {
            path: path.to_owned(),
            backend: Some(backend.to_owned()),
        }
    }

    fn items(paths: &[&str]) -> AddEntries {
        AddEntries::Items(paths.iter().map(|path| item(path)).collect())
    }

    fn strings(list: &[&str]) -> Vec<String> {
        list.iter().map(|text| (*text).to_owned()).collect()
    }

    fn paths(rows: &[PresetRow]) -> Vec<&str> {
        rows.iter().map(|row| row.path.as_str()).collect()
    }

    fn folder_paths(view: &PresetView) -> Vec<&str> {
        view.folders
            .iter()
            .map(|folder| folder.path.as_str())
            .collect()
    }

    fn skips(edit: &PartialEdit) -> Vec<(&str, &str)> {
        edit.skipped
            .iter()
            .map(|skip| (skip.item.as_str(), skip.reason.code.as_str()))
            .collect()
    }

    fn refused<T: std::fmt::Debug>(answer: Result<T, LibraryRefusal>) -> (RefusalClass, Reason) {
        match answer {
            Err(LibraryRefusal::Refused { class, reason }) => (class, reason),
            other => panic!("expected a refusal: {other:?}"),
        }
    }

    fn code(refusal: &(RefusalClass, Reason)) -> (RefusalClass, &str) {
        (refusal.0, refusal.1.code.as_str())
    }

    /// The library most tests start from: three instructions, a skill, a settings asset and preset `P.yaml`.
    fn library(w: &World, p_body: &str) {
        w.put("rules/a.md", "a");
        w.put("rules/b.md", "b");
        w.put("rules/c.md", "c");
        w.put("skills/s/SKILL.md", "---\nname: s\ndescription: d\n---\n");
        w.put("S.yaml", "kind: settings\nversion: 1\n");
        w.put("P.yaml", &preset(p_body));
    }

    // ── rev and the view ────────────────────────────────────────────────────────────────────────────────────

    #[test]
    fn rev_is_the_sha256_of_the_exact_bytes_and_equal_across_a_service_rebuild() {
        let w = World::new();
        let text = format!(
            "\u{feff}{}",
            preset("instructions:\n- library/rules/a.md\n")
        );
        w.put("rules/a.md", "a");
        w.put("P.yaml", &text);
        let view = w.view("library/P.yaml");
        assert_eq!(view.rev, sha256_hex(text.as_bytes()));
        let rebuilt = w
            .rebuild()
            .preset_get(&AssetRef::Library(lp("library/P.yaml")))
            .unwrap();
        assert_eq!(rebuilt.rev, view.rev);
    }

    #[test]
    fn the_view_shows_each_row_with_its_state_and_counts_the_agents() {
        let w = World::with(
            vec![agent(&["library/P.yaml"], false), agent(&[], true)],
            true,
        );
        library(
            &w,
            "description: team\npresets:\n- library/P.yaml\ninstructions:\n- library/rules/a.md\n- library/gone.md\n- library/S.yaml\n\
             skills:\n- library/skills/s\n",
        );
        let view = w.view("library/P.yaml");
        assert_eq!(view.description.as_deref(), Some("team"));
        let state = |row: &PresetRow| row.state.as_ref().map(|state| state.code.clone());
        assert_eq!(
            view.instructions.iter().map(state).collect::<Vec<_>>(),
            [
                None,
                Some(NOT_FOUND.to_owned()),
                Some(WRONG_KIND.to_owned())
            ]
        );
        assert_eq!(state(&view.presets[0]).as_deref(), Some(PRESET_CYCLE));
        assert_eq!(state(&view.skills[0]), None);
        assert!(view.problems.iter().any(
            |problem| matches!(problem, BlockReason::Missing { item, .. } if item == "library/gone.md")
        ));
        assert_eq!(view.agents, 1);
        let default = w
            .svc
            .preset_get(&AssetRef::BuiltIn(BuiltinId::DefaultPreset))
            .unwrap();
        assert_eq!(default.agents, 1, "the agent whose default slot is on");
    }

    #[test]
    fn the_view_lists_folders_normalized_and_deduplicated_with_their_stat() {
        let w = World::new();
        let f1 = w.folder("f1");
        let sep = std::path::MAIN_SEPARATOR;
        let gone = w.base.join("gone").to_string_lossy().into_owned();
        w.put(
            "P.yaml",
            &preset(&format!(
                "folders:\n- '{f1}'\n- '{f1}{sep}'\n- '{f1}{sep}..{sep}f1'\n- rel/x\n- '{gone}'\n"
            )),
        );
        let view = w.view("library/P.yaml");
        let shown: Vec<(&str, Option<&FolderStatus>)> = view
            .folders
            .iter()
            .map(|folder| (folder.path.as_str(), folder.status.as_ref()))
            .collect();
        assert_eq!(
            shown,
            [
                (f1.as_str(), Some(&FolderStatus::Ok)),
                ("rel/x", None),
                (gone.as_str(), Some(&FolderStatus::Missing)),
            ]
        );
    }

    // ADR-0400 decision 4: with no mount registered a library path names no mount — absent, not unavailable.
    #[test]
    fn the_builtin_default_answers_with_no_root_while_library_presets_and_edits_do_not() {
        let w = World::with(Vec::new(), false);
        w.put("P.yaml", &preset(""));
        let default = w
            .svc
            .preset_get(&AssetRef::BuiltIn(BuiltinId::DefaultPreset))
            .unwrap();
        assert_eq!(default.path, "engram:/default");
        let got = refused(w.svc.preset_get(&AssetRef::Library(lp("library/P.yaml"))));
        assert_eq!(code(&got), (RefusalClass::NotFound, NOT_FOUND));
        let added = refused(w.svc.preset_add(
            &lp("library/P.yaml"),
            &items(&["library/rules/a.md"]),
            None,
            None,
        ));
        assert_eq!(code(&added), (RefusalClass::NotFound, NOT_FOUND));
        let builtin_only = PresetDraft {
            instructions: vec![item("engram:/priming")],
            ..PresetDraft::default()
        };
        assert!(w
            .svc
            .preset_check(&lp("library/P.yaml"), &builtin_only)
            .is_ok());
        let library_draft = PresetDraft {
            instructions: vec![item("library/rules/a.md")],
            ..PresetDraft::default()
        };
        let checked = w
            .svc
            .preset_check(&lp("library/P.yaml"), &library_draft)
            .unwrap();
        // With no mount the check answers; its library item stands behind the block-④ row of an unset library.
        assert!(
            matches!(
                checked.problems.as_slice(),
                [BlockReason::LibraryUnavailable { reason }] if reason.code == LIBRARY_UNAVAILABLE
            ),
            "{:?}",
            checked.problems
        );
    }

    // ── add · remove ────────────────────────────────────────────────────────────────────────────────────────

    #[test]
    fn add_is_idempotent_and_a_second_add_of_the_same_item_is_skipped() {
        let w = World::new();
        library(&w, "instructions:\n- library/rules/a.md\n");
        let added = w
            .svc
            .preset_add(
                &lp("library/P.yaml"),
                &items(&["library/rules/b.md"]),
                None,
                None,
            )
            .unwrap();
        assert_eq!(added.applied, ["library/rules/b.md"]);
        assert_eq!(
            paths(&added.view.instructions),
            ["library/rules/a.md", "library/rules/b.md"]
        );
        let (bytes, writes) = (w.bytes("P.yaml"), w.writes());
        let again = w
            .svc
            .preset_add(
                &lp("library/P.yaml"),
                &items(&["library/rules/b.md"]),
                None,
                None,
            )
            .unwrap();
        assert_eq!(skips(&again), [("library/rules/b.md", ALREADY_PRESENT)]);
        assert_eq!(w.bytes("P.yaml"), bytes);
        assert_eq!(w.writes(), writes);
        assert_eq!(again.view.rev, added.view.rev);
    }

    #[test]
    fn add_skips_each_item_that_cannot_be_listed_with_its_reason_and_applies_the_rest() {
        let w = World::new();
        library(&w, "");
        w.put("Q.yaml", &preset("presets:\n- library/P.yaml\n"));
        let entries = AddEntries::Items(vec![
            item("library/a//b"),
            item("library/gone.md"),
            tagged("library/rules/a.md", "nobody"),
            tagged("library/S.yaml", "claude"),
            item("engram:/default"),
            item("library/Q.yaml"),
            item("library/P.yaml"),
            item("library/skills/s/SKILL.md"),
            tagged("library/rules/b.md", "Codex"),
            item("library/skills/s"),
            item("library/S.yaml"),
            item("library/rules/b.md"),
        ]);
        let added = w
            .svc
            .preset_add(&lp("library/P.yaml"), &entries, None, None)
            .unwrap();
        assert_eq!(
            skips(&added),
            [
                ("library/a//b", ITEM_PATH_INVALID),
                ("library/gone.md", NOT_FOUND),
                ("library/rules/a.md", BACKEND_TAG_UNKNOWN),
                ("library/S.yaml", BACKEND_TAG_NOT_ALLOWED),
                ("engram:/default", BUILTIN_IN_LIBRARY_PRESET),
                ("library/Q.yaml", PRESET_CYCLE),
                ("library/P.yaml", PRESET_CYCLE),
                ("library/skills/s/SKILL.md", WRONG_KIND),
                ("library/rules/b.md", ALREADY_PRESENT),
            ]
        );
        assert_eq!(
            added.applied,
            ["library/rules/b.md", "library/skills/s", "library/S.yaml"]
        );
        let view = added.view;
        assert_eq!(paths(&view.instructions), ["library/rules/b.md"]);
        assert_eq!(view.instructions[0].backend.as_deref(), Some("codex"));
        assert_eq!(paths(&view.skills), ["library/skills/s"]);
        assert_eq!(paths(&view.settings), ["library/S.yaml"]);
    }

    #[test]
    fn add_places_items_after_the_anchor_in_call_order_and_appends_without_one() {
        let w = World::new();
        library(
            &w,
            "instructions:\n- library/rules/a.md\n- library/rules/c.md\n",
        );
        w.put("rules/d.md", "d");
        w.put("rules/e.md", "e");
        let p = lp("library/P.yaml");
        let view = w
            .svc
            .preset_add(
                &p,
                &items(&["library/rules/b.md", "library/rules/d.md"]),
                Some("library/rules/a.md"),
                None,
            )
            .unwrap()
            .view;
        assert_eq!(
            paths(&view.instructions),
            [
                "library/rules/a.md",
                "library/rules/b.md",
                "library/rules/d.md",
                "library/rules/c.md"
            ]
        );
        let view = w
            .svc
            .preset_add(
                &p,
                &items(&["library/rules/e.md"]),
                Some("library/rules/zz.md"),
                None,
            )
            .unwrap()
            .view;
        assert_eq!(
            paths(&view.instructions).last(),
            Some(&"library/rules/e.md")
        );
    }

    #[test]
    fn remove_drops_every_entry_of_an_item_and_is_idempotent() {
        let w = World::new();
        library(
            &w,
            "instructions:\n- library/rules/a.md\n- library/rules/b.md\n- path: library/rules/a.md\n  backend: codex\n",
        );
        let remove = RemoveEntries::Items(strings(&["library/rules/a.md"]));
        let view = w
            .svc
            .preset_remove(&lp("library/P.yaml"), &remove, None)
            .unwrap();
        assert_eq!(paths(&view.instructions), ["library/rules/b.md"]);
        let writes = w.writes();
        let again = w
            .svc
            .preset_remove(&lp("library/P.yaml"), &remove, None)
            .unwrap();
        assert_eq!(
            w.writes(),
            writes,
            "removing what is not there writes nothing"
        );
        assert_eq!(again.rev, view.rev);
    }

    // ── reorder ─────────────────────────────────────────────────────────────────────────────────────────────

    #[test]
    fn reorder_refuses_a_stale_rev_and_an_order_that_is_not_a_permutation() {
        let w = World::new();
        library(
            &w,
            "instructions:\n- library/rules/a.md\n- library/rules/b.md\n",
        );
        let p = lp("library/P.yaml");
        let rev = w.view("library/P.yaml").rev;
        let swapped = strings(&["library/rules/b.md", "library/rules/a.md"]);
        let stale =
            refused(
                w.svc
                    .preset_reorder(&p, ListName::Instructions, &swapped, "stale", None),
            );
        assert_eq!(code(&stale), (RefusalClass::Conflict, REV_MISMATCH));
        assert_eq!(stale.1.args["rev"], rev);
        let other = strings(&["library/rules/b.md", "library/rules/c.md"]);
        let mismatch =
            refused(
                w.svc
                    .preset_reorder(&p, ListName::Instructions, &other, &rev, None),
            );
        assert_eq!(
            code(&mismatch),
            (RefusalClass::InvalidArgument, REORDER_ORDER_MISMATCH)
        );
        assert_eq!(
            mismatch.1.args["expected"],
            json!(["library/rules/a.md", "library/rules/b.md"])
        );
        assert_eq!(w.writes(), 0);
        let view = w
            .svc
            .preset_reorder(&p, ListName::Instructions, &swapped, &rev, None)
            .unwrap();
        assert_eq!(
            paths(&view.instructions),
            ["library/rules/b.md", "library/rules/a.md"]
        );
    }

    #[test]
    fn reorder_of_folders_writes_the_normalized_deduplicated_list() {
        let w = World::new();
        let (f1, f2) = (w.folder("f1"), w.folder("f2"));
        let sep = std::path::MAIN_SEPARATOR;
        w.put(
            "P.yaml",
            &preset(&format!("folders:\n- '{f1}{sep}'\n- '{f2}'\n- '{f1}'\n")),
        );
        let rev = w.view("library/P.yaml").rev;
        let order = vec![f2.clone(), format!("{f1}{sep}")];
        let view = w
            .svc
            .preset_reorder(&lp("library/P.yaml"), ListName::Folders, &order, &rev, None)
            .unwrap();
        assert_eq!(folder_paths(&view), [f2.as_str(), f1.as_str()]);
        let text = w.text("P.yaml");
        assert_eq!(text.matches(f1.as_str()).count(), 1, "{text}");
        assert!(!text.contains(&format!("{f1}{sep}")), "{text}");
    }

    // ── setBackend ──────────────────────────────────────────────────────────────────────────────────────────

    #[test]
    fn set_backend_applies_per_item_skips_what_it_cannot_tag_and_keeps_an_entrys_other_keys() {
        let w = World::new();
        library(
            &w,
            "instructions:\n- path: library/rules/a.md\n  note: keep me\nskills:\n- library/skills/s\nsettings:\n- library/S.yaml\n",
        );
        let p = lp("library/P.yaml");
        let named = strings(&[
            "library/rules/a.md",
            "library/skills/s",
            "library/S.yaml",
            "library/gone.md",
        ]);
        let set = w
            .svc
            .preset_set_backend(&p, &named, Some("codex"), None)
            .unwrap();
        assert_eq!(set.applied, ["library/rules/a.md", "library/skills/s"]);
        assert_eq!(
            skips(&set),
            [
                ("library/S.yaml", BACKEND_TAG_NOT_ALLOWED),
                ("library/gone.md", NOT_FOUND)
            ]
        );
        assert_eq!(set.view.instructions[0].backend.as_deref(), Some("codex"));
        assert_eq!(set.view.skills[0].backend.as_deref(), Some("codex"));
        let text = w.text("P.yaml");
        assert!(text.contains("note: keep me"), "{text}");
        let writes = w.writes();
        let unknown = refused(w.svc.preset_set_backend(
            &p,
            &strings(&["library/rules/a.md"]),
            Some("nobody"),
            None,
        ));
        assert_eq!(
            code(&unknown),
            (RefusalClass::InvalidArgument, BACKEND_TAG_UNKNOWN)
        );
        assert_eq!(w.writes(), writes);
    }

    // ── save ────────────────────────────────────────────────────────────────────────────────────────────────

    #[test]
    fn save_refuses_every_mismatch_whole_and_leaves_the_file_untouched() {
        let w = World::new();
        library(&w, "instructions:\n- library/rules/a.md\n");
        w.put("Q.yaml", &preset("presets:\n- library/P.yaml\n"));
        w.put("P2.yaml", &preset(""));
        let f1 = w.folder("f1");
        let sep = std::path::MAIN_SEPARATOR;
        let draft = PresetDraft {
            presets: vec![
                item("engram:/default"),
                item("library/Q.yaml"),
                tagged("library/P2.yaml", "claude"),
            ],
            instructions: vec![
                item("library/a//b"),
                item("library/rules/a.md"),
                tagged("library/rules/a.md", "codex"),
                tagged("library/rules/b.md", "nobody"),
            ],
            skills: vec![item("library/rules/a.md")],
            settings: Vec::new(),
            folders: Some(vec!["rel/x".to_owned(), f1.clone(), format!("{f1}{sep}")]),
        };
        let before = w.bytes("P.yaml");
        let save = refused(w.svc.preset_save(&lp("library/P.yaml"), &draft, None, None));
        assert_eq!(
            code(&save),
            (RefusalClass::InvalidArgument, PRESET_SAVE_REFUSED)
        );
        let listed: Vec<(&str, &str)> = save.1.args["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| {
                (
                    entry["list"].as_str().unwrap(),
                    entry["code"].as_str().unwrap(),
                )
            })
            .collect();
        assert_eq!(
            listed,
            [
                ("presets", BUILTIN_IN_LIBRARY_PRESET),
                ("presets", PRESET_CYCLE),
                ("presets", BACKEND_TAG_NOT_ALLOWED),
                ("instructions", ITEM_PATH_INVALID),
                ("instructions", ITEM_DUPLICATE),
                ("instructions", BACKEND_TAG_UNKNOWN),
                ("skills", WRONG_KIND),
                ("folders", FOLDER_PATH_RELATIVE),
                ("folders", ITEM_DUPLICATE),
            ]
        );
        assert_eq!(w.bytes("P.yaml"), before);
        assert_eq!(w.writes(), 0);
    }

    #[test]
    fn save_keeps_missing_items_and_folders_as_legal_content_and_leaves_other_keys() {
        let w = World::new();
        library(
            &w,
            "description: team rules\nx-owner: me\ninstructions:\n- library/rules/a.md\n",
        );
        let gone = w.base.join("gone").to_string_lossy().into_owned();
        let draft = PresetDraft {
            instructions: vec![item("library/rules/a.md"), item("library/gone.md")],
            folders: Some(vec![gone]),
            ..PresetDraft::default()
        };
        let view = w
            .svc
            .preset_save(&lp("library/P.yaml"), &draft, None, None)
            .unwrap();
        assert_eq!(
            paths(&view.instructions),
            ["library/rules/a.md", "library/gone.md"]
        );
        assert_eq!(view.instructions[1].state.as_ref().unwrap().code, NOT_FOUND);
        assert_eq!(view.folders[0].status, Some(FolderStatus::Missing));
        let text = w.text("P.yaml");
        assert!(
            text.contains("description: team rules") && text.contains("x-owner: me"),
            "{text}"
        );
    }

    #[test]
    fn save_of_a_respelled_item_keeps_its_entry_and_other_keys() {
        // A respelling keeps the item only when it has the same catalog key — where names fold case.
        if !same_path(Path::new("Rules/A.md"), Path::new("rules/a.md")) {
            return;
        }
        let w = World::new();
        library(
            &w,
            "instructions:\n- path: library/rules/a.md\n  note: keep me\n",
        );
        let draft = PresetDraft {
            instructions: vec![item("library/Rules/A.md")],
            ..PresetDraft::default()
        };
        w.svc
            .preset_save(&lp("library/P.yaml"), &draft, None, None)
            .unwrap();
        let text = w.text("P.yaml");
        assert!(
            text.contains("path: library/Rules/A.md") && text.contains("note: keep me"),
            "{text}"
        );
    }

    #[test]
    fn save_with_a_base_rev_other_than_the_file_refuses_rev_mismatch() {
        let w = World::new();
        library(&w, "");
        let draft = PresetDraft {
            instructions: vec![item("library/rules/a.md")],
            ..PresetDraft::default()
        };
        let stale = refused(
            w.svc
                .preset_save(&lp("library/P.yaml"), &draft, Some("old"), None),
        );
        assert_eq!(code(&stale), (RefusalClass::Conflict, REV_MISMATCH));
        assert_eq!(w.writes(), 0);
        let rev = w.view("library/P.yaml").rev;
        w.svc
            .preset_save(&lp("library/P.yaml"), &draft, Some(&rev), None)
            .unwrap();
        assert_eq!(w.writes(), 1);
    }

    // ── the pipeline ────────────────────────────────────────────────────────────────────────────────────────

    #[test]
    fn a_target_gone_from_disk_answers_not_found_and_is_not_recreated() {
        let w = World::new();
        library(&w, "");
        w.put("team/T.yaml", &preset(""));
        w.view("library/P.yaml");
        fs::remove_file(w.lib("P.yaml")).unwrap();
        let gone = refused(w.svc.preset_add(
            &lp("library/P.yaml"),
            &items(&["library/rules/a.md"]),
            None,
            None,
        ));
        assert_eq!(code(&gone), (RefusalClass::NotFound, NOT_FOUND));
        assert!(!w.lib("P.yaml").exists());
        fs::remove_dir_all(w.lib("team")).unwrap();
        let draft = PresetDraft {
            instructions: vec![item("library/rules/a.md")],
            ..PresetDraft::default()
        };
        let parent_gone =
            refused(
                w.svc
                    .preset_save(&lp("library/team/T.yaml"), &draft, None, None),
            );
        assert_eq!(code(&parent_gone), (RefusalClass::NotFound, NOT_FOUND));
        assert!(!w.lib("team").exists(), "the parent is not re-created");
        assert_eq!(w.writes(), 0);
    }

    #[test]
    fn writer_and_reader_refusals_leave_the_file_untouched() {
        let w = World::new();
        library(&w, "big: 1e20\n");
        w.put("B.yaml", "kind: preset\nversion: [\n");
        let before = w.bytes("P.yaml");
        let writer = refused(w.svc.preset_add(
            &lp("library/P.yaml"),
            &items(&["library/rules/a.md"]),
            None,
            None,
        ));
        assert_eq!(
            code(&writer),
            (RefusalClass::InvalidArgument, "preset-write-number-range")
        );
        assert_eq!(w.bytes("P.yaml"), before);
        let reader = refused(w.svc.preset_add(
            &lp("library/B.yaml"),
            &items(&["library/rules/a.md"]),
            None,
            None,
        ));
        assert_eq!(code(&reader), (RefusalClass::InvalidArgument, YAML_SYNTAX));
        assert_eq!(w.writes(), 0);
        let view = refused(w.svc.preset_get(&AssetRef::Library(lp("library/B.yaml"))));
        assert_eq!(code(&view), (RefusalClass::InvalidArgument, YAML_SYNTAX));
    }

    #[test]
    fn bytes_read_that_differ_from_the_catalog_are_applied_also_on_a_refusal() {
        let w = World::new();
        library(&w, "instructions:\n- library/rules/a.md\n");
        let old = w.view("library/P.yaml").rev;
        let outside = preset("instructions:\n- library/rules/a.md\n- library/rules/b.md\n");
        w.put("P.yaml", &outside);
        let p = lp("library/P.yaml");
        let order = strings(&["library/rules/b.md", "library/rules/a.md"]);
        let stale = refused(
            w.svc
                .preset_reorder(&p, ListName::Instructions, &order, &old, None),
        );
        assert_eq!(code(&stale), (RefusalClass::Conflict, REV_MISMATCH));
        let new = sha256_hex(outside.as_bytes());
        assert_eq!(stale.1.args["rev"], new);
        assert_eq!(
            w.catalog_rev("library/P.yaml"),
            Some(new.clone()),
            "the catalog holds the bytes read"
        );
        assert_eq!(
            paths(&w.view("library/P.yaml").instructions),
            ["library/rules/a.md", "library/rules/b.md"]
        );
        w.svc
            .preset_reorder(&p, ListName::Instructions, &order, &new, None)
            .unwrap();
    }

    #[test]
    fn a_read_error_answers_library_read_failed_and_leaves_the_catalog() {
        let w = World::new();
        library(&w, "");
        let rev = w.view("library/P.yaml").rev;
        *lock(&w.disk.fail_read) = Some(w.lib("P.yaml"));
        let failed = refused(w.svc.preset_add(
            &lp("library/P.yaml"),
            &items(&["library/rules/a.md"]),
            None,
            None,
        ));
        assert_eq!(
            code(&failed),
            (RefusalClass::Unavailable, LIBRARY_READ_FAILED)
        );
        assert_eq!(w.catalog_rev("library/P.yaml"), Some(rev));
        assert_eq!(w.writes(), 0);
    }

    #[test]
    fn a_write_failure_answers_library_write_failed_and_applies_only_bytes_that_differ() {
        let w = World::new();
        library(&w, "");
        let rev = w.view("library/P.yaml").rev;
        let seq = w.read_seq("library/P.yaml");
        *lock(&w.disk.fail_write) = Some(w.lib("P.yaml"));
        let add = || {
            w.svc.preset_add(
                &lp("library/P.yaml"),
                &items(&["library/rules/a.md"]),
                None,
                None,
            )
        };
        assert_eq!(
            code(&refused(add())),
            (RefusalClass::Unavailable, LIBRARY_WRITE_FAILED)
        );
        assert_eq!(w.catalog_rev("library/P.yaml"), Some(rev));
        assert_eq!(
            w.read_seq("library/P.yaml"),
            seq,
            "bytes equal to the catalog item are not applied"
        );
        let outside = preset("description: changed\n");
        w.put("P.yaml", &outside);
        assert_eq!(refused(add()).1.code, LIBRARY_WRITE_FAILED);
        assert_eq!(
            w.catalog_rev("library/P.yaml"),
            Some(sha256_hex(outside.as_bytes()))
        );
        assert_eq!(w.text("P.yaml"), outside);
    }

    #[test]
    fn an_edit_that_changes_nothing_writes_nothing_and_keeps_rev() {
        let w = World::new();
        let f1 = w.folder("f1");
        library(
            &w,
            &format!(
                "instructions:\n- path: library/rules/a.md\n  backend: codex\n- library/rules/b.md\nfolders:\n- '{f1}'\n"
            ),
        );
        let view = w.view("library/P.yaml");
        let p = lp("library/P.yaml");
        let svc = &w.svc;
        svc.preset_remove(
            &p,
            &RemoveEntries::Items(strings(&["library/rules/zz.md"])),
            None,
        )
        .unwrap();
        svc.preset_add(
            &p,
            &items(&["library/rules/b.md", "library/gone.md"]),
            None,
            None,
        )
        .unwrap();
        svc.preset_add(&p, &AddEntries::Folders(vec![f1.clone()]), None, None)
            .unwrap();
        svc.preset_set_backend(&p, &strings(&["library/rules/a.md"]), Some("codex"), None)
            .unwrap();
        let order = strings(&["library/rules/a.md", "library/rules/b.md"]);
        svc.preset_reorder(&p, ListName::Instructions, &order, &view.rev, None)
            .unwrap();
        let draft = PresetDraft {
            instructions: vec![
                tagged("library/rules/a.md", "codex"),
                item("library/rules/b.md"),
            ],
            folders: Some(vec![f1]),
            ..PresetDraft::default()
        };
        let after = svc.preset_save(&p, &draft, Some(&view.rev), None).unwrap();
        assert_eq!(w.writes(), 0);
        assert_eq!(after.rev, view.rev);
    }

    /// A line whose link probe fails is not `not-found` but `library-read-failed`, and a path under a file stays
    /// `not-found` — the rule of the answer's problems, so the two agree (ADR-0387).
    #[test]
    fn a_failed_link_probe_marks_the_line_as_the_problems_mark_the_row() {
        let w = World::new();
        library(&w, "instructions:\n- library/gone.md\n");
        let state_and_problem = |view: &PresetView| {
            let state = view.instructions[0].state.clone().expect("a state");
            let [problem] = view.problems.as_slice() else {
                panic!("{:?}", view.problems)
            };
            let row = match problem {
                BlockReason::Missing { reason, .. } | BlockReason::Broken { reason, .. } => {
                    reason.clone()
                }
                other => panic!("{other:?}"),
            };
            (state, row)
        };
        *lock(&w.disk.fail_link) = Some(io::ErrorKind::PermissionDenied);
        let (state, row) = state_and_problem(&w.view("library/P.yaml"));
        assert_eq!(state.code, LIBRARY_READ_FAILED);
        assert_eq!(state.args["path"], "library/gone.md");
        assert_eq!(row.code, LIBRARY_READ_FAILED);

        *lock(&w.disk.fail_link) = Some(io::ErrorKind::NotADirectory);
        let (state, row) = state_and_problem(&w.view("library/P.yaml"));
        assert_eq!(
            (state.code.as_str(), row.code.as_str()),
            (NOT_FOUND, NOT_FOUND)
        );
    }

    #[test]
    fn an_edit_through_a_link_or_on_a_dot_or_temp_name_is_refused_before_any_read() {
        let w = World::new();
        library(&w, "instructions:\n- library/team/x.md\n");
        let outside = w.base.join("outside");
        fs::create_dir_all(&outside).unwrap();
        let theirs = preset("");
        fs::write(outside.join("T.yaml"), &theirs).unwrap();
        fs::write(outside.join("x.md"), "x").unwrap();
        link_dir(&outside, &w.lib("team")).unwrap();
        let view = w.view("library/P.yaml");
        assert_eq!(
            view.instructions[0].state.as_ref().unwrap().code,
            PATH_THROUGH_LINK
        );
        let through = refused(w.svc.preset_add(
            &lp("library/team/T.yaml"),
            &items(&["library/rules/a.md"]),
            None,
            None,
        ));
        assert_eq!(
            code(&through),
            (RefusalClass::InvalidArgument, PATH_THROUGH_LINK)
        );
        let team = w.lib("team");
        assert!(
            !lock(&w.disk.reads)
                .iter()
                .any(|read| read.starts_with(&team)),
            "nothing below the link was read"
        );
        assert_eq!(fs::read_to_string(outside.join("T.yaml")).unwrap(), theirs);
        w.put(".hidden/H.yaml", &preset(""));
        for name in ["library/.hidden/H.yaml", "library/P.yaml.tmp12.3"] {
            let hidden =
                refused(
                    w.svc
                        .preset_add(&lp(name), &items(&["library/rules/a.md"]), None, None),
                );
            assert_eq!(
                code(&hidden),
                (RefusalClass::InvalidArgument, ITEM_PATH_INVALID),
                "{name}"
            );
        }
        let dot = w.lib(".hidden");
        assert!(!lock(&w.disk.reads)
            .iter()
            .any(|read| read.starts_with(&dot)));
        assert_eq!(w.writes(), 0);
    }

    // ── folders ─────────────────────────────────────────────────────────────────────────────────────────────

    #[test]
    fn a_folder_add_is_all_or_nothing_and_names_each_bad_path() {
        let w = World::new();
        library(&w, "");
        let good = w.folder("good");
        let cmd_unsafe = w.folder("a&b");
        let locked = w.folder("locked");
        lock(&w.ops.fixed).insert(
            PathBuf::from(&locked),
            FolderStatus::Unreadable("denied".to_owned()),
        );
        let file = w.base.join("file.txt");
        fs::write(&file, "x").unwrap();
        let file = file.to_string_lossy().into_owned();
        let missing = w.base.join("missing").to_string_lossy().into_owned();
        let given = vec![
            good,
            "rel/x".to_owned(),
            missing.clone(),
            file.clone(),
            cmd_unsafe.clone(),
            locked.clone(),
        ];
        let all = refused(w.svc.preset_add(
            &lp("library/P.yaml"),
            &AddEntries::Folders(given),
            None,
            None,
        ));
        assert_eq!(
            code(&all),
            (RefusalClass::InvalidArgument, PRESET_FOLDERS_REFUSED)
        );
        assert_eq!(
            all.1.args["folders"],
            json!([
                { "path": "rel/x", "code": FOLDER_PATH_RELATIVE },
                { "path": missing, "code": FOLDER_NOT_FOUND, "why": "missing" },
                { "path": file, "code": FOLDER_NOT_FOUND, "why": "not a folder" },
                { "path": cmd_unsafe, "code": FOLDER_PATH_CLI_UNSAFE },
                { "path": locked, "code": FOLDER_UNREADABLE, "why": "denied" },
            ])
        );
        assert_eq!(w.writes(), 0);
    }

    #[test]
    fn a_folder_add_matches_by_normalized_key_skips_what_is_listed_and_stores_normalized() {
        let w = World::new();
        let (f1, f2, f3) = (w.folder("f1"), w.folder("f2"), w.folder("f3"));
        let sep = std::path::MAIN_SEPARATOR;
        w.put(
            "P.yaml",
            &preset(&format!("folders:\n- '{f1}{sep}'\n- '{f3}'\n")),
        );
        let dotted = format!("{f2}{sep}..{sep}f2");
        let given = vec![f1.clone(), format!("{f2}{sep}"), dotted.clone()];
        let anchor = format!("{f1}{sep}.");
        let added = w
            .svc
            .preset_add(
                &lp("library/P.yaml"),
                &AddEntries::Folders(given),
                Some(&anchor),
                None,
            )
            .unwrap();
        assert_eq!(added.applied, [f2.clone()]);
        assert_eq!(
            skips(&added),
            [
                (f1.as_str(), ALREADY_PRESENT),
                (dotted.as_str(), ALREADY_PRESENT)
            ]
        );
        assert_eq!(
            folder_paths(&added.view),
            [f1.as_str(), f2.as_str(), f3.as_str()]
        );
        let text = w.text("P.yaml");
        assert!(
            !text.contains(&format!("{f2}{sep}")),
            "a new folder is stored normalized: {text}"
        );
        assert!(
            text.contains(&format!("{f1}{sep}")),
            "an untouched entry keeps its spelling: {text}"
        );
    }

    #[test]
    fn a_folder_remove_drops_every_spelling_of_the_folder() {
        let w = World::new();
        let (f1, f2) = (w.folder("f1"), w.folder("f2"));
        let sep = std::path::MAIN_SEPARATOR;
        w.put(
            "P.yaml",
            &preset(&format!("folders:\n- '{f1}{sep}'\n- '{f2}'\n- '{f1}'\n")),
        );
        let remove = RemoveEntries::Folders(vec![format!("{f1}{sep}.")]);
        let view = w
            .svc
            .preset_remove(&lp("library/P.yaml"), &remove, None)
            .unwrap();
        assert_eq!(folder_paths(&view), [f2.as_str()]);
        assert!(!w.text("P.yaml").contains(f1.as_str()));
    }

    #[test]
    fn folder_stats_of_an_add_run_outside_the_library_lock() {
        let w = World::new();
        library(&w, "");
        let f1 = w.folder("f1");
        w.view("library/P.yaml");
        w.svc
            .preset_add(
                &lp("library/P.yaml"),
                &AddEntries::Folders(vec![f1.clone()]),
                None,
                None,
            )
            .unwrap();
        assert!(
            w.ops.stats_of(Path::new(&f1)) >= 2,
            "the add's own stat of the folder, then the answer's"
        );
        assert!(
            !w.ops.under_lock.load(Ordering::SeqCst),
            "a stat ran under the library lock"
        );
    }

    #[test]
    fn the_folder_preview_verdict_is_the_add_verdict_and_it_lists_what_loads() {
        let w = World::new();
        library(&w, "");
        let good = w.folder("good");
        fs::write(Path::new(&good).join("AGENTS.md"), "rules").unwrap();
        let cmd_unsafe = w.folder("a&b");
        let missing = w.base.join("missing").to_string_lossy().into_owned();
        let locked = w.folder("locked");
        lock(&w.ops.fixed).insert(
            PathBuf::from(&locked),
            FolderStatus::Unreadable("denied".to_owned()),
        );
        for given in ["rel/x".to_owned(), missing, cmd_unsafe, locked.clone()] {
            let preview = w.svc.preview_folder(&given, None, None).unwrap();
            let add = refused(w.svc.preset_add(
                &lp("library/P.yaml"),
                &AddEntries::Folders(vec![given.clone()]),
                None,
                None,
            ));
            let refusal = preview.refused.expect("the preview refuses it too");
            let mut entry = json!({ "path": refusal.path, "code": refusal.code });
            if let Some(why) = refusal.why {
                entry["why"] = json!(why);
            }
            assert_eq!(add.1.args["folders"], json!([entry]), "{given}");
        }
        let unreadable = w.svc.preview_folder(&locked, None, None).unwrap();
        let refusal = unreadable.refused.expect("refused");
        assert_eq!(
            (refusal.code, refusal.why.as_deref()),
            (FOLDER_UNREADABLE, Some("denied")),
            "ADR-0387"
        );
        let preview = w.svc.preview_folder(&good, None, None).unwrap();
        assert_eq!(preview.refused, None);
        assert_eq!(preview.path.as_deref(), Some(Path::new(&good)));
        let codex_json = preview
            .loads
            .iter()
            .find(|load| load.backend == "codex" && load.mode == AgentOutputFormat::StreamJson)
            .expect("codex json is previewed");
        assert!(!codex_json.predicted);
        assert!(codex_json
            .set
            .loads
            .iter()
            .any(|load| load.path == Path::new("AGENTS.md")));
        let claude: Vec<&PreviewLoad> = preview
            .loads
            .iter()
            .filter(|load| load.backend == "claude")
            .collect();
        assert!(!claude.is_empty() && claude.iter().all(|load| load.predicted));
        let unknown = refused(w.svc.preview_folder(&good, Some("nobody"), None));
        assert_eq!(
            code(&unknown),
            (RefusalClass::InvalidArgument, BACKEND_TAG_UNKNOWN)
        );
    }

    // ── check ───────────────────────────────────────────────────────────────────────────────────────────────

    #[test]
    fn check_judges_and_resolves_the_draft_and_writes_nothing() {
        let w = World::new();
        library(&w, "instructions:\n- library/rules/a.md\n");
        let view = w.view("library/P.yaml");
        let draft = PresetDraft {
            instructions: vec![item("library/gone.md")],
            skills: vec![item("library/rules/a.md")],
            ..PresetDraft::default()
        };
        let checked = w.svc.preset_check(&lp("library/P.yaml"), &draft).unwrap();
        let mismatch = checked
            .problems
            .iter()
            .find_map(|problem| match problem {
                BlockReason::Broken { item, via, reason } if item == "library/rules/a.md" => {
                    Some((via.clone(), reason.clone()))
                }
                _ => None,
            })
            .expect("the mismatch is a problem row");
        assert_eq!(mismatch.0, ["library/P.yaml"]);
        assert_eq!(mismatch.1.code, WRONG_KIND);
        assert_eq!(
            mismatch.1.args,
            json!({ "path": "library/rules/a.md", "list": "skills" })
        );
        let missing = checked
            .problems
            .iter()
            .find_map(|problem| match problem {
                BlockReason::Missing { item, reason, .. } if item == "library/gone.md" => {
                    Some(reason)
                }
                _ => None,
            })
            .expect("the draft's missing item is a problem row");
        assert_eq!(missing.line, None, "a draft item has no file line");
        assert!(checked.save_refuses);
        assert_eq!(w.writes(), 0);
        let after = w.view("library/P.yaml");
        assert_eq!(after.rev, view.rev);
        assert_eq!(paths(&after.instructions), ["library/rules/a.md"]);
    }

    // ── fix round 1 ─────────────────────────────────────────────────────────────────────────────────────────

    #[test]
    fn an_edit_answers_the_committed_state_even_when_the_catalog_moves_after_its_change() {
        let w = World::new();
        let f = w.folder("f");
        library(
            &w,
            &format!("instructions:\n- library/rules/a.md\nfolders:\n- '{f}'\n"),
        );
        w.view("library/P.yaml");
        // At the first stat after the edit's change — the answer's folder stat, or the root stat of a view read again
        //   from the catalog — the written file is kept aside, removed, and a refresh drops the item.
        let written = Arc::new(Mutex::new(None));
        let (file, svc, kept) = (
            w.lib("P.yaml"),
            Arc::downgrade(&w.svc),
            Arc::clone(&written),
        );
        *lock(&w.ops.on_stat) = Some((
            None,
            Box::new(move || {
                *lock(&kept) = Some(fs::read(&file).unwrap());
                fs::remove_file(&file).unwrap();
                svc.upgrade().unwrap().refresh(None);
            }),
        ));
        let added = w
            .svc
            .preset_add(
                &lp("library/P.yaml"),
                &items(&["library/rules/b.md"]),
                None,
                None,
            )
            .unwrap();
        let written = lock(&written)
            .clone()
            .expect("the hook ran after the change");
        assert_eq!(
            w.catalog_rev("library/P.yaml"),
            None,
            "the catalog moved on"
        );
        assert_eq!(
            paths(&added.view.instructions),
            ["library/rules/a.md", "library/rules/b.md"]
        );
        assert_eq!(added.view.rev, sha256_hex(&written), "the committed bytes");
        assert_eq!(folder_paths(&added.view), [f.as_str()]);
    }

    #[test]
    fn save_replaces_each_supplied_list_whole_unmodeled_entries_included() {
        let w = World::new();
        library(
            &w,
            "instructions:\n- library/42\n- library/rules/a.md\nfolders:\n- {a: b}\n",
        );
        let draft = PresetDraft {
            instructions: vec![item("library/rules/a.md")],
            ..PresetDraft::default()
        };
        w.svc
            .preset_save(&lp("library/P.yaml"), &draft, None, None)
            .unwrap();
        let text = w.text("P.yaml");
        assert!(!text.contains("42"), "{text}");
        assert!(
            text.contains("a: b"),
            "an omitted list stays as it is: {text}"
        );
        w.put("E.yaml", &preset("instructions:\n- library/42\n"));
        let writes = w.writes();
        w.svc
            .preset_save(&lp("library/E.yaml"), &PresetDraft::default(), None, None)
            .unwrap();
        assert_eq!(
            w.writes(),
            writes + 1,
            "dropping an unmodeled entry is a change"
        );
        assert!(!w.text("E.yaml").contains("42"));
    }

    #[test]
    fn check_keeps_the_line_problems_of_the_folders_a_save_keeps() {
        let w = World::new();
        library(&w, "folders:\n- rel/x\n");
        let draft = PresetDraft {
            instructions: vec![item("library/rules/a.md")],
            ..PresetDraft::default()
        };
        let checked = w.svc.preset_check(&lp("library/P.yaml"), &draft).unwrap();
        let relative = checked
            .problems
            .iter()
            .find_map(|problem| match problem {
                BlockReason::Broken { reason, .. } if reason.code == FOLDER_PATH_RELATIVE => {
                    Some(reason)
                }
                _ => None,
            })
            .expect("the kept relative folder keeps its line problem");
        assert_eq!(relative.line, Some(4), "the entry's line in the file");
        assert_eq!(checked.folders.len(), 1);
        assert_eq!(checked.folders[0].path, "rel/x");
        assert!(
            !checked.save_refuses,
            "a kept entry's problem is not a reason to refuse the save"
        );
    }

    #[test]
    fn a_preset_listed_under_another_list_does_not_close_a_cycle() {
        let w = World::new();
        library(&w, "");
        w.put("Q.yaml", &preset("instructions:\n- library/P.yaml\n"));
        let added = w
            .svc
            .preset_add(
                &lp("library/P.yaml"),
                &items(&["library/Q.yaml"]),
                None,
                None,
            )
            .unwrap();
        assert_eq!(added.applied, ["library/Q.yaml"]);
        assert!(added.view.problems.iter().any(|problem| matches!(
            problem,
            BlockReason::Broken { item, reason, .. } if item == "library/P.yaml" && reason.code == WRONG_KIND
        )));
    }

    #[test]
    fn the_builtin_default_waits_for_the_first_scan_when_it_lists_a_library_item() {
        let w =
            World::with_default("kind: preset\nversion: 1\ninstructions:\n- library/rules/a.md\n");
        w.put("rules/a.md", "a");
        let view = w
            .svc
            .preset_get(&AssetRef::BuiltIn(BuiltinId::DefaultPreset))
            .unwrap();
        assert_eq!(paths(&view.instructions), ["library/rules/a.md"]);
        assert_eq!(view.instructions[0].state, None, "the first scan found it");
    }

    #[test]
    fn a_folder_add_skips_a_listed_folder_even_when_it_now_fails_its_check() {
        let w = World::new();
        let (f1, cmd_unsafe) = (w.folder("f1"), w.folder("a&b"));
        library(&w, &format!("folders:\n- '{f1}'\n- '{cmd_unsafe}'\n"));
        fs::remove_dir(&f1).unwrap();
        let added = w
            .svc
            .preset_add(
                &lp("library/P.yaml"),
                &AddEntries::Folders(vec![f1.clone(), cmd_unsafe.clone()]),
                None,
                None,
            )
            .unwrap();
        assert_eq!(
            skips(&added),
            [
                (f1.as_str(), ALREADY_PRESENT),
                (cmd_unsafe.as_str(), ALREADY_PRESENT)
            ]
        );
        assert_eq!(w.writes(), 0);
        let missing = w.base.join("missing").to_string_lossy().into_owned();
        let refused_add = refused(w.svc.preset_add(
            &lp("library/P.yaml"),
            &AddEntries::Folders(vec![f1, missing.clone()]),
            None,
            None,
        ));
        assert_eq!(
            refused_add.1.args["folders"],
            json!([{ "path": missing, "code": FOLDER_NOT_FOUND, "why": "missing" }])
        );
    }

    #[test]
    fn a_builtin_only_check_that_keeps_the_folders_waits_for_the_first_scan() {
        let w = World::new();
        let f1 = w.folder("f1");
        w.put("P.yaml", &preset(&format!("folders:\n- '{f1}'\n")));
        let draft = PresetDraft {
            instructions: vec![item("engram:/priming")],
            ..PresetDraft::default()
        };
        let checked = w.svc.preset_check(&lp("library/P.yaml"), &draft).unwrap();
        let shown: Vec<&str> = checked
            .folders
            .iter()
            .map(|folder| folder.path.as_str())
            .collect();
        assert_eq!(shown, [f1.as_str()]);
        assert_eq!(checked.folders[0].status, Some(FolderStatus::Ok));
    }

    #[test]
    fn a_link_probe_that_finds_no_folder_lets_the_read_answer_not_found() {
        let w = World::new();
        library(&w, "");
        w.view("library/P.yaml");
        fs::remove_file(w.lib("P.yaml")).unwrap();
        *lock(&w.disk.fail_link) = Some(io::ErrorKind::NotADirectory);
        let gone = refused(w.svc.preset_add(
            &lp("library/P.yaml"),
            &items(&["library/rules/a.md"]),
            None,
            None,
        ));
        assert_eq!(code(&gone), (RefusalClass::NotFound, NOT_FOUND));
        *lock(&w.disk.fail_link) = Some(io::ErrorKind::PermissionDenied);
        let failed = refused(w.svc.preset_add(
            &lp("library/P.yaml"),
            &items(&["library/rules/a.md"]),
            None,
            None,
        ));
        assert_eq!(
            code(&failed),
            (RefusalClass::Unavailable, LIBRARY_READ_FAILED)
        );
    }

    #[test]
    fn save_without_folders_keeps_the_files_folders() {
        let w = World::new();
        let f1 = w.folder("f1");
        library(&w, &format!("folders:\n- '{f1}'\n"));
        let draft = PresetDraft {
            instructions: vec![item("library/rules/a.md")],
            ..PresetDraft::default()
        };
        let view = w
            .svc
            .preset_save(&lp("library/P.yaml"), &draft, None, None)
            .unwrap();
        assert_eq!(folder_paths(&view), [f1.as_str()]);
        assert!(w.text("P.yaml").contains(f1.as_str()));
    }

    #[test]
    fn an_edit_of_a_file_that_is_not_yaml_answers_wrong_kind() {
        let w = World::new();
        library(&w, "");
        let wrong = refused(w.svc.preset_add(
            &lp("library/rules/a.md"),
            &items(&["library/rules/b.md"]),
            None,
            None,
        ));
        assert_eq!(code(&wrong), (RefusalClass::InvalidArgument, WRONG_KIND));
        assert_eq!(w.writes(), 0);
    }

    #[test]
    fn remove_with_a_spelling_that_is_not_a_reference_refuses_the_whole_call() {
        let w = World::new();
        library(&w, "instructions:\n- library/rules/a.md\n");
        let remove = RemoveEntries::Items(strings(&["library/rules/a.md", "library/a//b"]));
        let invalid = refused(w.svc.preset_remove(&lp("library/P.yaml"), &remove, None));
        assert_eq!(
            code(&invalid),
            (RefusalClass::InvalidArgument, ITEM_PATH_INVALID)
        );
        assert_eq!(w.writes(), 0);
        assert_eq!(
            paths(&w.view("library/P.yaml").instructions),
            ["library/rules/a.md"]
        );
    }

    #[test]
    fn set_backend_with_no_backend_clears_the_tag() {
        let w = World::new();
        library(
            &w,
            "instructions:\n- path: library/rules/a.md\n  backend: codex\n",
        );
        let set = w
            .svc
            .preset_set_backend(
                &lp("library/P.yaml"),
                &strings(&["library/rules/a.md"]),
                None,
                None,
            )
            .unwrap();
        assert_eq!(set.applied, ["library/rules/a.md"]);
        assert_eq!(set.view.instructions[0].backend, None);
        assert!(!w.text("P.yaml").contains("codex"));
    }

    #[test]
    fn the_view_of_an_item_that_is_not_a_preset_answers_wrong_kind() {
        let w = World::new();
        library(&w, "");
        let wrong = refused(
            w.svc
                .preset_get(&AssetRef::Library(lp("library/rules/a.md"))),
        );
        assert_eq!(code(&wrong), (RefusalClass::InvalidArgument, WRONG_KIND));
    }

    #[test]
    fn an_edit_queued_behind_a_root_change_with_the_old_generation_is_stale_and_writes_nothing() {
        let w = World::new();
        library(&w, "");
        w.view("library/P.yaml");
        let generation = w.svc.status().root_generation;
        let other = w.base.join("lib2");
        fs::create_dir_all(&other).unwrap();
        fs::write(other.join("P.yaml"), preset("")).unwrap();
        fs::write(other.join("a.md"), "a").unwrap();
        let (arrived, at_walk) = mpsc::channel();
        let (go, wait_go) = mpsc::channel();
        *lock(&w.disk.hold) = Some((other.clone(), arrived, wait_go));
        let mover = {
            let (svc, other) = (Arc::clone(&w.svc), other.clone());
            thread::spawn(move || svc.set_root(&other, false).map(drop))
        };
        at_walk.recv_timeout(Duration::from_secs(10)).unwrap();
        let editor = {
            let svc = Arc::clone(&w.svc);
            thread::spawn(move || {
                svc.preset_add(
                    &lp("library/P.yaml"),
                    &items(&["library/a.md"]),
                    None,
                    Some(generation),
                )
                .map(drop)
            })
        };
        w.svc
            .gate
            .wait_for("the edit queued behind the root change", |line, _| {
                line.iter()
                    .any(|(kind, entered)| *kind == "change" && !entered)
            });
        go.send(()).unwrap();
        mover.join().unwrap().unwrap();
        let stale = refused(editor.join().unwrap());
        assert_eq!(
            code(&stale),
            (RefusalClass::Conflict, crate::ROOT_GENERATION_STALE)
        );
        assert_eq!(w.writes(), 0);
        assert_eq!(w.text("P.yaml"), preset(""));
        assert_eq!(
            fs::read_to_string(other.join("P.yaml")).unwrap(),
            preset("")
        );
    }

    #[test]
    fn bytes_read_on_a_refusal_update_who_reaches_what() {
        let w = World::new();
        library(&w, "");
        w.put("Q.yaml", &preset(""));
        w.view("library/P.yaml");
        w.put("P.yaml", &preset("presets:\n- library/Q.yaml\n"));
        let order: Vec<String> = Vec::new();
        let stale = refused(w.svc.preset_reorder(
            &lp("library/P.yaml"),
            ListName::Presets,
            &order,
            "old",
            None,
        ));
        assert_eq!(code(&stale), (RefusalClass::Conflict, REV_MISMATCH));
        let reaching: Vec<String> = w.svc.with_read(|state| {
            state
                .catalog
                .presets_reaching(&key(&lp("library/Q.yaml")))
                .iter()
                .map(|item| item.path.as_str().to_owned())
                .collect()
        });
        assert_eq!(reaching, ["library/P.yaml"]);
    }

    #[test]
    fn check_shows_the_problem_of_a_tag_the_save_keeps_from_the_file() {
        let w = World::new();
        library(&w, "settings:\n- path: library/S.yaml\n  backend: codex\n");
        let draft = PresetDraft {
            settings: vec![item("library/S.yaml")],
            ..PresetDraft::default()
        };
        let checked = w.svc.preset_check(&lp("library/P.yaml"), &draft).unwrap();
        assert!(checked.problems.iter().any(|problem| matches!(
            problem,
            BlockReason::Broken { reason, .. } if reason.code == BACKEND_TAG_NOT_ALLOWED
        )));
        assert!(!checked.save_refuses);
        w.svc
            .preset_save(&lp("library/P.yaml"), &draft, None, None)
            .unwrap();
        assert!(
            w.text("P.yaml").contains("backend: codex"),
            "the save keeps the entry as written (ADR-0358)"
        );
    }

    #[test]
    fn an_edit_of_a_file_the_catalog_does_not_hold_as_a_preset_answers_wrong_kind() {
        let w = World::new();
        library(&w, "");
        w.put("F.yaml", "{kind: preset, version: 1}\n");
        let remove = RemoveEntries::Items(strings(&["library/rules/a.md"]));
        let wrong = refused(w.svc.preset_remove(&lp("library/F.yaml"), &remove, None));
        assert_eq!(code(&wrong), (RefusalClass::InvalidArgument, WRONG_KIND));
        let gone = refused(w.svc.preset_get(&AssetRef::Library(lp("library/F.yaml"))));
        assert_eq!(code(&gone), (RefusalClass::NotFound, NOT_FOUND));
        assert_eq!(w.writes(), 0);
    }
}
