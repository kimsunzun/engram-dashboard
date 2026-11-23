//! Step-9b file operations — inherent [`LibraryService`] methods answering crate types: delete (into the item's own mount's
//! `.trash`), restore (back out of it), and move and rename on one volume. Copy and the cross-volume move join them later
//! (an item whose rename crosses volumes fails [`CROSSES_VOLUMES`] until then). Each runs plan → execute → fix
//! references (ADR-0335) under one `Change` hold (ADR-0409 M1).
//!
//! ## Rules (invariants)
//!
//! - **Per item** (ADR-0409): every given path answers on its own ([`Outcome`]) — done, skipped ([`Skip`]: a path given
//!   with a folder above it — only the folder counts —, a move into the folder it is in, a taken name with `onConflict`
//!   skip), a conflict, or failed with a code — and what finished stays finished. Refused
//!   per item before the hold: a spelling that is no library path or holds a dot or temp name (a path under `.trash`
//!   included) `item-path-invalid`, a mount's top `virtual-root-read-only`, the built-in mount `mount-read-only`, a mount
//!   that cannot be read `library-unavailable` (walked again first — ADR-0400 decision 6). Inside the hold, before any
//!   disk step: the mount judged again from the recorded state, the link admission (ADR-0365 — `path-through-link`), and
//!   a no-follow stat (absent `not-found`).
//! - **The wait before the ticket** (ADR-0409 decision 14): delete and restore always may touch `.trash`, a move only with
//!   `overwrite`; those run through [`LibraryService::change_held_at`] with the waiting count — raised just before the
//!   ticket, lowered after the hold or on any earlier exit — and take the trash lock once (a delete at its first item,
//!   a restore before its per-mount loop, a move at its first displaced entry), holding it to the end of their trash
//!   moves only (it is released before any preset file or agent is touched).
//! - **Delete** (ADR-0409 decisions 5–8): the plan names who refers to what goes — the preset files listing a deleted path
//!   (a folder: anything inside it; a lone `SKILL.md` — a file the catalog holds as a skill's, never a folder of that
//!   name: its skill folder or itself — ADR-0339) that are not deleted themselves, and the agents attaching a deleted path
//!   (never for a `SKILL.md`); its digest pins those and the items. Referenced and no `yes` → one
//!   [`DELETE_UNCONFIRMED`] for the call with the references and the two forms; `yes` keeps them (they read as not
//!   found until restored); `yes` + `detachRefs` must carry the plan's digest; `detachRefs` without `yes` is no `yes`
//!   (ADR-0339 M108 — every delete item is of a detachable shape, so a delete never refuses `detachRefs`). The same
//!   judgement runs again inside the hold before any disk step: a plan that changed answers the conflict again with the
//!   fresh plan, nothing written (ADR-0405 decision 1 · ADR-0409 decision 2). In the hold, per mount: `.trash` checked and
//!   made without following once (unusable → every item there `trash-unusable`), one bundle id per operation (a taken id
//!   in a later mount draws a new one there), each item moved by the store's no-replace trash step into
//!   `.trash/<bundle>/<its path in the mount, as the catalog spells it when that is the same entry>`, each mount's record
//!   published after the trash moves (a failed publication leaves the moves standing — the bundle reads by its files), a
//!   bundle that received no root pruned. Then the
//!   catalog range leaves and the parent folder is re-classified from disk ([`LibraryService::reclassify`] — a folder
//!   that lost its `SKILL.md` is no skill at once). With `detachRefs`, kept to the confirmed plan (the hold's
//!   judgement): its preset files that still list a path that went drop those lines (a preset inside an item that did
//!   not go, or trashed with the operation, is not rewritten), then the change number is raised, then its agents that
//!   still attach a deleted preset path are written without it (`ProfileEditPort::set_attachments`, one by one under the
//!   write lock; an agent inserted after the judgement keeps its attachment — an `agent.new` judged before the raise and
//!   inserted after it judges again, ADR-0400 decision 18). After the hold, the `.git/info/exclude` line is checked for
//!   every mount whose `.trash` was made usable (even when its bundle then failed).
//! - **Move and rename** (ADR-0409 decisions 2, 4 and 10, M3, M7, M8): a rename is a move to a new name in the same
//!   folder. Before the ticket, per item: the entrance rules above (the destination's mount too), a move into the
//!   folder the item is in skipped (a rename only to its own spelling — a case-only rename is a rename), a destination
//!   inside the item [`INTO_ITSELF`] (by path key, and for a folder by canonical path and identity —
//!   `mounts::lies_within` —, asked only when the item and the destination folder pass the link admission: a link there
//!   is left to the hold's `path-through-link`, ADR-0414 decision 5), and with `overwrite` the destination mount's
//!   `.trash` checked without following (unusable `trash-unusable`, nothing of the item moved — judged again in the
//!   hold before the first rename). A lone `SKILL.md` (the catalog's `SkillFile`) that the move takes from directly
//!   under its skill folder — into another folder, its own subfolder included, or to a name that is no `SKILL.md` —
//!   follows the delete's `yes` / `detachRefs` / digest rule with [`SKILL_UNCONFIRMED`], judged at the call and again
//!   in the hold; so does a skill folder a merge would leave without its `SKILL.md` (the file moved, an entry left
//!   behind — planned from no-follow listings and stats at the call and again in the hold, only for an item holding a
//!   skill, and detached only when the folder is no skill after the disk step and did not merge away whole); a merge
//!   moves a skill's `SKILL.md` last and keeps it (left `skill-unconfirmed`) when an entry stayed behind that the
//!   hold's plan did not name, unless it would stay anyway (it then answers its own reason); `detachRefs` on a call
//!   with none of these is [`DETACH_REFS_INVALID`] (ADR-0339 M108 · M110). In the hold, per item: both mounts judged
//!   from the recorded state, the item and the destination admitted (ADR-0365), the item stat'ed without following, the
//!   destination folder a real folder (never made — ADR-0335 decision 7), then one no-replace rename — an item whose
//!   source an earlier item of the call displaced or moved another entry onto fails `not-found`. A taken name without
//!   `onConflict` is the item's conflict `name-exists` (a rename never has one); same-name folders merge entry by entry
//!   by the same rules and a source folder emptied so is removed empty-only (one left holding anything stays; one whose
//!   removal fails is named in `left`); `skip` leaves the entry; `keepBoth` takes `name (2)`, `name (3)` …; `overwrite`
//!   moves the entry there into its mount's `.trash` (bundle, record — a path displaced twice goes into a second
//!   bundle; a file onto a folder or the reverse [`KIND_MISMATCH`]) and renames the item onto the name, renaming the
//!   displaced entry back when that fails. The catalog follows in the same hold: the displaced ranges leave, each moved
//!   range is re-keyed by the path-key prefix mapper, what the catalog still holds in a merge's emptied source folder
//!   leaves, the moved files whose kind may have changed and the `SKILL.md` of a skill at a moved folder's new path are
//!   re-read (through no link — ADR-0365), and every folder whose direct children changed is re-classified (a skill
//!   gained or lost at once). Then the references, without a plan confirmation (ADR-0409 decision 4): every preset file
//!   of every readable mount that names a moved path is rewritten whole with the old → new mapping, its pairs applied
//!   in move order (a touched list keeps the first place of each path — ADR-0339; a merge's emptied source folder maps
//!   only itself, and only when its `SKILL.md` became the destination's), with `detachRefs` the confirmed plan's files
//!   also drop the lines naming the gone skill (no agent step); then the change number is raised and the agents are
//!   re-pointed once through `ProfileEditPort::rewrite_preset_paths` over the current roster. The answer carries the
//!   mapping (each row marked whole-range or exact, to apply in order), the files written and skipped and the git
//!   working trees their folders lie in (a folder behind a link: none).
//! - **Stop rules** (ADR-0335, relink's): a per-file refusal skips the file; a read or write failure stops before the
//!   agents (the answer carries it); a file in a mount that cannot be read is skipped `library-unavailable`, a mount
//!   never walked this run is one skipped row at its top. Nothing is journaled: an interrupted detach leaves kept
//!   references, an interrupted move lines that read as not found until one relink of the old path fixes them (for a
//!   merge's exact folder pair a relink re-points by prefix, so entries left under the old folder follow too — rare).
//! - **Restore** (ADR-0409 decisions 5, 6 and 9): per row `{mount, bundle, path}` — a bundle name that does not parse as an id
//!   is foreign (`not-found`); the trash folder and the bundle are checked without following (a check that fails other
//!   than by absence `library-read-failed`); the row is found in the record's existing roots (a staging root
//!   `not-restorable`, a path inside a root `inside-root`, an `other` row `not-restorable`, a root behind a link
//!   `path-through-link`) or, without a readable record, among its files (a temp name `not-restorable`; a bundle's files
//!   are walked once per call). The destination — the same path in the same mount, spelled as the bundle holds it — is
//!   checked name by name without following before anything is made (a link on the way `path-through-link`), missing
//!   parents are made with `create_dir` one name at a time (removed again, empty-only, when the move fails), and the row
//!   is moved back by the no-replace rename: a taken
//!   name is that row's conflict — with keep-both it goes back as `name (2)`, `name (3)`… — and nothing is ever
//!   overwritten. The bundle shell is pruned after each row. The catalog gains each restored range by a folder-scoped
//!   walk after the hold — from the topmost folder the restore made, so those folders are listed too (a scan classifies
//!   it — a restored `SKILL.md` makes its folder a skill), before the answer.
// ADR-0335
// ADR-0339
// ADR-0358
// ADR-0365
// ADR-0400 decisions 6, 18 and 23
// ADR-0405
// ADR-0409
// ADR-0414

use std::collections::{HashMap, HashSet};
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use engram_dashboard_agent::launch::{escape_controls, Reason};
use engram_dashboard_agent::profile::{StoreError, StoreStatus};
use engram_dashboard_agent::profile_edit::{
    Attachments, PresetPathRename, ProfileEditError, ProfileEditPort,
};
use engram_dashboard_agent::types::AgentId;
use engram_dashboard_platform::fs::{path_key, same_path, PathKey};
use serde_json::{json, Value};

use crate::agent_edit::port_refused;
use crate::catalog::{
    key, makes_skill, preset_refs, skill_file_name, within, Catalog, ItemContent, ItemKind,
    ScanItem, ScanResult,
};
use crate::create::NAME_EXISTS;
use crate::edit::{as_read, lib_path, list_mut, ITEM_LISTS, LIBRARY_WRITE_FAILED};
use crate::format::read::ITEM_PATH_INVALID;
use crate::gate::NOT_FOUND;
use crate::launch::LIBRARY_READ_FAILED;
use crate::mounts::{
    admits_inside, in_builtin, is_builtin, lies_within, virtual_path, DiskProbe, Mount, MountName,
    MountPath,
};
use crate::path::{AssetRef, LibPath};
use crate::relink::{RelinkSkip, RelinkStop};
use crate::rename::{lines_digest, GitTree};
use crate::resolve::ref_key;
use crate::store::{
    EntryKind, LibraryStore, NameWalk, ScanEntry, ScanListing, NO_REPLACE_UNSUPPORTED,
};
use crate::trash::{
    below, check_trash_folder, ensure_trash_folder, fallback_rows, make_bundle, prune_shell,
    publish_record, read_record, row_path, trash_unusable, BundleId, FallbackRow, RecordRead,
    RecordRoot, RootState, TrashHeld, TrashUnavailable, TRASH_NOT_RESERVED,
};
use crate::{
    mount_read_only, mount_unavailable, refuse_unusable_now, virtual_root_read_only, Hold,
    LibraryRefusal, LibraryService, MountsNow, RefusalClass, LIBRARY_READ_CAP, PATH_THROUGH_LINK,
};

/// The code of a delete whose items are referenced, refused for the whole call (`Conflict`; `args` = `{plan,
/// plan_digest, forms}` — the referencing preset files and agents, and the two ways to go on: `yes` keeps the
/// references, `yes` + `detachRefs` + that `plan_digest` removes them).
// ADR-0339
// ADR-0409 decision 2
pub const DELETE_UNCONFIRMED: &str = "delete-unconfirmed";

/// The code of a trash row that is never restored (`args` = `{path}`): a staging row, an `other` row, a name with a dot
/// or in base's temp shape.
// ADR-0409 decision 8
pub const NOT_RESTORABLE: &str = "not-restorable";

/// The code of a trash row that lies inside a root of its bundle (`args` = `{path, root}`): the root is the row.
// ADR-0409 decision 8
pub const INSIDE_ROOT: &str = "inside-root";

/// The code of a move whose destination folder is the item itself or lies inside it (`args` = `{path, to}`) — judged by
/// path key and, before the ticket, by canonical path and identity as `mounts::overlap` judges folders.
// ADR-0409
pub const INTO_ITSELF: &str = "into-itself";

/// The code of a move under `overwrite` of a file onto a folder of the same name or the reverse (`args` = `{path,
/// to}`) — that entry's conflict, nothing moved.
// ADR-0409 M8
pub const KIND_MISMATCH: &str = "kind-mismatch";

/// The code of a move or rename that takes a referenced skill's lone `SKILL.md` away from directly under its skill
/// folder, or a merge that would leave a referenced skill folder without its `SKILL.md`, refused for the whole call
/// (`Conflict`; `args` as [`DELETE_UNCONFIRMED`]'s). Also a merge's `left` reason (`args` = `{path}`, no plan) for a
/// `SKILL.md` kept in place because an entry stayed that the hold's plan did not name; a new call plans it afresh.
// ADR-0339 M110
// ADR-0409 decision 2
pub const SKILL_UNCONFIRMED: &str = "skill-unconfirmed";

/// The code of `detachRefs` on a move or rename with no item of a detachable shape — no lone `SKILL.md` leaving its
/// skill folder and no merge leaving a skill folder without its `SKILL.md` (`InvalidArgument`, `args` = `{}`).
// ADR-0339 M108
pub const DETACH_REFS_INVALID: &str = "detach-refs-invalid";

/// The code of an item whose no-replace rename answered that its destination lies on another volume (`args` = `{path,
/// to, why}`) — nothing moved; step-9b U4's cross-volume move takes these items over.
pub const CROSSES_VOLUMES: &str = "crosses-volumes";

/// How many keep-both names (`name (2)` …) a restore or a move tries before it answers the conflict.
const KEEP_BOTH_TRIES: u32 = 1_000;

/// What one item or row of an operation came to.
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome<T> {
    Done(T),
    /// Not acted on, and why.
    Skipped(Skip),
    /// A name collision — nothing replaced.
    Conflict(Reason),
    Failed(Reason),
}

/// Why an item was not acted on.
#[derive(Clone, Debug, PartialEq)]
pub enum Skip {
    /// It lies inside this folder, given in the same call: only the folder counts (P5).
    Inside(String),
    /// A move into the folder it is already in, or a rename to its own spelling (P5).
    InPlace,
    /// Something is at this destination path and `onConflict` said skip.
    Taken(String),
}

/// A row of a mount's trash: the mount, the bundle and the row's path inside the bundle (its path in the mount).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrashAddress {
    pub mount: String,
    pub bundle: String,
    pub path: String,
}

/// One given path of a delete and what came of it — done: where it went.
#[derive(Clone, Debug, PartialEq)]
pub struct DeleteItem {
    pub path: String,
    pub outcome: Outcome<TrashAddress>,
}

/// The answer of [`LibraryService::delete`].
#[derive(Debug)]
pub struct DeleteAnswer {
    /// In the order given.
    pub items: Vec<DeleteItem>,
    /// With `detachRefs`: the deleted paths whose references were removed (the items that went).
    pub detached: Vec<String>,
    /// The preset files written (lines dropped), in walk (path) order.
    pub written: Vec<String>,
    /// The preset files skipped, each with its refusal; a row whose path is a mount's name: that mount could not be read.
    pub skipped: Vec<RelinkSkip>,
    /// The agents whose attachments lost a deleted preset path, in id order.
    pub agents: Vec<AgentId>,
    /// Why the reference fix stopped part-way (the items stay deleted); `None` = it ran to the end or did not run.
    pub stopped: Option<RelinkStop>,
}

/// What a delete — or a move or rename taking a lone `SKILL.md` out of its skill, or a merge leaving a skill folder
/// without its `SKILL.md` — would take references from, as its `CONFLICT` names it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeletePlan {
    /// The preset files listing a deleted path, by path.
    pub presets: Vec<String>,
    /// The agents attaching a deleted preset, `(id, name)` in id order.
    pub agents: Vec<(AgentId, String)>,
    /// What `yes` + `detachRefs` must carry.
    pub plan_digest: String,
}

impl DeletePlan {
    /// The refusal's `args`.
    pub fn to_json(&self) -> Value {
        json!({
            "plan": {
                "presets": self.presets,
                "agents": self.agents.iter().map(|(id, name)| json!({
                    "id": id.to_string(),
                    "name": name,
                })).collect::<Vec<_>>(),
            },
            "plan_digest": self.plan_digest,
            "forms": {
                "keep": { "yes": true },
                "detach": { "yes": true, "detachRefs": true, "plan_digest": self.plan_digest },
            },
        })
    }
}

/// One row of a restore and what came of it — done: the virtual path it is back at.
#[derive(Clone, Debug, PartialEq)]
pub struct RestoreItem {
    pub row: TrashAddress,
    pub outcome: Outcome<String>,
}

/// The answer of [`LibraryService::restore`].
#[derive(Debug)]
pub struct RestoreAnswer {
    /// In the order given.
    pub items: Vec<RestoreItem>,
}

/// How a move answers a name taken at its destination — Explorer's three choices; none = that item's conflict.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OnConflict {
    /// The entry there goes to its mount's `.trash`, then the item takes its name (ADR-0339 decision 1).
    Overwrite,
    /// The item stays where it is.
    Skip,
    /// The item takes the next free keep-both name (`name (2)`, `name (3)` …).
    KeepBoth,
}

/// What a moved item came to.
#[derive(Clone, Debug, PartialEq)]
pub struct Moved {
    /// The virtual path it is at now — a merged folder: the folder it merged into.
    pub to: String,
    /// The entries an overwrite moved into the trash of their mount, in move order.
    pub trashed: Vec<TrashAddress>,
    /// A merge: the entries left where they were, each with why (`name-exists` — skipped or colliding —,
    /// `kind-mismatch`, [`SKILL_UNCONFIRMED`] — a skill's `SKILL.md` kept because an entry stayed behind that the
    /// judgement had not planned —, a failure's code); the folders holding them stay, and so does a source folder whose
    /// removal failed (named here too).
    pub left: Vec<(String, Reason)>,
}

/// One given path of a move or rename and what came of it.
#[derive(Clone, Debug, PartialEq)]
pub struct MoveItem {
    pub path: String,
    pub outcome: Outcome<Moved>,
}

/// The answer of [`LibraryService::move_items`] and [`LibraryService::rename`].
#[derive(Debug)]
pub struct MoveAnswer {
    /// In the order given.
    pub items: Vec<MoveItem>,
    /// Every old → new pair the references followed, as spelled from the virtual root, in move order — each pair maps
    /// what the pairs before it made of a path: each item moved whole and, in a merge, each entry moved whole and each
    /// source folder emptied (removed, or named in `left` when its removal failed) whose `SKILL.md` became the
    /// destination's. `(old, new, exact)`: `exact` =
    /// only `old` itself maps, nothing below it (such an emptied folder).
    pub mapping: Vec<(String, String, bool)>,
    /// With `detachRefs`: the skill folders whose references were removed.
    pub detached: Vec<String>,
    /// The preset files written, in walk (path) order.
    pub written: Vec<String>,
    /// The preset files skipped, each with its refusal; a row whose path is a mount's name: that mount could not be read.
    pub skipped: Vec<RelinkSkip>,
    /// The git working trees the files written or skipped lie in, judged from each file's folder, each `(mount, tree)`
    /// once in list order — they were rewritten whole without asking (ADR-0409 decision 4).
    pub trees: Vec<(String, GitTree)>,
    /// The agents whose attachments were re-pointed, in id order.
    pub agents: Vec<AgentId>,
    /// Why the reference fix stopped part-way (the items stay moved); `None` = it ran to the end or did not run.
    pub stopped: Option<RelinkStop>,
}

#[cfg(test)]
thread_local! {
    /// The bundle id the next delete on this thread wants — a test makes it taken in one mount.
    pub(crate) static WANTED_BUNDLE: std::cell::RefCell<Option<BundleId>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
thread_local! {
    /// Run once on the operating thread after the call's judgement, right before its hold — a test lands a change there.
    pub(crate) static BEFORE_HOLD: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        const { std::cell::RefCell::new(None) };
}

/// [`BEFORE_HOLD`]'s seam; nothing outside tests.
fn before_hold() {
    #[cfg(test)]
    if let Some(hook) = BEFORE_HOLD.with(|hook| hook.borrow_mut().take()) {
        hook();
    }
}

/// The id an operation's bundles want (ADR-0409 decision 7).
fn wanted_bundle() -> BundleId {
    #[cfg(test)]
    if let Some(id) = WANTED_BUNDLE.with(|wanted| wanted.borrow_mut().take()) {
        return id;
    }
    BundleId::new(SystemTime::now())
}

/// A path a delete takes away.
#[derive(Clone, Debug)]
struct Target {
    /// Where it was given.
    at: usize,
    path: LibPath,
    /// A lone `SKILL.md` (a file the catalog holds as a skill's): its skill folder — references to the folder (or the
    /// file) go with it, agents never.
    skill_folder: Option<LibPath>,
}

impl Target {
    /// Whether a reference to `path` is one to what this target takes away.
    fn takes(&self, path: &LibPath) -> bool {
        match &self.skill_folder {
            Some(folder) => key(path) == key(folder) || key(path) == key(&self.path),
            None => within(path, Some(&self.path)),
        }
    }

    /// Whether `path` itself goes with this target (into the trash).
    fn removes(&self, path: &LibPath) -> bool {
        within(path, Some(&self.path))
    }

    /// Whether an agent's attachment at `path` goes with this target.
    fn detaches_agent(&self, path: &LibPath) -> bool {
        self.skill_folder.is_none() && within(path, Some(&self.path))
    }
}

/// Who refers to what a delete takes away, read from the catalog and the registry.
struct Referrers {
    /// The preset files listing a path taken away that are not taken away themselves, by path.
    presets: Vec<LibPath>,
    /// The agents attaching a preset taken away, in id order.
    agents: Vec<AgentId>,
    /// The path key of each path taken away — the digest pins the items too.
    items: Vec<PathKey>,
}

impl Referrers {
    fn of(catalog: &Catalog, attachments: &[(AgentId, Attachments)], targets: &[Target]) -> Self {
        let mut presets: Vec<LibPath> = catalog
            .items()
            .filter(|item| !targets.iter().any(|target| target.removes(&item.path)))
            .filter(|item| match &item.content {
                ItemContent::Preset { doc, .. } => {
                    preset_refs(doc).any(|path| targets.iter().any(|target| target.takes(path)))
                }
                _ => false,
            })
            .map(|item| item.path.clone())
            .collect();
        presets.sort();
        let mut agents: Vec<AgentId> = attachments
            .iter()
            .filter(|(_, attached)| !detached_paths(attached, targets).is_empty())
            .map(|(id, _)| *id)
            .collect();
        agents.sort_unstable();
        let items = targets.iter().map(|target| key(&target.path)).collect();
        Referrers {
            presets,
            agents,
            items,
        }
    }

    fn referenced(&self) -> bool {
        !self.presets.is_empty() || !self.agents.is_empty()
    }

    /// The `plan_digest` (ADR-0409 decision 2): the referencing preset files and agents, and the items taken away — a
    /// digest answered for some items does not confirm a call with more.
    fn digest(&self) -> String {
        lines_digest(
            self.presets
                .iter()
                .map(|path| format!("preset\t{}", path.as_str()))
                .chain(self.agents.iter().map(|id| format!("agent\t{id}")))
                .chain(self.items.iter().map(|item| format!("item\t{item:?}")))
                .collect(),
        )
    }

    /// Whether the delete may go on: nothing referenced, plain `yes`, or `yes` + `detachRefs` carrying this digest
    /// (`detach` without `yes` is no `yes` — ADR-0339 M108).
    fn confirmed(&self, yes: bool, detach: bool, plan_digest: Option<&str>) -> bool {
        !self.referenced()
            || (yes && (!detach || plan_digest.is_some_and(|seen| seen == self.digest())))
    }
}

/// The attached spellings of `attached` that go with `targets` (each once by `same_path`).
fn detached_paths(attached: &Attachments, targets: &[Target]) -> Vec<String> {
    let mut paths: Vec<String> = Vec::new();
    for preset in &attached.presets {
        let Ok(AssetRef::Library(path)) = AssetRef::parse(&preset.path) else {
            continue;
        };
        if targets.iter().any(|target| target.detaches_agent(&path))
            && !paths
                .iter()
                .any(|seen| same_path(Path::new(seen), Path::new(&preset.path)))
        {
            paths.push(preset.path.clone());
        }
    }
    paths
}

/// A target that passed the hold's checks: its mount, its path there and its disk path.
struct Checked {
    target: Target,
    mount: Mount,
    rest: MountPath,
    disk: PathBuf,
}

/// One row of a restore's hold: where it was given, its outcome and — restored — the range the catalog walks.
type Restored = (usize, Outcome<String>, Option<LibPath>);

/// The rows of each bundle a restore read without its record, by `(mount folder, bundle id)` — walked once per call.
type Fallbacks = HashMap<(PathBuf, String), io::Result<Vec<FallbackRow>>>;

/// A restore row found in its bundle: its mount, what it is and its path spelled as the bundle holds it.
struct Found {
    mount: Mount,
    kind: EntryKind,
    path: MountPath,
}

/// What a delete's hold did.
struct Deleted {
    items: Vec<(usize, Outcome<TrashAddress>)>,
    detached: Vec<String>,
    written: Vec<String>,
    skipped: Vec<RelinkSkip>,
    agents: Vec<AgentId>,
    stopped: Option<RelinkStop>,
    /// The mount folders whose trash a step made or put a bundle in — their exclude line is checked after the hold.
    bundled: Vec<PathBuf>,
}

/// One item of a move or rename: where it was given, where it is and where it goes; a lone `SKILL.md` leaving its skill
/// folder carries that folder (from the catalog — ADR-0339 M110), a folder merging into one of its name the `SKILL.md`
/// of each skill folder the merge would leave without it ([`LibraryService::plan_leaves`]).
#[derive(Clone, Debug)]
struct Request {
    at: usize,
    path: LibPath,
    dest: LibPath,
    skill_folder: Option<LibPath>,
    leaves: Vec<LibPath>,
}

impl Request {
    /// The detach target of a lone `SKILL.md` leaving its skill folder: its references to the folder (or the file) go
    /// with it, agents never.
    fn lone(&self) -> Option<Target> {
        self.skill_folder.clone().map(|folder| Target {
            at: self.at,
            path: self.path.clone(),
            skill_folder: Some(folder),
        })
    }

    /// The detach targets of the skill folders a merge would leave without their `SKILL.md`, as [`Request::lone`]'s.
    fn left(&self) -> impl Iterator<Item = Target> + '_ {
        self.leaves.iter().map(|file| Target {
            at: self.at,
            path: file.clone(),
            skill_folder: file.parent(),
        })
    }

    /// Every detach target of this item (ADR-0339 M110).
    fn targets(&self) -> Vec<Target> {
        self.lone().into_iter().chain(self.left()).collect()
    }
}

/// One side of a move: the mount, the path in it and the disk path.
#[derive(Clone, Debug)]
struct Side {
    mount: Mount,
    rest: MountPath,
    disk: PathBuf,
}

impl Side {
    fn lib(&self) -> LibPath {
        virtual_path(&self.mount.name, Some(&self.rest))
    }

    /// The entry `name` inside this folder; `None` = no library name.
    fn child(&self, name: &str) -> Option<Side> {
        let rest = MountPath::parse(&format!("{}/{name}", self.rest.as_str())).ok()?;
        Some(Side {
            disk: self.disk.join(name),
            mount: self.mount.clone(),
            rest,
        })
    }

    /// The path `rest` in this side's mount.
    fn at(&self, rest: MountPath) -> Side {
        Side {
            disk: rest.to_path(&self.mount.folder),
            mount: self.mount.clone(),
            rest,
        }
    }
}

/// An item that passed the hold's checks: both sides and what the source is.
struct Placed {
    request: Request,
    from: Side,
    to: Side,
    kind: EntryKind,
}

/// What placing one entry came to.
enum Placement {
    Moved {
        to: Side,
        /// The entry an overwrite displaced into the trash.
        trashed: Option<TrashAddress>,
    },
    /// A same-name folder merged into; `emptied` = the source folder went (it held nothing more).
    Merged {
        emptied: bool,
    },
    /// Taken, and `onConflict` said skip: what is at this path.
    Skipped(LibPath),
    Conflict(Reason),
    Failed(Reason),
}

/// One old → new pair of a move's disk step — library paths only (a dot or temp name is no item).
#[derive(Clone, Debug)]
struct Pair {
    old: LibPath,
    new: LibPath,
    kind: EntryKind,
    /// A merge's emptied source folder whose `SKILL.md` became the destination's: only the folder itself maps — what
    /// was below it moved by its own pairs or did not move.
    exact: bool,
}

impl Pair {
    /// `path` as this pair re-points it — `old` itself or, unless `exact`, anything below it, by the path-key prefix
    /// mapper (`LibPath::replace_prefix`); `None` = not this pair's.
    fn map(&self, path: &LibPath) -> Option<LibPath> {
        match self.exact {
            true => (key(path) == key(&self.old)).then(|| self.new.clone()),
            false => path.replace_prefix(&self.old, &self.new),
        }
    }
}

/// What a move's disk step did, for the catalog and the references.
#[derive(Default)]
struct Moves {
    /// In move order.
    pairs: Vec<Pair>,
    /// The destination entries an overwrite put in the trash — their catalog range goes.
    displaced: Vec<LibPath>,
    /// The folders of a merge (both sides) — their direct children changed.
    folders: Vec<LibPath>,
    /// The source folders a merge emptied and removed — what the catalog still holds there goes.
    emptied: Vec<LibPath>,
}

impl Moves {
    /// Records `from` → `to` (of `kind`) as a pair when both are library paths.
    fn moved(&mut self, from: &Side, to: &Side, kind: EntryKind, exact: bool) {
        let (old, new) = (from.lib(), to.lib());
        if admits_inside(&old) && admits_inside(&new) {
            self.pairs.push(Pair {
                old,
                new,
                kind,
                exact,
            });
        }
    }

    /// Whether `path` is or lies inside an entry this step displaced or moved whole into place — no longer what the
    /// checks before the step saw there.
    fn changed(&self, path: &LibPath) -> bool {
        self.displaced
            .iter()
            .chain(
                self.pairs
                    .iter()
                    .filter(|pair| !pair.exact)
                    .map(|pair| &pair.new),
            )
            .any(|at| within(path, Some(at)))
    }
}

/// An operation's trash side under its hold: the trash lock taken at the first entry it trashes and kept to
/// [`Trashing::finish`] (ADR-0412 decision 5); per mount its `.trash` checked and made once and the bundles made in it.
struct Trashing<'h, 'a> {
    hold: &'h Hold<'a>,
    /// The trash lock and the operation's bundle id (ADR-0409 decision 7), both taken at the first entry.
    held: Option<(TrashHeld<'h>, BundleId)>,
    /// Per mount: its bundles with the roots put in each, or why its trash cannot be used.
    mounts: Vec<(Mount, Result<Vec<(BundleId, Vec<RecordRoot>)>, Reason>)>,
}

impl<'h, 'a> Trashing<'h, 'a> {
    fn new(hold: &'h Hold<'a>) -> Self {
        Trashing {
            hold,
            held: None,
            mounts: Vec::new(),
        }
    }

    /// Moves the entry `rest` of `mount` (at `disk`; `path` names it in a refusal) into the mount's trash: `.trash`
    /// checked and made without following at the mount's first entry (unusable → `trash-unusable` — ADR-0409 decision
    /// 5), the first of the mount's bundles that holds no root at this path (one made when none does — a path displaced
    /// twice keeps both entries; a taken id draws another), then the store's no-replace trash step. Answers the trash
    /// row and its bundle.
    fn put(
        &mut self,
        store: &dyn LibraryStore,
        mount: &Mount,
        rest: &MountPath,
        disk: &Path,
        path: &LibPath,
    ) -> Result<(TrashAddress, BundleId), Reason> {
        if self.held.is_none() {
            let held = match self.hold.take_trash() {
                Ok(held) => held,
                Err(TrashUnavailable::NotReserved) => {
                    return Err(reason(TRASH_NOT_RESERVED, json!({ "path": path.as_str() })))
                }
                Err(TrashUnavailable::AlreadyHeld) => {
                    unreachable!("an operation takes the trash lock once")
                }
            };
            self.held = Some((held, wanted_bundle()));
        }
        let (held, wanted) = self.held.as_ref().expect("taken above");
        let slot = match self
            .mounts
            .iter()
            .position(|(held_by, _)| held_by.name.key() == mount.name.key())
        {
            Some(slot) => slot,
            None => {
                let ready = ensure_trash_folder(store, &mount.folder, held)
                    .map(|()| Vec::new())
                    .map_err(|unusable| {
                        tracing::warn!(
                            mount = mount.name.as_str(),
                            "library trash step: trash unusable: {}",
                            escape_controls(&unusable.why)
                        );
                        trash_unusable(&mount.name, &unusable)
                    });
                self.mounts.push((mount.clone(), ready));
                self.mounts.len() - 1
            }
        };
        let bundles = self.mounts[slot]
            .1
            .as_mut()
            .map_err(|unusable| unusable.clone())?;
        let free = bundles.iter().position(|(_, roots)| {
            !roots
                .iter()
                .any(|root| same_path(Path::new(root.path.as_str()), Path::new(rest.as_str())))
        });
        let at = match free {
            Some(at) => at,
            None => {
                let id = make_bundle(store, &mount.folder, wanted, held).map_err(|e| {
                    reason(
                        LIBRARY_WRITE_FAILED,
                        json!({ "path": path.as_str(), "why": e.to_string() }),
                    )
                })?;
                bundles.push((id, Vec::new()));
                bundles.len() - 1
            }
        };
        let id = bundles[at].0.clone();
        store
            .trash(&mount.folder, disk, &id.bundle())
            .map_err(|e| move_failed(path, &e))?;
        bundles[at].1.push(RecordRoot {
            path: rest.clone(),
            staging: None,
        });
        let address = TrashAddress {
            mount: mount.name.as_str().to_owned(),
            bundle: id.as_str().to_owned(),
            path: rest.as_str().to_owned(),
        };
        Ok((address, id))
    }

    /// Moves the entry [`Trashing::put`] put in bundle `id` back to `to` (the rename onto its place failed): its root
    /// leaves the bundle's roots and the bundle shell above it is pruned — a bundle gone so is made again at need.
    fn take_back(&mut self, store: &dyn LibraryStore, to: &Side, id: &BundleId) -> io::Result<()> {
        let (held, _) = self
            .held
            .as_ref()
            .expect("a trashed entry moved under the trash lock");
        let folder = &to.mount.folder;
        store.rename(&row_path(id, &to.rest).to_path(folder), &to.disk)?;
        let bundles = self
            .mounts
            .iter_mut()
            .find(|(mount, _)| mount.name.key() == to.mount.name.key())
            .and_then(|(_, bundles)| bundles.as_mut().ok())
            .expect("the mount the entry was trashed in");
        let at = bundles
            .iter()
            .position(|(bundle, _)| bundle.as_str() == id.as_str())
            .expect("the bundle the entry went into");
        bundles[at].1.retain(|root| root.path != to.rest);
        match prune_shell(store, folder, id, Some(&to.rest), held) {
            Ok(true) => {
                bundles.remove(at);
            }
            Ok(false) => {}
            Err(e) => tracing::debug!(
                bundle = id.as_str(),
                "library move: bundle shell left after a put-back: {e}"
            ),
        }
        Ok(())
    }

    /// Ends the trash side: each bundle's record published, a bundle that kept no root pruned (ADR-0409 decisions 7
    /// and 8), then the trash lock released. Answers the mount folders whose `.trash` was made usable — their exclude
    /// line is checked after the hold.
    fn finish(self, store: &dyn LibraryStore) -> Vec<PathBuf> {
        let Some((held, _)) = &self.held else {
            return Vec::new();
        };
        let mut bundled = Vec::new();
        for (mount, bundles) in &self.mounts {
            let Ok(bundles) = bundles else {
                continue;
            };
            bundled.push(mount.folder.clone());
            for (id, roots) in bundles {
                if roots.is_empty() {
                    // ADR-0409 decision 7: a bundle that received no root goes again, empty-only.
                    if let Err(e) = prune_shell(store, &mount.folder, id, None, held) {
                        tracing::warn!(
                            bundle = id.as_str(),
                            "library trash step: empty bundle left: {e}"
                        );
                    }
                } else if let Err(e) = publish_record(store, &mount.folder, id, roots, held) {
                    // The moves stand; the bundle reads by its files.
                    tracing::warn!(
                        bundle = id.as_str(),
                        "library trash step: bundle record not published: {e}"
                    );
                }
            }
        }
        bundled
    }
}

/// A move's disk step as it goes.
struct Step<'h, 'a> {
    trashing: Trashing<'h, 'a>,
    moves: Moves,
    /// The `SKILL.md` of each skill folder the hold's judgement confirmed a merge may leave behind (`Request.leaves`).
    leaves: Vec<PathKey>,
    /// The current item's merge: the entries left, and those an overwrite displaced.
    left: Vec<(String, Reason)>,
    trashed: Vec<TrashAddress>,
}

/// What a move's hold did.
struct Relocated {
    items: Vec<(usize, Outcome<Moved>)>,
    mapping: Vec<(String, String, bool)>,
    detached: Vec<String>,
    written: Vec<String>,
    skipped: Vec<RelinkSkip>,
    agents: Vec<AgentId>,
    stopped: Option<RelinkStop>,
    bundled: Vec<PathBuf>,
}

impl LibraryService {
    /// Deletes `paths` into each one's own mount's `.trash` (module header). `yes` confirms a delete of referenced items
    /// (references kept); with `detach_refs` and the `plan_digest` of the plan it answered, their references are removed
    /// too. Answers per item, and what the reference fix wrote, skipped and why it stopped.
    ///
    /// Refusals of the whole call, nothing applied: no first scan applied `Unavailable` · `library-unavailable`; items
    /// referenced without `yes`, or `yes` + `detach_refs` without this plan's `plan_digest`, or a plan that changed before
    /// the hold `Conflict` · [`DELETE_UNCONFIRMED`]; with `detach_refs`, agents to change while the agent store is
    /// read-only — the store's refusal — or while the port cell is empty `Internal` · `profile-edit-unavailable`; a
    /// stale `root_generation` `Conflict` · `root-generation-stale`. Everything else answers per item.
    // ADR-0339
    // ADR-0409
    pub fn delete(
        &self,
        paths: &[&str],
        yes: bool,
        detach_refs: bool,
        plan_digest: Option<&str>,
        root_generation: Option<u64>,
    ) -> Result<DeleteAnswer, LibraryRefusal> {
        let mut outcomes: Vec<Option<Outcome<TrashAddress>>> = vec![None; paths.len()];
        let mut targets: Vec<Target> = Vec::new();
        for (at, given) in paths.iter().enumerate() {
            match deletable(given) {
                Ok(path) => targets.push(Target {
                    at,
                    path,
                    skill_folder: None,
                }),
                Err(reason) => outcomes[at] = Some(Outcome::Failed(reason)),
            }
        }
        let inside = {
            let given: Vec<(usize, &LibPath)> = targets
                .iter()
                .map(|target| (target.at, &target.path))
                .collect();
            nested(&given)
        };
        for (at, folder) in inside {
            outcomes[at] = Some(Outcome::Skipped(Skip::Inside(folder)));
        }
        targets.retain(|target| outcomes[target.at].is_none());
        if !targets.is_empty() {
            self.first_scan_join()
                .map_err(LibraryRefusal::unavailable)?;
        }
        // ADR-0400 decision 6: nothing is moved in a mount that cannot be read — its disk is not touched under the lock.
        targets.retain(|target| match self.refuse_unusable_mount(&target.path) {
            Ok(()) => true,
            Err(refusal) => {
                outcomes[target.at] = Some(Outcome::Failed(reason_of(refusal)));
                false
            }
        });
        self.gate
            .with_read(|state| classify(&state.catalog, &mut targets));
        let mut deleted = None;
        if !targets.is_empty() {
            if yes && detach_refs {
                // Relink's rule: the files of a mount that cannot be read are skipped by its recorded state.
                let names: Vec<MountName> = self.gate.with_read(|state| {
                    state
                        .mounts
                        .iter()
                        .map(|mount| mount.name.clone())
                        .collect()
                });
                self.rewalk_unusable(&names);
            }
            let plan = self.delete_plan(&targets);
            if !plan.0.confirmed(yes, detach_refs, plan_digest) {
                return Err(unconfirmed(&plan.1));
            }
            let detach = yes && detach_refs && plan.0.referenced();
            let port = match detach && !plan.0.agents.is_empty() {
                true => {
                    // ADR-0291 · ADR-0379: a read-only agent store refuses before anything is moved.
                    if let StoreStatus::ReadOnly(refusal) = self.values.registry.store_status() {
                        return Err(LibraryRefusal::Store(StoreError::Refused(refusal)));
                    }
                    Some(self.profile_edit()?)
                }
                false => None,
            };
            before_hold();
            let done = self.change_held_at(root_generation, true, |catalog, mounts, hold| {
                let mut touched = Vec::new();
                let answer = self.delete_in(
                    &targets,
                    (yes, detach_refs, plan_digest),
                    detach,
                    port,
                    catalog,
                    mounts,
                    hold,
                    &mut touched,
                );
                (answer, touched)
            })?;
            match done {
                Err(LibraryRefusal::Refused { reason, .. })
                    if reason.code == DELETE_UNCONFIRMED =>
                {
                    // The fresh plan classifies as the hold did.
                    self.gate
                        .with_read(|state| classify(&state.catalog, &mut targets));
                    return Err(unconfirmed(&self.delete_plan(&targets).1));
                }
                done => deleted = Some(done?),
            }
        }
        let mut answer = DeleteAnswer {
            items: Vec::new(),
            detached: Vec::new(),
            written: Vec::new(),
            skipped: Vec::new(),
            agents: Vec::new(),
            stopped: None,
        };
        if let Some(deleted) = deleted {
            for folder in deleted.bundled {
                self.exclude_trash(folder);
            }
            for (at, outcome) in deleted.items {
                outcomes[at] = Some(outcome);
            }
            answer.detached = deleted.detached;
            answer.written = deleted.written;
            answer.skipped = deleted.skipped;
            answer.agents = deleted.agents;
            answer.stopped = deleted.stopped;
        }
        answer.items = paths
            .iter()
            .zip(outcomes)
            .map(|(path, outcome)| DeleteItem {
                path: (*path).to_owned(),
                outcome: outcome.expect("every item answered"),
            })
            .collect();
        let done = answer
            .items
            .iter()
            .filter(|item| matches!(item.outcome, Outcome::Done(_)))
            .count();
        tracing::info!(
            items = answer.items.len(),
            done,
            written = answer.written.len(),
            stopped = answer.stopped.is_some(),
            "library delete"
        );
        Ok(answer)
    }

    /// The referrers of `targets` and the plan a refusal shows, from one read lock.
    fn delete_plan(&self, targets: &[Target]) -> (Referrers, DeletePlan) {
        let referrers = self.gate.with_read(|state| {
            Referrers::of(&state.catalog, &self.values.registry.attachments(), targets)
        });
        let names: Vec<(AgentId, String)> = self
            .values
            .registry
            .list()
            .into_iter()
            .filter(|profile| referrers.agents.contains(&profile.id))
            .map(|profile| (profile.id, profile.canonical_name_when_live()))
            .collect();
        let mut agents = names;
        agents.sort_by_key(|(id, _)| *id);
        let plan = DeletePlan {
            presets: referrers
                .presets
                .iter()
                .map(|path| path.as_str().to_owned())
                .collect(),
            agents,
            plan_digest: referrers.digest(),
        };
        (referrers, plan)
    }

    /// The body of a delete's hold (module header). `asked` = `(yes, detach_refs, plan_digest)` as the call gave them;
    /// `detach` = the call removes references; `port` = the port checked before the hold when agents were to change.
    #[allow(clippy::too_many_arguments)]
    fn delete_in(
        &self,
        targets: &[Target],
        asked: (bool, bool, Option<&str>),
        detach: bool,
        port: Option<std::sync::Arc<dyn ProfileEditPort>>,
        catalog: &mut Catalog,
        mounts: &MountsNow,
        hold: &Hold,
        touched: &mut Vec<PathKey>,
    ) -> Result<Deleted, LibraryRefusal> {
        // ADR-0409 decision 2 · ADR-0405 decision 1: judged again before any disk step; what it names is the confirmed
        //   plan the reference fix keeps to.
        let (yes, detach_refs, plan_digest) = asked;
        // ADR-0339: the lone-`SKILL.md` kind is read again from the hold's catalog — a restore or scan since the call's
        //   plan may have made the file a skill's.
        let mut targets = targets.to_vec();
        classify(catalog, &mut targets);
        let targets = &targets[..];
        let judged = Referrers::of(catalog, &self.values.registry.attachments(), targets);
        if !judged.confirmed(yes, detach_refs, plan_digest) {
            return Err(LibraryRefusal::new(
                RefusalClass::Conflict,
                DELETE_UNCONFIRMED,
                json!({}),
            ));
        }
        let mut items: Vec<(usize, Outcome<TrashAddress>)> = Vec::new();
        let mut checked: Vec<Checked> = Vec::new();
        for target in targets {
            match self.check_target(target, catalog, mounts) {
                Ok(item) => checked.push(item),
                Err(reason) => items.push((target.at, Outcome::Failed(reason))),
            }
        }
        let mut deleted = Deleted {
            items: Vec::new(),
            detached: Vec::new(),
            written: Vec::new(),
            skipped: Vec::new(),
            agents: Vec::new(),
            stopped: None,
            bundled: Vec::new(),
        };
        // ADR-0409 decision 14: the trash lock is taken at the first item and released before the catalog and the
        //   references.
        let mut trashing = Trashing::new(hold);
        let mut gone: Vec<Checked> = Vec::new();
        for item in checked {
            let store = &*self.store;
            match trashing.put(
                store,
                &item.mount,
                &item.rest,
                &item.disk,
                &item.target.path,
            ) {
                Ok((address, _)) => {
                    items.push((item.target.at, Outcome::Done(address)));
                    gone.push(item);
                }
                Err(why) => items.push((item.target.at, Outcome::Failed(why))),
            }
        }
        deleted.bundled = trashing.finish(&*self.store);
        // The catalog follows the disk: the ranges leave, then their folders are judged again from disk.
        let at = hold.seq();
        for item in &gone {
            touched.extend(catalog.remove_range(&item.target.path, at));
        }
        let mut parents: Vec<(Mount, MountPath)> = Vec::new();
        for item in &gone {
            let Some(parent) = item.rest.parent() else {
                continue;
            };
            let seen = parents.iter().any(|(mount, folder)| {
                mount.name.key() == item.mount.name.key()
                    && path_key(Path::new(folder.as_str())) == path_key(Path::new(parent.as_str()))
            });
            if !seen {
                parents.push((item.mount.clone(), parent));
            }
        }
        for (mount, folder) in &parents {
            match self.reclassify(catalog, mount, folder, &|| hold.seq()) {
                Ok(removed) => touched.extend(removed),
                Err(e) => tracing::warn!(
                    folder = virtual_path(&mount.name, Some(folder)).as_str(),
                    "library delete: folder not re-classified; the next scan does it: {e}"
                ),
            }
        }
        deleted.items = items;
        if detach && !gone.is_empty() {
            let gone: Vec<Target> = gone.into_iter().map(|item| item.target).collect();
            self.detach_refs(
                &gone,
                &judged,
                port,
                catalog,
                mounts,
                hold,
                touched,
                &mut deleted,
            );
        }
        Ok(deleted)
    }

    /// The path a delete acts on and its disk path: `spelled` (the catalog's spelling of `given`) when it passes the link
    /// admission too and the store's identities say both name one disk entry, else `given` (admitted by the caller as
    /// `given_disk`). Two entries can share one catalog key (a case-sensitive folder on Windows, a volume whose case table
    /// differs on rare characters), and a delete must act on the entry it was given. No identity (an error) keeps `given`.
    /// Identities follow links, so two links to one target spelled apart read as one entry (not handled).
    fn same_entry_spelling(
        &self,
        mounts: &MountsNow,
        given: &LibPath,
        given_disk: PathBuf,
        spelled: LibPath,
    ) -> (LibPath, PathBuf) {
        if spelled.as_str() == given.as_str() {
            return (spelled, given_disk);
        }
        // ADR-0365: the link-following identity probe runs only on spellings the no-follow admission passed.
        let Ok(spelled_disk) = self.admit(mounts, &spelled) else {
            return (given.clone(), given_disk);
        };
        let same = matches!(
            (self.store.identity(&given_disk), self.store.identity(&spelled_disk)),
            (Ok(a), Ok(b)) if a == b
        );
        match same {
            true => (spelled, spelled_disk),
            false => (given.clone(), given_disk),
        }
    }

    /// The hold's checks of one target before any disk step (module header): its mount from the recorded state, the
    /// link admission (of the given spelling first) and a no-follow stat. Its path in the mount is spelled as the catalog
    /// spells it ([`catalog_spelling`]) when that is the same disk entry ([`LibraryService::same_entry_spelling`]) — the
    /// name its trash row and record root keep, and a restore brings back.
    fn check_target(
        &self,
        target: &Target,
        catalog: &Catalog,
        mounts: &MountsNow,
    ) -> Result<Checked, Reason> {
        refuse_unusable_now(mounts, &target.path).map_err(reason_of)?;
        let given_disk = self.admit(mounts, &target.path).map_err(reason_of)?;
        let (spelled, disk) = self.same_entry_spelling(
            mounts,
            &target.path,
            given_disk,
            catalog_spelling(catalog, &target.path),
        );
        let Some((mount, Some(rest))) = mounts.locate(&spelled) else {
            return Err(not_found_reason(target.path.as_str()));
        };
        match self.store.stat(&disk) {
            Ok(_) => Ok(Checked {
                target: target.clone(),
                mount: mount.clone(),
                rest,
                disk,
            }),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                Err(not_found_reason(target.path.as_str()))
            }
            Err(e) => Err(reason(
                LIBRARY_READ_FAILED,
                json!({ "path": target.path.as_str(), "why": e.to_string() }),
            )),
        }
    }

    /// Re-classifies `folder` (`rest` in `mount`) from one no-follow listing after its direct children changed
    /// (ADR-0409 M3): a skill whose folder no longer holds a file the scan's classifier takes for its `SKILL.md` is no
    /// skill — its item goes at a number drawn from `seq`; a folder that holds one and is no skill in the catalog becomes
    /// one (that file read as a scan reads it — [`LibraryService::reread_file`]). A folder gone meanwhile has nothing to
    /// classify. Answers the keys touched; `Err` = the listing or the read failed and the catalog keeps what it knew.
    fn reclassify(
        &self,
        catalog: &mut Catalog,
        mount: &Mount,
        rest: &MountPath,
        seq: &dyn Fn() -> u64,
    ) -> io::Result<Vec<PathKey>> {
        let folder = virtual_path(&mount.name, Some(rest));
        let skill = catalog
            .get(&key(&folder))
            .is_some_and(|item| item.kind == ItemKind::Skill);
        let listed = match self.store.list(&rest.to_path(&mount.folder)) {
            Ok(listed) => listed,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e),
        };
        let file = listed
            .iter()
            .filter(|(_, kind)| *kind == EntryKind::File)
            .filter_map(|(name, _)| name.to_str())
            .find(|name| makes_skill(name, &folder))
            .map(str::to_owned);
        match (skill, file) {
            (true, None) => {
                if !catalog.remove(&key(&folder), seq()) {
                    return Ok(Vec::new());
                }
                tracing::info!(
                    folder = folder.as_str(),
                    "library folder is no skill any more"
                );
                Ok(vec![key(&folder)])
            }
            (false, Some(name)) => {
                let file = MountPath::parse(&format!("{}/{name}", rest.as_str()))
                    .expect("a listed name the classifier takes for a SKILL.md is a valid name");
                let touched = self.reread_file(catalog, mount, &file, seq)?;
                tracing::info!(folder = folder.as_str(), "library folder became a skill");
                Ok(touched)
            }
            _ => Ok(Vec::new()),
        }
    }

    /// Re-reads the file `rest` in `mount` as a scan reads it (`ScanResult::build` over a one-file listing) and puts
    /// what it makes in place of what the catalog holds at its key: a kind changes with the name or the folder (`x.md`
    /// ↔ `SKILL.md`, a `SKILL.md` at a mount's top), and a skill's name and `folder` come from its `SKILL.md` and its
    /// folder's name (ADR-0409 M3). A `SKILL.md` makes its folder's skill item too. Answers the keys touched; `Err` =
    /// the file could not be read, or a name from the mount's folder down to it is a link (ADR-0365), and the catalog
    /// keeps what it had.
    fn reread_file(
        &self,
        catalog: &mut Catalog,
        mount: &Mount,
        rest: &MountPath,
        seq: &dyn Fn() -> u64,
    ) -> io::Result<Vec<PathKey>> {
        // ADR-0365: `tag` and `read` follow links — a path through one is left to the next scan.
        if let Some(link) = self.store.first_link(&mount.folder, rest)? {
            return Err(io::Error::other(format!(
                "{PATH_THROUGH_LINK}: {} is a link",
                link.as_str()
            )));
        }
        let read_seq = seq();
        let tag = self.store.tag(&rest.to_path(&mount.folder))?;
        let listing = ScanListing {
            root: mount.folder.clone(),
            files: vec![ScanEntry {
                rel: rest.clone(),
                tag,
                seq: read_seq,
            }],
            ..ScanListing::default()
        };
        let read = |path: &Path| self.store.read(path, LIBRARY_READ_CAP);
        let result = ScanResult::build(
            listing,
            &mount.name,
            Some(rest),
            &HashMap::new(),
            &read,
            &*self.values.backends,
            SystemTime::now(),
        );
        if let Some((_, why)) = result.errors.into_iter().next() {
            return Err(io::Error::other(why));
        }
        let at = key(&virtual_path(&mount.name, Some(rest)));
        let mut touched = Vec::new();
        if catalog.remove(&at, read_seq) {
            touched.push(at);
        }
        let items = result
            .entries
            .into_iter()
            .filter_map(|entry| match entry {
                ScanItem::Fresh(item) => Some(item),
                ScanItem::Unchanged(..) => None,
            })
            .collect();
        touched.extend(catalog.apply_items(items, true));
        Ok(touched)
    }

    /// The reference fix of a delete with `detachRefs` (module header) over the targets that went, kept to what the
    /// hold's judgement (`judged`, the confirmed plan) named: of its preset files those that still list a path that went,
    /// of its agents those that still attach a deleted preset path.
    #[allow(clippy::too_many_arguments)]
    fn detach_refs(
        &self,
        gone: &[Target],
        judged: &Referrers,
        port: Option<std::sync::Arc<dyn ProfileEditPort>>,
        catalog: &mut Catalog,
        mounts: &MountsNow,
        hold: &Hold,
        touched: &mut Vec<PathKey>,
        deleted: &mut Deleted,
    ) {
        deleted.detached = gone
            .iter()
            .map(|target| target.path.as_str().to_owned())
            .collect();
        // ADR-0409 decision 2: a preset inside a target that did not go was no referrer in the confirmed plan — it is not
        //   rewritten now. The ranges that went left the catalog already.
        let still = Referrers::of(catalog, &[], gone).presets;
        let files: Vec<LibPath> = judged
            .presets
            .iter()
            .filter(|path| still.contains(path))
            .cloned()
            .collect();
        let lists = |_: &LibPath, doc: &crate::format::PresetDoc| dropped(doc, gone);
        let walked = self.walk_refs(
            &files,
            lists,
            catalog,
            mounts,
            hold,
            touched,
            &mut deleted.written,
            &mut deleted.skipped,
        );
        if let Err(stop) = walked {
            deleted.stopped = Some(stop);
            return;
        }
        // ADR-0400 decision 18: raised before the agents pass, so an `agent.new` judged before it and inserted after it
        //   judges again. It cannot move earlier: a late merge takes a moved number for "the disk change is done".
        hold.raise();
        // ADR-0409 decision 2: only the agents the confirmed plan named — one inserted since the judgement (an
        //   `agent.new` before the raise) keeps its attachment, which reads as not found. Each is written whole: every
        //   writer of an existing agent's attachments queues under this write lock.
        let mut next: Vec<(AgentId, Attachments)> = Vec::new();
        for (id, attached) in self.values.registry.attachments() {
            if !judged.agents.contains(&id) {
                continue;
            }
            let paths = detached_paths(&attached, gone);
            if paths.is_empty() {
                continue;
            }
            let presets = attached
                .presets
                .into_iter()
                .filter(|preset| {
                    !paths
                        .iter()
                        .any(|path| same_path(Path::new(path), Path::new(&preset.path)))
                })
                .collect();
            next.push((
                id,
                Attachments {
                    default_preset: attached.default_preset,
                    presets,
                },
            ));
        }
        if next.is_empty() {
            return;
        }
        next.sort_by_key(|(id, _)| *id);
        // Taken before the hold whenever the call's plan named agents (the confirmed plan names the same ones).
        let port = match port.map_or_else(|| self.profile_edit(), Ok) {
            Ok(port) => port,
            Err(refusal) => {
                deleted.stopped = Some(RelinkStop {
                    path: None,
                    refusal,
                });
                return;
            }
        };
        for (id, attachments) in next {
            match port.set_attachments(id, attachments) {
                Ok(()) => deleted.agents.push(id),
                // Removed since the registry read: nothing is left to detach.
                Err(ProfileEditError::NotFound(_)) => {
                    tracing::debug!(agent = %id, "library delete: agent gone before its presets were removed");
                }
                Err(e) => {
                    deleted.stopped = Some(RelinkStop {
                        path: None,
                        refusal: port_refused(e),
                    });
                    break;
                }
            }
        }
        tracing::info!(
            agents = deleted.agents.len(),
            "agent presets removed by a delete"
        );
    }

    /// The reference walk over `files` (module header «Stop rules» — ADR-0335): first one skipped row per mount never
    /// walked this run that cannot be read, then each file rewritten whole with what `lists` makes of its document —
    /// one in a mount that cannot be read (the recorded state — ADR-0400 decision 6) or one refused is skipped, its disk
    /// untouched; a read or write failure stops the walk (`Err`; the files after it are not touched).
    #[allow(clippy::too_many_arguments)]
    fn walk_refs(
        &self,
        files: &[LibPath],
        lists: impl Fn(&LibPath, &crate::format::PresetDoc) -> crate::format::PresetLists,
        catalog: &mut Catalog,
        mounts: &MountsNow,
        hold: &Hold,
        touched: &mut Vec<PathKey>,
        written: &mut Vec<String>,
        skipped: &mut Vec<RelinkSkip>,
    ) -> Result<(), RelinkStop> {
        let mut unknown: Vec<RelinkSkip> = mounts
            .iter()
            .filter_map(|mount| {
                let seen = mounts.seen(&mount.name);
                (!seen.usable() && !seen.walked).then(|| RelinkSkip {
                    path: mount.name.as_str().to_owned(),
                    reason: mount_unavailable(&mount.name, &seen.shown()),
                })
            })
            .collect();
        unknown.sort_by(|a, b| a.path.cmp(&b.path));
        skipped.extend(unknown);
        let seq = || hold.seq();
        for file in files {
            if let Err(refusal) = refuse_unusable_now(mounts, file) {
                skipped.push(RelinkSkip {
                    path: file.as_str().to_owned(),
                    reason: reason_of(refusal),
                });
                continue;
            }
            let rewritten = self.rewrite_in(
                file,
                None,
                catalog,
                mounts,
                &seq,
                |edit| Ok((lists(file, edit.doc), ())),
                touched,
            );
            match rewritten {
                Ok(done) => {
                    if done.written {
                        written.push(file.as_str().to_owned());
                    }
                }
                Err(LibraryRefusal::Refused { class, reason })
                    if class != RefusalClass::Unavailable =>
                {
                    skipped.push(RelinkSkip {
                        path: file.as_str().to_owned(),
                        reason,
                    });
                }
                Err(refusal) => {
                    tracing::warn!(path = file.as_str(), refusal = ?refusal, "library reference fix stopped");
                    return Err(RelinkStop {
                        path: Some(file.as_str().to_owned()),
                        refusal,
                    });
                }
            }
        }
        Ok(())
    }

    /// Restores `rows` from their bundles to the same paths in the same mounts (module header). `keep_both` = a taken
    /// name restores as `name (2)`, `name (3)` … instead of answering the row's conflict. Answers per row.
    ///
    /// Refusals of the whole call, nothing applied: no first scan applied `Unavailable` · `library-unavailable`; a stale
    /// `root_generation` `Conflict` · `root-generation-stale`. Everything else answers per row.
    // ADR-0409 decisions 5, 6, 8 and 9
    pub fn restore(
        &self,
        rows: &[TrashAddress],
        keep_both: bool,
        root_generation: Option<u64>,
    ) -> Result<RestoreAnswer, LibraryRefusal> {
        let mut outcomes: Vec<Option<Outcome<String>>> = vec![None; rows.len()];
        let mut wanted: Vec<(usize, MountName, BundleId, MountPath, LibPath)> = Vec::new();
        for (at, row) in rows.iter().enumerate() {
            match restorable(row) {
                Ok((mount, id, path, original)) => wanted.push((at, mount, id, path, original)),
                Err(outcome) => outcomes[at] = Some(outcome),
            }
        }
        if !wanted.is_empty() {
            self.first_scan_join()
                .map_err(LibraryRefusal::unavailable)?;
        }
        // ADR-0400 decision 6
        wanted.retain(
            |(at, _, _, _, original)| match self.refuse_unusable_mount(original) {
                Ok(()) => true,
                Err(refusal) => {
                    outcomes[*at] = Some(Outcome::Failed(reason_of(refusal)));
                    false
                }
            },
        );
        let mut restored = 0;
        let mut walks: Vec<LibPath> = Vec::new();
        if !wanted.is_empty() {
            before_hold();
            let done = self.change_held_at(root_generation, true, |_, mounts, hold| {
                let answer: Result<Vec<Restored>, LibraryRefusal> =
                    Ok(self.restore_in(&wanted, keep_both, mounts, hold));
                (answer, Vec::new())
            })??;
            for (at, outcome, walk) in done {
                if matches!(outcome, Outcome::Done(_)) {
                    restored += 1;
                }
                walks.extend(walk);
                outcomes[at] = Some(outcome);
            }
        }
        // ADR-0409 M3: each restored range enters the catalog by a walk, classified as a scan classifies it — from the
        //   topmost folder the restore made, so the folders it made are listed too.
        for path in &walks {
            let report = self.refresh(Some(path));
            if !report.errors.is_empty() {
                tracing::warn!(
                    path = path.as_str(),
                    errors = report.errors.len(),
                    "library restore: the walk of a restored range reported errors"
                );
            }
        }
        tracing::info!(rows = rows.len(), restored, "library restore");
        Ok(RestoreAnswer {
            items: rows
                .iter()
                .zip(outcomes)
                .map(|(row, outcome)| RestoreItem {
                    row: row.clone(),
                    outcome: outcome.expect("every row answered"),
                })
                .collect(),
        })
    }

    /// The body of a restore's hold: the trash lock taken once, then each row (module header).
    fn restore_in(
        &self,
        wanted: &[(usize, MountName, BundleId, MountPath, LibPath)],
        keep_both: bool,
        mounts: &MountsNow,
        hold: &Hold,
    ) -> Vec<Restored> {
        let held = match hold.take_trash() {
            Ok(held) => held,
            Err(TrashUnavailable::NotReserved) => {
                return wanted
                    .iter()
                    .map(|(at, _, _, _, original)| {
                        let args = json!({ "path": original.as_str() });
                        (*at, Outcome::Failed(reason(TRASH_NOT_RESERVED, args)), None)
                    })
                    .collect()
            }
            Err(TrashUnavailable::AlreadyHeld) => {
                unreachable!("a restore takes the trash lock once")
            }
        };
        let mut fallbacks = Fallbacks::new();
        let mut done = Vec::new();
        for (at, name, id, path, original) in wanted {
            let found = match self.find_row(mounts, name, id, path, original, &mut fallbacks) {
                Ok(found) => found,
                Err(outcome) => {
                    done.push((*at, outcome, None));
                    continue;
                }
            };
            let (dest, made) =
                match self.put_back(&found.mount, id, &found.path, found.kind, keep_both) {
                    Ok(back) => back,
                    Err(outcome) => {
                        done.push((*at, outcome, None));
                        continue;
                    }
                };
            if let Some(Ok(rows)) =
                fallbacks.get_mut(&(found.mount.folder.clone(), id.as_str().to_owned()))
            {
                rows.retain(|row| row.path != found.path);
            }
            if let Err(e) = prune_shell(
                &*self.store,
                &found.mount.folder,
                id,
                Some(&found.path),
                &held,
            ) {
                tracing::warn!(
                    bundle = id.as_str(),
                    "library restore: bundle shell not pruned: {e}"
                );
            }
            let back = virtual_path(&found.mount.name, Some(&dest));
            let walk = made.map_or_else(
                || back.clone(),
                |top| virtual_path(&found.mount.name, Some(&top)),
            );
            done.push((*at, Outcome::Done(back.as_str().to_owned()), Some(walk)));
        }
        done
    }

    /// Finds row `path` of bundle `id` in mount `name` (module header): where it is and what, or the row's outcome. A
    /// bundle read without its record is walked once per call (`fallbacks`).
    fn find_row(
        &self,
        mounts: &MountsNow,
        name: &MountName,
        id: &BundleId,
        path: &MountPath,
        original: &LibPath,
        fallbacks: &mut Fallbacks,
    ) -> Result<Found, Outcome<String>> {
        let failed = |code: &str, args: Value| Err(Outcome::Failed(reason(code, args)));
        let Some(mount) = mounts.get(name.as_str()).cloned() else {
            return Err(Outcome::Failed(not_found_reason(original.as_str())));
        };
        if let Err(refusal) = refuse_unusable_now(mounts, original) {
            return Err(Outcome::Failed(reason_of(refusal)));
        }
        let store = &*self.store;
        match check_trash_folder(store, &mount.folder) {
            Ok(true) => {}
            Ok(false) => return Err(Outcome::Failed(not_found_reason(original.as_str()))),
            Err(unusable) => return Err(Outcome::Failed(trash_unusable(&mount.name, &unusable))),
        }
        let is = |other: &MountPath| {
            path_key(Path::new(other.as_str())) == path_key(Path::new(path.as_str()))
        };
        let not_restorable = || failed(NOT_RESTORABLE, json!({ "path": original.as_str() }));
        let read_failed = |why: String| {
            failed(
                LIBRARY_READ_FAILED,
                json!({ "path": original.as_str(), "why": why }),
            )
        };
        match read_record(store, &mount.folder, id) {
            RecordRead::NotABundle { .. } => {
                Err(Outcome::Failed(not_found_reason(original.as_str())))
            }
            RecordRead::Failed { why } => read_failed(why),
            RecordRead::Roots(roots) => {
                if let Some(root) = roots.iter().find(|root| is(&root.root.path)) {
                    if root.root.staging.is_some() {
                        return not_restorable();
                    }
                    return match &root.state {
                        // The record's spelling: the row goes back under the names it had.
                        RootState::Present(kind) => Ok(Found {
                            kind: *kind,
                            path: root.root.path.clone(),
                            mount,
                        }),
                        RootState::ThroughLink => {
                            failed(PATH_THROUGH_LINK, json!({ "path": original.as_str() }))
                        }
                        RootState::Failed(why) => read_failed(why.clone()),
                    };
                }
                if let Some(root) = roots.iter().find(|root| below(path, &root.root.path)) {
                    return failed(
                        INSIDE_ROOT,
                        json!({ "path": original.as_str(), "root": root.root.path.as_str() }),
                    );
                }
                match store.check_names(&mount.folder, &row_path(id, path)) {
                    // An `other` row.
                    Ok(NameWalk::Present(_)) => not_restorable(),
                    Ok(NameWalk::Link(_)) => {
                        failed(PATH_THROUGH_LINK, json!({ "path": original.as_str() }))
                    }
                    Ok(NameWalk::Missing(_) | NameWalk::NotAFolder(_)) => {
                        Err(Outcome::Failed(not_found_reason(original.as_str())))
                    }
                    Err(e) => read_failed(e.to_string()),
                }
            }
            RecordRead::Missing | RecordRead::Unreadable { .. } => {
                let found = fallbacks
                    .entry((mount.folder.clone(), id.as_str().to_owned()))
                    .or_insert_with(|| fallback_rows(store, &mount.folder, id));
                match found {
                    Ok(found) => match found.iter().find(|row| is(&row.path)) {
                        Some(row) if row.staging => not_restorable(),
                        Some(row) => Ok(Found {
                            kind: row.kind,
                            path: row.path.clone(),
                            mount,
                        }),
                        None => Err(Outcome::Failed(not_found_reason(original.as_str()))),
                    },
                    Err(e)
                        if matches!(
                            e.kind(),
                            io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
                        ) =>
                    {
                        Err(Outcome::Failed(not_found_reason(original.as_str())))
                    }
                    Err(e) => read_failed(e.to_string()),
                }
            }
        }
    }

    /// Moves row `path` (an entry of `kind`) of bundle `id` back to `path` in `mount` (module header) — under keep-both
    /// to the first free `name (n)` — answering where it went and the topmost folder made for it, or the row's conflict
    /// or failure.
    fn put_back(
        &self,
        mount: &Mount,
        id: &BundleId,
        path: &MountPath,
        kind: EntryKind,
        keep_both: bool,
    ) -> Result<(MountPath, Option<MountPath>), Outcome<String>> {
        let original = virtual_path(&mount.name, Some(path));
        let from = row_path(id, path).to_path(&mount.folder);
        let taken = || Outcome::Conflict(reason(NAME_EXISTS, json!({ "path": original.as_str() })));
        for n in 1..=KEEP_BOTH_TRIES {
            let dest = match n {
                1 => path.clone(),
                n => numbered(path, n, kind != EntryKind::Folder),
            };
            let made = match self.make_parents(mount, &dest)? {
                Some(made) => made,
                None if keep_both => continue,
                None => return Err(taken()),
            };
            match self.store.rename(&from, &dest.to_path(&mount.folder)) {
                Ok(()) => return Ok((dest, made.into_iter().next())),
                Err(e) => {
                    self.unmake(mount, &made);
                    match e.kind() {
                        io::ErrorKind::AlreadyExists if keep_both => continue,
                        io::ErrorKind::AlreadyExists => return Err(taken()),
                        io::ErrorKind::Unsupported => {
                            return Err(Outcome::Failed(reason(
                                NO_REPLACE_UNSUPPORTED,
                                json!({ "path": original.as_str(), "why": e.to_string() }),
                            )))
                        }
                        _ => {
                            return Err(Outcome::Failed(reason(
                                LIBRARY_WRITE_FAILED,
                                json!({ "path": original.as_str(), "why": e.to_string() }),
                            )))
                        }
                    }
                }
            }
        }
        Err(taken())
    }

    /// Checks every name of `dest` in `mount` without following and makes its missing folders with `create_dir`, one name
    /// at a time: `Some(made)` = free, with the folders made, topmost first; `None` = something is at `dest`; `Err` = a
    /// link on the way (`path-through-link`), a file where a folder must be (a conflict, `name-exists`), or a failed stat
    /// or make.
    fn make_parents(
        &self,
        mount: &Mount,
        dest: &MountPath,
    ) -> Result<Option<Vec<MountPath>>, Outcome<String>> {
        let store = &*self.store;
        let mut made: Vec<MountPath> = Vec::new();
        let at_path = |at: &MountPath| virtual_path(&mount.name, Some(at)).as_str().to_owned();
        let give_up = |made: &[MountPath], outcome: Outcome<String>| {
            self.unmake(mount, made);
            Err(outcome)
        };
        // Each pass makes one missing folder (or finds it made meanwhile), so one pass per name ends the walk.
        for _ in 0..=dest.as_str().split('/').count() {
            match store.check_names(&mount.folder, dest) {
                Ok(NameWalk::Missing(at)) if at == *dest => return Ok(Some(made)),
                Ok(NameWalk::Missing(at)) => match store.create_dir(&at.to_path(&mount.folder)) {
                    Ok(()) => made.push(at),
                    Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
                    Err(e) => {
                        let args = json!({ "path": at_path(&at), "why": e.to_string() });
                        return give_up(&made, Outcome::Failed(reason(LIBRARY_WRITE_FAILED, args)));
                    }
                },
                Ok(NameWalk::Present(_)) => {
                    self.unmake(mount, &made);
                    return Ok(None);
                }
                Ok(NameWalk::Link(at)) => {
                    let args = json!({ "path": at_path(&at) });
                    return give_up(&made, Outcome::Failed(reason(PATH_THROUGH_LINK, args)));
                }
                Ok(NameWalk::NotAFolder(at)) => {
                    let args = json!({ "path": at_path(&at) });
                    return give_up(&made, Outcome::Conflict(reason(NAME_EXISTS, args)));
                }
                Err(e) => {
                    let args = json!({ "path": at_path(dest), "why": e.to_string() });
                    return give_up(&made, Outcome::Failed(reason(LIBRARY_READ_FAILED, args)));
                }
            }
        }
        let args = json!({ "path": at_path(dest), "why": "a folder kept reading as missing after it was made" });
        give_up(&made, Outcome::Failed(reason(LIBRARY_WRITE_FAILED, args)))
    }

    /// Removes the folders `made` in `mount` again, deepest first, empty-only; stops at the first that cannot go.
    fn unmake(&self, mount: &Mount, made: &[MountPath]) {
        for dir in made.iter().rev() {
            if let Err(e) = self.store.remove_dir(&dir.to_path(&mount.folder)) {
                tracing::debug!(
                    folder = virtual_path(&mount.name, Some(dir)).as_str(),
                    "library restore: folder left: {e}"
                );
                break;
            }
        }
    }
}

impl LibraryService {
    /// Moves `paths` into the folder `to`, each keeping its name (module header). A name taken there follows
    /// `on_conflict` (none = that item's conflict). `yes` / `detach_refs` / `plan_digest` answer the conflict of a
    /// referenced skill's lone `SKILL.md` leaving its skill folder, or of a merge leaving a referenced skill folder
    /// without its `SKILL.md`, as a delete's. Answers per item, the old → new mapping and the reference fix.
    ///
    /// Refusals of the whole call, nothing applied: `to` no library path, or a dot or temp name in it `InvalidArgument` ·
    /// `item-path-invalid`; `to` in the built-in `Conflict` · `mount-read-only`; no first scan applied `Unavailable` ·
    /// `library-unavailable`; `detach_refs` with neither a lone `SKILL.md` leaving its skill folder nor a merge leaving a
    /// skill folder without its `SKILL.md` `InvalidArgument` · [`DETACH_REFS_INVALID`]; such a skill referenced without
    /// its confirmation `Conflict` ·
    /// [`SKILL_UNCONFIRMED`] `{plan, plan_digest, forms}`; agents attaching a moved path while the agent store is
    /// read-only or the port cell is empty — that refusal; a stale `root_generation` `Conflict` ·
    /// `root-generation-stale`.
    // ADR-0335
    // ADR-0339
    // ADR-0409 decisions 2 and 4
    #[allow(clippy::too_many_arguments)]
    pub fn move_items(
        &self,
        paths: &[&str],
        to: &str,
        on_conflict: Option<OnConflict>,
        yes: bool,
        detach_refs: bool,
        plan_digest: Option<&str>,
        root_generation: Option<u64>,
    ) -> Result<MoveAnswer, LibraryRefusal> {
        let to = lib_path(to)?;
        // ADR-0400 decision 20
        if in_builtin(&to) {
            return Err(mount_read_only(to.as_str()));
        }
        if !admits_inside(&to) {
            return Err(LibraryRefusal::new(
                RefusalClass::InvalidArgument,
                ITEM_PATH_INVALID,
                json!({ "path": to.as_str(), "why": "a dot or temp name" }),
            ));
        }
        let mut outcomes: Vec<Option<Outcome<Moved>>> = vec![None; paths.len()];
        let mut requests = Vec::new();
        for (at, given) in paths.iter().enumerate() {
            match deletable(given) {
                Ok(path) => {
                    let dest = LibPath::parse(&format!("{}/{}", to.as_str(), path.file_name()))
                        .expect("a library path's last name is a valid name in a folder");
                    requests.push(Request {
                        at,
                        path,
                        dest,
                        skill_folder: None,
                        leaves: Vec::new(),
                    });
                }
                Err(reason) => outcomes[at] = Some(Outcome::Failed(reason)),
            }
        }
        self.relocate(
            paths,
            outcomes,
            requests,
            on_conflict,
            false,
            (yes, detach_refs, plan_digest),
            root_generation,
        )
    }

    /// Renames the item `path` to `name` in its folder (module header) — a move to a new name: a taken name is the
    /// item's conflict `name-exists` (no `onConflict`); a case-only rename goes through the no-replace rename like any
    /// other. `yes` / `detach_refs` / `plan_digest` as [`LibraryService::move_items`]'s, for a skill's `SKILL.md`
    /// renamed to a name that no longer makes its folder a skill.
    ///
    /// Refusals of the whole call as [`LibraryService::move_items`]'s, with `name` — not one valid name, or a dot or
    /// temp name: `InvalidArgument` · `item-path-invalid` — in place of `to`.
    // ADR-0339
    // ADR-0409 decisions 2 and 4
    pub fn rename(
        &self,
        path: &str,
        name: &str,
        yes: bool,
        detach_refs: bool,
        plan_digest: Option<&str>,
        root_generation: Option<u64>,
    ) -> Result<MoveAnswer, LibraryRefusal> {
        let mut outcomes: Vec<Option<Outcome<Moved>>> = vec![None];
        let mut requests = Vec::new();
        match deletable(path) {
            Ok(from) => {
                let folder = from
                    .parent()
                    .expect("a deletable path lies below a mount's top");
                requests.push(Request {
                    at: 0,
                    dest: new_name(&folder, name)?,
                    path: from,
                    skill_folder: None,
                    leaves: Vec::new(),
                });
            }
            Err(reason) => outcomes[0] = Some(Outcome::Failed(reason)),
        }
        self.relocate(
            &[path],
            outcomes,
            requests,
            None,
            true,
            (yes, detach_refs, plan_digest),
            root_generation,
        )
    }

    /// The steps a move and a rename share (module header): `requests` passed the per-item entrance of `given`;
    /// `rename` = a rename (only its own spelling is in place — a case-only rename is not).
    #[allow(clippy::too_many_arguments)]
    fn relocate(
        &self,
        given: &[&str],
        mut outcomes: Vec<Option<Outcome<Moved>>>,
        mut requests: Vec<Request>,
        on_conflict: Option<OnConflict>,
        rename: bool,
        asked: (bool, bool, Option<&str>),
        root_generation: Option<u64>,
    ) -> Result<MoveAnswer, LibraryRefusal> {
        let (yes, detach_refs, plan_digest) = asked;
        let inside = {
            let paths: Vec<(usize, &LibPath)> = requests
                .iter()
                .map(|request| (request.at, &request.path))
                .collect();
            nested(&paths)
        };
        for (at, folder) in inside {
            outcomes[at] = Some(Outcome::Skipped(Skip::Inside(folder)));
        }
        requests.retain(|request| outcomes[request.at].is_none());
        requests.retain(|request| {
            let same = key(&request.dest) == key(&request.path);
            if (same && !rename) || request.dest.as_str() == request.path.as_str() {
                outcomes[request.at] = Some(Outcome::Skipped(Skip::InPlace));
                return false;
            }
            if !same && within(&request.dest, Some(&request.path)) {
                outcomes[request.at] = Some(Outcome::Failed(into_itself(request)));
                return false;
            }
            true
        });
        if !requests.is_empty() {
            self.first_scan_join()
                .map_err(LibraryRefusal::unavailable)?;
        }
        // ADR-0400 decision 6: nothing is moved out of or into a mount that cannot be read.
        requests.retain(|request| {
            let usable = self
                .refuse_unusable_mount(&request.path)
                .and_then(|()| self.refuse_unusable_mount(&request.dest));
            match usable {
                Ok(()) => true,
                Err(refusal) => {
                    outcomes[request.at] = Some(Outcome::Failed(reason_of(refusal)));
                    false
                }
            }
        });
        self.refuse_into_itself(&mut requests, &mut outcomes);
        if on_conflict == Some(OnConflict::Overwrite) {
            // ADR-0409 decision 5: an overwrite's displaced entry goes to the destination mount's trash — one that cannot
            //   be used (checked without following) fails the item before any disk step, a merge's free entries included.
            let mounts = self.gate.with_read(|state| state.mounts.clone());
            requests.retain(|request| {
                let Some(unusable) = mounts
                    .locate(&request.dest)
                    .and_then(|(mount, _)| self.unusable_trash(mount))
                else {
                    return true;
                };
                outcomes[request.at] = Some(Outcome::Failed(unusable));
                false
            });
        }
        self.classify_relocation(&mut requests, on_conflict);
        // ADR-0339 M108: judged by the shape of the operation, not by whether anything refers to it.
        if detach_refs && requests.iter().all(|request| request.targets().is_empty()) {
            return Err(LibraryRefusal::new(
                RefusalClass::InvalidArgument,
                DETACH_REFS_INVALID,
                json!({}),
            ));
        }
        let mut relocated = None;
        if !requests.is_empty() {
            // Relink's rule: every mount that cannot be read is walked again first; inside the hold the recorded
            //   states decide which files are skipped.
            let names: Vec<MountName> = self.gate.with_read(|state| {
                state
                    .mounts
                    .iter()
                    .map(|mount| mount.name.clone())
                    .collect()
            });
            self.rewalk_unusable(&names);
            let targets: Vec<Target> = requests.iter().flat_map(Request::targets).collect();
            let mut detach = false;
            if !targets.is_empty() {
                let (referrers, plan) = self.delete_plan(&targets);
                if !referrers.confirmed(yes, detach_refs, plan_digest) {
                    return Err(skill_unconfirmed(&plan));
                }
                detach = yes && detach_refs && referrers.referenced();
            }
            let port = match agents_within(&self.values.registry.attachments(), &requests) {
                true => {
                    // ADR-0291 · ADR-0379: a read-only agent store refuses before anything is moved.
                    if let StoreStatus::ReadOnly(refusal) = self.values.registry.store_status() {
                        return Err(LibraryRefusal::Store(StoreError::Refused(refusal)));
                    }
                    Some(self.profile_edit()?)
                }
                false => None,
            };
            // ADR-0409 decision 14: of a same-volume move only an overwrite puts anything in a trash.
            let trash = on_conflict == Some(OnConflict::Overwrite);
            before_hold();
            let done = self.change_held_at(root_generation, trash, |catalog, mounts, hold| {
                let mut touched = Vec::new();
                let answer = self.move_in(
                    &requests,
                    on_conflict,
                    asked,
                    detach,
                    port,
                    catalog,
                    mounts,
                    hold,
                    &mut touched,
                );
                (answer, touched)
            })?;
            match done {
                Err(LibraryRefusal::Refused { reason, .. }) if reason.code == SKILL_UNCONFIRMED => {
                    // The fresh plan classifies as the hold did.
                    self.classify_relocation(&mut requests, on_conflict);
                    let targets: Vec<Target> = requests.iter().flat_map(Request::targets).collect();
                    return Err(skill_unconfirmed(&self.delete_plan(&targets).1));
                }
                done => relocated = Some(done?),
            }
        }
        let mut answer = MoveAnswer {
            items: Vec::new(),
            mapping: Vec::new(),
            detached: Vec::new(),
            written: Vec::new(),
            skipped: Vec::new(),
            trees: Vec::new(),
            agents: Vec::new(),
            stopped: None,
        };
        if let Some(done) = relocated {
            for folder in done.bundled {
                self.exclude_trash(folder);
            }
            for (at, outcome) in done.items {
                outcomes[at] = Some(outcome);
            }
            answer.mapping = done.mapping;
            answer.detached = done.detached;
            answer.written = done.written;
            answer.skipped = done.skipped;
            answer.agents = done.agents;
            answer.stopped = done.stopped;
            let files: Vec<&str> = answer
                .written
                .iter()
                .chain(answer.skipped.iter().map(|skip| &skip.path))
                .map(String::as_str)
                .collect();
            answer.trees = self.working_trees(&files);
        }
        answer.items = given
            .iter()
            .zip(outcomes)
            .map(|(path, outcome)| MoveItem {
                path: (*path).to_owned(),
                outcome: outcome.expect("every item answered"),
            })
            .collect();
        let done = answer
            .items
            .iter()
            .filter(|item| matches!(item.outcome, Outcome::Done(_)))
            .count();
        tracing::info!(
            items = answer.items.len(),
            done,
            written = answer.written.len(),
            stopped = answer.stopped.is_some(),
            "library move"
        );
        Ok(answer)
    }

    /// The detach shapes of `requests` from the current catalog and disk, outside every lock: each lone `SKILL.md`
    /// leaving its skill folder ([`classify_moves`]) and each skill folder a merge would leave without its `SKILL.md`
    /// ([`LibraryService::plan_leaves`]) — at the call for its plan, and again after the hold answered a fresh one.
    fn classify_relocation(&self, requests: &mut [Request], on_conflict: Option<OnConflict>) {
        let (skills, mounts) = self.gate.with_read(|state| {
            classify_moves(&state.catalog, requests);
            let skills = match on_conflict {
                Some(_) => skill_files(&state.catalog),
                None => HashMap::new(),
            };
            (skills, state.mounts.clone())
        });
        self.plan_leaves(requests, on_conflict, &skills, &mounts);
    }

    /// Sets each request's `leaves` (ADR-0339 M110 — a merge that takes a skill's `SKILL.md` and leaves its folder
    /// behind is a `SKILL.md` moved out): with `onConflict`, a folder item whose destination is a folder of its name
    /// merges, and each skill folder in it (`skills`: folder key → (folder path, `SKILL.md` name)) whose `SKILL.md` the merge would
    /// move while the folder keeps an entry is named by that file — judged from no-follow listings and stats as
    /// [`LibraryService::merge`] places entries ([`LibraryService::plan_merge`]). An item with no skill of `skills` at
    /// or below it plans nothing (no disk read). The item and the destination pass the link admission first (ADR-0365);
    /// one that does not, or a stat that fails, plans nothing (the hold answers it).
    fn plan_leaves(
        &self,
        requests: &mut [Request],
        on_conflict: Option<OnConflict>,
        skills: &HashMap<PathKey, (LibPath, String)>,
        mounts: &crate::mounts::Mounts,
    ) {
        for request in requests.iter_mut() {
            request.leaves.clear();
            let Some(choice) = on_conflict else {
                continue;
            };
            if !skills
                .values()
                .any(|(folder, _)| within(folder, Some(&request.path)))
            {
                continue;
            }
            let admitted = (
                self.admit(mounts, &request.path),
                self.admit(mounts, &request.dest),
            );
            let (Ok(from), Ok(to)) = admitted else {
                continue;
            };
            if matches!(
                (self.store.stat(&from), self.store.stat(&to)),
                (Ok(EntryKind::Folder), Ok(EntryKind::Folder))
            ) {
                let path = request.path.clone();
                self.plan_merge(&from, &path, &to, choice, skills, &mut request.leaves);
            }
        }
    }

    /// Whether merging the folder `from` (at `from_disk`) into the folder at `to_disk` under `choice` would leave an
    /// entry in it, as [`LibraryService::merge`] places entries (a failed listing or stat: it stays); pushes the
    /// `SKILL.md` of each skill folder in it that the merge would move away while that folder stays.
    fn plan_merge(
        &self,
        from_disk: &Path,
        from: &LibPath,
        to_disk: &Path,
        choice: OnConflict,
        skills: &HashMap<PathKey, (LibPath, String)>,
        leaves: &mut Vec<LibPath>,
    ) -> bool {
        let Ok(entries) = self.store.list(from_disk) else {
            return true;
        };
        let skill_file = skills.get(&key(from)).map(|(_, file)| file);
        let (mut stays, mut skill_moved) = (false, false);
        for (name, kind) in entries {
            let named = name.to_str().and_then(|name| {
                let inner = LibPath::parse(&format!("{}/{name}", from.as_str())).ok()?;
                Some((name, inner))
            });
            let Some((name, inner)) = named else {
                stays = true;
                continue;
            };
            let into = to_disk.join(name);
            let moves = match self.store.stat(&into) {
                Err(e) if e.kind() == io::ErrorKind::NotFound => true,
                Err(_) => false,
                Ok(EntryKind::Folder) if kind == EntryKind::Folder => {
                    let inner_disk = from_disk.join(name);
                    stays |= self.plan_merge(&inner_disk, &inner, &into, choice, skills, leaves);
                    continue;
                }
                Ok(there) => match choice {
                    OnConflict::Skip => false,
                    OnConflict::KeepBoth => true,
                    OnConflict::Overwrite => {
                        (kind == EntryKind::Folder) == (there == EntryKind::Folder)
                    }
                },
            };
            match moves {
                true => skill_moved |= skill_file.is_some_and(|file| file == name),
                false => stays = true,
            }
        }
        if let Some(file) = skill_file.filter(|_| skill_moved && stays) {
            leaves.push(
                LibPath::parse(&format!("{}/{file}", from.as_str()))
                    .expect("a skill folder and its listed SKILL.md name are a library path"),
            );
        }
        stays
    }

    /// Refuses `into-itself` each folder item whose destination folder is it or lies inside it by canonical path and
    /// identity, as `mounts::overlap` judges folders (ADR-0409) — before the ticket, holding no lock, one read budget.
    /// The catalog tells a folder (a file holds nothing). The probe follows links, so it asks only when the item and the
    /// destination folder passed the no-follow link admission (a mount's top may be a link); otherwise the item is left
    /// to the hold, which refuses it (`path-through-link` — ADR-0365 · ADR-0414 decision 5). An ask that gave no answer
    /// fails the item `library-read-failed`.
    fn refuse_into_itself(
        &self,
        requests: &mut Vec<Request>,
        outcomes: &mut [Option<Outcome<Moved>>],
    ) {
        let (mounts, folders): (_, Vec<bool>) = self.gate.with_read(|state| {
            let folders = requests
                .iter()
                .map(|request| {
                    let at = key(&request.path);
                    state.catalog.folders().any(|folder| key(folder) == at)
                        || state
                            .catalog
                            .get(&at)
                            .is_some_and(|item| item.kind == ItemKind::Skill)
                })
                .collect();
            (state.mounts.clone(), folders)
        });
        if !folders.contains(&true) {
            return;
        }
        let read = self.io.begin_read();
        let probe = DiskProbe {
            io: &self.io,
            read: &read,
        };
        let mut folders = folders.into_iter();
        requests.retain(|request| {
            if folders.next() != Some(true) {
                return true;
            }
            let folder = request
                .dest
                .parent()
                .expect("a destination lies in a folder");
            let into = match folder.parent() {
                None => mounts.disk_path(&folder),
                Some(_) => self.admit(&mounts, &folder).ok(),
            };
            let (Ok(item), Some(into)) = (self.admit(&mounts, &request.path), into) else {
                return true;
            };
            let refused = match lies_within(&into, &item, &probe) {
                Ok(false) => return true,
                Ok(true) => into_itself(request),
                Err(why) => reason(
                    LIBRARY_READ_FAILED,
                    json!({ "path": request.path.as_str(), "why": why }),
                ),
            };
            outcomes[request.at] = Some(Outcome::Failed(refused));
            false
        });
    }

    /// The body of a move's hold (module header): the `SKILL.md` judgement again, each item's checks, the disk step
    /// item by item, the trash records, the catalog, then the reference fix.
    #[allow(clippy::too_many_arguments)]
    fn move_in(
        &self,
        requests: &[Request],
        on_conflict: Option<OnConflict>,
        asked: (bool, bool, Option<&str>),
        detach: bool,
        port: Option<std::sync::Arc<dyn ProfileEditPort>>,
        catalog: &mut Catalog,
        mounts: &MountsNow,
        hold: &Hold,
        touched: &mut Vec<PathKey>,
    ) -> Result<Relocated, LibraryRefusal> {
        // ADR-0409 decision 2 · ADR-0414 decision 4: the lone-`SKILL.md` kind is read again from the hold's catalog,
        //   the merge leaves are planned again from the disk, and the plan is judged again before any disk step; what
        //   it names is the confirmed plan a detach keeps to.
        let (yes, detach_refs, plan_digest) = asked;
        let mut requests = requests.to_vec();
        classify_moves(catalog, &mut requests);
        if on_conflict.is_some() {
            self.plan_leaves(&mut requests, on_conflict, &skill_files(catalog), mounts);
        }
        let targets: Vec<Target> = requests.iter().flat_map(Request::targets).collect();
        let judged = Referrers::of(catalog, &self.values.registry.attachments(), &targets);
        if !judged.confirmed(yes, detach_refs, plan_digest) {
            return Err(LibraryRefusal::new(
                RefusalClass::Conflict,
                SKILL_UNCONFIRMED,
                json!({}),
            ));
        }
        let mut items: Vec<(usize, Outcome<Moved>)> = Vec::new();
        let mut placed: Vec<Placed> = Vec::new();
        for request in &requests {
            match self.place_request(request, catalog, mounts) {
                Ok(item) => placed.push(item),
                Err(reason) => items.push((request.at, Outcome::Failed(reason))),
            }
        }
        if on_conflict == Some(OnConflict::Overwrite) {
            // ADR-0409 decision 5: judged again before the call's first rename — a `.trash` broken since the call's
            //   check fails its items with nothing of them moved, a merge's free entries included.
            placed.retain(|item| match self.unusable_trash(&item.to.mount) {
                Some(unusable) => {
                    items.push((item.request.at, Outcome::Failed(unusable)));
                    false
                }
                None => true,
            });
        }
        let mut step = Step {
            trashing: Trashing::new(hold),
            moves: Moves::default(),
            leaves: requests
                .iter()
                .flat_map(|request| request.leaves.iter().map(key))
                .collect(),
            left: Vec::new(),
            trashed: Vec::new(),
        };
        let mut gone: Vec<Target> = Vec::new();
        let mut leaving: Vec<Target> = Vec::new();
        for item in &placed {
            let outcome = self.move_item(item, on_conflict, catalog, mounts, &mut step);
            if matches!(outcome, Outcome::Done(_)) {
                gone.extend(item.request.lone());
                leaving.extend(item.request.left());
            }
            items.push((item.request.at, outcome));
        }
        let Step {
            trashing, moves, ..
        } = step;
        let bundled = trashing.finish(&*self.store);
        self.follow_moves(catalog, mounts, hold, &moves, touched);
        // A skill folder a merge was to leave went only when it is no skill now (its `SKILL.md` did move) and did not
        //   merge away whole — then it is the destination's skill and its references follow the exact pair.
        gone.extend(leaving.into_iter().filter(|target| {
            target.skill_folder.as_ref().is_some_and(|folder| {
                !moves
                    .pairs
                    .iter()
                    .any(|pair| pair.exact && key(&pair.old) == key(folder))
                    && catalog
                        .get(&key(folder))
                        .is_none_or(|item| item.kind != ItemKind::Skill)
            })
        }));
        let mut relocated = Relocated {
            items,
            mapping: moves
                .pairs
                .iter()
                .map(|pair| {
                    let (old, new) = (pair.old.as_str(), pair.new.as_str());
                    (old.to_owned(), new.to_owned(), pair.exact)
                })
                .collect(),
            detached: Vec::new(),
            written: Vec::new(),
            skipped: Vec::new(),
            agents: Vec::new(),
            stopped: None,
            bundled,
        };
        if !moves.pairs.is_empty() {
            self.follow_refs(
                &moves.pairs,
                &gone,
                detach,
                &judged,
                port,
                catalog,
                mounts,
                hold,
                touched,
                &mut relocated,
            );
        }
        Ok(relocated)
    }

    /// The hold's checks of one item before any disk step (module header): both mounts from the recorded state, the
    /// link admission of the item and of its destination, a no-follow stat of the item (absent `not-found`), the
    /// destination folder a real folder (absent or a file `not-found`, a link `path-through-link`). The item and the
    /// destination folder are spelled as the catalog spells them when that is the same disk entry
    /// ([`LibraryService::same_entry_spelling`]); the new name stays as given.
    fn place_request(
        &self,
        request: &Request,
        catalog: &Catalog,
        mounts: &MountsNow,
    ) -> Result<Placed, Reason> {
        refuse_unusable_now(mounts, &request.path).map_err(reason_of)?;
        refuse_unusable_now(mounts, &request.dest).map_err(reason_of)?;
        let given_disk = self.admit(mounts, &request.path).map_err(reason_of)?;
        let (spelled, disk) = self.same_entry_spelling(
            mounts,
            &request.path,
            given_disk,
            catalog_spelling(catalog, &request.path),
        );
        let Some((mount, Some(rest))) = mounts.locate(&spelled) else {
            return Err(not_found_reason(request.path.as_str()));
        };
        let kind = match self.store.stat(&disk) {
            Ok(kind) => kind,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                return Err(not_found_reason(request.path.as_str()))
            }
            Err(e) => {
                return Err(reason(
                    LIBRARY_READ_FAILED,
                    json!({ "path": request.path.as_str(), "why": e.to_string() }),
                ))
            }
        };
        self.admit(mounts, &request.dest).map_err(reason_of)?;
        let folder = request
            .dest
            .parent()
            .expect("a destination lies in a folder");
        let Some((to_mount, folder_rest)) = mounts.locate(&folder) else {
            return Err(not_found_reason(folder.as_str()));
        };
        let folder = match folder_rest {
            // A mount's top: its folder is there (the mount is usable).
            None => folder,
            Some(folder_rest) => {
                match self.store.check_names(&to_mount.folder, &folder_rest) {
                    Ok(NameWalk::Present(EntryKind::Folder)) => {}
                    Ok(NameWalk::Present(EntryKind::Link) | NameWalk::Link(_)) => {
                        return Err(reason(
                            PATH_THROUGH_LINK,
                            json!({ "path": folder.as_str() }),
                        ))
                    }
                    // ADR-0335 decision 7: a folder that is not there is not made.
                    Ok(_) => return Err(not_found_reason(folder.as_str())),
                    Err(e) => {
                        return Err(reason(
                            LIBRARY_READ_FAILED,
                            json!({ "path": folder.as_str(), "why": e.to_string() }),
                        ))
                    }
                }
                let folder_disk = folder_rest.to_path(&to_mount.folder);
                let respelled = catalog_spelling(catalog, &folder);
                self.same_entry_spelling(mounts, &folder, folder_disk, respelled)
                    .0
            }
        };
        let dest = LibPath::parse(&format!("{}/{}", folder.as_str(), request.dest.file_name()))
            .expect("a folder and a valid name are a valid path");
        let Some((to_mount, Some(to_rest))) = mounts.locate(&dest) else {
            return Err(not_found_reason(request.dest.as_str()));
        };
        Ok(Placed {
            request: request.clone(),
            from: Side {
                mount: mount.clone(),
                rest,
                disk,
            },
            to: Side {
                disk: to_rest.to_path(&to_mount.folder),
                mount: to_mount.clone(),
                rest: to_rest,
            },
            kind,
        })
    }

    /// Moves one checked item ([`LibraryService::place`]) and answers it: a merge carries the entries it left and
    /// those its overwrites displaced.
    fn move_item(
        &self,
        item: &Placed,
        on_conflict: Option<OnConflict>,
        catalog: &Catalog,
        mounts: &MountsNow,
        step: &mut Step<'_, '_>,
    ) -> Outcome<Moved> {
        step.left.clear();
        step.trashed.clear();
        // An earlier item of this step displaced the source or moved another entry onto it: it is no longer the entry
        //   the checks saw.
        if step.moves.changed(&item.from.lib()) {
            return Outcome::Failed(not_found_reason(item.request.path.as_str()));
        }
        match self.place(
            &item.from,
            &item.to,
            item.kind,
            on_conflict,
            catalog,
            mounts,
            step,
        ) {
            Placement::Moved { to, trashed } => Outcome::Done(Moved {
                to: to.lib().as_str().to_owned(),
                trashed: trashed.into_iter().collect(),
                left: Vec::new(),
            }),
            Placement::Merged { .. } => Outcome::Done(Moved {
                to: item.to.lib().as_str().to_owned(),
                trashed: std::mem::take(&mut step.trashed),
                left: std::mem::take(&mut step.left),
            }),
            Placement::Skipped(at) => Outcome::Skipped(Skip::Taken(at.as_str().to_owned())),
            Placement::Conflict(reason) => Outcome::Conflict(reason),
            Placement::Failed(reason) => Outcome::Failed(reason),
        }
    }

    /// Places `from` (of `kind`) at `to` by the no-replace rename (ADR-0409 decision 10); a taken name follows
    /// `on_conflict`: none = a conflict; same-name folders merge ([`LibraryService::merge`]); otherwise skip, keep both
    /// ([`kept_name`]), or overwrite ([`LibraryService::overwrite`] — a file onto a folder or the reverse
    /// [`KIND_MISMATCH`], ADR-0409 M8). A link is no folder: it moves, collides and is displaced as a link.
    #[allow(clippy::too_many_arguments)]
    fn place(
        &self,
        from: &Side,
        to: &Side,
        kind: EntryKind,
        on_conflict: Option<OnConflict>,
        catalog: &Catalog,
        mounts: &MountsNow,
        step: &mut Step<'_, '_>,
    ) -> Placement {
        match self.store.rename(&from.disk, &to.disk) {
            Ok(()) => {
                step.moves.moved(from, to, kind, false);
                return Placement::Moved {
                    to: to.clone(),
                    trashed: None,
                };
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
            Err(e) => return Placement::Failed(rename_failed(&from.lib(), &to.lib(), &e)),
        }
        let Some(choice) = on_conflict else {
            return Placement::Conflict(name_taken(&to.lib()));
        };
        let there = match self.store.stat(&to.disk) {
            Ok(there) => there,
            Err(e) => {
                return Placement::Failed(reason(
                    LIBRARY_READ_FAILED,
                    json!({ "path": to.lib().as_str(), "why": e.to_string() }),
                ))
            }
        };
        let (folder, onto_folder) = (kind == EntryKind::Folder, there == EntryKind::Folder);
        match choice {
            _ if folder && onto_folder => self.merge(from, to, choice, catalog, mounts, step),
            OnConflict::Skip => Placement::Skipped(to.lib()),
            OnConflict::KeepBoth => self.keep_both(from, to, kind, step),
            OnConflict::Overwrite if folder != onto_folder => Placement::Conflict(reason(
                KIND_MISMATCH,
                json!({ "path": from.lib().as_str(), "to": to.lib().as_str() }),
            )),
            OnConflict::Overwrite => self.overwrite(from, to, kind, catalog, mounts, step),
        }
    }

    /// Places `from` at the first free keep-both name of `to` (`name (2)`, `name (3)` … — [`kept_name`]).
    fn keep_both(
        &self,
        from: &Side,
        to: &Side,
        kind: EntryKind,
        step: &mut Step<'_, '_>,
    ) -> Placement {
        for n in 2..=KEEP_BOTH_TRIES {
            let next = to.at(kept_name(&to.rest, n, kind != EntryKind::Folder));
            match self.store.rename(&from.disk, &next.disk) {
                Ok(()) => {
                    step.moves.moved(from, &next, kind, false);
                    return Placement::Moved {
                        to: next,
                        trashed: None,
                    };
                }
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Placement::Failed(rename_failed(&from.lib(), &next.lib(), &e)),
            }
        }
        Placement::Conflict(name_taken(&to.lib()))
    }

    /// Overwrites `to` with `from` (ADR-0339 decision 1): the entry at `to` — spelled as the catalog spells it when
    /// that is the same entry — goes to its mount's trash ([`Trashing::put`]), then `from` takes the name.
    /// When that rename fails the displaced entry is renamed back; when that fails too, the item fails naming the trash
    /// row it is in and the name it left.
    #[allow(clippy::too_many_arguments)]
    fn overwrite(
        &self,
        from: &Side,
        to: &Side,
        kind: EntryKind,
        catalog: &Catalog,
        mounts: &MountsNow,
        step: &mut Step<'_, '_>,
    ) -> Placement {
        let to = self.spelled_side(to, catalog, mounts);
        let store = &*self.store;
        let put = step
            .trashing
            .put(store, &to.mount, &to.rest, &to.disk, &to.lib());
        let (address, id) = match put {
            Ok(displaced) => displaced,
            Err(reason) => return Placement::Failed(reason),
        };
        let e = match self.store.rename(&from.disk, &to.disk) {
            Ok(()) => {
                step.moves.displaced.push(to.lib());
                step.moves.moved(from, &to, kind, false);
                return Placement::Moved {
                    to,
                    trashed: Some(address),
                };
            }
            Err(e) => e,
        };
        let failed = rename_failed(&from.lib(), &to.lib(), &e);
        match step.trashing.take_back(store, &to, &id) {
            Ok(()) => Placement::Failed(failed),
            Err(back) => {
                step.moves.displaced.push(to.lib());
                tracing::warn!(
                    path = to.lib().as_str(),
                    bundle = id.as_str(),
                    "library move: an overwritten entry stays in the trash: {back}"
                );
                let mut args = failed.args.clone();
                args["trashed"] = json!({
                    "mount": address.mount,
                    "bundle": address.bundle,
                    "path": address.path,
                });
                args["put_back"] = json!(back.to_string());
                Placement::Failed(Reason { args, ..failed })
            }
        }
    }

    /// Why `mount`'s `.trash` cannot take an overwrite's displaced entry, checked without following
    /// ([`check_trash_folder`] — absent is usable, it is made at the first displacement); `None` = usable.
    fn unusable_trash(&self, mount: &Mount) -> Option<Reason> {
        check_trash_folder(&*self.store, &mount.folder)
            .err()
            .map(|unusable| trash_unusable(&mount.name, &unusable))
    }

    /// `side` spelled as the catalog spells it when that is the same disk entry
    /// ([`LibraryService::same_entry_spelling`]) — the name a trash row keeps and a restore brings back.
    fn spelled_side(&self, side: &Side, catalog: &Catalog, mounts: &MountsNow) -> Side {
        let given = side.lib();
        let respelled = catalog_spelling(catalog, &given);
        let (spelled, disk) =
            self.same_entry_spelling(mounts, &given, side.disk.clone(), respelled);
        match mounts.locate(&spelled) {
            Some((mount, Some(rest))) => Side {
                mount: mount.clone(),
                rest,
                disk,
            },
            _ => side.clone(),
        }
    }

    /// Merges the folder `from` into the same-name folder `to`: each entry placed by the same rules
    /// ([`LibraryService::place`], `choice` for each collision), then `from` removed empty-only (ADR-0409 M7) — a
    /// folder left holding a skipped or failed entry stays, one whose removal fails is named in `left`. A name that is
    /// no library name stays where it is. A skill folder's `SKILL.md` goes last, and stays (left
    /// [`SKILL_UNCONFIRMED`]) when an entry stayed behind, the file would move (its name free at `to`, or keep-both,
    /// or an overwrite of its kind) and the judgement did not confirm that leave (`Step.leaves`); one that would stay
    /// anyway is placed and answers its own reason. A folder whose entries all moved maps to `to` only when its
    /// `SKILL.md` became `to`'s — also when its removal fails; what the catalog still holds in it leaves only when it
    /// was removed.
    fn merge(
        &self,
        from: &Side,
        to: &Side,
        choice: OnConflict,
        catalog: &Catalog,
        mounts: &MountsNow,
        step: &mut Step<'_, '_>,
    ) -> Placement {
        // A folder with a dot or temp name holds no item: nothing in it is classified.
        for side in [from, to] {
            let folder = side.lib();
            if admits_inside(&folder) {
                step.moves.folders.push(folder);
            }
        }
        let mut entries = match self.store.list(&from.disk) {
            Ok(entries) => entries,
            Err(e) => {
                return Placement::Failed(reason(
                    LIBRARY_READ_FAILED,
                    json!({ "path": from.lib().as_str(), "why": e.to_string() }),
                ))
            }
        };
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        // ADR-0339: a reference to the folder itself names its skill — it follows only where that skill's `SKILL.md`
        //   went (under keep-both, or left behind, the destination is another skill or none).
        let skill_file = match catalog.get(&key(&from.lib())).map(|item| &item.content) {
            Some(ItemContent::Skill { file_name, .. }) => Some(file_name.as_str()),
            _ => None,
        };
        // ADR-0339 M110: the `SKILL.md` goes last, so whether anything stays behind is known before it leaves.
        if let Some(at) = entries
            .iter()
            .position(|(name, _)| skill_file.is_some() && name.to_str() == skill_file)
        {
            let last = entries.remove(at);
            entries.push(last);
        }
        let mut skill_moved = false;
        let mut stays = false;
        for (name, kind) in entries {
            let sides = name
                .to_str()
                .and_then(|name| Some((from.child(name)?, to.child(name)?)));
            let Some((inner, into)) = sides else {
                stays = true;
                let at = escape_controls(&format!(
                    "{}/{}",
                    from.lib().as_str(),
                    name.to_string_lossy()
                ))
                .to_string();
                let why = reason(
                    ITEM_PATH_INVALID,
                    json!({ "path": at, "why": "a name that is no library name" }),
                );
                step.left.push((at, why));
                continue;
            };
            let is_skill_file = skill_file.is_some() && name.to_str() == skill_file;
            // ADR-0339 M110 · ADR-0409 decision 2: a skill left behind that the judgement did not plan (an earlier item
            //   of the call or a failed entry changed what stays) keeps its `SKILL.md` — nothing unasked leaves it. A
            //   `SKILL.md` that stays anyway (taken under skip, a folder there under overwrite) keeps its own reason.
            if is_skill_file
                && stays
                && !step.leaves.contains(&key(&inner.lib()))
                && !self.stays_anyway(&into, kind, choice)
            {
                let at = inner.lib().as_str().to_owned();
                let why = reason(SKILL_UNCONFIRMED, json!({ "path": at }));
                step.left.push((at, why));
                continue;
            }
            match self.place(&inner, &into, kind, Some(choice), catalog, mounts, step) {
                Placement::Moved {
                    to: placed,
                    trashed,
                } => {
                    skill_moved |= is_skill_file && makes_skill(placed.rest.file_name(), &to.lib());
                    step.trashed.extend(trashed);
                }
                Placement::Merged { emptied } => stays |= !emptied,
                Placement::Skipped(at) => {
                    stays = true;
                    step.left
                        .push((inner.lib().as_str().to_owned(), name_taken(&at)));
                }
                Placement::Conflict(why) | Placement::Failed(why) => {
                    stays = true;
                    step.left.push((inner.lib().as_str().to_owned(), why));
                }
            }
        }
        let emptied = !stays
            && match self.store.remove_dir(&from.disk) {
                Ok(()) => true,
                Err(e) => {
                    let args = json!({ "path": from.lib().as_str(), "why": e.to_string() });
                    step.left.push((
                        from.lib().as_str().to_owned(),
                        reason(LIBRARY_WRITE_FAILED, args),
                    ));
                    false
                }
            };
        if emptied {
            step.moves.emptied.push(from.lib());
        }
        // ADR-0339: everything moved and the skill's `SKILL.md` became `to`'s — the skill is `to` now, whether or
        //   not the empty folder could be removed (one left is re-classified as no skill).
        if !stays && skill_moved {
            step.moves.moved(from, to, EntryKind::Folder, true);
        }
        Placement::Merged { emptied }
    }

    /// Whether a merge's entry `into` (of `kind`) would stay where it is under `choice` by what is at its destination
    /// (a no-follow stat): a taken name under skip, a folder onto a file or the reverse under overwrite. A stat that
    /// fails other than by absence: not known to stay.
    fn stays_anyway(&self, into: &Side, kind: EntryKind, choice: OnConflict) -> bool {
        match self.store.stat(&into.disk) {
            Ok(there) => match choice {
                OnConflict::Skip => true,
                OnConflict::KeepBoth => false,
                OnConflict::Overwrite => {
                    (kind == EntryKind::Folder) != (there == EntryKind::Folder)
                }
            },
            Err(_) => false,
        }
    }

    /// The catalog after a move's disk step (ADR-0409 M3): the displaced ranges leave, each pair's range is re-keyed
    /// by the path-key prefix mapper (`LibPath::replace_prefix`) and what the catalog still holds in a merge's emptied
    /// source folder leaves; each moved file whose kind may have changed — a new name, or a `SKILL.md` name at either
    /// end — and the `SKILL.md` of a skill at a moved folder's new path are re-read ([`LibraryService::reread_file`]),
    /// then every folder whose direct children changed is re-classified ([`LibraryService::reclassify`]). A read or
    /// listing that fails leaves what the catalog had; the next scan does it.
    fn follow_moves(
        &self,
        catalog: &mut Catalog,
        mounts: &MountsNow,
        hold: &Hold,
        moves: &Moves,
        touched: &mut Vec<PathKey>,
    ) {
        let at = hold.seq();
        for gone in &moves.displaced {
            touched.extend(catalog.remove_range(gone, at));
        }
        // An exact pair moves no range: what was below its folder moved by its own pairs, and the rest is gone.
        let whole: Vec<&Pair> = moves.pairs.iter().filter(|pair| !pair.exact).collect();
        for pair in &whole {
            touched.extend(catalog.rekey_range(&pair.old, |path| {
                path.replace_prefix(&pair.old, &pair.new)
                    .expect("a path within the range has its prefix")
            }));
        }
        for gone in &moves.emptied {
            touched.extend(catalog.remove_range(gone, at));
        }
        let mut reread: Vec<LibPath> = Vec::new();
        for pair in &whole {
            match pair.kind {
                EntryKind::File
                    if pair.old.file_name() != pair.new.file_name()
                        || skill_file_name(pair.old.file_name())
                        || skill_file_name(pair.new.file_name()) =>
                {
                    reread.push(pair.new.clone());
                }
                // ADR-0409 M3: a skill's name and folder come from its `SKILL.md` and its own folder's name — one below a
                //   moved folder keeps both, so only the skill at the pair's new path is re-read.
                EntryKind::Folder => {
                    let skill = catalog.get(&key(&pair.new)).map(|item| &item.content);
                    if let Some(ItemContent::Skill { file_name, .. }) = skill {
                        reread.extend(
                            LibPath::parse(&format!("{}/{file_name}", pair.new.as_str())).ok(),
                        );
                    }
                }
                _ => {}
            }
        }
        let seq = || hold.seq();
        let mut seen: HashSet<PathKey> = HashSet::new();
        for file in reread.iter().filter(|file| seen.insert(key(file))) {
            let Some((mount, Some(rest))) = mounts.locate(file) else {
                continue;
            };
            match self.reread_file(catalog, mount, &rest, &seq) {
                Ok(keys) => touched.extend(keys),
                Err(e) => tracing::warn!(
                    path = file.as_str(),
                    "library move: file not re-read; the next scan does it: {e}"
                ),
            }
        }
        let mut folders: Vec<LibPath> = moves.folders.clone();
        for pair in &moves.pairs {
            folders.extend(pair.old.parent());
            folders.extend(pair.new.parent());
        }
        let mut seen: HashSet<PathKey> = HashSet::new();
        for folder in folders.iter().filter(|folder| seen.insert(key(folder))) {
            // A mount's top is never a skill.
            let Some((mount, Some(rest))) = mounts.locate(folder) else {
                continue;
            };
            match self.reclassify(catalog, mount, &rest, &seq) {
                Ok(keys) => touched.extend(keys),
                Err(e) => tracing::warn!(
                    folder = folder.as_str(),
                    "library move: folder not re-classified; the next scan does it: {e}"
                ),
            }
        }
    }

    /// The reference fix of a move over its `pairs` (module header — ADR-0409 decision 4, no plan confirmation): every
    /// preset file the catalog lists that names a moved path is rewritten whole with the mapping (ADR-0358; a touched
    /// list keeps the first place of each path — ADR-0339) and, with `detach`, the files of the confirmed plan
    /// (`judged`) that still list what a gone lone `SKILL.md` or a merge leave took away drop those lines too (ADR-0414
    /// decision 1);
    /// then the change number is raised and the agents are re-pointed once over the current roster
    /// (`ProfileEditPort::rewrite_preset_paths`). Relink's stop rules.
    #[allow(clippy::too_many_arguments)]
    fn follow_refs(
        &self,
        pairs: &[Pair],
        gone: &[Target],
        detach: bool,
        judged: &Referrers,
        port: Option<std::sync::Arc<dyn ProfileEditPort>>,
        catalog: &mut Catalog,
        mounts: &MountsNow,
        hold: &Hold,
        touched: &mut Vec<PathKey>,
        out: &mut Relocated,
    ) {
        let drop: &[Target] = if detach { gone } else { &[] };
        let mut dropping: Vec<LibPath> = Vec::new();
        if !drop.is_empty() {
            out.detached = drop
                .iter()
                .filter_map(|target| target.skill_folder.as_ref())
                .map(|folder| folder.as_str().to_owned())
                .collect();
            // The confirmed plan's files by the path they have now; the catalog still lists their old references.
            let still = Referrers::of(catalog, &[], drop).presets;
            dropping = judged
                .presets
                .iter()
                .map(|path| mapped(pairs, path).unwrap_or_else(|| path.clone()))
                .filter(|path| still.iter().any(|now| key(now) == key(path)))
                .collect();
        }
        let mut files: Vec<LibPath> = catalog
            .items()
            .filter(|item| match &item.content {
                ItemContent::Preset { doc, .. } => {
                    preset_refs(doc).any(|path| mapped(pairs, path).is_some())
                }
                _ => false,
            })
            .map(|item| item.path.clone())
            .collect();
        for file in &dropping {
            if !files.iter().any(|listed| key(listed) == key(file)) {
                files.push(file.clone());
            }
        }
        files.sort();
        let lists = |file: &LibPath, doc: &crate::format::PresetDoc| {
            let lines: &[Target] = match dropping.iter().any(|path| key(path) == key(file)) {
                true => drop,
                false => &[],
            };
            moved_lists(doc, pairs, lines)
        };
        let walked = self.walk_refs(
            &files,
            lists,
            catalog,
            mounts,
            hold,
            touched,
            &mut out.written,
            &mut out.skipped,
        );
        if let Err(stop) = walked {
            out.stopped = Some(stop);
            return;
        }
        // ADR-0400 decision 18 · ADR-0414 decision 2: raised after the disk step and the preset files, before the
        //   agents pass — an `agent.new` judged before it and inserted after it judges again; one inserted before it
        //   is in the roster the port rewrites.
        hold.raise();
        let renames = agent_moves(&self.values.registry.attachments(), pairs);
        if renames.is_empty() {
            return;
        }
        let port = match port.map_or_else(|| self.profile_edit(), Ok) {
            Ok(port) => port,
            Err(refusal) => {
                out.stopped = Some(RelinkStop {
                    path: None,
                    refusal,
                });
                return;
            }
        };
        match port.rewrite_preset_paths(&renames) {
            Ok(changed) => {
                tracing::info!(agents = changed.len(), "agent presets re-pointed by a move");
                out.agents = changed;
            }
            Err(e) => {
                out.stopped = Some(RelinkStop {
                    path: None,
                    refusal: port_refused(e),
                })
            }
        }
    }

    /// The git working tree each of `files` lies in, judged from its folder ([`LibraryService::git_tops`] — one bounded
    /// batch, outside every lock; a row naming a mount from that mount's folder), each `(mount, tree)` once in list
    /// order — a mount may hold several trees. The probe follows links, so a folder inside a mount passes the link
    /// admission first (ADR-0365): behind a link the file has no tree; an admission that gave no answer `Unknown`.
    fn working_trees(&self, files: &[&str]) -> Vec<(String, GitTree)> {
        let mounts = self.gate.with_read(|state| state.mounts.clone());
        // Per file: its mount and the folder to probe; `None` = the admission gave no answer.
        let mut places: Vec<(String, Option<PathBuf>)> = Vec::new();
        for file in files {
            let Ok(path) = LibPath::parse(file) else {
                continue;
            };
            let Some((mount, rest)) = mounts.locate(&path) else {
                continue;
            };
            let folder = match rest.and_then(|_| path.parent()) {
                // ADR-0365: a mount's top — its folder may be a link.
                None => Some(mount.folder.clone()),
                Some(folder) if folder.parent().is_none() => Some(mount.folder.clone()),
                Some(folder) => match self.admit(&mounts, &folder) {
                    Ok(disk) => Some(disk),
                    Err(LibraryRefusal::Refused { reason, .. })
                        if reason.code == PATH_THROUGH_LINK =>
                    {
                        continue
                    }
                    Err(_) => None,
                },
            };
            places.push((mount.name.as_str().to_owned(), folder));
        }
        if places.is_empty() {
            return Vec::new();
        }
        let folders: Vec<PathBuf> = places
            .iter()
            .filter_map(|(_, folder)| folder.clone())
            .collect();
        let mut trees = self.git_tops(&self.io.begin_read(), &folders).into_iter();
        let mut answer: Vec<(String, GitTree)> = Vec::new();
        for (mount, folder) in places {
            let tree = match folder {
                Some(_) => trees.next().expect("one tree per folder asked"),
                None => GitTree::Unknown,
            };
            let row = (mount, tree);
            if !answer.contains(&row) {
                answer.push(row);
            }
        }
        answer
    }
}

/// The library path a delete, a move or a rename may take: a library path below a mount's top, outside the built-in
/// mount, with no dot or temp name — or why not.
fn deletable(given: &str) -> Result<LibPath, Reason> {
    let path = lib_path(given).map_err(reason_of)?;
    // ADR-0400 decision 5
    if path.parent().is_none() {
        return Err(reason_of(virtual_root_read_only(Some(&path))));
    }
    // ADR-0400 decision 20
    if in_builtin(&path) {
        return Err(reason_of(mount_read_only(path.as_str())));
    }
    // ADR-0320: a path under `.trash` is a dot name.
    if !admits_inside(&path) {
        return Err(reason(
            ITEM_PATH_INVALID,
            json!({ "path": path.as_str(), "why": "a dot or temp name" }),
        ));
    }
    Ok(path)
}

/// Sets each target's lone-`SKILL.md` folder from `catalog` (ADR-0339: a file the scan took as a skill's, never a folder
/// of that name) — at the call for its plan and again in the hold for its judgement.
fn classify(catalog: &Catalog, targets: &mut [Target]) {
    for target in targets {
        target.skill_folder = lone_skill_file(catalog, &target.path);
    }
}

/// The skill folder of `path` when the catalog holds `path` as a skill's `SKILL.md` — a file the scan's classifier took
/// as one (a folder of that name is no item; a `SKILL.md` at a mount's top is an instruction).
fn lone_skill_file(catalog: &Catalog, path: &LibPath) -> Option<LibPath> {
    catalog
        .get(&key(path))
        .filter(|item| item.kind == ItemKind::SkillFile)?;
    path.parent()
}

/// `path` spelled as the catalog spells it: its deepest leading part the catalog holds as an item or a folder takes the
/// catalog's spelling, the names below it stay as given (as given throughout when the catalog holds no part of it).
fn catalog_spelling(catalog: &Catalog, path: &LibPath) -> LibPath {
    let names: Vec<&str> = path.as_str().split('/').collect();
    for n in (2..=names.len()).rev() {
        let part =
            LibPath::parse(&names[..n].join("/")).expect("a leading part of a library path is one");
        let at = key(&part);
        let held = catalog
            .get(&at)
            .map(|item| &item.path)
            .or_else(|| catalog.folders().find(|folder| key(folder) == at));
        if let Some(held) = held {
            let rest = &names[n..];
            let spelled = match rest.is_empty() {
                true => held.as_str().to_owned(),
                false => format!("{}/{}", held.as_str(), rest.join("/")),
            };
            return LibPath::parse(&spelled).expect("a re-spelled library path is one");
        }
    }
    path.clone()
}

/// The paths given inside another given path (or given twice) — `(where given, path)` each — with the path of the one it
/// lies in: only that one counts (ADR-0409 · P5).
fn nested(paths: &[(usize, &LibPath)]) -> Vec<(usize, String)> {
    paths
        .iter()
        .enumerate()
        .filter_map(|(i, (at, path))| {
            paths
                .iter()
                .enumerate()
                .find(|(j, (_, other))| {
                    *j != i && within(path, Some(other)) && (key(path) != key(other) || *j < i)
                })
                .map(|(_, (_, other))| (*at, other.as_str().to_owned()))
        })
        .collect()
}

/// A restore row as given: its mount name, bundle id, path in the mount and original virtual path — or the row's
/// outcome (module header).
fn restorable(
    row: &TrashAddress,
) -> Result<(MountName, BundleId, MountPath, LibPath), Outcome<String>> {
    let invalid = |why: String| {
        Outcome::Failed(reason(
            ITEM_PATH_INVALID,
            json!({ "path": escape_controls(&row.path), "why": why }),
        ))
    };
    if is_builtin(&row.mount) {
        return Err(Outcome::Failed(reason_of(mount_read_only(&row.mount))));
    }
    let mount = MountName::parse(&row.mount).map_err(|why| invalid(why.to_string()))?;
    let path = MountPath::parse(&row.path).map_err(|why| invalid(why.to_string()))?;
    let original = virtual_path(&mount, Some(&path));
    // ADR-0409 decision 6: a name that is not a bundle id is foreign.
    let Some(id) = BundleId::parse(&row.bundle) else {
        return Err(Outcome::Failed(not_found_reason(original.as_str())));
    };
    // A staging name or OS litter never goes back into the library.
    if !admits_inside(&original) {
        return Err(Outcome::Failed(reason(
            NOT_RESTORABLE,
            json!({ "path": original.as_str() }),
        )));
    }
    Ok((mount, id, path, original))
}

/// `path` with its last name numbered `n` the keep-both way: `a (2).md` for a file (before its last extension), `a (2)`
/// for a folder or a name without one.
fn numbered(path: &MountPath, n: u32, file: bool) -> MountPath {
    let name = path.file_name();
    let renamed = match name
        .rsplit_once('.')
        .filter(|(stem, _)| file && !stem.is_empty())
    {
        Some((stem, extension)) => format!("{stem} ({n}).{extension}"),
        None => format!("{name} ({n})"),
    };
    let spelled = match path.parent() {
        Some(parent) => format!("{}/{renamed}", parent.as_str()),
        None => renamed,
    };
    MountPath::parse(&spelled).expect("a numbered name of a valid name is valid")
}

/// Sets each request's leaving skill folder from `catalog` ([`leaves_skill`]) — at the call for its plan and again in
/// the hold for its judgement.
fn classify_moves(catalog: &Catalog, requests: &mut [Request]) {
    for request in requests {
        request.skill_folder = leaves_skill(catalog, &request.path, &request.dest);
    }
}

/// Each skill folder of `catalog` by key, with its path and its `SKILL.md` name as on disk.
fn skill_files(catalog: &Catalog) -> HashMap<PathKey, (LibPath, String)> {
    catalog
        .items()
        .filter_map(|item| match &item.content {
            ItemContent::Skill { file_name, .. } => {
                Some((key(&item.path), (item.path.clone(), file_name.clone())))
            }
            _ => None,
        })
        .collect()
}

/// The skill folder of `path` when the catalog holds it as a skill's lone `SKILL.md` ([`lone_skill_file`]) and `dest`
/// takes it away from directly under that folder — another folder (a subfolder of the skill's too — ADR-0339 M110) or
/// a name the classifier does not take for a `SKILL.md`.
fn leaves_skill(catalog: &Catalog, path: &LibPath, dest: &LibPath) -> Option<LibPath> {
    let folder = lone_skill_file(catalog, path)?;
    let stays = dest
        .parent()
        .is_some_and(|parent| key(&parent) == key(&folder))
        && makes_skill(dest.file_name(), &folder);
    (!stays).then_some(folder)
}

/// Whether an agent attaches a path at or below one of `requests`.
fn agents_within(attachments: &[(AgentId, Attachments)], requests: &[Request]) -> bool {
    attachments.iter().any(|(_, attached)| {
        attached.presets.iter().any(|preset| {
            matches!(
                AssetRef::parse(&preset.path),
                Ok(AssetRef::Library(path))
                    if requests.iter().any(|request| within(&path, Some(&request.path)))
            )
        })
    })
}

/// `path` as a move's `pairs` re-point it, in move order — each pair maps what the pairs before it made of the path
/// ([`Pair::map`]), as the catalog's re-keys run one after another; `None` = not moved (or moved to the same
/// spelling).
fn mapped(pairs: &[Pair], path: &LibPath) -> Option<LibPath> {
    let new = pairs
        .iter()
        .fold(path.clone(), |at, pair| pair.map(&at).unwrap_or(at));
    (new.as_str() != path.as_str()).then_some(new)
}

/// The lists of `doc` without the entries a `drop` target takes away, then with every library entry `pairs` maps
/// re-pointed (its origin and tag kept); in a list that changed only the first place of each target stays
/// (ADR-0339). Folder entries untouched.
fn moved_lists(
    doc: &crate::format::PresetDoc,
    pairs: &[Pair],
    drop: &[Target],
) -> crate::format::PresetLists {
    let mut lists = as_read(doc);
    for list in ITEM_LISTS {
        let entries = list_mut(&mut lists, list);
        let before = entries.len();
        entries.retain(|entry| match &entry.target {
            AssetRef::Library(path) => !drop.iter().any(|target| target.takes(path)),
            AssetRef::BuiltIn(_) => true,
        });
        let mut changed = entries.len() != before;
        for entry in entries.iter_mut() {
            let new = match &entry.target {
                AssetRef::Library(path) => mapped(pairs, path),
                AssetRef::BuiltIn(_) => None,
            };
            if let Some(new) = new {
                entry.target = AssetRef::Library(new);
                changed = true;
            }
        }
        if changed {
            let mut seen = HashSet::new();
            entries.retain(|entry| seen.insert(ref_key(&entry.target)));
        }
    }
    lists
}

/// One rename line per attached spelling a move re-points (each once, by platform `same_path`) — the port matches
/// whole paths, so a moved folder is expanded per attachment.
fn agent_moves(attachments: &[(AgentId, Attachments)], pairs: &[Pair]) -> Vec<PresetPathRename> {
    let mut renames: Vec<PresetPathRename> = Vec::new();
    for (_, attached) in attachments {
        for preset in &attached.presets {
            let Ok(AssetRef::Library(path)) = AssetRef::parse(&preset.path) else {
                continue;
            };
            let Some(new) = mapped(pairs, &path) else {
                continue;
            };
            if renames
                .iter()
                .any(|rename| same_path(Path::new(&rename.from), Path::new(&preset.path)))
            {
                continue;
            }
            renames.push(PresetPathRename {
                from: preset.path.clone(),
                to: new.as_str().to_owned(),
            });
        }
    }
    renames
}

/// The path `name` takes in `folder` — one valid library name, no dot or temp name — or `InvalidArgument` ·
/// `item-path-invalid`.
fn new_name(folder: &LibPath, name: &str) -> Result<LibPath, LibraryRefusal> {
    let invalid = |why: String| {
        LibraryRefusal::new(
            RefusalClass::InvalidArgument,
            ITEM_PATH_INVALID,
            json!({ "path": escape_controls(name), "why": why }),
        )
    };
    let dest = LibPath::parse(&format!("{}/{name}", folder.as_str()))
        .map_err(|why| invalid(why.to_string()))?;
    if dest.file_name() != name {
        return Err(invalid("a new name is one name".to_owned()));
    }
    if !admits_inside(&dest) {
        return Err(invalid("a dot or temp name".to_owned()));
    }
    Ok(dest)
}

/// The keep-both name `n` (2, 3 …) of a move's colliding entry — the one place a move's keep-both naming is decided.
fn kept_name(path: &MountPath, n: u32, file: bool) -> MountPath {
    numbered(path, n, file)
}

fn into_itself(request: &Request) -> Reason {
    reason(
        INTO_ITSELF,
        json!({ "path": request.path.as_str(), "to": request.dest.as_str() }),
    )
}

/// `name-exists` naming what is at `to`.
fn name_taken(to: &LibPath) -> Reason {
    reason(NAME_EXISTS, json!({ "path": to.as_str() }))
}

/// A failed no-replace rename of `from` to `to` as an item's reason (a taken name is the caller's).
fn rename_failed(from: &LibPath, to: &LibPath, e: &io::Error) -> Reason {
    let args = json!({ "path": from.as_str(), "to": to.as_str(), "why": e.to_string() });
    match e.kind() {
        io::ErrorKind::CrossesDevices => reason(CROSSES_VOLUMES, args),
        io::ErrorKind::Unsupported => reason(NO_REPLACE_UNSUPPORTED, args),
        io::ErrorKind::NotFound => not_found_reason(from.as_str()),
        _ => reason(LIBRARY_WRITE_FAILED, args),
    }
}

/// `Conflict` · [`SKILL_UNCONFIRMED`] with `plan`.
fn skill_unconfirmed(plan: &DeletePlan) -> LibraryRefusal {
    LibraryRefusal::new(RefusalClass::Conflict, SKILL_UNCONFIRMED, plan.to_json())
}

/// The lists of `doc` without the library entries a target takes away (folder entries untouched).
fn dropped(doc: &crate::format::PresetDoc, targets: &[Target]) -> crate::format::PresetLists {
    let mut lists = as_read(doc);
    for list in ITEM_LISTS {
        list_mut(&mut lists, list).retain(|entry| match &entry.target {
            AssetRef::Library(path) => !targets.iter().any(|target| target.takes(path)),
            AssetRef::BuiltIn(_) => true,
        });
    }
    lists
}

/// A store failure of a trash move as an item's reason.
fn move_failed(path: &LibPath, e: &io::Error) -> Reason {
    let args = json!({ "path": path.as_str(), "why": e.to_string() });
    match e.kind() {
        io::ErrorKind::Unsupported => reason(NO_REPLACE_UNSUPPORTED, args),
        // A link or a file among the names on the way.
        io::ErrorKind::NotADirectory => reason(PATH_THROUGH_LINK, args),
        io::ErrorKind::NotFound => not_found_reason(path.as_str()),
        _ => reason(LIBRARY_WRITE_FAILED, args),
    }
}

/// `Conflict` · [`DELETE_UNCONFIRMED`] with `plan`.
fn unconfirmed(plan: &DeletePlan) -> LibraryRefusal {
    LibraryRefusal::new(RefusalClass::Conflict, DELETE_UNCONFIRMED, plan.to_json())
}

fn reason(code: &str, args: Value) -> Reason {
    Reason {
        code: code.to_owned(),
        args,
        line: None,
    }
}

fn not_found_reason(path: &str) -> Reason {
    reason(NOT_FOUND, json!({ "path": path }))
}

/// The reason of a per-item refusal (an admission or a mount judgement — never the agent store's).
fn reason_of(refusal: LibraryRefusal) -> Reason {
    match refusal {
        LibraryRefusal::Refused { reason, .. } => reason,
        LibraryRefusal::Store(_) => unreachable!("a per-item check refuses with a reason"),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc::{self, Receiver, Sender};
    use std::sync::{Arc, Mutex, Weak};
    use std::thread::{self, JoinHandle};
    use std::time::Duration;

    use engram_dashboard_agent::profile::{
        AgentCommand, AgentProfile, AttachedPreset, ProfileRegistry, ProfileStore,
    };
    use engram_dashboard_agent::profile_edit::{PresetPathRename, ProfileEditError, WorkspaceLock};
    use engram_dashboard_base::sync::lock;
    use engram_dashboard_platform::fs::{link_dir, unlink_dir};
    use engram_dashboard_platform::fs::{path_key, same_path};
    use tempfile::TempDir;

    use super::*;
    use crate::backends::AgentBackends;
    use crate::config::{LibraryConfig, MountPlace, MountSetting};
    use crate::edit::{AddEntries, NewItem};
    use crate::folder::{
        BoundedIo, FolderListing, FolderOps, FolderStatus, LaunchWaits, StdFolderOps, ThreadSpawn,
        IO_LIMIT, LIST_CAP,
    };
    use crate::store::{ContentTag, FsStore, LibraryStore, ScanListing};
    use crate::trash::{RowKind, RowSource, TrashListing, RECORD_NAME, TRASH_UNUSABLE};
    use crate::{
        LibraryValues, LIBRARY_UNAVAILABLE, MOUNT_READ_ONLY, ROOT_GENERATION_STALE,
        VIRTUAL_ROOT_READ_ONLY,
    };

    /// A stat that parks: it tells the sender, then waits for the receiver.
    type Door = (PathBuf, Sender<()>, Receiver<()>);

    /// What runs right after a rename from this path.
    type AfterRename = (PathBuf, Box<dyn FnOnce() + Send>);

    /// [`FsStore`] with eight seams: the record publication fails (a no-replace rename onto the record name) while
    /// `fail_record` is set, a write of a `fail_write` path fails, a rename from a `fail_renames` path fails after its
    /// passes, `after_rename` runs once right after a rename from its path, a `remove_dir` of a `fail_remove` path
    /// fails, the next stat of the door's path parks, every stat of `fail_stat`'s path fails, and `identity_of` `(at,
    /// as)` answers the identity of `as` for `at`; `identities` counts the identity probes, `lists` logs the folders
    /// listed.
    #[derive(Default)]
    struct Disk {
        fail_record: AtomicBool,
        /// A write of one of these paths fails.
        fail_write: Mutex<Vec<PathBuf>>,
        /// A rename from `.0` fails once `.1` renames from it have passed.
        fail_renames: Mutex<Vec<(PathBuf, u32)>>,
        /// Runs once right after a rename from `.0` went through.
        after_rename: Mutex<Option<AfterRename>>,
        /// A `remove_dir` of one of these paths fails.
        fail_remove: Mutex<Vec<PathBuf>>,
        stat_door: Mutex<Option<Door>>,
        fail_stat: Mutex<Option<PathBuf>>,
        identity_of: Mutex<Option<(PathBuf, PathBuf)>>,
        identities: std::sync::atomic::AtomicUsize,
        lists: Mutex<Vec<PathBuf>>,
    }

    impl LibraryStore for Disk {
        fn scan(
            &self,
            root: &Path,
            under: Option<&MountPath>,
            seq: &dyn Fn() -> u64,
        ) -> io::Result<ScanListing> {
            FsStore.scan(root, under, seq)
        }
        fn first_link(&self, root: &Path, rel: &MountPath) -> io::Result<Option<MountPath>> {
            FsStore.first_link(root, rel)
        }
        fn read(&self, path: &Path, cap: u64) -> io::Result<Vec<u8>> {
            FsStore.read(path, cap)
        }
        fn tag(&self, path: &Path) -> io::Result<ContentTag> {
            FsStore.tag(path)
        }
        fn write_atomic(&self, path: &Path, bytes: &[u8]) -> io::Result<()> {
            if lock(&self.fail_write).iter().any(|failing| failing == path) {
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
            if let Some((_, passes)) = lock(&self.fail_renames)
                .iter_mut()
                .find(|(failing, _)| failing == from)
            {
                match passes {
                    0 => return Err(io::Error::other("injected rename failure")),
                    passes => *passes -= 1,
                }
            }
            if self.fail_record.load(Ordering::SeqCst)
                && to.file_name().is_some_and(|name| name == RECORD_NAME)
            {
                return Err(io::Error::other("injected record publication failure"));
            }
            FsStore.rename(from, to)?;
            let after = {
                let mut after = lock(&self.after_rename);
                match after.as_ref() {
                    Some((at, _)) if at == from => after.take(),
                    _ => None,
                }
            };
            if let Some((_, hook)) = after {
                hook();
            }
            Ok(())
        }
        fn copy(&self, from: &Path, to: &Path) -> io::Result<()> {
            FsStore.copy(from, to)
        }
        fn exists(&self, path: &Path) -> bool {
            FsStore.exists(path)
        }
        fn stat(&self, path: &Path) -> io::Result<EntryKind> {
            let door = {
                let mut door = lock(&self.stat_door);
                match door.as_ref() {
                    Some((at, _, _)) if at == path => door.take(),
                    _ => None,
                }
            };
            if let Some((_, entered, release)) = door {
                entered.send(()).unwrap();
                let _ = release.recv_timeout(Duration::from_secs(20));
            }
            if lock(&self.fail_stat).as_deref() == Some(path) {
                return Err(io::Error::other("injected stat failure"));
            }
            FsStore.stat(path)
        }
        fn list(&self, path: &Path) -> io::Result<Vec<(std::ffi::OsString, EntryKind)>> {
            lock(&self.lists).push(path.to_path_buf());
            FsStore.list(path)
        }
        fn remove_dir(&self, path: &Path) -> io::Result<()> {
            if lock(&self.fail_remove)
                .iter()
                .any(|failing| failing == path)
            {
                return Err(io::Error::other("injected remove failure"));
            }
            FsStore.remove_dir(path)
        }
        fn remove_file(&self, path: &Path) -> io::Result<()> {
            FsStore.remove_file(path)
        }
        fn identity(&self, path: &Path) -> io::Result<engram_dashboard_platform::fs::FileIdentity> {
            self.identities.fetch_add(1, Ordering::SeqCst);
            match lock(&self.identity_of).as_ref() {
                Some((at, other)) if at == path => FsStore.identity(other),
                _ => FsStore.identity(path),
            }
        }
    }

    /// [`StdFolderOps`] that logs every path whose canonical path is asked — the overlap probes.
    #[derive(Default)]
    struct Probes {
        canonical: Mutex<Vec<PathBuf>>,
    }

    impl FolderOps for Probes {
        fn stat(&self, p: &Path) -> FolderStatus {
            StdFolderOps.stat(p)
        }
        fn list(&self, p: &Path, cap: usize) -> Result<FolderListing, FolderStatus> {
            StdFolderOps.list(p, cap)
        }
        fn canonical(&self, p: &Path) -> io::Result<PathBuf> {
            lock(&self.canonical).push(p.to_path_buf());
            StdFolderOps.canonical(p)
        }
        fn identity(&self, p: &Path) -> io::Result<engram_dashboard_platform::fs::FileIdentity> {
            StdFolderOps.identity(p)
        }
        fn read(&self, p: &Path, cap: u64) -> io::Result<String> {
            StdFolderOps.read(p, cap)
        }
    }

    /// A profile-edit port over the service's registry: an attachment write or a path rewrite goes through the
    /// registry's public update, the paths an attachment write drops and each rewrite's lines are logged; `entered` is
    /// told when a write starts, which then waits for `release` (when set).
    struct Port {
        registry: Arc<ProfileRegistry>,
        removed: Mutex<Vec<Vec<String>>>,
        renamed: Mutex<Vec<Vec<PresetPathRename>>>,
        entered: Mutex<Option<Sender<()>>>,
        release: Mutex<Option<Receiver<()>>>,
    }

    impl ProfileEditPort for Port {
        fn set_attachments(&self, id: AgentId, next: Attachments) -> Result<(), ProfileEditError> {
            if let Some(entered) = lock(&self.entered).take() {
                entered.send(()).unwrap();
            }
            if let Some(release) = lock(&self.release).take() {
                release
                    .recv_timeout(Duration::from_secs(20))
                    .expect("the test releases the agents pass");
            }
            let before = self
                .registry
                .get(id)
                .ok_or(ProfileEditError::NotFound(id))?
                .presets;
            let dropped: Vec<String> = before
                .iter()
                .filter(|attached| !next.presets.contains(attached))
                .map(|attached| attached.path.clone())
                .collect();
            lock(&self.removed).push(dropped);
            self.registry.try_update_with(id, |stored| {
                stored.default_preset = next.default_preset;
                stored.presets = next.presets;
            })?;
            Ok(())
        }

        fn rewrite_preset_paths(
            &self,
            renames: &[PresetPathRename],
        ) -> Result<Vec<AgentId>, ProfileEditError> {
            if let Some(entered) = lock(&self.entered).take() {
                entered.send(()).unwrap();
            }
            if let Some(release) = lock(&self.release).take() {
                release
                    .recv_timeout(Duration::from_secs(20))
                    .expect("the test releases the agents pass");
            }
            lock(&self.renamed).push(renames.to_vec());
            let mut changed = Vec::new();
            for profile in self.registry.list() {
                let mut seen = std::collections::HashSet::new();
                let next: Vec<AttachedPreset> = profile
                    .presets
                    .iter()
                    .map(|attached| {
                        match renames.iter().find(|rename| {
                            same_path(Path::new(&rename.from), Path::new(&attached.path))
                        }) {
                            Some(rename) => AttachedPreset {
                                path: rename.to.clone(),
                            },
                            None => attached.clone(),
                        }
                    })
                    .filter(|attached| seen.insert(path_key(Path::new(&attached.path))))
                    .collect();
                if next != profile.presets {
                    self.registry
                        .try_update_with(profile.id, |stored| stored.presets = next)?;
                    changed.push(profile.id);
                }
            }
            changed.sort_unstable();
            Ok(changed)
        }

        fn remove_preset_paths(&self, _: &[String]) -> Result<Vec<AgentId>, ProfileEditError> {
            unreachable!("a delete writes the confirmed agents one by one")
        }

        fn workspace_lock(&self, _: AgentId) -> Result<Option<WorkspaceLock>, ProfileEditError> {
            Ok(None)
        }
    }

    struct Fixed(Vec<AgentProfile>);

    impl ProfileStore for Fixed {
        fn save(
            &self,
            _: &[AgentProfile],
        ) -> Result<(), engram_dashboard_agent::profile::StoreError> {
            Ok(())
        }
        fn load(&self) -> Vec<AgentProfile> {
            self.0.clone()
        }
    }

    fn agent(presets: &[&str]) -> AgentProfile {
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
        profile.presets = presets
            .iter()
            .map(|path| AttachedPreset {
                path: (*path).to_owned(),
            })
            .collect();
        profile
    }

    fn preset(body: &str) -> String {
        format!("kind: preset\nversion: 1\n{body}")
    }

    fn lp(text: &str) -> LibPath {
        LibPath::parse(text).unwrap()
    }

    /// Two mounts in a temp folder — `library` at `lib` and `team` at `lib2` — on [`Disk`] and [`Probes`], with the
    /// port plugged.
    struct World {
        _temp: TempDir,
        base: PathBuf,
        links: Mutex<Vec<PathBuf>>,
        disk: Arc<Disk>,
        probes: Arc<Probes>,
        port: Arc<Port>,
        svc: Arc<LibraryService>,
    }

    impl World {
        fn new() -> World {
            World::make(Vec::new(), IO_LIMIT)
        }

        fn with(profiles: Vec<AgentProfile>) -> World {
            World::make(profiles, IO_LIMIT)
        }

        fn make(profiles: Vec<AgentProfile>, limit: Duration) -> World {
            let temp = tempfile::tempdir().unwrap();
            let base = dunce::canonicalize(temp.path()).unwrap();
            for dir in ["lib", "lib2", "state", "keep"] {
                fs::create_dir_all(base.join(dir)).unwrap();
            }
            let config = LibraryConfig::mounted(
                &base.join("state"),
                vec![
                    MountSetting {
                        name: MountName::parse("library").unwrap(),
                        place: MountPlace::Explicit(base.join("lib")),
                    },
                    MountSetting {
                        name: MountName::parse("team").unwrap(),
                        place: MountPlace::Explicit(base.join("lib2")),
                    },
                ],
            );
            let registry = Arc::new(ProfileRegistry::new(Arc::new(Fixed(profiles))));
            let values = LibraryValues {
                registry: Arc::clone(&registry),
                backends: Arc::new(AgentBackends),
                config,
                default_root: base.join("default-library"),
                daemon_id: "test-daemon".to_owned(),
                root_generation_base: 7,
                builtin_dir: None,
                launch_root: base.join("launch"),
                home: None,
                env: Arc::new(|_| None),
            };
            let probes = Arc::new(Probes::default());
            let io = BoundedIo::new(
                limit,
                LaunchWaits::LAUNCH,
                LIST_CAP,
                Arc::clone(&probes) as Arc<dyn FolderOps>,
                Arc::new(ThreadSpawn),
            );
            let disk = Arc::new(Disk::default());
            let svc = Arc::new(LibraryService::new(
                values,
                Arc::clone(&disk) as Arc<dyn LibraryStore>,
                io,
            ));
            let port = Arc::new(Port {
                registry,
                removed: Mutex::new(Vec::new()),
                renamed: Mutex::new(Vec::new()),
                entered: Mutex::new(None),
                release: Mutex::new(None),
            });
            let weak: Weak<Port> = Arc::downgrade(&port);
            svc.plug_profile_edit(weak);
            World {
                _temp: temp,
                base,
                links: Mutex::new(Vec::new()),
                disk,
                probes,
                port,
                svc,
            }
        }

        /// `rel` (`/` names) below the temp folder.
        fn at(&self, rel: &str) -> PathBuf {
            rel.split('/')
                .fold(self.base.clone(), |path, name| path.join(name))
        }

        fn put(&self, rel: &str, text: &str) {
            let path = self.at(rel);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        }

        fn text(&self, rel: &str) -> String {
            fs::read_to_string(self.at(rel)).unwrap()
        }

        fn there(&self, rel: &str) -> bool {
            fs::symlink_metadata(self.at(rel)).is_ok()
        }

        /// A link at `rel` to the folder `to` (both below the temp folder) — removed on drop.
        fn link(&self, to: &str, rel: &str) {
            link_dir(&self.at(to), &self.at(rel)).unwrap();
            lock(&self.links).push(self.at(rel));
        }

        fn scan(&self) {
            self.svc.first_scan_join().unwrap();
        }

        fn kind(&self, path: &str) -> Option<ItemKind> {
            self.svc.with_read(|state| {
                state
                    .catalog
                    .get(&key(&lp(path)))
                    .map(|item| item.kind.clone())
            })
        }

        fn delete(&self, paths: &[&str]) -> DeleteAnswer {
            self.svc.delete(paths, false, false, None, None).unwrap()
        }

        fn presets_of(&self, id: AgentId) -> Vec<String> {
            self.svc
                .values
                .registry
                .get(id)
                .unwrap()
                .presets
                .into_iter()
                .map(|attached| attached.path)
                .collect()
        }

        fn removed(&self) -> Vec<Vec<String>> {
            lock(&self.port.removed).clone()
        }

        fn mv(&self, paths: &[&str], to: &str, on_conflict: Option<OnConflict>) -> MoveAnswer {
            self.svc
                .move_items(paths, to, on_conflict, false, false, None, None)
                .unwrap()
        }

        fn rename(&self, path: &str, name: &str) -> MoveAnswer {
            self.svc
                .rename(path, name, false, false, None, None)
                .unwrap()
        }

        /// The name and folder of the skill item at `path`.
        fn skill(&self, path: &str) -> (String, String) {
            self.svc.with_read(|state| {
                match state.catalog.get(&key(&lp(path))).map(|item| &item.content) {
                    Some(ItemContent::Skill { name, folder, .. }) => (name.clone(), folder.clone()),
                    other => panic!("{path} is no skill: {other:?}"),
                }
            })
        }

        /// The presets the catalog has listing `path`, by path.
        fn listing(&self, path: &str) -> Vec<String> {
            self.svc.with_read(|state| {
                state
                    .catalog
                    .presets_listing(&key(&lp(path)))
                    .iter()
                    .map(|item| item.path.as_str().to_owned())
                    .collect()
            })
        }

        fn view(&self) -> TrashListing {
            self.svc.trash_view()
        }
    }

    impl Drop for World {
        fn drop(&mut self) {
            for link in lock(&self.links).iter() {
                if fs::symlink_metadata(link).is_ok() {
                    if let Err(e) = unlink_dir(link) {
                        eprintln!("could not remove the test link {}: {e}", link.display());
                    }
                }
            }
        }
    }

    fn done<T: Clone + std::fmt::Debug>(outcome: &Outcome<T>) -> T {
        match outcome {
            Outcome::Done(value) => value.clone(),
            other => panic!("not done: {other:?}"),
        }
    }

    fn failed_code<T: std::fmt::Debug>(outcome: &Outcome<T>) -> String {
        match outcome {
            Outcome::Failed(reason) => reason.code.clone(),
            other => panic!("not failed: {other:?}"),
        }
    }

    /// The refusal's code and args.
    fn refused<T: std::fmt::Debug>(answer: Result<T, LibraryRefusal>) -> (String, Value) {
        match answer {
            Err(LibraryRefusal::Refused { reason, .. }) => (reason.code, reason.args),
            other => panic!("not refused: {other:?}"),
        }
    }

    fn digest_of(args: &Value) -> String {
        args["plan_digest"].as_str().unwrap().to_owned()
    }

    fn on_thread<T: Send + 'static>(
        svc: &Arc<LibraryService>,
        f: impl FnOnce(&LibraryService) -> T + Send + 'static,
    ) -> JoinHandle<T> {
        let svc = Arc::clone(svc);
        thread::spawn(move || f(&svc))
    }

    // ── delete ──

    // ADR-0409 decisions 5–8 · ADR-0400 decision 23: each item goes into its own mount's trash at its path there, one
    //   bundle id for the operation, a record naming the roots; the catalog loses them; the view shows both mounts.
    #[test]
    fn a_delete_moves_each_item_into_its_own_mounts_trash_under_one_bundle_id_with_a_record() {
        let world = World::new();
        world.put("lib/rules/a.md", "a");
        world.put("lib2/b.md", "b");
        world.scan();

        let answer = world.delete(&["library/rules/a.md", "team/b.md"]);

        let (a, b) = (
            done(&answer.items[0].outcome),
            done(&answer.items[1].outcome),
        );
        assert_eq!(
            (a.mount.as_str(), a.path.as_str()),
            ("library", "rules/a.md")
        );
        assert_eq!((b.mount.as_str(), b.path.as_str()), ("team", "b.md"));
        assert_eq!(a.bundle, b.bundle, "one id per operation");
        assert!(BundleId::parse(&a.bundle).is_some());
        assert_eq!(
            world.text(&format!("lib/.trash/{}/rules/a.md", a.bundle)),
            "a"
        );
        assert_eq!(world.text(&format!("lib2/.trash/{}/b.md", b.bundle)), "b");
        assert!(!world.there("lib/rules/a.md") && !world.there("lib2/b.md"));
        assert!(world.there("lib/rules"), "only the item moved");
        let id = BundleId::parse(&a.bundle).unwrap();
        match read_record(&FsStore, &world.at("lib"), &id) {
            RecordRead::Roots(roots) => {
                let paths: Vec<&str> = roots.iter().map(|root| root.root.path.as_str()).collect();
                assert_eq!(paths, ["rules/a.md"]);
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(world.kind("library/rules/a.md"), None);
        assert_eq!(world.kind("team/b.md"), None);

        let view = world.view();
        let names: Vec<&str> = view
            .mounts
            .iter()
            .map(|mount| mount.mount.as_str())
            .collect();
        assert_eq!(names, ["library", "team"], "one combined view");
        let library = &view.mounts[0];
        let bundles = library.view.as_ref().unwrap();
        assert_eq!(bundles.len(), 1);
        assert_eq!(bundles[0].id, id);
        assert_eq!(bundles[0].source, RowSource::Record);
        assert_eq!(
            library.original(&bundles[0].rows[0]).as_str(),
            "library/rules/a.md"
        );
        let team = view.mounts[1].view.as_ref().unwrap();
        assert_eq!(team[0].rows[0].path.as_str(), "b.md");
    }

    #[test]
    fn a_bundle_id_taken_in_a_later_mount_draws_a_new_one_there() {
        let world = World::new();
        world.put("lib/a.md", "a");
        world.put("lib2/b.md", "b");
        world.scan();
        let wanted = BundleId::parse("20261123T081500Z-a1b2c3").unwrap();
        fs::create_dir_all(world.at(&format!("lib2/.trash/{}", wanted.as_str()))).unwrap();
        WANTED_BUNDLE.with(|id| *id.borrow_mut() = Some(wanted.clone()));

        let answer = world.delete(&["library/a.md", "team/b.md"]);

        let (a, b) = (
            done(&answer.items[0].outcome),
            done(&answer.items[1].outcome),
        );
        assert_eq!(a.bundle, wanted.as_str());
        assert_ne!(b.bundle, wanted.as_str());
        assert_eq!(
            BundleId::parse(&b.bundle).unwrap().deleted_at(),
            wanted.deleted_at(),
            "the new id keeps the operation's time"
        );
        assert_eq!(
            fs::read_dir(world.at(&format!("lib2/.trash/{}", wanted.as_str())))
                .unwrap()
                .count(),
            0
        );
    }

    // P5 · ADR-0400 decisions 5 and 20 · ADR-0320: per item — a mount's top, the built-in mount, a path under .trash and
    //   a path that is not there refuse; a child given with its folder is skipped; the rest goes.
    #[test]
    fn a_delete_answers_per_item_and_refuses_tops_the_builtin_trash_paths_and_children_of_given_folders(
    ) {
        let world = World::new();
        world.put("lib/d/x.md", "x");
        world.put("lib/keep.md", "k");
        world.put("lib/.trash/20261123T081500Z-a1b2c3/old.md", "trashed");
        world.scan();

        let answer = world.delete(&[
            "library",
            "engram/default",
            "library/.trash/20261123T081500Z-a1b2c3/old.md",
            "library/d/x.md",
            "library/d",
            "library/missing.md",
            "library/keep.md",
            "../x",
        ]);

        let items = &answer.items;
        assert_eq!(failed_code(&items[0].outcome), VIRTUAL_ROOT_READ_ONLY);
        assert_eq!(failed_code(&items[1].outcome), MOUNT_READ_ONLY);
        assert_eq!(failed_code(&items[2].outcome), ITEM_PATH_INVALID);
        assert_eq!(
            items[3].outcome,
            Outcome::Skipped(Skip::Inside("library/d".to_owned()))
        );
        assert_eq!(done(&items[4].outcome).path, "d");
        assert_eq!(failed_code(&items[5].outcome), NOT_FOUND);
        assert_eq!(done(&items[6].outcome).path, "keep.md");
        assert_eq!(failed_code(&items[7].outcome), ITEM_PATH_INVALID);
        assert!(world.there("lib/.trash/20261123T081500Z-a1b2c3/old.md"));
        let bundle = done(&items[4].outcome).bundle;
        assert_eq!(world.text(&format!("lib/.trash/{bundle}/d/x.md")), "x");
    }

    // ADR-0339 · ADR-0409 decision 2: referenced and no yes — one conflict with the plan and the two forms, nothing moved;
    //   plain yes moves it and keeps the references (they read as not found until restored).
    #[test]
    fn a_referenced_delete_without_yes_is_refused_with_the_plan_and_plain_yes_keeps_the_references()
    {
        let world = World::new();
        world.put("lib/a.md", "a");
        world.put("lib/P.yaml", &preset("instructions:\n- library/a.md\n"));
        world.scan();

        let (code, args) = refused(
            world
                .svc
                .delete(&["library/a.md"], false, false, None, None),
        );
        assert_eq!(code, DELETE_UNCONFIRMED);
        assert_eq!(args["plan"]["presets"], json!(["library/P.yaml"]));
        assert_eq!(args["forms"]["keep"], json!({ "yes": true }));
        assert_eq!(args["forms"]["detach"]["plan_digest"], args["plan_digest"]);
        assert!(world.there("lib/a.md"));
        let (code, _) = refused(world.svc.delete(
            &["library/a.md"],
            false,
            true,
            Some(&digest_of(&args)),
            None,
        ));
        assert_eq!(code, DELETE_UNCONFIRMED, "detachRefs without yes is no yes");

        let before = world.text("lib/P.yaml");
        let answer = world
            .svc
            .delete(&["library/a.md"], true, false, None, None)
            .unwrap();

        done(&answer.items[0].outcome);
        assert!(!world.there("lib/a.md"));
        assert_eq!(world.text("lib/P.yaml"), before, "the reference is kept");
        assert!(answer.written.is_empty() && answer.detached.is_empty());
        let listing: Vec<String> = world.svc.with_read(|state| {
            state
                .catalog
                .presets_listing(&key(&lp("library/a.md")))
                .iter()
                .map(|item| item.path.as_str().to_owned())
                .collect()
        });
        assert_eq!(listing, ["library/P.yaml"]);
        assert_eq!(
            world.kind("library/a.md"),
            None,
            "the reference now reads as not found"
        );
    }

    // ADR-0339 M105 · ADR-0409: detachRefs drops the lines of every deleted path (a folder: everything inside), leaves a
    //   preset trashed in the same operation as it is, and hands only the deleted preset paths to the port.
    #[test]
    fn a_delete_with_detach_refs_drops_every_deleted_path_and_hands_only_preset_paths_to_the_port()
    {
        let attached = agent(&["library/d/inner.yaml", "library/P.yaml"]);
        let id = attached.id;
        let world = World::with(vec![attached]);
        world.put("lib/d/x.md", "x");
        let inner = preset("instructions:\n- library/d/x.md\n");
        world.put("lib/d/inner.yaml", &inner);
        world.put("lib/keep.md", "k");
        world.put(
            "lib/P.yaml",
            &preset("presets:\n- library/d/inner.yaml\ninstructions:\n- library/d/x.md\n- library/keep.md\n"),
        );
        world.scan();

        let (code, args) = refused(world.svc.delete(&["library/d"], true, true, None, None));
        assert_eq!(
            code, DELETE_UNCONFIRMED,
            "the detach form carries the digest"
        );
        assert_eq!(
            args["plan"]["presets"],
            json!(["library/P.yaml"]),
            "a preset deleted with it is no referrer"
        );
        assert_eq!(args["plan"]["agents"][0]["id"], json!(id.to_string()));

        let answer = world
            .svc
            .delete(&["library/d"], true, true, Some(&digest_of(&args)), None)
            .unwrap();

        let bundle = done(&answer.items[0].outcome).bundle;
        assert_eq!(answer.detached, ["library/d"]);
        assert_eq!(answer.written, ["library/P.yaml"]);
        assert_eq!(answer.agents, [id]);
        assert!(answer.stopped.is_none() && answer.skipped.is_empty());
        let p = world.text("lib/P.yaml");
        assert!(!p.contains("library/d/"), "{p}");
        assert!(p.contains("library/keep.md"), "{p}");
        assert_eq!(
            world.text(&format!("lib/.trash/{bundle}/d/inner.yaml")),
            inner,
            "not rewritten in the trash"
        );
        assert_eq!(world.removed(), [vec!["library/d/inner.yaml".to_owned()]]);
        assert!(
            lock(&world.port.renamed).is_empty(),
            "a delete re-points nothing"
        );
        assert_eq!(world.presets_of(id), ["library/P.yaml"]);
    }

    // ADR-0339 (a lone SKILL.md counts its skill folder's references; agents never) · ADR-0409 M3: the folder is no skill
    //   in the catalog right after the call, without a refresh.
    #[test]
    fn a_lone_skill_md_takes_its_folders_references_never_calls_the_port_and_the_folder_is_no_skill_at_once(
    ) {
        let attached = agent(&["library/P.yaml", "library/s/SKILL.md"]);
        let id = attached.id;
        let world = World::with(vec![attached]);
        world.put(
            "lib/s/SKILL.md",
            "---\nname: s\ndescription: d\n---\nbody\n",
        );
        world.put("lib/s/ref.md", "r");
        world.put(
            "lib/P.yaml",
            &preset("instructions:\n- library/s/ref.md\nskills:\n- library/s\n"),
        );
        world.scan();
        assert_eq!(world.kind("library/s"), Some(ItemKind::Skill));

        let (_, args) =
            refused(
                world
                    .svc
                    .delete(&["library/s/SKILL.md"], false, false, None, None),
            );
        assert_eq!(args["plan"]["presets"], json!(["library/P.yaml"]));
        assert_eq!(args["plan"]["agents"], json!([]));

        let answer = world
            .svc
            .delete(
                &["library/s/SKILL.md"],
                true,
                true,
                Some(&digest_of(&args)),
                None,
            )
            .unwrap();

        done(&answer.items[0].outcome);
        assert_eq!(world.kind("library/s"), None, "no skill any more");
        assert_eq!(world.kind("library/s/ref.md"), Some(ItemKind::Instruction));
        let p = world.text("lib/P.yaml");
        assert!(!p.contains("- library/s\n"), "{p}");
        assert!(p.contains("library/s/ref.md"), "{p}");
        assert!(
            world.removed().is_empty() && lock(&world.port.renamed).is_empty(),
            "a SKILL.md case never calls the port"
        );
        assert!(answer.agents.is_empty());
        assert_eq!(
            world.presets_of(id),
            ["library/P.yaml", "library/s/SKILL.md"],
            "an attached SKILL.md is no agent reference of the delete"
        );
    }

    // ADR-0409 decision 2 (UD2): a preset that came to reference the item after the conflict makes the old digest stale.
    #[test]
    fn a_detach_whose_plan_grew_after_the_conflict_is_asked_again_with_the_fresh_plan() {
        let world = World::new();
        world.put("lib/a.md", "a");
        world.put("lib/P.yaml", &preset("instructions:\n- library/a.md\n"));
        world.put("lib/R.yaml", &preset(""));
        world.scan();
        let (_, args) = refused(
            world
                .svc
                .delete(&["library/a.md"], false, false, None, None),
        );
        let entries = AddEntries::Items(vec![NewItem {
            path: "library/a.md".to_owned(),
            backend: None,
        }]);
        world
            .svc
            .preset_add(&lp("library/R.yaml"), &entries, None, None)
            .unwrap();
        let (p, r) = (world.text("lib/P.yaml"), world.text("lib/R.yaml"));

        let (code, fresh) =
            refused(
                world
                    .svc
                    .delete(&["library/a.md"], true, true, Some(&digest_of(&args)), None),
            );

        assert_eq!(code, DELETE_UNCONFIRMED);
        assert_eq!(
            fresh["plan"]["presets"],
            json!(["library/P.yaml", "library/R.yaml"])
        );
        assert_ne!(digest_of(&fresh), digest_of(&args));
        assert!(world.there("lib/a.md"));
        assert_eq!(
            (world.text("lib/P.yaml"), world.text("lib/R.yaml")),
            (p, r),
            "nothing written"
        );
    }

    // ADR-0405 decision 1 · ADR-0409 decision 2: an edit landing between the confirming call's plan and its hold is seen
    //   inside the hold — the conflict again, nothing moved or written.
    #[test]
    fn a_detach_whose_plan_changed_before_its_hold_is_asked_again_and_writes_nothing() {
        let world = World::new();
        world.put("lib/a.md", "a");
        world.put("lib/P.yaml", &preset("instructions:\n- library/a.md\n"));
        world.put("lib/S.yaml", &preset(""));
        world.scan();
        let (_, args) = refused(
            world
                .svc
                .delete(&["library/a.md"], false, false, None, None),
        );
        let svc = Arc::clone(&world.svc);
        BEFORE_HOLD.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move || {
                let entries = AddEntries::Items(vec![NewItem {
                    path: "library/a.md".to_owned(),
                    backend: None,
                }]);
                svc.preset_add(&lp("library/S.yaml"), &entries, None, None)
                    .unwrap();
            }));
        });
        let p = world.text("lib/P.yaml");

        let (code, fresh) =
            refused(
                world
                    .svc
                    .delete(&["library/a.md"], true, true, Some(&digest_of(&args)), None),
            );

        assert_eq!(code, DELETE_UNCONFIRMED);
        assert_eq!(
            fresh["plan"]["presets"],
            json!(["library/P.yaml", "library/S.yaml"])
        );
        assert!(world.there("lib/a.md"), "nothing moved");
        assert!(!world.there("lib/.trash"), "no bundle made");
        assert_eq!(world.text("lib/P.yaml"), p, "nothing written");
        assert_eq!(world.svc.gate.trash().waiting(), 0);
    }

    // ADR-0400 decision 18: an agent.new inserted before the delete loses the preset in the agents pass; one judged before
    //   it and inserted while the pass runs (after the raise) finds the number moved, judges again and gets not-found.
    #[test]
    fn an_agent_new_judged_before_a_detaching_delete_loses_the_preset_or_judges_again() {
        let world = World::new();
        world.put("lib/S.yaml", &preset(""));
        world.scan();
        let svc = &world.svc;
        let presets = ["library/S.yaml".to_owned()];
        let judged = svc.judge_new_presets(&agent(&[]), &presets, None).unwrap();
        let mut early = agent(&[]);
        early.presets = judged.attachments.presets.clone();
        let early_id = early.id;
        let at = judged.change.unwrap();
        assert!(svc
            .values
            .registry
            .try_upsert_if(early, || svc.change_number() == at)
            .unwrap());
        let (_, args) = refused(svc.delete(&["library/S.yaml"], false, false, None, None));
        let digest = digest_of(&args);
        let late_judged = svc.judge_new_presets(&agent(&[]), &presets, None).unwrap();
        let late_at = late_judged.change.unwrap();
        let (entered_tx, entered) = mpsc::channel();
        let (release, release_rx) = mpsc::channel();
        *lock(&world.port.entered) = Some(entered_tx);
        *lock(&world.port.release) = Some(release_rx);

        let delete = on_thread(svc, move |svc| {
            svc.delete(&["library/S.yaml"], true, true, Some(&digest), None)
        });
        entered.recv_timeout(Duration::from_secs(10)).unwrap();
        let mut late = agent(&[]);
        late.presets = late_judged.attachments.presets;
        let late_id = late.id;
        assert!(
            !svc.values
                .registry
                .try_upsert_if(late, || svc.change_number() == late_at)
                .unwrap(),
            "the raised number refuses the insert"
        );
        release.send(()).unwrap();
        let answer = delete.join().unwrap().unwrap();

        assert_eq!(answer.agents, [early_id]);
        assert!(world.presets_of(early_id).is_empty());
        assert!(svc.values.registry.get(late_id).is_none());
        let again = svc.judge_new_presets(&agent(&[]), &presets, None).unwrap();
        assert!(again.attachments.presets.is_empty());
        assert_eq!(again.skipped[0].reason.code, NOT_FOUND);
    }

    // ADR-0409 decision 2: the reference fix keeps to the confirmed plan — a preset inside a target that failed to move
    //   was no referrer then and is not rewritten (its comment survives); the confirmed referrer is.
    #[test]
    fn a_detach_rewrites_only_the_confirmed_presets_never_one_inside_a_target_that_stayed() {
        let world = World::new();
        world.put("lib/a.md", "a");
        world.put("lib/P.yaml", &preset("instructions:\n- library/a.md\n"));
        let inner = preset("instructions:\n- library/a.md\n# kept only if never rewritten\n");
        world.put("lib2/d/Q.yaml", &inner);
        world.put("lib2/.trash", "a file where the trash folder goes");
        world.scan();
        let paths = ["library/a.md", "team/d"];
        let (_, args) = refused(world.svc.delete(&paths, false, false, None, None));
        assert_eq!(args["plan"]["presets"], json!(["library/P.yaml"]));

        let answer = world
            .svc
            .delete(&paths, true, true, Some(&digest_of(&args)), None)
            .unwrap();

        done(&answer.items[0].outcome);
        assert_eq!(failed_code(&answer.items[1].outcome), TRASH_UNUSABLE);
        assert_eq!(answer.written, ["library/P.yaml"]);
        assert!(!world.text("lib/P.yaml").contains("library/a.md"));
        assert_eq!(world.text("lib2/d/Q.yaml"), inner, "never rewritten");
    }

    // ADR-0409 decision 2 · ADR-0400 decision 18: an agent inserted after the hold's judgement and before its raise (an
    //   agent.new judged before the delete) is no agent of the confirmed plan — it keeps its attachment; the confirmed
    //   one loses it.
    #[test]
    fn an_agent_inserted_after_the_holds_judgement_keeps_its_attachment_and_the_confirmed_one_loses_it(
    ) {
        let confirmed = agent(&["library/S.yaml"]);
        let confirmed_id = confirmed.id;
        let world = World::with(vec![confirmed]);
        world.put("lib/S.yaml", &preset(""));
        world.scan();
        let svc = &world.svc;
        let (_, args) = refused(svc.delete(&["library/S.yaml"], false, false, None, None));
        let digest = digest_of(&args);
        let presets = ["library/S.yaml".to_owned()];
        let judged = svc.judge_new_presets(&agent(&[]), &presets, None).unwrap();
        let at = judged.change.unwrap();
        let (entered_tx, entered) = mpsc::channel();
        let (release, release_rx) = mpsc::channel();
        // The hold's no-follow stat of the item comes after its judgement and before its raise.
        *lock(&world.disk.stat_door) = Some((world.at("lib/S.yaml"), entered_tx, release_rx));

        let delete = on_thread(svc, move |svc| {
            svc.delete(&["library/S.yaml"], true, true, Some(&digest), None)
        });
        entered.recv_timeout(Duration::from_secs(10)).unwrap();
        let mut late = agent(&[]);
        late.presets = judged.attachments.presets;
        let late_id = late.id;
        assert!(
            svc.values
                .registry
                .try_upsert_if(late, || svc.change_number() == at)
                .unwrap(),
            "inserted before the raise"
        );
        release.send(()).unwrap();
        let answer = delete.join().unwrap().unwrap();

        done(&answer.items[0].outcome);
        assert_eq!(answer.agents, [confirmed_id]);
        assert!(world.presets_of(confirmed_id).is_empty());
        assert_eq!(world.presets_of(late_id), ["library/S.yaml"]);
    }

    // ADR-0339: a folder named SKILL.md is a folder — what is inside it is referenced through it (agents included), and a
    //   reference to the folder above is none of its references.
    #[test]
    fn a_folder_named_skill_md_is_deleted_as_a_folder_not_as_a_lone_skill_file() {
        let attached = agent(&["library/s/SKILL.md/inner.yaml"]);
        let id = attached.id;
        let world = World::with(vec![attached]);
        world.put("lib/s/SKILL.md/x.md", "x");
        world.put("lib/s/SKILL.md/inner.yaml", &preset(""));
        world.put(
            "lib/P.yaml",
            &preset("instructions:\n- library/s/SKILL.md/x.md\n"),
        );
        world.put("lib/Q.yaml", &preset("skills:\n- library/s\n"));
        world.scan();

        let (code, args) =
            refused(
                world
                    .svc
                    .delete(&["library/s/SKILL.md"], false, false, None, None),
            );

        assert_eq!(code, DELETE_UNCONFIRMED);
        assert_eq!(args["plan"]["presets"], json!(["library/P.yaml"]));
        assert_eq!(args["plan"]["agents"][0]["id"], json!(id.to_string()));
    }

    // ADR-0409 decision 2: the digest pins the items too — one answered for [a] does not confirm [a, b].
    #[test]
    fn a_digest_answered_for_some_items_does_not_confirm_a_call_with_more() {
        let world = World::new();
        world.put("lib/a.md", "a");
        world.put("lib/b.md", "b");
        world.put("lib/P.yaml", &preset("instructions:\n- library/a.md\n"));
        world.scan();
        let (_, args) = refused(
            world
                .svc
                .delete(&["library/a.md"], false, false, None, None),
        );

        let (code, fresh) = refused(world.svc.delete(
            &["library/a.md", "library/b.md"],
            true,
            true,
            Some(&digest_of(&args)),
            None,
        ));

        assert_eq!(code, DELETE_UNCONFIRMED);
        assert_ne!(digest_of(&fresh), digest_of(&args));
        assert!(world.there("lib/a.md") && world.there("lib/b.md"));
    }

    // ADR-0409 decision 8: the trash row and the record keep the catalog's spelling and a restore brings the record's
    //   spelling back, whatever case the caller used — where names compare without case (the platform's rule).
    #[test]
    fn a_delete_and_a_restore_keep_the_names_as_the_library_spells_them() {
        if !same_path(Path::new("A"), Path::new("a")) {
            // Names compare by case here: another spelling names another item.
            return;
        }
        let world = World::new();
        world.put("lib/Rules/A.md", "a");
        world.scan();
        let names = |rel: &str| {
            let mut names: Vec<String> = fs::read_dir(world.at(rel))
                .unwrap()
                .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
                .filter(|name| name != RECORD_NAME)
                .collect();
            names.sort();
            names
        };

        let address = done(&world.delete(&["library/rules/a.md"]).items[0].outcome);

        assert_eq!(address.path, "Rules/A.md");
        assert_eq!(names(&format!("lib/.trash/{}", address.bundle)), ["Rules"]);
        fs::remove_dir(world.at("lib/Rules")).unwrap();
        let row = TrashAddress {
            path: "rules/a.md".to_owned(),
            ..address
        };
        let answer = world.svc.restore(&[row], false, None).unwrap();

        assert_eq!(done(&answer.items[0].outcome), "library/Rules/A.md");
        assert_eq!(names("lib"), [".trash", "Rules"]);
        assert_eq!(names("lib/Rules"), ["A.md"]);
    }

    // ADR-0339 · ADR-0409 decision 2: the hold reads the lone-SKILL.md kind again — a SKILL.md that became its folder's
    //   skill file after the call's plan (restored and walked in between) brings the folder's references into the hold's
    //   judgement: the conflict again with the fresh plan, nothing moved.
    #[test]
    fn a_skill_md_that_became_a_skills_after_the_plan_is_judged_as_one_in_the_hold() {
        let world = World::new();
        world.put("lib/s/ref.md", "r");
        world.put("lib/P.yaml", &preset("skills:\n- library/s\n"));
        world.scan();
        let svc = Arc::clone(&world.svc);
        let skill_md = world.at("lib/s/SKILL.md");
        BEFORE_HOLD.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move || {
                fs::write(&skill_md, "---\nname: s\ndescription: d\n---\nbody\n").unwrap();
                svc.refresh(Some(&lp("library/s")));
            }));
        });

        let (code, fresh) =
            refused(
                world
                    .svc
                    .delete(&["library/s/SKILL.md"], false, false, None, None),
            );

        assert_eq!(code, DELETE_UNCONFIRMED);
        assert_eq!(fresh["plan"]["presets"], json!(["library/P.yaml"]));
        assert!(world.there("lib/s/SKILL.md"), "nothing moved");
        assert!(!world.there("lib/.trash"), "no bundle made");
        assert_eq!(world.kind("library/s"), Some(ItemKind::Skill));
    }

    // Where two disk entries could share one catalog key, the delete acts on the entry it was given: the catalog's
    //   spelling is taken only when the store says both spellings are one entry (here the store answers another identity
    //   for the catalog's spelling). Only where names compare without case (the platform's rule).
    #[test]
    fn a_delete_keeps_the_given_spelling_when_the_catalogs_names_another_entry() {
        if !same_path(Path::new("A"), Path::new("a")) {
            // Names compare by case here: another spelling names another item.
            return;
        }
        let world = World::new();
        world.put("lib/Rules/A.md", "a");
        world.put("keep/other.md", "another entry");
        world.scan();
        *lock(&world.disk.identity_of) =
            Some((world.at("lib/Rules/A.md"), world.at("keep/other.md")));

        let address = done(&world.delete(&["library/rules/a.md"]).items[0].outcome);

        assert_eq!(address.path, "rules/a.md");
    }

    // ADR-0365: a case-variant delete whose catalogued parent became a link is refused path-through-link by the
    //   no-follow admission before any link-following identity probe runs.
    #[test]
    fn a_case_variant_delete_through_a_parent_that_became_a_link_is_refused_before_any_identity_probe(
    ) {
        if !same_path(Path::new("A"), Path::new("a")) {
            // Names compare by case here: another spelling names another item.
            return;
        }
        let world = World::new();
        world.put("lib/Rules/A.md", "a");
        world.put("keep/A.md", "kept");
        world.scan();
        fs::remove_dir_all(world.at("lib/Rules")).unwrap();
        world.link("keep", "lib/Rules");

        let answer = world.delete(&["library/rules/a.md"]);

        assert_eq!(failed_code(&answer.items[0].outcome), PATH_THROUGH_LINK);
        assert_eq!(world.disk.identities.load(Ordering::SeqCst), 0, "no probe");
        assert_eq!(world.text("keep/A.md"), "kept");
    }

    // ADR-0409 decision 14: a delete and a restore raise the waiting count just before their ticket (here: held at the
    //   write lock) and lower it after; a refusal inside the gate lowers it too.
    #[test]
    fn a_trash_step_raises_the_waiting_count_just_before_its_ticket_and_lowers_it_after_or_on_a_refusal(
    ) {
        let world = World::new();
        world.put("lib/a.md", "a");
        world.scan();
        let trash = || world.svc.gate.trash();

        let (code, _) = refused(
            world
                .svc
                .delete(&["library/a.md"], false, false, None, Some(1)),
        );
        assert_eq!(code, ROOT_GENERATION_STALE);
        assert_eq!(trash().waiting(), 0, "released on the stale generation");
        assert!(world.there("lib/a.md"));

        // The op runs on a worker; its hook (after the call's judgement, right before its ticket) answers the waiting count
        //   there, then the op meets the held write lock with the count raised.
        let held_at_lock = |op: Box<dyn FnOnce(&LibraryService) -> Option<TrashAddress> + Send>| {
            let (at_hook, reached) = mpsc::channel::<usize>();
            let (go, proceed) = mpsc::channel::<()>();
            let svc = Arc::clone(&world.svc);
            let worker = on_thread(&world.svc, move |on| {
                BEFORE_HOLD.with(|hook| {
                    *hook.borrow_mut() = Some(Box::new(move || {
                        at_hook.send(svc.gate.trash().waiting()).unwrap();
                        proceed.recv_timeout(Duration::from_secs(20)).unwrap();
                    }));
                });
                op(on)
            });
            let before = reached.recv_timeout(Duration::from_secs(10)).unwrap();
            assert_eq!(before, 0, "not raised before the ticket");
            let guard = world.svc.gate.lib().write().unwrap();
            go.send(()).unwrap();
            trash().wait_until("the step waits at its write lock", |view| view.waiting == 1);
            assert!(!worker.is_finished());
            (guard, worker)
        };

        let (guard, worker) = held_at_lock(Box::new(|svc| {
            let answer = svc
                .delete(&["library/a.md"], false, false, None, None)
                .unwrap();
            Some(done(&answer.items[0].outcome))
        }));
        assert!(world.there("lib/a.md"), "nothing moved before the ticket");
        drop(guard);
        let address = worker.join().unwrap().expect("the delete answers its row");
        assert_eq!(trash().waiting(), 0, "lowered after the hold");
        assert!(!world.there("lib/a.md"));

        let (guard, worker) = held_at_lock(Box::new(move |svc| {
            let answer = svc.restore(&[address], false, None).unwrap();
            done(&answer.items[0].outcome);
            None
        }));
        assert!(!world.there("lib/a.md"));
        drop(guard);
        worker.join().unwrap();
        assert_eq!(trash().waiting(), 0, "lowered after the hold");
        assert_eq!(world.text("lib/a.md"), "a");
    }

    // ── restore ──

    // ADR-0409 decisions 8 and 9 · G11: the row goes back to its path, missing parents made; the nested root's bundle
    //   folder is gone after; the catalog has it again at once.
    #[test]
    fn a_restore_puts_a_row_back_making_missing_parents_and_the_emptied_bundle_goes() {
        let world = World::new();
        world.put("lib/d/e/f.md", "f");
        world.scan();
        let address = done(&world.delete(&["library/d/e/f.md"]).items[0].outcome);
        // The folders leave the disk and the catalog.
        done(&world.delete(&["library/d"]).items[0].outcome);

        let answer = world.svc.restore(&[address.clone()], false, None).unwrap();

        assert_eq!(done(&answer.items[0].outcome), "library/d/e/f.md");
        assert_eq!(world.text("lib/d/e/f.md"), "f");
        assert!(
            !world.there(&format!("lib/.trash/{}", address.bundle)),
            "the bundle folder is gone"
        );
        assert_eq!(world.kind("library/d/e/f.md"), Some(ItemKind::Instruction));
        let folders: Vec<String> = world.svc.with_read(|state| {
            state
                .catalog
                .folders()
                .map(|folder| folder.as_str().to_owned())
                .collect()
        });
        for made in ["library/d", "library/d/e"] {
            assert!(
                folders.iter().any(|folder| folder == made),
                "{made} in {folders:?}"
            );
        }
    }

    // G11 · M26: a taken name is that row's conflict and nothing is overwritten; keep-both restores it as (2).
    #[test]
    fn a_taken_name_is_that_rows_conflict_never_overwritten_and_keep_both_restores_it_as_2() {
        let world = World::new();
        world.put("lib/a.md", "old");
        world.put("lib/b.md", "b");
        world.scan();
        let answer = world.delete(&["library/a.md", "library/b.md"]);
        let (a, b) = (
            done(&answer.items[0].outcome),
            done(&answer.items[1].outcome),
        );
        world.put("lib/a.md", "new");

        let answer = world.svc.restore(&[a.clone(), b], false, None).unwrap();

        match &answer.items[0].outcome {
            Outcome::Conflict(reason) => assert_eq!(reason.code, NAME_EXISTS),
            other => panic!("{other:?}"),
        }
        assert_eq!(done(&answer.items[1].outcome), "library/b.md");
        assert_eq!(world.text("lib/a.md"), "new", "never overwritten");
        assert_eq!(world.text(&format!("lib/.trash/{}/a.md", a.bundle)), "old");

        let answer = world.svc.restore(&[a.clone()], true, None).unwrap();

        assert_eq!(done(&answer.items[0].outcome), "library/a (2).md");
        assert_eq!(world.text("lib/a (2).md"), "old");
        assert_eq!(world.text("lib/a.md"), "new");
        assert_eq!(world.kind("library/a (2).md"), Some(ItemKind::Instruction));
        assert!(
            !world.there(&format!("lib/.trash/{}", a.bundle)),
            "the last root went: the bundle with it"
        );
    }

    // ADR-0400 decision 23: the bundle lives in the mount folder, so a row restores into the renamed mount.
    #[test]
    fn a_restore_after_a_mount_rename_returns_into_the_renamed_mount() {
        let world = World::new();
        world.put("lib2/b.md", "b");
        world.scan();
        let address = done(&world.delete(&["team/b.md"]).items[0].outcome);
        world
            .svc
            .rename_mount("team", "crew", false, None, None)
            .unwrap();

        let row = TrashAddress {
            mount: "crew".to_owned(),
            ..address
        };
        let answer = world.svc.restore(&[row], false, None).unwrap();

        assert_eq!(done(&answer.items[0].outcome), "crew/b.md");
        assert_eq!(world.text("lib2/b.md"), "b");
    }

    // ADR-0409 decisions 6 and 8: a path inside a root, an other row, a temp name and a foreign bundle name are never
    //   restored.
    #[test]
    fn rows_inside_a_root_other_rows_temp_names_and_foreign_bundles_are_not_restored() {
        let world = World::new();
        world.put("lib/dir/x.md", "x");
        world.scan();
        let address = done(&world.delete(&["library/dir"]).items[0].outcome);
        let bundle = address.bundle.clone();
        world.put(&format!("lib/.trash/{bundle}/Thumbs.db"), "litter");
        world.put("lib/.trash/Obsidian/dir/x.md", "another app's");
        let row = |bundle: &str, path: &str| TrashAddress {
            mount: "library".to_owned(),
            bundle: bundle.to_owned(),
            path: path.to_owned(),
        };

        let answer = world
            .svc
            .restore(
                &[
                    row(&bundle, "dir/x.md"),
                    row(&bundle, "Thumbs.db"),
                    row(&bundle, "dir.tmp12.3"),
                    row("Obsidian", "dir/x.md"),
                    row(&bundle, "nope.md"),
                ],
                false,
                None,
            )
            .unwrap();

        let codes: Vec<String> = answer
            .items
            .iter()
            .map(|item| failed_code(&item.outcome))
            .collect();
        assert_eq!(
            codes,
            [
                INSIDE_ROOT,
                NOT_RESTORABLE,
                NOT_RESTORABLE,
                NOT_FOUND,
                NOT_FOUND
            ]
        );
        assert!(world.there(&format!("lib/.trash/{bundle}/dir/x.md")));
        assert!(world.there(&format!("lib/.trash/{bundle}/Thumbs.db")));
        assert!(!world.there("lib/dir"));
    }

    // ADR-0409 decision 8: a record that failed to publish after the moves — the bundle shows its files and they restore.
    #[test]
    fn a_bundle_whose_record_failed_to_publish_shows_its_files_and_they_restore() {
        let world = World::new();
        world.put("lib/a.md", "a");
        world.scan();
        world.disk.fail_record.store(true, Ordering::SeqCst);
        let address = done(&world.delete(&["library/a.md"]).items[0].outcome);
        world.disk.fail_record.store(false, Ordering::SeqCst);
        assert!(!world.there(&format!("lib/.trash/{}/{RECORD_NAME}", address.bundle)));

        let view = world.view();
        let bundles = view.mounts[0].view.as_ref().unwrap();
        assert_eq!(bundles[0].source, RowSource::NoRecord);
        let rows: Vec<(&str, RowKind)> = bundles[0]
            .rows
            .iter()
            .map(|row| (row.path.as_str(), row.kind))
            .collect();
        assert_eq!(rows, [("a.md", RowKind::Found)]);

        let answer = world.svc.restore(&[address.clone()], false, None).unwrap();

        assert_eq!(done(&answer.items[0].outcome), "library/a.md");
        assert_eq!(world.text("lib/a.md"), "a");
        assert!(!world.there(&format!("lib/.trash/{}", address.bundle)));
    }

    // ADR-0409 decision 8: a bundle read without its record is walked once per call — two of its rows restore in one
    //   call, and a row given twice comes back once (the second is not found, not a conflict with the first).
    #[test]
    fn rows_of_a_bundle_without_a_record_restore_together_and_a_repeated_row_once() {
        let world = World::new();
        for name in ["a", "b", "c"] {
            world.put(&format!("lib/{name}.md"), name);
        }
        world.scan();
        world.disk.fail_record.store(true, Ordering::SeqCst);
        let answer = world.delete(&["library/a.md", "library/b.md", "library/c.md"]);
        world.disk.fail_record.store(false, Ordering::SeqCst);
        let (a, b) = (
            done(&answer.items[0].outcome),
            done(&answer.items[1].outcome),
        );

        let answer = world
            .svc
            .restore(&[a.clone(), b, a.clone()], false, None)
            .unwrap();

        assert_eq!(done(&answer.items[0].outcome), "library/a.md");
        assert_eq!(done(&answer.items[1].outcome), "library/b.md");
        assert_eq!(failed_code(&answer.items[2].outcome), NOT_FOUND);
        assert_eq!(
            world.text(&format!("lib/.trash/{}/c.md", a.bundle)),
            "c",
            "the bundle keeps its other row"
        );
    }

    // ADR-0409 decision 8: a bundle whose check fails with an IO error is no absence — the view names it with the error
    //   and its restore fails with it, the row left where it is.
    #[test]
    fn a_bundle_whose_check_fails_is_named_in_the_view_and_its_restore_fails_not_not_found() {
        let world = World::new();
        world.put("lib/a.md", "a");
        world.scan();
        let address = done(&world.delete(&["library/a.md"]).items[0].outcome);
        *lock(&world.disk.fail_stat) = Some(world.at(&format!("lib/.trash/{}", address.bundle)));

        let view = world.view();
        let answer = world.svc.restore(&[address.clone()], false, None).unwrap();

        let bundles = view.mounts[0].view.as_ref().unwrap();
        assert_eq!(bundles[0].id.as_str(), address.bundle);
        assert_eq!(bundles[0].source, RowSource::Unchecked);
        assert!(bundles[0]
            .trouble
            .as_deref()
            .is_some_and(|why| why.contains("injected")));
        assert_eq!(failed_code(&answer.items[0].outcome), LIBRARY_READ_FAILED);
        assert_eq!(
            world.text(&format!("lib/.trash/{}/a.md", address.bundle)),
            "a"
        );
    }

    // ── foreign content and links (real disk) ──

    // ADR-0409 decision 6: another app's files, a loose file, a link and a badly named folder in .trash are absent from the
    //   view, refused by address and survive a delete beside them.
    #[test]
    fn foreign_content_in_the_trash_is_not_listed_not_restored_and_survives() {
        let world = World::new();
        world.put("lib/a.md", "a");
        world.put("keep/a.md", "kept");
        world.put("lib/.trash/notes.txt", "loose");
        world.put("lib/.trash/Obsidian/page.md", "another app's");
        world.put("lib/.trash/20261399T000000Z-000003/z.md", "badly named");
        world.link("keep", "lib/.trash/20261125T000000Z-000002");
        world.scan();

        let address = done(&world.delete(&["library/a.md"]).items[0].outcome);

        let view = world.view();
        let ids: Vec<&str> = view.mounts[0]
            .view
            .as_ref()
            .unwrap()
            .iter()
            .map(|bundle| bundle.id.as_str())
            .collect();
        assert_eq!(ids, [address.bundle.as_str()]);
        let row = |bundle: &str, path: &str| TrashAddress {
            mount: "library".to_owned(),
            bundle: bundle.to_owned(),
            path: path.to_owned(),
        };
        let answer = world
            .svc
            .restore(
                &[
                    row("20261125T000000Z-000002", "a.md"),
                    row("Obsidian", "page.md"),
                    row("20261399T000000Z-000003", "z.md"),
                ],
                false,
                None,
            )
            .unwrap();
        for item in &answer.items {
            assert_eq!(failed_code(&item.outcome), NOT_FOUND, "{:?}", item.row);
        }
        for rel in [
            "lib/.trash/notes.txt",
            "lib/.trash/Obsidian/page.md",
            "lib/.trash/20261399T000000Z-000003/z.md",
            "keep/a.md",
        ] {
            assert!(world.there(rel), "{rel}");
        }
        assert!(
            fs::symlink_metadata(world.at("lib/.trash/20261125T000000Z-000002"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert!(
            !world.there("lib/a.md"),
            "nothing came back through the link"
        );
    }

    // ADR-0365 · ADR-0409 decision 9 — link safety (2): a link two folders down inside a deleted folder is moved with it,
    //   viewed inside its one row and restored as a link; what it points at is untouched.
    #[test]
    fn a_link_inside_a_deleted_folder_moves_and_restores_as_a_link() {
        let world = World::new();
        world.put("lib/d/x/note.md", "n");
        world.put("keep/z.md", "kept");
        world.link("keep", "lib/d/x/y");
        world.scan();

        let address = done(&world.delete(&["library/d"]).items[0].outcome);
        lock(&world.links).push(world.at(&format!("lib/.trash/{}/d/x/y", address.bundle)));

        let view = world.view();
        let bundles = view.mounts[0].view.as_ref().unwrap();
        let rows: Vec<&str> = bundles[0]
            .rows
            .iter()
            .map(|row| row.path.as_str())
            .collect();
        assert_eq!(rows, ["d"]);
        let answer = world.svc.restore(&[address], false, None).unwrap();
        done(&answer.items[0].outcome);
        assert!(fs::symlink_metadata(world.at("lib/d/x/y"))
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(world.text("keep/z.md"), "kept");
        assert_eq!(fs::read_dir(world.at("keep")).unwrap().count(), 1);
    }

    // Link safety (3): a bundle folder replaced by a link is foreign — not viewed, not restored through.
    #[test]
    fn a_bundle_folder_replaced_by_a_link_is_not_viewed_or_restored_through() {
        let world = World::new();
        world.put("lib/a.md", "a");
        world.put("keep/a.md", "kept");
        world.scan();
        let address = done(&world.delete(&["library/a.md"]).items[0].outcome);
        let bundle = format!("lib/.trash/{}", address.bundle);
        fs::remove_dir_all(world.at(&bundle)).unwrap();
        world.link("keep", &bundle);

        let view = world.view();
        assert!(view.mounts[0].view.as_ref().unwrap().is_empty());
        let answer = world.svc.restore(&[address], false, None).unwrap();

        assert_eq!(failed_code(&answer.items[0].outcome), NOT_FOUND);
        assert_eq!(world.text("keep/a.md"), "kept");
        assert!(!world.there("lib/a.md"));
    }

    // Link safety (4) · ADR-0409 decision 5: .trash itself a link — the delete refuses, the view names the mount, nothing is
    //   restored, and nothing lands behind the link.
    #[test]
    fn a_trash_folder_that_is_a_link_refuses_the_delete_and_the_restore_and_the_view_names_it() {
        let world = World::new();
        world.put("lib/a.md", "a");
        world.put("keep/20261123T081500Z-a1b2c3/a.md", "kept");
        world.link("keep", "lib/.trash");
        world.scan();

        let answer = world.delete(&["library/a.md"]);

        assert_eq!(failed_code(&answer.items[0].outcome), TRASH_UNUSABLE);
        assert_eq!(world.text("lib/a.md"), "a");
        let view = world.view();
        assert_eq!(
            view.mounts[0].view.as_ref().unwrap_err().code,
            TRASH_UNUSABLE
        );
        let row = TrashAddress {
            mount: "library".to_owned(),
            bundle: "20261123T081500Z-a1b2c3".to_owned(),
            path: "a.md".to_owned(),
        };
        let answer = world.svc.restore(&[row], false, None).unwrap();
        assert_eq!(failed_code(&answer.items[0].outcome), TRASH_UNUSABLE);
        assert_eq!(world.text("keep/20261123T081500Z-a1b2c3/a.md"), "kept");
        assert_eq!(
            fs::read_dir(world.at("keep")).unwrap().count(),
            1,
            "nothing made behind the link"
        );
    }

    // ADR-0365 · ADR-0409 decision 9 — link safety: a folder on the way back that is a link now is never restored
    //   through — the row answers path-through-link and stays in its bundle.
    #[test]
    fn a_restore_whose_destination_folder_is_a_link_writes_nothing_behind_it() {
        let world = World::new();
        world.put("lib/d/f.md", "f");
        world.put("keep/k.md", "kept");
        world.scan();
        let address = done(&world.delete(&["library/d/f.md"]).items[0].outcome);
        fs::remove_dir(world.at("lib/d")).unwrap();
        world.link("keep", "lib/d");

        let answer = world.svc.restore(&[address.clone()], false, None).unwrap();

        assert_eq!(failed_code(&answer.items[0].outcome), PATH_THROUGH_LINK);
        assert_eq!(
            fs::read_dir(world.at("keep")).unwrap().count(),
            1,
            "nothing written behind the link"
        );
        assert_eq!(
            world.text(&format!("lib/.trash/{}/d/f.md", address.bundle)),
            "f"
        );
    }

    // Link safety (3): a link between the bundle and a row — a root of the record or not — refuses the row
    //   path-through-link; what the link points at is untouched.
    #[test]
    fn a_link_between_the_bundle_and_a_row_refuses_the_restore_and_leaves_its_target() {
        let world = World::new();
        world.put("lib/d/x.md", "x");
        world.put("keep/x.md", "kept");
        world.put("keep/y.md", "kept too");
        world.scan();
        let address = done(&world.delete(&["library/d/x.md"]).items[0].outcome);
        let folder = format!("lib/.trash/{}/d", address.bundle);
        fs::remove_dir_all(world.at(&folder)).unwrap();
        world.link("keep", &folder);
        let other = TrashAddress {
            path: "d/y.md".to_owned(),
            ..address.clone()
        };

        let answer = world.svc.restore(&[address, other], false, None).unwrap();

        for item in &answer.items {
            assert_eq!(
                failed_code(&item.outcome),
                PATH_THROUGH_LINK,
                "{:?}",
                item.row
            );
        }
        assert_eq!(world.text("keep/x.md"), "kept");
        assert_eq!(world.text("keep/y.md"), "kept too");
        assert!(!world.there("lib/d/x.md") && !world.there("lib/d/y.md"));
    }

    // ADR-0409 M15: the view reads each mount on a bounded job — a mount that does not answer is named, the others listed.
    #[test]
    fn a_mount_whose_trash_does_not_answer_is_named_and_the_others_are_viewed() {
        let world = World::make(Vec::new(), Duration::from_millis(300));
        world.put("lib/a.md", "a");
        world.scan();
        done(&world.delete(&["library/a.md"]).items[0].outcome);
        let (entered_tx, entered) = mpsc::channel();
        let (release, release_rx) = mpsc::channel();
        *lock(&world.disk.stat_door) = Some((world.at("lib2"), entered_tx, release_rx));

        let view = world.view();

        entered.recv_timeout(Duration::from_secs(10)).unwrap();
        assert_eq!(view.mounts[0].view.as_ref().unwrap().len(), 1);
        let team = view.mounts[1].view.as_ref().unwrap_err();
        assert_eq!(team.code, LIBRARY_UNAVAILABLE);
        assert_eq!(team.args["mount"], json!("team"));
        release.send(()).unwrap();
    }
    // ── move and rename ──

    fn conflict_code<T: std::fmt::Debug>(outcome: &Outcome<T>) -> String {
        match outcome {
            Outcome::Conflict(reason) => reason.code.clone(),
            other => panic!("not a conflict: {other:?}"),
        }
    }

    fn pairs(answer: &MoveAnswer) -> Vec<(&str, &str)> {
        answer
            .mapping
            .iter()
            .map(|(old, new, _)| (old.as_str(), new.as_str()))
            .collect()
    }

    // ADR-0409 M3: a rename to SKILL.md makes the folder a skill and the file its SKILL.md at once, and the rename back
    //   makes them a plain folder and an instruction — no refresh in between.
    #[test]
    fn a_rename_to_skill_md_and_back_changes_the_kinds_without_a_refresh() {
        let world = World::new();
        world.put(
            "lib/s/x.md",
            "---\nname: named\ndescription: d\n---\nbody\n",
        );
        world.scan();
        assert_eq!(world.kind("library/s"), None);

        let answer = world.rename("library/s/x.md", "SKILL.md");

        assert_eq!(done(&answer.items[0].outcome).to, "library/s/SKILL.md");
        assert_eq!(world.kind("library/s"), Some(ItemKind::Skill));
        assert_eq!(world.kind("library/s/SKILL.md"), Some(ItemKind::SkillFile));
        assert_eq!(world.kind("library/s/x.md"), None);
        assert_eq!(world.skill("library/s").0, "named");

        let answer = world.rename("library/s/SKILL.md", "x.md");

        assert_eq!(done(&answer.items[0].outcome).to, "library/s/x.md");
        assert_eq!(world.kind("library/s"), None, "no skill any more");
        assert_eq!(world.kind("library/s/x.md"), Some(ItemKind::Instruction));
        assert_eq!(world.kind("library/s/SKILL.md"), None);
    }

    // ADR-0409 M3: a SKILL.md moved from a mount's top (an instruction there) into a folder makes the folder a skill;
    //   moved back out it is an instruction again and the folder no skill.
    #[test]
    fn a_skill_md_moved_into_a_folder_and_out_again_changes_the_kinds_without_a_refresh() {
        let world = World::new();
        world.put("lib/SKILL.md", "---\nname: s\ndescription: d\n---\n");
        world.put("lib/f/keep.md", "k");
        world.scan();
        assert_eq!(world.kind("library/SKILL.md"), Some(ItemKind::Instruction));

        done(&world.mv(&["library/SKILL.md"], "library/f", None).items[0].outcome);

        assert_eq!(world.kind("library/f"), Some(ItemKind::Skill));
        assert_eq!(world.kind("library/f/SKILL.md"), Some(ItemKind::SkillFile));
        assert_eq!(world.kind("library/SKILL.md"), None);

        done(&world.mv(&["library/f/SKILL.md"], "library", None).items[0].outcome);

        assert_eq!(world.kind("library/f"), None);
        assert_eq!(world.kind("library/SKILL.md"), Some(ItemKind::Instruction));
        assert_eq!(world.kind("library/f/keep.md"), Some(ItemKind::Instruction));
    }

    // ADR-0409 M3: a skill without `name:` takes its folder's name — re-keying alone would keep the old one; its
    //   SKILL.md is re-read in the hold, for the skill folder renamed and for it moved to a keep-both name.
    #[test]
    fn a_skill_folder_without_a_name_renamed_or_moved_shows_its_new_name_and_folder_at_once() {
        let world = World::new();
        world.put("lib/up/s/SKILL.md", "---\ndescription: d\n---\nbody\n");
        world.put("lib/d/t", "a file of the skill's name");
        world.scan();
        assert_eq!(
            world.skill("library/up/s"),
            ("s".to_owned(), "s".to_owned())
        );

        done(&world.rename("library/up/s", "t").items[0].outcome);

        assert_eq!(world.kind("library/up/s"), None);
        assert_eq!(
            world.skill("library/up/t"),
            ("t".to_owned(), "t".to_owned())
        );
        assert_eq!(
            world.kind("library/up/t/SKILL.md"),
            Some(ItemKind::SkillFile)
        );

        let answer = world.mv(&["library/up/t"], "library/d", Some(OnConflict::KeepBoth));

        assert_eq!(pairs(&answer), [("library/up/t", "library/d/t (2)")]);
        assert_eq!(
            world.skill("library/d/t (2)"),
            ("t (2)".to_owned(), "t (2)".to_owned())
        );
        assert_eq!(world.kind("library/up/t"), None);
    }

    // ADR-0409 (P5): a taken name without onConflict is that item's conflict and nothing is replaced; a child given
    //   with its folder is skipped; a move into the folder it is in is skipped; into itself is refused.
    #[test]
    fn a_move_answers_per_item_by_the_explorer_rules() {
        let world = World::new();
        world.put("lib/a.md", "mine");
        world.put("lib/d/b.md", "b");
        world.put("lib/d/e/c.md", "c");
        world.put("lib/t/a.md", "theirs");
        world.scan();

        let answer = world.mv(
            &["library/a.md", "library/d", "library/d/b.md"],
            "library/t",
            None,
        );

        assert_eq!(conflict_code(&answer.items[0].outcome), NAME_EXISTS);
        assert_eq!(world.text("lib/a.md"), "mine");
        assert_eq!(world.text("lib/t/a.md"), "theirs", "nothing replaced");
        assert_eq!(done(&answer.items[1].outcome).to, "library/t/d");
        assert_eq!(
            answer.items[2].outcome,
            Outcome::Skipped(Skip::Inside("library/d".to_owned()))
        );
        assert_eq!(world.text("lib/t/d/e/c.md"), "c");
        assert_eq!(
            world.kind("library/t/d/e/c.md"),
            Some(ItemKind::Instruction)
        );
        assert_eq!(world.kind("library/d/b.md"), None);

        let answer = world.mv(&["library/t/d"], "library/t", None);
        assert_eq!(answer.items[0].outcome, Outcome::Skipped(Skip::InPlace));

        let answer = world.mv(&["library/t", "library/t/d"], "library/t/d/e", None);
        assert_eq!(failed_code(&answer.items[0].outcome), INTO_ITSELF);
        assert_eq!(
            answer.items[1].outcome,
            Outcome::Skipped(Skip::Inside("library/t".to_owned()))
        );
        let answer = world.mv(&["library/t/d"], "library/t/d/e", None);
        assert_eq!(failed_code(&answer.items[0].outcome), INTO_ITSELF);
        assert!(world.there("lib/t/d/e/c.md"));
    }

    // ADR-0339 decision 1 · ADR-0409 decision 8: skip leaves both, keep both takes `name (2)`, overwrite puts the old
    //   entry in the destination mount's trash with its record, the bundle row in the view.
    #[test]
    fn on_conflict_skip_keep_both_and_overwrite_answer_a_taken_name() {
        let world = World::new();
        world.put("lib/a.md", "first");
        world.put("lib/b/a.md", "second");
        world.put("lib/t/a.md", "old");
        world.scan();

        let answer = world.mv(&["library/a.md"], "library/t", Some(OnConflict::Skip));
        assert_eq!(
            answer.items[0].outcome,
            Outcome::Skipped(Skip::Taken("library/t/a.md".to_owned()))
        );
        assert_eq!(world.text("lib/a.md"), "first");

        let answer = world.mv(&["library/a.md"], "library/t", Some(OnConflict::KeepBoth));
        assert_eq!(done(&answer.items[0].outcome).to, "library/t/a (2).md");
        assert_eq!(world.text("lib/t/a (2).md"), "first");
        assert_eq!(world.text("lib/t/a.md"), "old");
        assert_eq!(
            world.kind("library/t/a (2).md"),
            Some(ItemKind::Instruction)
        );

        let answer = world.mv(
            &["library/b/a.md"],
            "library/t",
            Some(OnConflict::Overwrite),
        );

        let moved = done(&answer.items[0].outcome);
        assert_eq!(moved.to, "library/t/a.md");
        assert_eq!(world.text("lib/t/a.md"), "second");
        assert!(!world.there("lib/b/a.md"));
        let row = &moved.trashed[0];
        assert_eq!(
            (row.mount.as_str(), row.path.as_str()),
            ("library", "t/a.md")
        );
        assert_eq!(
            world.text(&format!("lib/.trash/{}/t/a.md", row.bundle)),
            "old"
        );
        let view = world.view();
        let bundles = view.mounts[0].view.as_ref().unwrap();
        assert_eq!(bundles[0].source, RowSource::Record);
        assert_eq!(bundles[0].rows[0].path.as_str(), "t/a.md");
        assert_eq!(world.kind("library/t/a.md"), Some(ItemKind::Instruction));
    }

    // ADR-0409 M7: same-name folders merge entry by entry under overwrite; the emptied source folder goes and the
    //   mapping holds each entry moved whole (the folder itself is no skill, so it maps nowhere); with skip a colliding
    //   file stays and so does its folder.
    #[test]
    fn same_name_folders_merge_entry_by_entry_and_only_an_emptied_source_folder_goes() {
        let world = World::new();
        world.put("lib/src/d/x.md", "x1");
        world.put("lib/src/d/y.md", "y1");
        world.put("lib/src/d/sub/z.md", "z");
        world.put("lib/t/d/x.md", "x0");
        world.put("lib/t/d/w.md", "w");
        world.scan();

        let answer = world.mv(&["library/src/d"], "library/t", Some(OnConflict::Overwrite));

        let moved = done(&answer.items[0].outcome);
        assert_eq!(moved.to, "library/t/d");
        assert!(moved.left.is_empty(), "{:?}", moved.left);
        assert_eq!(moved.trashed.len(), 1);
        assert_eq!(moved.trashed[0].path, "t/d/x.md");
        assert_eq!(
            pairs(&answer),
            [
                ("library/src/d/sub", "library/t/d/sub"),
                ("library/src/d/x.md", "library/t/d/x.md"),
                ("library/src/d/y.md", "library/t/d/y.md"),
            ]
        );
        assert!(!world.there("lib/src/d"), "the emptied source folder went");
        assert_eq!(world.text("lib/t/d/x.md"), "x1");
        assert_eq!(world.text("lib/t/d/w.md"), "w");
        assert_eq!(world.text("lib/t/d/sub/z.md"), "z");
        assert_eq!(
            world.kind("library/t/d/sub/z.md"),
            Some(ItemKind::Instruction)
        );
        assert_eq!(world.kind("library/src/d/y.md"), None);

        let world = World::new();
        world.put("lib/src/d/x.md", "x1");
        world.put("lib/src/d/y.md", "y1");
        world.put("lib/t/d/x.md", "x0");
        world.scan();

        let answer = world.mv(&["library/src/d"], "library/t", Some(OnConflict::Skip));

        let moved = done(&answer.items[0].outcome);
        assert_eq!(moved.left.len(), 1);
        assert_eq!(moved.left[0].0, "library/src/d/x.md");
        assert_eq!(moved.left[0].1.code, NAME_EXISTS);
        assert_eq!(world.text("lib/src/d/x.md"), "x1", "the skipped file stays");
        assert_eq!(world.text("lib/t/d/x.md"), "x0");
        assert_eq!(world.text("lib/t/d/y.md"), "y1");
        assert_eq!(pairs(&answer), [("library/src/d/y.md", "library/t/d/y.md")]);
        assert_eq!(
            world.kind("library/src/d/x.md"),
            Some(ItemKind::Instruction)
        );
    }

    // ADR-0409 M8: a file onto a folder of its name under overwrite is that item's conflict, nothing moved; under keep
    //   both it takes the next name like any collision.
    #[test]
    fn a_file_onto_a_folder_of_its_name_is_kind_mismatch_under_overwrite_and_renamed_under_keep_both(
    ) {
        let world = World::new();
        world.put("lib/src/doc", "file");
        world.put("lib/t/doc/in.md", "in");
        world.scan();

        let answer = world.mv(
            &["library/src/doc"],
            "library/t",
            Some(OnConflict::Overwrite),
        );
        assert_eq!(conflict_code(&answer.items[0].outcome), KIND_MISMATCH);
        assert_eq!(world.text("lib/src/doc"), "file");
        assert_eq!(world.text("lib/t/doc/in.md"), "in");
        assert!(!world.there("lib/.trash"), "nothing went to the trash");

        let answer = world.mv(
            &["library/src/doc"],
            "library/t",
            Some(OnConflict::KeepBoth),
        );
        assert_eq!(done(&answer.items[0].outcome).to, "library/t/doc (2)");
        assert_eq!(world.text("lib/t/doc (2)"), "file");
    }

    // ADR-0409 decision 4 · ADR-0339: a folder moved to another mount on the same volume takes every reference along —
    //   by the path-key prefix mapper, in the moved preset itself too, each touched list keeping the first place — and
    //   the agents once; the answer names the files and the mount's working tree.
    #[test]
    fn references_follow_a_folder_moved_across_mounts_and_a_touched_list_keeps_the_first_place() {
        let attached = agent(&["library/r/P.yaml"]);
        let id = attached.id;
        let world = World::with(vec![attached]);
        world.put("lib/r/a.md", "a");
        world.put("lib/r/P.yaml", &preset("instructions:\n- library/r/a.md\n"));
        world.put(
            "lib2/Q.yaml",
            &preset("instructions:\n- path: library/r/a.md\n  backend: codex\n- team/r/a.md\n"),
        );
        world.scan();

        let answer = world.mv(&["library/r"], "team", None);

        assert_eq!(done(&answer.items[0].outcome).to, "team/r");
        assert_eq!(pairs(&answer), [("library/r", "team/r")]);
        assert_eq!(answer.written, ["team/Q.yaml", "team/r/P.yaml"]);
        assert!(answer.stopped.is_none());
        let q = world.text("lib2/Q.yaml");
        assert_eq!(q.matches("team/r/a.md").count(), 1, "{q}");
        assert!(
            q.contains("backend: codex"),
            "the first place stays, with its tag: {q}"
        );
        assert!(!q.contains("library/r"), "{q}");
        assert!(world.text("lib2/r/P.yaml").contains("- team/r/a.md"));
        assert_eq!(answer.agents, [id]);
        assert_eq!(world.presets_of(id), ["team/r/P.yaml"]);
        assert_eq!(world.kind("team/r/a.md"), Some(ItemKind::Instruction));
        assert_eq!(world.kind("library/r/a.md"), None);
        assert_eq!(
            world.listing("team/r/a.md"),
            ["team/Q.yaml", "team/r/P.yaml"]
        );
        assert_eq!(answer.trees.len(), 1);
        assert_eq!(answer.trees[0].0, "team");
    }

    // ADR-0400 decision 6: a referencing file in a mount that cannot be read is skipped with `library-unavailable`,
    //   its disk untouched; the move itself goes.
    #[test]
    fn a_referencing_file_in_a_mount_that_cannot_be_read_is_skipped() {
        let world = World::new();
        world.put("lib/a.md", "a");
        world.put("lib/d/keep.md", "k");
        world.put("lib2/Q.yaml", &preset("instructions:\n- library/a.md\n"));
        world.scan();
        fs::rename(world.at("lib2"), world.at("lib2-away")).unwrap();
        // The scan records the mount it cannot read (relink's rule: the recorded state decides).
        world.svc.refresh(Some(&lp("team")));

        let answer = world.mv(&["library/a.md"], "library/d", None);

        assert_eq!(done(&answer.items[0].outcome).to, "library/d/a.md");
        assert_eq!(answer.skipped.len(), 1, "{:?}", answer.skipped);
        assert_eq!(answer.skipped[0].path, "team/Q.yaml");
        assert_eq!(answer.skipped[0].reason.code, LIBRARY_UNAVAILABLE);
        assert!(answer.written.is_empty());
        assert!(world.text("lib2-away/Q.yaml").contains("library/a.md"));
    }

    // ADR-0400 decision 18 · ADR-0414 decision 2: the move's hold raises the change number before its agents pass — an
    //   `agent.new` judged before the move and inserted while the agents pass is refused by the number and judges again;
    //   one inserted before the move is re-pointed.
    #[test]
    fn an_agent_new_judged_before_a_move_is_re_pointed_or_judges_again() {
        let world = World::new();
        world.put("lib/S.yaml", &preset(""));
        world.put("lib/d/keep.md", "k");
        world.scan();
        let svc = &world.svc;
        let presets = ["library/S.yaml".to_owned()];
        let judged = svc.judge_new_presets(&agent(&[]), &presets, None).unwrap();
        let mut early = agent(&[]);
        early.presets = judged.attachments.presets.clone();
        let early_id = early.id;
        let at = judged.change.unwrap();
        assert!(svc
            .values
            .registry
            .try_upsert_if(early, || svc.change_number() == at)
            .unwrap());
        let late_judged = svc.judge_new_presets(&agent(&[]), &presets, None).unwrap();
        let late_at = late_judged.change.unwrap();
        let (entered_tx, entered) = mpsc::channel();
        let (release, release_rx) = mpsc::channel();
        *lock(&world.port.entered) = Some(entered_tx);
        *lock(&world.port.release) = Some(release_rx);

        let moving = on_thread(svc, |svc| {
            svc.move_items(
                &["library/S.yaml"],
                "library/d",
                None,
                false,
                false,
                None,
                None,
            )
        });
        entered.recv_timeout(Duration::from_secs(10)).unwrap();
        let mut late = agent(&[]);
        late.presets = late_judged.attachments.presets;
        let late_id = late.id;
        assert!(
            !svc.values
                .registry
                .try_upsert_if(late, || svc.change_number() == late_at)
                .unwrap(),
            "the raised number refuses the insert"
        );
        release.send(()).unwrap();
        let answer = moving.join().unwrap().unwrap();

        assert_eq!(answer.agents, [early_id]);
        assert_eq!(world.presets_of(early_id), ["library/d/S.yaml"]);
        assert!(svc.values.registry.get(late_id).is_none());
        let again = svc.judge_new_presets(&agent(&[]), &presets, None).unwrap();
        assert!(again.attachments.presets.is_empty());
        assert_eq!(again.skipped[0].reason.code, NOT_FOUND);
    }

    // ADR-0335 decision 2: a move interrupted after its disk step (at the first preset file) or after k of N files
    //   journals nothing — the files not fixed still name the old path, which reads as not found, and one relink of it
    //   fixes them.
    #[test]
    fn an_interrupted_move_leaves_unfixed_lines_not_found_and_one_relink_fixes_them() {
        for fails in ["P1", "P2"] {
            let world = World::new();
            world.put("lib/r/a.md", "a");
            world.put("lib/d/keep.md", "k");
            for p in ["P1", "P2", "P3"] {
                world.put(
                    &format!("lib/{p}.yaml"),
                    &preset("instructions:\n- library/r/a.md\n"),
                );
            }
            world.scan();
            lock(&world.disk.fail_write).push(world.at(&format!("lib/{fails}.yaml")));

            let answer = world.mv(&["library/r"], "library/d", None);

            assert_eq!(done(&answer.items[0].outcome).to, "library/d/r");
            let stop = answer.stopped.as_ref().expect("the walk stopped");
            assert_eq!(
                stop.path.as_deref(),
                Some(format!("library/{fails}.yaml").as_str())
            );
            let fixed: &[&str] = if fails == "P1" {
                &[]
            } else {
                &["library/P1.yaml"]
            };
            assert_eq!(answer.written, fixed);
            assert_eq!(
                world.kind("library/r/a.md"),
                None,
                "the old path reads as not found"
            );
            let unfixed: Vec<String> = ["P1", "P2", "P3"]
                .iter()
                .filter(|p| !(fails == "P2" && **p == "P1"))
                .map(|p| format!("library/{p}.yaml"))
                .collect();
            assert_eq!(world.listing("library/r/a.md"), unfixed);
            let mut names: Vec<String> = fs::read_dir(world.at("lib"))
                .unwrap()
                .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
                .collect();
            names.sort();
            assert_eq!(
                names,
                ["P1.yaml", "P2.yaml", "P3.yaml", "d"],
                "nothing journaled"
            );

            lock(&world.disk.fail_write).clear();
            let relinked = world
                .svc
                .relink("library/r/a.md", "library/d/r/a.md", None)
                .unwrap();

            assert_eq!(relinked.written, unfixed);
            assert!(world.listing("library/r/a.md").is_empty());
            assert_eq!(world.listing("library/d/r/a.md").len(), 3);
        }
    }

    // ADR-0339 decision 1: when the rename onto an overwritten name fails, the displaced entry is renamed back and its
    //   bundle pruned; when that fails too, the answer names the trash row the entry is in.
    #[test]
    fn an_overwrite_whose_rename_fails_takes_the_entry_back_or_names_its_trash_row() {
        let world = World::new();
        world.put("lib/a.md", "new");
        world.put("lib/t/a.md", "old");
        world.scan();
        let wanted = BundleId::parse("20261123T081500Z-a1b2c3").unwrap();
        lock(&world.disk.fail_renames).push((world.at("lib/a.md"), 1));
        WANTED_BUNDLE.with(|id| *id.borrow_mut() = Some(wanted.clone()));

        let answer = world.mv(&["library/a.md"], "library/t", Some(OnConflict::Overwrite));

        assert_eq!(failed_code(&answer.items[0].outcome), LIBRARY_WRITE_FAILED);
        assert_eq!(world.text("lib/t/a.md"), "old", "taken back");
        assert_eq!(world.text("lib/a.md"), "new");
        assert!(
            !world.there(&format!("lib/.trash/{}", wanted.as_str())),
            "its bundle pruned"
        );
        assert_eq!(world.kind("library/t/a.md"), Some(ItemKind::Instruction));

        *lock(&world.disk.fail_renames) = vec![
            (world.at("lib/a.md"), 1),
            (
                world.at(&format!("lib/.trash/{}/t/a.md", wanted.as_str())),
                0,
            ),
        ];
        WANTED_BUNDLE.with(|id| *id.borrow_mut() = Some(wanted.clone()));

        let answer = world.mv(&["library/a.md"], "library/t", Some(OnConflict::Overwrite));

        let Outcome::Failed(reason) = &answer.items[0].outcome else {
            panic!("{:?}", answer.items[0].outcome);
        };
        assert_eq!(reason.args["to"], "library/t/a.md");
        assert_eq!(
            reason.args["trashed"],
            json!({ "mount": "library", "bundle": wanted.as_str(), "path": "t/a.md" })
        );
        assert!(!world.there("lib/t/a.md"));
        assert_eq!(
            world.text(&format!("lib/.trash/{}/t/a.md", wanted.as_str())),
            "old"
        );
        assert_eq!(world.kind("library/t/a.md"), None);
    }

    // ADR-0339 M110 · ADR-0409 decision 2: a referenced skill's SKILL.md moved out of it — into its own subfolder too —
    //   asks first with the plan; detachRefs without yes is no yes; plain yes keeps the references; yes + detachRefs +
    //   the digest moves it and drops the preset lines naming the folder, no agent step.
    #[test]
    fn a_skill_md_leaving_its_referenced_skill_asks_first_and_detaches_only_with_the_digest() {
        let world = World::new();
        world.put(
            "lib/s/SKILL.md",
            "---\nname: s\ndescription: d\n---\nbody\n",
        );
        world.put("lib/s/sub/keep.md", "k");
        world.put("lib/P.yaml", &preset("skills:\n- library/s\n"));
        world.scan();

        let (code, args) = refused(world.svc.move_items(
            &["library/s/SKILL.md"],
            "library/s/sub",
            None,
            false,
            false,
            None,
            None,
        ));
        assert_eq!(code, SKILL_UNCONFIRMED);
        assert_eq!(args["plan"]["presets"], json!(["library/P.yaml"]));
        assert!(world.there("lib/s/SKILL.md"));
        let (code, _) = refused(world.svc.move_items(
            &["library/s/SKILL.md"],
            "library/s/sub",
            None,
            false,
            true,
            Some(&digest_of(&args)),
            None,
        ));
        assert_eq!(code, SKILL_UNCONFIRMED, "detachRefs without yes is no yes");
        for digest in [None, Some("not-the-plans-digest")] {
            let (code, _) = refused(world.svc.move_items(
                &["library/s/SKILL.md"],
                "library/s/sub",
                None,
                true,
                true,
                digest,
                None,
            ));
            assert_eq!(
                code, SKILL_UNCONFIRMED,
                "detachRefs needs the plan's digest: {digest:?}"
            );
        }
        assert!(world.there("lib/s/SKILL.md"));

        let answer = world
            .svc
            .move_items(
                &["library/s/SKILL.md"],
                "library/s/sub",
                None,
                true,
                true,
                Some(&digest_of(&args)),
                None,
            )
            .unwrap();

        done(&answer.items[0].outcome);
        assert_eq!(answer.detached, ["library/s"]);
        assert!(!world.text("lib/P.yaml").contains("library/s"));
        assert!(lock(&world.port.renamed).is_empty(), "no agent step");
        assert_eq!(world.kind("library/s"), None);
        assert_eq!(world.kind("library/s/sub"), Some(ItemKind::Skill));

        let world = World::new();
        world.put("lib/s/SKILL.md", "---\nname: s\ndescription: d\n---\n");
        world.put("lib/P.yaml", &preset("skills:\n- library/s\n"));
        world.scan();
        let (code, _) =
            refused(
                world
                    .svc
                    .rename("library/s/SKILL.md", "notes.md", false, false, None, None),
            );
        assert_eq!(
            code, SKILL_UNCONFIRMED,
            "a rename away from SKILL.md asks too"
        );
        let before = world.text("lib/P.yaml");

        let answer = world
            .svc
            .rename("library/s/SKILL.md", "notes.md", true, false, None, None)
            .unwrap();

        done(&answer.items[0].outcome);
        assert_eq!(
            world.text("lib/P.yaml"),
            before,
            "plain yes keeps the reference"
        );
        assert_eq!(world.kind("library/s"), None);
        assert_eq!(
            world.kind("library/s/notes.md"),
            Some(ItemKind::Instruction)
        );
    }

    // ADR-0339 M108: detachRefs on a move with no lone SKILL.md leaving its skill is refused whole, nothing moved.
    #[test]
    fn detach_refs_on_a_move_without_a_skill_md_leaving_its_skill_is_invalid() {
        let world = World::new();
        world.put("lib/a.md", "a");
        world.put("lib/d/keep.md", "k");
        world.scan();

        let answer =
            world
                .svc
                .move_items(&["library/a.md"], "library/d", None, true, true, None, None);

        match answer {
            Err(LibraryRefusal::Refused { class, reason }) => {
                assert_eq!(class, RefusalClass::InvalidArgument);
                assert_eq!(reason.code, DETACH_REFS_INVALID);
            }
            other => panic!("{other:?}"),
        }
        assert!(world.there("lib/a.md"));
    }

    // ADR-0409 decision 10: a rename onto a taken name is `name-exists`, nothing replaced; a case-only rename goes
    //   through the no-replace rename and the catalog spells the new name.
    #[test]
    fn a_rename_onto_a_taken_name_conflicts_and_a_case_only_rename_goes_through() {
        let world = World::new();
        world.put("lib/a.md", "a");
        world.put("lib/b.md", "b");
        world.scan();

        let answer = world.rename("library/a.md", "b.md");
        assert_eq!(conflict_code(&answer.items[0].outcome), NAME_EXISTS);
        assert_eq!(world.text("lib/b.md"), "b");

        let answer = world.rename("library/a.md", "A.md");

        assert_eq!(done(&answer.items[0].outcome).to, "library/A.md");
        let names: Vec<String> = fs::read_dir(world.at("lib"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert!(names.contains(&"A.md".to_owned()), "{names:?}");
        assert!(!names.contains(&"a.md".to_owned()), "{names:?}");
        let spelled = world.svc.with_read(|state| {
            state
                .catalog
                .get(&key(&lp("library/A.md")))
                .map(|item| item.path.as_str().to_owned())
        });
        assert_eq!(spelled.as_deref(), Some("library/A.md"));
        let (code, _) =
            refused(
                world
                    .svc
                    .rename("library/A.md", "x/y.md", false, false, None, None),
            );
        assert_eq!(code, ITEM_PATH_INVALID);
    }

    // ADR-0365 · ADR-0414 decision 5: the canonical into-itself probe follows links, so it runs only on what the
    //   no-follow admission passed — a destination folder that is a link (here into the item) is refused
    //   `path-through-link` by the hold, and no canonical path was asked through it.
    #[test]
    fn a_destination_folder_behind_a_link_is_refused_by_the_admission_never_probed_through() {
        let world = World::new();
        world.put("lib/d/sub/x.md", "x");
        world.link("lib/d/sub", "lib/j");
        world.scan();

        let answer = world.mv(&["library/d"], "library/j", None);

        assert_eq!(failed_code(&answer.items[0].outcome), PATH_THROUGH_LINK);
        let link = world.at("lib/j");
        assert!(
            !lock(&world.probes.canonical)
                .iter()
                .any(|asked| same_path(asked, &link)),
            "no canonical path asked through the link"
        );
        assert!(world.there("lib/d/sub/x.md"));
    }

    // ADR-0365: a moved skill's SKILL.md is re-read through no link — a folder that became a link between the rename
    //   and the re-read is left to the next scan (the catalog keeps the re-keyed item); a skill below a moved folder
    //   whose folder a junction replaced before the move is not read through it either, nor a moved SKILL.md whose new
    //   folder a junction replaced.
    #[test]
    fn a_skill_re_read_never_follows_a_link_that_replaced_its_folder() {
        let world = World::new();
        world.put("lib/up/s/SKILL.md", "---\ndescription: d\n---\n");
        world.put(
            "keep/evil/SKILL.md",
            "---\nname: evil\ndescription: d\n---\n",
        );
        world.scan();
        let (moved, real, evil) = (
            world.at("lib/up/t"),
            world.at("keep/real"),
            world.at("keep/evil"),
        );
        lock(&world.links).push(moved.clone());
        let hook: Box<dyn FnOnce() + Send> = Box::new(move || {
            fs::rename(&moved, &real).unwrap();
            link_dir(&evil, &moved).unwrap();
        });
        *lock(&world.disk.after_rename) = Some((world.at("lib/up/s"), hook));

        done(&world.rename("library/up/s", "t").items[0].outcome);

        assert!(
            lock(&world.disk.after_rename).is_none(),
            "the link replaced the folder"
        );
        assert_eq!(
            world.skill("library/up/t"),
            ("s".to_owned(), "s".to_owned()),
            "nothing read through the link"
        );

        let world = World::new();
        world.put("lib/up/s/SKILL.md", "---\ndescription: d\n---\n");
        world.put(
            "keep/evil/SKILL.md",
            "---\nname: evil\ndescription: d\n---\n",
        );
        world.put("lib/d/keep.md", "k");
        world.scan();
        fs::rename(world.at("lib/up/s"), world.at("keep/real")).unwrap();
        world.link("keep/evil", "lib/up/s");
        lock(&world.links).push(world.at("lib/d/up/s"));

        done(&world.mv(&["library/up"], "library/d", None).items[0].outcome);

        assert_eq!(
            world.skill("library/d/up/s"),
            ("s".to_owned(), "s".to_owned())
        );

        // A moved SKILL.md whose new folder a junction replaced right after the rename: neither its own re-read nor the
        //   folder's re-classification reads through the link.
        let world = World::new();
        world.put("lib/SKILL.md", "---\ndescription: d\n---\n");
        world.put("lib/f/keep.md", "k");
        world.put(
            "keep/evil/SKILL.md",
            "---\nname: evil\ndescription: d\n---\n",
        );
        world.scan();
        let (folder, real, evil) = (
            world.at("lib/f"),
            world.at("keep/real"),
            world.at("keep/evil"),
        );
        lock(&world.links).push(folder.clone());
        let hook: Box<dyn FnOnce() + Send> = Box::new(move || {
            fs::rename(&folder, &real).unwrap();
            link_dir(&evil, &folder).unwrap();
        });
        *lock(&world.disk.after_rename) = Some((world.at("lib/SKILL.md"), hook));

        done(&world.mv(&["library/SKILL.md"], "library/f", None).items[0].outcome);

        assert!(
            lock(&world.disk.after_rename).is_none(),
            "the link replaced the folder"
        );
        assert_eq!(
            world.kind("library/f"),
            None,
            "no skill read through the link"
        );
    }

    // ADR-0409 M7 · ADR-0339: a merge's emptied source folder maps only itself, and only when its SKILL.md became the
    //   destination's — a reference to an entry under it that did not move (gone from disk) stays, and what the
    //   catalog still holds under it leaves instead of landing on the destination's keys.
    #[test]
    fn a_merged_skill_folder_maps_only_itself_and_its_stale_catalog_range_leaves() {
        let world = World::new();
        world.put("lib/src/s/SKILL.md", "---\nname: s\ndescription: d\n---\n");
        world.put("lib/src/s/w.md", "mine");
        world.put("lib/t/s/w.md", "theirs");
        world.put(
            "lib/P.yaml",
            &preset("skills:\n- library/src/s\ninstructions:\n- library/src/s/w.md\n"),
        );
        world.scan();
        fs::remove_file(world.at("lib/src/s/w.md")).unwrap();

        let answer = world.mv(&["library/src/s"], "library/t", Some(OnConflict::Overwrite));

        done(&answer.items[0].outcome);
        assert!(!world.there("lib/src/s"));
        assert_eq!(
            pairs(&answer),
            [
                ("library/src/s/SKILL.md", "library/t/s/SKILL.md"),
                ("library/src/s", "library/t/s"),
            ]
        );
        let exact: Vec<bool> = answer.mapping.iter().map(|row| row.2).collect();
        assert_eq!(
            exact,
            [false, true],
            "the emptied folder's row is marked exact"
        );
        let p = world.text("lib/P.yaml");
        assert!(p.lines().any(|line| line.ends_with(" library/t/s")), "{p}");
        assert!(
            p.contains("library/src/s/w.md") && !p.contains("library/t/s/w.md"),
            "a reference to an entry that did not move stays: {p}"
        );
        assert_eq!(
            world.kind("library/src/s/w.md"),
            None,
            "the stale item left"
        );
        let excerpt = world.svc.with_read(|state| {
            match state
                .catalog
                .get(&key(&lp("library/t/s/w.md")))
                .map(|item| &item.content)
            {
                Some(ItemContent::Instruction { excerpt }) => excerpt.clone(),
                other => panic!("{other:?}"),
            }
        });
        assert!(
            excerpt.contains("theirs"),
            "the destination's own item: {excerpt}"
        );
        assert_eq!(world.kind("library/t/s"), Some(ItemKind::Skill));
    }

    // ADR-0339: a skill merged into a skill under keep-both keeps its SKILL.md as `SKILL (2).md` — the folder's
    //   references do not move onto the other skill.
    #[test]
    fn a_skill_merged_into_a_skill_under_keep_both_keeps_its_references() {
        let world = World::new();
        world.put(
            "lib/src/s/SKILL.md",
            "---\nname: one\ndescription: d\n---\n",
        );
        world.put("lib/t/s/SKILL.md", "---\nname: two\ndescription: d\n---\n");
        world.put("lib/P.yaml", &preset("skills:\n- library/src/s\n"));
        world.scan();
        let before = world.text("lib/P.yaml");

        let answer = world.mv(&["library/src/s"], "library/t", Some(OnConflict::KeepBoth));

        done(&answer.items[0].outcome);
        assert!(!world.there("lib/src/s"), "emptied and removed");
        assert_eq!(
            pairs(&answer),
            [("library/src/s/SKILL.md", "library/t/s/SKILL (2).md")]
        );
        assert_eq!(
            world.text("lib/P.yaml"),
            before,
            "skill one's reference does not name skill two"
        );
        assert_eq!(world.skill("library/t/s").0, "two");
    }

    // ADR-0409: an item whose source an earlier item of the same call displaced or moved another entry onto is no
    //   longer what the checks saw — it fails `not-found` and nothing of it moves.
    #[test]
    fn an_item_whose_source_an_earlier_item_replaced_fails_not_found() {
        let world = World::new();
        world.put("lib/p/d/e.md", "p");
        world.put("lib/t/d/e.md", "t");
        world.scan();

        let answer = world.mv(
            &["library/p/d", "library/t/d/e.md"],
            "library/t",
            Some(OnConflict::Overwrite),
        );

        assert_eq!(done(&answer.items[0].outcome).trashed.len(), 1);
        assert_eq!(failed_code(&answer.items[1].outcome), NOT_FOUND);
        assert_eq!(world.text("lib/t/d/e.md"), "p");
        assert!(!world.there("lib/t/e.md"));
        assert_eq!(pairs(&answer), [("library/p/d/e.md", "library/t/d/e.md")]);
    }

    // ADR-0409 decision 5: an overwriting move checks its destination mount's trash before any disk step — unusable,
    //   the item fails `trash-unusable` and nothing of it moves, a merge's free entries and a free name included.
    #[test]
    fn an_overwriting_move_into_a_mount_whose_trash_cannot_be_used_moves_nothing() {
        let world = World::new();
        world.put("lib/src/d/x.md", "x1");
        world.put("lib/src/d/y.md", "y1");
        world.put("lib/t/d/x.md", "x0");
        world.put("lib/b.md", "b");
        world.put("lib/.trash", "not a folder");
        world.scan();

        let answer = world.mv(
            &["library/src/d", "library/b.md"],
            "library/t",
            Some(OnConflict::Overwrite),
        );

        assert_eq!(failed_code(&answer.items[0].outcome), TRASH_UNUSABLE);
        assert_eq!(failed_code(&answer.items[1].outcome), TRASH_UNUSABLE);
        assert_eq!(world.text("lib/src/d/y.md"), "y1");
        assert!(!world.there("lib/t/d/y.md"));
        assert!(world.there("lib/b.md") && !world.there("lib/t/b.md"));
        assert!(answer.mapping.is_empty());
    }

    // ADR-0409 decisions 7 and 8: two items overwriting one destination path keep both displaced entries — the second
    //   goes into a second bundle of the mount — and each answer names its own row.
    #[test]
    fn two_items_overwriting_one_name_keep_both_displaced_entries_under_two_rows() {
        let world = World::new();
        world.put("lib/a/x.md", "A");
        world.put("lib/b/x.md", "B");
        world.put("lib/d/x.md", "old");
        world.scan();

        let answer = world.mv(
            &["library/a/x.md", "library/b/x.md"],
            "library/d",
            Some(OnConflict::Overwrite),
        );

        let first = done(&answer.items[0].outcome).trashed;
        let second = done(&answer.items[1].outcome).trashed;
        assert_ne!(first[0].bundle, second[0].bundle);
        assert_eq!(
            world.text(&format!("lib/.trash/{}/d/x.md", first[0].bundle)),
            "old"
        );
        assert_eq!(
            world.text(&format!("lib/.trash/{}/d/x.md", second[0].bundle)),
            "A"
        );
        assert_eq!(world.text("lib/d/x.md"), "B");
        let view = world.view();
        let bundles = view.mounts[0].view.as_ref().unwrap();
        assert_eq!(bundles.len(), 2);
        assert!(bundles
            .iter()
            .all(|bundle| bundle.source == RowSource::Record));
    }

    // ADR-0409 M7: a merged source folder whose removal fails stays, is named in `left` and maps nowhere.
    #[test]
    fn a_merged_folder_that_cannot_be_removed_is_named_in_left() {
        let world = World::new();
        world.put("lib/src/d/y.md", "y");
        world.put("lib/t/d/x.md", "x");
        world.scan();
        lock(&world.disk.fail_remove).push(world.at("lib/src/d"));

        let answer = world.mv(&["library/src/d"], "library/t", Some(OnConflict::Skip));

        let moved = done(&answer.items[0].outcome);
        assert_eq!(moved.left.len(), 1, "{:?}", moved.left);
        assert_eq!(moved.left[0].0, "library/src/d");
        assert_eq!(moved.left[0].1.code, LIBRARY_WRITE_FAILED);
        assert!(world.there("lib/src/d"));
        assert_eq!(pairs(&answer), [("library/src/d/y.md", "library/t/d/y.md")]);
    }

    // ADR-0409 decision 4: the git working tree of each rewritten file is judged from its own folder — a repository
    //   nested in a mount is its own tree.
    #[test]
    fn each_rewritten_files_git_tree_is_judged_from_its_folder() {
        let world = World::new();
        world.put("lib/a.md", "a");
        world.put("lib/d/keep.md", "k");
        world.put("lib/Q.yaml", &preset("instructions:\n- library/a.md\n"));
        world.put("lib/sub/P.yaml", &preset("instructions:\n- library/a.md\n"));
        fs::create_dir_all(world.at("lib/sub/.git")).unwrap();
        world.scan();

        let answer = world.mv(&["library/a.md"], "library/d", None);

        assert_eq!(answer.written, ["library/Q.yaml", "library/sub/P.yaml"]);
        assert_eq!(answer.trees.len(), 2, "{:?}", answer.trees);
        let nested = GitTree::At(world.at("lib/sub"));
        assert_eq!(answer.trees[1], ("library".to_owned(), nested.clone()));
        assert_ne!(answer.trees[0].1, nested);
    }

    // ADR-0365: a move admits the item and the destination without following — an item or a destination folder behind
    //   a link is refused `path-through-link` with nothing moved through it; inside a merge a link moves as a link
    //   and a destination link is never merged into.
    #[test]
    fn a_move_goes_through_no_link_and_a_merge_moves_links_as_links() {
        let world = World::new();
        world.put("lib/a.md", "a");
        world.put("keep/ext/x.md", "x");
        world.put("keep/far/f.md", "f");
        world.put("lib/src/d/sub/z.md", "z");
        world.put("lib/t/d/keep.md", "k");
        world.link("keep/ext", "lib/j");
        world.link("keep/far", "lib/src/d/l");
        world.link("keep/far", "lib/t/d/sub");
        lock(&world.links).push(world.at("lib/t/d/l"));
        world.scan();

        let answer = world.mv(&["library/j/x.md"], "library/t", None);
        assert_eq!(failed_code(&answer.items[0].outcome), PATH_THROUGH_LINK);
        let answer = world.mv(&["library/a.md"], "library/j", None);
        assert_eq!(failed_code(&answer.items[0].outcome), PATH_THROUGH_LINK);
        assert!(world.there("lib/a.md") && !world.there("keep/ext/a.md"));
        assert!(world.there("keep/ext/x.md"));

        let answer = world.mv(&["library/src/d"], "library/t", Some(OnConflict::Overwrite));

        let moved = done(&answer.items[0].outcome);
        assert_eq!(moved.left.len(), 1, "{:?}", moved.left);
        assert_eq!(moved.left[0].0, "library/src/d/sub");
        assert_eq!(moved.left[0].1.code, KIND_MISMATCH);
        assert!(moved.trashed.is_empty());
        assert!(
            fs::symlink_metadata(world.at("lib/t/d/l"))
                .unwrap()
                .file_type()
                .is_symlink(),
            "moved as a link"
        );
        assert_eq!(world.text("keep/far/f.md"), "f");
        assert!(
            !world.there("keep/far/z.md"),
            "nothing merged through the link"
        );
        assert_eq!(world.text("lib/src/d/sub/z.md"), "z");
    }

    // ADR-0409 decision 2 · ADR-0414 decision 4: a move's SKILL.md judgement runs again in its hold — a preset that
    //   started listing the skill after the call's judgement answers the conflict with the fresh plan, nothing moved.
    #[test]
    fn a_skill_md_whose_skill_became_referenced_before_the_hold_is_asked_with_the_fresh_plan() {
        let world = World::new();
        world.put("lib/s/SKILL.md", "---\nname: s\ndescription: d\n---\n");
        world.put("lib/d/keep.md", "k");
        world.put("lib/S.yaml", &preset(""));
        world.scan();
        let svc = Arc::clone(&world.svc);
        BEFORE_HOLD.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move || {
                let entries = AddEntries::Items(vec![NewItem {
                    path: "library/s".to_owned(),
                    backend: None,
                }]);
                svc.preset_add(&lp("library/S.yaml"), &entries, None, None)
                    .unwrap();
            }));
        });

        let (code, fresh) = refused(world.svc.move_items(
            &["library/s/SKILL.md"],
            "library/d",
            None,
            false,
            false,
            None,
            None,
        ));

        assert_eq!(code, SKILL_UNCONFIRMED);
        assert_eq!(fresh["plan"]["presets"], json!(["library/S.yaml"]));
        assert!(world.there("lib/s/SKILL.md"), "nothing moved");
        assert!(!world.there("lib/d/SKILL.md"));
    }

    // ADR-0362 · ADR-0409 decision 4: the prefix mapper matches by the platform path key — a reference spelled in
    //   another case follows a folder moved across mounts where keys fold case, and elsewhere names no moved path.
    #[test]
    fn a_reference_spelled_in_another_case_follows_a_move_across_mounts_by_the_path_key() {
        let world = World::new();
        world.put("lib/r/a.md", "a");
        world.put("lib2/Q.yaml", &preset("instructions:\n- Library/R/a.md\n"));
        world.scan();
        let folds = path_key(Path::new("Library/R")) == path_key(Path::new("library/r"));

        let answer = world.mv(&["library/r"], "team", None);

        let written: &[&str] = if folds { &["team/Q.yaml"] } else { &[] };
        assert_eq!(answer.written, written);
        assert_eq!(world.text("lib2/Q.yaml").contains("team/r/a.md"), folds);
    }

    // ADR-0339 M110 · ADR-0409 decision 2: a merge that takes a referenced skill's SKILL.md and leaves its folder behind
    //   (an entry skipped) asks like a SKILL.md moved out — the conflict with the plan, nothing moved; plain yes moves
    //   and keeps the reference; yes + detachRefs + the digest moves and drops the line.
    #[test]
    fn a_merge_leaving_a_referenced_skill_without_its_skill_md_asks_first() {
        let setup = || {
            let world = World::new();
            world.put(
                "lib/review/SKILL.md",
                "---\nname: review\ndescription: d\n---\n",
            );
            world.put("lib/review/notes.md", "mine");
            world.put("lib2/review/notes.md", "theirs");
            world.put("lib/P.yaml", &preset("skills:\n- library/review\n"));
            world.scan();
            world
        };
        let skip = Some(OnConflict::Skip);
        let world = setup();

        let (code, args) = refused(world.svc.move_items(
            &["library/review"],
            "team",
            skip,
            false,
            false,
            None,
            None,
        ));

        assert_eq!(code, SKILL_UNCONFIRMED);
        assert_eq!(args["plan"]["presets"], json!(["library/P.yaml"]));
        assert!(world.there("lib/review/SKILL.md") && !world.there("lib2/review/SKILL.md"));
        let before = world.text("lib/P.yaml");

        let answer = world
            .svc
            .move_items(&["library/review"], "team", skip, true, false, None, None)
            .unwrap();

        let moved = done(&answer.items[0].outcome);
        assert_eq!(moved.left[0].0, "library/review/notes.md");
        assert!(world.there("lib2/review/SKILL.md") && world.there("lib/review/notes.md"));
        assert_eq!(
            world.text("lib/P.yaml"),
            before,
            "plain yes keeps the reference"
        );
        assert_eq!(world.kind("library/review"), None);
        assert_eq!(world.kind("team/review"), Some(ItemKind::Skill));

        let world = setup();
        let (_, args) = refused(world.svc.move_items(
            &["library/review"],
            "team",
            skip,
            false,
            false,
            None,
            None,
        ));

        let answer = world
            .svc
            .move_items(
                &["library/review"],
                "team",
                skip,
                true,
                true,
                Some(&digest_of(&args)),
                None,
            )
            .unwrap();

        done(&answer.items[0].outcome);
        assert_eq!(answer.detached, ["library/review"]);
        assert_eq!(answer.written, ["library/P.yaml"]);
        assert!(!world.text("lib/P.yaml").contains("review"));
        assert!(lock(&world.port.renamed).is_empty(), "no agent step");

        // Judged again in the hold: the entry that was to stay behind went before it, so the merge empties the folder
        //   and its reference follows the SKILL.md instead of being dropped.
        let world = setup();
        let (_, args) = refused(world.svc.move_items(
            &["library/review"],
            "team",
            skip,
            false,
            false,
            None,
            None,
        ));
        let theirs = world.at("lib2/review/notes.md");
        BEFORE_HOLD.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move || fs::remove_file(theirs).unwrap()));
        });

        let answer = world
            .svc
            .move_items(
                &["library/review"],
                "team",
                skip,
                true,
                true,
                Some(&digest_of(&args)),
                None,
            )
            .unwrap();

        done(&answer.items[0].outcome);
        assert!(answer.detached.is_empty(), "{:?}", answer.detached);
        assert!(!world.there("lib/review"));
        let p = world.text("lib/P.yaml");
        assert!(p.lines().any(|line| line.ends_with(" team/review")), "{p}");
    }

    // ADR-0339 M110: a merge moves a skill's SKILL.md last — when an earlier item of the call made an entry stay
    //   behind that the judgement did not plan (no ask), the SKILL.md stays too (left skill-unconfirmed) and the
    //   skill keeps its references.
    #[test]
    fn a_merge_keeps_a_skill_md_whose_folder_an_earlier_item_left_holding_an_entry() {
        let world = World::new();
        world.put("lib/a/S/x.md", "a");
        world.put("lib/b/S/SKILL.md", "---\nname: S\ndescription: d\n---\n");
        world.put("lib/b/S/x.md", "b");
        world.put("lib/t/keep.md", "k");
        world.put("lib/P.yaml", &preset("skills:\n- library/b/S\n"));
        world.scan();
        let before = world.text("lib/P.yaml");

        let answer = world.mv(
            &["library/a/S", "library/b/S"],
            "library/t",
            Some(OnConflict::Skip),
        );

        assert_eq!(done(&answer.items[0].outcome).to, "library/t/S");
        let left: Vec<(String, String)> = done(&answer.items[1].outcome)
            .left
            .into_iter()
            .map(|(path, why)| (path, why.code))
            .collect();
        assert_eq!(
            left,
            [
                ("library/b/S/x.md".to_owned(), NAME_EXISTS.to_owned()),
                (
                    "library/b/S/SKILL.md".to_owned(),
                    SKILL_UNCONFIRMED.to_owned()
                ),
            ]
        );
        assert!(world.there("lib/b/S/SKILL.md") && !world.there("lib/t/S/SKILL.md"));
        assert_eq!(world.kind("library/b/S"), Some(ItemKind::Skill));
        assert_eq!(world.text("lib/P.yaml"), before);
    }

    // ADR-0339 M110: an entry of a skill folder failing after the SKILL.md would have moved (it went first) leaves the
    //   SKILL.md where it is — nothing planned it to leave.
    #[test]
    fn a_merge_whose_entry_fails_keeps_the_skill_md_that_would_have_moved() {
        let world = World::new();
        world.put("lib/b/S/SKILL.md", "---\nname: S\ndescription: d\n---\n");
        world.put("lib/b/S/x.md", "b");
        world.put("lib/t/S/other.md", "o");
        world.put("lib/P.yaml", &preset("skills:\n- library/b/S\n"));
        world.scan();
        let before = world.text("lib/P.yaml");
        lock(&world.disk.fail_renames).push((world.at("lib/b/S/x.md"), 0));

        let answer = world.mv(&["library/b/S"], "library/t", Some(OnConflict::Skip));

        let left: Vec<(String, String)> = done(&answer.items[0].outcome)
            .left
            .into_iter()
            .map(|(path, why)| (path, why.code))
            .collect();
        assert_eq!(
            left,
            [
                (
                    "library/b/S/x.md".to_owned(),
                    LIBRARY_WRITE_FAILED.to_owned()
                ),
                (
                    "library/b/S/SKILL.md".to_owned(),
                    SKILL_UNCONFIRMED.to_owned()
                ),
            ]
        );
        assert!(answer.mapping.is_empty());
        assert!(world.there("lib/b/S/SKILL.md"));
        assert_eq!(world.kind("library/b/S"), Some(ItemKind::Skill));
        assert_eq!(world.text("lib/P.yaml"), before);
    }

    // ADR-0339 M110 · ADR-0414 decision 1: a confirmed detach of a skill a merge was to leave behind drops nothing when
    //   an earlier item of the call cleared the collision and the folder merged away whole — its references follow
    //   the exact pair onto the destination skill.
    #[test]
    fn a_skill_folder_that_merged_away_whole_after_its_planned_leave_keeps_its_references() {
        let world = World::new();
        world.put("lib/b/S/SKILL.md", "---\nname: S\ndescription: d\n---\n");
        world.put("lib/b/S/x.md", "b");
        world.put("lib/t/S/x.md", "t");
        world.put("lib/P.yaml", &preset("skills:\n- library/b/S\n"));
        world.scan();
        let paths = ["library/t/S/x.md", "library/b/S"];
        let skip = Some(OnConflict::Skip);
        let (code, args) =
            refused(
                world
                    .svc
                    .move_items(&paths, "library/t", skip, false, false, None, None),
            );
        assert_eq!(code, SKILL_UNCONFIRMED);

        let answer = world
            .svc
            .move_items(
                &paths,
                "library/t",
                skip,
                true,
                true,
                Some(&digest_of(&args)),
                None,
            )
            .unwrap();

        done(&answer.items[0].outcome);
        done(&answer.items[1].outcome);
        assert!(!world.there("lib/b/S"));
        assert!(answer.detached.is_empty(), "{:?}", answer.detached);
        let p = world.text("lib/P.yaml");
        assert!(p.lines().any(|line| line.ends_with(" library/t/S")), "{p}");
        assert_eq!(world.kind("library/t/S"), Some(ItemKind::Skill));
    }

    // ADR-0409 decision 4: each pair maps what the pairs before it made of a reference — an entry merged into a folder
    //   that a later item of the call moves on is followed to where it is now, by the preset files and the agents.
    #[test]
    fn references_and_agents_follow_every_pair_in_move_order() {
        let profile = agent(&["library/p/S/y/q.yaml"]);
        let id = profile.id;
        let world = World::with(vec![profile]);
        world.put("lib/p/S/y/f.md", "f");
        world.put("lib/p/S/y/q.yaml", &preset(""));
        world.put("lib/t/S/y/g.md", "g");
        world.put(
            "lib/P.yaml",
            &preset("instructions:\n- library/p/S/y/f.md\n"),
        );
        world.scan();

        let answer = world.mv(
            &["library/p/S", "library/t/S/y"],
            "library/t",
            Some(OnConflict::Skip),
        );

        done(&answer.items[0].outcome);
        assert_eq!(done(&answer.items[1].outcome).to, "library/t/y");
        assert_eq!(
            pairs(&answer),
            [
                ("library/p/S/y/f.md", "library/t/S/y/f.md"),
                ("library/p/S/y/q.yaml", "library/t/S/y/q.yaml"),
                ("library/t/S/y", "library/t/y"),
            ]
        );
        let p = world.text("lib/P.yaml");
        assert!(p.contains("library/t/y/f.md"), "{p}");
        assert_eq!(world.presets_of(id), ["library/t/y/q.yaml"]);
        assert_eq!(world.kind("library/t/y/q.yaml"), Some(ItemKind::Preset));
    }

    // ADR-0365: the git working tree of a skipped file is probed only through admitted folders — a folder that became
    //   a link gives its file no tree, and nothing is asked behind it.
    #[test]
    fn a_skipped_file_behind_a_link_is_given_no_git_tree() {
        let world = World::new();
        world.put("lib/a.md", "a");
        world.put("lib/d/keep.md", "k");
        world.put("lib/j/P.yaml", &preset("instructions:\n- library/a.md\n"));
        world.scan();
        fs::rename(world.at("lib/j"), world.at("keep/real")).unwrap();
        world.put(
            "keep/ext/P.yaml",
            &preset("instructions:\n- library/a.md\n"),
        );
        fs::create_dir_all(world.at("keep/ext/.git")).unwrap();
        world.link("keep/ext", "lib/j");

        let answer = world.mv(&["library/a.md"], "library/d", None);

        assert_eq!(answer.skipped.len(), 1, "{:?}", answer.skipped);
        assert_eq!(answer.skipped[0].path, "library/j/P.yaml");
        assert_eq!(answer.skipped[0].reason.code, PATH_THROUGH_LINK);
        assert!(answer.trees.is_empty(), "{:?}", answer.trees);
    }

    // ADR-0409 decision 5: an overwriting move judges its destination's trash again in the hold before its first
    //   rename — a .trash broken after the call's check fails the item with nothing of it moved.
    #[test]
    fn an_overwrite_whose_trash_broke_before_the_hold_moves_nothing() {
        let world = World::new();
        world.put("lib/src/d/x.md", "x1");
        world.put("lib/src/d/y.md", "y1");
        world.put("lib/t/d/x.md", "x0");
        world.scan();
        let trash = world.at("lib/.trash");
        BEFORE_HOLD.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move || fs::write(trash, "not a folder").unwrap()));
        });

        let answer = world.mv(&["library/src/d"], "library/t", Some(OnConflict::Overwrite));

        assert_eq!(failed_code(&answer.items[0].outcome), TRASH_UNUSABLE);
        assert_eq!(world.text("lib/src/d/y.md"), "y1");
        assert!(!world.there("lib/t/d/y.md"));
        assert_eq!(world.text("lib/t/d/x.md"), "x0");
        assert!(answer.mapping.is_empty());
    }

    // ADR-0339 M110: the merge plan finds a skill left without its SKILL.md under keep-both (SKILL (2).md while an
    //   entry stays), under overwrite (a file onto a folder stays) and in a nested folder — each asks first, nothing
    //   moved; keep-both confirmed moves the SKILL.md to its keep-both name.
    #[test]
    fn the_merge_plan_finds_a_skill_left_under_keep_both_by_a_kind_mismatch_and_nested() {
        let ask = |world: &World, paths: &[&str], on_conflict: OnConflict| {
            let (code, args) = refused(world.svc.move_items(
                paths,
                "team",
                Some(on_conflict),
                false,
                false,
                None,
                None,
            ));
            assert_eq!(code, SKILL_UNCONFIRMED);
            assert_eq!(args["plan"]["presets"], json!(["library/P.yaml"]));
        };

        let world = World::new();
        world.put("lib/s/SKILL.md", "---\nname: s\ndescription: d\n---\n");
        world.put("lib/s/x.md", "mine");
        world.put("lib2/s/SKILL.md", "---\nname: other\ndescription: d\n---\n");
        world.put("lib2/s/x.md", "theirs");
        world.put("lib/P.yaml", &preset("skills:\n- library/s\n"));
        world.scan();
        *lock(&world.disk.fail_stat) = Some(world.at("lib2/s/x.md"));
        ask(&world, &["library/s"], OnConflict::KeepBoth);
        assert!(!world.there("lib2/s/SKILL (2).md"));
        let answer = world
            .svc
            .move_items(
                &["library/s"],
                "team",
                Some(OnConflict::KeepBoth),
                true,
                false,
                None,
                None,
            )
            .unwrap();
        let moved = done(&answer.items[0].outcome);
        assert_eq!(moved.left[0].0, "library/s/x.md");
        assert!(world.there("lib2/s/SKILL (2).md") && world.there("lib/s/x.md"));

        let world = World::new();
        world.put("lib/s/SKILL.md", "---\nname: s\ndescription: d\n---\n");
        world.put("lib/s/doc", "a file");
        world.put("lib2/s/doc/in.md", "in");
        world.put("lib/P.yaml", &preset("skills:\n- library/s\n"));
        world.scan();
        ask(&world, &["library/s"], OnConflict::Overwrite);
        assert!(world.there("lib/s/SKILL.md") && !world.there("lib2/s/SKILL.md"));

        let world = World::new();
        world.put("lib/up/s/SKILL.md", "---\nname: s\ndescription: d\n---\n");
        world.put("lib/up/s/x.md", "mine");
        world.put("lib2/up/s/x.md", "theirs");
        world.put("lib/P.yaml", &preset("skills:\n- library/up/s\n"));
        world.scan();
        ask(&world, &["library/up"], OnConflict::Skip);
        assert!(world.there("lib/up/s/SKILL.md") && !world.there("lib2/up/s/SKILL.md"));
    }

    // ADR-0409 decision 2 · ADR-0414 decision 4: a collision that appears before the hold turns a merge into one that
    //   leaves a referenced skill behind — the hold's plan asks with the fresh plan, nothing moved.
    #[test]
    fn a_merge_leave_appearing_before_the_hold_is_asked_with_the_fresh_plan() {
        let world = World::new();
        world.put("lib/s/SKILL.md", "---\nname: s\ndescription: d\n---\n");
        world.put("lib/s/x.md", "mine");
        world.put("lib2/s/other.md", "o");
        world.put("lib/P.yaml", &preset("skills:\n- library/s\n"));
        world.scan();
        let theirs = world.at("lib2/s/x.md");
        BEFORE_HOLD.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move || fs::write(theirs, "theirs").unwrap()));
        });

        let (code, fresh) = refused(world.svc.move_items(
            &["library/s"],
            "team",
            Some(OnConflict::Skip),
            false,
            false,
            None,
            None,
        ));

        assert_eq!(code, SKILL_UNCONFIRMED);
        assert_eq!(fresh["plan"]["presets"], json!(["library/P.yaml"]));
        assert!(world.there("lib/s/SKILL.md") && world.there("lib/s/x.md"));
        assert!(!world.there("lib2/s/SKILL.md"));
    }

    // ADR-0339 M110: the merge plan reads the disk only for an item holding a skill — a plain folder merge lists its
    //   source only in the merge itself and in the re-classification after it (the plan at the call and in the hold
    //   would list it twice more).
    #[test]
    fn a_merge_without_a_skill_plans_nothing_from_disk() {
        let world = World::new();
        world.put("lib/src/d/x.md", "x");
        world.put("lib/t/d/y.md", "y");
        world.scan();

        done(
            &world
                .mv(&["library/src/d"], "library/t", Some(OnConflict::Skip))
                .items[0]
                .outcome,
        );

        let source = world.at("lib/src/d");
        let listed = lock(&world.disk.lists)
            .iter()
            .filter(|path| **path == source)
            .count();
        assert_eq!(listed, 2);
    }

    // ADR-0365 · ADR-0414 decision 5: the into-itself probe admits the item too — an item behind a link is refused
    //   path-through-link by the hold and no canonical path is asked through the link.
    #[test]
    fn an_item_behind_a_link_is_refused_by_the_admission_never_probed_through() {
        let world = World::new();
        world.put("lib/j/d/x.md", "x");
        world.put("lib/t/keep.md", "k");
        world.scan();
        fs::rename(world.at("lib/j"), world.at("keep/real")).unwrap();
        world.put("keep/ext/d/x.md", "ext");
        world.link("keep/ext", "lib/j");

        let answer = world.mv(&["library/j/d"], "library/t", None);

        assert_eq!(failed_code(&answer.items[0].outcome), PATH_THROUGH_LINK);
        let link = world.at("lib/j");
        assert!(
            !lock(&world.probes.canonical)
                .iter()
                .any(|asked| asked.starts_with(&link)),
            "no canonical path asked through the link"
        );
        assert!(world.there("keep/ext/d/x.md") && !world.there("lib/t/d"));
    }

    // ADR-0339 M110: a SKILL.md that would stay anyway — taken under skip, a folder of its name there under overwrite —
    //   answers its own reason, not skill-unconfirmed: no confirmation could move it.
    #[test]
    fn a_skill_md_that_stays_anyway_in_a_merge_answers_its_own_reason() {
        let codes = |answer: &MoveAnswer| -> Vec<(String, String)> {
            done(&answer.items[0].outcome)
                .left
                .into_iter()
                .map(|(path, why)| (path, why.code))
                .collect()
        };
        let world = World::new();
        world.put("lib/s/SKILL.md", "---\nname: s\ndescription: d\n---\n");
        world.put("lib/s/x.md", "mine");
        world.put("lib2/s/SKILL.md", "---\nname: other\ndescription: d\n---\n");
        world.put("lib2/s/x.md", "theirs");
        world.scan();

        let answer = world.mv(&["library/s"], "team", Some(OnConflict::Skip));

        assert_eq!(
            codes(&answer),
            [
                ("library/s/x.md".to_owned(), NAME_EXISTS.to_owned()),
                ("library/s/SKILL.md".to_owned(), NAME_EXISTS.to_owned()),
            ]
        );

        let world = World::new();
        world.put("lib/s/SKILL.md", "---\nname: s\ndescription: d\n---\n");
        world.put("lib/s/doc", "a file");
        world.put("lib2/s/SKILL.md/in.md", "in");
        world.put("lib2/s/doc/in.md", "in");
        world.scan();

        let answer = world.mv(&["library/s"], "team", Some(OnConflict::Overwrite));

        assert_eq!(
            codes(&answer),
            [
                ("library/s/doc".to_owned(), KIND_MISMATCH.to_owned()),
                ("library/s/SKILL.md".to_owned(), KIND_MISMATCH.to_owned()),
            ]
        );
        assert!(world.there("lib/s/SKILL.md"));
    }

    // ADR-0339 · ADR-0409 M7: a skill whose every entry merged into the destination keeps its references there even
    //   when its empty folder cannot be removed — the folder is named in left, is no skill any more and maps exactly.
    #[test]
    fn a_merged_skill_whose_empty_folder_cannot_be_removed_still_maps_its_references() {
        let world = World::new();
        world.put("lib/src/s/SKILL.md", "---\nname: s\ndescription: d\n---\n");
        world.put("lib/src/s/w.md", "w");
        world.put("lib/t/s/other.md", "o");
        world.put("lib/P.yaml", &preset("skills:\n- library/src/s\n"));
        world.scan();
        lock(&world.disk.fail_remove).push(world.at("lib/src/s"));

        let answer = world.mv(&["library/src/s"], "library/t", Some(OnConflict::Skip));

        let moved = done(&answer.items[0].outcome);
        assert_eq!(moved.left.len(), 1, "{:?}", moved.left);
        assert_eq!(moved.left[0].0, "library/src/s");
        assert_eq!(moved.left[0].1.code, LIBRARY_WRITE_FAILED);
        assert!(world.there("lib/src/s"));
        assert_eq!(
            answer.mapping.last().unwrap(),
            &("library/src/s".to_owned(), "library/t/s".to_owned(), true)
        );
        let p = world.text("lib/P.yaml");
        assert!(p.lines().any(|line| line.ends_with(" library/t/s")), "{p}");
        assert_eq!(world.kind("library/src/s"), None);
        assert_eq!(world.kind("library/t/s"), Some(ItemKind::Skill));
    }
}
